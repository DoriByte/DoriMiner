use crate::pool::{connect, send_line, LineReader, PoolConfig};
use crate::state::Shared;
use prl_pow::PoolJob;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SUBSCRIBE_ID: u64 = 1;
const AUTHORIZE_ID: u64 = 2;
const FIRST_SUBMIT_ID: u64 = 100;
const POLL_INTERVAL: Duration = Duration::from_millis(20);

const SILENCE_LIMIT: Duration = Duration::from_secs(180);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(20);

const ANSWER_LIMIT: Duration = Duration::from_secs(120);

const STALE_CODE: i64 = 21;
const MAX_JOB_ID: usize = 128;

#[derive(Clone)]
pub struct Work {
    pub generation: u64,
    pub job: Arc<PoolJob>,

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

    fn set(&self, job: PoolJob, dev: bool) -> bool {
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

pub struct Share {

    pub dev: bool,
    pub generation: u64,
    pub job_id: String,

    pub proof: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Accepted,
    Stale,
    Rejected(String),
}

#[derive(Debug, PartialEq)]
pub enum Incoming {
    Job { job: PoolJob, height: Option<u64> },
    BadJob(String),
    Authorized { compressed: bool },
    Refused(String),
    Submitted { id: u64, outcome: Outcome },
    Other,
}

fn error_text(error: &Value) -> String {
    match error {
        Value::Array(a) => a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join(" "),
        Value::Object(o) => o.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| error.to_string()),
        other => other.to_string(),
    }
}

fn error_code(error: &Value) -> Option<i64> {
    match error {
        Value::Array(a) => a.first().and_then(Value::as_i64),
        Value::Object(o) => o.get("code").and_then(Value::as_i64),
        _ => None,
    }
}

pub fn parse_incoming(line: &str) -> Result<Incoming, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("not JSON: {e}"))?;
    if v.get("method").and_then(Value::as_str) == Some("mining.notify") {
        let p = &v["params"];
        let text = |k: &str| p.get(k).and_then(Value::as_str);
        let (Some(id), Some(header), Some(target)) = (text("job_id"), text("header"), text("target")) else {
            return Ok(Incoming::BadJob("a job without job_id, header or target".into()));
        };
        if id.len() > MAX_JOB_ID || id.chars().any(char::is_control) {
            return Ok(Incoming::BadJob("a job id that is too long or not printable".into()));
        }
        return Ok(match PoolJob::parse(id, header, target) {
            Ok(job) => Incoming::Job { job, height: p.get("height").and_then(Value::as_u64) },
            Err(e) => Incoming::BadJob(e),
        });
    }
    match v.get("id").and_then(Value::as_u64) {
        Some(AUTHORIZE_ID) => {
            let ok = v.get("result").is_some_and(|r| r == &Value::Bool(true) || r.get("type").is_some()) && v.get("error").is_none_or(Value::is_null);
            if !ok {
                return Ok(Incoming::Refused(v.get("error").map(error_text).unwrap_or_else(|| line.to_string())));
            }

            let v2 = |x: &Value| x.get("type").and_then(Value::as_str) == Some("v2");
            Ok(Incoming::Authorized { compressed: v2(&v) || v2(&v["result"]) })
        }
        Some(id) if id >= FIRST_SUBMIT_ID => {
            let outcome = if v.get("result") == Some(&Value::Bool(true)) {
                Outcome::Accepted
            } else {
                let error = v.get("error").cloned().unwrap_or(Value::Null);
                if error_code(&error) == Some(STALE_CODE) { Outcome::Stale } else { Outcome::Rejected(error_text(&error)) }
            };
            Ok(Incoming::Submitted { id, outcome })
        }
        _ => Ok(Incoming::Other),
    }
}

fn wallet_and_worker(login: &str) -> (&str, &str) {
    match login.find(['.', '/']) {
        Some(i) => (&login[..i], &login[i + 1..]),
        None => (login, ""),
    }
}

pub fn login_lines(cfg: &PoolConfig) -> [String; 2] {
    let (wallet, worker) = wallet_and_worker(&cfg.login);
    [
        json!({ "id": SUBSCRIBE_ID, "method": "mining.subscribe", "params": [cfg.agent] }).to_string(),
        json!({ "id": AUTHORIZE_ID, "method": "mining.authorize", "params": {
            "wallet": wallet, "worker": if worker.is_empty() { "dori" } else { worker }, "pass": cfg.password, "agent": cfg.agent, "type": "v2" } })
        .to_string(),
    ]
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(bytes).expect("writing to memory");
    z.finish().expect("writing to memory")
}

pub fn submit_line(id: u64, cfg: &PoolConfig, share: &Share, compressed: bool) -> String {
    let (_, worker) = wallet_and_worker(&cfg.login);
    let proof = if compressed { prl_pow::base64(&gzip(&share.proof)) } else { prl_pow::base64(&share.proof) };
    json!({ "id": id, "method": "mining.submit", "params": {
        "job_id": share.job_id, "plain_proof": proof, "worker": if worker.is_empty() { "dori" } else { worker } } })
    .to_string()
}

fn session(cfg: &PoolConfig, shared: &Shared, jobs: &Jobs, shares: &Receiver<Share>, logged_in: &mut bool) -> Result<(), String> {
    let mut transport = connect(cfg)?;
    let transport = transport.as_mut();
    for line in login_lines(cfg) {
        send_line(transport, &line)?;
    }
    let mut reader = LineReader { buffer: Vec::new() };
    let mut compressed: Option<bool> = None;
    let mut pending: HashMap<u64, Instant> = HashMap::new();
    let mut next_id = FIRST_SUBMIT_ID;
    let connected = Instant::now();
    let mut last_line = Instant::now();

    let dev = cfg.dev;
    let turn = || shared.fee.active.load(Ordering::SeqCst) == dev;
    let (accepted, rejected, stale) =
        if dev { (&shared.fee.accepted, &shared.fee.rejected, &shared.fee.stale) } else { (&shared.accepted, &shared.rejected, &shared.stale) };
    let mut latest: Option<PoolJob> = None;
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
        if compressed.is_none() && connected.elapsed() > LOGIN_TIMEOUT {
            return Err(format!("no answer to the login in {} seconds", LOGIN_TIMEOUT.as_secs()));
        }
        for line in reader.poll(transport)? {
            let incoming = parse_incoming(&line);
            if incoming.is_ok() {
                last_line = Instant::now();
            }
            match incoming {
                Ok(Incoming::Authorized { compressed: v2 }) => {
                    compressed = Some(v2);
                    *logged_in = true;
                    if !dev {
                        shared.pool_connected.store(true, Ordering::SeqCst);
                    }
                    log::info!(target: crate::ui::GOOD, "logged in to {} as {}{}", cfg.url(), cfg.login, if v2 { " (proofs compressed)" } else { "" });
                }
                Ok(Incoming::Refused(message)) => return Err(format!("the pool refused the login: {message}")),
                Ok(Incoming::Job { job, height }) => {
                    latest = Some(job.clone());
                    let id = job.id.clone();
                    if turn() && jobs.set(job, dev) {
                        log::info!(target: crate::ui::JOB, "new job {}{id}{}", if dev { "for the dev fee " } else { "" }, height.map(|h| format!(" (height {h})")).unwrap_or_default());
                    }
                }
                Ok(Incoming::BadJob(e)) => {
                    log::error!("a job that cannot be mined ({e}), idle until the next: {line}");
                    latest = None;
                    if turn() {
                        jobs.clear();
                    }
                }
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
                Ok(Incoming::Other) => log::debug!("pool: {line}"),
                Err(e) => log::warn!("unreadable pool message ({e}): {line}"),
            }
        }
        if dev {
            shared.fee.ready.store(compressed.is_some() && latest.is_some(), Ordering::SeqCst);
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
            match compressed {
                None => {
                    stale.fetch_add(1, Ordering::Relaxed);
                    log::info!("dropped a share for job {}: found before this login", share.job_id);
                }

                Some(_) if share.generation != jobs.generation() => {
                    stale.fetch_add(1, Ordering::Relaxed);
                    log::info!(target: crate::ui::DETAIL, "dropped a share for job {}: the job changed", share.job_id);
                }
                Some(v2) => {
                    let id = next_id;
                    next_id += 1;
                    send_line(transport, &submit_line(id, cfg, &share, v2))?;
                    pending.insert(id, Instant::now());
                }
            }
        }
        pending.retain(|_, sent| sent.elapsed() < ANSWER_LIMIT);
        if last_line.elapsed() > SILENCE_LIMIT {
            return Err(format!("no message from the pool for {} seconds", SILENCE_LIMIT.as_secs()));
        }
    }
}

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

    fn cfg(login: &str) -> PoolConfig {
        PoolConfig {
            host: "prl.kryptex.network".into(),
            port: 7048,
            tls: false,
            tls_fingerprint: None,
            login: login.into(),
            password: "x".into(),
            agent: "DoriMiner/test".into(),
            fast_poll: false,
            submit_late: false,
            dev: false,
        }
    }

    const HEADER: &str = "00000020aabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccdd1122334411223344112233441122334411223344112233441122334411223344c0ffee0016b37317";

    #[test]
    fn reads_what_the_pool_sends() {
        let notify = format!(r#"{{"jsonrpc":"2.0","method":"mining.notify","params":{{"job_id":"8382f7fc_4194304","header":"{HEADER}","target":"00000000000003ffffffffffffffffffffffffffffffffffffffffffffffffff","height":120501,"cert_version":3}}}}"#);
        match parse_incoming(&notify).unwrap() {
            Incoming::Job { job, height } => {
                assert_eq!(job.id, "8382f7fc_4194304");
                assert_eq!(height, Some(120_501));
                assert_eq!(job.target_be[6], 0x03);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(parse_incoming(r#"{"method":"mining.notify","params":{"job_id":"x","header":"00","target":"00"}}"#).unwrap(), Incoming::BadJob(_)));
        assert_eq!(parse_incoming(r#"{"id":2,"result":true,"error":null,"type":"v2"}"#).unwrap(), Incoming::Authorized { compressed: true });
        assert_eq!(parse_incoming(r#"{"id":2,"result":true,"error":null}"#).unwrap(), Incoming::Authorized { compressed: false });
        assert!(matches!(parse_incoming(r#"{"id":2,"result":false,"error":[24,"bad wallet"]}"#).unwrap(), Incoming::Refused(m) if m.contains("bad wallet")));
        assert_eq!(parse_incoming(r#"{"id":101,"result":true,"error":null}"#).unwrap(), Incoming::Submitted { id: 101, outcome: Outcome::Accepted });
        assert_eq!(parse_incoming(r#"{"id":102,"result":null,"error":[21,"Job not found"]}"#).unwrap(), Incoming::Submitted { id: 102, outcome: Outcome::Stale });
        assert!(matches!(parse_incoming(r#"{"id":103,"result":false,"error":{"code":23,"message":"Invalid share"}}"#).unwrap(), Incoming::Submitted { id: 103, outcome: Outcome::Rejected(m) } if m == "Invalid share"));
        assert_eq!(parse_incoming(r#"{"id":1,"result":[null,"abc"],"error":null}"#).unwrap(), Incoming::Other);
        assert!(parse_incoming("not json").is_err());
    }

    #[test]
    fn login_and_submit_lines() {
        let [sub, auth] = login_lines(&cfg("prl1qwallet.rig1"));
        assert!(sub.contains("mining.subscribe"));
        let a: Value = serde_json::from_str(&auth).unwrap();
        assert_eq!(a["params"]["wallet"], "prl1qwallet");
        assert_eq!(a["params"]["worker"], "rig1");
        assert_eq!(a["params"]["type"], "v2");
        let share = Share { dev: false, generation: 1, job_id: "j".into(), proof: vec![0u8; 4000] };
        let plain: Value = serde_json::from_str(&submit_line(100, &cfg("prl1qwallet"), &share, false)).unwrap();
        let packed: Value = serde_json::from_str(&submit_line(101, &cfg("prl1qwallet"), &share, true)).unwrap();
        assert_eq!(plain["params"]["worker"], "dori");
        let (p, z) = (plain["params"]["plain_proof"].as_str().unwrap(), packed["params"]["plain_proof"].as_str().unwrap());
        assert!(z.len() * 10 < p.len(), "zeros compress: {} vs {}", z.len(), p.len());
    }

    #[test]
    fn a_job_sent_again_is_not_a_new_job() {
        let jobs = Jobs::default();
        let job = PoolJob::parse("j", HEADER, &"ff".repeat(32)).unwrap();
        assert!(jobs.set(job.clone(), false));
        let g = jobs.generation();
        assert!(!jobs.set(job.clone(), false));
        assert_eq!(jobs.generation(), g);
        assert!(jobs.set(job, true) && jobs.current_is_dev(), "the same job from the other link is a change of link");
        jobs.clear();
        assert!(jobs.current().is_none() && jobs.generation() > g);
    }
}
