//! Client for the Vela (VELA) pool dialect (Suprnova, 2026-10-24): newline-delimited JSON over TCP or TLS with object
//! parameters. `mining.subscribe` answers the extranonce1; `mining.notify` carries a Bitcoin-style job (coinbase halves,
//! merkle branch, version, bits, ntime) plus the model's hash and the prompt's tokens; `mining.submit` sends the
//! extranonce2, the nonce, the whole x0 (hex) and hashPoI. Mined with the prototype on 2026-10-24: 5 of 5 accepted.

use crate::pool::{connect, send_line, LineReader, PoolConfig};
use crate::state::Shared;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SUBSCRIBE_ID: u64 = 1;
const AUTHORIZE_ID: u64 = 2;
const FIRST_SUBMIT_ID: u64 = 100;
const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Vela blocks are 5 minutes apart and the pool pings in between; this long without a line means a dead link.
const SILENCE_LIMIT: Duration = Duration::from_secs(360);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(20);
const ANSWER_LIMIT: Duration = Duration::from_secs(120);
const MAX_JOB_ID: usize = 128;

/// A job as the pool sends it, with the difficulty and extranonce1 of its link.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct VelaJob {
    pub id: String,
    pub prev_hash: String,
    pub coinbase1: String,
    pub coinbase2: String,
    pub branch: Vec<String>,
    pub version: String,
    pub bits: String,
    pub ntime: String,
    pub model_hash: String,
    pub height: u64,
    pub clean: bool,
    pub tokens: Vec<i32>,
    pub difficulty: f64,
    pub extranonce1: String,
    pub extranonce2_size: usize,
}

fn unhex(h: &str) -> Result<Vec<u8>, String> {
    if h.len() % 2 != 0 || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("not hex: {h:.40}"));
    }
    Ok((0..h.len() / 2).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap()).collect())
}

impl VelaJob {
    /// The 76 bytes of the header before the nonce, for a given extranonce2 (hex): version, previous hash (reversed),
    /// merkle root, ntime, bits, all little-endian as in Bitcoin.
    pub fn header76(&self, extranonce2: &str) -> Result<[u8; 76], String> {
        let mut cb = unhex(&self.coinbase1)?;
        cb.extend(unhex(&self.extranonce1)?);
        cb.extend(unhex(extranonce2)?);
        cb.extend(unhex(&self.coinbase2)?);
        let mut root = vela_cuda::sha256d(&cb);
        for b in &self.branch {
            let mut pair = root.to_vec();
            pair.extend(unhex(b)?);
            if pair.len() != 64 {
                return Err("a merkle branch entry that is not 32 bytes".into());
            }
            root = vela_cuda::sha256d(&pair);
        }
        let word = |h: &str| u32::from_str_radix(h, 16).map_err(|_| format!("not a 32-bit hex word: {h:.20}"));
        let prev = unhex(&self.prev_hash)?;
        if prev.len() != 32 {
            return Err("a previous hash that is not 32 bytes".into());
        }
        let mut h = [0u8; 76];
        h[..4].copy_from_slice(&word(&self.version)?.to_le_bytes());
        for i in 0..32 {
            h[4 + i] = prev[31 - i];
        }
        h[36..68].copy_from_slice(&root);
        h[68..72].copy_from_slice(&word(&self.ntime)?.to_le_bytes());
        h[72..76].copy_from_slice(&word(&self.bits)?.to_le_bytes());
        Ok(h)
    }
}

#[derive(Clone)]
pub struct Work {
    pub generation: u64,
    pub job: Arc<VelaJob>,
    pub dev: bool,
}

#[derive(Default)]
pub struct Jobs {
    slot: Mutex<Option<Work>>,
    generation: AtomicU64,
}

impl Jobs {
    pub fn current(&self) -> Option<Work> {
        self.slot.lock().unwrap().clone()
    }
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
    fn set(&self, job: VelaJob, dev: bool) -> bool {
        let mut slot = self.slot.lock().unwrap();
        if slot.as_ref().is_some_and(|w| *w.job == job && w.dev == dev) {
            return false;
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *slot = Some(Work { generation, job: Arc::new(job), dev });
        true
    }
    fn current_is_dev(&self) -> bool {
        self.slot.lock().unwrap().as_ref().is_some_and(|w| w.dev)
    }
    fn clear(&self) {
        let mut slot = self.slot.lock().unwrap();
        self.generation.fetch_add(1, Ordering::SeqCst);
        *slot = None;
    }
}

/// A share that meets the pool's target (checked on the CPU from the card's x0).
pub struct Share {
    pub dev: bool,
    pub generation: u64,
    pub job_id: String,
    pub extranonce2: String,
    pub ntime: String,
    pub nonce: u32,
    pub x0: Vec<u8>,
    pub hash_poi: [u8; 32],
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Accepted,
    Stale,
    Rejected(String),
}

#[derive(Debug, PartialEq)]
pub enum Incoming {
    Difficulty(f64),
    Job(VelaJob),
    BadJob(String),
    Subscribed { extranonce1: String, extranonce2_size: usize },
    Authorized,
    Refused(String),
    Submitted { id: u64, outcome: Outcome },
    Ping(Value),
    Other,
}

fn error_text(error: &Value) -> String {
    match error {
        Value::Array(a) => a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join(" "),
        Value::Object(o) => o.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| error.to_string()),
        other => other.to_string(),
    }
}

/// A rejection that only says the job is gone.
fn is_stale(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("stale") || t.contains("job not found") || t.contains("unknown job")
}

pub fn parse_incoming(line: &str) -> Result<Incoming, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("not JSON: {e}"))?;
    if let Some(method) = v.get("method").and_then(Value::as_str) {
        let p = &v["params"];
        return Ok(match method {
            "mining.set_difficulty" => {
                let d = p.get("difficulty").and_then(Value::as_f64).or_else(|| p.get(0).and_then(Value::as_f64));
                match d {
                    Some(d) if d.is_finite() && d > 0.0 => Incoming::Difficulty(d),
                    _ => Incoming::Other,
                }
            }
            "mining.notify" => {
                let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                let job = VelaJob {
                    id: text("job_id"),
                    prev_hash: text("prev_hash"),
                    coinbase1: text("coinbase1"),
                    coinbase2: text("coinbase2"),
                    branch: p.get("merkle_branch").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
                    version: text("version"),
                    bits: text("bits"),
                    ntime: text("ntime"),
                    model_hash: text("model_hash"),
                    height: p.get("height").and_then(Value::as_u64).unwrap_or(0),
                    clean: p.get("clean_jobs").and_then(Value::as_bool).unwrap_or(false),
                    tokens: p.get("prompt_tokens").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_i64).map(|t| t as i32).collect()).unwrap_or_default(),
                    ..VelaJob::default()
                };
                if job.id.is_empty() || job.id.len() > MAX_JOB_ID || job.id.chars().any(char::is_control) {
                    Incoming::BadJob("a job id that is empty, too long or not printable".into())
                } else if job.prev_hash.len() != 64 || job.version.is_empty() || job.bits.is_empty() || job.ntime.is_empty() {
                    Incoming::BadJob("a job without a previous hash, version, bits or ntime".into())
                } else if job.tokens.len() > vela_cuda::MAX_TOKENS {
                    Incoming::BadJob(format!("a prompt of {} tokens (at most {})", job.tokens.len(), vela_cuda::MAX_TOKENS))
                } else if !job.model_hash.is_empty() && !job.model_hash.eq_ignore_ascii_case(vela_cuda::MODEL_HASH) {
                    Incoming::BadJob(format!("a job for another model ({})", job.model_hash))
                } else {
                    Incoming::Job(job)
                }
            }
            "mining.ping" => Incoming::Ping(v.get("id").cloned().unwrap_or(Value::Null)),
            _ => Incoming::Other,
        });
    }
    let error = v.get("error").cloned().unwrap_or(Value::Null);
    let result = v.get("result").cloned().unwrap_or(Value::Null);
    let ok = !result.is_null() && result != Value::Bool(false) && error.is_null();
    match v.get("id").and_then(Value::as_u64) {
        Some(SUBSCRIBE_ID) => {
            let en1 = result.get("extranonce1").and_then(Value::as_str).or_else(|| result.get(1).and_then(Value::as_str));
            let size = result.get("extranonce2_size").and_then(Value::as_u64).or_else(|| result.get(2).and_then(Value::as_u64)).unwrap_or(4) as usize;
            Ok(match en1 {
                Some(e) if ok && unhex(e).is_ok() => Incoming::Subscribed { extranonce1: e.to_string(), extranonce2_size: size.clamp(1, 16) },
                _ => Incoming::Refused(format!("the subscription failed: {line:.200}")),
            })
        }
        Some(AUTHORIZE_ID) => Ok(if ok { Incoming::Authorized } else { Incoming::Refused(error_text(&error)) }),
        Some(id) if id >= FIRST_SUBMIT_ID => {
            let outcome = if ok {
                Outcome::Accepted
            } else {
                let t = error_text(&error);
                if is_stale(&t) { Outcome::Stale } else { Outcome::Rejected(t) }
            };
            Ok(Incoming::Submitted { id, outcome })
        }
        _ => Ok(Incoming::Other),
    }
}

pub fn login_lines(cfg: &PoolConfig) -> [String; 2] {
    [
        json!({ "id": SUBSCRIBE_ID, "method": "mining.subscribe", "params": { "user_agent": cfg.agent } }).to_string(),
        json!({ "id": AUTHORIZE_ID, "method": "mining.authorize", "params": { "wallet": cfg.login, "password": cfg.password } }).to_string(),
    ]
}

pub fn submit_line(id: u64, cfg: &PoolConfig, share: &Share) -> String {
    json!({ "id": id, "method": "mining.submit", "params": {
        "worker": cfg.login, "job_id": share.job_id, "extranonce2": share.extranonce2, "ntime": share.ntime,
        "nonce": format!("{:08x}", share.nonce), "x0_hex": vela_cuda::hex(&share.x0), "hashpoi_hex": vela_cuda::hex(&share.hash_poi) } })
    .to_string()
}

fn session(cfg: &PoolConfig, shared: &Shared, jobs: &Jobs, shares: &Receiver<Share>, logged_in: &mut bool) -> Result<(), String> {
    let mut transport = connect(cfg)?;
    let transport = transport.as_mut();
    for line in login_lines(cfg) {
        send_line(transport, &line)?;
    }
    let mut reader = LineReader { buffer: Vec::new() };
    let (mut extranonce, mut authorized, mut difficulty) = (None::<(String, usize)>, false, 1.0f64);
    let mut pending: HashMap<u64, Instant> = HashMap::new();
    let mut next_id = FIRST_SUBMIT_ID;
    let connected = Instant::now();
    let mut last_line = Instant::now();
    let dev = cfg.dev;
    let turn = || shared.fee.active.load(Ordering::SeqCst) == dev;
    let (accepted, rejected, stale) =
        if dev { (&shared.fee.accepted, &shared.fee.rejected, &shared.fee.stale) } else { (&shared.accepted, &shared.rejected, &shared.stale) };
    let mut latest: Option<VelaJob> = None;
    let mut had_turn = turn();
    loop {
        if shared.stopping() || (dev && !shared.fee.wanted.load(Ordering::SeqCst)) {
            return Ok(());
        }
        let has_turn = turn();
        if has_turn && !had_turn {
            match &latest {
                Some(job) => {
                    jobs.set(job.clone(), dev);
                }
                None => jobs.clear(),
            }
        }
        had_turn = has_turn;
        if !authorized && connected.elapsed() > LOGIN_TIMEOUT {
            return Err(format!("no answer to the login in {} seconds", LOGIN_TIMEOUT.as_secs()));
        }
        for line in reader.poll(transport)? {
            let incoming = parse_incoming(&line);
            if incoming.is_ok() {
                last_line = Instant::now();
            }
            match incoming {
                Ok(Incoming::Subscribed { extranonce1, extranonce2_size }) => extranonce = Some((extranonce1, extranonce2_size)),
                Ok(Incoming::Authorized) => {
                    authorized = true;
                    *logged_in = true;
                    if !dev {
                        shared.pool_connected.store(true, Ordering::SeqCst);
                    }
                    log::info!(target: crate::ui::GOOD, "logged in to {} as {}", cfg.url(), cfg.login);
                }
                Ok(Incoming::Refused(message)) => return Err(format!("the pool refused the login: {message}")),
                Ok(Incoming::Difficulty(d)) => {
                    difficulty = d;
                    log::info!(target: crate::ui::DETAIL, "{}difficulty {d}", if dev { "dev fee " } else { "" });
                }
                Ok(Incoming::Job(mut job)) => {
                    let Some((en1, size)) = &extranonce else {
                        log::warn!("a job before the subscription's answer: skipped");
                        continue;
                    };
                    job.difficulty = difficulty;
                    job.extranonce1 = en1.clone();
                    job.extranonce2_size = *size;
                    latest = Some(job.clone());
                    let (id, height) = (job.id.clone(), job.height);
                    if turn() && jobs.set(job, dev) {
                        log::info!(target: crate::ui::JOB, "new job {}{id} (height {height}, difficulty {difficulty})", if dev { "for the dev fee " } else { "" });
                    }
                }
                Ok(Incoming::BadJob(e)) => {
                    log::error!("a job that cannot be mined ({e}), idle until the next");
                    latest = None;
                    if turn() {
                        jobs.clear();
                    }
                }
                Ok(Incoming::Ping(id)) => send_line(transport, &json!({ "id": id, "result": "pong", "error": null }).to_string())?,
                Ok(Incoming::Submitted { id, outcome }) => {
                    let Some(sent) = pending.remove(&id) else { continue };
                    let ms = format!(" ({} ms)", sent.elapsed().as_millis());
                    match outcome {
                        Outcome::Accepted => {
                            let n = accepted.fetch_add(1, Ordering::Relaxed) + 1;
                            log::info!(target: crate::ui::GOOD, "{}share accepted{ms}, {n} so far", if dev { "dev fee " } else { "" });
                        }
                        Outcome::Stale => {
                            stale.fetch_add(1, Ordering::Relaxed);
                            log::warn!("share was stale{ms}");
                        }
                        Outcome::Rejected(message) => {
                            rejected.fetch_add(1, Ordering::Relaxed);
                            log::warn!("share rejected{ms}: {message}");
                        }
                    }
                }
                Ok(Incoming::Other) => log::debug!("pool: {line:.300}"),
                Err(e) => log::warn!("unreadable pool message ({e}): {line:.300}"),
            }
        }
        if dev {
            shared.fee.ready.store(authorized && latest.is_some(), Ordering::SeqCst);
        }
        let share = match shares.recv_timeout(POLL_INTERVAL) {
            Ok(share) => Some(share),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                std::thread::sleep(POLL_INTERVAL);
                None
            }
        };
        if let Some(share) = share {
            if !authorized {
                stale.fetch_add(1, Ordering::Relaxed);
                log::info!("dropped a share for job {}: found before this login", share.job_id);
            } else if share.generation != jobs.generation() {
                stale.fetch_add(1, Ordering::Relaxed);
                log::info!(target: crate::ui::DETAIL, "dropped a share for job {}: the job changed", share.job_id);
            } else {
                let id = next_id;
                next_id += 1;
                send_line(transport, &submit_line(id, cfg, &share))?;
                pending.insert(id, Instant::now());
            }
        }
        pending.retain(|_, sent| sent.elapsed() < ANSWER_LIMIT);
        if last_line.elapsed() > SILENCE_LIMIT {
            return Err(format!("no message from the pool for {} seconds", SILENCE_LIMIT.as_secs()));
        }
    }
}

/// Keeps the pool connection up until a stop is requested, reconnecting with a growing delay.
pub fn run(cfg: PoolConfig, shared: Arc<Shared>, jobs: Arc<Jobs>, shares: Receiver<Share>) {
    let mut delay = Duration::from_secs(1);
    while !shared.stopping() {
        if cfg.dev && !shared.fee.wanted.load(Ordering::SeqCst) {
            while shares.try_recv().is_ok() {}
            std::thread::sleep(Duration::from_millis(100));
            delay = Duration::from_secs(1);
            continue;
        }
        let mut logged_in = false;
        let result = session(&cfg, &shared, &jobs, &shares, &mut logged_in);
        if cfg.dev {
            shared.fee.ready.store(false, Ordering::SeqCst);
            if jobs.current_is_dev() && !shared.pool_connected.load(Ordering::SeqCst) {
                jobs.clear();
            }
        } else {
            shared.pool_connected.store(false, Ordering::SeqCst);
            if !shared.fee.active.load(Ordering::SeqCst) {
                jobs.clear();
            }
        }
        let Err(e) = result else { continue };
        if logged_in {
            delay = Duration::from_secs(1);
        }
        log::warn!("{}pool connection lost: {e}; retrying in {} s", if cfg.dev { "dev fee " } else { "" }, delay.as_secs());
        let until = Instant::now() + delay;
        while Instant::now() < until && !shared.stopping() {
            std::thread::sleep(Duration::from_millis(100));
        }
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> PoolConfig {
        PoolConfig {
            host: "vela.suprnova.cc".into(),
            port: 7604,
            tls: true,
            tls_fingerprint: None,
            login: "vela1qexample.rig1".into(),
            password: "x".into(),
            agent: "DoriMiner/test".into(),
            fast_poll: false,
            submit_late: false,
            dev: false,
        }
    }

    #[test]
    fn reads_what_the_pool_sends() {
        let notify = format!(
            r#"{{"id":null,"method":"mining.notify","params":{{"job_id":"0647c9","prev_hash":"{}","coinbase1":"01","coinbase2":"02","merkle_branch":[],"version":"20000000","bits":"1d00ffff","ntime":"6718a2b0","model_hash":"{}","height":2798,"clean_jobs":true,"prompt_tokens":[320,736,3240]}}}}"#,
            "11".repeat(32),
            vela_cuda::MODEL_HASH
        );
        match parse_incoming(&notify).unwrap() {
            Incoming::Job(j) => {
                assert_eq!(j.id, "0647c9");
                assert_eq!(j.tokens, vec![320, 736, 3240]);
                assert!(j.clean);
                assert_eq!(j.height, 2798);
            }
            other => panic!("{other:?}"),
        }
        let other_model = notify.replace(vela_cuda::MODEL_HASH, &"00".repeat(32));
        assert!(matches!(parse_incoming(&other_model).unwrap(), Incoming::BadJob(_)));
        assert_eq!(parse_incoming(r#"{"id":null,"method":"mining.set_difficulty","params":{"difficulty":2048}}"#).unwrap(), Incoming::Difficulty(2048.0));
        assert_eq!(parse_incoming(r#"{"method":"mining.set_difficulty","params":[6268]}"#).unwrap(), Incoming::Difficulty(6268.0));
        assert_eq!(
            parse_incoming(r#"{"id":1,"result":{"extranonce1":"f516f56e","extranonce2_size":4},"error":null}"#).unwrap(),
            Incoming::Subscribed { extranonce1: "f516f56e".into(), extranonce2_size: 4 }
        );
        assert_eq!(parse_incoming(r#"{"id":2,"result":true,"error":null}"#).unwrap(), Incoming::Authorized);
        assert!(matches!(parse_incoming(r#"{"id":2,"result":false,"error":{"code":24,"message":"bad wallet"}}"#).unwrap(), Incoming::Refused(m) if m == "bad wallet"));
        assert_eq!(parse_incoming(r#"{"id":100,"result":true,"error":null}"#).unwrap(), Incoming::Submitted { id: 100, outcome: Outcome::Accepted });
        assert_eq!(parse_incoming(r#"{"id":101,"result":null,"error":{"code":21,"message":"Stale share"}}"#).unwrap(), Incoming::Submitted { id: 101, outcome: Outcome::Stale });
        assert!(matches!(parse_incoming(r#"{"id":7,"method":"mining.ping","params":{}}"#).unwrap(), Incoming::Ping(_)));
    }

    #[test]
    fn header_and_submit() {
        let j = VelaJob {
            id: "j".into(),
            prev_hash: (0..32u8).map(|b| format!("{b:02x}")).collect(),
            coinbase1: "aa".into(),
            coinbase2: "bb".into(),
            version: "20000000".into(),
            bits: "1d00ffff".into(),
            ntime: "6718a2b0".into(),
            extranonce1: "f516f56e".into(),
            extranonce2_size: 4,
            ..VelaJob::default()
        };
        let h = j.header76("00000001").unwrap();
        assert_eq!(&h[..4], &0x2000_0000u32.to_le_bytes());
        assert_eq!(h[4], 31);
        assert_eq!(h[35], 0);
        assert_eq!(&h[36..68], &vela_cuda::sha256d(&unhex("aaf516f56e00000001bb").unwrap()));
        assert_eq!(&h[72..76], &0x1d00_ffffu32.to_le_bytes());
        let share = Share { dev: false, generation: 1, job_id: "j".into(), extranonce2: "00000001".into(), ntime: j.ntime.clone(), nonce: 7, x0: vec![1, 2], hash_poi: [3; 32] };
        let v: Value = serde_json::from_str(&submit_line(100, &cfg(), &share)).unwrap();
        assert_eq!(v["params"]["nonce"], "00000007");
        assert_eq!(v["params"]["x0_hex"], "0102");
        assert_eq!(v["params"]["worker"], "vela1qexample.rig1");
    }
}
