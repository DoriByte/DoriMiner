use primitive_types::U512;
use qtc_cuda::DeviceInfo;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const RATE_WINDOW: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct Work {
    pub generation: u64,
    pub job_id: String,
    pub header: [u8; 32],
    pub difficulty: U512,
    pub target: U512,
    pub extranonce: Vec<u8>,

    pub dev: bool,
}

#[derive(Clone, Debug)]
pub struct Share {
    pub generation: u64,
    pub job_id: String,
    pub nonce_be: [u8; 64],
    pub hash: U512,

    pub dev: bool,
}

#[derive(Default)]
pub struct RateWindow {
    batches: VecDeque<(Instant, u64)>,
    first: Option<Instant>,
}

impl RateWindow {

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

#[derive(Default)]
pub struct CheckStats {

    pub hits: AtomicU64,

    pub cpu_hashes: AtomicU64,
    pub cpu_hits: AtomicU64,
    pub cpu_missed: AtomicU64,
}

pub struct GpuStats {
    pub info: DeviceInfo,
    pub hashes: AtomicU64,
    pub launches: AtomicU64,

    pub stale_hashes: AtomicU64,
    pub false_candidates: AtomicU64,

    pub dropped_candidates: AtomicU64,

    pub failed: AtomicBool,
    pub check: CheckStats,

    pub notices: Mutex<Vec<&'static str>>,
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
            notices: Mutex::new(Vec::new()),
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

    pub fn set_notice(&self, code: &'static str, on: bool) {
        let mut n = self.notices.lock().unwrap();
        n.retain(|c| *c != code);
        if on {
            n.push(code);
        }
    }
}

#[derive(Default)]
pub struct Fee {

    pub percent: AtomicU64,

    pub wanted: AtomicBool,

    pub ready: AtomicBool,

    pub active: AtomicBool,
    pub accepted: AtomicU64,
    pub rejected: AtomicU64,
    pub stale: AtomicU64,

    pub seconds: AtomicU64,
}

pub struct Shared {
    work: Mutex<Option<Arc<Work>>>,

    changed: Condvar,
    generation: AtomicU64,
    stop: AtomicBool,

    pub needs_new_process: AtomicBool,

    pub algo: &'static str,
    pub started: Instant,
    pub pool_url: String,
    pub pool_connected: AtomicBool,
    pub accepted: AtomicU64,
    pub rejected: AtomicU64,

    pub stale: AtomicU64,

    pub late_sent: AtomicU64,
    pub late_accepted: AtomicU64,
    pub fee: Fee,
    pub gpus: Vec<GpuStats>,

    pub model: Mutex<Option<ModelState>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelState {
    pub state: &'static str,
    pub bytes: u64,
    pub total: u64,
    pub error: Option<String>,
}

impl Shared {
    pub fn new(pool_url: String, devices: Vec<DeviceInfo>) -> Self {
        Self::new_for("quantus", pool_url, devices)
    }

    pub fn new_for(algo: &'static str, pool_url: String, devices: Vec<DeviceInfo>) -> Self {
        Self {
            algo,
            work: Mutex::new(None),
            changed: Condvar::new(),
            generation: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            needs_new_process: AtomicBool::new(false),
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
            model: Mutex::new(None),
        }
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.changed.notify_all();
    }

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

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn work(&self) -> Option<Arc<Work>> {
        self.work.lock().unwrap().clone()
    }

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

        for i in 0..10 {
            window.record(at(i * 1000), at((i + 1) * 1000), 600);
        }
        assert_eq!(window.rate(at(10_000)), 600.0);

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

        let start = Instant::now();
        assert_eq!(shared.wait_for_generation(0, Duration::from_millis(30)), 0);
        assert!(start.elapsed() >= Duration::from_millis(25));

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
