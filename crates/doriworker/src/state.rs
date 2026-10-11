//! State shared by the pool client, the GPU workers and the local API.

use primitive_types::U512;
use qtc_cuda::DeviceInfo;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// What the hash rate is averaged over.
const RATE_WINDOW: Duration = Duration::from_secs(10);

/// The work every GPU is on. A new `generation` means drop the old search.
#[derive(Clone, Debug)]
pub struct Work {
    pub generation: u64,
    pub job_id: String,
    pub header: [u8; 32],
    pub difficulty: U512,
    pub target: U512,
    pub extranonce: Vec<u8>,
    /// A job of the dev fee link: its shares go there.
    pub dev: bool,
}

/// A nonce a GPU found and the CPU confirmed, on its way to the pool.
#[derive(Clone, Debug)]
pub struct Share {
    pub generation: u64,
    pub job_id: String,
    pub nonce_be: [u8; 64],
    pub hash: U512,
    /// Found for a job of the dev fee link.
    pub dev: bool,
}

/// Hashes over the trailing window, from the batches that completed in it.
#[derive(Default)]
pub struct RateWindow {
    batches: VecDeque<(Instant, u64)>,
    first: Option<Instant>,
}

impl RateWindow {
    /// `started` is when the batch was launched, `now` when it completed.
    pub fn record(&mut self, started: Instant, now: Instant, hashes: u64) {
        self.first.get_or_insert(started);
        self.batches.push_back((now, hashes));
        self.trim(now);
    }

    fn trim(&mut self, now: Instant) {
        while let Some(&(at, _)) = self.batches.front() {
            if now.duration_since(at) > RATE_WINDOW {
                self.batches.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn rate(&mut self, now: Instant) -> f64 {
        self.trim(now);
        let Some(first) = self.first else {
            return 0.0;
        };
        let hashes: u64 = self.batches.iter().map(|(_, h)| h).sum();
        let span = now.duration_since(first).min(RATE_WINDOW).as_secs_f64();
        if span <= 0.0 {
            0.0
        } else {
            hashes as f64 / span
        }
    }
}

/// What a self-check (`worker::SelfCheck`) has counted on a card.
#[derive(Default)]
pub struct CheckStats {
    /// Nonces the card reported below the check's target that the CPU hash
    /// confirmed. The ones it did not confirm are `false_candidates`.
    pub hits: AtomicU64,
    /// Nonces of the launches the CPU hashed again in full, the valid ones
    /// among them, and the valid ones the card had not reported.
    pub cpu_hashes: AtomicU64,
    pub cpu_hits: AtomicU64,
    pub cpu_missed: AtomicU64,
}

pub struct GpuStats {
    pub info: DeviceInfo,
    pub hashes: AtomicU64,
    pub launches: AtomicU64,
    /// Hashes of launches that ended after their job had been replaced.
    pub stale_hashes: AtomicU64,
    pub false_candidates: AtomicU64,
    /// Candidates beyond what a launch records: with a share target, none.
    pub dropped_candidates: AtomicU64,
    /// Set while the device is down: it failed and is waiting to be opened
    /// again, or its thread crashed. The API shows it.
    pub failed: AtomicBool,
    pub check: CheckStats,
    window: Mutex<RateWindow>,
}

impl GpuStats {
    pub fn new(info: DeviceInfo) -> Self {
        Self {
            info,
            hashes: AtomicU64::new(0),
            launches: AtomicU64::new(0),
            stale_hashes: AtomicU64::new(0),
            false_candidates: AtomicU64::new(0),
            dropped_candidates: AtomicU64::new(0),
            failed: AtomicBool::new(false),
            check: CheckStats::default(),
            window: Mutex::new(RateWindow::default()),
        }
    }

    pub fn record_batch(&self, started: Instant, hashes: u64) {
        self.hashes.fetch_add(hashes, Ordering::Relaxed);
        self.launches.fetch_add(1, Ordering::Relaxed);
        self.window
            .lock()
            .unwrap()
            .record(started, Instant::now(), hashes);
    }

    pub fn hashrate(&self) -> f64 {
        self.window.lock().unwrap().rate(Instant::now())
    }
}

/// The optional dev fee: now and then the GPUs mine for the DoriByte wallet for a minute. The schedule
/// (`devfee.rs`) sets `wanted` and `active`, the second pool link sets `ready` and counts its shares.
#[derive(Default)]
pub struct Fee {
    /// Percent of the time, 0 when off. Set once, before the threads start.
    pub percent: AtomicU64,
    /// The dev fee link should be connected.
    pub wanted: AtomicBool,
    /// The dev fee link is logged in and holds a job.
    pub ready: AtomicBool,
    /// The GPUs are on the dev fee link's jobs.
    pub active: AtomicBool,
    pub accepted: AtomicU64,
    pub rejected: AtomicU64,
    pub stale: AtomicU64,
    /// Whole seconds mined for the dev fee so far.
    pub seconds: AtomicU64,
}

pub struct Shared {
    work: Mutex<Option<Arc<Work>>>,
    /// Signalled whenever the generation changes.
    changed: Condvar,
    generation: AtomicU64,
    stop: AtomicBool,
    pub started: Instant,
    pub pool_url: String,
    pub pool_connected: AtomicBool,
    pub accepted: AtomicU64,
    pub rejected: AtomicU64,
    /// Shares the pool called stale, plus the ones dropped before sending
    /// because the job had already changed.
    pub stale: AtomicU64,
    /// Shares sent although their job had changed (`--submit-late`), and how
    /// many of those the pool accepted.
    pub late_sent: AtomicU64,
    pub late_accepted: AtomicU64,
    pub fee: Fee,
    pub gpus: Vec<GpuStats>,
}

impl Shared {
    pub fn new(pool_url: String, devices: Vec<DeviceInfo>) -> Self {
        Self {
            work: Mutex::new(None),
            changed: Condvar::new(),
            generation: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            started: Instant::now(),
            pool_url,
            pool_connected: AtomicBool::new(false),
            accepted: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            stale: AtomicU64::new(0),
            late_sent: AtomicU64::new(0),
            late_accepted: AtomicU64::new(0),
            fee: Fee::default(),
            gpus: devices.into_iter().map(GpuStats::new).collect(),
        }
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.changed.notify_all();
    }

    /// Sleeps until the generation is no longer `seen`, a stop is requested
    /// or `timeout` passes, and returns the generation. A change wakes the
    /// caller at once: it does not wait for a timer.
    pub fn wait_for_generation(&self, seen: u64, timeout: Duration) -> u64 {
        let guard = self.work.lock().unwrap();
        let _unused = self
            .changed
            .wait_timeout_while(guard, timeout, |_| {
                self.generation() == seen && !self.stopping()
            })
            .unwrap();
        self.generation()
    }

    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// The generation the GPUs should be working on. Changes on every new job
    /// and when work is withdrawn.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn work(&self) -> Option<Arc<Work>> {
        self.work.lock().unwrap().clone()
    }

    /// Replaces the current work and returns its generation.
    pub fn set_work(
        &self,
        job_id: String,
        header: [u8; 32],
        difficulty: U512,
        target: U512,
        extranonce: Vec<u8>,
    ) -> u64 {
        self.set_work_of(false, job_id, header, difficulty, target, extranonce)
    }

    /// The same, saying which link the job came from.
    pub fn set_work_of(
        &self,
        dev: bool,
        job_id: String,
        header: [u8; 32],
        difficulty: U512,
        target: U512,
        extranonce: Vec<u8>,
    ) -> u64 {
        let mut slot = self.work.lock().unwrap();
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *slot = Some(Arc::new(Work {
            generation,
            job_id,
            header,
            difficulty,
            target,
            extranonce,
            dev,
        }));
        self.changed.notify_all();
        generation
    }

    /// Stops the GPUs until the next job (pool connection lost).
    pub fn clear_work(&self) {
        let mut slot = self.work.lock().unwrap();
        self.generation.fetch_add(1, Ordering::SeqCst);
        *slot = None;
        self.changed.notify_all();
    }

    pub fn total_hashrate(&self) -> f64 {
        self.gpus.iter().map(GpuStats::hashrate).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_is_hashes_over_the_window() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut window = RateWindow::default();
        assert_eq!(window.rate(t0), 0.0);

        // Ten 1-second batches of 600 hashes each.
        for i in 0..10 {
            window.record(at(i * 1000), at((i + 1) * 1000), 600);
        }
        assert_eq!(window.rate(at(10_000)), 600.0);

        // Early on, the span is the time since the first launch, not the full window.
        let mut early = RateWindow::default();
        early.record(at(0), at(2000), 1000);
        assert_eq!(early.rate(at(2000)), 500.0);
    }

    #[test]
    fn rate_falls_to_zero_when_batches_stop() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut window = RateWindow::default();
        window.record(at(0), at(1000), 1000);
        assert!(window.rate(at(5000)) > 0.0);
        assert_eq!(window.rate(at(12_000)), 0.0);
    }

    #[test]
    fn a_job_change_wakes_a_waiter_at_once() {
        let shared = Arc::new(Shared::new("pool".into(), Vec::new()));
        // Nothing changes: the wait runs out.
        let start = Instant::now();
        assert_eq!(shared.wait_for_generation(0, Duration::from_millis(30)), 0);
        assert!(start.elapsed() >= Duration::from_millis(25));
        // Already changed: no wait at all.
        shared.set_work("a".into(), [0; 32], U512::one(), U512::MAX, vec![]);
        assert_eq!(shared.wait_for_generation(0, Duration::from_secs(5)), 1);

        let waiter = {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let generation = shared.wait_for_generation(1, Duration::from_secs(20));
                (generation, Instant::now())
            })
        };
        std::thread::sleep(Duration::from_millis(50));
        let changed = Instant::now();
        shared.clear_work();
        let (generation, woke) = waiter.join().unwrap();
        assert_eq!(generation, 2);
        assert!(woke.duration_since(changed) < Duration::from_secs(2));

        // A stop ends the wait too.
        let waiter = {
            let shared = shared.clone();
            std::thread::spawn(move || shared.wait_for_generation(2, Duration::from_secs(20)))
        };
        std::thread::sleep(Duration::from_millis(50));
        shared.request_stop();
        assert_eq!(waiter.join().unwrap(), 2);
    }

    #[test]
    fn every_job_change_bumps_the_generation() {
        let shared = Shared::new("pool".into(), Vec::new());
        assert!(shared.work().is_none());
        let g1 = shared.set_work(
            "a".into(),
            [0; 32],
            U512::one(),
            U512::MAX,
            vec![1, 2, 3, 4],
        );
        assert_eq!(shared.generation(), g1);
        assert_eq!(shared.work().unwrap().job_id, "a");
        shared.clear_work();
        assert!(shared.work().is_none());
        assert_ne!(shared.generation(), g1);
        let g2 = shared.set_work("b".into(), [0; 32], U512::one(), U512::MAX, vec![]);
        assert!(g2 > g1);
    }
}
