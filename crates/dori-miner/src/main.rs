mod api;
mod bench;
mod devfee;
mod pearl;
mod pool;
mod prl_pool;
mod sensors;
mod state;
mod ui;
mod erg;
mod erg_pool;
mod vela;
mod vela_pool;
mod worker;

use clap::{Args, Parser, Subcommand};
use state::Shared;
use std::process::ExitCode;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const DEFAULT_BATCH: u32 = 2_000_000;
const REPORT_EVERY: Duration = Duration::from_secs(30);

const ALL_FAILED_LIMIT: Duration = Duration::from_secs(120);

#[derive(Parser)]
#[command(
    name = "dori-miner",
    version,
    about = "DoriMiner: NVIDIA miner for Quantus (QTC), Pearl (PRL, RTX 50, 40 and 30), Vela (VELA, RTX 50, 40 and 30) and Ergo (ERG, RTX 50, 40 and 30). Optional 1% dev fee, --dev-fee 0 turns it off"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    mine: MineArgs,
}

#[derive(Subcommand)]
enum Command {

    Devices,

    Bench(BenchArgs),

    PoolCheck {

        #[arg(long, default_value_t = 30)]
        seconds: u64,
    },
}

#[derive(Args)]
struct MineArgs {

    #[arg(short = 'o', long = "pool", global = true)]
    pool: Option<String>,

    #[arg(short = 'u', long = "wallet", global = true)]
    wallet: Option<String>,

    #[arg(short = 'w', long = "worker", global = true)]
    worker: Option<String>,

    #[arg(short = 'p', long = "password", default_value = "x", global = true)]
    password: String,

    #[arg(short = 'd', long = "devices")]
    devices: Option<String>,

    #[arg(long, global = true)]
    tls: bool,

    #[arg(long = "tls-fingerprint", global = true)]
    tls_fingerprint: Option<String>,

    #[arg(long = "api-port")]
    api_port: Option<String>,

    #[arg(long, default_value_t = DEFAULT_BATCH)]
    batch: u32,
    #[command(flatten)]
    tune: TuneArgs,

    #[arg(long = "fast-poll", global = true)]
    fast_poll: bool,

    #[arg(long = "submit-late", global = true)]
    submit_late: bool,

    #[arg(long = "dev-fee", value_name = "PERCENT", default_value_t = devfee::DEFAULT_PERCENT, global = true, overrides_with = "dev_fee")]
    dev_fee: u64,

    #[arg(long, value_name = "COIN", global = true)]
    coin: Option<String>,

    #[arg(long, global = true)]
    lithos: bool,

    #[arg(long = "vela-model", value_name = "FILE", global = true)]
    vela_model: Option<std::path::PathBuf>,
}

#[derive(Args, Clone, Default)]
struct TuneArgs {

    #[arg(long = "self-check", value_name = "BITS", value_parser = clap::value_parser!(u32).range(8..=40))]
    self_check: Option<u32>,

    #[arg(long = "self-check-threads", default_value_t = 4, value_parser = clap::value_parser!(u8).range(0..=64))]
    self_check_threads: u8,
}

impl TuneArgs {
    fn worker_options(&self, batch: u32) -> worker::Options {
        let (depth, lanes) = qtc_cuda::defaults();
        worker::Options {
            depth,
            lanes,
            self_check: self.self_check.map(|bits| worker::SelfCheck {
                bits,
                cpu_threads: self.self_check_threads as usize,
            }),
            ..worker::Options::new(batch)
        }
    }
}

#[derive(Args)]
struct BenchArgs {

    #[arg(long, default_value_t = 30)]
    duration: u64,

    #[arg(short = 'd', long = "devices")]
    devices: Option<String>,

    #[arg(long, default_value_t = DEFAULT_BATCH)]
    batch: u32,

    #[arg(long = "no-check")]
    no_check: bool,

    #[arg(long = "job-every", default_value_t = 0)]
    job_every: u64,

    #[arg(long)]
    coin: Option<String>,
    #[command(flatten)]
    tune: TuneArgs,
}

fn select_devices(spec: Option<&str>, count: usize) -> Result<Vec<usize>, String> {
    let Some(spec) = spec else {
        return Ok((0..count).collect());
    };
    let mut selected = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let index: usize = part
            .parse()
            .map_err(|_| format!("invalid device index '{part}'"))?;
        if index >= count {
            return Err(format!("device {index} does not exist ({count} found)"));
        }
        if !selected.contains(&index) {
            selected.push(index);
        }
    }
    if selected.is_empty() {
        return Err("no device selected".into());
    }
    Ok(selected)
}

fn open_devices(spec: Option<&str>) -> Result<Vec<qtc_cuda::DeviceInfo>, String> {
    let count = qtc_cuda::device_count().map_err(|e| e.to_string())?;
    if count == 0 {
        return Err("no NVIDIA GPU found".into());
    }
    select_devices(spec, count)?
        .into_iter()
        .map(|ordinal| qtc_cuda::device_info(ordinal).map_err(|e| e.to_string()))
        .collect()
}

fn devices() -> Result<(), String> {
    for info in open_devices(None)? {
        let kernel = match qtc_cuda::kernel_arch_for(info.capability) {
            Some(arch) => format!("kernel sm_{arch}"),
            None => "not supported".into(),
        };
        println!(
            "{}: {} (sm_{}, {kernel}, PCI {})",
            info.ordinal,
            info.name,
            info.capability,
            info.pci_bus_id.as_deref().unwrap_or("unknown")
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Coin {
    Quantus,
    Pearl,
    Vela,
    Ergo,
}

fn is_local_host(host: &str) -> bool {
    let h = host.trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase();
    if h == "localhost" {
        return true;
    }
    match h.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(a)) => a.is_loopback() || a.is_private() || a.is_link_local(),
        Ok(std::net::IpAddr::V6(a)) => a.is_loopback() || (a.segments()[0] & 0xfe00) == 0xfc00 || (a.segments()[0] & 0xffc0) == 0xfe80,
        Err(_) => false,
    }
}

const LITHOS_POOL: &str = "127.0.0.1:4444";

fn coin_of(args: &MineArgs) -> Result<Coin, String> {
    if args.lithos {
        return match args.coin.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None | Some("ergo") | Some("erg") | Some("autolykos2") => Ok(Coin::Ergo),
            Some(other) => Err(format!("--lithos mines ergo, not '{other}'")),
        };
    }
    match args.coin.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("quantus") | Some("qtc") => Ok(Coin::Quantus),
        Some("pearl") | Some("prl") | Some("pearlhash") => Ok(Coin::Pearl),
        Some("vela") => Ok(Coin::Vela),
        Some("ergo") | Some("erg") | Some("autolykos2") => Ok(Coin::Ergo),
        Some(other) => Err(format!("this miner mines quantus, pearl, vela and ergo, not '{other}'")),
    }
}

fn pool_config(args: &MineArgs) -> Result<pool::PoolConfig, String> {
    coin_of(args)?;

    let pool = args
        .pool
        .as_deref()
        .or(args.lithos.then_some(LITHOS_POOL))
        .ok_or("a pool is required: -o host:port")?;
    let wallet = args
        .wallet
        .clone()
        .or(args.lithos.then(|| "lithos".to_string()))
        .ok_or("a wallet address is required: -u ADDRESS")?;
    let (host, port, scheme_tls) = pool::parse_pool_url(pool)?;
    let tls_fingerprint = match &args.tls_fingerprint {
        None => None,
        Some(text) => Some(
            hex::decode(text.replace(':', ""))
                .ok()
                .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                .ok_or("--tls-fingerprint must be 64 hex characters")?,
        ),
    };
    let kryptex = host.to_ascii_lowercase().ends_with(".kryptex.network");
    Ok(pool::PoolConfig {
        host,
        port,

        tls: scheme_tls || args.tls || tls_fingerprint.is_some(),
        tls_fingerprint,
        login: match &args.worker {

            Some(worker) if !worker.is_empty() && !wallet.contains('/') => {
                format!("{wallet}{}{worker}", if kryptex { '/' } else { '.' })
            }
            _ => wallet,
        },
        password: args.password.clone(),
        agent: format!("DoriMiner/{}", env!("CARGO_PKG_VERSION")),
        fast_poll: args.fast_poll,
        submit_late: args.submit_late,
        dev: false,
    })
}

fn pool_check(args: &MineArgs, seconds: u64) -> Result<(), String> {
    let config = pool_config(args)?;
    if coin_of(args)? == Coin::Ergo {
        return erg::pool_check(config, seconds);
    }
    let shared = Arc::new(Shared::new(config.url(), Vec::new()));
    let (_share_tx, share_rx) = mpsc::channel();
    let client = {
        let shared = shared.clone();
        std::thread::spawn(move || pool::run(config, shared, share_rx))
    };
    let started = Instant::now();
    let mut jobs = 0u64;
    let mut seen = 0u64;
    while started.elapsed() < Duration::from_secs(seconds) {
        std::thread::sleep(Duration::from_millis(50));
        let generation = shared.generation();
        if generation != seen {
            seen = generation;
            jobs += shared.work().is_some() as u64;
        }
    }
    let connected = shared.pool_connected.load(Ordering::SeqCst);
    shared.request_stop();
    let _ = client.join();
    println!("{jobs} jobs in {seconds} s, connected at the end: {connected}");
    if jobs == 0 {
        return Err("the pool sent no job".into());
    }
    Ok(())
}

fn dev_config(user: &pool::PoolConfig) -> pool::PoolConfig {
    pool::PoolConfig {
        login: devfee::dev_login(&user.login),

        password: devfee::DEV_PASSWORD.to_string(),
        dev: true,
        ..user.clone()
    }
}

const SHARE_QUEUE: usize = 1024;

const MAX_LOG_LINE: usize = 600;

fn mine(args: MineArgs) -> Result<(), String> {
    let config = pool_config(&args)?;
    if coin_of(&args)? == Coin::Ergo {
        if args.dev_fee > devfee::MAX_PERCENT {
            return Err(format!("--dev-fee must be 0 to {}", devfee::MAX_PERCENT));
        }
        let local = is_local_host(&config.host);
        return erg::mine(erg::Options {
            config,

            dev_fee: if args.lithos || local { 0 } else { args.dev_fee },
            lithos: args.lithos,
            wallet: args.wallet.clone().unwrap_or_default(),
            worker: args.worker.clone(),
            devices: args.devices.clone(),
            bind: args.api_port.as_deref().map(api::parse_bind).transpose()?,
        });
    }
    if coin_of(&args)? == Coin::Vela {
        if args.dev_fee > devfee::MAX_PERCENT {
            return Err(format!("--dev-fee must be 0 to {}", devfee::MAX_PERCENT));
        }
        return vela::mine(vela::Options {
            config,
            dev_fee: args.dev_fee,
            wallet: args.wallet.clone().unwrap_or_default(),
            worker: args.worker.clone(),
            devices: args.devices.clone(),
            bind: args.api_port.as_deref().map(api::parse_bind).transpose()?,
            model: args.vela_model.clone(),
        });
    }
    if coin_of(&args)? == Coin::Pearl {
        if args.dev_fee > devfee::MAX_PERCENT {
            return Err(format!("--dev-fee must be 0 to {}", devfee::MAX_PERCENT));
        }
        return pearl::mine(pearl::Options {
            config,
            dev_fee: args.dev_fee,
            wallet: args.wallet.clone().unwrap_or_default(),
            worker: args.worker.clone(),
            devices: args.devices.clone(),
            bind: args.api_port.as_deref().map(api::parse_bind).transpose()?,
        });
    }
    if args.dev_fee > devfee::MAX_PERCENT {
        return Err(format!("--dev-fee must be 0 to {}", devfee::MAX_PERCENT));
    }
    if args.batch == 0 || args.batch > qtc_cuda::MAX_LAUNCH_SIZE {
        return Err(format!(
            "--batch must be 1 to {}",
            qtc_cuda::MAX_LAUNCH_SIZE
        ));
    }
    let bind = args.api_port.as_deref().map(api::parse_bind).transpose()?;

    let infos = open_devices(args.devices.as_deref())?;
    let shared = Arc::new(Shared::new(config.url(), infos));
    {
        let shared = shared.clone();
        ctrlc::set_handler(move || shared.request_stop())
            .map_err(|e| format!("could not install the stop handler: {e}"))?;
    }

    shared.fee.percent.store(args.dev_fee, Ordering::Relaxed);
    if ui::fancy() {
        ui::print_start(&ui::Header {
            version: env!("CARGO_PKG_VERSION"),
            coin: "Quantus (QTC)",
            pool: &config.url(),
            wallet: args.wallet.as_deref().unwrap_or(""),
            worker: args.worker.as_deref(),
            dev_fee: args.dev_fee,
            api: bind.map(|b| format!("http://{b}/summary")),
            gpus: shared
                .gpus
                .iter()
                .map(|g| match qtc_cuda::kernel_arch_for(g.info.capability) {
                    Some(arch) => format!("{}  {}  (kernel sm_{arch})", g.info.ordinal, g.info.name),
                    None => format!("{}  {}", g.info.ordinal, g.info.name),
                })
                .collect(),
        });
        ui::set_title(&format!("DoriMiner {} · starting", env!("CARGO_PKG_VERSION")));
    } else {
        log::info!(
            "DoriMiner {} ({}), pool {}",
            env!("CARGO_PKG_VERSION"),
            match args.dev_fee {
                0 => "no dev fee".to_string(),
                percent => format!("dev fee {percent}%, optional: --dev-fee 0 turns it off"),
            },
            config.url()
        );
    }

    let api = bind
        .map(|bind| api::run(bind, shared.clone()))
        .transpose()?;
    let (share_tx, all_shares) = mpsc::channel::<state::Share>();

    let (user_tx, share_rx) = mpsc::sync_channel(SHARE_QUEUE);
    let (dev_tx, dev_rx) = mpsc::sync_channel(SHARE_QUEUE);
    let mut threads = Vec::new();

    threads.push(std::thread::spawn(move || {
        let mut warned_full = false;
        for share in all_shares {
            let link = if share.dev { &dev_tx } else { &user_tx };
            match link.try_send(share) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => {
                    if !warned_full {
                        log::warn!("the pool is not taking shares as fast as they are found: some are dropped");
                        warned_full = true;
                    }
                }
                Err(mpsc::TrySendError::Disconnected(_)) => break,
            }
        }
    }));
    if args.dev_fee > 0 {
        let (shared, config) = (shared.clone(), dev_config(&config));
        threads.push(std::thread::spawn(move || {
            pool::run(config, shared, dev_rx)
        }));
    }
    let mut fee = devfee::Schedule::new(&shared, Instant::now());
    let options = worker::Options {
        startup_check: true,
        ..args.tune.worker_options(args.batch)
    };
    let self_check = options.self_check.is_some();
    for slot in 0..shared.gpus.len() {
        let (shared, share_tx) = (shared.clone(), share_tx.clone());
        let options = options.clone();
        threads.push(std::thread::spawn(move || {
            worker::run(slot, options, shared, share_tx)
        }));
    }
    drop(share_tx);
    {
        let shared = shared.clone();
        threads.push(std::thread::spawn(move || {
            pool::run(config, shared, share_rx)
        }));
    }

    let nvml = if ui::fancy() { sensors::Nvml::open() } else { None };
    let mut last_report = Instant::now();
    let mut all_failed_since: Option<Instant> = None;
    let mut gave_up = false;
    while !shared.stopping() {
        std::thread::sleep(Duration::from_millis(200));
        if let Some(fee) = &mut fee {
            fee.tick(&shared, Instant::now());
        }
        if shared
            .gpus
            .iter()
            .all(|gpu| gpu.failed.load(Ordering::SeqCst))
        {
            let since = *all_failed_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= ALL_FAILED_LIMIT {
                log::error!(
                    "every GPU has been down for {} s",
                    ALL_FAILED_LIMIT.as_secs()
                );
                gave_up = true;
                shared.request_stop();
            }
        } else {
            all_failed_since = None;
        }
        if last_report.elapsed() >= REPORT_EVERY {
            last_report = Instant::now();
            let (accepted, rejected, stale) = (
                shared.accepted.load(Ordering::Relaxed),
                shared.rejected.load(Ordering::Relaxed),
                shared.stale.load(Ordering::Relaxed),
            );
            let hashrate = qtc_pow::format_hashrate(shared.total_hashrate());
            if ui::fancy() {
                let gpus: Vec<ui::GpuLine> = shared
                    .gpus
                    .iter()
                    .map(|g| ui::GpuLine {
                        ordinal: g.info.ordinal,
                        name: g.info.name.trim_start_matches("NVIDIA ").trim_start_matches("GeForce ").to_string(),
                        hashrate: qtc_pow::format_hashrate(g.hashrate()),
                        failed: g.failed.load(Ordering::SeqCst),
                        sensors: nvml.as_ref().map(|n| n.read(g.info.pci_bus_id.as_deref(), g.info.ordinal)).unwrap_or_default(),
                    })
                    .collect();
                ui::print_report(
                    &ui::Totals { hashrate: hashrate.clone(), accepted, rejected, stale, uptime_s: shared.started.elapsed().as_secs() },
                    &gpus,
                );
                ui::set_title(&format!("DoriMiner {} · {hashrate} · {accepted} accepted", env!("CARGO_PKG_VERSION")));
            } else {
                log::info!("{hashrate} | accepted {accepted} rejected {rejected} stale {stale}");
            }
            for gpu in shared.gpus.iter().filter(|_| self_check) {
                let read = |counter: &std::sync::atomic::AtomicU64| counter.load(Ordering::Relaxed);
                log::info!(
                    "GPU {} self-check: {} hashes, {} hits confirmed, {} not confirmed, {} not recorded; CPU hashed {} again: {} valid, {} the card missed",
                    gpu.info.ordinal,
                    read(&gpu.hashes),
                    read(&gpu.check.hits),
                    read(&gpu.false_candidates),
                    read(&gpu.dropped_candidates),
                    read(&gpu.check.cpu_hashes),
                    read(&gpu.check.cpu_hits),
                    read(&gpu.check.cpu_missed)
                );
            }
        }
    }
    log::info!("stopping");
    for thread in threads.into_iter().chain(api) {
        let _ = thread.join();
    }
    if gave_up {
        return Err("mining stopped because every GPU failed".into());
    }
    if shared.needs_new_process.load(Ordering::SeqCst) {
        return Err("mining stopped: a GPU needs a new process to work again (a manager starts it at once)".into());
    }
    Ok(())
}

fn printable(message: &str) -> String {
    let mut out: String = message
        .chars()
        .take(MAX_LOG_LINE)
        .map(|c| if c.is_control() { '?' } else { c })
        .collect();
    if message.chars().nth(MAX_LOG_LINE).is_some() {
        out.push_str("...");
    }
    out
}

#[cfg(windows)]
fn only_system_libraries() {
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetDefaultDllDirectories(flags: u32) -> i32;
    }

    if unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) } == 0 {
        log::warn!("could not restrict the library search to the system folder");
    }
}
#[cfg(not(windows))]
fn only_system_libraries() {}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|out, record| {
            let timestamp = out.timestamp();
            ui::log_line(
                out,
                timestamp,
                record.level(),
                record.target(),
                &printable(&record.args().to_string()),
            )
        })
        .init();
    only_system_libraries();

    if std::env::var_os("CUDA_DEVICE_ORDER").is_none() {
        std::env::set_var("CUDA_DEVICE_ORDER", "PCI_BUS_ID");
    }
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Devices) => devices(),
        Some(Command::Bench(args)) => match args.coin.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None | Some("quantus") | Some("qtc") => bench::run(args),
            Some("pearl") | Some("prl") | Some("pearlhash") => pearl::bench(args.devices.as_deref(), args.duration, !args.no_check),
            Some("vela") => vela::bench(args.devices.as_deref(), args.duration, !args.no_check, cli.mine.vela_model.as_deref()),
            Some("ergo") | Some("erg") | Some("autolykos2") => erg::bench(args.devices.as_deref(), args.duration, !args.no_check),
            Some(other) => Err(format!("this miner mines quantus, pearl, vela and ergo, not '{other}'")),
        },
        Some(Command::PoolCheck { seconds }) => pool_check(&cli.mine, seconds),
        None => mine(cli.mine),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_selection() {
        assert_eq!(select_devices(None, 3), Ok(vec![0, 1, 2]));
        assert_eq!(select_devices(Some("2,0"), 3), Ok(vec![2, 0]));
        assert_eq!(select_devices(Some(" 1 , 1 "), 3), Ok(vec![1]));
        assert!(select_devices(Some("3"), 3).is_err());
        assert!(select_devices(Some("a"), 3).is_err());
        assert!(select_devices(Some(","), 3).is_err());
    }

    #[test]
    fn accepts_the_manager_style_command_line() {
        let cli = Cli::try_parse_from([
            "dori-miner",
            "--coin",
            "quantus",
            "-o",
            "stratum+ssl://qtc.kryptex.network:8049",
            "-u",
            "qzwallet",
            "-w",
            "rig",
            "-p",
            "x",
            "-d",
            "0,1",
            "--api-port",
            "127.0.0.1:4068",
        ])
        .unwrap();
        assert!(cli.command.is_none());
        assert_eq!(
            cli.mine.pool.as_deref(),
            Some("stratum+ssl://qtc.kryptex.network:8049")
        );
        assert_eq!(cli.mine.worker.as_deref(), Some("rig"));
        assert_eq!(cli.mine.devices.as_deref(), Some("0,1"));
        assert_eq!(cli.mine.batch, DEFAULT_BATCH);
        assert!(!cli.mine.fast_poll && !cli.mine.submit_late);

        let pinned = Cli::try_parse_from([
            "dori-miner",
            "-o",
            "h:1",
            "-u",
            "w",
            "--tls-fingerprint",
            "6502aeec80ac632364f8d12b21bf370ee6d5a31da911187e16fb8ff2d00cc948",
        ])
        .unwrap();
        assert!(pool_config(&pinned.mine).unwrap().tls);
        let plain = Cli::try_parse_from(["dori-miner", "-o", "h:1", "-u", "w"]).unwrap();
        assert!(!pool_config(&plain.mine).unwrap().tls);

        assert_eq!(plain.mine.dev_fee, devfee::DEFAULT_PERCENT);
        let twice = Cli::try_parse_from(["dori-miner", "-o", "h:1", "-u", "w", "--dev-fee", "1", "--api-port", "4068", "--dev-fee", "0"]).unwrap();
        assert_eq!(twice.mine.dev_fee, 0);
        let quick = Cli::try_parse_from([
            "dori-miner",
            "-o",
            "h:1",
            "-u",
            "w",
            "--fast-poll",
            "--submit-late",
        ])
        .unwrap();
        let config = pool_config(&quick.mine).unwrap();
        assert!(config.fast_poll && config.submit_late);
        let options = cli.mine.tune.worker_options(cli.mine.batch);
        assert_eq!((options.depth, options.lanes), qtc_cuda::defaults());

        let bench =
            Cli::try_parse_from(["dori-miner", "bench", "--duration", "60", "-d", "0"]).unwrap();
        assert!(matches!(
            bench.command,
            Some(Command::Bench(BenchArgs {
                duration: 60,
                job_every: 0,
                ..
            }))
        ));

    }

    #[test]
    fn lithos() {

        let bare = Cli::try_parse_from(["dori-miner", "--lithos"]).unwrap();
        assert!(coin_of(&bare.mine).unwrap() == Coin::Ergo);
        let c = pool_config(&bare.mine).unwrap();
        assert_eq!((c.host.as_str(), c.port, c.tls, c.login.as_str()), ("127.0.0.1", 4444, false, "lithos"));

        let rig = Cli::try_parse_from(["dori-miner", "--lithos", "-o", "192.168.1.20:4444", "-w", "rig2"]).unwrap();
        let c = pool_config(&rig.mine).unwrap();
        assert_eq!((c.host.as_str(), c.login.as_str()), ("192.168.1.20", "lithos.rig2"));
        assert!(coin_of(&Cli::try_parse_from(["dori-miner", "--lithos", "--coin", "erg"]).unwrap().mine).is_ok());
        assert!(coin_of(&Cli::try_parse_from(["dori-miner", "--lithos", "--coin", "pearl"]).unwrap().mine).is_err());

        for h in ["127.0.0.1", "localhost", "192.168.1.20", "10.1.2.3", "172.16.0.9", "[::1]"] {
            assert!(is_local_host(h), "{h}");
        }
        for h in ["erg-eu.kryptex.network", "8.8.8.8", "172.32.0.1", "ergo.herominers.com"] {
            assert!(!is_local_host(h), "{h}");
        }
    }
}
