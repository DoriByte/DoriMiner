use std::ffi::{c_char, c_int, c_uint, c_void, CString};

type Device = *mut c_void;
const SUCCESS: c_int = 0;
const TEMPERATURE_GPU: c_uint = 0;

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

unsafe impl Send for Nvml {}
unsafe impl Sync for Nvml {}

impl Nvml {

    pub fn open() -> Option<Self> {
        let name = if cfg!(windows) { "nvml.dll" } else { "libnvidia-ml.so.1" };

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

        unsafe {
            if let Some(pci) = pci_bus_id.and_then(|p| CString::new(p).ok()) {
                if (self.by_pci)(pci.as_ptr(), &mut device) == SUCCESS {
                    return Some(device);
                }
            }
            ((self.by_index)(ordinal as c_uint, &mut device) == SUCCESS).then_some(device)
        }
    }

    pub fn read(&self, pci_bus_id: Option<&str>, ordinal: usize) -> Reading {
        let Some(device) = self.device(pci_bus_id, ordinal) else {
            return Reading::default();
        };
        let mut out = Reading::default();
        let mut value: c_uint = 0;

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

                if matches!(field.value_type, 1 | 2 | 3 | 5) && (field.value as u32) < 200 {
                    out.memory_c = Some(field.value as u32);
                }
            }
        }
        out
    }
}
