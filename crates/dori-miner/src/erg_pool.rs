//! Client for the Ergo (ERG) pool dialect, as the open pools speak it (Miningcore's Blockchain/Ergo, 2026-10-28):
//! newline-delimited JSON over TCP or TLS with array parameters. `mining.subscribe` answers
//! `[subscriptions, extranonce1, extranonce2_size]` (the nonce is 8 bytes: extranonce1 ‖ extranonce2);
//! `mining.notify` carries `[job_id, height, msg, "", "", version, b, "", clean]` with b the share's target in
//! decimal (a share is valid when its hit < b); `mining.submit` sends `[worker, job_id, extranonce2, "", nonce]`.
//! The Lithos client (the decentralised pool's local stratum, 127.0.0.1:4444) speaks the same dialect with two
//! differences: its answers carry the request's id as text (`"id":"1"`), and it may hand a new extranonce1 in the
//! middle of a link with `mining.set_extranonce` `[extranonce1, extranonce2_size]` (after a duplicate nonce).

use crate::pool::{connect, send_line, LineReader, PoolConfig};
use crate::state::Shared;
use primitive_types::U256;
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
/// Ergo blocks are about 2 minutes apart and the pools resend the job in between; this long without a line means a
/// dead link.
const SILENCE_LIMIT: Duration = Duration::from_secs(300);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(20);
/// A share is answered in well under a second; this long without an answer means the link is dead even though the
/// socket still looks open (29/10: a 1-minute network drop left the miner sending shares into it for 5 minutes).
const ANSWER_DEADLINE: Duration = Duration::from_secs(30);
const ANSWER_FORGET: Duration = Duration::from_secs(120);
const MAX_JOB_ID: usize = 128;

/// A job as the pool sends it, with the extranonce1 of its link.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ErgJob {
    pub id: String,
    pub height: u32,
    pub msg: [u8; 32],
    pub target: U256,
    pub clean: bool,
    pub extranonce1: String,
    pub extranonce2_size: usize,
}

impl ErgJob {
    /// The nonce's fixed part: extranonce1 in the top bytes, as a number (the card counts up from it).
    pub fn nonce_prefix(&self) -> Result<u64, String> {
        let b = unhex(&self.extranonce1)?;
        if b.len() + self.extranonce2_size != 8 {
            return Err(format!("an extranonce of {} + {} bytes (the nonce is 8)", b.len(), self.extranonce2_size));
        }
        Ok(b.iter().fold(0u64, |a, &x| (a << 8) | x as u64).checked_shl(8 * self.extranonce2_size as u32).unwrap_or(0))
    }
}

fn unhex(h: &str) -> Result<Vec<u8>, String> {
    if h.len() % 2 != 0 || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("not hex: {h:.40}"));
    }
    Ok((0..h.len() / 2).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap()).collect())
}

#[derive(Clone)]
pub struct Work {
    pub generation: u64,
    pub job: Arc<ErgJob>,
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
    fn set(&self, job: ErgJob, dev: bool) -> bool {
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

/// A share that meets the pool's target (checked on the CPU, exactly, from the card's candidate).
pub struct Share {
    pub dev: bool,
    pub generation: u64,
    pub job_id: String,
    pub extranonce1: String,
    pub extranonce2_size: usize,
    pub nonce: u64,
}

impl Share {
    pub fn nonce_hex(&self) -> String {
        format!("{:016x}", self.nonce)
    }
    /// The nonce's extranonce2 part, as hex.
    pub fn extranonce2(&self) -> String {
        self.nonce_hex()[16 - 2 * self.extranonce2_size..].to_string()
    }
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
    Job(ErgJob),
    BadJob(String),
    Subscribed { extranonce1: String, extranonce2_size: usize },
    /// A new extranonce1 in the middle of the link (Lithos).
    Extranonce { extranonce1: String, extranonce2_size: usize },
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
        Value::String(t) => t.clone(),
        other => other.to_string(),
    }
}

/// A rejection that only says the job is gone.
fn is_stale(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    // the Lithos client says "21: solution was for old block" for a share found as the job changed
    t.contains("stale") || t.contains("job not found") || t.contains("unknown job") || t.contains("old block")
}

/// A number the pool may send as a JSON number or as text.
fn as_u64(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// An extranonce1 and the size of extranonce2, when they make the 8 bytes of an Ergo nonce.
fn extranonce(en1: Option<&Value>, size: Option<&Value>) -> Option<(String, usize)> {
    let (e, s) = (en1?.as_str()?, as_u64(size?)? as usize);
    (unhex(e).is_ok() && e.len() / 2 + s == 8).then(|| (e.to_ascii_lowercase(), s))
}

/// The target b: decimal text (Miningcore), or a JSON number (exact up to 2^53; above that the last digits are the
/// float's, which only moves the target by a part in 10^15).
fn as_target(v: &Value) -> Result<U256, String> {
    match v {
        Value::String(s) => erg_cuda::target_from_decimal(s),
        Value::Number(n) => match n.as_u64() {
            Some(x) => Ok(U256::from(x)),
            None => {
                let f = n.as_f64().filter(|f| f.is_finite() && *f >= 1.0).ok_or("a target that is not a positive number")?;
                let (m, e) = (f.log2().floor() as i32 - 52, f);
                let mant = (e / 2f64.powi(m)) as u64;
                Ok(if m >= 0 { U256::from(mant) << m as usize } else { U256::from(mant) >> (-m) as usize })
            }
        },
        _ => Err("a job without a target (b)".into()),
    }
}

pub fn parse_incoming(line: &str) -> Result<Incoming, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("not JSON: {e}"))?;
    if let Some(method) = v.get("method").and_then(Value::as_str) {
        let p = &v["params"];
        return Ok(match method {
            "mining.set_difficulty" => match p.get(0).and_then(Value::as_f64) {
                Some(d) if d.is_finite() && d > 0.0 => Incoming::Difficulty(d),
                _ => Incoming::Other,
            },
            "mining.notify" => {
                let id = p.get(0).map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).unwrap_or_default();
                let height = p.get(1).and_then(as_u64).unwrap_or(0);
                let msg = p.get(2).and_then(Value::as_str).map(unhex);
                let target = as_target(&p[6]);
                if id.is_empty() || id.len() > MAX_JOB_ID || id.chars().any(char::is_control) {
                    Incoming::BadJob("a job id that is empty, too long or not printable".into())
                } else if height == 0 || height > u32::MAX as u64 {
                    Incoming::BadJob("a job without a height".into())
                } else {
                    match (msg, target) {
                        (Some(Ok(m)), Ok(target)) if m.len() == 32 && !target.is_zero() => Incoming::Job(ErgJob {
                            id,
                            height: height as u32,
                            msg: m.try_into().unwrap(),
                            target,
                            clean: p.get(8).and_then(Value::as_bool).unwrap_or(false),
                            ..ErgJob::default()
                        }),
                        (_, Err(e)) => Incoming::BadJob(e),
                        _ => Incoming::BadJob("a job whose msg is not 32 bytes of hex (or a target of 0)".into()),
                    }
                }
            }
            "mining.set_extranonce" => match extranonce(p.get(0), p.get(1)) {
                Some((extranonce1, extranonce2_size)) => Incoming::Extranonce { extranonce1, extranonce2_size },
                None => return Err("an extranonce that is not 8 bytes in all".into()),
            },
            "mining.ping" => Incoming::Ping(v.get("id").cloned().unwrap_or(Value::Null)),
            _ => Incoming::Other,
        });
    }
    let error = v.get("error").cloned().unwrap_or(Value::Null);
    let result = v.get("result").cloned().unwrap_or(Value::Null);
    let ok = !result.is_null() && result != Value::Bool(false) && error.is_null();
    // the id as a number, or as text (the Lithos client answers "id":"1")
    match v.get("id").and_then(as_u64) {
        Some(SUBSCRIBE_ID) => {
            Ok(match extranonce(result.get(1), result.get(2)) {
                Some((extranonce1, extranonce2_size)) if ok => Incoming::Subscribed { extranonce1, extranonce2_size },
                _ => Incoming::Refused(format!("the subscription failed (an Ergo pool answers an extranonce of 8 bytes in all): {line:.200}")),
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
        json!({ "id": SUBSCRIBE_ID, "method": "mining.subscribe", "params": [cfg.agent] }).to_string(),
        json!({ "id": AUTHORIZE_ID, "method": "mining.authorize", "params": [cfg.login, cfg.password] }).to_string(),
    ]
}

pub fn submit_line(id: u64, cfg: &PoolConfig, share: &Share) -> String {
    json!({ "id": id, "method": "mining.submit", "params": [cfg.login, share.job_id, share.extranonce2(), "", share.nonce_hex()] }).to_string()
}

fn session(cfg: &PoolConfig, shared: &Shared, jobs: &Jobs, shares: &Receiver<Share>, logged_in: &mut bool) -> Result<(), String> {
    let mut transport = connect(cfg)?;
    let transport = transport.as_mut();
    for line in login_lines(cfg) {
        send_line(transport, &line)?;
    }
    let mut reader = LineReader { buffer: Vec::new() };
    let (mut extranonce, mut authorized) = (None::<(String, usize)>, false);
    let mut pending: HashMap<u64, Instant> = HashMap::new();
    let mut next_id = FIRST_SUBMIT_ID;
    let connected = Instant::now();
    let mut last_line = Instant::now();
    let dev = cfg.dev;
    let turn = || shared.fee.active.load(Ordering::SeqCst) == dev;
    let (accepted, rejected, stale) =
        if dev { (&shared.fee.accepted, &shared.fee.rejected, &shared.fee.stale) } else { (&shared.accepted, &shared.rejected, &shared.stale) };
    let mut latest: Option<ErgJob> = None;
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
                Ok(Incoming::Extranonce { extranonce1, extranonce2_size }) => {
                    log::info!(target: crate::ui::DETAIL, "{}new extranonce1 {extranonce1} from the pool", if dev { "dev fee " } else { "" });
                    extranonce = Some((extranonce1.clone(), extranonce2_size));
                    // the cards move to the new nonce space at once; shares of the old one are dropped as before a login
                    if let Some(job) = latest.as_mut() {
                        job.extranonce1 = extranonce1;
                        job.extranonce2_size = extranonce2_size;
                        if turn() {
                            jobs.set(job.clone(), dev);
                        }
                    }
                }
                Ok(Incoming::Difficulty(d)) => log::debug!("{}set_difficulty {d} (the share target comes in the job)", if dev { "dev fee " } else { "" }),
                Ok(Incoming::Job(mut job)) => {
                    let Some((en1, size)) = &extranonce else {
                        log::warn!("a job before the subscription's answer: skipped");
                        continue;
                    };
                    job.extranonce1 = en1.clone();
                    job.extranonce2_size = *size;
                    latest = Some(job.clone());
                    let (id, height, d) = (job.id.clone(), job.height, erg_cuda::difficulty(&job.target));
                    if turn() && jobs.set(job, dev) {
                        log::info!(target: crate::ui::JOB, "new job {}{id} (height {height}, share difficulty {})", if dev { "for the dev fee " } else { "" }, crate::erg::format_difficulty(d));
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
            if !authorized || extranonce.as_ref().map(|e| &e.0) != Some(&share.extranonce1) {
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
        // dead link: a share unanswered for ANSWER_DEADLINE and nothing at all from the pool in that time (a pool that
        // is alive but leaves a submit unanswered keeps the link; its entry goes after ANSWER_FORGET)
        if last_line.elapsed() > ANSWER_DEADLINE && pending.values().any(|sent| sent.elapsed() > ANSWER_DEADLINE) {
            return Err(format!("no answer to a share in {} seconds", ANSWER_DEADLINE.as_secs()));
        }
        pending.retain(|_, sent| sent.elapsed() < ANSWER_FORGET);
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
            host: "erg.example.org".into(),
            port: 3052,
            tls: false,
            tls_fingerprint: None,
            login: "9fexample.rig1".into(),
            password: "x".into(),
            agent: "DoriMiner/test".into(),
            fast_poll: false,
            submit_late: false,
            dev: false,
        }
    }

    #[test]
    fn reads_what_the_pool_sends() {
        let notify = r#"{"id":null,"method":"mining.notify","params":["1a2b",1904531,"548c3e602a8f36f8f2738f5f643b02425038044d98543a51cabaa9785e7e864f","","",2,"7067388259113537318333190002971674063283542741642755394446115914399301849","",true]}"#;
        match parse_incoming(notify).unwrap() {
            Incoming::Job(j) => {
                assert_eq!(j.id, "1a2b");
                assert_eq!(j.height, 1_904_531);
                assert_eq!(erg_cuda::hex(&j.msg), erg_cuda::VECTOR_MSG);
                assert!((erg_cuda::difficulty(&j.target) - 16384.0).abs() < 1.0);
                assert!(j.clean);
            }
            other => panic!("{other:?}"),
        }
        // the height as text and the target as a JSON number also read
        let n2 = r#"{"method":"mining.notify","params":[7,"614400","548c3e602a8f36f8f2738f5f643b02425038044d98543a51cabaa9785e7e864f","","",2,123456789,"",false]}"#;
        assert!(matches!(parse_incoming(n2).unwrap(), Incoming::Job(j) if j.id == "7" && j.height == 614_400 && j.target == U256::from(123_456_789u64)));
        let bad = notify.replace("\"7067388259113537318333190002971674063283542741642755394446115914399301849\"", "\"\"");
        assert!(matches!(parse_incoming(&bad).unwrap(), Incoming::BadJob(_)));
        assert_eq!(
            parse_incoming(r#"{"id":1,"result":[[["mining.set_difficulty","x"],["mining.notify","x"]],"a1b2",6],"error":null}"#).unwrap(),
            Incoming::Subscribed { extranonce1: "a1b2".into(), extranonce2_size: 6 }
        );
        assert!(matches!(parse_incoming(r#"{"id":1,"result":[[],"a1b2",4],"error":null}"#).unwrap(), Incoming::Refused(_)));
        assert_eq!(parse_incoming(r#"{"id":2,"result":true,"error":null}"#).unwrap(), Incoming::Authorized);
        assert_eq!(parse_incoming(r#"{"id":100,"result":true,"error":null}"#).unwrap(), Incoming::Submitted { id: 100, outcome: Outcome::Accepted });
        assert_eq!(parse_incoming(r#"{"id":101,"result":null,"error":[21,"Job not found",null]}"#).unwrap(), Incoming::Submitted { id: 101, outcome: Outcome::Stale });
        assert_eq!(parse_incoming(r#"{"id":null,"method":"mining.set_difficulty","params":[1]}"#).unwrap(), Incoming::Difficulty(1.0));
    }

    /// The lines of the Lithos client 1.0.6 (JStratum: the id comes back as text; `Response.subscribe`,
    /// `Announcement.extraNonce1`, `BlockTemplate.buildJobParams` with the version in hex and b in decimal).
    #[test]
    fn reads_what_lithos_sends() {
        assert_eq!(
            parse_incoming(r#"{"id":"1","result":[null,"0a1b2c3d",4],"error":null}"#).unwrap(),
            Incoming::Subscribed { extranonce1: "0a1b2c3d".into(), extranonce2_size: 4 }
        );
        assert_eq!(parse_incoming(r#"{"id":"2","result":true,"error":null}"#).unwrap(), Incoming::Authorized);
        assert_eq!(parse_incoming(r#"{"id":"100","result":true,"error":null}"#).unwrap(), Incoming::Submitted { id: 100, outcome: Outcome::Accepted });
        assert_eq!(
            parse_incoming(r#"{"id":"101","result":null,"error":"23: duplicate share"}"#).unwrap(),
            Incoming::Submitted { id: 101, outcome: Outcome::Rejected("23: duplicate share".into()) }
        );
        assert_eq!(
            parse_incoming(r#"{"id":null,"method":"mining.set_extranonce","params":["0A1B2C3E",4]}"#).unwrap(),
            Incoming::Extranonce { extranonce1: "0a1b2c3e".into(), extranonce2_size: 4 }
        );
        assert!(parse_incoming(r#"{"id":null,"method":"mining.set_extranonce","params":["0a1b2c",4]}"#).is_err());
        assert_eq!(
            parse_incoming(r#"{"id":"102","result":null,"error":"21: solution was for old block"}"#).unwrap(),
            Incoming::Submitted { id: 102, outcome: Outcome::Stale }
        );
        let notify = r#"{"id":null,"method":"mining.notify","params":["5",1904531,"548c3e602a8f36f8f2738f5f643b02425038044d98543a51cabaa9785e7e864f","","","2","7067388259113537318333190002971674063283542741642755394446115914399301849","",true]}"#;
        assert!(matches!(parse_incoming(notify).unwrap(), Incoming::Job(j) if j.id == "5" && j.height == 1_904_531 && j.clean));
    }

    #[test]
    fn nonce_and_submit() {
        let j = ErgJob { extranonce1: "a1b2".into(), extranonce2_size: 6, ..ErgJob::default() };
        assert_eq!(j.nonce_prefix().unwrap(), 0xa1b2_0000_0000_0000);
        assert!(ErgJob { extranonce1: "a1b2".into(), extranonce2_size: 4, ..ErgJob::default() }.nonce_prefix().is_err());
        let share = Share { dev: false, generation: 1, job_id: "1a2b".into(), extranonce1: "a1b2".into(), extranonce2_size: 6, nonce: 0xa1b2_0000_0012_3456 };
        let v: Value = serde_json::from_str(&submit_line(100, &cfg(), &share)).unwrap();
        assert_eq!(v["params"], json!(["9fexample.rig1", "1a2b", "000000123456", "", "a1b2000000123456"]));
    }
}
