//! Published without its comments: they describe the kernel, which is closed for now (see the README).
//! The code is all here.


use crate::state::{GpuStats, Share, Shared, Work};
use primitive_types::U512;
use qtc_cuda::{BatchOutcome, CudaSearcher, Job, LaunchTimes, PreparedJob, Walk};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const IDLE_POLL: Duration = Duration::from_millis(20);
const STATS_EVERY: Duration = Duration::from_secs(30);
const WATCH_POLL: Duration = Duration::from_millis(200);
const REOPEN_AFTER: (Duration, Duration) = (Duration::from_secs(5), Duration::from_secs(60));
const REOPEN_RESET: Duration = Duration::from_secs(120);
const SALT_BYTES: usize = 8;
pub const MAX_EXTRANONCE: usize = 32 - SALT_BYTES;
const CHECK_NONCES: u32 = 200_000;
const CHECK_BITS: u32 = 16;

struct WatcherGuard(Arc<AtomicBool>, Option<std::thread::JoinHandle<()>>);

impl Drop for WatcherGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
        if let Some(thread) = self.1.take() {
            let _ = thread.join();
        }
    }
}

struct PanicGuard<'a>(&'a GpuStats);

impl Drop for PanicGuard<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            log::error!("GPU {}: its thread crashed", self.0.info.ordinal);
            self.0.failed.store(true, Ordering::SeqCst);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelfCheck {
    pub bits: u32,
    pub cpu_threads: usize,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub batch: u32,
    pub launch_stats: bool,
    pub pipeline: usize,
    pub pipeline_streams: usize,
    pub abort_on_job: bool,
    pub nonces_per_thread: Option<u32>,
    pub whole_waves: bool,
    pub wait_mode: qtc_cuda::WaitMode,
    pub startup_check: bool,
    pub self_check: Option<SelfCheck>,
}

impl Options {
    pub fn new(batch: u32) -> Self {
        Self {
            batch,
            launch_stats: false,
            pipeline: 1,
            pipeline_streams: 1,
            abort_on_job: false,
            nonces_per_thread: None,
            whole_waves: false,
            wait_mode: qtc_cuda::WaitMode::Blocking,
            startup_check: false,
            self_check: None,
        }
    }
}

struct CheckTask {
    prepared: Arc<PreparedJob>,
    launch: u32,
    count: u32,
    found: Vec<[u8; 64]>,
}

struct Checker {
    target: U512,
    tasks: SyncSender<CheckTask>,
}

fn cpu_hits(
    job: &Job,
    walk: Walk,
    launch: u32,
    per_thread: u32,
    count: u32,
    threads: usize,
    stopping: &(dyn Fn() -> bool + Sync),
) -> Vec<[u8; 64]> {
    let threads = threads.clamp(1, count.max(1) as usize) as u32;
    let share = count.div_ceil(threads);
    let mut hits: Vec<[u8; 64]> = std::thread::scope(|scope| {
        let parts: Vec<_> = (0..threads)
            .map(|part| {
                scope.spawn(move || {
                    let mut hits = Vec::new();
                    let end = (part + 1).saturating_mul(share).min(count);
                    for index in part * share..end {
                        if index % 4096 == 0 && stopping() {
                            break;
                        }
                        if let Some(hit) = job.verify_at(walk, launch, per_thread, index) {
                            hits.push(hit.nonce_be);
                        }
                    }
                    hits
                })
            })
            .collect();
        parts
            .into_iter()
            .flat_map(|part| part.join().unwrap())
            .collect()
    });
    if stopping() {
        return Vec::new();
    }
    hits.sort();
    hits
}

fn check_launches(
    shared: &Shared,
    slot: usize,
    walk: Walk,
    per_thread: u32,
    tasks: &Mutex<Receiver<CheckTask>>,
) {
    let stats = &shared.gpus[slot];
    loop {
        let task = match tasks.lock().unwrap().recv() {
            Ok(task) => task,
            Err(_) => return,
        };
        let job = &task.prepared.job;
        let stopping = || shared.stopping();
        let valid = cpu_hits(job, walk, task.launch, per_thread, task.count, 1, &stopping);
        if shared.stopping() {
            return;
        }
        let missed: Vec<_> = valid
            .iter()
            .filter(|nonce| !task.found.contains(*nonce))
            .collect();
        for nonce in &missed {
            log::error!(
                "GPU {}: launch {} of {} did not report the valid nonce {}",
                stats.info.ordinal,
                task.launch,
                task.count,
                hex::encode(nonce)
            );
        }
        let check = &stats.check;
        check
            .cpu_hashes
            .fetch_add(task.count as u64, Ordering::Relaxed);
        check
            .cpu_hits
            .fetch_add(valid.len() as u64, Ordering::Relaxed);
        check
            .cpu_missed
            .fetch_add(missed.len() as u64, Ordering::Relaxed);
    }
}

struct Flight {
    work: Arc<Work>,
    prepared: Arc<PreparedJob>,
    launch: u32,
    count: u32,
    started: Instant,
}

fn finish(
    stats: &GpuStats,
    shared: &Shared,
    shares: &Sender<Share>,
    checker: Option<&Checker>,
    flight: Flight,
    outcome: BatchOutcome,
) -> bool {
    let work = &flight.work;
    stats.record_batch(flight.started, outcome.hashes);
    if shared.generation() != work.generation {
        stats
            .stale_hashes
            .fetch_add(outcome.hashes, Ordering::Relaxed);
    }
    if outcome.false_candidates > 0 {
        stats
            .false_candidates
            .fetch_add(outcome.false_candidates as u64, Ordering::Relaxed);
    }
    if outcome.dropped_candidates > 0 {
        stats
            .dropped_candidates
            .fetch_add(outcome.dropped_candidates as u64, Ordering::Relaxed);
        if checker.is_none() {
            log::warn!(
                "GPU {}: {} candidates did not fit in one launch",
                stats.info.ordinal,
                outcome.dropped_candidates
            );
        }
    }
    if let Some(checker) = checker {
        stats
            .check
            .hits
            .fetch_add(outcome.hits.len() as u64, Ordering::Relaxed);
        if outcome.dropped_candidates == 0 && outcome.skipped == 0 {
            let task = CheckTask {
                prepared: flight.prepared.clone(),
                launch: flight.launch,
                count: flight.count,
                found: outcome.hits.iter().map(|hit| hit.nonce_be).collect(),
            };
            if let Err(TrySendError::Disconnected(_)) = checker.tasks.try_send(task) {
                log::debug!("GPU {}: the self-check threads are gone", stats.info.ordinal);
            }
        }
    }
    for hit in outcome.hits {
        if hit.hash >= work.target {
            continue;
        }
        let share = Share {
            generation: work.generation,
            job_id: work.job_id.clone(),
            nonce_be: hit.nonce_be,
            hash: hit.hash,
            dev: work.dev,
        };
        if shares.send(share).is_err() {
            return false;
        }
    }
    true
}

struct StatsLog {
    enabled: bool,
    at: Instant,
    times: LaunchTimes,
    hashes: u64,
    stale: u64,
}

impl StatsLog {
    fn tick(&mut self, searcher: &CudaSearcher, stats: &GpuStats) {
        if !self.enabled || self.at.elapsed() < STATS_EVERY {
            return;
        }
        let times = searcher.times();
        let hashes = stats.hashes.load(Ordering::Relaxed);
        let stale = stats.stale_hashes.load(Ordering::Relaxed);
        log::info!(
            "GPU {}: {}; on replaced jobs: {:.3}% of the hashes",
            stats.info.ordinal,
            times.since(&self.times).summary(),
            (stale - self.stale) as f64 * 100.0 / (hashes - self.hashes).max(1) as f64
        );
        *self = Self {
            enabled: true,
            at: Instant::now(),
            times,
            hashes,
            stale,
        };
    }
}

pub fn nonce_prefix(extranonce: &[u8], salt: [u8; SALT_BYTES]) -> Option<[u8; 32]> {
    let mut prefix = [0u8; 32];
    let end = extranonce.len().checked_add(salt.len())?;
    if end > prefix.len() {
        return None;
    }
    prefix[..extranonce.len()].copy_from_slice(extranonce);
    prefix[extranonce.len()..end].copy_from_slice(&salt);
    Some(prefix)
}

fn random_salt() -> [u8; SALT_BYTES] {
    let mut salt = [0u8; SALT_BYTES];
    if getrandom::getrandom(&mut salt).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        salt = nanos.to_le_bytes();
    }
    salt
}

struct Search {
    work: Arc<Work>,
    prepared: Arc<PreparedJob>,
    launch: u32,
}

impl Search {
    fn take_launch(&mut self, walk: Walk, count: u32, per_thread: u32) -> u32 {
        while self.launch + 1 < qtc_cuda::MAX_LAUNCHES
            && self
                .prepared
                .launch_slips(walk, self.launch, count, per_thread)
        {
            self.launch += 1;
        }
        self.launch += 1;
        self.launch - 1
    }

    fn exhausted(&self) -> bool {
        self.launch >= qtc_cuda::MAX_LAUNCHES
    }
}

fn start_search(work: Arc<Work>, easy: Option<U512>) -> Option<Search> {
    let nonce_high = nonce_prefix(&work.extranonce, random_salt())?;
    let prepared = Job {
        header: work.header,
        target: easy.map_or(work.target, |easy| easy.max(work.target)),
        nonce_high,
    }
    .prepare()
    .with_epoch(work.generation as u32);
    Some(Search {
        work,
        prepared: Arc::new(prepared),
        launch: 0,
    })
}

fn startup_check(
    searcher: &mut CudaSearcher,
    depth: usize,
    batch: u32,
    stopping: &(dyn Fn() -> bool + Sync),
) -> Result<String, String> {
    let walk = searcher.walk();
    let count = batch.min(CHECK_NONCES);
    let per_thread = searcher.nonces_per_thread(count);
    let job = Job {
        header: [0x5a; 32],
        target: U512::MAX >> CHECK_BITS,
        nonce_high: [0x29; 32],
    };
    let prepared = job.prepare();
    let launches = depth as u32 + 1;
    let mut outcomes = Vec::new();
    let mut next = 0;
    while outcomes.len() < launches as usize {
        if depth == 1 {
            outcomes.push(searcher.search(&prepared, next, count));
            next += 1;
            continue;
        }
        while searcher.in_flight() < depth && next < launches {
            searcher
                .submit(&prepared, next, count)
                .map_err(|e| e.to_string())?;
            next += 1;
        }
        outcomes.push(searcher.collect());
    }
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    let mut valid = 0;
    for (launch, outcome) in outcomes.into_iter().enumerate() {
        let outcome = outcome.map_err(|e| e.to_string())?;
        let cpu = cpu_hits(&job, walk, launch as u32, per_thread, count, threads, stopping);
        if stopping() {
            return Err("stopped during the check against the CPU".into());
        }
        let gpu: Vec<[u8; 64]> = outcome.hits.iter().map(|hit| hit.nonce_be).collect();
        let agree = match outcome.dropped_candidates {
            0 => gpu.len() == cpu.len() && gpu.iter().all(|nonce| cpu.contains(nonce)),
            _ => gpu.iter().all(|nonce| cpu.contains(nonce)),
        };
        if !agree || outcome.false_candidates > 0 {
            return Err(format!(
                "the card found {} of the {} valid nonces of check launch {launch}, and {} that are not valid",
                gpu.len(),
                cpu.len(),
                outcome.false_candidates
            ));
        }
        valid += cpu.len();
    }
    if valid == 0 {
        return Err("the check against the CPU had no valid nonce to compare".into());
    }
    Ok(format!(
        "{valid} valid nonces in {launches} launches of {count} match the CPU"
    ))
}

pub fn run(slot: usize, options: Options, shared: Arc<Shared>, shares: Sender<Share>) {
    let stats = &shared.gpus[slot];
    let _crashed = PanicGuard(stats);
    let mut wait = REOPEN_AFTER.0;
    while !shared.stopping() {
        let opened = Instant::now();
        let Err(e) = run_once(slot, &options, &shared, &shares) else {
            return;
        };
        stats.failed.store(true, Ordering::SeqCst);
        if opened.elapsed() >= REOPEN_RESET {
            wait = REOPEN_AFTER.0;
        }
        log::error!(
            "GPU {} stopped: {e}; opening it again in {} s",
            stats.info.ordinal,
            wait.as_secs()
        );
        let until = Instant::now() + wait;
        while Instant::now() < until && !shared.stopping() {
            std::thread::sleep(Duration::from_millis(100));
        }
        wait = (wait * 2).min(REOPEN_AFTER.1);
    }
}

fn run_once(
    slot: usize,
    options: &Options,
    shared: &Arc<Shared>,
    shares: &Sender<Share>,
) -> Result<(), String> {
    let stats = &shared.gpus[slot];
    let mut batch = options.batch;
    let depth = options.pipeline.max(1);
    let (mut searcher, abort) = CudaSearcher::open(stats.info.ordinal)
        .and_then(|mut searcher| {
            if options.wait_mode != qtc_cuda::WaitMode::Blocking {
                searcher.set_wait_mode(options.wait_mode)?;
            }
            searcher.set_nonces_per_thread(options.nonces_per_thread);
            if options.whole_waves {
                let walk = searcher.walk();
                let per_thread = walk.fit_nonces_per_thread(
                    options
                        .nonces_per_thread
                        .unwrap_or_else(|| walk.default_nonces_per_thread(batch)),
                );
                let wave = searcher.concurrent_blocks()?;
                searcher.set_nonces_per_thread(Some(per_thread));
                batch = qtc_cuda::whole_waves(batch, per_thread, wave);
                log::info!(
                    "GPU {}: launches of {batch} nonces, {} waves of {wave} blocks, {per_thread} nonces per thread",
                    stats.info.ordinal,
                    batch as u64 / (wave as u64 * 256 * per_thread as u64)
                );
            }
            if options.launch_stats {
                searcher.enable_gpu_timing()?;
            }
            if depth > 1 {
                searcher.set_pipeline(depth, options.pipeline_streams.max(1))?;
            }
            let abort = match options.abort_on_job {
                true => Some(searcher.enable_abort()?),
                false => None,
            };
            Ok((searcher, abort))
        })
        .map_err(|e| e.to_string())?;
    if options.startup_check {
        let stopping = || shared.stopping();
        let report = startup_check(&mut searcher, depth, batch, &stopping);
        if shared.stopping() {
            return Ok(());
        }
        log::info!("GPU {}: {}", stats.info.ordinal, report?);
    }
    stats.failed.store(false, Ordering::SeqCst);
    let walk = searcher.walk();
    let per_thread = searcher.nonces_per_thread(batch);

    let mut check_threads = Vec::new();
    let checker = options.self_check.map(|check| {
        let (tasks, queue) = std::sync::mpsc::sync_channel(check.cpu_threads.max(1));
        let queue = Arc::new(Mutex::new(queue));
        for _ in 0..check.cpu_threads {
            let (shared, queue) = (shared.clone(), queue.clone());
            check_threads.push(std::thread::spawn(move || {
                check_launches(&shared, slot, walk, per_thread, &queue)
            }));
        }
        log::info!(
            "GPU {}: self-check on: the launches look for hashes below 2^-{} of the range, and {} CPU threads hash whole launches again",
            stats.info.ordinal,
            check.bits,
            check.cpu_threads
        );
        Checker {
            target: U512::MAX >> check.bits,
            tasks,
        }
    });
    let easy = checker.as_ref().map(|checker| checker.target);

    let finished = Arc::new(AtomicBool::new(false));
    let _watcher = abort.map(|handle| {
        let (shared, flag) = (shared.clone(), finished.clone());
        let ordinal = stats.info.ordinal;
        let thread = std::thread::spawn(move || {
            let mut seen = shared.generation();
            while !shared.stopping() && !flag.load(Ordering::SeqCst) {
                let generation = shared.wait_for_generation(seen, WATCH_POLL);
                if generation == seen {
                    continue;
                }
                seen = generation;
                if let Err(e) = handle.raise(generation as u32) {
                    log::warn!("GPU {ordinal}: {e}");
                    return;
                }
            }
        });
        WatcherGuard(finished.clone(), Some(thread))
    });
    let mut stats_log = StatsLog {
        enabled: options.launch_stats,
        at: Instant::now(),
        times: searcher.times(),
        hashes: 0,
        stale: 0,
    };

    let mut seen_generation = 0u64;
    let mut search: Option<Search> = None;
    let mut flights: VecDeque<Flight> = VecDeque::new();
    let mut result = Ok(());
    while !shared.stopping() {
        let generation = shared.generation();
        if generation != seen_generation {
            seen_generation = generation;
            let work = shared.work().filter(|work| work.generation == generation);
            search = work.clone().and_then(|work| start_search(work, easy));
            if let (Some(work), None) = (&work, &search) {
                log::error!(
                    "GPU {}: job {} has an extranonce of {} bytes, more than this miner can place; idle until the next job",
                    stats.info.ordinal,
                    work.job_id,
                    work.extranonce.len()
                );
            }
        }

        if depth == 1 {
            let Some(current) = search.as_mut() else {
                std::thread::sleep(IDLE_POLL);
                continue;
            };
            let flight = Flight {
                work: current.work.clone(),
                prepared: current.prepared.clone(),
                launch: current.take_launch(walk, batch, per_thread),
                count: batch,
                started: Instant::now(),
            };
            let outcome = match searcher.search(&flight.prepared, flight.launch, batch) {
                Ok(outcome) => outcome,
                Err(e) => {
                    result = Err(e.to_string());
                    break;
                }
            };
            if current.exhausted() {
                search = start_search(flight.work.clone(), easy);
            }
            if !finish(stats, shared, shares, checker.as_ref(), flight, outcome) {
                break;
            }
            stats_log.tick(&searcher, stats);
            continue;
        }

        while flights.len() < depth {
            let Some(current) = search.as_mut() else {
                break;
            };
            let launch = current.take_launch(walk, batch, per_thread);
            if let Err(e) = searcher.submit(&current.prepared, launch, batch) {
                result = Err(e.to_string());
                break;
            }
            flights.push_back(Flight {
                work: current.work.clone(),
                prepared: current.prepared.clone(),
                launch,
                count: batch,
                started: Instant::now(),
            });
            if current.exhausted() {
                search = start_search(current.work.clone(), easy);
            }
        }
        if result.is_err() {
            break;
        }
        let Some(flight) = flights.pop_front() else {
            std::thread::sleep(IDLE_POLL);
            continue;
        };
        let outcome = match searcher.collect() {
            Ok(outcome) => outcome,
            Err(e) => {
                result = Err(e.to_string());
                break;
            }
        };
        if !finish(stats, shared, shares, checker.as_ref(), flight, outcome) {
            break;
        }
        stats_log.tick(&searcher, stats);
    }
    drop(checker);
    for thread in check_threads {
        let _ = thread.join();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_is_extranonce_then_salt_then_zeros() {
        let prefix = nonce_prefix(&[0x3a, 0x37, 0xe8, 0x43], [1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        assert_eq!(prefix[..4], [0x3a, 0x37, 0xe8, 0x43]);
        assert_eq!(prefix[4..12], [1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(prefix[12..].iter().all(|&b| b == 0));
        assert_eq!(prefix.len(), 32);

        let no_extranonce = nonce_prefix(&[], [9; 8]).unwrap();
        assert_eq!(no_extranonce[..8], [9; 8]);
        assert!(nonce_prefix(&[0; 24], [0; 8]).is_some());
        assert!(nonce_prefix(&[0; 25], [0; 8]).is_none());
    }

    #[test]
    fn cpu_hits_are_the_same_on_any_number_of_threads() {
        let job = Job {
            header: [7; 32],
            target: U512::MAX >> 5,
            nonce_high: [3; 32],
        };
        let never = || false;
        for walk in [Walk::Sparse, Walk::TwoDirection] {
            let mut serial: Vec<[u8; 64]> = (0..1000)
                .filter_map(|index| job.verify_at(walk, 4, 16, index))
                .map(|hit| hit.nonce_be)
                .collect();
            serial.sort();
            assert!(serial.len() > 10);
            for threads in [1, 3, 8, 5000] {
                assert_eq!(cpu_hits(&job, walk, 4, 16, 1000, threads, &never), serial);
            }
            let stopped = || true;
            assert!(cpu_hits(&job, walk, 4, 16, 1000, 2, &stopped).is_empty());
        }
    }

    #[test]
    fn a_self_check_counts_every_hit_and_sends_only_shares() {
        let info = qtc_cuda::DeviceInfo {
            ordinal: 0,
            name: "test".into(),
            capability: 120,
            pci_bus_id: None,
        };
        let shared = Shared::new("pool".into(), vec![info]);
        shared.set_work("job".into(), [1; 32], U512::one(), U512::from(1000u64), vec![]);
        let work = shared.work().unwrap();
        let easy = U512::from(1_000_000u64);
        let search = start_search(work.clone(), Some(easy)).unwrap();
        assert_eq!(search.prepared.job.target, easy);
        let harder = start_search(work.clone(), Some(U512::from(10u64))).unwrap();
        assert_eq!(harder.prepared.job.target, work.target);

        let hit = |fill: u8, hash: u64| qtc_cuda::Hit {
            nonce_be: [fill; 64],
            hash: U512::from(hash),
        };
        let outcome = BatchOutcome {
            hashes: 2_000_000,
            hits: vec![hit(1, 999), hit(2, 1000), hit(3, 999_999)],
            false_candidates: 1,
            ..Default::default()
        };
        let flight = Flight {
            work: work.clone(),
            prepared: search.prepared.clone(),
            launch: 9,
            count: 2_000_000,
            started: Instant::now(),
        };
        let (tasks, queue) = std::sync::mpsc::sync_channel(1);
        let checker = Checker { target: easy, tasks };
        let (share_tx, share_rx) = std::sync::mpsc::channel();
        let stats = &shared.gpus[0];
        assert!(finish(stats, &shared, &share_tx, Some(&checker), flight, outcome));
        let sent: Vec<Share> = share_rx.try_iter().collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].nonce_be, [1; 64]);
        assert_eq!(stats.check.hits.load(Ordering::Relaxed), 3);
        assert_eq!(stats.false_candidates.load(Ordering::Relaxed), 1);
        let task = queue.try_recv().unwrap();
        assert_eq!((task.launch, task.count, task.found.len()), (9, 2_000_000, 3));
    }

    #[test]
    fn launch_numbers_follow_one_another() {
        let work = Arc::new(Work {
            generation: 1,
            job_id: "job".into(),
            header: [1; 32],
            difficulty: U512::one(),
            target: U512::MAX,
            extranonce: vec![],
            dev: false,
        });
        let mut search = start_search(work, None).unwrap();
        for walk in [Walk::Sparse, Walk::TwoDirection] {
            let first = search.launch;
            let taken: Vec<u32> = (0..50)
                .map(|_| search.take_launch(walk, 2_000_000, 16))
                .collect();
            assert_eq!(taken, (first..first + 50).collect::<Vec<_>>());
        }
        search.launch = qtc_cuda::MAX_LAUNCHES - 1;
        assert!(!search.exhausted());
        assert_eq!(
            search.take_launch(Walk::Sparse, 1000, 1),
            qtc_cuda::MAX_LAUNCHES - 1
        );
        assert!(search.exhausted());
    }

    #[test]
    fn two_searches_of_one_job_use_different_nonces() {
        let work = Arc::new(Work {
            generation: 1,
            job_id: "job".into(),
            header: [1; 32],
            difficulty: primitive_types::U512::one(),
            target: primitive_types::U512::MAX,
            extranonce: vec![0xaa, 0xbb, 0xcc, 0xdd],
            dev: false,
        });
        let a = start_search(work.clone(), None).unwrap();
        let b = start_search(work, None).unwrap();
        assert_eq!(a.prepared.job.nonce_high[..4], [0xaa, 0xbb, 0xcc, 0xdd]);
        assert_ne!(a.prepared.job.nonce_high, b.prepared.job.nonce_high);
    }
}
