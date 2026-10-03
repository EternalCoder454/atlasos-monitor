//! A card's own hwmon node (`device/hwmon/hwmonN`): temperatures, fan,
//! power and clocks, each held open while the GPU page is.
//!
//! Drivers label what they report. amdgpu has `edge`, `junction` (the
//! hottest point on the die) and `mem`; xe has `pkg` and `vram`; nouveau
//! and older drivers have one unlabelled `temp1`. Power is an average or an
//! instant reading in µW, except on Intel's discrete cards, which only count
//! energy (µJ): there power is the energy used since the last tick.

use std::path::{Path, PathBuf};

use super::Gpu;
use crate::sensors::Energy;
use crate::sysfs::{self, HeldFile};

#[derive(Debug, Default)]
pub(super) struct Hwmon {
    temperature: Option<HeldFile>,
    hotspot: Option<HeldFile>,
    memory_temperature: Option<HeldFile>,
    fan_rpm: Option<HeldFile>,
    /// PWM duty and its maximum (255 unless the driver says), for a fan
    /// whose speed can't be read.
    pwm: Option<(HeldFile, u64)>,
    /// `power1_average`, then `power1_input`: some amdgpu firmware lists
    /// both and fails reads of the average.
    power: [Option<HeldFile>; 2],
    energy: Option<Energy>,
    core_clock: Option<HeldFile>,
    memory_clock: Option<HeldFile>,
}

impl Hwmon {
    /// Opens the card's hwmon attributes; an empty `Hwmon` without one.
    pub fn open(device: &Path) -> Self {
        let Some(dir) = dir(device) else {
            return Self::default();
        };
        let at = |name: &str| dir.join(name);
        let mut h = Self::default();
        for i in 1..=8 {
            let input = at(&format!("temp{i}_input"));
            if !input.exists() {
                continue;
            }
            let label = sysfs::read_string(at(&format!("temp{i}_label"))).unwrap_or_default();
            let slot = match label.to_ascii_lowercase().as_str() {
                "edge" | "gpu" | "pkg" => &mut h.temperature,
                "junction" | "hotspot" => &mut h.hotspot,
                "mem" | "vram" => &mut h.memory_temperature,
                // An unlabelled first sensor is the card's temperature.
                "" if i == 1 => &mut h.temperature,
                _ => continue,
            };
            if slot.is_none() {
                *slot = HeldFile::open(input);
            }
        }
        for i in 1..=4 {
            let label = sysfs::read_string(at(&format!("freq{i}_label"))).unwrap_or_default();
            let slot = match label.as_str() {
                "sclk" => &mut h.core_clock,
                "mclk" => &mut h.memory_clock,
                _ => continue,
            };
            *slot = HeldFile::open(at(&format!("freq{i}_input")));
        }
        h.fan_rpm = HeldFile::open(at("fan1_input"));
        h.pwm = HeldFile::open(at("pwm1")).map(|f| {
            let max = sysfs::read_uint(at("pwm1_max")).filter(|&m| m > 0);
            (f, max.unwrap_or(255))
        });
        h.power = [
            HeldFile::open(at("power1_average")),
            HeldFile::open(at("power1_input")),
        ];
        if h.power.iter().all(Option::is_none) {
            h.energy = HeldFile::open(at("energy1_input")).map(|file| Energy::new(file, true));
        }
        h
    }

    /// Fills in what this card reports.
    pub fn read(&mut self, g: &mut Gpu) {
        let celsius = |f: &mut Option<HeldFile>| Some(f.as_mut()?.uint()? as f64 / 1000.0);
        g.temperature = celsius(&mut self.temperature);
        g.hotspot = celsius(&mut self.hotspot);
        g.memory_temperature = celsius(&mut self.memory_temperature);
        g.fan_rpm = self.fan_rpm.as_mut().and_then(HeldFile::uint);
        // A duty cycle only for a fan that can't be counted, or whose count
        // fails to read (amdgpu, with the fan stopped on some cards).
        if g.fan_rpm.is_none() {
            g.fan_percent = self.pwm.as_mut().and_then(|(pwm, max)| {
                pwm.uint()
                    .map(|v| (v as f64 / *max as f64 * 100.0).min(100.0))
            });
        }
        g.power = match &mut self.energy {
            Some(e) => e.watts(),
            None => self
                .power
                .iter_mut()
                .find_map(|p| p.as_mut()?.uint())
                .map(|uw| uw as f64 / 1e6),
        };
        let mhz = |f: &mut Option<HeldFile>| Some(f.as_mut()?.uint()? as f64 / 1e6);
        g.core_clock = mhz(&mut self.core_clock);
        g.memory_clock = mhz(&mut self.memory_clock);
    }

    /// Forgets the energy baseline: the card slept, and power over the
    /// whole nap would read as a low figure for a card just woken.
    pub fn rest(&mut self) {
        if let Some(e) = &mut self.energy {
            e.rest();
        }
    }
}

/// The card's hwmon directory: the first under `device/hwmon`.
fn dir(device: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(device.join("hwmon"))
        .ok()?
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.as_encoded_bytes().starts_with(b"hwmon"))
        })
        .collect();
    dirs.sort();
    dirs.into_iter().next()
}

/// The power limit the card runs under, in watts: `power1_cap` (amdgpu,
/// nouveau) or `power1_max` (Intel's sustained limit). Read once.
pub(super) fn power_limit(device: &Path) -> Option<f64> {
    let dir = dir(device)?;
    sysfs::read_uint(dir.join("power1_cap"))
        .or_else(|| sysfs::read_uint(dir.join("power1_max")))
        .filter(|&uw| uw > 0)
        .map(|uw| uw as f64 / 1e6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn hwmon(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dev = tempfile::tempdir().unwrap();
        let dir = dev.path().join("hwmon/hwmon3");
        fs::create_dir_all(&dir).unwrap();
        for (name, value) in files {
            fs::write(dir.join(name), value).unwrap();
        }
        dev
    }

    fn read(dev: &tempfile::TempDir) -> Gpu {
        let mut g = Gpu::default();
        Hwmon::open(dev.path()).read(&mut g);
        g
    }

    /// amdgpu, as on a Radeon RX 7900 XTX.
    #[test]
    fn amdgpu() {
        let dev = hwmon(&[
            ("name", "amdgpu\n"),
            ("temp1_label", "edge\n"),
            ("temp1_input", "52000\n"),
            ("temp2_label", "junction\n"),
            ("temp2_input", "59000\n"),
            ("temp3_label", "mem\n"),
            ("temp3_input", "72000\n"),
            ("fan1_input", "545\n"),
            ("pwm1", "38\n"),
            ("pwm1_max", "255\n"),
            ("power1_average", "89000000\n"),
            ("power1_cap", "327000000\n"),
            ("freq1_label", "sclk\n"),
            ("freq1_input", "249000000\n"),
            ("freq2_label", "mclk\n"),
            ("freq2_input", "1249000000\n"),
        ]);
        let g = read(&dev);
        assert_eq!(g.temperature, Some(52.0));
        assert_eq!(g.hotspot, Some(59.0));
        assert_eq!(g.memory_temperature, Some(72.0));
        assert_eq!(g.fan_rpm, Some(545));
        // An RPM reading makes the duty cycle redundant.
        assert_eq!(g.fan_percent, None);
        assert_eq!(g.power, Some(89.0));
        assert_eq!(g.core_clock, Some(249.0));
        assert_eq!(g.memory_clock, Some(1249.0));
        assert_eq!(power_limit(dev.path()), Some(327.0));
    }

    /// nouveau and older drivers: one unlabelled temperature, a duty cycle,
    /// power as an instant reading.
    #[test]
    fn unlabelled() {
        let dev = hwmon(&[
            ("temp1_input", "45500\n"),
            ("temp2_input", "99000\n"),
            ("pwm1", "51\n"),
            ("power1_input", "15000000\n"),
        ]);
        let g = read(&dev);
        assert_eq!(g.temperature, Some(45.5));
        assert_eq!(g.hotspot, None);
        assert_eq!(g.memory_temperature, None);
        assert_eq!(g.fan_rpm, None);
        assert_eq!(g.fan_percent, Some(20.0));
        assert_eq!(g.power, Some(15.0));
        assert_eq!(g.core_clock, None);
        assert_eq!(power_limit(dev.path()), None);
    }

    /// A file that is listed but fails to read gives way to the next.
    #[test]
    fn unreadable_files() {
        let dev = hwmon(&[
            ("pwm1", "128\n"),
            ("pwm1_max", "255\n"),
            ("power1_input", "30000000\n"),
        ]);
        // A directory opens but fails every read, like a file whose driver
        // answers EOPNOTSUPP.
        let dir = dev.path().join("hwmon/hwmon3");
        fs::create_dir(dir.join("power1_average")).unwrap();
        fs::create_dir(dir.join("fan1_input")).unwrap();
        let g = read(&dev);
        assert_eq!(g.power, Some(30.0));
        assert_eq!(g.fan_rpm, None);
        assert!((g.fan_percent.unwrap() - 50.2).abs() < 0.1);
    }

    /// Intel's discrete cards count energy, not power.
    #[test]
    fn energy_counter() {
        let dev = hwmon(&[
            ("temp2_label", "pkg\n"),
            ("temp2_input", "40000\n"),
            ("temp3_label", "vram\n"),
            ("temp3_input", "38000\n"),
            ("energy1_input", "1000000\n"),
            ("power1_max", "190000000\n"),
        ]);
        let mut h = Hwmon::open(dev.path());
        let path = dev.path().join("hwmon/hwmon3/energy1_input");
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(&path, "3000000\n").unwrap();
        let mut g = Gpu::default();
        h.read(&mut g);
        assert_eq!(g.temperature, Some(40.0));
        assert_eq!(g.memory_temperature, Some(38.0));
        // 2 J in a bit over 50 ms.
        let w = g.power.unwrap();
        assert!(w > 0.0 && w <= 40.0, "{w}");
        // A counter that went back (a reset) is no reading, then a baseline.
        fs::write(&path, "5\n").unwrap();
        h.read(&mut g);
        assert_eq!(g.power, None);
        assert_eq!(power_limit(dev.path()), Some(190.0));
        // After a nap the next reading only sets the baseline.
        h.rest();
        h.read(&mut g);
        assert_eq!(g.power, None);
    }

    #[test]
    fn no_hwmon() {
        let dev = tempfile::tempdir().unwrap();
        assert_eq!(read(&dev), Gpu::default());
        assert_eq!(power_limit(dev.path()), None);
    }
}
