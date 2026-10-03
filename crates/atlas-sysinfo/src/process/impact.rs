//! A coarse power rating per process, like Task Manager's "Power usage".
//!
//! A heuristic, not a measurement: Linux has package energy (RAPL) but
//! nothing per process, so this blends the activity that costs power: time
//! on a core, time on the GPU, and the I/O that keeps the storage bus and the
//! radio awake.

use super::Proc;

/// The rating, ordered so a sort by it reads low to high. The label is the
/// UI's to choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Impact {
    VeryLow,
    Low,
    Moderate,
    High,
}

// One fully busy core scores 100; the other terms are how much of a core's
// worth of power each is judged to cost when saturated.
const GPU_WEIGHT: f64 = 0.8;
const DISK_WEIGHT: f64 = 25.0;
const NET_WEIGHT: f64 = 15.0;
/// Bytes/s at which the disk and network terms stop growing.
const DISK_SATURATE: f64 = (50 << 20) as f64;
const NET_SATURATE: f64 = (20 << 20) as f64;

const HIGH: f64 = 60.0; // about two thirds of a core, sustained
const MODERATE: f64 = 20.0;
const LOW: f64 = 2.0;

impl Proc {
    /// The weighted activity behind [`impact`](Self::impact), for sorting by
    /// the number rather than the label. Unknown figures count as idle.
    pub fn power_score(&self) -> f64 {
        let disk = self.disk_read.unwrap_or(0.0) + self.disk_write.unwrap_or(0.0);
        let net = self.net_in.unwrap_or(0.0) + self.net_out.unwrap_or(0.0);
        self.cpu
            + GPU_WEIGHT * self.gpu.unwrap_or(0.0).max(0.0)
            + DISK_WEIGHT * saturating(disk, DISK_SATURATE)
            + NET_WEIGHT * saturating(net, NET_SATURATE)
    }

    pub fn impact(&self) -> Impact {
        match self.power_score() {
            s if s >= HIGH => Impact::High,
            s if s >= MODERATE => Impact::Moderate,
            s if s >= LOW => Impact::Low,
            _ => Impact::VeryLow,
        }
    }
}

/// A rate as 0..=1 of `full`, flat beyond it.
fn saturating(rate: f64, full: f64) -> f64 {
    (rate / full).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: f64 = (1 << 20) as f64;

    fn proc(cpu: f64) -> Proc {
        Proc {
            pid: 1,
            start_time: 0,
            name: "test".into(),
            parent: 0,
            kernel: false,
            unit: None,
            container: None,
            cpu,
            memory: 0,
            gpu: None,
            net_in: None,
            net_out: None,
            disk_read: None,
            disk_write: None,
        }
    }

    #[test]
    fn ratings() {
        let with = |cpu, gpu, disk: f64, net| Proc {
            gpu,
            disk_read: Some(disk),
            disk_write: Some(disk),
            net_in: Some(net),
            ..proc(cpu)
        };
        for (name, p, want) in [
            ("idle", proc(0.0), Impact::VeryLow),
            ("a trickle of CPU", proc(0.4), Impact::VeryLow),
            ("a few percent", proc(5.0), Impact::Low),
            ("a quarter core", proc(25.0), Impact::Moderate),
            ("a whole core", proc(100.0), Impact::High),
            ("a parallel build", proc(780.0), Impact::High),
            (
                "video playback",
                with(15.0, Some(40.0), 0.0, 0.0),
                Impact::Moderate,
            ),
            ("a game", with(60.0, Some(95.0), 0.0, 0.0), Impact::High),
            (
                "a big file copy",
                with(8.0, None, 200.0 * MIB, 0.0),
                Impact::Moderate,
            ),
            (
                "a quiet download",
                with(1.0, None, 0.0, 2.0 * MIB),
                Impact::Low,
            ),
        ] {
            assert_eq!(p.impact(), want, "{name} (score {:.1})", p.power_score());
        }
    }

    /// A GPU client at 0% scores the same as a process with no GPU handle.
    #[test]
    fn absent_gpu_is_idle() {
        let client = Proc {
            gpu: Some(0.0),
            ..proc(30.0)
        };
        assert_eq!(client.power_score(), proc(30.0).power_score());
    }

    /// Enormous I/O never outranks several busy cores.
    #[test]
    fn io_saturates() {
        let gib = (1u64 << 30) as f64;
        let io = Proc {
            disk_read: Some(8.0 * gib),
            disk_write: Some(8.0 * gib),
            net_in: Some(4.0 * gib),
            ..proc(1.0)
        };
        assert!(io.power_score() < proc(400.0).power_score());
    }

    #[test]
    fn sorts_low_to_high() {
        assert!(Impact::VeryLow < Impact::Low);
        assert!(Impact::Low < Impact::Moderate);
        assert!(Impact::Moderate < Impact::High);
    }
}
