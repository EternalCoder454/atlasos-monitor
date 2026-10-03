//! The sampling loop, without Qt: one worker thread that reads what the page
//! on screen shows, every `refreshInterval`, and hands each [`Tick`] to a
//! sink. `sampler.rs` is the QObject around it; its sink posts the parts to
//! the stats objects on the Qt thread.
//!
//! - Only the page on screen is read. Its readers are made when it opens,
//!   which takes their baseline, and dropped when it closes, so a page opened
//!   again starts with empty charts rather than stale history.
//! - The sidebar's live values (disk and network rates, battery charge) are
//!   read on every page: two held files and a power supply or two.
//! - `statvfs` (disk space) and the address dump run on a page's first tick
//!   and every 5th after. SMART and failed services are D-Bus questions on
//!   slower timers of their own, asked on a second thread so a slow or hung
//!   daemon never holds up a tick or closing the window. A tick uses the
//!   latest answers.
//! - The worker owns every reader and takes no lock: commands arrive over a
//!   channel, whose wait is also the sleep between ticks.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use atlas_sysinfo::gpu::{self, Card, Gpu, GpuSampler};
use atlas_sysinfo::health::{self, Alert, DiskSpace, Drive, Graphics, Machine};
use atlas_sysinfo::power::{self, PowerSampler, Supplies};
use atlas_sysinfo::sensors::{self, Device, Sensors};
use atlas_sysinfo::services::ServiceReader;
use atlas_sysinfo::smart::{self, SmartReader};
use atlas_sysinfo::stats::cpu::{self, CpuInfo, CpuSample, CpuSampler};
use atlas_sysinfo::stats::disk::{self, Disk, DiskIo, DiskSampler, Space};
use atlas_sysinfo::stats::memory::{Memory, MemorySampler};
use atlas_sysinfo::stats::net::{self, Addresses, NetInterface, NetIo, NetSampler};

/// Disk space and addresses are read on this tick of every this many.
const SLOW_EVERY: u64 = 5;
/// udisks2 refreshes SMART every 10 minutes; once a minute is plenty.
const SMART_EVERY: Duration = Duration::from_secs(60);
/// A page's first reading comes this soon after it opens (or one interval,
/// if shorter): long enough for a fair CPU load, short enough not to show
/// an empty page at a 5 s interval.
const FIRST_TICK: Duration = Duration::from_millis(500);

/// The page on screen, from QML's `activePage` ("overview", "cpu",
/// "disk:nvme0n1", ...). Pages with no live figures read only the sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Page {
    Overview,
    Cpu,
    Memory,
    Disk(String),
    Network(String),
    Gpu,
    Battery(String),
    Sensors,
    #[default]
    Other,
}

impl Page {
    pub fn parse(s: &str) -> Self {
        let (kind, device) = s.split_once(':').unwrap_or((s, ""));
        let device = device.to_owned();
        match kind {
            "overview" => Self::Overview,
            "cpu" => Self::Cpu,
            "memory" => Self::Memory,
            "disk" if !device.is_empty() => Self::Disk(device),
            "network" if !device.is_empty() => Self::Network(device),
            "gpu" => Self::Gpu,
            "battery" if !device.is_empty() => Self::Battery(device),
            "sensors" => Self::Sensors,
            _ => Self::Other,
        }
    }
}

/// What the sidebar shows on every page.
#[derive(Debug, Clone, Default)]
pub struct Devices {
    /// Whole disks, root first, swap last. Sent with a fresh tick only.
    pub disks: Option<Vec<Disk>>,
    /// Network interfaces. Sent with a fresh tick only.
    pub interfaces: Option<Vec<NetInterface>>,
    /// Throughput per disk, in the order of the list.
    pub disk_io: Vec<DiskIo>,
    /// Throughput per interface present now.
    pub net_io: Vec<NetIo>,
    /// The interface carrying the default route.
    #[allow(dead_code, reason = "the Network page will mark it; drop this then")]
    pub default_route: Option<String>,
    /// Batteries and adapters; `None` on a machine with none.
    #[allow(dead_code, reason = "the Battery page will read it; drop this then")]
    pub power: Option<Supplies>,
}

/// The processor, for the Overview and CPU pages.
#[derive(Debug, Clone, Default)]
pub struct CpuTick {
    pub sample: CpuSample,
    /// What the processor is. Sent with a fresh tick only.
    pub info: Option<CpuInfo>,
}

/// One graphics card's reading.
#[derive(Debug, Clone)]
pub struct GpuTick {
    pub card: Card,
    pub reading: Gpu,
}

/// The Disk page's drive.
#[derive(Debug, Clone, Default)]
pub struct DiskTick {
    pub name: String,
    /// What the drive is. On a fresh tick only; `None` for a drive that
    /// isn't in the list (gone since the page opened).
    pub disk: Option<Disk>,
    /// Its throughput this tick.
    pub io: Option<DiskIo>,
    /// `None` when nothing on it is mounted. Only on a slow tick.
    pub space: Option<Option<Space>>,
    /// What the drive says about itself: on the first tick, then whenever
    /// udisks2 answers (asked once a minute).
    pub health: Option<Option<smart::Health>>,
}

/// The Network page's interface.
#[derive(Debug, Clone, Default)]
pub struct NetTick {
    pub name: String,
    /// What the interface is. On a fresh tick only.
    pub interface: Option<NetInterface>,
    /// Its throughput this tick; `None` while it is gone (unplugged).
    pub io: Option<NetIo>,
    /// Its addresses. Only on a slow tick.
    pub addresses: Option<Addresses>,
}

/// One reading of everything the page on screen shows. A part the page
/// doesn't show is `None`.
#[allow(dead_code, reason = "read by the pages as they land; drop this then")]
#[derive(Debug, Clone, Default)]
pub struct Tick {
    /// The first tick since the page opened: charts start empty.
    pub fresh: bool,
    pub page: Page,
    pub devices: Devices,
    pub cpu: Option<CpuTick>,
    pub memory: Option<Memory>,
    pub gpus: Option<Vec<GpuTick>>,
    pub disk: Option<DiskTick>,
    pub net: Option<NetTick>,
    pub sensors: Option<Vec<Device>>,
    /// What is wrong, for the Overview.
    pub health: Option<Vec<Alert>>,
}

/// Every reader, owned by the worker thread.
pub struct Worker {
    page: Page,
    fresh: bool,
    ticks: u64,

    // Always read: the sidebar.
    disks: Vec<Disk>,
    disk_io: DiskSampler,
    interfaces: Vec<NetInterface>,
    net_io: NetSampler,
    power: Option<PowerSampler>,

    // Read for the page on screen; `None` on other pages.
    cpu: Option<CpuSampler>,
    memory: Option<MemorySampler>,
    gpus: Option<Vec<(Card, GpuSampler)>>,
    sensors: Option<Sensors>,

    // Read once, kept.
    cpu_info: Option<CpuInfo>,
    cards: Option<Vec<Card>>,

    // Slow figures, kept between their reads.
    space: Vec<Option<Space>>,
    questions: Option<Questions>,
    /// When each drive was last asked about, and its latest answer.
    asked: HashMap<String, Instant>,
    drives: HashMap<String, Option<smart::Health>>,
    /// Questions sent and not answered yet: none is asked twice at once.
    waiting_drives: HashSet<String>,
    waiting_failed: bool,
    failed: Vec<String>,
    /// The Disk page's drive was answered since its last tick.
    disk_news: bool,
}

impl Default for Worker {
    fn default() -> Self {
        Self::new()
    }
}

impl Worker {
    /// Lists the devices and takes the sidebar's baseline.
    pub fn new() -> Self {
        let disks = disk::disks();
        let disk_io = DiskSampler::new(&disks);
        Self {
            page: Page::Other,
            fresh: true,
            ticks: 0,
            space: vec![None; disks.len()],
            disks,
            disk_io,
            interfaces: net::interfaces(),
            net_io: NetSampler::new(),
            power: power::available().then(PowerSampler::new),
            cpu: None,
            memory: None,
            gpus: None,
            sensors: None,
            cpu_info: None,
            cards: None,
            questions: None,
            asked: HashMap::new(),
            drives: HashMap::new(),
            waiting_drives: HashSet::new(),
            waiting_failed: false,
            failed: Vec::new(),
            disk_news: false,
        }
    }

    pub fn page(&self) -> &Page {
        &self.page
    }

    /// Makes the readers `page` needs, dropping the ones it doesn't. The
    /// sidebar's readers are kept, so its rates don't restart.
    pub fn set_page(&mut self, page: Page) {
        self.cpu = matches!(page, Page::Overview | Page::Cpu).then(CpuSampler::new);
        self.memory = matches!(page, Page::Overview | Page::Memory).then(MemorySampler::new);
        self.gpus = matches!(page, Page::Overview | Page::Gpu).then(|| {
            let cards = self.cards.get_or_insert_with(gpu::cards);
            cards
                .iter()
                .map(|c| (c.clone(), GpuSampler::new(c, cards)))
                .collect()
        });
        self.sensors = (page == Page::Sensors && sensors::available()).then(Sensors::new);
        if page == Page::Cpu && self.cpu_info.is_none() {
            self.cpu_info = Some(cpu::info());
        }
        self.page = page;
        self.fresh = true;
        self.ticks = 0;
    }

    /// Reads everything the page shows.
    pub fn tick(&mut self) -> Tick {
        let fresh = std::mem::take(&mut self.fresh);
        let slow = self.ticks.is_multiple_of(SLOW_EVERY);
        self.ticks += 1;
        self.take_answers();

        let devices = Devices {
            disks: fresh.then(|| self.disks.clone()),
            interfaces: fresh.then(|| self.interfaces.clone()),
            disk_io: self.disk_io.sample().to_vec(),
            net_io: self.net_io.sample().to_vec(),
            default_route: self.net_io.default_route().map(str::to_owned),
            power: self.power.as_mut().map(|p| p.sample().clone()),
        };
        let cpu = self.cpu.as_mut().map(|c| CpuTick {
            sample: c.sample().clone(),
            info: (fresh && self.page == Page::Cpu)
                .then(|| self.cpu_info.clone())
                .flatten(),
        });
        let memory = self.memory.as_mut().and_then(MemorySampler::sample);
        let gpus = self.gpus.as_mut().map(|gpus| {
            gpus.iter_mut()
                .map(|(card, s)| GpuTick {
                    card: card.clone(),
                    reading: s.sample(),
                })
                .collect()
        });
        let sensors = self.sensors.as_mut().map(|s| s.sample().to_vec());

        let mut tick = Tick {
            fresh,
            page: self.page.clone(),
            devices,
            cpu,
            memory,
            gpus,
            sensors,
            ..Tick::default()
        };

        match self.page.clone() {
            Page::Overview => {
                if slow {
                    self.read_space(None);
                    self.ask(Ask::Failed);
                }
                let names: Vec<String> = self
                    .disks
                    .iter()
                    .filter(|d| !d.is_swap)
                    .map(|d| d.name.clone())
                    .collect();
                for name in names {
                    self.ask_drive(name);
                }
                tick.health = Some(self.check(&tick));
            }
            Page::Disk(name) => {
                if slow {
                    self.read_space(Some(&name));
                }
                let index = self.disks.iter().position(|d| d.name == name);
                self.ask_drive(name.clone());
                let news = std::mem::take(&mut self.disk_news);
                tick.disk = Some(DiskTick {
                    disk: fresh
                        .then(|| index.map(|i| self.disks[i].clone()))
                        .flatten(),
                    io: index.and_then(|i| tick.devices.disk_io.get(i).cloned()),
                    space: (slow && index.is_some()).then(|| index.and_then(|i| self.space[i])),
                    health: (news || fresh).then(|| self.drives.get(&name).cloned().flatten()),
                    name,
                });
            }
            Page::Network(name) => {
                let interface = self.interfaces.iter().find(|i| i.name == name);
                let addresses = slow.then(|| match (interface, net::addresses()) {
                    (Some(i), Ok(mut all)) => all.remove(&i.index).unwrap_or_default(),
                    _ => Addresses::default(),
                });
                tick.net = Some(NetTick {
                    interface: fresh.then(|| interface.cloned()).flatten(),
                    io: tick.devices.net_io.iter().find(|n| n.name == name).cloned(),
                    addresses,
                    name,
                });
            }
            _ => {}
        }
        tick
    }

    /// `statvfs` for one disk, or for every disk.
    fn read_space(&mut self, only: Option<&str>) {
        for (d, space) in self.disks.iter().zip(&mut self.space) {
            if only.is_none_or(|n| n == d.name) && !d.is_swap {
                *space = disk::space(&d.mounts);
            }
        }
    }

    /// Asks udisks2 about `name` if a minute has passed since it last did.
    fn ask_drive(&mut self, name: String) {
        let due = self
            .asked
            .get(&name)
            .is_none_or(|at| at.elapsed() >= SMART_EVERY);
        if due {
            self.asked.insert(name.clone(), Instant::now());
            self.ask(Ask::Drive(name));
        }
    }

    /// Sends a question unless the same one is still waiting for its answer.
    fn ask(&mut self, ask: Ask) {
        let waiting = match &ask {
            Ask::Drive(name) => !self.waiting_drives.insert(name.clone()),
            Ask::Failed => std::mem::replace(&mut self.waiting_failed, true),
        };
        if waiting {
            return;
        }
        let questions = self.questions.get_or_insert_with(Questions::start);
        if questions.asks.send(ask).is_err() {
            // The thread could not start or has died. Forget it and what it
            // was asked; the next question, a slow tick or a minute away,
            // starts a new one.
            log::warn!("the D-Bus question thread is gone; starting another on the next question");
            self.lost_questions();
        }
    }

    fn lost_questions(&mut self) {
        self.questions = None;
        self.waiting_drives.clear();
        self.waiting_failed = false;
    }

    /// Takes in what the question thread has answered since the last tick.
    fn take_answers(&mut self) {
        let Some(questions) = &self.questions else {
            return;
        };
        loop {
            match questions.answers.try_recv() {
                Ok(Answer::Drive(name, health)) => {
                    if self.page == Page::Disk(name.clone()) {
                        self.disk_news = true;
                    }
                    self.waiting_drives.remove(&name);
                    // A read that fails keeps the last answer, as below.
                    match health {
                        Some(h) => {
                            self.drives.insert(name, Some(h));
                        }
                        None => {
                            self.drives.entry(name).or_insert(None);
                        }
                    }
                }
                Ok(Answer::Failed(failed)) => {
                    self.waiting_failed = false;
                    // A read that fails keeps the last answer: a slow bus is
                    // not a machine whose services all recovered.
                    if let Some(failed) = failed {
                        self.failed = failed;
                    }
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.lost_questions();
                    return;
                }
            }
        }
    }

    fn check(&self, tick: &Tick) -> Vec<Alert> {
        let graphics: Vec<Graphics> = tick
            .gpus
            .iter()
            .flatten()
            .map(|g| Graphics {
                name: &g.card.name,
                temperature: g.reading.temperature,
            })
            .collect();
        let disks: Vec<DiskSpace> = self
            .disks
            .iter()
            .zip(&self.space)
            .map(|(disk, &space)| DiskSpace { disk, space })
            .collect();
        let drives: Vec<Drive> = self
            .disks
            .iter()
            .filter_map(|d| {
                let health = self.drives.get(&d.name)?;
                Some(Drive {
                    name: d.label(),
                    health: health.as_ref()?,
                })
            })
            .collect();
        health::check(&Machine {
            cpu_temperature: tick.cpu.as_ref().and_then(|c| c.sample.temperature),
            graphics: &graphics,
            memory: tick.memory,
            disks: &disks,
            drives: &drives,
            failed_services: &self.failed,
        })
    }
}

/// A D-Bus question for the question thread.
#[derive(Debug)]
enum Ask {
    /// SMART for a whole disk.
    Drive(String),
    /// The failed system services.
    Failed,
}

#[derive(Debug)]
enum Answer {
    Drive(String, Option<smart::Health>),
    /// `None` when the read failed.
    Failed(Option<Vec<String>>),
}

/// The thread that asks udisks2 and systemd, one question at a time. Each
/// read gives up after its reader's deadline (4 s). The thread is not
/// joined: dropping this closes its channel and it ends after the read in
/// hand, so a hung daemon never holds up closing the window.
struct Questions {
    asks: Sender<Ask>,
    answers: Receiver<Answer>,
}

impl Questions {
    fn start() -> Self {
        let (asks, rx) = mpsc::channel();
        let (tx, answers) = mpsc::channel();
        // A thread that could not start drops `rx` and `tx` with its
        // closure, so the first send or receive finds it gone.
        let spawned = std::thread::Builder::new()
            .name("dbus-questions".into())
            .spawn(move || ask_dbus(rx, tx));
        if let Err(e) = spawned {
            log::error!("starting the D-Bus question thread: {e}");
        }
        Self { asks, answers }
    }
}

fn ask_dbus(asks: Receiver<Ask>, answers: Sender<Answer>) {
    // Made here: the readers' runtimes belong to this thread.
    let mut smart: Option<Option<SmartReader>> = None;
    let mut services: Option<Option<ServiceReader>> = None;
    for ask in asks {
        let answer = match ask {
            Ask::Drive(name) => {
                let reader = smart.get_or_insert_with(SmartReader::new);
                let health = reader.as_mut().and_then(|r| r.read(&name));
                Answer::Drive(name, health)
            }
            Ask::Failed => {
                let reader = services.get_or_insert_with(ServiceReader::new);
                Answer::Failed(reader.as_mut().and_then(ServiceReader::failed))
            }
        };
        if answers.send(answer).is_err() {
            return;
        }
    }
}

/// What the Qt thread tells the loop.
#[derive(Debug)]
enum Command {
    Page(Page),
    Interval(Duration),
}

/// The running loop. Dropping it stops the thread and waits for the tick in
/// hand, which reads only files: the D-Bus questions run elsewhere.
pub struct Loop {
    commands: Option<Sender<Command>>,
    thread: Option<JoinHandle<()>>,
}

impl Loop {
    /// Starts the thread. Nothing is read until the first [`Loop::set_page`].
    pub fn start(
        interval: Duration,
        sink: impl FnMut(Tick) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("sampler".into())
            .spawn(move || run(rx, interval, sink))?;
        Ok(Self {
            commands: Some(tx),
            thread: Some(thread),
        })
    }

    pub fn set_page(&self, page: Page) {
        self.send(Command::Page(page));
    }

    pub fn set_interval(&self, interval: Duration) {
        self.send(Command::Interval(interval));
    }

    fn send(&self, command: Command) {
        if let Some(tx) = &self.commands {
            // The thread only ends when this is dropped, or by a panic,
            // which the crash hook has already recorded.
            let _ = tx.send(command);
        }
    }
}

impl Drop for Loop {
    fn drop(&mut self) {
        // Closing the channel is the stop signal.
        self.commands = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(rx: Receiver<Command>, mut interval: Duration, mut sink: impl FnMut(Tick)) {
    let mut worker: Option<Worker> = None;
    let mut next: Option<Instant> = None;
    loop {
        let command = match next {
            Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match command {
            Ok(Command::Page(page)) => {
                let w = worker.get_or_insert_with(Worker::new);
                if *w.page() != page || next.is_none() {
                    w.set_page(page);
                    next = Some(Instant::now() + interval.min(FIRST_TICK));
                }
            }
            Ok(Command::Interval(d)) => {
                interval = d;
                if let Some(at) = &mut next {
                    *at = (*at).min(Instant::now() + interval);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                let Some(w) = worker.as_mut() else { continue };
                sink(w.tick());
                // Keep to the beat, but never try to catch up after a stall.
                let now = Instant::now();
                let mut at = next.unwrap_or(now) + interval;
                if at <= now {
                    at = now + interval;
                }
                next = Some(at);
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn pages_parse() {
        assert_eq!(Page::parse("overview"), Page::Overview);
        assert_eq!(Page::parse("cpu"), Page::Cpu);
        assert_eq!(Page::parse("disk:nvme0n1"), Page::Disk("nvme0n1".into()));
        assert_eq!(
            Page::parse("network:wlp4s0"),
            Page::Network("wlp4s0".into())
        );
        assert_eq!(Page::parse("battery:BAT0"), Page::Battery("BAT0".into()));
        // A device page needs its device.
        assert_eq!(Page::parse("disk:"), Page::Other);
        assert_eq!(Page::parse("disk"), Page::Other);
        assert_eq!(Page::parse("settings"), Page::Other);
        assert_eq!(Page::parse(""), Page::Other);
    }

    #[test]
    fn a_page_reads_only_what_it_shows() {
        let mut w = Worker::new();
        w.set_page(Page::Memory);
        let t = w.tick();
        assert!(t.fresh);
        assert!(t.cpu.is_none() && t.gpus.is_none() && t.health.is_none());
        assert!(t.sensors.is_none() && t.disk.is_none());
        // /proc/meminfo is there wherever the tests run.
        assert!(t.memory.is_some_and(|m| m.total > 0));
        // The device lists come with the first tick only.
        assert!(t.devices.disks.is_some() && t.devices.interfaces.is_some());
        let t = w.tick();
        assert!(!t.fresh);
        assert!(t.devices.disks.is_none() && t.devices.interfaces.is_none());

        w.set_page(Page::Other);
        let t = w.tick();
        assert!(t.fresh);
        assert!(t.memory.is_none() && t.cpu.is_none());
        // The sidebar reads on every page.
        assert_eq!(t.devices.disk_io.len(), w.disks.len());
    }

    #[test]
    fn the_cpu_page_says_what_the_processor_is_once() {
        let mut w = Worker::new();
        w.set_page(Page::Cpu);
        let first = w.tick().cpu.unwrap();
        assert!(first.info.is_some_and(|i| i.logical > 0));
        assert!((0.0..=100.0).contains(&first.sample.usage));
        assert!(w.tick().cpu.unwrap().info.is_none());
        // The Overview reads the load but not the facts.
        w.set_page(Page::Overview);
        assert!(w.tick().cpu.unwrap().info.is_none());
    }

    #[test]
    fn the_overview_says_what_is_wrong() {
        let mut w = Worker::new();
        w.set_page(Page::Overview);
        let t = w.tick();
        // Whatever this machine has, the list is there, critical first.
        let alerts = t.health.unwrap();
        assert!(alerts.windows(2).all(|p| p[0].level >= p[1].level));
        assert!(t.memory.is_some() && t.cpu.is_some() && t.gpus.is_some());
    }

    #[test]
    fn a_question_is_not_asked_twice_while_it_waits() {
        let mut w = Worker::new();
        // A question thread that never answers: a hung daemon.
        let (asks, sent) = mpsc::channel();
        let (_tx, answers) = mpsc::channel();
        w.questions = Some(Questions { asks, answers });
        w.set_page(Page::Overview);
        for _ in 0..(SLOW_EVERY * 3) {
            w.tick();
        }
        let drives = w.disks.iter().filter(|d| !d.is_swap).count();
        let asked: Vec<Ask> = sent.try_iter().collect();
        assert_eq!(asked.len(), drives + 1, "{asked:?}");
        // The ticks went on without the answers.
        assert_eq!(w.ticks, SLOW_EVERY * 3);
    }

    #[test]
    fn a_dead_question_thread_is_forgotten() {
        let mut w = Worker::new();
        let (asks, sent) = mpsc::channel();
        let (tx, answers) = mpsc::channel();
        w.questions = Some(Questions { asks, answers });
        w.set_page(Page::Overview);
        w.tick();
        assert!(w.waiting_failed);
        drop((sent, tx));
        w.tick();
        assert!(w.questions.is_none() && !w.waiting_failed && w.waiting_drives.is_empty());
    }

    #[test]
    fn answers_reach_the_disk_page_and_the_overview() {
        let mut w = Worker::new();
        let (asks, _sent) = mpsc::channel();
        let (tx, answers) = mpsc::channel();
        w.questions = Some(Questions { asks, answers });
        w.set_page(Page::Disk("sdz".into()));
        // The first tick says what is known: nothing yet.
        assert_eq!(
            w.tick().disk.unwrap().health.map(|h| h.is_none()),
            Some(true)
        );
        assert!(w.tick().disk.unwrap().health.is_none());
        tx.send(Answer::Drive("sdz".into(), None)).unwrap();
        assert!(w.tick().disk.unwrap().health.is_some(), "an answer is news");
        assert!(w.tick().disk.unwrap().health.is_none());

        tx.send(Answer::Failed(Some(vec!["a.service".into()])))
            .unwrap();
        w.tick();
        assert_eq!(w.failed, ["a.service"]);
        // A failed read keeps the last list.
        tx.send(Answer::Failed(None)).unwrap();
        w.tick();
        assert_eq!(w.failed, ["a.service"]);
    }

    #[test]
    fn the_loop_ticks_and_stops() {
        let ticks = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&ticks);
        let l = Loop::start(Duration::from_millis(20), move |t| {
            seen.lock().unwrap().push((t.fresh, t.page));
        })
        .unwrap();
        // Nothing is read before a page is set.
        std::thread::sleep(Duration::from_millis(60));
        assert!(ticks.lock().unwrap().is_empty());

        l.set_page(Page::Memory);
        std::thread::sleep(Duration::from_millis(150));
        l.set_page(Page::Other);
        std::thread::sleep(Duration::from_millis(150));
        drop(l);

        let ticks = ticks.lock().unwrap();
        assert!(ticks.len() >= 4, "{} ticks", ticks.len());
        assert_eq!(ticks[0], (true, Page::Memory));
        let switch = ticks.iter().position(|t| t.1 == Page::Other).unwrap();
        assert!(ticks[switch].0, "the new page's first tick is fresh");
        assert_eq!(ticks.iter().filter(|t| t.0).count(), 2);
    }

    #[test]
    fn dropping_the_loop_with_no_page_returns() {
        let l = Loop::start(Duration::from_secs(60), |_| {}).unwrap();
        let start = Instant::now();
        drop(l);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
