//! Local read-only HTTP API. `GET /summary` returns JSON a manager app can
//! read: hash rates in H/s, cumulative share counts, and one entry per GPU
//! keyed by PCI address.

use crate::state::Shared;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// Accepts `PORT` or `ADDRESS:PORT`. A bare port binds to localhost.
pub fn parse_bind(text: &str) -> Result<SocketAddr, String> {
    if let Ok(port) = text.parse::<u16>() {
        return Ok(SocketAddr::from(([127, 0, 0, 1], port)));
    }
    text.parse()
        .map_err(|_| format!("invalid API address '{text}' (use PORT or ADDRESS:PORT)"))
}

pub fn summary(shared: &Shared) -> Value {
    let difficulty = shared.work().map(|work| work.difficulty.to_string());
    let gpus: Vec<Value> = shared
        .gpus
        .iter()
        .map(|gpu| {
            json!({
                "id": gpu.info.ordinal,
                "name": gpu.info.name,
                "pci_bus_id": gpu.info.pci_bus_id,
                "hashrate": gpu.hashrate(),
                "hashes": gpu.hashes.load(Ordering::Relaxed),
                "launches": gpu.launches.load(Ordering::Relaxed),
                "stale_hashes": gpu.stale_hashes.load(Ordering::Relaxed),
                "false_candidates": gpu.false_candidates.load(Ordering::Relaxed),
                "dropped_candidates": gpu.dropped_candidates.load(Ordering::Relaxed),
                "check_hits": gpu.check.hits.load(Ordering::Relaxed),
                "check_cpu_hashes": gpu.check.cpu_hashes.load(Ordering::Relaxed),
                "check_cpu_hits": gpu.check.cpu_hits.load(Ordering::Relaxed),
                "check_cpu_missed": gpu.check.cpu_missed.load(Ordering::Relaxed),
                "failed": gpu.failed.load(Ordering::Relaxed),
            })
        })
        .collect();
    json!({
        "miner": "dori-miner",
        "version": env!("CARGO_PKG_VERSION"),
        "algo": "quantus",
        "uptime": shared.started.elapsed().as_secs(),
        "pool": {
            "url": shared.pool_url,
            "connected": shared.pool_connected.load(Ordering::Relaxed),
            "difficulty": difficulty,
        },
        "dev_fee_percent": shared.fee.percent.load(Ordering::Relaxed),
        "dev_fee": {
            "active": shared.fee.active.load(Ordering::Relaxed),
            "seconds": shared.fee.seconds.load(Ordering::Relaxed),
            "accepted_shares": shared.fee.accepted.load(Ordering::Relaxed),
            "invalid_shares": shared.fee.rejected.load(Ordering::Relaxed),
            "stale_shares": shared.fee.stale.load(Ordering::Relaxed),
        },
        "hashrate": shared.total_hashrate(),
        "accepted_shares": shared.accepted.load(Ordering::Relaxed),
        "invalid_shares": shared.rejected.load(Ordering::Relaxed),
        "stale_shares": shared.stale.load(Ordering::Relaxed),
        "late_shares_sent": shared.late_sent.load(Ordering::Relaxed),
        "late_shares_accepted": shared.late_accepted.load(Ordering::Relaxed),
        "gpus": gpus,
    })
}

/// True for `localhost` and for any IP address, with or without a port.
fn host_is_local_or_address(host: &str) -> bool {
    let is_address = |h: &str| h.trim_matches(|c| c == '[' || c == ']').parse::<std::net::IpAddr>().is_ok();
    let name = match host.rsplit_once(':') {
        _ if host.parse::<SocketAddr>().is_ok() || is_address(host) => return true,
        Some((name, port)) if port.parse::<u16>().is_ok() => name,
        _ => host,
    };
    name.eq_ignore_ascii_case("localhost")
}

/// Serves until a stop is requested.
pub fn run(bind: SocketAddr, shared: Arc<Shared>) -> Result<std::thread::JoinHandle<()>, String> {
    let server = tiny_http::Server::http(bind)
        .map_err(|e| format!("could not open the API on {bind}: {e}"))?;
    let bind = server.server_addr().to_ip().unwrap_or(bind);
    log::info!("API on http://{bind}/summary");
    if !bind.ip().is_loopback() {
        log::warn!(
            "the API answers on {}, to every machine that reaches it, without a password: use a bare port to keep it on this PC",
            bind.ip()
        );
    }
    let local_only = bind.ip().is_loopback();
    Ok(std::thread::spawn(move || {
        let json_type =
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
        while !shared.stopping() {
            let request = match server.recv_timeout(Duration::from_millis(200)) {
                Ok(Some(request)) => request,
                Ok(None) => continue,
                Err(_) => break,
            };
            let path = request.url().split('?').next().unwrap_or("");
            // A web page open in the user's browser can make a name of its own
            // point at this PC and read the answer; it cannot make the browser
            // send a Host that is an address. (An API opened to the network on
            // purpose is reached by whatever name the machine has there.)
            let host = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("Host"))
                .map(|h| h.value.as_str().to_string());
            let response = if local_only && !host.as_deref().is_none_or(host_is_local_or_address) {
                tiny_http::Response::from_string("forbidden").with_status_code(403)
            } else if *request.method() == tiny_http::Method::Get && path == "/summary" {
                tiny_http::Response::from_string(summary(&shared).to_string())
                    .with_header(json_type.clone())
            } else {
                tiny_http::Response::from_string("not found").with_status_code(404)
            };
            let _ = request.respond(response);
        }
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_local_names_and_addresses_are_answered() {
        for ok in ["127.0.0.1:4068", "localhost:4068", "LOCALHOST", "[::1]:4068", "192.168.1.20:4068", "::1"] {
            assert!(super::host_is_local_or_address(ok), "{ok}");
        }
        for bad in ["evil.example:4068", "evil.example", "127.0.0.1.evil.example:80", ""] {
            assert!(!super::host_is_local_or_address(bad), "{bad}");
        }
    }

    use super::*;
    use primitive_types::U512;
    use qtc_cuda::DeviceInfo;
    use std::io::{Read, Write};
    use std::time::Instant;

    fn shared() -> Arc<Shared> {
        let shared = Shared::new(
            "stratum+tcp://pool:7049".into(),
            vec![DeviceInfo {
                ordinal: 0,
                name: "Test GPU".into(),
                capability: 120,
                pci_bus_id: Some("0000:01:00.0".into()),
            }],
        );
        shared.accepted.store(5, Ordering::Relaxed);
        shared.rejected.store(1, Ordering::Relaxed);
        shared.pool_connected.store(true, Ordering::Relaxed);
        shared.set_work(
            "job".into(),
            [0; 32],
            U512::from(18_253_611_008u64),
            U512::MAX,
            vec![],
        );
        let now = Instant::now();
        shared.gpus[0].record_batch(now - Duration::from_secs(2), 2_000_000_000);
        Arc::new(shared)
    }

    #[test]
    fn bind_defaults_to_localhost() {
        assert_eq!(parse_bind("4068").unwrap().to_string(), "127.0.0.1:4068");
        assert_eq!(
            parse_bind("127.0.0.1:4068").unwrap().to_string(),
            "127.0.0.1:4068"
        );
        assert!(parse_bind("nope").is_err());
    }

    /// The fields the manager app reads.
    #[test]
    fn summary_has_the_fields_the_manager_reads() {
        let value = summary(&shared());
        assert_eq!(value["accepted_shares"], 5);
        assert_eq!(value["invalid_shares"], 1);
        assert_eq!(value["pool"]["connected"], true);
        assert_eq!(value["pool"]["difficulty"], "18253611008");
        assert_eq!(value["dev_fee_percent"], 0, "the test state sets no fee");
        assert!(value["uptime"].is_u64());
        let total = value["hashrate"].as_f64().unwrap();
        assert!(total > 900e6 && total < 1100e6, "hashrate {total}");
        let gpu = &value["gpus"][0];
        assert_eq!(gpu["pci_bus_id"], "0000:01:00.0");
        // Read a moment apart from the total, so equal only to within the clock.
        let per_gpu = gpu["hashrate"].as_f64().unwrap();
        assert!(
            (per_gpu - total).abs() < total * 0.01,
            "{per_gpu} vs {total}"
        );
    }

    #[test]
    fn serves_summary_over_http() {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let bind = probe.local_addr().unwrap();
        drop(probe);
        let shared = shared();
        let server = run(bind, shared.clone()).unwrap();

        let get_as = |path: &str, host: &str| {
            let mut stream = std::net::TcpStream::connect(bind).unwrap();
            write!(
                stream,
                "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut text = String::new();
            stream.read_to_string(&mut text).unwrap();
            text
        };
        let get = |path: &str| get_as(path, "127.0.0.1");
        // A page in a browser reaches this port under a name of its own.
        assert!(get_as("/summary", "evil.example").starts_with("HTTP/1.1 403"));
        let reply = get("/summary");
        assert!(reply.starts_with("HTTP/1.1 200"));
        let body = reply.split("\r\n\r\n").nth(1).unwrap();
        let value: Value = serde_json::from_str(body).unwrap();
        assert_eq!(value["algo"], "quantus");
        assert!(get("/other").starts_with("HTTP/1.1 404"));

        shared.request_stop();
        server.join().unwrap();
    }
}
