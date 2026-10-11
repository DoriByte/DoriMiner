//! Published without its comments: they describe the kernel, which is closed for now (see the README).
//! The code is all here.


use crate::state::Shared;
use crate::{open_devices, worker, BenchArgs};
use primitive_types::U512;
use qtc_cuda::{CudaSearcher, Job, Walk};
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const CHECK_HASHES: u32 = 4096;
const CHECK_SEARCH: u32 = 400_000;
const CHECK_LAUNCH: u32 = 7;

fn check_per_thread(walk: Walk) -> u32 {
    match walk {
        Walk::TwoDirection => walk.max_nonces_per_thread(),
        Walk::Sparse => walk.default_nonces_per_thread(CHECK_SEARCH),
    }
}

fn check_device(ordinal: usize) -> Result<String, String> {
    let mut searcher = CudaSearcher::open(ordinal).map_err(|e| e.to_string())?;
    let header = [0x5au8; 32];
    let mut start = [0x17u8; 64];
    start[56..].copy_from_slice(&(u32::MAX as u64 - 100).to_be_bytes());
    let hashes = searcher
        .hash_nonces(header, start, CHECK_HASHES)
        .map_err(|e| e.to_string())?;
    let first = U512::from_big_endian(&start);
    for (i, hash) in hashes.iter().enumerate() {
        let nonce = (first + U512::from(i)).to_big_endian();
        if *hash != qtc_pow::get_nonce_hash(header, nonce) {
            return Err(format!("GPU {ordinal} hashed nonce {i} of the check wrong"));
        }
    }

    let job = Job {
        header,
        target: U512::MAX >> 17,
        nonce_high: [0x29u8; 32],
    };
    let walk = searcher.walk();
    if walk == Walk::TwoDirection {
        searcher.set_nonces_per_thread(Some(check_per_thread(walk)));
    }
    let per_thread = searcher.nonces_per_thread(CHECK_SEARCH);
    let outcome = searcher
        .search(&job.prepare(), CHECK_LAUNCH, CHECK_SEARCH)
        .map_err(|e| e.to_string())?;
    let mut gpu: Vec<[u8; 64]> = outcome.hits.iter().map(|hit| hit.nonce_be).collect();
    gpu.sort();
    let mut cpu: Vec<[u8; 64]> = (0..CHECK_SEARCH)
        .filter_map(|index| job.verify_at(walk, CHECK_LAUNCH, per_thread, index))
        .map(|hit| hit.nonce_be)
        .collect();
    cpu.sort();
    if gpu != cpu || outcome.dropped_candidates > 0 {
        return Err(format!(
            "GPU {ordinal} found {} of the {} valid nonces in the check search",
            gpu.len(),
            cpu.len()
        ));
    }
    Ok(format!(
        "{CHECK_HASHES} hashes and {} search hits match the CPU",
        cpu.len()
    ))
}

pub fn run(args: BenchArgs) -> Result<(), String> {
    if args.batch == 0 || args.batch > qtc_cuda::MAX_LAUNCH_SIZE || args.duration == 0 {
        return Err(format!(
            "--batch must be 1 to {} and --duration at least 1",
            qtc_cuda::MAX_LAUNCH_SIZE
        ));
    }
    let infos = open_devices(args.devices.as_deref())?;
    if !args.no_check {
        for info in &infos {
            let ordinal = info.ordinal;
            let report = std::thread::spawn(move || check_device(ordinal))
                .join()
                .map_err(|_| format!("the check of GPU {ordinal} crashed"))??;
            println!("GPU {ordinal} {}: {report}", info.name);
        }
    }

    let shared = Arc::new(Shared::new("benchmark".into(), infos));
    {
        let shared = shared.clone();
        ctrlc::set_handler(move || shared.request_stop())
            .map_err(|e| format!("could not install the stop handler: {e}"))?;
    }
    shared.set_work(
        "bench".into(),
        [0x5a; 32],
        U512::MAX,
        U512::one(),
        Vec::new(),
    );
    let (share_tx, _share_rx) = mpsc::channel();
    let options = args.tune.worker_options(args.batch);
    let mut threads: Vec<_> = (0..shared.gpus.len())
        .map(|slot| {
            let (shared, share_tx) = (shared.clone(), share_tx.clone());
            let options = options.clone();
            std::thread::spawn(move || worker::run(slot, options, shared, share_tx))
        })
        .collect();
    if args.job_every > 0 {
        let shared = shared.clone();
        let every = Duration::from_millis(args.job_every);
        threads.push(std::thread::spawn(move || {
            let mut next = Instant::now() + every;
            let mut n = 0u64;
            while !shared.stopping() {
                let now = Instant::now();
                if now < next {
                    std::thread::sleep((next - now).min(Duration::from_millis(50)));
                    continue;
                }
                next += every;
                n += 1;
                let mut header = [0x5au8; 32];
                header[..8].copy_from_slice(&n.to_le_bytes());
                shared.set_work(format!("bench-{n}"), header, U512::MAX, U512::one(), Vec::new());
            }
        }));
    }

    let started = Instant::now();
    let mut next_line = Duration::from_secs(5);
    while !shared.stopping() && started.elapsed() < Duration::from_secs(args.duration) {
        std::thread::sleep(Duration::from_millis(100));
        if shared
            .gpus
            .iter()
            .all(|gpu| gpu.failed.load(Ordering::SeqCst))
        {
            break;
        }
        if started.elapsed() >= next_line {
            next_line += Duration::from_secs(5);
            println!(
                "{:>4} s  {}",
                started.elapsed().as_secs(),
                qtc_pow::format_hashrate(shared.total_hashrate())
            );
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    shared.request_stop();
    for thread in threads {
        let _ = thread.join();
    }

    let mut total = 0.0;
    for gpu in &shared.gpus {
        if gpu.failed.load(Ordering::SeqCst) {
            println!("GPU {} {}: failed", gpu.info.ordinal, gpu.info.name);
            continue;
        }
        let rate = gpu.hashes.load(Ordering::Relaxed) as f64 / elapsed;
        total += rate;
        println!(
            "GPU {} {}: {} average over {elapsed:.0} s",
            gpu.info.ordinal,
            gpu.info.name,
            qtc_pow::format_hashrate(rate)
        );
        if args.job_every > 0 {
            let hashes = gpu.hashes.load(Ordering::Relaxed);
            let stale = gpu.stale_hashes.load(Ordering::Relaxed);
            println!(
                "GPU {}: {:.3}% of the hashes were on a job already replaced ({} launches)",
                gpu.info.ordinal,
                stale as f64 * 100.0 / hashes.max(1) as f64,
                gpu.launches.load(Ordering::Relaxed)
            );
        }
    }
    println!("Total: {}", qtc_pow::format_hashrate(total));
    if shared
        .gpus
        .iter()
        .any(|gpu| gpu.failed.load(Ordering::SeqCst))
    {
        return Err("at least one GPU failed during the benchmark".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_search_has_a_usable_number_of_hits() {
        let job = Job {
            header: [0x5au8; 32],
            target: U512::MAX >> 17,
            nonce_high: [0x29u8; 32],
        };
        for walk in [Walk::Sparse, Walk::TwoDirection] {
            let hits = (0..CHECK_SEARCH)
                .filter(|&index| {
                    job.verify_at(walk, CHECK_LAUNCH, check_per_thread(walk), index)
                        .is_some()
                })
                .count();
            assert!((1..=8).contains(&hits), "{walk:?}: {hits} hits");
        }
    }
}
