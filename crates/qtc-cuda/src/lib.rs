//! CUDA search for Quantus QPoW through the sealed engine: runs launches of nonces on each card and hands back the
//! candidates, which the host checks here with the exact hash on the CPU (`qtc_pow::get_nonce_hash`) before anything is
//! sent to a pool.
//!
//! Batch launch and candidate verification are derived from `crates/engine-cuda/src/lib.rs` of
//! Quantus-Network/quantus-miner (commit c7838cbc, Apache-2.0) and were changed by DoriByte.
//!
//! This file is published without its comments: they describe the kernel, which is closed (see the README).

#![deny(rust_2018_idioms)]

use cudarc::driver::sys;
use primitive_types::U512;
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::fmt;

extern "C" {
    fn qtc_const(which: c_int) -> u32;
    fn qtc_arch_for(capability: u32) -> u32;
    fn qtc_lim_a(variant: u32) -> u32;
    fn qtc_lim_b(variant: u32, n: u32) -> u32;
    fn qtc_lim_c(variant: u32, count: u32, unit: u32) -> u32;
    fn qtc_init(header: *const u8, high: *const u8, permute: PermuteFn, out: *mut u64);
    fn qtc_chk_a(state: *const u64, variant: u32, launch: u32, count: u32, per: u32) -> c_int;
    fn qtc_val_a(variant: u32, launch: u32, per: u32, index: u32, out: *mut u8);
    fn qtc_open(ordinal: c_int, permute: PermuteFn, info: *mut u32, err: *mut c_char, errlen: c_int) -> *mut c_void;
    fn qtc_close(h: *mut c_void);
    fn qtc_opt_b(h: *mut c_void, asked: c_int, per: u32);
    fn qtc_get_b(h: *mut c_void, count: u32) -> u32;
    fn qtc_opt_c(h: *mut c_void, depth: c_int, streams: c_int, err: *mut c_char, errlen: c_int) -> c_int;
    fn qtc_put(h: *mut c_void, state: *const u64, target: *const u32, epoch: u32, launch: u32, count: u32) -> u32;
    fn qtc_submit(h: *mut c_void, err: *mut c_char, errlen: c_int) -> c_int;
    fn qtc_wait(h: *mut c_void, err: *mut c_char, errlen: c_int) -> c_int;
    fn qtc_collect(h: *mut c_void, skipped: *mut u64, reported: *mut u32, kept: *mut u32, lows: *mut u8, err: *mut c_char, errlen: c_int) -> c_int;
    fn qtc_search_start(h: *mut c_void, err: *mut c_char, errlen: c_int) -> c_int;
    fn qtc_search_wait(h: *mut c_void, err: *mut c_char, errlen: c_int) -> c_int;
    fn qtc_search_read(h: *mut c_void, skipped: *mut u64, reported: *mut u32, kept: *mut u32, lows: *mut u8, err: *mut c_char, errlen: c_int) -> c_int;
    fn qtc_hash_many(h: *mut c_void, header: *const u8, start_be: *const u8, start_le: *const u32, count: u32, out: *mut u32, err: *mut c_char, errlen: c_int) -> c_int;
}

type PermuteFn = extern "C" fn(*mut u64);

extern "C" fn permute(state: *mut u64) {
    qtc_pow::permute(unsafe { &mut *(state as *mut [u64; qtc_pow::SPONGE_WIDTH]) });
}

const MAX_HITS: usize = 8;

#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn err(context: &str, e: impl fmt::Display) -> Error {
    Error(format!("{context}: {e}"))
}

fn engine_error(buf: &[c_char]) -> Error {
    Error(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned())
}

pub const MAX_LAUNCH_SIZE: u32 = 60_000_000;
pub const MAX_LAUNCHES: u32 = 1 << 30;

pub fn defaults() -> (usize, usize) {
    unsafe { (qtc_const(3) as usize, qtc_const(4) as usize) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Variant(u32);

impl Variant {
    pub fn all() -> [Variant; 2] {
        [Variant(0), Variant(1)]
    }

    pub fn param_max(self) -> u32 {
        unsafe { qtc_lim_a(self.0) }
    }

    pub fn param_fit(self, param: u32) -> u32 {
        unsafe { qtc_lim_b(self.0, param) }
    }

    pub fn param_default(self, count: u32, unit: u32) -> u32 {
        unsafe { qtc_lim_c(self.0, count, unit) }
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub header: [u8; 32],
    pub target: U512,
    pub nonce_high: [u8; 32],
}

impl Job {
    pub fn nonce_at(&self, variant: Variant, launch: u32, param: u32, index: u32) -> [u8; 64] {
        let mut nonce = [0u8; 64];
        nonce[..32].copy_from_slice(&self.nonce_high);
        unsafe { qtc_val_a(variant.0, launch, param, index, nonce[32..].as_mut_ptr()) };
        nonce
    }

    pub fn verify_at(&self, variant: Variant, launch: u32, param: u32, index: u32) -> Option<Hit> {
        let nonce_be = self.nonce_at(variant, launch, param, index);
        let hash = qtc_pow::get_nonce_hash(self.header, nonce_be);
        (hash < self.target).then_some(Hit { nonce_be, hash })
    }

    pub fn prepare(&self) -> PreparedJob {
        let mut state = [0u64; qtc_pow::SPONGE_WIDTH];
        unsafe { qtc_init(self.header.as_ptr(), self.nonce_high.as_ptr(), permute, state.as_mut_ptr()) };
        PreparedJob { state, target_limbs: qtc_pow::u512_to_le_u32s(self.target), job: self.clone(), epoch: 0 }
    }
}

#[derive(Clone, Debug)]
pub struct PreparedJob {
    pub job: Job,
    state: [u64; qtc_pow::SPONGE_WIDTH],
    target_limbs: [u32; 16],
    epoch: u32,
}

impl PreparedJob {
    pub fn with_epoch(mut self, epoch: u32) -> Self {
        self.epoch = epoch;
        self
    }

    pub fn excluded(&self, variant: Variant, launch: u32, count: u32, param: u32) -> bool {
        unsafe { qtc_chk_a(self.state.as_ptr(), variant.0, launch, count, param) != 0 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub nonce_be: [u8; 64],
    pub hash: U512,
}

#[derive(Clone, Debug, Default)]
pub struct BatchOutcome {
    pub hashes: u64,
    pub skipped: u64,
    pub hits: Vec<Hit>,
    pub false_candidates: u32,
    pub dropped_candidates: u32,
}

struct Flight {
    job: Job,
    launch: u32,
    count: u32,
    param: u32,
}

fn decode(job: &Job, launch: u32, count: u32, param: u32, skipped: u64, reported: u32, lows: &[u8]) -> BatchOutcome {
    let _ = (launch, param);
    let mut outcome = BatchOutcome {
        hashes: count as u64 - skipped,
        skipped,
        dropped_candidates: (reported as usize).saturating_sub(MAX_HITS) as u32,
        ..Default::default()
    };
    for low in lows.chunks_exact(32) {
        let mut nonce_be = [0u8; 64];
        nonce_be[..32].copy_from_slice(&job.nonce_high);
        nonce_be[32..].copy_from_slice(low);
        let hash = qtc_pow::get_nonce_hash(job.header, nonce_be);
        if hash < job.target {
            outcome.hits.push(Hit { nonce_be, hash });
        } else {
            outcome.false_candidates += 1;
        }
    }
    outcome
}

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub ordinal: usize,
    pub name: String,
    pub capability: u32,
    pub pci_bus_id: Option<String>,
}

pub fn kernel_arch_for(capability: u32) -> Option<u32> {
    match unsafe { qtc_arch_for(capability) } {
        0 => None,
        arch => Some(arch),
    }
}

pub fn device_count() -> Result<usize, Error> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let init = std::panic::catch_unwind(cudarc::driver::result::init);
    std::panic::set_hook(hook);
    match init {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(err("CUDA driver failed to initialise", e)),
        Err(_) => {
            return Err(Error(
                "NVIDIA driver not found (CUDA library missing)".into(),
            ))
        }
    }
    let count = cudarc::driver::result::device::get_count()
        .map_err(|e| err("could not count CUDA devices", e))?;
    Ok(count.max(0) as usize)
}

fn pci_bus_id(device: sys::CUdevice) -> Option<String> {
    let mut buf = [0 as std::ffi::c_char; 32];
    let result = unsafe { sys::cuDeviceGetPCIBusId(buf.as_mut_ptr(), buf.len() as i32, device) };
    result.result().ok()?;
    let text = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
    Some(text.to_string_lossy().into_owned())
}

pub fn device_info(ordinal: usize) -> Result<DeviceInfo, Error> {
    use cudarc::driver::result::device;
    use sys::CUdevice_attribute::{
        CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR as MAJOR,
        CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR as MINOR,
    };
    let wrap = |e| err(&format!("could not query CUDA device {ordinal}"), e);
    let handle = device::get(ordinal as i32).map_err(wrap)?;
    let major = unsafe { device::get_attribute(handle, MAJOR) }.map_err(wrap)?;
    let minor = unsafe { device::get_attribute(handle, MINOR) }.map_err(wrap)?;
    Ok(DeviceInfo {
        ordinal,
        name: device::get_name(handle).unwrap_or_else(|_| format!("cuda-device-{ordinal}")),
        capability: (major * 10 + minor) as u32,
        pci_bus_id: pci_bus_id(handle),
    })
}

pub struct CudaSearcher {
    h: *mut c_void,
    info: DeviceInfo,
    arch: u32,
    variant: Variant,
    unit: u32,
    flights: VecDeque<Flight>,
}

impl Drop for CudaSearcher {
    fn drop(&mut self) {
        unsafe { qtc_close(self.h) }
    }
}

macro_rules! errbuf {
    ($name:ident) => {
        let mut $name = [0 as c_char; 256];
    };
}

impl CudaSearcher {
    pub fn open(ordinal: usize) -> Result<Self, Error> {
        errbuf!(buf);
        let mut got = [0u32; 4];
        let h = unsafe { qtc_open(ordinal as c_int, permute, got.as_mut_ptr(), buf.as_mut_ptr(), buf.len() as c_int) };
        if h.is_null() {
            return Err(engine_error(&buf));
        }
        let [arch, capability, variant, unit] = got;
        let mut searcher = Self {
            h,
            info: DeviceInfo { ordinal, name: String::new(), capability, pci_bus_id: None },
            arch,
            variant: Variant(variant),
            unit,
            flights: VecDeque::new(),
        };
        searcher.info = device_info(ordinal)?;
        log::info!("{}: loaded the sm_{arch} kernel", searcher.info.name);
        log::info!(
            "CUDA device {ordinal}: {} (sm_{capability}, kernel sm_{arch})",
            searcher.info.name
        );
        Ok(searcher)
    }

    pub fn set_param(&mut self, param: Option<u32>) {
        let variant = self.variant;
        unsafe { qtc_opt_b(self.h, param.is_some() as c_int, param.unwrap_or(0)) };
        if param.is_some_and(|n| variant.param_fit(n) < n) {
            log::warn!(
                "{}: the setting can be at most {}; using that",
                self.info.name,
                variant.param_max()
            );
        }
    }

    pub fn variant(&self) -> Variant {
        self.variant
    }

    pub fn unit(&self) -> u32 {
        self.unit
    }

    pub fn param(&self, count: u32) -> u32 {
        unsafe { qtc_get_b(self.h, count) }
    }

    pub fn set_depth(&mut self, depth: usize, streams: usize) -> Result<(), Error> {
        errbuf!(buf);
        match unsafe { qtc_opt_c(self.h, depth.min(1024) as c_int, streams.min(1024) as c_int, buf.as_mut_ptr(), buf.len() as c_int) } {
            0 => Ok(()),
            _ => Err(engine_error(&buf)),
        }
    }

    pub fn in_flight(&self) -> usize {
        self.flights.len()
    }

    pub fn submit(&mut self, prepared: &PreparedJob, launch: u32, count: u32) -> Result<(), Error> {
        if count == 0 || count > MAX_LAUNCH_SIZE || launch >= MAX_LAUNCHES {
            return Err(Error(format!(
                "launch {launch} of {count} nonces is more than a job can take"
            )));
        }
        let param = unsafe { qtc_put(self.h, prepared.state.as_ptr(), prepared.target_limbs.as_ptr(), prepared.epoch, launch, count) };
        errbuf!(buf);
        if unsafe { qtc_submit(self.h, buf.as_mut_ptr(), buf.len() as c_int) } != 0 {
            return Err(engine_error(&buf));
        }
        self.flights.push_back(Flight { job: prepared.job.clone(), launch, count, param });
        Ok(())
    }

    pub fn collect(&mut self) -> Result<BatchOutcome, Error> {
        let flight = self.flights.pop_front().ok_or_else(|| Error("no launch is in flight".into()))?;
        errbuf!(buf);
        if unsafe { qtc_wait(self.h, buf.as_mut_ptr(), buf.len() as c_int) } != 0 {
            return Err(engine_error(&buf));
        }
        let (mut skipped, mut reported, mut kept, mut lows) = (0u64, 0u32, 0u32, [0u8; 32 * MAX_HITS]);
        if unsafe { qtc_collect(self.h, &mut skipped, &mut reported, &mut kept, lows.as_mut_ptr(), buf.as_mut_ptr(), buf.len() as c_int) } != 0 {
            return Err(engine_error(&buf));
        }
        let outcome = decode(&flight.job, flight.launch, flight.count, flight.param, skipped, reported, &lows[..32 * kept as usize]);
        Ok(outcome)
    }

    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }

    pub fn kernel_arch(&self) -> u32 {
        self.arch
    }

    pub fn search(&mut self, prepared: &PreparedJob, launch: u32, count: u32) -> Result<BatchOutcome, Error> {
        if count == 0 {
            return Ok(BatchOutcome::default());
        }
        if count > MAX_LAUNCH_SIZE || launch >= MAX_LAUNCHES {
            return Err(Error(format!(
                "launch {launch} of {count} nonces is more than a job can take"
            )));
        }
        if !self.flights.is_empty() {
            return Err(Error("launches are still in flight".into()));
        }
        let param = unsafe { qtc_put(self.h, prepared.state.as_ptr(), prepared.target_limbs.as_ptr(), prepared.epoch, launch, count) };
        errbuf!(buf);
        if unsafe { qtc_search_start(self.h, buf.as_mut_ptr(), buf.len() as c_int) } != 0 {
            return Err(engine_error(&buf));
        }
        if unsafe { qtc_search_wait(self.h, buf.as_mut_ptr(), buf.len() as c_int) } != 0 {
            return Err(engine_error(&buf));
        }
        let (mut skipped, mut reported, mut kept, mut lows) = (0u64, 0u32, 0u32, [0u8; 32 * MAX_HITS]);
        if unsafe { qtc_search_read(self.h, &mut skipped, &mut reported, &mut kept, lows.as_mut_ptr(), buf.as_mut_ptr(), buf.len() as c_int) } != 0 {
            return Err(engine_error(&buf));
        }
        let outcome = decode(&prepared.job, launch, count, param, skipped, reported, &lows[..32 * kept as usize]);
        Ok(outcome)
    }

    pub fn self_test_hashes(&mut self, header: [u8; 32], start_nonce_be: [u8; 64], count: u32) -> Result<Vec<U512>, Error> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let start = qtc_pow::u512_to_le_u32s(U512::from_big_endian(&start_nonce_be));
        let mut raw = vec![0u32; count as usize * 16];
        errbuf!(buf);
        if unsafe { qtc_hash_many(self.h, header.as_ptr(), start_nonce_be.as_ptr(), start.as_ptr(), count, raw.as_mut_ptr(), buf.as_mut_ptr(), buf.len() as c_int) } != 0 {
            return Err(engine_error(&buf));
        }
        Ok(raw.chunks_exact(16).map(|limbs| qtc_pow::u512_from_le_u32s(limbs.try_into().unwrap())).collect())
    }
}

