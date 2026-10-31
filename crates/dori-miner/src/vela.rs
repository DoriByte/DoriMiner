//! Mining Vela (VELA): `--coin vela`. Proof-of-Inference: each attempt is one step of the sd-turbo integer UNet on the
//! job's prompt (vela-cuda, exact to the official `sdint.py`), so the rate is in inferences a second (inf/s), as the
//! Vela miners give it. A thread per card runs batches of attempts; the CPU hashes each x0 (hashPoI, then the block
//! hash) against the share's target, and the pool link (vela_pool.rs) sends the ones that meet it.
//!
//! The model (sd-turbo-int.poisd, about 1.2 GB) is downloaded on the first start from the pool's server, checked by its
//! SHA-256 and kept in the data folder (or given with --vela-model); `GET /summary` shows the download as `model`.
//! Each card checks itself against the official test vector before mining.

use crate::pool::PoolConfig;
use crate::state::{ModelState, Shared};
use crate::vela_pool::{self, Jobs, Share};
use crate::{api, sensors, ui};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const REPORT_EVERY: Duration = Duration::from_secs(30);

/// Inferences a second as text.
pub fn format_rate(per_sec: f64) -> String {
    format!("{per_sec:.1} inf/s")
}

/// The miner's data folder: DORI_DATA_DIR, else %LOCALAPPDATA%\DoriMiner (Windows) or $XDG_DATA_HOME/dori-miner,
/// ~/.local/share/dori-miner (Linux). Kept across versions, so the model is downloaded once.
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("DORI_DATA_DIR") {
        return PathBuf::from(d);
    }
    if cfg!(windows) {
        if let Some(d) = std::env::var_os("LOCALAPPDATA") {
            return Path::new(&d).join("DoriMiner");
        }
    } else {
        if let Some(d) = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
            return Path::new(&d).join("dori-miner");
        }
        if let Some(h) = std::env::var_os("HOME") {
            return Path::new(&h).join(".local").join("share").join("dori-miner");
        }
    }
    PathBuf::from("data")
}

pub fn default_model_path() -> PathBuf {
    data_dir().join("vela").join(vela_cuda::MODEL_FILE)
}

fn set_model(shared: Option<&Shared>, state: &'static str, bytes: u64, total: u64, error: Option<String>) {
    if let Some(s) = shared {
        *s.model.lock().unwrap() = Some(ModelState { state, bytes, total, error });
    }
}

/// SHA-256 of a file, as hex, reporting the bytes read.
fn file_sha256(path: &Path, shared: Option<&Shared>) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let total = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        done += n as u64;
        set_model(shared, "verifying", done, total, None);
    }
    Ok(vela_cuda::hex(&h.finalize()))
}

fn download(url: &str, to: &Path, shared: Option<&Shared>) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).timeout_read(Duration::from_secs(60)).build();
    let resp = agent.get(url).set("User-Agent", &format!("DoriMiner/{}", env!("CARGO_PKG_VERSION"))).call().map_err(|e| format!("{url}: {e}"))?;
    let total = resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let mut body = resp.into_reader();
    let mut out = std::fs::File::create(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let mut buf = vec![0u8; 1 << 20];
    let (mut done, mut logged) = (0u64, Instant::now());
    loop {
        let n = body.read(&mut buf).map_err(|e| format!("{url}: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", to.display()))?;
        done += n as u64;
        set_model(shared, "downloading", done, total, None);
        if logged.elapsed() > Duration::from_secs(10) {
            logged = Instant::now();
            log::info!("model: {} of {} MB", done >> 20, total >> 20);
        }
    }
    out.sync_all().map_err(|e| format!("{}: {e}", to.display()))?;
    if total > 0 && done != total {
        return Err(format!("{url}: the download stopped at {done} of {total} bytes"));
    }
    Ok(())
}

/// The model ready to read: the given file (checked), or the one in the data folder (downloaded when missing or
/// wrong, from the pool's server and then its mirror, checked, written as .part and renamed).
pub fn ensure_model(given: Option<&Path>, shared: Option<&Shared>) -> Result<PathBuf, String> {
    let fail = |e: String| {
        set_model(shared, "error", 0, 0, Some(e.clone()));
        e
    };
    if let Some(p) = given {
        let sha = file_sha256(p, shared).map_err(fail)?;
        if sha != vela_cuda::MODEL_SHA256 {
            return Err(fail(format!("{} is not the Vela model (SHA-256 {sha}, expected {})", p.display(), vela_cuda::MODEL_SHA256)));
        }
        set_model(shared, "ready", 0, 0, None);
        return Ok(p.to_path_buf());
    }
    let path = default_model_path();
    if path.exists() {
        log::info!("model: checking {}", path.display());
        match file_sha256(&path, shared) {
            Ok(sha) if sha == vela_cuda::MODEL_SHA256 => {
                set_model(shared, "ready", 0, 0, None);
                return Ok(path);
            }
            Ok(sha) => log::warn!("model: {} has SHA-256 {sha}, not the Vela model's: downloading it again", path.display()),
            Err(e) => log::warn!("model: {e}: downloading it again"),
        }
    }
    let dir = path.parent().expect("a folder");
    std::fs::create_dir_all(dir).map_err(|e| fail(format!("{}: {e}", dir.display())))?;
    let part = path.with_extension("poisd.part");
    let mut last = String::new();
    for url in vela_cuda::MODEL_URLS {
        log::info!("model: downloading {url} (about 1.2 GB, once)");
        match download(url, &part, shared).and_then(|()| file_sha256(&part, shared)) {
            Ok(sha) if sha == vela_cuda::MODEL_SHA256 => {
                std::fs::rename(&part, &path).map_err(|e| fail(format!("{}: {e}", path.display())))?;
                set_model(shared, "ready", 0, 0, None);
                log::info!(target: ui::GOOD, "model: ready at {}", path.display());
                return Ok(path);
            }
            Ok(sha) => last = format!("{url} gave a file with SHA-256 {sha}, not the Vela model's"),
            Err(e) => last = e,
        }
        log::warn!("model: {last}");
        let _ = std::fs::remove_file(&part);
    }
    Err(fail(format!("the Vela model could not be downloaded: {last}")))
}

/// An extranonce2 of `size` bytes, random: two cards (or two machines) never mine the same attempts.
fn extranonce2(size: usize) -> String {
    let mut b = vec![0u8; size];
    getrandom::getrandom(&mut b).expect("the system's random numbers");
    vela_cuda::hex(&b)
}

/// One card: open the engine, check it against the official vector, then batches of attempts on the current job.
fn card(slot: usize, shared: Arc<Shared>, jobs: Arc<Jobs>, model: Arc<PathBuf>, shares: mpsc::Sender<Share>) {
    let gpu = &shared.gpus[slot];
    let ordinal = gpu.info.ordinal;
    let opened = vela_cuda::Engine::open(ordinal, &model, vela_cuda::DEFAULT_BATCH).and_then(|mut e| vela_cuda::self_check(&mut e).map(|()| e));
    let mut engine = match opened {
        Ok(e) => e,
        Err(e) => {
            log::error!("GPU {ordinal}: {e}");
            gpu.failed.store(true, Ordering::SeqCst);
            return;
        }
    };
    log::info!(target: ui::GOOD, "GPU {ordinal}: matches the official Vela test vector");
    let batch = engine.batch();
    let mut x0 = Vec::new();
    while !shared.stopping() {
        let Some(work) = jobs.current() else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        let job = work.job.clone();
        let result = (|| -> Result<(), String> {
            engine.set_prompt(&job.tokens)?;
            let target = vela_cuda::target_from_difficulty(job.difficulty);
            let mut en2 = extranonce2(job.extranonce2_size);
            let mut h76 = job.header76(&en2)?;
            let mut nonce: u32 = 0;
            while jobs.generation() == work.generation && !shared.stopping() {
                if nonce > u32::MAX - batch as u32 {
                    en2 = extranonce2(job.extranonce2_size);
                    h76 = job.header76(&en2)?;
                    nonce = 0;
                }
                let headers: Vec<[u8; 80]> = (0..batch as u32)
                    .map(|b| {
                        let mut h = [0u8; 80];
                        h[..76].copy_from_slice(&h76);
                        h[76..].copy_from_slice(&(nonce + b).to_le_bytes());
                        h
                    })
                    .collect();
                let seeds: Vec<[u8; 32]> = headers.iter().map(vela_cuda::seed).collect();
                let started = Instant::now();
                engine.run(&seeds, &mut x0)?;
                gpu.record_batch(started, batch as u64);
                for b in 0..batch {
                    let one = &x0[b * vela_cuda::X0_BYTES..(b + 1) * vela_cuda::X0_BYTES];
                    let poi = vela_cuda::hash_poi(&seeds[b], one);
                    if vela_cuda::meets(&vela_cuda::pow_hash(&headers[b], &poi), &target) {
                        log::info!(target: ui::DETAIL, "GPU {ordinal}: share found (job {})", job.id);
                        let _ = shares.send(Share {
                            dev: work.dev,
                            generation: work.generation,
                            job_id: job.id.clone(),
                            extranonce2: en2.clone(),
                            ntime: job.ntime.clone(),
                            nonce: nonce + b as u32,
                            x0: one.to_vec(),
                            hash_poi: poi,
                        });
                    }
                }
                nonce += batch as u32;
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

/// `bench --coin vela`: the model (downloaded if needed), each card checked against the official vector, then batches
/// back to back for `seconds`.
pub fn bench(devices: Option<&str>, seconds: u64, check: bool, model: Option<&Path>) -> Result<(), String> {
    if seconds == 0 {
        return Err("--duration must be at least 1".into());
    }
    let infos = crate::open_devices(devices)?;
    let model = ensure_model(model, None)?;
    for info in &infos {
        let (ordinal, model) = (info.ordinal, model.clone());
        let report = std::thread::spawn(move || -> Result<String, String> {
            let mut engine = vela_cuda::Engine::open(ordinal, &model, vela_cuda::DEFAULT_BATCH)?;
            let mut checked = String::new();
            if check {
                vela_cuda::self_check(&mut engine)?;
                checked = "matches the official Vela test vector; ".into();
            }
            engine.set_prompt(&vela_cuda::VECTOR_TOKENS)?;
            let mut x0 = Vec::new();
            let mut n: u32 = 0;
            let seeds = |n: u32| -> Vec<[u8; 32]> { (0..vela_cuda::DEFAULT_BATCH as u32).map(|b| vela_cuda::sha256(&(n + b).to_le_bytes())).collect() };
            engine.run(&seeds(n), &mut x0)?;
            let (started, mut done) = (Instant::now(), 0u64);
            while started.elapsed() < Duration::from_secs(seconds) {
                n += vela_cuda::DEFAULT_BATCH as u32;
                engine.run(&seeds(n), &mut x0)?;
                done += vela_cuda::DEFAULT_BATCH as u64;
            }
            Ok(format!("{checked}{} ({done} inferences in {seconds} s, batch {})", format_rate(done as f64 / started.elapsed().as_secs_f64()), vela_cuda::DEFAULT_BATCH))
        })
        .join()
        .map_err(|_| format!("the Vela bench of GPU {ordinal} crashed"))??;
        println!("GPU {ordinal} {}: {report}", info.name);
    }
    Ok(())
}

pub struct Options {
    pub config: PoolConfig,
    pub dev_fee: u64,
    pub wallet: String,
    pub worker: Option<String>,
    pub devices: Option<String>,
    pub bind: Option<std::net::SocketAddr>,
    pub model: Option<PathBuf>,
}

pub fn mine(o: Options) -> Result<(), String> {
    if !(o.wallet.starts_with("vela1") || o.wallet.starts_with('V') || o.wallet.starts_with('X')) {
        log::warn!("a Vela address starts with \"vela1\" (or V/X): the pool may refuse this one");
    }
    let infos = crate::open_devices(o.devices.as_deref())?;
    let old: Vec<String> = infos.iter().filter(|i| i.capability < 86).map(|i| format!("{} {} (sm_{})", i.ordinal, i.name, i.capability)).collect();
    if !old.is_empty() {
        return Err(format!("Vela mining needs an RTX 30, 40 or 50 card in this version: {}; choose the cards with -d", old.join(", ")));
    }
    let shared = Arc::new(Shared::new_for("vela", o.config.url(), infos));
    {
        let shared = shared.clone();
        ctrlc::set_handler(move || shared.request_stop()).map_err(|e| format!("could not install the stop handler: {e}"))?;
    }
    // the dev fee is off if there is no DoriByte wallet for the coin
    let dev_fee = if crate::devfee::DEV_WALLET_VELA.is_some() { o.dev_fee } else { 0 };
    if o.dev_fee > 0 && dev_fee == 0 {
        log::info!("Vela: no dev fee in this version");
    }
    if ui::fancy() {
        ui::print_start(&ui::Header {
            version: env!("CARGO_PKG_VERSION"),
            coin: "Vela (VELA)",
            pool: &o.config.url(),
            wallet: &o.wallet,
            worker: o.worker.as_deref(),
            dev_fee,
            api: o.bind.map(|b| format!("http://{b}/summary")),
            gpus: shared.gpus.iter().map(|g| format!("{}  {}", g.info.ordinal, g.info.name)).collect(),
        });
        ui::set_title(&format!("DoriMiner {} · Vela · starting", env!("CARGO_PKG_VERSION")));
    } else {
        log::info!("DoriMiner {} mining Vela, pool {}", env!("CARGO_PKG_VERSION"), o.config.url());
    }
    // the API answers while the model downloads
    let api = o.bind.map(|bind| api::run(bind, shared.clone())).transpose()?;
    shared.fee.percent.store(dev_fee, Ordering::Relaxed);
    let model = match ensure_model(o.model.as_deref(), Some(&shared)) {
        Ok(m) => Arc::new(m),
        Err(e) => {
            shared.request_stop();
            if let Some(t) = api {
                let _ = t.join();
            }
            return Err(e);
        }
    };
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
        threads.push(std::thread::spawn(move || vela_pool::run(config, shared, jobs, user_rx)));
    }
    if let (true, Some(wallet)) = (dev_fee > 0, crate::devfee::DEV_WALLET_VELA) {
        let config = PoolConfig {
            login: crate::devfee::dev_login_with(wallet, &o.config.login),
            password: crate::devfee::DEV_PASSWORD.to_string(),
            dev: true,
            ..o.config.clone()
        };
        let (shared, jobs) = (shared.clone(), jobs.clone());
        threads.push(std::thread::spawn(move || vela_pool::run(config, shared, jobs, dev_rx)));
    } else {
        drop(dev_rx);
    }
    let mut fee = if dev_fee > 0 { crate::devfee::Schedule::new(&shared, Instant::now()) } else { None };
    for slot in 0..shared.gpus.len() {
        let (shared, jobs, model, shares) = (shared.clone(), jobs.clone(), model.clone(), share_tx.clone());
        threads.push(std::thread::spawn(move || card(slot, shared, jobs, model, shares)));
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
                ui::set_title(&format!("DoriMiner {} · Vela · {rate} · {accepted} accepted", env!("CARGO_PKG_VERSION")));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Downloads the real model (1.2 GB) into DORI_DATA_DIR: `cargo test -- --ignored downloads_the_model`.
    #[test]
    #[ignore]
    fn downloads_the_model() {
        assert!(std::env::var_os("DORI_DATA_DIR").is_some(), "set DORI_DATA_DIR to a scratch folder");
        let shared = Shared::new_for("vela", "test".into(), Vec::new());
        let path = ensure_model(None, Some(&shared)).unwrap();
        assert_eq!(path, default_model_path());
        assert_eq!(shared.model.lock().unwrap().as_ref().unwrap().state, "ready");
        assert!(!path.with_extension("poisd.part").exists());
        // a second start only checks it
        let started = Instant::now();
        ensure_model(None, Some(&shared)).unwrap();
        println!("checked again in {} ms", started.elapsed().as_millis());
    }

    #[test]
    fn a_wrong_model_file_is_refused() {
        let dir = std::env::temp_dir().join(format!("dori-vela-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.poisd");
        std::fs::write(&f, b"not a model").unwrap();
        let shared = Shared::new_for("vela", "test".into(), Vec::new());
        let e = ensure_model(Some(&f), Some(&shared)).unwrap_err();
        assert!(e.contains("is not the Vela model"), "{e}");
        assert_eq!(shared.model.lock().unwrap().as_ref().unwrap().state, "error");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
