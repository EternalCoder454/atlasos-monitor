//! The kernel's per-client GPU counters: `/proc/<pid>/fdinfo/<fd>` of a
//! `/dev/dri` handle, in the drm-usage-stats format
//! (Documentation/gpu/drm-usage-stats.rst).
//!
//! Drivers count an engine's busy time one of two ways:
//!
//! - `drm-engine-<engine>: <ns> ns`, nanoseconds busy (amdgpu, i915,
//!   msm, panfrost, v3d...);
//! - `drm-cycles-<engine>` and `drm-total-cycles-<engine>`: GPU cycles busy
//!   and the GPU's own cycle counter read at the same moment (xe). Busy is
//!   the change in one over the change in the other.
//!
//! Either may come with `drm-engine-capacity-<engine>`: how many of that
//! engine there are (i915's and xe's video engines), so the busy figure can
//! reach that many times the elapsed time.
//!
//! This is the one parser for both users: the Apps table's per-process GPU
//! column ([`crate::process`]) and the GPU page's load on cards with no busy
//! file of their own ([`super::drm`]).

use crate::sysfs;

/// How many engines one client can list. amdgpu lists up to 9 (gfx,
/// compute, dma, dec, enc, enc_1, jpeg, vpe, ...), i915 5, xe 5.
pub const MAX_ENGINES: usize = 16;

/// Longest engine name kept; a longer one is cut, which only matters if two
/// names share their first 23 bytes.
const NAME: usize = 23;

/// One engine's counters in one client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Engine {
    name: [u8; NAME],
    len: u8,
    /// Nanoseconds busy (`drm-engine-<engine>`).
    pub ns: Option<u64>,
    /// GPU cycles busy (`drm-cycles-<engine>`).
    pub cycles: Option<u64>,
    /// The GPU's cycle counter when read (`drm-total-cycles-<engine>`).
    pub total_cycles: Option<u64>,
    /// How many of this engine there are; 1 when the driver doesn't say.
    pub capacity: u32,
}

impl Engine {
    const EMPTY: Engine = Engine {
        name: [0; NAME],
        len: 0,
        ns: None,
        cycles: None,
        total_cycles: None,
        capacity: 1,
    };

    /// The engine's name as the driver writes it: `gfx`, `render`, `rcs`...
    pub fn name(&self) -> &[u8] {
        &self.name[..usize::from(self.len)]
    }
}

/// One DRM client, from one fdinfo file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    /// `drm-client-id`: one client can be reachable through several
    /// descriptors (a dup, a handle passed over a socket), and must be
    /// counted once.
    pub id: Option<u64>,
    /// `drm-pdev`: the PCI device the client is on, to tell cards apart.
    pub pdev: Option<PciSlot>,
    engines: [Engine; MAX_ENGINES],
    count: u8,
}

impl Client {
    pub fn engines(&self) -> &[Engine] {
        &self.engines[..usize::from(self.count)]
    }

    /// The client's busy time summed over its engines, for the Apps table.
    /// An engine counted both ways (msm, panthor) is taken by its time.
    pub fn time(&self) -> GpuTime {
        let mut t = GpuTime::default();
        for e in self.engines() {
            match (e.ns, e.cycles, e.total_cycles) {
                (Some(ns), _, _) => t.ns = t.ns.saturating_add(ns),
                (None, Some(c), Some(total)) => {
                    t.cycles = t.cycles.saturating_add(c);
                    t.total_cycles = t.total_cycles.max(total);
                }
                _ => {}
            }
        }
        t
    }

    fn engine(&mut self, name: &[u8]) -> Option<&mut Engine> {
        let name = &name[..name.len().min(NAME)];
        let count = usize::from(self.count);
        if let Some(i) = self.engines[..count].iter().position(|e| e.name() == name) {
            return Some(&mut self.engines[i]);
        }
        if count == MAX_ENGINES {
            return None;
        }
        let e = &mut self.engines[count];
        e.name[..name.len()].copy_from_slice(name);
        e.len = name.len() as u8;
        self.count += 1;
        Some(e)
    }
}

/// A PCI device's address (`0000:03:00.0`) as one number, for comparing a
/// client's `drm-pdev` with a card's without strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PciSlot(u64);

impl PciSlot {
    /// Parses `domain:bus:device.function`, all hexadecimal. The domain can
    /// be longer than 4 digits (Intel VMD puts devices in `10000:`).
    pub fn parse(s: &[u8]) -> Option<Self> {
        let s = s.trim_ascii();
        let mut parts = s.splitn(3, |&c| c == b':');
        let (domain, bus, rest) = (parts.next()?, parts.next()?, parts.next()?);
        let dot = rest.iter().position(|&c| c == b'.')?;
        let (dev, func) = (&rest[..dot], &rest[dot + 1..]);
        let (domain, bus) = (hex(domain, 8)?, hex(bus, 2)?);
        let (dev, func) = (hex(dev, 2)?, hex(func, 1)?);
        if dev > 0x1f || func > 7 {
            return None;
        }
        Some(Self(domain << 16 | bus << 8 | dev << 3 | func))
    }

    /// On bus 0 of domain 0, the processor's own: where integrated devices
    /// sit, and never a card in a slot (that is behind a bridge).
    pub fn on_root_bus(self) -> bool {
        self.0 >> 8 == 0
    }
}

fn hex(s: &[u8], max_len: usize) -> Option<u64> {
    if s.is_empty() || s.len() > max_len {
        return None;
    }
    s.iter().try_fold(0u64, |v, &c| {
        Some(v << 4 | u64::from(char::from(c).to_digit(16)?))
    })
}

/// Busy time summed over engines, as the Apps table carries it from one
/// tick to the next.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GpuTime {
    /// Nanoseconds, from drivers that count time.
    pub ns: u64,
    /// Busy cycles, from drivers that count cycles.
    pub cycles: u64,
    /// The GPU's cycle counter at the same moment. One clock serves every
    /// engine, so this is the latest reading, not a sum.
    pub total_cycles: u64,
}

impl GpuTime {
    /// Adds another client's time: a process can hold several.
    pub fn add(&mut self, other: GpuTime) {
        self.ns = self.ns.saturating_add(other.ns);
        self.cycles = self.cycles.saturating_add(other.cycles);
        self.total_cycles = self.total_cycles.max(other.total_cycles);
    }

    /// Percent busy since `before`, `seconds` ago, 0..=100. Engines busy at
    /// once add up, as CPU time on several cores does, then the total is
    /// capped. A counter that went backwards (a client closed and another
    /// opened between reads) counts as idle.
    pub fn percent_since(&self, before: &GpuTime, seconds: f64) -> f64 {
        let mut busy = 0.0;
        if seconds > 0.0 && self.ns >= before.ns {
            busy += (self.ns - before.ns) as f64 / (seconds * 1e9);
        }
        // Without an earlier cycle reading (the process's first cycle-counting
        // client just appeared) the "change" would be its lifetime.
        let total = self.total_cycles.saturating_sub(before.total_cycles);
        if before.total_cycles > 0 && total > 0 && self.cycles >= before.cycles {
            busy += (self.cycles - before.cycles) as f64 / total as f64;
        }
        (busy * 100.0).clamp(0.0, 100.0)
    }
}

/// Parses a DRM handle's fdinfo. `None` when it has neither a client id
/// nor an engine counter: not a GPU client, or a driver that doesn't report
/// usage (the NVIDIA driver, nouveau before 6.x).
pub fn parse(b: &[u8]) -> Option<Client> {
    let mut c = Client {
        id: None,
        pdev: None,
        engines: [Engine::EMPTY; MAX_ENGINES],
        count: 0,
    };
    let mut found = false;
    for line in b.split(|&c| c == b'\n') {
        let Some(rest) = line.strip_prefix(b"drm-") else {
            continue;
        };
        let Some(colon) = rest.iter().position(|&c| c == b':') else {
            continue;
        };
        let (key, value) = (&rest[..colon], rest[colon + 1..].trim_ascii_start());
        if key == b"client-id" {
            c.id = sysfs::parse_uint(value);
            found |= c.id.is_some();
        } else if key == b"pdev" {
            c.pdev = PciSlot::parse(value);
        } else if let Some(name) = key.strip_prefix(b"engine-capacity-") {
            // Checked before drm-engine-: a count, not a time.
            if let (Some(n), Some(e)) = (sysfs::parse_uint(value), c.engine(name)) {
                e.capacity = u32::try_from(n).unwrap_or(u32::MAX).max(1);
            }
        } else if let Some(name) = key.strip_prefix(b"engine-") {
            if let (Some(ns), Some(e)) = (sysfs::parse_uint(value), c.engine(name)) {
                e.ns = Some(ns);
                found = true;
            }
        } else if let Some(name) = key.strip_prefix(b"total-cycles-") {
            if let (Some(n), Some(e)) = (sysfs::parse_uint(value), c.engine(name)) {
                e.total_cycles = Some(n);
            }
        } else if let Some(name) = key.strip_prefix(b"cycles-")
            && let (Some(n), Some(e)) = (sysfs::parse_uint(value), c.engine(name))
        {
            e.cycles = Some(n);
            found = true;
        }
    }
    found.then_some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AMDGPU: &[u8] = include_bytes!("../../tests/fixtures/fdinfo_amdgpu");
    const I915: &[u8] = include_bytes!("../../tests/fixtures/fdinfo_i915");
    /// An xe client in the layout of the kernel documentation's example,
    /// with some video engine time added.
    const XE: &[u8] = include_bytes!("../../tests/fixtures/fdinfo_xe");

    fn engine<'a>(c: &'a Client, name: &str) -> &'a Engine {
        c.engines()
            .iter()
            .find(|e| e.name() == name.as_bytes())
            .unwrap_or_else(|| panic!("no engine {name}"))
    }

    #[test]
    fn amdgpu() {
        let c = parse(AMDGPU).unwrap();
        assert_eq!(c.id, Some(64));
        assert_eq!(c.pdev, PciSlot::parse(b"0000:03:00.0"));
        assert_eq!(c.engines().len(), 2);
        assert_eq!(engine(&c, "gfx").ns, Some(5_715_782_407));
        assert_eq!(engine(&c, "compute").ns, Some(63_383_433));
        assert_eq!(c.time().ns, 5_715_782_407 + 63_383_433);
        assert_eq!(c.time().cycles, 0);
    }

    /// i915 lists how many of each engine there are beside the times; those
    /// counts are not nanoseconds.
    #[test]
    fn i915_capacities() {
        let c = parse(I915).unwrap();
        assert_eq!(c.id, Some(8));
        assert_eq!(engine(&c, "video").capacity, 2);
        assert_eq!(engine(&c, "video").ns, Some(88_000_123));
        assert_eq!(engine(&c, "render").capacity, 1);
        // video-enhance is its own engine, not video's.
        assert_eq!(engine(&c, "video-enhance").ns, Some(0));
        assert_eq!(c.time().ns, 3_124_890_011 + 88_000_123);
    }

    #[test]
    fn xe_cycles() {
        let c = parse(XE).unwrap();
        assert_eq!(c.id, Some(10));
        let rcs = engine(&c, "rcs");
        assert_eq!(rcs.cycles, Some(28_257_900));
        assert_eq!(rcs.total_cycles, Some(7_655_183_225));
        assert_eq!(rcs.ns, None);
        assert_eq!(engine(&c, "vcs").capacity, 2);
        assert_eq!(engine(&c, "ccs").capacity, 4);
        let t = c.time();
        assert_eq!(t.ns, 0);
        assert_eq!(t.cycles, 28_257_900 + 1_000);
        assert_eq!(t.total_cycles, 7_655_183_225);
    }

    #[test]
    fn without_usage() {
        assert_eq!(parse(b"pos:\t0\nflags:\t02\nmnt_id:\t24\n"), None);
        assert_eq!(parse(b""), None);
        // NVIDIA's driver: who and where, but no counters.
        assert_eq!(
            parse(b"drm-driver:\tnvidia-drm\ndrm-pdev:\t0000:01:00.0\n"),
            None
        );
        let c = parse(b"drm-engine-gfx:\t5 ns\n").unwrap();
        assert_eq!((c.id, c.time().ns), (None, 5));
        let c = parse(b"drm-client-id:\t3\n").unwrap();
        assert_eq!((c.id, c.engines().len()), (Some(3), 0));
        // A total with no busy count says nothing.
        assert_eq!(parse(b"drm-total-cycles-rcs:\t5\n"), None);
    }

    #[test]
    fn engine_limits() {
        let mut text = String::new();
        for i in 0..MAX_ENGINES + 4 {
            text.push_str(&format!("drm-engine-e{i}:\t{i} ns\n"));
        }
        let long = "x".repeat(40);
        text.push_str(&format!(
            "drm-engine-{long}:\t1 ns\ndrm-engine-{long}y:\t2 ns\n"
        ));
        let c = parse(text.as_bytes()).unwrap();
        assert_eq!(c.engines().len(), MAX_ENGINES);
        // Two names alike up to the cut are one engine; the later line wins.
        let mut c =
            parse(format!("drm-engine-{long}:\t1 ns\ndrm-engine-{long}y:\t2 ns\n").as_bytes())
                .unwrap();
        assert_eq!(c.engines().len(), 1);
        assert_eq!(c.engine(long.as_bytes()).unwrap().ns, Some(2));
        // A capacity of 0 would divide by zero later.
        let c = parse(b"drm-engine-capacity-v:\t0\ndrm-engine-v:\t1 ns\n").unwrap();
        assert_eq!(c.engines()[0].capacity, 1);
    }

    #[test]
    fn pci_slots() {
        let a = PciSlot::parse(b"0000:03:00.0").unwrap();
        assert_eq!(PciSlot::parse(b"0000:03:00.0\n"), Some(a));
        assert_ne!(PciSlot::parse(b"0000:03:00.1"), Some(a));
        assert_ne!(PciSlot::parse(b"0001:03:00.0"), Some(a));
        assert!(PciSlot::parse(b"10000:e1:00.0").is_some());
        assert!(PciSlot::parse(b"0000:00:1f.7").is_some());
        assert!(PciSlot::parse(b"0000:00:02.0").unwrap().on_root_bus());
        assert!(!a.on_root_bus());
        assert!(!PciSlot::parse(b"0001:00:02.0").unwrap().on_root_bus());
        for bad in [
            &b""[..],
            b"03:00.0",
            b"0000:03:00",
            b"0000:03:20.0",
            b"0000:03:00.8",
            b"0000:3g:00.0",
            b"0000:003:00.0",
            b"000000000:03:00.0",
        ] {
            assert_eq!(PciSlot::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn percentages() {
        let before = GpuTime {
            ns: 1_000_000_000,
            cycles: 100,
            total_cycles: 1_000,
        };
        // Half a second of engine time in one second.
        let ns = GpuTime {
            ns: 1_500_000_000,
            ..before
        };
        assert_eq!(ns.percent_since(&before, 1.0), 50.0);
        // A quarter of the cycles.
        let cy = GpuTime {
            cycles: 350,
            total_cycles: 2_000,
            ..before
        };
        assert_eq!(cy.percent_since(&before, 1.0), 25.0);
        // Engines add up, and are capped.
        let both = GpuTime {
            ns: 3_000_000_000,
            ..cy
        };
        assert_eq!(both.percent_since(&before, 1.0), 100.0);
        // Counters that went backwards, and no time at all.
        assert_eq!(before.percent_since(&ns, 1.0), 0.0);
        assert_eq!(ns.percent_since(&before, 0.0), 0.0);
        assert_eq!(before.percent_since(&before, 1.0), 0.0);
    }

    /// msm and panthor count an engine's time and its cycles: read once.
    #[test]
    fn counted_both_ways() {
        let c =
            parse(b"drm-engine-gpu:\t100 ns\ndrm-cycles-gpu:\t50\ndrm-total-cycles-gpu:\t500\n")
                .unwrap();
        assert_eq!(
            c.time(),
            GpuTime {
                ns: 100,
                ..GpuTime::default()
            }
        );
    }

    /// A process whose first cycle-counting client just appeared has no
    /// interval to measure it over.
    #[test]
    fn first_cycles_are_no_spike() {
        let now = GpuTime {
            cycles: 1_000_000,
            total_cycles: 5_000_000,
            ..GpuTime::default()
        };
        assert_eq!(now.percent_since(&GpuTime::default(), 1.0), 0.0);
    }

    #[test]
    fn times_add_up() {
        let mut t = GpuTime {
            ns: 5,
            cycles: 1,
            total_cycles: 10,
        };
        t.add(GpuTime {
            ns: 7,
            cycles: 2,
            total_cycles: 9,
        });
        assert_eq!(
            t,
            GpuTime {
                ns: 12,
                cycles: 3,
                total_cycles: 10
            }
        );
    }
}
