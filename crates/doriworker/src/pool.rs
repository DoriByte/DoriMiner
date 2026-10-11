//! Client for the Kryptex Quantus pool dialect: newline-delimited JSON over TCP
//! or TLS, with `login`, pushed `job` notifications and `submit`.
//!
//! Kryptex does not publish this protocol: the message shapes here are the ones
//! the pool sends and accepts.

use crate::state::{Share, Shared};
use crate::worker::MAX_EXTRANONCE;
use primitive_types::U512;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LOGIN_ID: u64 = 1;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the client waits for a share before it looks at the socket again.
/// A share wakes it at once; a new job is noticed within this.
const POLL_INTERVAL: Duration = Duration::from_millis(5);
/// The pause of the fast poll.
const FAST_POLL_INTERVAL: Duration = Duration::from_millis(1);
/// How long a write may stay blocked before the link counts as dead.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// Jobs arrive every few seconds; this long without a line means a dead link.
const SILENCE_LIMIT: Duration = Duration::from_secs(90);
const KEEPALIVE_EVERY: Duration = Duration::from_secs(30);
/// How long the pool may take to answer the login before the link is dropped
/// and made again.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_LINE: usize = 64 * 1024;
/// The longest job id taken from the pool: every share carries a copy of it.
const MAX_JOB_ID: usize = 128;
/// The whole TLS handshake, however slowly the server answers.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
/// Shares sent in one pass before the pool's messages are read again.
const SHARES_PER_PASS: usize = 64;
/// Shares the pool has not answered yet: more than this and the link is not working.
const MAX_PENDING: usize = 4096;

#[derive(Clone, Debug)]
pub struct PoolConfig {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    /// SHA-256 of the server certificate to require. Without it the TLS link is
    /// encrypted but the server is not authenticated (the pool's certificate is
    /// self-signed).
    pub tls_fingerprint: Option<[u8; 32]>,
    pub login: String,
    pub password: String,
    pub agent: String,
    /// Look at the socket more often than the default poll does.
    pub fast_poll: bool,
    /// Send a share even when the job changed while it waited, instead of
    /// dropping it.
    pub submit_late: bool,
    /// The dev fee link: connected only while the schedule wants it, and its jobs
    /// reach the GPUs only during a dev fee window (`devfee.rs`).
    pub dev: bool,
}

impl PoolConfig {
    pub fn url(&self) -> String {
        let scheme = if self.tls {
            "stratum+ssl"
        } else {
            "stratum+tcp"
        };
        format!("{scheme}://{}:{}", self.host, self.port)
    }
}

/// Splits `[scheme://]host:port`. A `ssl`/`tls` scheme turns TLS on.
pub fn parse_pool_url(url: &str) -> Result<(String, u16, bool), String> {
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => (String::new(), url),
    };
    let tls = match scheme.as_str() {
        "" | "tcp" | "stratum" | "stratum+tcp" => false,
        "ssl" | "tls" | "stratum+ssl" | "stratum+tls" => true,
        other => return Err(format!("unsupported pool scheme '{other}'")),
    };
    let rest = rest.trim_end_matches('/');
    let (host, port) = rest
        .rsplit_once(':')
        .ok_or_else(|| format!("pool address '{url}' needs a port"))?;
    let port: u16 = port
        .parse()
        .map_err(|_| format!("invalid pool port '{port}'"))?;
    if host.is_empty() {
        return Err(format!("pool address '{url}' has no host"));
    }
    Ok((host.to_string(), port, tls))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolJob {
    pub job_id: String,
    pub header: [u8; 32],
    pub difficulty: U512,
    pub extranonce: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareOutcome {
    Accepted,
    Stale,
    Rejected(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    LoginOk {
        session: String,
        job: Option<PoolJob>,
        /// Why the job that came with the reply could not be used. The login
        /// itself still counts: the session is needed for the jobs that follow.
        bad_job: Option<String>,
        /// The pool answers `keepalived` pings.
        keepalive: bool,
    },
    LoginFailed(String),
    Job(PoolJob),
    /// A job notification this miner cannot mine. The pool has replaced the
    /// job before it all the same.
    BadJob(String),
    SubmitResult {
        id: u64,
        outcome: ShareOutcome,
    },
    Other,
}

fn parse_difficulty(value: &Value) -> Option<U512> {
    let difficulty = match value {
        Value::Number(n) => U512::from(n.as_u64()?),
        Value::String(s) => U512::from_dec_str(s).ok()?,
        _ => return None,
    };
    (!difficulty.is_zero()).then_some(difficulty)
}

pub fn parse_job(value: &Value) -> Result<PoolJob, String> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("job has no '{key}'"))
    };
    let header: [u8; 32] = hex::decode(text("mining_hash")?)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or("job 'mining_hash' is not 32 bytes of hex")?;
    let extranonce = match value.get("extranonce") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => hex::decode(v.as_str().ok_or("job 'extranonce' is not a string")?)
            .map_err(|_| "job 'extranonce' is not hex")?,
    };
    if extranonce.len() > MAX_EXTRANONCE {
        return Err(format!(
            "job 'extranonce' is {} bytes, more than the {MAX_EXTRANONCE} this miner can place",
            extranonce.len()
        ));
    }
    let difficulty = value
        .get("difficulty")
        .and_then(parse_difficulty)
        .ok_or("job 'difficulty' is missing or zero")?;
    let job_id = text("job_id")?;
    if job_id.len() > MAX_JOB_ID {
        return Err(format!(
            "job 'job_id' is {} bytes, more than {MAX_JOB_ID}",
            job_id.len()
        ));
    }
    Ok(PoolJob {
        job_id: job_id.to_string(),
        header,
        difficulty,
        extranonce,
    })
}

fn error_message(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

pub fn parse_incoming(line: &str) -> Result<Incoming, String> {
    let value: Value = serde_json::from_str(line).map_err(|e| format!("not JSON: {e}"))?;
    if value.get("method").and_then(Value::as_str) == Some("job") {
        let job = value
            .pointer("/params/job")
            .ok_or_else(|| "job notification without a job".to_string())
            .and_then(parse_job);
        return Ok(match job {
            Ok(job) => Incoming::Job(job),
            Err(e) => Incoming::BadJob(e),
        });
    }
    let Some(id) = value.get("id").and_then(Value::as_u64) else {
        return Ok(Incoming::Other);
    };
    let error = value.get("error").filter(|e| !e.is_null());
    if id == LOGIN_ID {
        if let Some(error) = error {
            return Ok(Incoming::LoginFailed(error_message(error)));
        }
        let result = value.get("result").ok_or("login reply without a result")?;
        let session = result
            .get("id")
            .and_then(Value::as_str)
            .ok_or("login reply without a session id")?
            .to_string();
        let (job, bad_job) = match result.get("job").filter(|j| !j.is_null()).map(parse_job) {
            Some(Ok(job)) => (Some(job), None),
            Some(Err(e)) => (None, Some(e)),
            None => (None, None),
        };
        let keepalive = result
            .get("extensions")
            .and_then(Value::as_array)
            .is_some_and(|list| list.iter().any(|e| e == "keepalive"));
        return Ok(Incoming::LoginOk {
            session,
            job,
            bad_job,
            keepalive,
        });
    }
    let outcome = match error {
        Some(error) => {
            let message = error_message(error);
            if message.to_ascii_lowercase().contains("stale") {
                ShareOutcome::Stale
            } else {
                ShareOutcome::Rejected(message)
            }
        }
        None => {
            let status = value.pointer("/result/status").and_then(Value::as_str);
            if status == Some("KEEPALIVED") {
                return Ok(Incoming::Other);
            }
            if status == Some("OK") || value.get("result") == Some(&Value::Bool(true)) {
                ShareOutcome::Accepted
            } else {
                ShareOutcome::Rejected(format!("unexpected reply: {line}"))
            }
        }
    };
    Ok(Incoming::SubmitResult { id, outcome })
}

pub fn login_line(cfg: &PoolConfig) -> String {
    json!({
        "id": LOGIN_ID,
        "method": "login",
        "params": { "agent": cfg.agent, "login": cfg.login, "pass": cfg.password },
    })
    .to_string()
}

pub fn keepalive_line(id: u64, session: &str) -> String {
    json!({ "id": id, "method": "keepalived", "params": { "id": session } }).to_string()
}

pub fn submit_line(id: u64, session: &str, share: &Share) -> String {
    json!({
        "id": id,
        "method": "submit",
        "params": {
            "id": session,
            "job_id": share.job_id,
            "nonce": hex::encode(share.nonce_be),
            "result": hex::encode(share.hash.to_big_endian()),
        },
    })
    .to_string()
}

trait Transport: Read + Write + Send {}
impl<T: Read + Write + Send> Transport for T {}

/// Accepts the server's certificate, optionally requiring a known fingerprint.
#[derive(Debug)]
struct FingerprintVerifier {
    pin: Option<[u8; 32]>,
    seen: Arc<Mutex<Option<[u8; 32]>>>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl rustls::client::danger::ServerCertVerifier for FingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let fingerprint: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
        *self.seen.lock().unwrap() = Some(fingerprint);
        match self.pin {
            Some(pin) if pin != fingerprint => Err(rustls::Error::General(format!(
                "pool certificate fingerprint is {}, expected {}",
                hex::encode(fingerprint),
                hex::encode(pin)
            ))),
            _ => Ok(rustls::client::danger::ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn connect(cfg: &PoolConfig) -> Result<Box<dyn Transport>, String> {
    let address = (cfg.host.as_str(), cfg.port)
        .to_socket_addrs()
        .map_err(|e| format!("could not resolve {}: {e}", cfg.host))?
        .next()
        .ok_or_else(|| format!("{} has no address", cfg.host))?;
    let tcp = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
        .map_err(|e| format!("could not connect to {}:{}: {e}", cfg.host, cfg.port))?;
    tcp.set_nodelay(true).ok();
    if !cfg.tls {
        tcp.set_nonblocking(true).map_err(|e| e.to_string())?;
        return Ok(Box::new(tcp));
    }

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let seen = Arc::new(Mutex::new(None));
    let verifier = FingerprintVerifier {
        pin: cfg.tls_fingerprint,
        seen: seen.clone(),
        provider: provider.clone(),
    };
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(cfg.host.clone())
        .map_err(|e| format!("invalid TLS server name {}: {e}", cfg.host))?;
    let mut tls =
        rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| e.to_string())?;
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(CONNECT_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    while tls.is_handshaking() {
        if started.elapsed() > HANDSHAKE_TIMEOUT {
            return Err(format!(
                "TLS handshake with {} took more than {} seconds",
                cfg.host,
                HANDSHAKE_TIMEOUT.as_secs()
            ));
        }
        tls.complete_io(&mut tcp)
            .map_err(|e| format!("TLS handshake with {} failed: {e}", cfg.host))?;
    }
    if let Some(fingerprint) = *seen.lock().unwrap() {
        if cfg.tls_fingerprint.is_some() {
            log::info!("pool certificate matches the pinned fingerprint");
        } else {
            let seen = hex::encode(fingerprint);
            match known_fingerprint(&cfg.host) {
                Some(known) if known == seen => {
                    log::info!("pool certificate is the one this version knows for {}", cfg.host)
                }
                Some(known) => {
                    for line in [
                        "THE POOL'S CERTIFICATE IS NOT THE ONE THIS VERSION KNOWS.".to_string(),
                        format!("  seen {seen}, known {known}"),
                        "  Either the pool renewed it or something between you and the pool is answering in its place,".to_string(),
                        "  and could be taking your hashrate. Check with the pool or with us before mining on.".to_string(),
                        "  Mining continues; --tls-fingerprint makes the miner refuse any other certificate.".to_string(),
                    ] {
                        log::error!("{line}");
                    }
                }
                None => log::warn!(
                    "pool certificate is not verified (fingerprint {seen}); pass --tls-fingerprint to require it"
                ),
            }
        }
    }
    tcp.set_nonblocking(true).map_err(|e| e.to_string())?;
    Ok(Box::new(rustls::StreamOwned::new(tls, tcp)))
}

/// Pools whose certificate this version has seen: the end of the host name and the
/// SHA-256 of the certificate. Used only to warn, never to refuse: a pool may renew
/// its certificate at any time, and the miner must not stop for that.
/// All seen on 2026-10-11, on every regional host each pool lists.
const KNOWN_FINGERPRINTS: &[(&str, &str)] = &[
    // qtc, qtc-eu, qtc-us, qtc-sg, qtc-ru, port 8049
    (".kryptex.network", "0fa7a4c10145c389b35d0fbcf3d9c1436d06455551a9bd43de1e3b74b5d58b16"),
    // LuckyPool: eu, us, sg, port 5660
    (".lproute.com", "e3b4a63dacb32ff7c89b70c9de488f7335ddbd35e6b69a541da0ecee3f2e534f"),
    // quantus, stratum-eu2, stratum-ord, stratum-hkg, port 7074
    (".suprnova.cc", "6502aeec80ac632364f8d12b21bf370ee6d5a31da911187e16fb8ff2d00cc948"),
    // eu, in, ca, port 1336
    (".himpool.com", "681e0a0b0ba62be779582c2d5dd19c115be20d29660cfeda59a2f4c937c765c4"),
    // eu, us, cn (.com) and ru (.org), port 5660
    (".k1pool.com", "689227ee170bb9b8c193a56bc685a944eba6431b1713938c4ed1ac4ba84907f6"),
    (".k1pool.org", "689227ee170bb9b8c193a56bc685a944eba6431b1713938c4ed1ac4ba84907f6"),
];

fn known_fingerprint(host: &str) -> Option<&'static str> {
    let host = host.to_ascii_lowercase();
    KNOWN_FINGERPRINTS
        .iter()
        .find(|(suffix, _)| host.ends_with(suffix))
        .map(|(_, fingerprint)| *fingerprint)
}

/// Buffers bytes from a transport with a short read timeout and hands back
/// whole lines.
struct LineReader {
    buffer: Vec<u8>,
}

impl LineReader {
    fn poll(&mut self, transport: &mut dyn Transport) -> Result<Vec<String>, String> {
        let mut chunk = [0u8; 4096];
        match transport.read(&mut chunk) {
            Ok(0) => return Err("the pool closed the connection".into()),
            Ok(n) => self.buffer.extend_from_slice(&chunk[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(format!("read from the pool failed: {e}")),
        }
        let mut lines = Vec::new();
        while let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let text = String::from_utf8_lossy(&line).trim().to_string();
            if !text.is_empty() {
                lines.push(text);
            }
        }
        if self.buffer.len() > MAX_LINE {
            return Err("the pool sent an oversized line".into());
        }
        Ok(lines)
    }
}

/// Writes one line to a non-blocking transport, waiting out a full send buffer.
fn send_line(transport: &mut dyn Transport, line: &str) -> Result<(), String> {
    let mut data = Vec::with_capacity(line.len() + 1);
    data.extend_from_slice(line.as_bytes());
    data.push(b'\n');
    let deadline = Instant::now() + WRITE_TIMEOUT;
    let blocked = |e: io::Error| -> Result<(), String> {
        let retry = matches!(
            e.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
        );
        if !retry || Instant::now() > deadline {
            return Err(format!("write to the pool failed: {e}"));
        }
        std::thread::sleep(Duration::from_millis(1));
        Ok(())
    };
    let mut sent = 0;
    while sent < data.len() {
        match transport.write(&data[sent..]) {
            Ok(0) => return Err("the pool closed the connection".into()),
            Ok(n) => sent += n,
            Err(e) => blocked(e)?,
        }
    }
    loop {
        match transport.flush() {
            Ok(()) => return Ok(()),
            Err(e) => blocked(e)?,
        }
    }
}

fn publish_job(shared: &Shared, dev: bool, job: PoolJob) {
    let Some(target) = qtc_pow::target_from_difficulty(job.difficulty) else {
        return;
    };
    // A pool may repeat the job it already sent; restarting the search for it
    // would throw away the shares in flight.
    if shared.work().is_some_and(|work| {
        work.dev == dev
            && work.job_id == job.job_id
            && work.header == job.header
            && work.difficulty == job.difficulty
            && work.extranonce == job.extranonce
    }) {
        return;
    }
    log::info!(
        "new job {}{} (share difficulty {})",
        if dev { "for the dev fee " } else { "" },
        job.job_id,
        job.difficulty
    );
    shared.set_work_of(
        dev,
        job.job_id,
        job.header,
        job.difficulty,
        target,
        job.extranonce,
    );
}

/// One connection, from login until it fails or a stop is requested. `Ok` means
/// stop; the flag says whether the pool ever accepted the login.
fn session(
    cfg: &PoolConfig,
    shared: &Shared,
    shares: &Receiver<Share>,
    logged_in: &mut bool,
) -> Result<(), String> {
    let mut transport = connect(cfg)?;
    let transport = transport.as_mut();
    send_line(transport, &login_line(cfg))?;
    let mut reader = LineReader { buffer: Vec::new() };
    let mut session_id: Option<String> = None;
    let mut next_id = LOGIN_ID + 1;
    // When each request was sent, and whether its share was already late.
    let mut pending: HashMap<u64, (Instant, bool)> = HashMap::new();
    let mut last_line = Instant::now();
    let mut keepalive = false;
    let mut last_sent = Instant::now();
    let connected = Instant::now();
    // The two links take turns at the GPUs. Each keeps the pool's last job, to
    // hand it over the moment its turn comes.
    let dev = cfg.dev;
    let turn = || shared.fee.active.load(Ordering::SeqCst) == dev;
    let counters = if dev {
        (&shared.fee.accepted, &shared.fee.rejected, &shared.fee.stale)
    } else {
        (&shared.accepted, &shared.rejected, &shared.stale)
    };
    let (accepted, rejected, stale) = counters;
    let mut latest: Option<PoolJob> = None;
    let mut had_turn = turn();

    loop {
        if shared.stopping() {
            return Ok(());
        }
        if dev && !shared.fee.wanted.load(Ordering::SeqCst) {
            return Ok(());
        }
        let has_turn = turn();
        if has_turn && !had_turn {
            match &latest {
                Some(job) => publish_job(shared, dev, job.clone()),
                // Nothing of this link to mine: the other link's job must not stay.
                None => shared.clear_work(),
            }
        }
        had_turn = has_turn;
        if session_id.is_none() && connected.elapsed() > LOGIN_TIMEOUT {
            return Err(format!(
                "no answer to the login in {} seconds",
                LOGIN_TIMEOUT.as_secs()
            ));
        }
        for line in reader.poll(transport)? {
            let incoming = parse_incoming(&line);
            // Only a message this miner can read counts as the pool being there.
            if incoming.is_ok() {
                last_line = Instant::now();
            }
            match incoming {
                Ok(Incoming::LoginOk {
                    session,
                    job,
                    bad_job,
                    keepalive: supported,
                }) => {
                    keepalive = supported;
                    log::info!("logged in to {} as {}", cfg.url(), cfg.login);
                    session_id = Some(session);
                    *logged_in = true;
                    if !dev {
                        shared.pool_connected.store(true, Ordering::SeqCst);
                    }
                    if let Some(job) = job {
                        latest = Some(job.clone());
                        if turn() {
                            publish_job(shared, dev, job);
                        }
                    }
                    if let Some(e) = bad_job {
                        log::error!("the job in the login reply cannot be mined ({e}): {line}");
                    }
                }
                Ok(Incoming::LoginFailed(message)) => {
                    return Err(format!("the pool refused the login: {message}"));
                }
                // Shares need the session: a job that comes before the login
                // is answered would be mined for nothing.
                Ok(Incoming::Job(_)) if session_id.is_none() => {
                    log::warn!("ignored a job that came before the login was answered");
                }
                Ok(Incoming::Job(job)) => {
                    latest = Some(job.clone());
                    if turn() {
                        publish_job(shared, dev, job);
                    }
                }
                Ok(Incoming::BadJob(e)) => {
                    // The pool has dropped the job before this one: stop
                    // rather than mine shares it will call stale.
                    log::error!("a job that cannot be mined ({e}), idle until the next: {line}");
                    latest = None;
                    if turn() {
                        shared.clear_work();
                    }
                }
                Ok(Incoming::SubmitResult { id, outcome }) => {
                    let Some((sent, late)) = pending.remove(&id) else {
                        log::debug!("pool reply to an unknown request: {line}");
                        continue;
                    };
                    let ms = format!(" ({} ms)", sent.elapsed().as_millis());
                    match outcome {
                        ShareOutcome::Accepted => {
                            let n = accepted.fetch_add(1, Ordering::Relaxed) + 1;
                            if late {
                                shared.late_accepted.fetch_add(1, Ordering::Relaxed);
                                log::info!("late share accepted{ms}, {n} so far");
                            } else {
                                log::info!("share accepted{ms}, {n} so far");
                            }
                        }
                        ShareOutcome::Stale => {
                            stale.fetch_add(1, Ordering::Relaxed);
                            log::warn!("share was stale{ms}");
                        }
                        ShareOutcome::Rejected(message) => {
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
            shared
                .fee
                .ready
                .store(session_id.is_some() && latest.is_some(), Ordering::SeqCst);
        }
        // Wait here, so a share goes out the moment a GPU finds it.
        let mut next_share = if cfg.fast_poll {
            let share = shares.try_recv().ok();
            if share.is_none() {
                std::thread::sleep(FAST_POLL_INTERVAL);
            }
            share
        } else {
            match shares.recv_timeout(POLL_INTERVAL) {
                Ok(share) => Some(share),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    std::thread::sleep(POLL_INTERVAL);
                    None
                }
            }
        };
        // A bounded pass: the pool's messages are read again in between, so a pool
        // that makes shares pour in cannot keep this loop from seeing it go quiet.
        let mut sent_now = 0;
        while let Some(share) = next_share.take() {
            sent_now += 1;
            if sent_now < SHARES_PER_PASS {
                next_share = shares.try_recv().ok();
            }
            let Some(session_id) = &session_id else {
                // Found for a job of the link before this one.
                stale.fetch_add(1, Ordering::Relaxed);
                log::info!(
                    "dropped a share for job {}: found before this login",
                    share.job_id
                );
                continue;
            };
            // The job changed while the share waited: the pool would call it stale.
            let late = share.generation != shared.generation();
            if late && !cfg.submit_late {
                stale.fetch_add(1, Ordering::Relaxed);
                log::info!("dropped a share for job {}: the job changed", share.job_id);
                continue;
            }
            if late {
                shared.late_sent.fetch_add(1, Ordering::Relaxed);
                log::info!("sending a share for job {} after the job changed", share.job_id);
            }
            let id = next_id;
            next_id += 1;
            send_line(transport, &submit_line(id, session_id, &share))?;
            pending.insert(id, (Instant::now(), late));
            last_sent = Instant::now();
        }
        if let (true, Some(session_id)) = (keepalive, &session_id) {
            if last_sent.elapsed() >= KEEPALIVE_EVERY {
                send_line(transport, &keepalive_line(next_id, session_id))?;
                next_id += 1;
                last_sent = Instant::now();
            }
        }
        pending.retain(|_, (sent, _)| sent.elapsed() < SILENCE_LIMIT);
        if pending.len() > MAX_PENDING {
            return Err(format!("the pool left {} shares unanswered", pending.len()));
        }
        if last_line.elapsed() > SILENCE_LIMIT {
            return Err("no message from the pool for 90 seconds".into());
        }
    }
}

/// Keeps a pool connection up until a stop is requested, reconnecting with a
/// growing delay.
pub fn run(cfg: PoolConfig, shared: Arc<Shared>, shares: Receiver<Share>) {
    let mut delay = Duration::from_secs(1);
    while !shared.stopping() {
        if cfg.dev && !shared.fee.wanted.load(Ordering::SeqCst) {
            // Not its time: shares that came late for it are dropped.
            while shares.try_recv().is_ok() {}
            std::thread::sleep(Duration::from_millis(100));
            delay = Duration::from_secs(1);
            continue;
        }
        let mut logged_in = false;
        let result = session(&cfg, &shared, &shares, &mut logged_in);
        if cfg.dev {
            // The schedule ends the window when the link is not ready, and
            // the other link takes the GPUs back.
            shared.fee.ready.store(false, Ordering::SeqCst);
        } else {
            shared.pool_connected.store(false, Ordering::SeqCst);
            if !shared.fee.active.load(Ordering::SeqCst) {
                shared.clear_work();
            }
        }
        let Err(e) = result else {
            continue;
        };
        if logged_in {
            delay = Duration::from_secs(1);
        }
        log::warn!(
            "{}pool connection lost: {e}; retrying in {} s",
            if cfg.dev { "dev fee " } else { "" },
            delay.as_secs()
        );
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
    use std::io::{BufRead, BufReader};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;

    const LOGIN_REPLY: &str = r#"{"id":1,"error":null,"result":{"extensions":["keepalive"],"id":"00000000-0000-4000-8000-000000000001","status":"OK","job":{"difficulty":18253611008,"extranonce":"3a37e843","job_id":"86334eee_18253611008","mining_hash":"d8227906b63edc2a5b8b0600bd8c181b01651aec3a6d834ac2be76226a4bf859","target":"000000003c3c3c3c"}}}"#;
    const JOB_PUSH: &str = r#"{"jsonrpc":"2.0","method":"job","params":{"clean_jobs":true,"job":{"difficulty":18253611008,"extranonce":"3a37e843","job_id":"f5f6dbc4_18253611008","mining_hash":"1d104e63552591f9f1415812eef65f4b4e2b645a5eb971dac21a070a24b374fb","target":"000000003c3c3c3c"}}}"#;

    fn config(port: u16) -> PoolConfig {
        PoolConfig {
            host: "127.0.0.1".into(),
            port,
            tls: false,
            tls_fingerprint: None,
            login: "qzwallet.rig".into(),
            password: "x".into(),
            agent: "DoriMiner/test".into(),
            fast_poll: false,
            submit_late: false,
            dev: false,
        }
    }

    /// A pool that logs a client in and then does what `script` says with the
    /// connection. Returns its port.
    fn scripted(
        script: impl FnOnce(std::io::Lines<BufReader<TcpStream>>, TcpStream) + Send + 'static,
    ) -> (u16, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_nodelay(true).unwrap();
            let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
            let mut out = stream;
            lines.next().unwrap().unwrap();
            writeln!(out, "{LOGIN_REPLY}").unwrap();
            script(lines, out);
        });
        (port, server)
    }

    fn wait_until(what: &str, check: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// With `submit_late` a share for a replaced job goes out, and the pool's
    /// answer to it is counted apart.
    #[test]
    fn late_shares_are_sent_when_asked() {
        let (port, server) = scripted(|mut lines, mut out| {
            writeln!(out, "{JOB_PUSH}").unwrap();
            let submit: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
            assert_eq!(submit["params"]["job_id"], "86334eee_18253611008");
            let id = submit["id"].as_u64().unwrap();
            writeln!(
                out,
                r#"{{"id":{id},"error":null,"result":{{"status":"OK"}}}}"#
            )
            .unwrap();
            // Hold the link until the client has counted the answer.
            let _ = lines.next();
        });
        let shared = Arc::new(Shared::new("test".into(), Vec::new()));
        let (share_tx, share_rx) = mpsc::channel();
        let client = {
            let shared = shared.clone();
            let cfg = PoolConfig {
                submit_late: true,
                fast_poll: true,
                ..config(port)
            };
            std::thread::spawn(move || run(cfg, shared, share_rx))
        };
        wait_until("the second job", || {
            shared
                .work()
                .is_some_and(|w| w.job_id == "f5f6dbc4_18253611008")
        });
        // A share of the first job (generation 1), found too late.
        share_tx
            .send(Share {
                generation: 1,
                job_id: "86334eee_18253611008".into(),
                nonce_be: [0x44; 64],
                hash: U512::one(),
                dev: false,
            })
            .unwrap();
        wait_until("the late accept", || {
            shared.late_accepted.load(Ordering::Relaxed) == 1
        });
        assert_eq!(shared.late_sent.load(Ordering::Relaxed), 1);
        assert_eq!(shared.accepted.load(Ordering::Relaxed), 1);
        assert_eq!(shared.stale.load(Ordering::Relaxed), 0);
        shared.request_stop();
        client.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn pool_urls() {
        assert_eq!(
            parse_pool_url("qtc.kryptex.network:7049"),
            Ok(("qtc.kryptex.network".into(), 7049, false))
        );
        assert_eq!(
            parse_pool_url("stratum+ssl://qtc-eu.kryptex.network:8049"),
            Ok(("qtc-eu.kryptex.network".into(), 8049, true))
        );
        assert_eq!(
            parse_pool_url("stratum+tcp://host:1/"),
            Ok(("host".into(), 1, false))
        );
        assert!(parse_pool_url("host").is_err());
        assert!(parse_pool_url("quic://host:1").is_err());
        assert!(parse_pool_url("host:99999").is_err());
    }

    #[test]
    fn parses_the_login_reply() {
        let Incoming::LoginOk {
            session,
            job,
            bad_job,
            keepalive,
        } = parse_incoming(LOGIN_REPLY).unwrap()
        else {
            panic!("not a login reply");
        };
        assert!(keepalive);
        assert_eq!(bad_job, None);
        assert_eq!(session, "00000000-0000-4000-8000-000000000001");
        let job = job.unwrap();
        assert_eq!(job.job_id, "86334eee_18253611008");
        assert_eq!(job.difficulty, U512::from(18_253_611_008u64));
        assert_eq!(job.extranonce, [0x3a, 0x37, 0xe8, 0x43]);
        assert_eq!(job.header[..4], [0xd8, 0x22, 0x79, 0x06]);
    }

    #[test]
    fn parses_job_pushes_and_submit_replies() {
        let Incoming::Job(job) = parse_incoming(JOB_PUSH).unwrap() else {
            panic!("not a job");
        };
        assert_eq!(job.job_id, "f5f6dbc4_18253611008");

        let reply = |line| match parse_incoming(line).unwrap() {
            Incoming::SubmitResult { id, outcome } => (id, outcome),
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(
            reply(r#"{"id":2,"error":null,"result":{"status":"OK"}}"#),
            (2, ShareOutcome::Accepted)
        );
        assert_eq!(
            reply(r#"{"id":3,"error":{"code":-1,"message":"Stale share"},"result":null}"#),
            (3, ShareOutcome::Stale)
        );
        assert_eq!(
            reply(r#"{"id":4,"error":{"code":-1,"message":"Invalid nonce"},"result":null}"#),
            (4, ShareOutcome::Rejected("Invalid nonce".into()))
        );
        assert_eq!(
            parse_incoming(
                r#"{"id":1,"error":{"code":-1,"message":"Invalid address"},"result":null}"#
            ),
            Ok(Incoming::LoginFailed("Invalid address".into()))
        );
        assert_eq!(
            parse_incoming(r#"{"id":50,"error":null,"result":{"status":"KEEPALIVED"}}"#),
            Ok(Incoming::Other)
        );
        let ping: Value = serde_json::from_str(&keepalive_line(9, "session")).unwrap();
        assert_eq!(ping["method"], "keepalived");
        assert_eq!(ping["params"]["id"], "session");
        assert_eq!(
            parse_incoming(r#"{"jsonrpc":"2.0","method":"other"}"#),
            Ok(Incoming::Other)
        );
        assert!(parse_incoming("not json").is_err());
    }

    #[test]
    fn rejects_malformed_jobs() {
        let job = |text: &str| parse_job(&serde_json::from_str(text).unwrap());
        assert!(job(r#"{"job_id":"a","mining_hash":"00","difficulty":1}"#).is_err());
        let hash = "00".repeat(32);
        assert!(job(&format!(
            r#"{{"job_id":"a","mining_hash":"{hash}","difficulty":0}}"#
        ))
        .is_err());
        assert!(job(&format!(r#"{{"mining_hash":"{hash}","difficulty":1}}"#)).is_err());
        let ok = job(&format!(r#"{{"job_id":"a","mining_hash":"{hash}","difficulty":"340282366920938463463374607431768211456"}}"#)).unwrap();
        assert_eq!(ok.difficulty, U512::one() << 128);
        assert!(ok.extranonce.is_empty());
        // The extranonce and this miner's 8 bytes share half the nonce.
        let with = |bytes: usize| {
            job(&format!(
                r#"{{"job_id":"a","mining_hash":"{hash}","difficulty":1,"extranonce":"{}"}}"#,
                "ab".repeat(bytes)
            ))
        };
        let longest = with(24).unwrap().extranonce;
        assert_eq!(longest.len(), 24);
        assert!(crate::worker::nonce_prefix(&longest, [0; 8]).is_some());
        assert!(with(25).is_err());
    }

    const FLOAT_DIFFICULTY: (&str, &str) = (r#""difficulty":18253611008"#, r#""difficulty":1.5"#);

    /// A login reply whose job cannot be read still opens the session; a job
    /// push that cannot be read is told apart from any other bad line.
    #[test]
    fn unreadable_jobs_are_told_apart() {
        let reply = LOGIN_REPLY.replace(FLOAT_DIFFICULTY.0, FLOAT_DIFFICULTY.1);
        let Incoming::LoginOk {
            session,
            job,
            bad_job,
            ..
        } = parse_incoming(&reply).unwrap()
        else {
            panic!("not a login reply");
        };
        assert_eq!(session, "00000000-0000-4000-8000-000000000001");
        assert_eq!(job, None);
        assert!(bad_job.unwrap().contains("difficulty"));

        let push = JOB_PUSH.replace(FLOAT_DIFFICULTY.0, FLOAT_DIFFICULTY.1);
        assert!(matches!(parse_incoming(&push), Ok(Incoming::BadJob(_))));
        assert!(matches!(
            parse_incoming(r#"{"jsonrpc":"2.0","method":"job","params":{}}"#),
            Ok(Incoming::BadJob(_))
        ));
    }

    /// The login reply carries a job this miner cannot read: the session is
    /// kept, the next job is mined and its share reaches the pool. Then a job
    /// push that cannot be read withdraws the work instead of leaving the
    /// cards on the job the pool has just replaced.
    #[test]
    fn a_bad_job_neither_loses_the_session_nor_leaves_dead_work() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_nodelay(true).unwrap();
            let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
            let mut out = stream;
            lines.next().unwrap().unwrap();
            let reply = LOGIN_REPLY.replace(FLOAT_DIFFICULTY.0, FLOAT_DIFFICULTY.1);
            writeln!(out, "{reply}").unwrap();
            writeln!(out, "{JOB_PUSH}").unwrap();
            let submit: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
            assert_eq!(
                submit["params"]["id"],
                "00000000-0000-4000-8000-000000000001"
            );
            assert_eq!(submit["params"]["job_id"], "f5f6dbc4_18253611008");
            let id = submit["id"].as_u64().unwrap();
            writeln!(
                out,
                r#"{{"id":{id},"error":null,"result":{{"status":"OK"}}}}"#
            )
            .unwrap();
            let bad = JOB_PUSH
                .replace("f5f6dbc4", "0badbad0")
                .replace(FLOAT_DIFFICULTY.0, FLOAT_DIFFICULTY.1);
            writeln!(out, "{bad}").unwrap();
            let _ = lines.next();
        });
        let shared = Arc::new(Shared::new("test".into(), Vec::new()));
        let (share_tx, share_rx) = mpsc::channel();
        let client = {
            let shared = shared.clone();
            std::thread::spawn(move || run(config(port), shared, share_rx))
        };
        wait_until("the job after the login", || shared.work().is_some());
        assert!(shared.pool_connected.load(Ordering::SeqCst));
        let work = shared.work().unwrap();
        assert_eq!(work.job_id, "f5f6dbc4_18253611008");
        share_tx
            .send(Share {
                generation: work.generation,
                job_id: work.job_id.clone(),
                nonce_be: [0x55; 64],
                hash: U512::one(),
                dev: false,
            })
            .unwrap();
        wait_until("the accept", || {
            shared.accepted.load(Ordering::Relaxed) == 1
        });
        wait_until("the work to be withdrawn", || shared.work().is_none());
        assert!(shared.pool_connected.load(Ordering::SeqCst));
        assert_ne!(shared.generation(), work.generation);
        shared.request_stop();
        client.join().unwrap();
        server.join().unwrap();
    }

    /// A job pushed before the login is answered is not mined, and a share
    /// that reaches a link with no session is counted, not lost.
    #[test]
    fn nothing_is_mined_or_lost_before_the_login_is_answered() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_nodelay(true).unwrap();
            let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
            let mut out = stream;
            lines.next().unwrap().unwrap();
            writeln!(out, "{JOB_PUSH}").unwrap();
            go_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            writeln!(out, "{LOGIN_REPLY}").unwrap();
            let _ = lines.next();
        });
        let shared = Arc::new(Shared::new("test".into(), Vec::new()));
        let (share_tx, share_rx) = mpsc::channel();
        let client = {
            let shared = shared.clone();
            std::thread::spawn(move || run(config(port), shared, share_rx))
        };
        share_tx
            .send(Share {
                generation: 0,
                job_id: "old".into(),
                nonce_be: [0x66; 64],
                hash: U512::one(),
                dev: false,
            })
            .unwrap();
        wait_until("the share to be counted", || {
            shared.stale.load(Ordering::Relaxed) == 1
        });
        assert!(shared.work().is_none());
        go_tx.send(()).unwrap();
        wait_until("the job of the login reply", || shared.work().is_some());
        assert_eq!(shared.work().unwrap().job_id, "86334eee_18253611008");
        shared.request_stop();
        client.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn submit_line_carries_session_nonce_and_digest() {
        let share = Share {
            generation: 1,
            job_id: "f5f6dbc4_18253611008".into(),
            nonce_be: [0xab; 64],
            hash: U512::from(0x1234u64),
            dev: false,
        };
        let value: Value = serde_json::from_str(&submit_line(7, "session", &share)).unwrap();
        assert_eq!(value["id"], 7);
        assert_eq!(value["method"], "submit");
        assert_eq!(value["params"]["id"], "session");
        assert_eq!(value["params"]["job_id"], "f5f6dbc4_18253611008");
        assert_eq!(value["params"]["nonce"], "ab".repeat(64));
        let result = value["params"]["result"].as_str().unwrap();
        assert_eq!(result.len(), 128);
        assert!(result.ends_with("1234"));
    }

    /// The dev fee link and the user's take turns: each mines its own jobs and
    /// gets its own shares, and the user's link stays up through the window.
    #[test]
    fn the_two_links_take_turns() {
        // A pool that logs one client in with the job `job`, answers one share and waits.
        fn pool(job: &'static str) -> (u16, std::thread::JoinHandle<String>) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                stream.set_nodelay(true).unwrap();
                let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
                let mut out = stream;
                let login = lines.next().unwrap().unwrap();
                writeln!(out, "{}", LOGIN_REPLY.replace("86334eee", job)).unwrap();
                let submit: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                assert!(submit["params"]["job_id"].as_str().unwrap().starts_with(job));
                let id = submit["id"].as_u64().unwrap();
                writeln!(out, r#"{{"id":{id},"error":null,"result":{{"status":"OK"}}}}"#).unwrap();
                let _ = lines.next();
                login
            });
            (port, server)
        }
        let (user_port, user_pool) = pool("86334eee");
        let (dev_port, dev_pool) = pool("dev00000");
        let shared = Arc::new(Shared::new("test".into(), Vec::new()));
        let (user_tx, user_rx) = mpsc::channel();
        let (dev_tx, dev_rx) = mpsc::channel();
        let user = {
            let shared = shared.clone();
            std::thread::spawn(move || run(config(user_port), shared, user_rx))
        };
        let dev = {
            let shared = shared.clone();
            let cfg = PoolConfig {
                login: "devwallet".into(),
                dev: true,
                ..config(dev_port)
            };
            std::thread::spawn(move || run(cfg, shared, dev_rx))
        };
        let fee = &shared.fee;
        let share = |work: &crate::state::Work| Share {
            generation: work.generation,
            job_id: work.job_id.clone(),
            nonce_be: [0x55; 64],
            hash: U512::one(),
            dev: work.dev,
        };
        wait_until("the user's job", || shared.work().is_some());
        let first = shared.work().unwrap();
        assert!(!first.dev && first.job_id.starts_with("86334eee"));
        assert!(!fee.ready.load(Ordering::SeqCst), "the dev fee link waits to be asked for");

        // The link comes up; the GPUs are still the user's.
        fee.wanted.store(true, Ordering::SeqCst);
        wait_until("the dev fee link", || fee.ready.load(Ordering::SeqCst));
        assert_eq!(shared.generation(), first.generation);

        // The window: the dev fee job, its share to its link, its own count.
        fee.active.store(true, Ordering::SeqCst);
        wait_until("the dev fee job", || shared.work().is_some_and(|work| work.dev));
        let window = shared.work().unwrap();
        assert!(window.job_id.starts_with("dev00000"));
        dev_tx.send(share(&window)).unwrap();
        wait_until("the dev fee share", || fee.accepted.load(Ordering::Relaxed) == 1);
        assert_eq!(shared.accepted.load(Ordering::Relaxed), 0);
        assert!(shared.pool_connected.load(Ordering::SeqCst));

        // The end: the user's job is back without a new login, the dev fee link goes away.
        fee.active.store(false, Ordering::SeqCst);
        fee.wanted.store(false, Ordering::SeqCst);
        wait_until("the user's job again", || shared.work().is_some_and(|work| !work.dev));
        let back = shared.work().unwrap();
        assert!(back.job_id.starts_with("86334eee"));
        user_tx.send(share(&back)).unwrap();
        wait_until("the user's share", || shared.accepted.load(Ordering::Relaxed) == 1);
        assert_eq!(fee.accepted.load(Ordering::Relaxed), 1);
        wait_until("the dev fee link to close", || !fee.ready.load(Ordering::SeqCst));
        assert!(dev_pool.join().unwrap().contains("devwallet"));

        shared.request_stop();
        user.join().unwrap();
        dev.join().unwrap();
        assert!(user_pool.join().unwrap().contains("qzwallet.rig"));
    }

    #[test]
    fn full_session_against_a_scripted_pool() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen_tx, seen_rx) = mpsc::channel::<String>();

        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
            let mut out = stream;
            seen_tx.send(lines.next().unwrap().unwrap()).unwrap();
            writeln!(out, "{LOGIN_REPLY}").unwrap();
            let submit = lines.next().unwrap().unwrap();
            let id = serde_json::from_str::<Value>(&submit).unwrap()["id"]
                .as_u64()
                .unwrap();
            seen_tx.send(submit).unwrap();
            writeln!(
                out,
                r#"{{"id":{id},"error":null,"result":{{"status":"OK"}}}}"#
            )
            .unwrap();
            writeln!(out, "{JOB_PUSH}").unwrap();
            // Wait for the client to confirm it saw the new job, then drop the link.
            seen_tx.send(lines.next().unwrap().unwrap()).unwrap();
            drop(out);
            drop(lines);
            let (stream, _) = listener.accept().unwrap();
            let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
            seen_tx.send(lines.next().unwrap().unwrap()).unwrap();
        });

        let shared = Arc::new(Shared::new("test".into(), Vec::new()));
        let (share_tx, share_rx) = mpsc::channel();
        let client = {
            let shared = shared.clone();
            std::thread::spawn(move || run(config(port), shared, share_rx))
        };
        let wait = |what: &str, check: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !check() {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let next = || seen_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let login: Value = serde_json::from_str(&next()).unwrap();
        assert_eq!(login["method"], "login");
        assert_eq!(login["params"]["login"], "qzwallet.rig");
        assert_eq!(login["params"]["agent"], "DoriMiner/test");

        wait("the first job", &|| shared.work().is_some());
        let work = shared.work().unwrap();
        assert!(shared.pool_connected.load(Ordering::SeqCst));
        assert_eq!(work.job_id, "86334eee_18253611008");
        assert_eq!(work.target, U512::MAX / U512::from(18_253_611_008u64));
        let share = |generation, job_id: &str, fill| Share {
            generation,
            job_id: job_id.into(),
            nonce_be: [fill; 64],
            hash: U512::one(),
            dev: false,
        };
        share_tx
            .send(share(work.generation, &work.job_id, 0x11))
            .unwrap();
        let submit: Value = serde_json::from_str(&next()).unwrap();
        assert_eq!(
            submit["params"]["id"],
            "00000000-0000-4000-8000-000000000001"
        );
        assert_eq!(submit["params"]["nonce"], "11".repeat(64));
        wait("the accept", &|| {
            shared.accepted.load(Ordering::Relaxed) == 1
        });

        wait("the second job", &|| {
            shared
                .work()
                .is_some_and(|w| w.job_id == "f5f6dbc4_18253611008")
        });
        // A share for the old job is dropped locally; one for the new job goes out.
        share_tx
            .send(share(work.generation, &work.job_id, 0x22))
            .unwrap();
        let current = shared.work().unwrap();
        share_tx
            .send(share(current.generation, &current.job_id, 0x33))
            .unwrap();
        let submit: Value = serde_json::from_str(&next()).unwrap();
        assert_eq!(submit["params"]["nonce"], "33".repeat(64));
        assert_eq!(submit["params"]["job_id"], "f5f6dbc4_18253611008");
        assert_eq!(shared.stale.load(Ordering::Relaxed), 1);

        // The pool drops the link: work is withdrawn and the client logs in again.
        wait("work to be withdrawn", &|| shared.work().is_none());
        assert!(!shared.pool_connected.load(Ordering::SeqCst));
        let relogin: Value = serde_json::from_str(&next()).unwrap();
        assert_eq!(relogin["method"], "login");

        shared.request_stop();
        client.join().unwrap();
        server.join().unwrap();
    }
}
