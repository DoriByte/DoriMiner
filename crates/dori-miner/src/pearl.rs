//! Published without its comments: they describe the kernel, which is closed for now (see the README).
//! The code is all here.


use crate::pool::PoolConfig;
use crate::prl_pool::{self, Jobs};
use crate::state::Shared;
use crate::{api, sensors, ui};
use prl_pow::{ASide, Candidate, JobSide, Pair, ShareError};
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const REPORT_EVERY: Duration = Duration::from_secs(30);
const CANDIDATE_QUEUE: usize = 256;

pub fn format_rate(products_per_sec: f64) -> String {
    if products_per_sec >= 1e12 {
        format!("{:.1} TH/s", products_per_sec / 1e12)
    } else {
        format!("{:.1} GH/s", products_per_sec / 1e9)
    }
}

struct Draw(u64);

impl Draw {
    fn new() -> Self {
        let mut seed = [0u8; 8];
        getrandom::getrandom(&mut seed).expect("the system's random numbers");
        Self(u64::from_le_bytes(seed) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn byte(&mut self, cells: usize) -> (usize, i8) {
        let x = self.next();
        ((x % cells as u64) as usize, 1 + ((x >> 40) % prl_pow::MAX_BYTE as u64) as i8)
    }
}

struct Found {
    slot: usize,
    dev: bool,
    generation: u64,
    candidate: Candidate,
    alt: Option<u32>,
}

fn card(slot: usize, shared: Arc<Shared>, jobs: Arc<Jobs>, found: mpsc::SyncSender<Found>) {
    let gpu = &shared.gpus[slot];
    let opened = prl_cuda::Series::for_card(gpu.info.capability).and_then(|series| prl_cuda::Searcher::new(gpu.info.ordinal, series));
    let mut searcher = match opened {
        Ok(s) => s,
        Err(e) => {
            log::error!("GPU {}: {e}", gpu.info.ordinal);
            gpu.failed.store(true, Ordering::SeqCst);
            return;
        }
    };
    let shape = searcher.shape();
    let cells = searcher.range();
    let mut draw = Draw::new();
    let mut warned_full = false;
    while !shared.stopping() {
        let Some(work) = jobs.current() else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        let started = Instant::now();
        let job = match JobSide::new((*work.job).clone(), shape) {
            Ok(j) => Arc::new(j),
            Err(e) => {
                log::error!("job {} cannot be mined: {e}", work.job.id);
                while jobs.generation() == work.generation && !shared.stopping() {
                    std::thread::sleep(Duration::from_millis(50));
                }
                continue;
            }
        };
        let mut a = ASide::new(&shape, &job.key);
        let limit = prl_pow::words32(&job.limit.para_bytes_le());
        let result = (|| -> Result<(), String> {
            searcher.set_job(&job)?;
            log::debug!("GPU {}: job {} ready in {} ms", gpu.info.ordinal, job.job.id, started.elapsed().as_millis());
            let (pos, val) = draw.byte(cells);
            let mut pair = Pair::new(&job, &mut a, pos, val);
            let tables = searcher.upload(&pair)?;
            let mut launched = Instant::now();
            let mut queued = searcher.launch(&pair, tables, &limit)?;
            loop {
                let next = if jobs.generation() == work.generation && !shared.stopping() {
                    let (pos, val) = draw.byte(cells);
                    let p = Pair::new(&job, &mut a, pos, val);
                    let t = searcher.upload(&p)?;
                    let at = Instant::now();
                    let q = searcher.launch(&p, t, &limit)?;
                    Some((p, q, at))
                } else {
                    None
                };
                let tiles = searcher.tiles(queued)?;
                gpu.record_batch(launched, shape.products());
                for (row, col) in tiles {
                    let candidate = Candidate { job: job.clone(), pos: pair.pos, val: pair.val, seed_a: pair.seed_a, row, col };
                    let alt = searcher.alt(row);
                    if found.try_send(Found { slot, dev: work.dev, generation: work.generation, candidate, alt }).is_err() && !warned_full {
                        log::warn!("GPU {}: tiles are coming faster than they can be checked; some are dropped", gpu.info.ordinal);
                        warned_full = true;
                    }
                }
                let Some((p, q, at)) = next else { return Ok(()) };
                pair = p;
                queued = q;
                launched = at;
            }
        })();
        if let Err(e) = result {
            log::error!("GPU {}: {e}", gpu.info.ordinal);
            gpu.failed.store(true, Ordering::SeqCst);
            shared.needs_new_process.store(true, Ordering::SeqCst);
            shared.request_stop();
            return;
        }
    }
}

fn checker(shared: Arc<Shared>, found: mpsc::Receiver<Found>, shares: mpsc::Sender<prl_pool::Share>) {
    while let Ok(f) = found.recv() {
        if shared.stopping() {
            break;
        }
        let gpu = &shared.gpus[f.slot];
        let started = Instant::now();
        let mut candidate = f.candidate;
        if let Some(row) = f.alt.filter(|_| !prl_pow::twin_wins(&candidate)) {
            candidate.row = row;
        }
        match prl_pow::check_share(&candidate) {
            Ok(proof) => {
                log::info!(target: ui::DETAIL, "GPU {}: share found, checked in {} ms (proof {} bytes)", gpu.info.ordinal, started.elapsed().as_millis(), proof.len());
                let _ = shares.send(prl_pool::Share { dev: f.dev, generation: f.generation, job_id: candidate.job.job.id.clone(), proof });
            }
            Err(ShareError::TwinSaysNo) => {
                gpu.false_candidates.fetch_add(1, Ordering::Relaxed);
                log::warn!("GPU {}: a tile the card reported does not win by the CPU: not sent", gpu.info.ordinal);
            }
            Err(e) => {
                gpu.false_candidates.fetch_add(1, Ordering::Relaxed);
                log::error!("GPU {}: {e}: not sent", gpu.info.ordinal);
            }
        }
    }
}

const CHECK_TILES: f64 = 24.0;

pub fn bench(devices: Option<&str>, seconds: u64, check: bool) -> Result<(), String> {
    if seconds == 0 {
        return Err("--duration must be at least 1".into());
    }
    let infos = crate::open_devices(devices)?;
    for info in &infos {
        let series = prl_cuda::Series::for_card(info.capability).map_err(|e| format!("GPU {} {} (sm_{}): {e}", info.ordinal, info.name, info.capability))?;
        let ordinal = info.ordinal;
        let report = std::thread::spawn(move || bench_card(ordinal, series, seconds, check))
            .join()
            .map_err(|_| format!("the Pearl bench of GPU {ordinal} crashed"))??;
        println!("GPU {ordinal} {}: {report}", info.name);
    }
    Ok(())
}

fn bench_job(shape: prl_pow::Shape) -> Result<JobSide, String> {
    let header: String = (0..76u32).map(|i| format!("{:02x}", (i * 37 + 11) as u8)).collect();
    let target = if shape.h == 4 { "00000f" } else { "000001" };
    JobSide::new(prl_pow::PoolJob::parse("bench", &header, &format!("{target}{}", "ff".repeat(29)))?, shape)
}

fn bench_card(ordinal: usize, series: prl_cuda::Series, seconds: u64, check: bool) -> Result<String, String> {
    let mut searcher = prl_cuda::Searcher::new(ordinal, series)?;
    let shape = searcher.shape();
    let mut job = bench_job(shape)?;
    let tiles = (shape.m * shape.n / (shape.h as usize * 2 * shape.na)) as f64;
    let mut checked = String::new();
    if check {
        job.limit = prl_pow::U256([0, 0, 0, (CHECK_TILES / tiles * 2f64.powi(64)) as u64]);
        let shared = Arc::new(job);
        searcher.set_job(&shared)?;
        let mut a = ASide::new(&shape, &shared.key);
        let limit = prl_pow::words32(&shared.limit.para_bytes_le());
        let pairs: Vec<Pair> = [(12_345, 7), (987_654, 3), (55_555, 60)].iter().map(|&(pos, val)| Pair::new(&shared, &mut a, pos, val)).collect();
        let mut queued = std::collections::VecDeque::new();
        let mut counts = Vec::new();
        for i in 0..=pairs.len() {
            if let Some(pair) = pairs.get(i) {
                let tables = searcher.upload(pair)?;
                queued.push_back(searcher.launch(pair, tables, &limit)?);
            }
            if i == 0 {
                continue;
            }
            let pair = &pairs[i - 1];
            let (found, count) = searcher.tiles_counted(queued.pop_front().expect("a search per pair"))?;
            let wins = |row: u32, col: u32| prl_pow::twin_wins(&Candidate { job: shared.clone(), pos: pair.pos, val: pair.val, seed_a: pair.seed_a, row, col });
            let wrong: Vec<(u32, u32)> = found
                .iter()
                .copied()
                .filter(|&(row, col)| !wins(row, col) && !searcher.alt(row).is_some_and(|r| wins(r, col)))
                .collect();
            let spread = 4.0 * CHECK_TILES.sqrt();
            if !wrong.is_empty() || (count as f64) < CHECK_TILES - spread || (count as f64) > CHECK_TILES + spread {
                return Err(format!(
                    "GPU {ordinal}: the Pearl kernel does NOT match the CPU (pair {i} of {}): {count} tiles won (about {CHECK_TILES} expected), {} of the {} kept fail the CPU twin {wrong:?}",
                    pairs.len(),
                    wrong.len(),
                    found.len()
                ));
            }
            counts.push(count.to_string());
        }
        drop(pairs);
        checked = format!("matches the CPU ({} winning tiles in 3 pairs queued back to back, each confirmed by the CPU twin); ", counts.join(" + "));
        job = Arc::try_unwrap(shared).map_err(|_| "the check's job is still in use")?;
    }
    job.limit = prl_pow::U256::ZERO;
    let job = Arc::new(job);
    searcher.set_job(&job)?;
    let limit = prl_pow::words32(&job.limit.para_bytes_le());
    let mut a = ASide::new(&shape, &job.key);
    let mut draw = Draw::new();
    let cells = searcher.range();
    let (pos, val) = draw.byte(cells);
    let pair = Pair::new(&job, &mut a, pos, val);
    let mut tables = searcher.upload(&pair)?;
    let queued = searcher.launch(&pair, tables, &limit)?;
    searcher.tiles(queued)?;
    tables = searcher.upload(&pair)?;
    let (started, mut pairs) = (Instant::now(), 0u64);
    let mut queued = searcher.launch(&pair, tables, &limit)?;
    while started.elapsed() < Duration::from_secs(seconds) {
        let (pos, val) = draw.byte(cells);
        let next = Pair::new(&job, &mut a, pos, val);
        tables = searcher.upload(&next)?;
        let after = searcher.launch(&next, tables, &limit)?;
        searcher.tiles(queued)?;
        queued = after;
        pairs += 1;
    }
    searcher.tiles(queued)?;
    pairs += 1;
    let rate = (pairs * shape.products()) as f64 / started.elapsed().as_secs_f64();
    Ok(format!("{checked}{} ({} kernel, {pairs} pairs of {} x {} x {} in {seconds} s)", format_rate(rate), series.name(), shape.m, shape.n, shape.k))
}

pub struct Options {
    pub config: PoolConfig,
    pub dev_fee: u64,
    pub wallet: String,
    pub worker: Option<String>,
    pub devices: Option<String>,
    pub bind: Option<std::net::SocketAddr>,
}

pub fn mine(o: Options) -> Result<(), String> {
    if !o.wallet.starts_with("prl1") {
        log::warn!("a Pearl address starts with \"prl1\": the pool may refuse this one");
    }
    let infos = crate::open_devices(o.devices.as_deref())?;
    let unsupported: Vec<String> = infos.iter().filter(|i| !prl_cuda::supported(i.capability)).map(|i| format!("{} {} (sm_{})", i.ordinal, i.name, i.capability)).collect();
    for info in infos.iter().filter(|i| prl_cuda::supported(i.capability)) {
        prl_cuda::Series::for_card(info.capability).map_err(|e| format!("GPU {} {}: {e}", info.ordinal, info.name))?;
    }
    if !unsupported.is_empty() {
        return Err(format!(
            "Pearl mining needs an RTX 50 (sm_120), RTX 40 (sm_89) or RTX 30 (sm_86) card in this version: {}; choose the cards with -d",
            unsupported.join(", ")
        ));
    }
    let shared = Arc::new(Shared::new_for("pearl", o.config.url(), infos));
    {
        let shared = shared.clone();
        ctrlc::set_handler(move || shared.request_stop()).map_err(|e| format!("could not install the stop handler: {e}"))?;
    }
    if ui::fancy() {
        ui::print_start(&ui::Header {
            version: env!("CARGO_PKG_VERSION"),
            coin: "Pearl (PRL)",
            pool: &o.config.url(),
            wallet: &o.wallet,
            worker: o.worker.as_deref(),
            dev_fee: o.dev_fee,
            api: o.bind.map(|b| format!("http://{b}/summary")),
            gpus: shared.gpus.iter().map(|g| format!("{}  {}", g.info.ordinal, g.info.name)).collect(),
        });
        ui::set_title(&format!("DoriMiner {} · Pearl · starting", env!("CARGO_PKG_VERSION")));
    } else {
        log::info!(
            "DoriMiner {} mining Pearl ({}), pool {}",
            env!("CARGO_PKG_VERSION"),
            match o.dev_fee {
                0 => "no dev fee".to_string(),
                percent => format!("dev fee {percent}%, optional: --dev-fee 0 turns it off"),
            },
            o.config.url()
        );
    }
    if !prl_cuda::measured() {
        log::warn!("this build carries a Pearl kernel other than the measured one");
    }
    let api = o.bind.map(|bind| api::run(bind, shared.clone())).transpose()?;
    shared.fee.percent.store(o.dev_fee, Ordering::Relaxed);
    let jobs = Arc::new(Jobs::default());
    let (share_tx, all_shares) = mpsc::channel::<prl_pool::Share>();
    let (user_tx, user_rx) = mpsc::channel();
    let (dev_tx, dev_rx) = mpsc::channel();
    let (found_tx, found_rx) = mpsc::sync_channel(CANDIDATE_QUEUE);
    let mut threads = Vec::new();
    threads.push(std::thread::spawn(move || {
        for share in all_shares {
            let _ = if share.dev { dev_tx.send(share) } else { user_tx.send(share) };
        }
    }));
    {
        let (shared, jobs, config) = (shared.clone(), jobs.clone(), o.config.clone());
        threads.push(std::thread::spawn(move || prl_pool::run(config, shared, jobs, user_rx)));
    }
    if o.dev_fee > 0 {
        let config = PoolConfig {
            login: crate::devfee::dev_login_with(crate::devfee::DEV_WALLET_PEARL, &o.config.login),
            password: crate::devfee::DEV_PASSWORD.to_string(),
            dev: true,
            ..o.config.clone()
        };
        let (shared, jobs) = (shared.clone(), jobs.clone());
        threads.push(std::thread::spawn(move || prl_pool::run(config, shared, jobs, dev_rx)));
    }
    let mut fee = crate::devfee::Schedule::new(&shared, Instant::now());
    let checker = {
        let shared = shared.clone();
        std::thread::spawn(move || checker(shared, found_rx, share_tx))
    };
    for slot in 0..shared.gpus.len() {
        let (shared, jobs, found) = (shared.clone(), jobs.clone(), found_tx.clone());
        threads.push(std::thread::spawn(move || card(slot, shared, jobs, found)));
    }
    drop(found_tx);

    let nvml = if ui::fancy() { sensors::Nvml::open() } else { None };
    let mut last_report = Instant::now();
    while !shared.stopping() {
        std::thread::sleep(Duration::from_millis(200));
        if let Some(fee) = &mut fee {
            fee.tick(&shared, Instant::now());
        }
        if shared.gpus.iter().all(|g| g.failed.load(Ordering::SeqCst)) {
            log::error!("no card is mining");
            shared.request_stop();
        }
        if last_report.elapsed() >= REPORT_EVERY {
            last_report = Instant::now();
            let (accepted, rejected, stale) = (shared.accepted.load(Ordering::Relaxed), shared.rejected.load(Ordering::Relaxed), shared.stale.load(Ordering::Relaxed));
            let rate = format_rate(shared.total_hashrate());
            if ui::fancy() {
                let gpus: Vec<ui::GpuLine> = shared
                    .gpus
                    .iter()
                    .map(|g| ui::GpuLine {
                        ordinal: g.info.ordinal,
                        name: g.info.name.trim_start_matches("NVIDIA ").trim_start_matches("GeForce ").to_string(),
                        hashrate: format_rate(g.hashrate()),
                        failed: g.failed.load(Ordering::SeqCst),
                        sensors: nvml.as_ref().map(|n| n.read(g.info.pci_bus_id.as_deref(), g.info.ordinal)).unwrap_or_default(),
                    })
                    .collect();
                ui::print_report(&ui::Totals { hashrate: rate.clone(), accepted, rejected, stale, uptime_s: shared.started.elapsed().as_secs() }, &gpus);
                ui::set_title(&format!("DoriMiner {} · Pearl · {rate} · {accepted} accepted", env!("CARGO_PKG_VERSION")));
            } else {
                log::info!("{rate} | accepted {accepted} rejected {rejected} stale {stale}");
            }
        }
    }
    log::info!("stopping");
    for thread in threads.into_iter().chain(api) {
        let _ = thread.join();
    }
    let _ = checker.join();
    if shared.needs_new_process.load(Ordering::SeqCst) {
        return Err("mining stopped: a GPU needs a new process to work again (a manager starts it at once)".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_cover_a_and_stay_in_range() {
        let mut d = Draw::new();
        let cells = 65_536 * 8192;
        let mut seen_high = false;
        for _ in 0..10_000 {
            let (pos, val) = d.byte(cells);
            assert!(pos < cells && (1..=prl_pow::MAX_BYTE).contains(&val));
            seen_high |= pos > cells / 2 && val > 32;
        }
        assert!(seen_high);
    }

    #[test]
    fn rates_read_as_the_pearl_miners_give_them() {
        assert_eq!(format_rate(123.4e12), "123.4 TH/s");
        assert_eq!(format_rate(5.0e11), "500.0 GH/s");
    }
}
