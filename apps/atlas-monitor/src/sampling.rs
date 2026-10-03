//! The sampling loop, without Qt: one worker thread that reads what the page
//! on screen shows, every `refreshInterval`, and hands each [`Tick`] to a
//! sink. `sampler.rs` is the QObject around it; its sink posts the parts to
//! the stats objects on the Qt thread.
//!
//! - Only the page on screen is read. Its readers are made when it opens,
//!   which takes their baseline, and dropped when it closes, so a page opened
//!   again starts with empty charts rather than stale history.
//! - The sidebar's live values (processor and memory use, disk and network
//!   rates, battery charge, graphics load) are read on every page: a few
//!   held files and a power supply or two. Where the page reads the same
//!   thing, the sidebar shows the page's figure, so the two agree.
//!   A graphics card is read for the sidebar only if that can't keep it
//!   awake ([`Card::stays_awake`]).
//! - `statvfs` (disk space) and the address dump run on a page's first tick
//!   and every 5th after. SMART and failed services are D-Bus questions on
//!   slower timers of their own, asked on a second thread so a slow or hung
//!   daemon never holds up a tick or closing the window. A tick uses the
//!   latest answers.
//! - The worker owns every reader and takes no lock: commands arrive over a
//!   channel, whose wait is also the sleep between ticks.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use atlas_sysinfo::apps::{App, Group, GroupKey, Grouper, Resolver};
use atlas_sysinfo::gpu::{self, Card, Gpu, GpuSampler};
use atlas_sysinfo::health::{self, Alert, DiskSpace, Drive, Graphics, Machine};
use atlas_sysinfo::power::{self, PowerSampler, Supplies};
use atlas_sysinfo::process::{Proc, ProcessSampler, Wanted};
use atlas_sysinfo::sensors::{self, Device, Sensors};
use atlas_sysinfo::services::{Service, ServiceReader, Status};
use atlas_sysinfo::smart::{self, SmartReader};
use atlas_sysinfo::stats::cpu::{self, CpuInfo, CpuSample, CpuSampler};
use atlas_sysinfo::stats::disk::{self, Disk, DiskIo, DiskSampler, Space};
use atlas_sysinfo::stats::memory::{Memory, MemorySampler};
use atlas_sysinfo::stats::net::{self, Addresses, NetInterface, NetIo, NetSampler};

/// Disk space and addresses are read, and drives and interfaces listed
/// again, on this tick of every this many: every 5 s at the default
/// interval, every 50 s at the longest.
const SLOW_EVERY: u64 = 5;
/// udisks2 refreshes SMART every 10 minutes; once a minute is plenty.
const SMART_EVERY: Duration = Duration::from_secs(60);
/// A page's first reading comes this soon after it opens (or one interval,
/// if shorter): long enough for a fair CPU load, short enough not to show
/// an empty page at a 5 or 10 s interval.
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
    /// A card by its DRM node.
    Gpu(String),
    Battery(String),
    Sensors,
    Apps,
    Services,
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
            "gpu" if !device.is_empty() => Self::Gpu(device),
            "battery" if !device.is_empty() => Self::Battery(device),
            "sensors" => Self::Sensors,
            "apps" => Self::Apps,
            "services" => Self::Services,
            _ => Self::Other,
        }
    }
}

/// What the sidebar shows on every page.
#[derive(Debug, Clone, Default)]
pub struct Devices {
    /// Whole disks, root first, swap last. Sent with a fresh tick only.
    pub disks: Option<Vec<Disk>>,
    /// Network interfaces. Sent with a fresh tick, and with a slow one that
    /// finds an adapter plugged in or out.
    pub interfaces: Option<Vec<NetInterface>>,
    /// Graphics cards, the one most worth showing first. Sent with a fresh
    /// tick only.
    pub cards: Option<Vec<Card>>,
    /// Throughput per disk, in the order of the list.
    pub disk_io: Vec<DiskIo>,
    /// Throughput per interface present now.
    pub net_io: Vec<NetIo>,
    /// The interface carrying the default route.
    #[allow(dead_code, reason = "the Network page will mark it; drop this then")]
    pub default_route: Option<String>,
    /// Batteries and adapters; `None` on a machine with none.
    pub power: Option<Supplies>,
    /// Whole-processor load, percent.
    pub cpu_usage: Option<f64>,
    /// Memory in use, percent.
    pub memory_usage: Option<f64>,
    /// Each card's load, percent, in the order of the cards; `None` for a
    /// card left unread so it can sleep.
    pub gpu_usages: Vec<Option<f64>>,
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
    /// Watts; `None` until the card has been awake.
    pub power_limit: Option<f64>,
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
    pub apps: Option<AppsTick>,
    /// The Services page's list when a read has come in since the last
    /// tick; `Some(None)` when systemd didn't answer.
    pub services: Option<Option<Vec<Service>>>,
}

/// The Apps table's processes. `keys` and `apps` run parallel to `procs`:
/// each process's grouped row and the application it belongs to.
#[derive(Debug, Clone, Default)]
pub struct AppsTick {
    pub procs: Vec<Proc>,
    pub keys: Vec<GroupKey>,
    pub apps: Vec<Option<Arc<App>>>,
    /// One per application (or name), in the order of their first members.
    pub groups: Vec<Group>,
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
    /// The sidebar's own readers, kept for the whole run (one made as a
    /// page closes would measure its first load over a moment) and read
    /// every tick. Where the page reads the same thing, its figure is shown,
    /// so the sidebar and the page agree.
    side_cpu: CpuSampler,
    side_memory: MemorySampler,
    /// Per card, in the order of `cards`: the load, for a card reading
    /// can't keep awake.
    side_gpus: Vec<Option<GpuSampler>>,

    // Read for the page on screen; `None` on other pages.
    cpu: Option<CpuSampler>,
    memory: Option<MemorySampler>,
    gpus: Option<Vec<(Card, GpuSampler)>>,
    sensors: Option<Sensors>,
    procs: Option<ProcessSampler>,

    // The Apps table's grouping, kept across visits: its caches (desktop
    // files, icons) are what make a tick cheap.
    resolver: Option<Resolver>,
    grouper: Grouper,
    /// Qt's icon theme, for the resolver's icon lookups.
    icon_theme: String,
    kernel_threads: bool,

    // Read once, kept.
    cpu_info: Option<CpuInfo>,
    cards: Vec<Card>,

    // Slow figures, kept between their reads.
    space: Vec<Option<Space>>,
    questions: Option<Questions>,
    /// When each drive was last asked about, and its latest answer.
    asked: HashMap<String, Instant>,
    drives: HashMap<String, Option<smart::Health>>,
    /// Questions sent and not answered yet: none is asked twice at once.
    waiting_drives: HashSet<String>,
    /// Questions in flight about a drive that has since left or been
    /// swapped for another under its name: their answers are dropped.
    stale_drives: HashSet<String>,
    waiting_failed: bool,
    failed: Vec<String>,
    waiting_services: bool,
    /// The services list's unit files must be read again (after an action).
    services_stale: bool,
    /// The list being read was asked for with the files read again: if the
    /// question is lost, the next one must be too.
    services_asked_stale: bool,
    /// An action finished while a list was being read: that list may be
    /// from before it, and is dropped.
    services_outdated: bool,
    services_news: Option<Option<Vec<Service>>>,
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
            side_cpu: CpuSampler::load_only(),
            side_memory: MemorySampler::new(),
            side_gpus: Vec::new(),
            cpu: None,
            memory: None,
            gpus: None,
            sensors: None,
            procs: None,
            resolver: None,
            grouper: Grouper::default(),
            icon_theme: String::new(),
            kernel_threads: false,
            cpu_info: None,
            cards: gpu::cards(),
            questions: None,
            asked: HashMap::new(),
            drives: HashMap::new(),
            waiting_drives: HashSet::new(),
            stale_drives: HashSet::new(),
            waiting_failed: false,
            failed: Vec::new(),
            waiting_services: false,
            services_stale: false,
            services_asked_stale: false,
            services_outdated: false,
            services_news: None,
            disk_news: false,
        }
    }

    pub fn page(&self) -> &Page {
        &self.page
    }

    /// Qt's icon theme, which the Apps table's icons are looked up in.
    pub fn set_icon_theme(&mut self, theme: &str) {
        theme.clone_into(&mut self.icon_theme);
        if let Some(r) = &mut self.resolver {
            r.set_icon_theme(theme);
        }
    }

    /// Kernel threads as rows of the Apps table.
    pub fn set_kernel_threads(&mut self, on: bool) {
        self.kernel_threads = on;
        let wanted = self.wanted();
        if let Some(p) = &mut self.procs {
            p.set_wanted(wanted);
        }
    }

    /// A service was acted on: the next list reads the unit files again,
    /// for whether each starts at boot.
    pub fn services_changed(&mut self) {
        self.services_stale = true;
        self.services_outdated = self.waiting_services;
        // An answer taken in but not passed on yet is from before it too.
        self.services_news = None;
    }

    fn wanted(&self) -> Wanted {
        Wanted {
            kernel_threads: self.kernel_threads,
            gpu: !self.cards.is_empty(),
            ..Wanted::default()
        }
    }

    /// Makes the readers `page` needs, dropping the ones it doesn't. The
    /// sidebar's readers are kept, so its rates don't restart.
    pub fn set_page(&mut self, page: Page) {
        self.cpu = matches!(page, Page::Overview | Page::Cpu).then(CpuSampler::new);
        self.memory = matches!(page, Page::Overview | Page::Memory).then(MemorySampler::new);
        // The Overview reads every card; a GPU page only its own, so a
        // laptop's other card can still sleep. Each sampler gets the whole
        // list, to know whether its card is the only one.
        let shown = |c: &Card| match &page {
            Page::Overview => true,
            Page::Gpu(node) => c.node == *node,
            _ => false,
        };
        self.gpus = matches!(page, Page::Overview | Page::Gpu(_)).then(|| {
            self.cards
                .iter()
                .filter(|c| shown(c))
                .map(|c| (c.clone(), GpuSampler::new(c, &self.cards)))
                .collect()
        });
        self.sensors = (page == Page::Sensors && sensors::available()).then(Sensors::new);
        self.procs = (page == Page::Apps).then(|| ProcessSampler::new(self.wanted()));
        if page == Page::Cpu && self.cpu_info.is_none() {
            self.cpu_info = Some(cpu::info());
        }
        self.page = page;
        self.fresh = true;
        self.ticks = 0;
    }

    /// Makes or drops the sidebar's card readers: one for each card that
    /// stays awake anyway. A screen turned on or off changes that.
    fn refresh_side_gpus(&mut self) {
        self.side_gpus.resize_with(self.cards.len(), || None);
        for (i, card) in self.cards.iter().enumerate() {
            let wanted = card.stays_awake();
            match (&self.side_gpus[i], wanted) {
                (None, true) => self.side_gpus[i] = Some(GpuSampler::load_only(card, &self.cards)),
                (Some(_), false) => self.side_gpus[i] = None,
                _ => {}
            }
        }
    }

    /// Reads everything the page shows.
    pub fn tick(&mut self) -> Tick {
        let fresh = std::mem::take(&mut self.fresh);
        let slow = self.ticks.is_multiple_of(SLOW_EVERY);
        self.ticks += 1;
        self.take_answers();

        // A USB adapter plugged in or out joins or leaves the sidebar within
        // a slow tick.
        let mut plugged = false;
        if slow {
            let now = net::interfaces();
            plugged = now
                .iter()
                .map(|i| &i.name)
                .ne(self.interfaces.iter().map(|i| &i.name));
            if plugged {
                self.interfaces = now;
            }
        }

        // A drive plugged in or out, or a filesystem mounted on one, likewise.
        let mut disks_changed = false;
        if slow {
            let now = disk::disks();
            if now != self.disks {
                self.replace_disks(now);
                disks_changed = true;
            }
        }

        if slow {
            self.refresh_side_gpus();
        }

        // A battery that turns up later (a dock, a pack put back in) gets
        // its sidebar entry within a slow tick.
        if self.power.is_none() && slow && power::available() {
            self.power = Some(PowerSampler::new());
        }

        let devices = Devices {
            disks: (fresh || disks_changed).then(|| self.disks.clone()),
            interfaces: (fresh || plugged).then(|| self.interfaces.clone()),
            cards: fresh.then(|| self.cards.clone()),
            disk_io: self.disk_io.sample().to_vec(),
            net_io: self.net_io.sample().to_vec(),
            default_route: self.net_io.default_route().map(str::to_owned),
            power: self.power.as_mut().map(|p| p.sample().clone()),
            ..Devices::default()
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
                    power_limit: s.power_limit(),
                })
                .collect()
        });
        let sensors = self.sensors.as_mut().map(|s| s.sample().to_vec());
        let mut devices = devices;
        // The sidebar's own readers are read every tick, so their next
        // figure is never an average over a page that read the same thing.
        let side_cpu = self.side_cpu.sample().usage;
        devices.cpu_usage = Some(cpu.as_ref().map_or(side_cpu, |c| c.sample.usage));
        devices.memory_usage = memory.as_ref().map(Memory::usage_percent).or_else(|| {
            self.side_memory
                .sample()
                .as_ref()
                .map(Memory::usage_percent)
        });
        devices.gpu_usages = self
            .cards
            .iter()
            .zip(&mut self.side_gpus)
            .map(|(card, side)| {
                let on_page = gpus
                    .as_ref()
                    .and_then(|g: &Vec<GpuTick>| g.iter().find(|t| t.card.node == card.node));
                let side = side.as_mut().and_then(|s| s.sample().usage);
                on_page.map_or(side, |t| t.reading.usage)
            })
            .collect();

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
                    disk: (fresh || disks_changed)
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
            Page::Apps => tick.apps = self.read_apps(),
            Page::Services => {
                // The list is read on the question thread; a tick passes on
                // the latest that has come in.
                if !self.waiting_services {
                    let stale = std::mem::take(&mut self.services_stale);
                    self.services_asked_stale = stale;
                    self.ask(Ask::Services(stale));
                }
                tick.services = self.services_news.take();
            }
            _ => {}
        }
        tick
    }

    /// Every process, with its application and grouped row.
    fn read_apps(&mut self) -> Option<AppsTick> {
        let procs = self.procs.as_mut()?.sample().to_vec();
        let theme = &self.icon_theme;
        let resolver = self
            .resolver
            .get_or_insert_with(|| Resolver::for_session(theme));
        let groups = self.grouper.group(&procs, resolver).to_vec();
        let keys = procs.iter().map(|p| resolver.key_of(p)).collect();
        let apps = procs.iter().map(|p| resolver.of_proc(p).cloned()).collect();
        Some(AppsTick {
            procs,
            keys,
            apps,
            groups,
        })
    }

    /// Takes a new disk list. Rates and free space are kept for the same
    /// drive ([`Disk::same_drive`]); a new one's space is read on the next
    /// slow tick that reads space, and a drive that left, or was swapped for
    /// another under its name, loses its SMART answer and is asked again.
    fn replace_disks(&mut self, now: Vec<Disk>) {
        self.disk_io.set_disks(&now);
        self.space = now
            .iter()
            .map(|d| {
                let old = self.disks.iter().position(|o| o.same_drive(d));
                old.and_then(|i| self.space[i])
            })
            .collect();
        for old in &self.disks {
            if !now.iter().any(|d| d.same_drive(old)) {
                self.drives.remove(&old.name);
                self.asked.remove(&old.name);
                if self.waiting_drives.contains(&old.name) {
                    self.stale_drives.insert(old.name.clone());
                }
            }
        }
        self.disks = now;
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
            Ask::Services(_) => std::mem::replace(&mut self.waiting_services, true),
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
        self.stale_drives.clear();
        self.waiting_failed = false;
        self.waiting_services = false;
        // The reader forgets its unit files only when the question reaches
        // it: a lost one still owes that.
        self.services_stale |= std::mem::take(&mut self.services_asked_stale);
        self.services_outdated = false;
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
                    if self.stale_drives.remove(&name) {
                        // About the drive that was there before: the next
                        // tick asks about this one.
                        continue;
                    }
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
                Ok(Answer::Services(list)) => {
                    self.waiting_services = false;
                    self.services_asked_stale = false;
                    if std::mem::take(&mut self.services_outdated) {
                        continue;
                    }
                    if let Some(list) = &list {
                        let mut failed: Vec<String> = list
                            .iter()
                            .filter(|s| s.status() == Status::Failed)
                            .map(|s| s.name.clone())
                            .collect();
                        failed.sort_unstable();
                        failed.dedup();
                        self.failed = failed;
                    }
                    self.services_news = Some(list);
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
    /// Every service, the unit files read again if true.
    Services(bool),
}

#[derive(Debug)]
enum Answer {
    Drive(String, Option<smart::Health>),
    /// `None` when the read failed.
    Failed(Option<Vec<String>>),
    Services(Option<Vec<Service>>),
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
            Ask::Services(stale) => {
                let reader = services.get_or_insert_with(ServiceReader::new);
                Answer::Services(reader.as_mut().and_then(|r| {
                    if stale {
                        r.invalidate();
                    }
                    r.list()
                }))
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
    IconTheme(String),
    KernelThreads(bool),
    ServicesChanged,
    Paused(bool),
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

    pub fn set_icon_theme(&self, theme: String) {
        self.send(Command::IconTheme(theme));
    }

    pub fn set_kernel_threads(&self, on: bool) {
        self.send(Command::KernelThreads(on));
    }

    pub fn services_changed(&self) {
        self.send(Command::ServicesChanged);
    }

    /// Stops ticking while the window can't be seen, and starts again with
    /// a fresh tick.
    pub fn set_paused(&self, paused: bool) {
        self.send(Command::Paused(paused));
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
    let mut paused = false;
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
                    if !paused {
                        next = Some(Instant::now() + interval.min(FIRST_TICK));
                    }
                }
            }
            Ok(Command::IconTheme(theme)) => {
                worker
                    .get_or_insert_with(Worker::new)
                    .set_icon_theme(&theme);
            }
            Ok(Command::KernelThreads(on)) => {
                worker
                    .get_or_insert_with(Worker::new)
                    .set_kernel_threads(on);
            }
            Ok(Command::ServicesChanged) => {
                worker.get_or_insert_with(Worker::new).services_changed();
            }
            Ok(Command::Paused(p)) if p != paused => {
                paused = p;
                // Back on screen, a tick at once: its rates are averages
                // over the pause, as after any slow tick.
                next = (!paused && worker.is_some()).then(Instant::now);
            }
            Ok(Command::Paused(_)) => {}
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
        assert_eq!(Page::parse("gpu:card1"), Page::Gpu("card1".into()));
        assert_eq!(Page::parse("gpu"), Page::Other);
        // A device page needs its device.
        assert_eq!(Page::parse("disk:"), Page::Other);
        assert_eq!(Page::parse("disk"), Page::Other);
        assert_eq!(Page::parse("apps"), Page::Apps);
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
        assert!(t.devices.cards.is_some());
        let t = w.tick();
        assert!(!t.fresh);
        assert!(t.devices.disks.is_none() && t.devices.interfaces.is_none());
        assert!(t.devices.cards.is_none());

        w.set_page(Page::Other);
        let t = w.tick();
        assert!(t.fresh);
        assert!(t.memory.is_none() && t.cpu.is_none());
        // The sidebar reads on every page.
        assert_eq!(t.devices.disk_io.len(), w.disks.len());
    }

    #[test]
    fn the_sidebar_figures_come_on_every_page() {
        let mut w = Worker::new();
        let percent = |v: Option<f64>| v.is_some_and(|v| (0.0..=100.0).contains(&v));
        for page in [
            Page::Other,
            Page::Cpu,
            Page::Memory,
            Page::Overview,
            Page::Apps,
        ] {
            w.set_page(page.clone());
            for _ in 0..2 {
                let t = w.tick();
                assert!(percent(t.devices.cpu_usage), "{page:?}");
                assert!(percent(t.devices.memory_usage), "{page:?}");
                assert_eq!(t.devices.gpu_usages.len(), w.cards.len(), "{page:?}");
                // The page's figure is the sidebar's: one reading, not two.
                if let Some(c) = &t.cpu {
                    assert_eq!(t.devices.cpu_usage, Some(c.sample.usage));
                }
                if let Some(m) = &t.memory {
                    assert_eq!(t.devices.memory_usage, Some(m.usage_percent()));
                }
            }
        }
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
    fn a_paused_loop_reads_nothing_and_resumes_at_once() {
        let ticks = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&ticks);
        let l = Loop::start(Duration::from_secs(60), move |t| {
            seen.lock().unwrap().push(t.page);
        })
        .unwrap();
        l.set_paused(true);
        // A page opened while hidden waits for the window to show.
        l.set_page(Page::Memory);
        std::thread::sleep(Duration::from_millis(100));
        assert!(ticks.lock().unwrap().is_empty());

        // Shown again: a tick now, not a minute away.
        l.set_paused(false);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(*ticks.lock().unwrap(), [Page::Memory]);
        drop(l);
    }

    #[test]
    fn dropping_the_loop_with_no_page_returns() {
        let l = Loop::start(Duration::from_secs(60), |_| {}).unwrap();
        let start = Instant::now();
        drop(l);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn apps_page_lists_processes_with_their_rows() {
        let mut w = Worker::new();
        w.set_page(Page::Apps);
        let t = w.tick().apps.expect("the Apps page reads processes");
        // This test's own process is always there.
        assert!(t.procs.iter().any(|p| p.pid == std::process::id()));
        assert_eq!(t.keys.len(), t.procs.len());
        assert_eq!(t.apps.len(), t.procs.len());
        let members: u32 = t.groups.iter().map(|g| g.count).sum();
        assert_eq!(members as usize, t.procs.len());
        // Off the page, nothing is read.
        w.set_page(Page::Cpu);
        assert!(w.tick().apps.is_none());
    }
}
