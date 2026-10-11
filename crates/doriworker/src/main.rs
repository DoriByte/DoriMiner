//! DoriMiner: an NVIDIA miner for Quantus (QTC), with an optional 1% dev fee.

mod api;
mod bench;
mod devfee;
mod pool;
mod state;
mod worker;

use clap::{Args, Parser, Subcommand};
use state::Shared;
use std::process::ExitCode;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Nonces per kernel launch.
const DEFAULT_BATCH: u32 = 2_000_000;
const REPORT_EVERY: Duration = Duration::from_secs(30);
/// With every card down for this long the process ends, for whatever started
/// it to start it again: a card that fails is opened again by its own thread,
/// but a driver that has reset may need a new process.
const ALL_FAILED_LIMIT: Duration = Duration::from_secs(120);

#[derive(Parser)]
#[command(
    name = "dori-miner",
    version,
    about = "DoriMiner: NVIDIA miner for Quantus (QTC). Optional 1% dev fee, --dev-fee 0 turns it off"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    mine: MineArgs,
}

#[derive(Subcommand)]
enum Command {
    /// List the NVIDIA GPUs this miner can use
    Devices,
    /// Measure the hash rate offline, after checking the GPU against the CPU hash
    Bench(BenchArgs),
    /// Log in to the pool given by -o/-u and print the jobs it sends, without mining
    PoolCheck {
        /// Seconds to stay connected
        #[arg(long, default_value_t = 30)]
        seconds: u64,
    },
}

#[derive(Args)]
struct MineArgs {
    /// Pool as [scheme://]host:port; a stratum+ssl:// scheme turns TLS on
    #[arg(short = 'o', long = "pool", global = true)]
    pool: Option<String>,
    /// Wallet address (Quantus addresses start with "qz")
    #[arg(short = 'u', long = "wallet", global = true)]
    wallet: Option<String>,
    /// Worker name, sent to the pool as wallet.worker (wallet/worker on Kryptex)
    #[arg(short = 'w', long = "worker", global = true)]
    worker: Option<String>,
    /// Pool password
    #[arg(short = 'p', long = "password", default_value = "x", global = true)]
    password: String,
    /// GPUs to use, as nvidia-smi indexes separated by commas (default: all)
    #[arg(short = 'd', long = "devices")]
    devices: Option<String>,
    /// Use TLS even without a stratum+ssl:// scheme
    #[arg(long, global = true)]
    tls: bool,
    /// Require this SHA-256 fingerprint (hex) of the pool's TLS certificate
    #[arg(long = "tls-fingerprint", global = true)]
    tls_fingerprint: Option<String>,
    /// Serve GET /summary on PORT or ADDRESS:PORT (a bare port binds to localhost)
    #[arg(long = "api-port")]
    api_port: Option<String>,
    /// Nonces per kernel launch
    #[arg(long, default_value_t = DEFAULT_BATCH)]
    batch: u32,
    #[command(flatten)]
    tune: TuneArgs,
    /// Read the pool's messages more often, so a new job reaches the cards sooner
    #[arg(long = "fast-poll", global = true)]
    fast_poll: bool,
    /// Send shares found for a job the pool has since replaced, instead of
    /// dropping them
    #[arg(long = "submit-late", global = true)]
    submit_late: bool,
    /// Percent of the time mined for the DoriByte wallet, to support the
    /// miner's development (0 to 5; 1 when not given, 0 turns it off).
    /// Just under a minute at a time, on the same pool, and never more than the
    /// percent of a run: at 1% the first minute comes after 99 minutes of mining
    /// for you
    #[arg(long = "dev-fee", value_name = "PERCENT", default_value_t = devfee::DEFAULT_PERCENT, global = true, overrides_with = "dev_fee")]
    dev_fee: u64,
    /// Accepted for compatibility with other miners' command lines; must be "quantus"
    #[arg(long, hide = true)]
    coin: Option<String>,
}

#[derive(Args, Clone, Default)]
struct TuneArgs {
    #[arg(hide = true, long = "launch-stats")]
    launch_stats: bool,
    #[arg(hide = true, long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=8))]
    pipeline: u8,
    #[arg(hide = true, long = "pipeline-streams", default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=2))]
    pipeline_streams: u8,
    #[arg(long = "abort-on-job", hide = true)]
    abort_on_job: bool,
    #[arg(hide = true, long = "nonces-per-thread", value_parser = clap::value_parser!(u32).range(1..=65536))]
    nonces_per_thread: Option<u32>,
    #[arg(hide = true, long = "whole-waves")]
    whole_waves: bool,
    #[arg(hide = true, long = "wait-mode", default_value = "blocking")]
    wait_mode: qtc_cuda::WaitMode,
    /// Measure the card against the CPU while mining: the launches look for
    /// hashes below 2^-BITS of the range (20 gives about two per launch of 2M,
    /// and a launch records eight) and every one the card reports is checked;
    /// shares still go to the pool. The counts are in the log and in /summary
    #[arg(long = "self-check", value_name = "BITS", value_parser = clap::value_parser!(u32).range(8..=40))]
    self_check: Option<u32>,
    /// With --self-check: CPU threads that hash whole launches again, to
    /// find valid nonces the card did not report
    #[arg(long = "self-check-threads", default_value_t = 4, value_parser = clap::value_parser!(u8).range(0..=64))]
    self_check_threads: u8,
}

impl TuneArgs {
    fn worker_options(&self, batch: u32) -> worker::Options {
        worker::Options {
            launch_stats: self.launch_stats,
            pipeline: self.pipeline as usize,
            pipeline_streams: (self.pipeline_streams as usize).min(self.pipeline as usize),
            abort_on_job: self.abort_on_job,
            nonces_per_thread: self.nonces_per_thread,
            whole_waves: self.whole_waves,
            wait_mode: self.wait_mode,
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
    /// Seconds to measure for
    #[arg(long, default_value_t = 30)]
    duration: u64,
    /// GPUs to use, as nvidia-smi indexes separated by commas (default: all)
    #[arg(short = 'd', long = "devices")]
    devices: Option<String>,
    /// Nonces per kernel launch
    #[arg(long, default_value_t = DEFAULT_BATCH)]
    batch: u32,
    /// Skip the GPU-against-CPU check that runs first
    #[arg(long = "no-check")]
    no_check: bool,
    /// Replace the job every this many milliseconds, as a pool would (0 = never)
    #[arg(long = "job-every", default_value_t = 0)]
    job_every: u64,
    #[command(flatten)]
    tune: TuneArgs,
}

/// The selected device ordinals, checked against what the driver reports.
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

fn pool_config(args: &MineArgs) -> Result<pool::PoolConfig, String> {
    if let Some(coin) = &args.coin {
        if !coin.eq_ignore_ascii_case("quantus") {
            return Err(format!("this miner only mines quantus, not '{coin}'"));
        }
    }
    let pool = args
        .pool
        .as_deref()
        .ok_or("a pool is required: -o host:port")?;
    let wallet = args
        .wallet
        .clone()
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
        // A pinned certificate means TLS, whatever the URL says: without this a
        // fingerprint given with a plain address was ignored and the link was
        // neither encrypted nor checked.
        tls: scheme_tls || args.tls || tls_fingerprint.is_some(),
        tls_fingerprint,
        login: match &args.worker {
            // Kryptex names a worker as WALLET/WORKER, the other pools as WALLET.WORKER.
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

/// Connects without any GPU: the jobs are logged as they arrive.
fn pool_check(args: &MineArgs, seconds: u64) -> Result<(), String> {
    let config = pool_config(args)?;
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

/// The second link, for the dev fee: the same pool, the DoriByte wallet.
fn dev_config(user: &pool::PoolConfig) -> pool::PoolConfig {
    pool::PoolConfig {
        login: devfee::dev_login(&user.login),
        // The user's password stays on the user's link.
        password: devfee::DEV_PASSWORD.to_string(),
        dev: true,
        ..user.clone()
    }
}

/// Shares waiting for a pool link, at most.
const SHARE_QUEUE: usize = 1024;
/// The longest log line, in characters.
const MAX_LOG_LINE: usize = 600;

fn mine(args: MineArgs) -> Result<(), String> {
    let config = pool_config(&args)?;
    if args.dev_fee > devfee::MAX_PERCENT {
        return Err(format!("--dev-fee must be 0 to {}", devfee::MAX_PERCENT));
    }
    if args.tune.abort_on_job && !qtc_cuda::ABORT_BUILT {
        return Err("this build has no --abort-on-job".into());
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
    let api = bind
        .map(|bind| api::run(bind, shared.clone()))
        .transpose()?;

    shared.fee.percent.store(args.dev_fee, Ordering::Relaxed);
    log::info!(
        "DoriMiner {} ({}), pool {}",
        env!("CARGO_PKG_VERSION"),
        match args.dev_fee {
            0 => "no dev fee".to_string(),
            percent => format!("dev fee {percent}%, optional: --dev-fee 0 turns it off"),
        },
        config.url()
    );
    let (share_tx, all_shares) = mpsc::channel::<state::Share>();
    // Bounded: a pool that sets a difficulty every hash meets and then stops
    // reading cannot make the shares pile up in memory.
    let (user_tx, share_rx) = mpsc::sync_channel(SHARE_QUEUE);
    let (dev_tx, dev_rx) = mpsc::sync_channel(SHARE_QUEUE);
    let mut threads = Vec::new();
    // Each share goes to the link its job came from.
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
            log::info!(
                "{} | accepted {} rejected {} stale {}",
                qtc_pow::format_hashrate(shared.total_hashrate()),
                shared.accepted.load(Ordering::Relaxed),
                shared.rejected.load(Ordering::Relaxed),
                shared.stale.load(Ordering::Relaxed)
            );
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
    Ok(())
}

/// What a log line may carry of text that came from outside (the pool's
/// messages and job ids): no control characters, so nothing the pool sends can
/// move the cursor, erase lines or forge new ones in the terminal, and no more
/// than `MAX_LOG_LINE` characters.
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

/// Windows looks for a library asked for by name in the current folder and in
/// every folder of PATH; the CUDA driver is looked for under several names that
/// do not exist before its own. From here on only the system folder is searched,
/// so a file with one of those names somewhere else is never loaded.
#[cfg(windows)]
fn only_system_libraries() {
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetDefaultDllDirectories(flags: u32) -> i32;
    }
    // SAFETY: a plain call of the Windows API, with a documented flag.
    if unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) } == 0 {
        log::warn!("could not restrict the library search to the system folder");
    }
}
#[cfg(not(windows))]
fn only_system_libraries() {}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|out, record| {
            use std::io::Write;
            writeln!(
                out,
                "[{} {:<5}] {}",
                out.timestamp(),
                record.level(),
                printable(&record.args().to_string())
            )
        })
        .init();
    only_system_libraries();
    // Device ordinals then match nvidia-smi indexes.
    if std::env::var_os("CUDA_DEVICE_ORDER").is_none() {
        std::env::set_var("CUDA_DEVICE_ORDER", "PCI_BUS_ID");
    }
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Devices) => devices(),
        Some(Command::Bench(args)) => bench::run(args),
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

    /// The command line a manager app builds parses here too.
    #[test]
    fn accepts_the_manager_style_command_line() {
        let cli = Cli::try_parse_from([
            "doriworker",
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
        assert!(!cli.mine.tune.launch_stats);
        assert_eq!(cli.mine.tune.pipeline, 2);
        assert!(!cli.mine.fast_poll && !cli.mine.submit_late);
        // A fingerprint with a plain address still gets TLS; without one a plain address stays plain.
        let pinned = Cli::try_parse_from([
            "doriworker",
            "-o",
            "h:1",
            "-u",
            "w",
            "--tls-fingerprint",
            "6502aeec80ac632364f8d12b21bf370ee6d5a31da911187e16fb8ff2d00cc948",
        ])
        .unwrap();
        assert!(pool_config(&pinned.mine).unwrap().tls);
        let plain = Cli::try_parse_from(["doriworker", "-o", "h:1", "-u", "w"]).unwrap();
        assert!(!pool_config(&plain.mine).unwrap().tls);
        // The fee: 1 when not given; the manager app adds its own --dev-fee at the end of the line,
        // after whatever the profile already says, and the last one counts.
        assert_eq!(plain.mine.dev_fee, devfee::DEFAULT_PERCENT);
        let twice = Cli::try_parse_from(["doriworker", "-o", "h:1", "-u", "w", "--dev-fee", "1", "--api-port", "4068", "--dev-fee", "0"]).unwrap();
        assert_eq!(twice.mine.dev_fee, 0);
        let quick = Cli::try_parse_from([
            "doriworker",
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
        assert_eq!((options.pipeline, options.pipeline_streams), (2, 2));
        assert!(!options.abort_on_job);
        assert_eq!((options.nonces_per_thread, options.whole_waves), (None, false));
        assert_eq!(options.wait_mode, qtc_cuda::WaitMode::Blocking);
        let spin = Cli::try_parse_from(["doriworker", "bench", "--wait-mode", "spin"]).unwrap();
        assert!(matches!(
            spin.command,
            Some(Command::Bench(b)) if b.tune.wait_mode == qtc_cuda::WaitMode::Spin
        ));
        assert!(Cli::try_parse_from(["doriworker", "bench", "--wait-mode", "busy"]).is_err());
        let shaped = Cli::try_parse_from([
            "doriworker",
            "bench",
            "--nonces-per-thread",
            "4",
            "--whole-waves",
        ])
        .unwrap();
        let Some(Command::Bench(shaped)) = shaped.command else {
            panic!("not a bench");
        };
        let options = shaped.tune.worker_options(shaped.batch);
        assert_eq!((options.nonces_per_thread, options.whole_waves), (Some(4), true));
        assert!(Cli::try_parse_from(["doriworker", "bench", "--nonces-per-thread", "0"]).is_err());
        assert!(
            Cli::try_parse_from(["doriworker", "bench", "--abort-on-job"])
                .is_ok_and(|cli| matches!(cli.command, Some(Command::Bench(b)) if b.tune.abort_on_job))
        );
        let queued = Cli::try_parse_from([
            "doriworker",
            "bench",
            "--pipeline",
            "2",
            "--pipeline-streams",
            "2",
            "--batch",
            "4000000",
        ])
        .unwrap();
        let Some(Command::Bench(queued)) = queued.command else {
            panic!("not a bench");
        };
        let options = queued.tune.worker_options(queued.batch);
        assert_eq!((options.pipeline, options.pipeline_streams), (2, 2));
        assert!(Cli::try_parse_from(["doriworker", "bench", "--pipeline", "0"]).is_err());
        assert!(Cli::try_parse_from(["doriworker", "bench", "--pipeline-streams", "3"]).is_err());

        let bench =
            Cli::try_parse_from(["doriworker", "bench", "--duration", "60", "-d", "0"]).unwrap();
        assert!(matches!(
            bench.command,
            Some(Command::Bench(BenchArgs {
                duration: 60,
                job_every: 0,
                ..
            }))
        ));

        let stats = Cli::try_parse_from([
            "doriworker",
            "bench",
            "--launch-stats",
            "--job-every",
            "4000",
        ])
        .unwrap();
        let Some(Command::Bench(bench)) = stats.command else {
            panic!("not a bench");
        };
        assert!(bench.tune.launch_stats);
        assert_eq!(bench.job_every, 4000);
        let mine = Cli::try_parse_from(["doriworker", "-o", "h:1", "-u", "w", "--launch-stats"])
            .unwrap();
        assert!(mine.mine.tune.launch_stats);
    }
}
