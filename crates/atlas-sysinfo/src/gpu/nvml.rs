//! NVIDIA cards through NVML, the library that ships with NVIDIA's driver.
//! The driver says almost nothing through sysfs or fdinfo, so this is the
//! only way to its load, memory, temperature, fan, power and clocks.
//!
//! AtlasOS ships no NVIDIA driver, so the library is loaded at run time with
//! `dlopen`, and only for a card bound to the `nvidia` driver: someone who
//! layered it. Nothing here is a build dependency. Every entry point but the
//! first three is optional; one that is missing leaves its figure out.

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};

use super::Gpu;

type Device = *mut c_void;
/// `nvmlReturn_t`: 0 is success.
type Ret = c_int;

#[repr(C)]
#[derive(Default)]
struct Utilization {
    gpu: c_uint,
    memory: c_uint,
}

#[repr(C)]
#[derive(Default)]
struct Memory {
    total: u64,
    free: u64,
    used: u64,
}

/// `NVML_TEMPERATURE_GPU`, `NVML_CLOCK_GRAPHICS`, `NVML_CLOCK_MEM`.
const TEMPERATURE_GPU: c_int = 0;
const CLOCK_GRAPHICS: c_int = 0;
const CLOCK_MEM: c_int = 2;

/// NVML loaded, initialised, and holding one card.
pub(super) struct Nvml {
    lib: *mut c_void,
    device: Device,
    shutdown: unsafe extern "C" fn() -> Ret,
    utilization: Option<unsafe extern "C" fn(Device, *mut Utilization) -> Ret>,
    memory: Option<unsafe extern "C" fn(Device, *mut Memory) -> Ret>,
    temperature: Option<unsafe extern "C" fn(Device, c_int, *mut c_uint) -> Ret>,
    fan: Option<unsafe extern "C" fn(Device, *mut c_uint) -> Ret>,
    power: Option<unsafe extern "C" fn(Device, *mut c_uint) -> Ret>,
    clock: Option<unsafe extern "C" fn(Device, c_int, *mut c_uint) -> Ret>,
    power_limit: Option<unsafe extern "C" fn(Device, *mut c_uint) -> Ret>,
}

// SAFETY: NVML is thread-safe, and the sampler using it lives on one thread.
unsafe impl Send for Nvml {}

impl std::fmt::Debug for Nvml {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Nvml")
    }
}

/// Looks `name` up in `lib` as a function of type `F`.
///
/// SAFETY: `F` must be the symbol's real signature.
unsafe fn symbol<F: Copy>(lib: *mut c_void, name: &CStr) -> Option<F> {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    // SAFETY: `lib` is a live handle; a null result is checked.
    let p = unsafe { libc::dlsym(lib, name.as_ptr()) };
    // SAFETY: function pointers and data pointers are the same size on every
    // target Linux runs on, and the caller vouches for the signature.
    (!p.is_null()).then(|| unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p) })
}

impl Nvml {
    /// Loads NVML and finds the card at `slot` (`0000:01:00.0`). `None` if
    /// the library is missing, won't start, or doesn't know the card.
    pub fn open(slot: &str) -> Option<Self> {
        let bus_id = CString::new(slot).ok()?;
        let lib = ["libnvidia-ml.so.1", "libnvidia-ml.so"]
            .iter()
            .find_map(|name| {
                let name = CString::new(*name).ok()?;
                // SAFETY: a plain library load; a null result is checked.
                let lib =
                    unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
                (!lib.is_null()).then_some(lib)
            })?;
        let close = |lib| {
            // SAFETY: `lib` came from dlopen and nothing from it is kept.
            unsafe { libc::dlclose(lib) };
            None
        };
        // SAFETY: each type below is the NVML function's documented signature.
        let (init, by_bus, shutdown) = unsafe {
            (
                symbol::<unsafe extern "C" fn() -> Ret>(lib, c"nvmlInit_v2"),
                symbol::<unsafe extern "C" fn(*const c_char, *mut Device) -> Ret>(
                    lib,
                    c"nvmlDeviceGetHandleByPciBusId_v2",
                ),
                symbol::<unsafe extern "C" fn() -> Ret>(lib, c"nvmlShutdown"),
            )
        };
        let (Some(init), Some(by_bus), Some(shutdown)) = (init, by_bus, shutdown) else {
            return close(lib);
        };
        // SAFETY: NVML's own initialisation, paired with `shutdown` on drop.
        if unsafe { init() } != 0 {
            return close(lib);
        }
        let mut device: Device = std::ptr::null_mut();
        // SAFETY: a NUL-terminated bus ID and a pointer to write the handle to.
        if unsafe { by_bus(bus_id.as_ptr(), &mut device) } != 0 || device.is_null() {
            // SAFETY: initialised above.
            unsafe { shutdown() };
            return close(lib);
        }
        // SAFETY: as above, documented signatures.
        unsafe {
            Some(Self {
                lib,
                device,
                shutdown,
                utilization: symbol(lib, c"nvmlDeviceGetUtilizationRates"),
                memory: symbol(lib, c"nvmlDeviceGetMemoryInfo"),
                temperature: symbol(lib, c"nvmlDeviceGetTemperature"),
                fan: symbol(lib, c"nvmlDeviceGetFanSpeed"),
                power: symbol(lib, c"nvmlDeviceGetPowerUsage"),
                clock: symbol(lib, c"nvmlDeviceGetClockInfo"),
                power_limit: symbol(lib, c"nvmlDeviceGetEnforcedPowerLimit"),
            })
        }
    }

    /// Fills in what NVML reports.
    pub fn read(&mut self, g: &mut Gpu) {
        let d = self.device;
        // SAFETY (every call below): a live handle from `open`, and
        // out-pointers to values of the documented types.
        if let Some(f) = self.utilization {
            let mut u = Utilization::default();
            if unsafe { f(d, &mut u) } == 0 {
                g.usage = Some(f64::from(u.gpu).min(100.0));
            }
        }
        if let Some(f) = self.memory {
            let mut m = Memory::default();
            if unsafe { f(d, &mut m) } == 0 {
                g.memory_used = Some(m.used);
                g.memory_total = Some(m.total);
            }
        }
        let uint = |f: Option<unsafe extern "C" fn(Device, *mut c_uint) -> Ret>| {
            let mut v: c_uint = 0;
            (unsafe { f?(d, &mut v) } == 0).then_some(v)
        };
        let uint_of = |f: Option<unsafe extern "C" fn(Device, c_int, *mut c_uint) -> Ret>,
                       which: c_int| {
            let mut v: c_uint = 0;
            (unsafe { f?(d, which, &mut v) } == 0).then_some(v)
        };
        g.temperature = uint_of(self.temperature, TEMPERATURE_GPU).map(f64::from);
        g.fan_percent = uint(self.fan).map(|p| f64::from(p).min(100.0));
        g.power = uint(self.power).map(|mw| f64::from(mw) / 1000.0);
        g.core_clock = uint_of(self.clock, CLOCK_GRAPHICS).map(f64::from);
        g.memory_clock = uint_of(self.clock, CLOCK_MEM).map(f64::from);
    }

    /// The power limit in force, in watts.
    pub fn power_limit(&self) -> Option<f64> {
        let f = self.power_limit?;
        let mut mw: c_uint = 0;
        // SAFETY: as in `read`.
        (unsafe { f(self.device, &mut mw) } == 0 && mw > 0).then(|| f64::from(mw) / 1000.0)
    }
}

impl Drop for Nvml {
    fn drop(&mut self) {
        // SAFETY: initialised in `open`; NVML counts its users, so this only
        // undoes ours. Nothing from the library outlives `self`.
        unsafe {
            (self.shutdown)();
            libc::dlclose(self.lib);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without NVIDIA's driver (CI, AMD and Intel machines) there is
    /// nothing to open, and asking costs nothing worse than a failed load.
    #[test]
    fn open_without_a_card() {
        assert!(Nvml::open("ffff:ff:1f.7").is_none());
        assert!(Nvml::open("bad\0slot").is_none());
    }
}
