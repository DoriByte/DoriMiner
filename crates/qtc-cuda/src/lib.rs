//! CUDA search engine for Quantus QPoW: loads the sealed kernel image of each
//! card's architecture, runs launches of nonces on it and hands back the
//! candidates, which the host checks with the exact hash on the CPU
//! (`qtc_pow::get_nonce_hash`) before anything is sent to a pool.
//!
//! Batch launch and candidate verification are derived from
//! `crates/engine-cuda/src/lib.rs` of Quantus-Network/quantus-miner (commit
//! c7838cbc, Apache-2.0) and were changed by DoriByte.
//!
//! This file is published without its comments: they describe the kernel, which
//! is closed for now (see the README). The code is all here.

#![deny(rust_2018_idioms)]

use cudarc::driver::{
    result, sys, CudaContext, CudaEvent, CudaFunction, CudaSlice, CudaStream, DevicePtr,
    LaunchConfig, PinnedHostSlice, PushKernelArg,
};
use cudarc::nvrtc::Ptx;
use primitive_types::U512;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

include!(concat!(env!("OUT_DIR"), "/kernels.rs"));

mod seal;

const THREADS_PER_BLOCK: u32 = 256;
const MAX_BLOCKS: u32 = 4096;
const MAX_HITS: usize = 8;
pub const ABORT_BUILT: bool = false;
const RESULTS_U32S: usize = 1 + MAX_HITS + ABORT_BUILT as usize;

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

#[repr(C)]
#[derive(Clone, Copy)]
struct MiningParams {
    round1: [u32; 24],
    lanes: [u32; 4],
    difficulty_target: [u32; 16],
    dispatch_config: [u32; 3],
    walk_u: [[u64; qtc_pow::WALK2_ROW]; qtc_pow::WALK2_MAX_PER_THREAD as usize],
}

unsafe impl cudarc::driver::DeviceRepr for MiningParams {}

pub const MAX_LAUNCH_SIZE: u32 = qtc_pow::SPARSE_MAX_INDEX;
pub const MAX_LAUNCHES: u32 = qtc_pow::SPARSE_MAX_LAUNCH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Walk {
    Sparse,
    TwoDirection,
}

const WALK2_DEFAULT_PER_THREAD: u32 = 16;

impl Walk {
    pub fn of_arch(arch: u32) -> Walk {
        match KERNEL_WALK2.iter().any(|(a, walk2)| *a == arch && *walk2) {
            true => Walk::TwoDirection,
            false => Walk::Sparse,
        }
    }

    pub fn max_nonces_per_thread(self) -> u32 {
        match self {
            Walk::TwoDirection => qtc_pow::WALK2_MAX_PER_THREAD,
            Walk::Sparse => u32::MAX,
        }
    }

    pub fn fit_nonces_per_thread(self, per_thread: u32) -> u32 {
        per_thread.clamp(1, self.max_nonces_per_thread())
    }

    pub fn default_nonces_per_thread(self, count: u32) -> u32 {
        let grid = count.div_ceil(MAX_BLOCKS * THREADS_PER_BLOCK);
        match self {
            Walk::TwoDirection => self.fit_nonces_per_thread(grid.max(WALK2_DEFAULT_PER_THREAD)),
            Walk::Sparse => grid.max(1),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub header: [u8; 32],
    pub target: U512,
    pub nonce_high: [u8; 32],
}

impl Job {
    pub fn nonce_at(&self, walk: Walk, launch: u32, per_thread: u32, index: u32) -> [u8; 64] {
        let mut nonce = [0u8; 64];
        nonce[..32].copy_from_slice(&self.nonce_high);
        let low = match walk {
            Walk::TwoDirection => {
                let per_thread = per_thread.max(1);
                qtc_pow::walk2_nonce_low(launch, index / per_thread, index % per_thread)
            }
            Walk::Sparse => qtc_pow::sparse_nonce_low(launch, index),
        };
        nonce[32..].copy_from_slice(&low);
        nonce
    }

    pub fn verify_at(&self, walk: Walk, launch: u32, per_thread: u32, index: u32) -> Option<Hit> {
        let nonce_be = self.nonce_at(walk, launch, per_thread, index);
        let hash = qtc_pow::get_nonce_hash(self.header, nonce_be);
        (hash < self.target).then_some(Hit { nonce_be, hash })
    }

    pub fn prepare(&self) -> PreparedJob {
        PreparedJob {
            midstate: qtc_pow::mining_midstate(self.header, self.nonce_high),
            target_limbs: qtc_pow::u512_to_le_u32s(self.target),
            job: self.clone(),
            epoch: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PreparedJob {
    pub job: Job,
    midstate: [u64; qtc_pow::SPONGE_WIDTH],
    target_limbs: [u32; 16],
    epoch: u32,
}

impl PreparedJob {
    pub fn with_epoch(mut self, epoch: u32) -> Self {
        self.epoch = epoch;
        self
    }

    pub fn launch_slips(&self, walk: Walk, launch: u32, count: u32, per_thread: u32) -> bool {
        const P: u64 = 0xffff_ffff_0000_0001;
        const SMALL: u64 = 1 << 11;
        let inputs = qtc_pow::launch_inputs(self.midstate, launch);
        let (up, down, steps) = match walk {
            Walk::TwoDirection => (inputs[2], inputs[6], count.div_ceil(per_thread.max(1))),
            Walk::Sparse => (inputs[3], inputs[7], count),
        };
        let last = qtc_pow::SPARSE_STEP * steps.saturating_sub(1) as u64;
        let reaches = |low: u64, high: u64| (low..=high + last).contains(&down);
        let lands_small = up < SMALL || up >= P - last;
        reaches(2, SMALL) || reaches((1 << 31) - 1, (1 << 32) - 1) || lands_small
    }

    fn launch_params(&self, walk: Walk, launch: u32, dispatch_config: [u32; 3]) -> MiningParams {
        let mut walk_u = [[0u64; qtc_pow::WALK2_ROW]; qtc_pow::WALK2_MAX_PER_THREAD as usize];
        let (round1, lane_a, lane_b) = match walk {
            Walk::TwoDirection => {
                let pre = qtc_pow::walk2_prestate(self.midstate, launch);
                let rows = dispatch_config[1] as usize;
                for (step, row) in walk_u.iter_mut().enumerate().take(rows) {
                    *row = pre.row(step as u32);
                }
                (pre.round1, pre.lane2, pre.lane6)
            }
            Walk::Sparse => {
                let pre = qtc_pow::sparse_prestate(self.midstate, launch);
                (pre.round1, pre.lane3, pre.lane7)
            }
        };
        MiningParams {
            round1: qtc_pow::felts_to_u32s(round1),
            lanes: [
                lane_a as u32,
                (lane_a >> 32) as u32,
                lane_b as u32,
                (lane_b >> 32) as u32,
            ],
            difficulty_target: self.target_limbs,
            dispatch_config,
            walk_u,
        }
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaitMode {
    #[default]
    Blocking,
    Yield,
    Spin,
}

impl std::str::FromStr for WaitMode {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        match text {
            "blocking" => Ok(Self::Blocking),
            "yield" => Ok(Self::Yield),
            "spin" => Ok(Self::Spin),
            other => Err(format!("'{other}' is not blocking, yield or spin")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LaunchTimes {
    pub launches: u64,
    pub prepare_ns: u64,
    pub submit_ns: u64,
    pub wait_ns: u64,
    pub readback_ns: u64,
    pub verify_ns: u64,
    pub outside_ns: u64,
    pub gpu_gap_ns: u64,
    pub gpu_clear_ns: u64,
    pub gpu_kernel_ns: u64,
    pub gpu_launches: u64,
}

impl LaunchTimes {
    pub fn since(&self, earlier: &Self) -> Self {
        Self {
            launches: self.launches - earlier.launches,
            prepare_ns: self.prepare_ns - earlier.prepare_ns,
            submit_ns: self.submit_ns - earlier.submit_ns,
            wait_ns: self.wait_ns - earlier.wait_ns,
            readback_ns: self.readback_ns - earlier.readback_ns,
            verify_ns: self.verify_ns - earlier.verify_ns,
            outside_ns: self.outside_ns - earlier.outside_ns,
            gpu_gap_ns: self.gpu_gap_ns - earlier.gpu_gap_ns,
            gpu_clear_ns: self.gpu_clear_ns - earlier.gpu_clear_ns,
            gpu_kernel_ns: self.gpu_kernel_ns - earlier.gpu_kernel_ns,
            gpu_launches: self.gpu_launches - earlier.gpu_launches,
        }
    }

    pub fn host_ns_per_launch(&self) -> f64 {
        let host = self.prepare_ns
            + self.submit_ns
            + self.readback_ns
            + self.verify_ns
            + self.outside_ns;
        host as f64 / self.launches.max(1) as f64
    }

    pub fn gpu_idle_fraction(&self) -> Option<f64> {
        let total = self.gpu_gap_ns + self.gpu_clear_ns + self.gpu_kernel_ns;
        (self.gpu_launches > 0 && total > 0)
            .then(|| (self.gpu_gap_ns + self.gpu_clear_ns) as f64 / total as f64)
    }

    pub fn summary(&self) -> String {
        let per = |ns: u64, n: u64| ns as f64 / n.max(1) as f64 / 1e6;
        let n = self.launches;
        let mut text = format!(
            "{n} launches; host per launch: wait {:.3} ms, submit {:.3}, readback {:.3}, verify {:.3}, prepare {:.3}, outside {:.3} (not waiting: {:.3} ms)",
            per(self.wait_ns, n),
            per(self.submit_ns, n),
            per(self.readback_ns, n),
            per(self.verify_ns, n),
            per(self.prepare_ns, n),
            per(self.outside_ns, n),
            self.host_ns_per_launch() / 1e6,
        );
        if let Some(idle) = self.gpu_idle_fraction() {
            let g = self.gpu_launches;
            text.push_str(&format!(
                "; card per launch: kernel {:.3} ms, gap {:.3}, clear {:.3} (between kernels: {:.2}%)",
                per(self.gpu_kernel_ns, g),
                per(self.gpu_gap_ns, g),
                per(self.gpu_clear_ns, g),
                idle * 100.0
            ));
        }
        text
    }
}

fn ns_between(from: Instant, to: Instant) -> u64 {
    to.saturating_duration_since(from).as_nanos() as u64
}

struct Slot {
    stream: Arc<CudaStream>,
    _results: CudaSlice<u32>,
    results_ptr: sys::CUdeviceptr,
    _host: PinnedHostSlice<u32>,
    host_ptr: *mut u32,
    done: CudaEvent,
    timing: Option<SlotTiming>,
}

struct SlotTiming {
    start: CudaEvent,
    cleared: CudaEvent,
    kernel_end: CudaEvent,
}

struct Flight {
    slot: usize,
    job: Job,
    launch: u32,
    count: u32,
    per_thread: u32,
}

struct AbortInner {
    stream: Arc<CudaStream>,
    word: Arc<CudaSlice<u32>>,
    word_ptr: sys::CUdeviceptr,
    _host: PinnedHostSlice<u32>,
    host_ptr: *mut u32,
    newest: Mutex<u32>,
}

unsafe impl Send for AbortInner {}
unsafe impl Sync for AbortInner {}

#[derive(Clone)]
pub struct AbortHandle(Arc<AbortInner>);

impl AbortHandle {
    pub fn raise(&self, epoch: u32) -> Result<(), Error> {
        let inner = &*self.0;
        let mut newest = inner.newest.lock().unwrap();
        if epoch <= *newest {
            return Ok(());
        }
        *newest = epoch;
        let wrap = |e| err("could not tell the card its job is over", e);
        inner.stream.context().bind_to_thread().map_err(wrap)?;
        let _keep = &inner.word;
        unsafe {
            *inner.host_ptr = epoch;
            result::memcpy_htod_async(
                inner.word_ptr,
                std::slice::from_raw_parts(inner.host_ptr, 1),
                inner.stream.cu_stream(),
            )
            .map_err(wrap)?;
            let _ = sys::cuStreamQuery(inner.stream.cu_stream());
        }
        Ok(())
    }
}

fn skipped_nonces(count: u32, per_thread: u32, blocks: u32) -> u64 {
    let block = THREADS_PER_BLOCK as u64 * per_thread.max(1) as u64;
    let full = count as u64 / block;
    let rest = count as u64 % block;
    let blocks = blocks as u64;
    blocks.min(full) * block + if blocks > full { rest } else { 0 }
}

fn decode(
    walk: Walk,
    job: &Job,
    launch: u32,
    count: u32,
    per_thread: u32,
    raw: &[u32],
) -> Result<BatchOutcome, Error> {
    let reported = raw[0] as usize;
    let skipped = match ABORT_BUILT {
        true => skipped_nonces(count, per_thread, raw[1 + MAX_HITS]),
        false => 0,
    };
    let mut outcome = BatchOutcome {
        hashes: count as u64 - skipped,
        skipped,
        dropped_candidates: reported.saturating_sub(MAX_HITS) as u32,
        ..Default::default()
    };
    for &index in &raw[1..=reported.min(MAX_HITS)] {
        if index >= count {
            return Err(Error(format!(
                "the kernel returned candidate index {index} for a launch of {count}"
            )));
        }
        match job.verify_at(walk, launch, per_thread, index) {
            Some(hit) => outcome.hits.push(hit),
            None => outcome.false_candidates += 1,
        }
    }
    Ok(outcome)
}

struct GpuTiming {
    start: CudaEvent,
    cleared: CudaEvent,
    end: CudaEvent,
    previous_end: CudaEvent,
    has_previous: bool,
}

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub ordinal: usize,
    pub name: String,
    pub capability: u32,
    pub pci_bus_id: Option<String>,
}

pub fn kernel_arch_for(capability: u32) -> Option<u32> {
    KERNELS
        .iter()
        .map(|(arch, _)| *arch)
        .filter(|arch| *arch <= capability)
        .max()
}

const NONCES_PER_THREAD: Option<&str> = None;

pub fn whole_waves(count: u32, per_thread: u32, wave: u32) -> u32 {
    let block = THREADS_PER_BLOCK as u64 * per_thread.max(1) as u64;
    let wave = wave.max(1) as u64;
    let mut blocks = ((count as u64 + block * wave / 2) / (block * wave)).max(1) * wave;
    while blocks > wave && blocks * block > MAX_LAUNCH_SIZE as u64 {
        blocks -= wave;
    }
    match blocks * block <= MAX_LAUNCH_SIZE as u64 {
        true => (blocks * block) as u32,
        false => count,
    }
}

fn dispatch(walk: Walk, count: u32, per_thread: Option<u32>) -> (u32, u32, u32) {
    let fixed = per_thread
        .or_else(|| NONCES_PER_THREAD.and_then(|text| text.parse::<u32>().ok()))
        .or_else(|| match walk {
            Walk::TwoDirection => Some(walk.default_nonces_per_thread(count)),
            Walk::Sparse => None,
        });
    if let Some(per_thread) = fixed {
        let per_thread = walk
            .fit_nonces_per_thread(per_thread)
            .min((u32::MAX - count) / THREADS_PER_BLOCK)
            .max(1);
        let blocks = count
            .div_ceil(per_thread)
            .div_ceil(THREADS_PER_BLOCK)
            .max(1);
        return (blocks, blocks * THREADS_PER_BLOCK, per_thread);
    }
    let blocks = count.div_ceil(THREADS_PER_BLOCK).clamp(1, MAX_BLOCKS);
    let threads = blocks * THREADS_PER_BLOCK;
    (blocks, threads, count.div_ceil(threads).max(1))
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

fn load_kernel_module(
    ctx: &Arc<CudaContext>,
    table: &[(u32, &[u8])],
    arch: u32,
    name: &str,
) -> Result<Arc<cudarc::driver::CudaModule>, Error> {
    let mut last_error = String::from("no image embedded");
    for (_, stored) in table.iter().filter(|(a, _)| *a == arch) {
        let opened = seal::unseal(stored);
        let image: &[u8] = opened.as_deref().unwrap_or(stored);
        let native = image.starts_with(b"\x7fELF");
        let ptx = if native {
            Ptx::from_binary(image.to_vec())
        } else {
            Ptx::from_src(String::from_utf8_lossy(image))
        };
        match ctx.load_module(ptx) {
            Ok(module) => {
                log::info!("{name}: loaded the sm_{arch} kernel");
                return Ok(module);
            }
            Err(e) => last_error = e.to_string(),
        }
    }
    Err(Error(format!(
        "could not load the sm_{arch} kernel on {name}: {last_error}"
    )))
}

fn shared_pad() -> u32 {
    let pad = 0;
    pad
}

pub struct CudaSearcher {
    info: DeviceInfo,
    stream: Arc<CudaStream>,
    results: CudaSlice<u32>,
    mine: CudaFunction,
    hash: CudaFunction,
    registers: i32,
    local_bytes: i32,
    times: LaunchTimes,
    last_return: Option<Instant>,
    gpu_timing: Option<GpuTiming>,
    slots: Vec<Slot>,
    next_slot: usize,
    flights: VecDeque<Flight>,
    #[allow(dead_code)]
    abort_word: Arc<CudaSlice<u32>>,
    abort: Option<AbortHandle>,
    nonces_per_thread: Option<u32>,
    wait_mode: WaitMode,
    walk: Walk,
}

impl Drop for CudaSearcher {
    fn drop(&mut self) {
        if !self.flights.is_empty() {
            let _ = self.stream.context().synchronize();
        }
    }
}

impl CudaSearcher {
    pub fn open(ordinal: usize) -> Result<Self, Error> {
        let ctx = CudaContext::new(ordinal)
            .map_err(|e| err(&format!("could not open CUDA device {ordinal}"), e))?;
        ctx.set_blocking_synchronize()
            .map_err(|e| err("could not enable blocking synchronisation", e))?;
        let info = device_info(ordinal)?;
        let (name, capability) = (&info.name, info.capability);
        let arch = kernel_arch_for(capability).ok_or_else(|| {
            Error(format!(
                "{name} (sm_{capability}) is older than any kernel this build carries"
            ))
        })?;
        let module = load_kernel_module(&ctx, KERNELS, arch, name)?;
        let walk = Walk::of_arch(arch);
        let mine = module
            .load_function("mining_main")
            .map_err(|e| err("could not find mining_main", e))?;
        let hash = module
            .load_function("hash_nonces")
            .map_err(|e| err("could not find hash_nonces", e))?;
        let registers = mine.num_regs().unwrap_or(-1);
        let local_bytes = mine.local_size_bytes().unwrap_or(-1);
        log::info!(
            "CUDA device {ordinal}: {} (sm_{capability}, kernel sm_{arch})",
            info.name
        );
        let stream = ctx.default_stream();
        let results = stream
            .alloc_zeros::<u32>(RESULTS_U32S)
            .map_err(|e| err("could not allocate the result buffer", e))?;
        let abort_word = stream
            .alloc_zeros::<u32>(1)
            .map_err(|e| err("could not allocate the abort word", e))?;
        Ok(Self {
            info,
            stream,
            results,
            mine,
            hash,
            registers,
            local_bytes,
            times: LaunchTimes::default(),
            last_return: None,
            gpu_timing: None,
            slots: Vec::new(),
            next_slot: 0,
            flights: VecDeque::new(),
            abort_word: Arc::new(abort_word),
            abort: None,
            nonces_per_thread: None,
            wait_mode: WaitMode::Blocking,
            walk,
        })
    }

    pub fn set_wait_mode(&mut self, mode: WaitMode) -> Result<(), Error> {
        let flags = match mode {
            WaitMode::Blocking => sys::CUctx_flags::CU_CTX_SCHED_BLOCKING_SYNC,
            WaitMode::Yield => sys::CUctx_flags::CU_CTX_SCHED_YIELD,
            WaitMode::Spin => sys::CUctx_flags::CU_CTX_SCHED_SPIN,
        };
        self.stream
            .context()
            .set_flags(flags)
            .map_err(|e| err("could not change how the thread waits for the card", e))?;
        self.wait_mode = mode;
        Ok(())
    }

    pub fn set_nonces_per_thread(&mut self, per_thread: Option<u32>) {
        let walk = self.walk;
        self.nonces_per_thread = per_thread.map(|n| walk.fit_nonces_per_thread(n));
        if per_thread.is_some_and(|n| walk.fit_nonces_per_thread(n) < n) {
            log::warn!(
                "{}: --nonces-per-thread can be at most {}; using that",
                self.info.name,
                walk.max_nonces_per_thread()
            );
        }
    }

    pub fn walk(&self) -> Walk {
        self.walk
    }

    pub fn nonces_per_thread(&self, count: u32) -> u32 {
        dispatch(self.walk, count, self.nonces_per_thread).2
    }

    pub fn concurrent_blocks(&self) -> Result<u32, Error> {
        let wrap = |e| err("could not query the kernel's occupancy", e);
        let ctx = self.stream.context();
        ctx.bind_to_thread().map_err(wrap)?;
        let processors = ctx
            .attribute(sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_MULTIPROCESSOR_COUNT)
            .map_err(wrap)?;
        let per_processor = self
            .mine
            .occupancy_max_active_blocks_per_multiprocessor(THREADS_PER_BLOCK, 0, None)
            .map_err(wrap)?;
        Ok(processors.max(1) as u32 * per_processor.max(1))
    }

    pub fn enable_abort(&mut self) -> Result<AbortHandle, Error> {
        if !ABORT_BUILT {
            return Err(Error(
                "this build's kernel has no abort check"
                    .into(),
            ));
        }
        if let Some(handle) = &self.abort {
            return Ok(handle.clone());
        }
        let ctx = self.stream.context().clone();
        let wrap = |e| err("could not prepare the abort word", e);
        unsafe { ctx.disable_event_tracking() };
        let stream = ctx.new_stream().map_err(wrap)?;
        let mut host = unsafe { ctx.alloc_pinned_with_flags::<u32>(1, 0) }.map_err(wrap)?;
        let host_ptr = host.as_mut_ptr().map_err(wrap)?;
        unsafe { *host_ptr = 0 };
        let handle = AbortHandle(Arc::new(AbortInner {
            stream,
            word_ptr: self.abort_word.device_ptr(&self.stream).0,
            word: self.abort_word.clone(),
            _host: host,
            host_ptr,
            newest: Mutex::new(0),
        }));
        self.abort = Some(handle.clone());
        Ok(handle)
    }

    pub fn set_pipeline(&mut self, depth: usize, streams: usize) -> Result<(), Error> {
        if !self.flights.is_empty() {
            return Err(Error("launches are still in flight".into()));
        }
        if !(1..=8).contains(&depth) || !(1..=depth).contains(&streams) {
            return Err(Error(format!(
                "cannot queue {depth} launches on {streams} streams"
            )));
        }
        let ctx = self.stream.context().clone();
        let wrap = |e| err("could not prepare the launch queue", e);
        let mut lanes = vec![self.stream.clone()];
        if streams > 1 {
            unsafe { ctx.disable_event_tracking() };
        }
        while lanes.len() < streams {
            lanes.push(ctx.new_stream().map_err(wrap)?);
        }
        let timed = self.gpu_timing.is_some();
        let event = |flags| ctx.new_event(Some(flags)).map_err(wrap);
        self.slots.clear();
        self.next_slot = 0;
        for i in 0..depth {
            let results = self
                .stream
                .alloc_zeros::<u32>(RESULTS_U32S)
                .map_err(wrap)?;
            let results_ptr = results.device_ptr(&self.stream).0;
            let mut host = unsafe { ctx.alloc_pinned_with_flags::<u32>(RESULTS_U32S, 0) }
                .map_err(wrap)?;
            let host_ptr = host.as_mut_ptr().map_err(wrap)?;
            unsafe { std::ptr::write_bytes(host_ptr, 0, RESULTS_U32S) };
            let timing = match timed {
                true => Some(SlotTiming {
                    start: event(sys::CUevent_flags::CU_EVENT_DEFAULT)?,
                    cleared: event(sys::CUevent_flags::CU_EVENT_DEFAULT)?,
                    kernel_end: event(sys::CUevent_flags::CU_EVENT_DEFAULT)?,
                }),
                false => None,
            };
            self.slots.push(Slot {
                stream: lanes[i % streams].clone(),
                _results: results,
                results_ptr,
                _host: host,
                host_ptr,
                done: event(match self.wait_mode {
                    WaitMode::Blocking => sys::CUevent_flags::CU_EVENT_BLOCKING_SYNC,
                    _ => sys::CUevent_flags::CU_EVENT_DEFAULT,
                })?,
                timing,
            });
        }
        self.stream.synchronize().map_err(wrap)?;
        Ok(())
    }

    pub fn in_flight(&self) -> usize {
        self.flights.len()
    }

    pub fn submit(&mut self, prepared: &PreparedJob, launch: u32, count: u32) -> Result<(), Error> {
        if count == 0 || count > MAX_LAUNCH_SIZE || launch >= MAX_LAUNCHES {
            return Err(Error(format!(
                "launch {launch} of {count} nonces is outside the nonce walk"
            )));
        }
        if self.flights.len() >= self.slots.len() {
            return Err(Error("no free slot to queue a launch in".into()));
        }
        let entered = Instant::now();
        if let Some(last) = self.last_return {
            self.times.outside_ns += ns_between(last, entered);
        }
        let (blocks, threads, per_thread) = dispatch(self.walk, count, self.nonces_per_thread);
        let params = prepared.launch_params(self.walk, launch, [threads, per_thread, count]);
        let prepared_at = Instant::now();

        let index = self.next_slot;
        self.next_slot = (index + 1) % self.slots.len();
        let slot = &self.slots[index];
        let cu_stream = slot.stream.cu_stream();
        let wrap = |e| err("could not queue the launch", e);
        slot.stream.context().bind_to_thread().map_err(wrap)?;
        if let Some(timing) = &slot.timing {
            timing.start.record(&slot.stream).map_err(wrap)?;
        }
        unsafe {
            result::memset_d8_async(
                slot.results_ptr,
                0,
                RESULTS_U32S * std::mem::size_of::<u32>(),
                cu_stream,
            )
            .map_err(wrap)?;
            if let Some(timing) = &slot.timing {
                timing.cleared.record(&slot.stream).map_err(wrap)?;
            }
            let mut args = [
                &slot.results_ptr as *const sys::CUdeviceptr as *mut c_void,
                &params as *const MiningParams as *mut c_void,
            ];
            result::launch_kernel(
                self.mine.cu_function(),
                (blocks, 1, 1),
                (THREADS_PER_BLOCK, 1, 1),
                shared_pad(),
                cu_stream,
                &mut args,
            )
            .map_err(|e| err("kernel launch failed", e))?;
            if let Some(timing) = &slot.timing {
                timing.kernel_end.record(&slot.stream).map_err(wrap)?;
            }
            result::memcpy_dtoh_async(
                std::slice::from_raw_parts_mut(slot.host_ptr, RESULTS_U32S),
                slot.results_ptr,
                cu_stream,
            )
            .map_err(wrap)?;
            slot.done.record(&slot.stream).map_err(wrap)?;
            let _ = sys::cuStreamQuery(cu_stream);
        }
        self.flights.push_back(Flight {
            slot: index,
            job: prepared.job.clone(),
            launch,
            count,
            per_thread,
        });
        let done = Instant::now();
        self.times.prepare_ns += ns_between(entered, prepared_at);
        self.times.submit_ns += ns_between(prepared_at, done);
        self.last_return = Some(done);
        Ok(())
    }

    pub fn collect(&mut self) -> Result<BatchOutcome, Error> {
        let entered = Instant::now();
        if let Some(last) = self.last_return {
            self.times.outside_ns += ns_between(last, entered);
        }
        let flight = self
            .flights
            .pop_front()
            .ok_or_else(|| Error("no launch is in flight".into()))?;
        let slot = &self.slots[flight.slot];
        slot.done
            .synchronize()
            .map_err(|e| err("kernel run failed", e))?;
        let woke_at = Instant::now();
        let mut raw = [0u32; RESULTS_U32S];
        unsafe { std::ptr::copy_nonoverlapping(slot.host_ptr, raw.as_mut_ptr(), RESULTS_U32S) };
        let next = self.flights.front().map(|next| &self.slots[next.slot]);
        if let (Some(timing), Some(next)) = (&slot.timing, next.and_then(|n| n.timing.as_ref())) {
            let wrap = |e| err("could not time the launch", e);
            let ns = |ms: f32| (ms.max(0.0) as f64 * 1e6) as u64;
            let clear = timing.start.elapsed_ms(&timing.cleared).map_err(wrap)?;
            let kernel = timing.cleared.elapsed_ms(&timing.kernel_end).map_err(wrap)?;
            let gap = timing.kernel_end.elapsed_ms(&next.start).map_err(wrap)?;
            self.times.gpu_clear_ns += ns(clear);
            self.times.gpu_kernel_ns += ns(kernel);
            self.times.gpu_gap_ns += ns(gap);
            self.times.gpu_launches += 1;
        }
        let read_at = Instant::now();
        let outcome = decode(
            self.walk,
            &flight.job,
            flight.launch,
            flight.count,
            flight.per_thread,
            &raw,
        )?;
        let done = Instant::now();
        self.times.launches += 1;
        self.times.wait_ns += ns_between(entered, woke_at);
        self.times.readback_ns += ns_between(woke_at, read_at);
        self.times.verify_ns += ns_between(read_at, done);
        self.last_return = Some(done);
        Ok(outcome)
    }

    pub fn times(&self) -> LaunchTimes {
        self.times
    }

    pub fn enable_gpu_timing(&mut self) -> Result<(), Error> {
        if self.gpu_timing.is_some() {
            return Ok(());
        }
        let ctx = self.stream.context().clone();
        let event = || {
            ctx.new_event(Some(sys::CUevent_flags::CU_EVENT_DEFAULT))
                .map_err(|e| err("could not create a timing event", e))
        };
        self.gpu_timing = Some(GpuTiming {
            start: event()?,
            cleared: event()?,
            end: event()?,
            previous_end: event()?,
            has_previous: false,
        });
        Ok(())
    }

    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }

    pub fn kernel_footprint(&self) -> (i32, i32) {
        (self.registers, self.local_bytes)
    }

    pub fn search(
        &mut self,
        prepared: &PreparedJob,
        launch: u32,
        count: u32,
    ) -> Result<BatchOutcome, Error> {
        if count == 0 {
            return Ok(BatchOutcome::default());
        }
        if count > MAX_LAUNCH_SIZE || launch >= MAX_LAUNCHES {
            return Err(Error(format!(
                "launch {launch} of {count} nonces is outside the nonce walk"
            )));
        }
        if !self.flights.is_empty() {
            return Err(Error("launches are still in flight".into()));
        }
        let entered = Instant::now();
        if let Some(last) = self.last_return {
            self.times.outside_ns += ns_between(last, entered);
        }
        let (blocks, threads, per_thread) = dispatch(self.walk, count, self.nonces_per_thread);
        let params = prepared.launch_params(self.walk, launch, [threads, per_thread, count]);
        let prepared_at = Instant::now();

        let timing_error = |e| err("could not time the launch", e);
        if let Some(timing) = &self.gpu_timing {
            timing.start.record(&self.stream).map_err(timing_error)?;
        }
        self.stream
            .memset_zeros(&mut self.results)
            .map_err(|e| err("could not clear the result buffer", e))?;
        if let Some(timing) = &self.gpu_timing {
            timing.cleared.record(&self.stream).map_err(timing_error)?;
        }
        let cfg = LaunchConfig {
            grid_dim: (blocks, 1, 1),
            block_dim: (THREADS_PER_BLOCK, 1, 1),
            shared_mem_bytes: shared_pad(),
        };
        let mut builder = self.stream.launch_builder(&self.mine);
        builder.arg(&mut self.results);
        builder.arg(&params);
        unsafe { builder.launch(cfg) }.map_err(|e| err("kernel launch failed", e))?;
        if let Some(timing) = &self.gpu_timing {
            timing.end.record(&self.stream).map_err(timing_error)?;
        }
        let submitted_at = Instant::now();
        self.stream
            .synchronize()
            .map_err(|e| err("kernel run failed", e))?;
        let woke_at = Instant::now();
        let raw = self
            .stream
            .clone_dtoh(&self.results)
            .map_err(|e| err("could not read the result buffer", e))?;
        let read_at = Instant::now();
        if let Some(timing) = &mut self.gpu_timing {
            let ns = |ms: f32| (ms.max(0.0) as f64 * 1e6) as u64;
            if timing.has_previous {
                let gap = timing.previous_end.elapsed_ms(&timing.start);
                let clear = timing.start.elapsed_ms(&timing.cleared);
                let kernel = timing.cleared.elapsed_ms(&timing.end);
                self.times.gpu_gap_ns += ns(gap.map_err(timing_error)?);
                self.times.gpu_clear_ns += ns(clear.map_err(timing_error)?);
                self.times.gpu_kernel_ns += ns(kernel.map_err(timing_error)?);
                self.times.gpu_launches += 1;
            }
            std::mem::swap(&mut timing.end, &mut timing.previous_end);
            timing.has_previous = true;
        }

        let outcome = decode(self.walk, &prepared.job, launch, count, per_thread, &raw)?;
        let done = Instant::now();
        self.times.launches += 1;
        self.times.prepare_ns += ns_between(entered, prepared_at);
        self.times.submit_ns += ns_between(prepared_at, submitted_at);
        self.times.wait_ns += ns_between(submitted_at, woke_at);
        self.times.readback_ns += ns_between(woke_at, read_at);
        self.times.verify_ns += ns_between(read_at, done);
        self.last_return = Some(done);
        Ok(outcome)
    }

    pub fn hash_nonces(
        &mut self,
        header: [u8; 32],
        start_nonce_be: [u8; 64],
        count: u32,
    ) -> Result<Vec<U512>, Error> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let start = U512::from_big_endian(&start_nonce_be);
        let midstate = qtc_pow::felts_to_u32s(qtc_pow::mining_midstate(
            header,
            start_nonce_be[..32].try_into().unwrap(),
        ));
        let wrap = |e| err("hash self-test failed", e);
        let midstate_dev = self.stream.clone_htod(&midstate).map_err(wrap)?;
        let start_dev = self
            .stream
            .clone_htod(&qtc_pow::u512_to_le_u32s(start))
            .map_err(wrap)?;
        let mut hashes = self
            .stream
            .alloc_zeros::<u32>(count as usize * 16)
            .map_err(wrap)?;
        let cfg = LaunchConfig {
            grid_dim: (count.div_ceil(THREADS_PER_BLOCK).max(1), 1, 1),
            block_dim: (THREADS_PER_BLOCK, 1, 1),
            shared_mem_bytes: 0,
        };
        let mut builder = self.stream.launch_builder(&self.hash);
        builder.arg(&mut hashes);
        builder.arg(&midstate_dev);
        builder.arg(&start_dev);
        builder.arg(&count);
        unsafe { builder.launch(cfg) }.map_err(wrap)?;
        self.stream.synchronize().map_err(wrap)?;
        let raw = self.stream.clone_dtoh(&hashes).map_err(wrap)?;
        Ok(raw
            .chunks_exact(16)
            .map(|limbs| qtc_pow::u512_from_le_u32s(limbs.try_into().unwrap()))
            .collect())
    }
}

