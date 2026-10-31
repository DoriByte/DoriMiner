//! Card sensors read through NVML, the management library the NVIDIA driver
//! installs (nvml.dll on Windows, libnvidia-ml.so.1 on Linux), loaded at run
//! time: without it the miner runs the same and only shows no temperatures.
//! Read only: nothing here changes a clock, a limit or a fan.

use std::ffi::{c_char, c_int, c_uint, c_void, CString};

type Device = *mut c_void;
const SUCCESS: c_int = 0;
const TEMPERATURE_GPU: c_uint = 0;
/// NVML_FI_DEV_MEMORY_TEMP: most GeForce cards answer "not supported".
const FIELD_MEMORY_TEMP: c_uint = 82;

#[repr(C)]
struct FieldValue {
    field_id: c_uint,
    scope_id: c_uint,
    timestamp: i64,
    latency_usec: i64,
    value_type: c_uint,
    status: c_int,
    value: u64,
}

/// What one card reads now; a field the card does not report is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    pub core_c: Option<u32>,
    pub memory_c: Option<u32>,
    pub watts: Option<f64>,
    pub fan_pct: Option<u32>,
}

pub struct Nvml {
    _lib: libloading::Library,
    by_pci: unsafe extern "C" fn(*const c_char, *mut Device) -> c_int,
    by_index: unsafe extern "C" fn(c_uint, *mut Device) -> c_int,
    temperature: unsafe extern "C" fn(Device, c_uint, *mut c_uint) -> c_int,
    power: unsafe extern "C" fn(Device, *mut c_uint) -> c_int,
    fan: unsafe extern "C" fn(Device, *mut c_uint) -> c_int,
    fields: unsafe extern "C" fn(Device, c_int, *mut FieldValue) -> c_int,
}

// SAFETY: NVML is documented as thread safe; the handles are plain pointers it owns.
unsafe impl Send for Nvml {}
unsafe impl Sync for Nvml {}

impl Nvml {
    /// The driver's NVML, initialised; `None` if it is not there.
    pub fn open() -> Option<Self> {
        let name = if cfg!(windows) { "nvml.dll" } else { "libnvidia-ml.so.1" };
        // SAFETY: the driver's own library, loaded by name from the system folders
        // (on Windows the search is limited to System32 before this runs), and
        // symbols with the signatures of the NVML headers.
        unsafe {
            let lib = libloading::Library::new(name).ok()?;
            let init: libloading::Symbol<unsafe extern "C" fn() -> c_int> = lib.get(b"nvmlInit_v2\0").ok()?;
            if init() != SUCCESS {
                return None;
            }
            let nvml = Nvml {
                by_pci: *lib.get(b"nvmlDeviceGetHandleByPciBusId_v2\0").ok()?,
                by_index: *lib.get(b"nvmlDeviceGetHandleByIndex_v2\0").ok()?,
                temperature: *lib.get(b"nvmlDeviceGetTemperature\0").ok()?,
                power: *lib.get(b"nvmlDeviceGetPowerUsage\0").ok()?,
                fan: *lib.get(b"nvmlDeviceGetFanSpeed\0").ok()?,
                fields: *lib.get(b"nvmlDeviceGetFieldValues\0").ok()?,
                _lib: lib,
            };
            Some(nvml)
        }
    }

    fn device(&self, pci_bus_id: Option<&str>, ordinal: usize) -> Option<Device> {
        let mut device: Device = std::ptr::null_mut();
        // SAFETY: NVML calls with a valid out pointer and a NUL-terminated string.
        unsafe {
            if let Some(pci) = pci_bus_id.and_then(|p| CString::new(p).ok()) {
                if (self.by_pci)(pci.as_ptr(), &mut device) == SUCCESS {
                    return Some(device);
                }
            }
            ((self.by_index)(ordinal as c_uint, &mut device) == SUCCESS).then_some(device)
        }
    }

    /// The card at this PCI address (or, failing that, this nvidia-smi index).
    pub fn read(&self, pci_bus_id: Option<&str>, ordinal: usize) -> Reading {
        let Some(device) = self.device(pci_bus_id, ordinal) else {
            return Reading::default();
        };
        let mut out = Reading::default();
        let mut value: c_uint = 0;
        // SAFETY: NVML calls on a handle it gave, with valid out pointers.
        unsafe {
            if (self.temperature)(device, TEMPERATURE_GPU, &mut value) == SUCCESS {
                out.core_c = Some(value);
            }
            if (self.power)(device, &mut value) == SUCCESS {
                out.watts = Some(value as f64 / 1000.0);
            }
            if (self.fan)(device, &mut value) == SUCCESS {
                out.fan_pct = Some(value);
            }
            let mut field = FieldValue { field_id: FIELD_MEMORY_TEMP, scope_id: 0, timestamp: 0, latency_usec: 0, value_type: 0, status: -1, value: 0 };
            if (self.fields)(device, 1, &mut field) == SUCCESS && field.status == SUCCESS {
                // unsigned int, unsigned long, unsigned long long or signed int: the low bits
                if matches!(field.value_type, 1 | 2 | 3 | 5) && (field.value as u32) < 200 {
                    out.memory_c = Some(field.value as u32);
                }
            }
        }
        out
    }
}
