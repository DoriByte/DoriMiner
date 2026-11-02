
use crate::erg_pool::{self, Jobs, Share};
use crate::pool::PoolConfig;
use crate::state::Shared;
use crate::{api, sensors, ui};
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const REPORT_EVERY: Duration = Duration::from_secs(30);
pub const NOTICE_TABLE: &str = "erg_compact_table";
const CHECK_NONCES: usize = 4;
const BENCH_HEIGHT: u32 = 1_904_531;

pub fn format_rate(per_sec: f64) -> String {
    format!("{:.2} MH/s", per_sec / 1e6)
}

pub fn format_difficulty(d: f64) -> String {
    match d {
        d if d >= 1e12 => format!("{:.2} T", d / 1e12),
        d if d >= 1e9 => format!("{:.2} G", d / 1e9),
        d if d >= 1e6 => format!("{:.2} M", d / 1e6),
        d if d >= 1e3 => format!("{:.2} K", d / 1e3),
        d => format!("{d:.0}"),
    }
}

fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    getrandom::getrandom(&mut b).expect("the system's random numbers");
    u64::from_le_bytes(b)
}

fn table(engine: &mut erg_cuda::Engine, gpu: &crate::state::GpuStats, height: u32) -> Result<(), String> {
    let ordinal = gpu.info.ordinal;
    let started = Instant::now();
    let made = engine.set_height(height)?;
    let compact = engine.compact();
    if compact && !gpu.notices.lock().unwrap().contains(&NOTICE_TABLE) {
        log::warn!(
            "GPU {ordinal}: the table for height {height} does not fit on this card in full ({:.2} GB): \
             {:.0} % of it is kept in the engine's compact form ({:.2} GB), which mines slower",
            erg_cuda::table_bytes(height) as f64 / 1e9,
            100.0 * engine.level(),
            engine.table_bytes() as f64 / 1e9
        );
    }
    gpu.set_notice(NOTICE_TABLE, compact);
    if made {
        let layout = if compact { format!(", {:.0} % in compact form", 100.0 * engine.level()) } else { String::new() };
        log::info!(target: ui::DETAIL, "GPU {ordinal}: table for height {height} ({:.2} GB{layout}) made in {:.1} s", engine.table_bytes() as f64 / 1e9, started.elapsed().as_secs_f64());
    } else {
        log::info!(target: ui::DETAIL, "GPU {ordinal}: table for height {height} ready");
    }
    Ok(())
}

fn card(slot: usize, shared: Arc<Shared>, jobs: Arc<Jobs>, shares: mpsc::Sender<Share>) {
    let gpu = &shared.gpus[slot];
    let ordinal = gpu.info.ordinal;
    let mut engine = match erg_cuda::Engine::open(ordinal) {
        Ok(e) => e,
        Err(e) => {
            log::error!("GPU {ordinal}: {e}");
            gpu.failed.store(true, Ordering::SeqCst);
            return;
        }
    };
    let mut checked_height = 0u32;
    while !shared.stopping() {
        let Some(work) = jobs.current() else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        let job = work.job.clone();
        let result = (|| -> Result<(), String> {
            if engine.height() != job.height {
                table(&mut engine, gpu, job.height)?;
            }
            if checked_height != job.height {
                erg_cuda::self_check(&mut engine, &job.msg, random_u64(), CHECK_NONCES)?;
                if checked_height == 0 {
                    log::info!(target: ui::GOOD, "GPU {ordinal}: matches the CPU (Autolykos v2, height {})", job.height);
                }
                checked_height = job.height;
            }
            let size = engine.launch_size();
            let per_launch = engine.per_launch(size);
            let prefix = job.nonce_prefix()?;
            let space: u64 = if job.extranonce2_size >= 8 { u64::MAX } else { (1u64 << (8 * job.extranonce2_size)) - 1 };
            if space < per_launch {
                return Err(format!("an extranonce2 of {} bytes is too small for a launch of {per_launch} nonces", job.extranonce2_size));
            }
            let mut next = random_u64() % (space - per_launch + 1);
            while jobs.generation() == work.generation && !shared.stopping() {
                if next > space - per_launch {
                    next = random_u64() % (space - per_launch + 1);
                }
                let started = Instant::now();
                let found = engine.mine(&job.msg, &job.target, prefix | next, size)?;
                gpu.record_batch(started, per_launch);
                for nonce in found {
                    let hit = erg_cuda::hit(&job.msg, nonce, job.height);
                    if erg_cuda::meets(&hit, &job.target) {
                        log::info!(target: ui::DETAIL, "GPU {ordinal}: share found (job {})", job.id);
                        let _ = shares.send(Share {
                            dev: work.dev,
                            generation: work.generation,
                            job_id: job.id.clone(),
                            extranonce1: job.extranonce1.clone(),
                            extranonce2_size: job.extranonce2_size,
                            nonce,
                        });
                    } else if (u64::from_be_bytes(hit[..8].try_into().unwrap())) > erg_cuda::target_top(&job.target) {
                        log::warn!("GPU {ordinal}: a candidate the CPU does not confirm (nonce {nonce:016x}, height {})", job.height);
                    }
                }
                next += per_launch;
            }
            Ok(())
        })();
        if let Err(e) = result {
            log::error!("GPU {ordinal}: {e}");
            gpu.failed.store(true, Ordering::SeqCst);
            shared.needs_new_process.store(true, Ordering::SeqCst);
            shared.request_stop();
            return;
        }
    }
}

pub fn bench(devices: Option<&str>, seconds: u64, check: bool) -> Result<(), String> {
    if seconds == 0 {
        return Err("--duration must be at least 1".into());
    }
    let infos = crate::open_devices(devices)?;
    for info in &infos {
        let ordinal = info.ordinal;
        let report = std::thread::spawn(move || -> Result<String, String> {
            let mut engine = erg_cuda::Engine::open(ordinal)?;
            let msg: [u8; 32] = std::array::from_fn(|k| (random_u64() >> (k % 8)) as u8);
            let t0 = Instant::now();
            let height = std::env::var("ERG_BENCH_HEIGHT").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(BENCH_HEIGHT);
            engine.set_height(height)?;
            let made = t0.elapsed().as_secs_f64();
            let mut checked = String::new();
            if check {
                erg_cuda::self_check(&mut engine, &msg, random_u64(), 16)?;
                checked = "matches the CPU; ".into();
            }
            let size = engine.launch_size();
            let per = engine.per_launch(size);
            let impossible = primitive_types::U256::zero();
            engine.mine(&msg, &impossible, 0, size)?;
            let (started, mut done) = (Instant::now(), 0u64);
            while started.elapsed() < Duration::from_secs(seconds) {
                engine.mine(&msg, &impossible, done + per, size)?;
                done += per;
            }
            let rate = done as f64 / started.elapsed().as_secs_f64();
            let layout = if engine.compact() { format!(", {:.0} % in compact form", 100.0 * engine.level()) } else { String::new() };
            Ok(format!("{checked}{} ({done} nonces in {seconds} s); table of height {height}{layout} made in {made:.1} s", format_rate(rate)))
        })
        .join()
        .map_err(|_| format!("the ERG bench of GPU {ordinal} crashed"))??;
        println!("GPU {ordinal} {}: {report}", info.name);
    }
    Ok(())
}

pub fn pool_check(config: PoolConfig, seconds: u64) -> Result<(), String> {
    let shared = Arc::new(Shared::new_for("autolykos2", config.url(), Vec::new()));
    let jobs = Arc::new(Jobs::default());
    let (_share_tx, share_rx) = mpsc::channel();
    let client = {
        let (shared, jobs) = (shared.clone(), jobs.clone());
        std::thread::spawn(move || erg_pool::run(config, shared, jobs, share_rx))
    };
    let started = Instant::now();
    let (mut seen, mut count) = (0u64, 0u64);
    while started.elapsed() < Duration::from_secs(seconds) {
        std::thread::sleep(Duration::from_millis(50));
        if jobs.generation() != seen {
            seen = jobs.generation();
            if let Some(work) = jobs.current() {
                count += 1;
                let j = &work.job;
                println!("job {} height {} share difficulty {} extranonce1 {} ({} + {} bytes)", j.id, j.height, format_difficulty(erg_cuda::difficulty(&j.target)), j.extranonce1, j.extranonce1.len() / 2, j.extranonce2_size);
            }
        }
    }
    let connected = shared.pool_connected.load(Ordering::SeqCst);
    shared.request_stop();
    let _ = client.join();
    println!("{count} jobs in {seconds} s, connected at the end: {connected}");
    if count == 0 {
        return Err("the pool sent no job".into());
    }
    Ok(())
}

pub struct Options {
    pub config: PoolConfig,
    pub dev_fee: u64,
    pub lithos: bool,
    pub wallet: String,
    pub worker: Option<String>,
    pub devices: Option<String>,
    pub bind: Option<std::net::SocketAddr>,
}

pub fn mine(o: Options) -> Result<(), String> {
    if o.lithos {
        log::info!("Lithos: mining to the client at {}; it pays its own node's wallet", o.config.url());
    } else if !o.wallet.starts_with('9') {
        log::warn!("an Ergo mainnet address starts with \"9\": the pool may refuse this one");
    }
    let infos = crate::open_devices(o.devices.as_deref())?;
    let old: Vec<String> = infos.iter().filter(|i| i.capability < 75).map(|i| format!("{} {} (sm_{})", i.ordinal, i.name, i.capability)).collect();
    if !old.is_empty() {
        return Err(format!("Ergo mining needs an RTX 20, 30, 40 or 50 card: {}; choose the cards with -d", old.join(", ")));
    }
    let shared = Arc::new(Shared::new_for("autolykos2", o.config.url(), infos));
    {
        let shared = shared.clone();
        ctrlc::set_handler(move || shared.request_stop()).map_err(|e| format!("could not install the stop handler: {e}"))?;
    }
    let dev_fee = if crate::devfee::DEV_WALLET_ERG.is_some() { o.dev_fee } else { 0 };
    if o.dev_fee > 0 && dev_fee == 0 {
        log::info!("Ergo: no dev fee in this version");
    }
    if ui::fancy() {
        ui::print_start(&ui::Header {
            version: env!("CARGO_PKG_VERSION"),
            coin: "Ergo (ERG)",
            pool: &o.config.url(),
            wallet: &o.wallet,
            worker: o.worker.as_deref(),
            dev_fee,
            api: o.bind.map(|b| format!("http://{b}/summary")),
            gpus: shared.gpus.iter().map(|g| format!("{}  {}", g.info.ordinal, g.info.name)).collect(),
        });
        ui::set_title(&format!("DoriMiner {} · Ergo · starting", env!("CARGO_PKG_VERSION")));
    } else {
        log::info!("DoriMiner {} mining Ergo, pool {}", env!("CARGO_PKG_VERSION"), o.config.url());
    }
    let api = o.bind.map(|bind| api::run(bind, shared.clone())).transpose()?;
    shared.fee.percent.store(dev_fee, Ordering::Relaxed);
    let jobs = Arc::new(Jobs::default());
    let (share_tx, all_shares) = mpsc::channel::<Share>();
    let (user_tx, user_rx) = mpsc::channel();
    let (dev_tx, dev_rx) = mpsc::channel();
    let mut threads = Vec::new();
    threads.push(std::thread::spawn(move || {
        for share in all_shares {
            let _ = if share.dev { dev_tx.send(share) } else { user_tx.send(share) };
        }
    }));
    {
        let (shared, jobs, config) = (shared.clone(), jobs.clone(), o.config.clone());
        threads.push(std::thread::spawn(move || erg_pool::run(config, shared, jobs, user_rx)));
    }
    if let (true, Some(wallet)) = (dev_fee > 0, crate::devfee::DEV_WALLET_ERG) {
        let config = PoolConfig {
            login: crate::devfee::dev_login_with(wallet, &o.config.login),
            password: crate::devfee::DEV_PASSWORD.to_string(),
            dev: true,
            ..o.config.clone()
        };
        let (shared, jobs) = (shared.clone(), jobs.clone());
        threads.push(std::thread::spawn(move || erg_pool::run(config, shared, jobs, dev_rx)));
    } else {
        drop(dev_rx);
    }
    let mut fee = if dev_fee > 0 { crate::devfee::Schedule::new(&shared, Instant::now()) } else { None };
    for slot in 0..shared.gpus.len() {
        let (shared, jobs, shares) = (shared.clone(), jobs.clone(), share_tx.clone());
        threads.push(std::thread::spawn(move || card(slot, shared, jobs, shares)));
    }
    drop(share_tx);
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
                ui::set_title(&format!("DoriMiner {} · Ergo · {rate} · {accepted} accepted", env!("CARGO_PKG_VERSION")));
            } else {
                log::info!("{rate} | accepted {accepted} rejected {rejected} stale {stale}");
            }
        }
    }
    log::info!("stopping");
    for thread in threads.into_iter().chain(api) {
        let _ = thread.join();
    }
    if shared.needs_new_process.load(Ordering::SeqCst) {
        return Err("mining stopped: a GPU needs a new process to work again (a manager starts it at once)".into());
    }
    Ok(())
}
