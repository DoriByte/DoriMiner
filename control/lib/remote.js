const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { execFile } = require('node:child_process');
const attestation = require('./attestation');
const config = require('./config');
const { COINS } = require('./coins');

const DEFAULT_PORT = 4782;
const DEFAULT_HTTPS_PORT = 8443;
const PAIR_UNUSED_MS = 15 * 60_000;
const DEVICE_IDLE_MS = 30 * 24 * 3_600_000;
const MAX_DEVICES = 20;
const MAX_FAILURES = 10;
const FAILURE_WINDOW_MS = 15 * 60_000;
const TOUCH_EVERY_MS = 60_000;
const MAX_BODY = 4096;
const MAX_ATTEST_BODY = 256 * 1024;
const MAX_CONFIG_BODY = 256 * 1024;
const ATTEST_CHALLENGE_MS = 2 * 60_000;
const ID = /^[a-z0-9-]{1,40}$/;

const BAD_NAME = /[\p{Cc}\p{Cf}]/u;

const TS_NET = /^[a-z0-9-]+(\.[a-z0-9-]+)*\.ts\.net$/;

const HEADERS = {
  'Cache-Control': 'no-store',
  'X-Content-Type-Options': 'nosniff',
  'Referrer-Policy': 'no-referrer',
  'Content-Security-Policy': "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
  'Strict-Transport-Security': 'max-age=31536000',
};

function hostnameOf(host, httpsPort = DEFAULT_HTTPS_PORT) {
  if (typeof host !== 'string') return null;
  const m = /^([a-z0-9.-]+)(?::(\d{1,5}))?$/i.exec(host.trim());
  if (!m) return null;
  if (m[2] !== undefined && m[2] !== String(httpsPort) && m[2] !== '443') return null;
  return m[1].toLowerCase();
}

const hostAllowed = (host, httpsPort) => {
  const name = hostnameOf(host, httpsPort);
  return name !== null && TS_NET.test(name);
};

function originAllowed(origin, host, httpsPort = DEFAULT_HTTPS_PORT) {
  const name = hostnameOf(host, httpsPort);
  if (name === null || typeof origin !== 'string') return false;
  const o = origin.trim().toLowerCase();
  return o === `https://${name}` || o === `https://${name}:${httpsPort}` || o === `https://${name}:443`;
}

class FailureLimiter {
  constructor(max = MAX_FAILURES, windowMs = FAILURE_WINDOW_MS, now = Date.now) {
    this.max = max;
    this.windowMs = windowMs;
    this.now = now;
    this.failures = new Map();
  }

  recent(key) {
    const cutoff = this.now() - this.windowMs;
    const kept = (this.failures.get(key) || []).filter((t) => t > cutoff);
    if (kept.length === 0) this.failures.delete(key);
    else this.failures.set(key, kept);
    return kept;
  }

  blocked(key) {
    return this.recent(key).length >= this.max;
  }

  fail(key) {
    this.failures.set(key, [...this.recent(key), this.now()]);
  }
}

const redactWallet = (w) => (typeof w !== 'string' ? '' : w.length <= 12 ? `${w.slice(0, 3)}…` : `${w.slice(0, 6)}…${w.slice(-4)}`);

const scrubPaths = (text) =>
  String(text)
    .replace(/[A-Za-z]:\\[^\s"'<>|:]*[^\s"'<>|:.,]/g, '<path>')
    .replace(/(?<![\w.:/])\/(?:[^\s"'/<>|:]+\/)+[^\s"'/<>|:]*[^\s"'/<>|:.,]?/g, '<path>');

const scrubDeep = (value) => {
  if (typeof value === 'string') return scrubPaths(value);
  if (Array.isArray(value)) return value.map(scrubDeep);
  if (value === null || typeof value !== 'object') return value;
  return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, scrubDeep(v)]));
};

const sha256Hex = (text) => crypto.createHash('sha256').update(text, 'utf8').digest('hex');

function bearerOf(header) {
  if (typeof header !== 'string') return null;
  const m = /^Bearer ([0-9a-f]{64})$/.exec(header);
  return m ? m[1] : null;
}

function validName(input) {
  if (typeof input !== 'string') return null;
  const name = input.trim().replace(/\s+/g, ' ');
  if (name === '' || name.length > 40 || /[\p{Cc}\p{Cf}]/u.test(name)) return null;
  return name;
}

const idleExpired = (d, now) => d.lastUsedAt !== null && now - Date.parse(d.lastUsedAt) > DEVICE_IDLE_MS;
const unusedExpired = (d, now) => d.lastUsedAt === null && now - Date.parse(d.createdAt) > PAIR_UNUSED_MS;

const deviceView = (d, now) => ({
  id: d.id,
  name: d.name,
  createdAt: d.createdAt,
  lastUsedAt: d.lastUsedAt,
  tailscaleLogin: d.tailscaleLogin,
  expired: idleExpired(d, now) || unusedExpired(d, now),
  attested: d.attestedKey ? { at: d.attestedAt, level: d.attestedLevel } : null,
});

class DeviceStore {
  constructor(file, now = Date.now) {
    this.file = file;
    this.now = now;
    this.devices = this.load();
    this.touched = new Map();
  }

  load() {
    try {
      const raw = JSON.parse(fs.readFileSync(this.file, 'utf8'));
      if (raw.version !== 1 || !Array.isArray(raw.devices)) return [];
      return raw.devices.filter((d) => d && typeof d.id === 'string' && typeof d.name === 'string' && /^[0-9a-f]{64}$/.test(d.tokenSha256) && typeof d.createdAt === 'string').map((d) => ({
        id: d.id,
        name: d.name,
        tokenSha256: d.tokenSha256,
        createdAt: d.createdAt,
        lastUsedAt: typeof d.lastUsedAt === 'string' ? d.lastUsedAt : null,
        tailscaleLogin: typeof d.tailscaleLogin === 'string' ? d.tailscaleLogin : null,
        ...(typeof d.attestedKey === 'string' && d.attestedKey.includes('BEGIN PUBLIC KEY') && typeof d.attestedAt === 'string'
          ? { attestedKey: d.attestedKey, attestedAt: d.attestedAt, attestedLevel: d.attestedLevel === 'strongbox' ? 'strongbox' : 'tee' }
          : {}),
      }));
    } catch {
      return [];
    }
  }

  save() {
    fs.mkdirSync(path.dirname(this.file), { recursive: true });
    const tmp = `${this.file}.tmp`;
    fs.writeFileSync(tmp, `${JSON.stringify({ version: 1, devices: this.devices }, null, 2)}\n`, { mode: 0o600 });
    fs.renameSync(tmp, this.file);
  }

  list() {
    return this.devices.map((d) => deviceView(d, this.now()));
  }

  pair(nameInput) {
    const name = validName(nameInput);
    if (name === null) return { ok: false, code: 'badDeviceName' };
    if (this.devices.length >= MAX_DEVICES) return { ok: false, code: 'tooManyDevices' };
    const token = crypto.randomBytes(32).toString('hex');
    const device = { id: crypto.randomBytes(6).toString('hex'), name, tokenSha256: sha256Hex(token), createdAt: new Date(this.now()).toISOString(), lastUsedAt: null, tailscaleLogin: null };
    this.devices = [...this.devices, device];
    this.save();
    return { ok: true, device: deviceView(device, this.now()), token };
  }

  revoke(id) {
    const before = this.devices.length;
    this.devices = this.devices.filter((d) => d.id !== id);
    this.touched.delete(id);
    if (this.devices.length === before) return false;
    this.save();
    return true;
  }

  authenticate(header) {
    const token = bearerOf(header);
    if (token === null) return null;
    const given = Buffer.from(sha256Hex(token), 'hex');
    let found = null;
    for (const d of this.devices) {
      const wanted = Buffer.from(d.tokenSha256, 'hex');
      if (wanted.length === given.length && crypto.timingSafeEqual(given, wanted)) found = d;
    }
    if (found === null) return null;
    const now = this.now();
    if (unusedExpired(found, now) || idleExpired(found, now)) return null;
    return found;
  }

  setAttested(id, publicKeyPem, level) {
    const d = this.devices.find((x) => x.id === id);
    if (!d) return null;
    d.attestedKey = publicKeyPem;
    d.attestedAt = new Date(this.now()).toISOString();
    d.attestedLevel = level;
    this.save();
    return { at: d.attestedAt, level };
  }

  bindLogin(id, login) {
    const d = this.devices.find((x) => x.id === id);
    if (!d) return 'mismatch';
    if (d.tailscaleLogin === login) return 'ok';
    if (d.tailscaleLogin !== null) return 'mismatch';
    d.tailscaleLogin = login;
    this.save();
    return 'bound';
  }

  touch(id) {
    const d = this.devices.find((x) => x.id === id);
    if (!d) return;
    const now = this.now();
    d.lastUsedAt = new Date(now).toISOString();
    const written = this.touched.get(id);
    if (written !== undefined && now - written < TOUCH_EVERY_MS) return;
    this.touched.set(id, now);
    try {
      this.save();
    } catch {

    }
  }
}

function manageView(cfg) {
  const pools = {};
  for (const coin of Object.values(COINS)) pools[coin.id] = coin.pools.map((p) => ({ ref: `preset:${p.id}`, name: p.name }));
  for (const p of cfg.pools) if (pools[p.coin]) pools[p.coin].push({ ref: `pool:${p.id}`, name: p.name });
  return {
    canManage: true,
    wallets: cfg.wallets.map((w) => ({ id: w.id, coin: w.coin, name: w.name, address: w.address })),
    coins: Object.values(COINS).map((c) => c.id),
    pools,
  };
}

const firstError = (errors) => Object.values(errors)[0] ?? null;
const text = (v) => (typeof v === 'string' ? v.trim() : '');

function applyManage(cfg, what, body) {
  const next = JSON.parse(JSON.stringify(cfg));
  const id = body.id === undefined || body.id === null ? null : body.id;
  if (id !== null && (typeof id !== 'string' || !ID.test(id))) return { ok: false, code: 'badId' };
  if (what === 'wallet') {
    if (BAD_NAME.test(text(body.name))) return { ok: false, code: 'badName' };
    const fields = { coin: body.coin, name: text(body.name), address: text(body.address) };
    if (id === null) next.wallets.push({ id: config.newId('w', next.wallets), ...fields });
    else {
      const w = next.wallets.find((x) => x.id === id);
      if (!w) return { ok: false, code: 'unknownWallet' };
      Object.assign(w, fields);
    }
  } else if (what === 'wallet-delete') {
    if (!next.wallets.some((x) => x.id === id)) return { ok: false, code: 'unknownWallet' };
    if (next.profiles.some((p) => p.walletId === id)) return { ok: false, code: 'walletInUse' };
    next.wallets = next.wallets.filter((x) => x.id !== id);
  } else if (what === 'profile') {
    let p;
    if (id === null) {
      if (typeof body.coin !== 'string' || !Object.hasOwn(COINS, body.coin)) return { ok: false, code: 'unknownCoin' };
      p = config.defaultProfile(config.newId('p', next.profiles), body.coin);
      next.profiles.push(p);
    } else {
      p = next.profiles.find((x) => x.id === id);
      if (!p) return { ok: false, code: 'unknownProfile' };
    }
    if (BAD_NAME.test(text(body.name))) return { ok: false, code: 'badName' };
    p.name = text(body.name);
    p.worker = text(body.worker);

    if (body.walletId !== undefined) {
      if (body.walletId !== null && typeof body.walletId !== 'string') return { ok: false, code: 'unknownWallet' };
      if (body.walletId === null && id !== null && (next.activeProfile === id || scheduled(next, id))) return { ok: false, code: 'profileActive' };
      p.walletId = body.walletId;
    }
    if (body.pools !== undefined) {
      if (!Array.isArray(body.pools)) return { ok: false, code: 'badPools' };
      p.pools = body.pools;
    }
  } else if (what === 'profile-delete') {
    if (!next.profiles.some((x) => x.id === id)) return { ok: false, code: 'unknownProfile' };
    if (next.activeProfile === id) return { ok: false, code: 'profileActive' };
    if (scheduled(next, id)) return { ok: false, code: 'profileInSchedule' };
    next.profiles = next.profiles.filter((x) => x.id !== id);
  } else return { ok: false, code: 'unknown' };
  const checked = config.normalize(next);
  const code = firstError(config.validate(checked)) ?? firstError(config.newAddressProblem(cfg, checked) ?? {});
  if (code !== null) return { ok: false, code };
  const made = what === 'wallet' && id === null ? checked.wallets[checked.wallets.length - 1].id : what === 'profile' && id === null ? checked.profiles[checked.profiles.length - 1].id : null;
  return { ok: true, next: checked, newId: made };
}

function configRev(cfg) {
  return crypto.createHash('sha256').update(JSON.stringify([cfg.wallets, cfg.pools, cfg.profiles, cfg.activeProfile, cfg.currency, cfg.machineName, cfg.auto.kwhEur])).digest('hex').slice(0, 16);
}

function applyPageSave(cfg, next) {
  if (next === null || typeof next !== 'object' || !Array.isArray(next.wallets) || !Array.isArray(next.profiles) || !Array.isArray(next.pools)) return { ok: false, code: 'body' };
  const out = JSON.parse(JSON.stringify(cfg));
  const isText = (v) => typeof v === 'string';

  if (next.rev !== undefined && next.rev !== configRev(cfg)) return { ok: false, code: 'stale' };
  if (!next.profiles.some((p) => p?.id === cfg.activeProfile)) return { ok: false, code: 'profileActive' };
  const badName = next.wallets.concat(next.pools, next.profiles).some((x) => !x || BAD_NAME.test(text(x.name)));
  if (badName) return { ok: false, code: 'badName' };
  out.wallets = next.wallets.map((w) => ({ id: w.id, coin: w.coin, name: w.name, address: isText(w.address) ? w.address.trim() : '' }));
  out.pools = next.pools.map((p) => ({ id: p.id, coin: p.coin, name: p.name, url: p.url }));
  out.profiles = next.profiles.map((p) => {
    const was = cfg.profiles.find((x) => x.id === p?.id);
    if (was) return { ...JSON.parse(JSON.stringify(was)), name: p.name, walletId: p.walletId ?? null, worker: p.worker, pools: p.pools };
    if (!isText(p?.coin) || !Object.hasOwn(COINS, p.coin)) return { id: p?.id, coin: p?.coin, name: p?.name };
    return { ...config.defaultProfile(p.id, p.coin), name: p.name, walletId: p.walletId ?? null, worker: p.worker, pools: p.pools };
  });
  if (isText(next.activeProfile)) out.activeProfile = next.activeProfile;
  if (next.auto && Object.hasOwn(next.auto, 'kwhEur')) out.auto.kwhEur = next.auto.kwhEur;
  if (next.currency === 'eur' || next.currency === 'usd') out.currency = next.currency;
  if (typeof next.machineName === 'string') out.machineName = next.machineName.trim();

  for (const was of cfg.profiles) {
    if (!out.profiles.some((p) => p.id === was.id) && scheduled(cfg, was.id)) return { ok: false, code: 'profileInSchedule' };
  }
  const active = out.profiles.find((p) => p.id === out.activeProfile);
  if (active && active.walletId === null && cfg.profiles.find((p) => p.id === active.id)?.walletId) return { ok: false, code: 'profileActive' };
  let checked;
  try {
    checked = config.normalize(out);
  } catch {
    return { ok: false, code: 'body' };
  }
  const errors = { ...config.validate(checked), ...(config.newAddressProblem(cfg, checked) ?? {}) };
  return Object.keys(errors).length > 0 ? { ok: false, errors } : { ok: true, next: checked };
}

function redactWallets(value, cfg) {
  let text2 = JSON.stringify(value);
  for (const w of cfg.wallets) if (typeof w.address === 'string' && w.address.length > 12) text2 = text2.split(JSON.stringify(w.address).slice(1, -1)).join(redactWallet(w.address));
  return JSON.parse(text2);
}

function pageConfig(cfg, manage) {
  const copy = JSON.parse(JSON.stringify(cfg));
  if (manage) return { ...copy, minerPath: '', rev: configRev(cfg) };

  const base = config.defaults();
  return redactWallets({
    ...base,
    machineName: copy.machineName,
    currency: copy.currency,
    activeProfile: copy.activeProfile,
    auto: { ...base.auto, kwhEur: copy.auto.kwhEur },
    wallets: copy.wallets,
    pools: copy.pools.map((p) => ({ id: p.id, coin: p.coin, name: p.name, url: '' })),
    profiles: copy.profiles.map((p) => ({ ...config.defaultProfile(p.id, p.coin), name: p.name, walletId: p.walletId, pools: p.pools, worker: '' })),
  }, cfg);
}

function pageStatus(status, cfg, manage) {
  const out = {
    ...status,
    pid: null,
    minerPath: '',
    log: [],
    admin: true,
    cardsChanged: false,
    showWindow: false,
    note: status.note ? scrubPaths(status.note) : null,
    lastExit: status.lastExit ? { ...status.lastExit, message: status.lastExit.message ? scrubPaths(status.lastExit.message) : status.lastExit.message } : status.lastExit,
  };
  const safe = scrubDeep(out);
  if (manage) return safe;
  delete safe.systemName;
  if (typeof safe.note === 'string') safe.note = safe.note.replace(/\b[a-z][a-z0-9+.-]*:\/\/\S+/gi, '<pool>');
  if (safe.pool) safe.pool = { ...safe.pool, url: '' };
  if (safe.summary?.pool) safe.summary = { ...safe.summary, pool: { ...safe.summary.pool, url: '' } };
  return redactWallets(safe, cfg);
}

const scheduled = (c, id) => c.schedule.slots.some((s) => s.profileId === id) || c.auto.profiles.includes(id);

function changesMining(before, after, profileId, poolIndex) {
  const args = (c) => {
    const p = c.profiles.find((x) => x.id === profileId);
    return p ? JSON.stringify(config.minerArgs(c, p, poolIndex)) : null;
  };
  return args(before) !== args(after);
}

function describeChange(c, what, body) {
  if (what.startsWith('wallet')) {
    const w = c.wallets.find((x) => x.id === body.id);
    return w ? { id: w.id, coin: w.coin, name: w.name, address: w.address } : null;
  }
  const p = c.profiles.find((x) => x.id === body.id);
  if (!p) return null;
  const w = c.wallets.find((x) => x.id === p.walletId);
  return { id: p.id, name: p.name, worker: p.worker, pools: p.pools, walletId: p.walletId, address: w ? w.address : null };
}

function remoteState(status, cfg, machine, manage = false) {
  const sum = status.summary;
  const profiles = cfg.profiles.map((p) => {
    const w = cfg.wallets.find((x) => x.id === p.walletId);
    return { id: p.id, name: p.name, coin: p.coin, wallet: w ? redactWallet(w.address) : null, ...(manage ? { walletId: p.walletId, worker: p.worker, pools: p.pools } : {}) };
  });
  return {
    control: true,
    machine,
    state: status.state,
    armed: status.armed,
    profileId: status.profileId,
    activeProfile: cfg.activeProfile,
    scheduled: cfg.schedule.enabled,
    profiles,
    note: status.note ? scrubPaths(status.note) : null,
    pool: status.pool ? { name: status.pool.name, index: status.pool.index, of: status.pool.of } : null,
    summary: sum
      ? {
          hashrate: sum.hashrate ?? 0,
          algo: sum.algo ?? null,
          accepted: sum.accepted_shares ?? 0,
          rejected: sum.invalid_shares ?? 0,
          stale: sum.stale_shares ?? 0,
          uptime: sum.uptime ?? 0,
          connected: sum.pool?.connected === true,
          version: sum.version ?? null,
        }
      : null,
    gpus: (status.gpus || []).map((g) => ({
      index: g.index,
      name: g.name,
      coreC: g.coreC ?? null,
      memC: g.memC ?? null,
      watts: g.watts ?? null,
      fan: g.fanPct ?? null,
      clock: g.clockMHz ?? null,

      notices: (sum?.gpus ?? []).find((m) => m.id === g.index)?.notices ?? [],
      usedBy: g.usedBy ?? null,
    })),
    oc: status.oc?.code ?? null,
    limits: status.limits,

    earn: status.earn ? { net: status.earn.net, coinsPerDay: status.earn.coinsPerDay, coin: status.earn.coin } : null,
    ...(manage ? manageView(cfg) : {}),
  };
}

const STATIC = {
  '/app.js': [path.join('..', 'renderer', 'app.js'), 'text/javascript; charset=utf-8'],
  '/style.css': [path.join('..', 'renderer', 'style.css'), 'text/css; charset=utf-8'],
  '/remote.js': ['remote.js', 'text/javascript; charset=utf-8'],
  '/remote.css': ['remote.css', 'text/css; charset=utf-8'],
  '/icon.png': [path.join('..', 'assets', 'icon-192.png'), 'image/png'],
  '/coins/prl.png': [path.join('coins', 'prl.png'), 'image/png'],
  '/coins/qtc.png': [path.join('coins', 'qtc.png'), 'image/png'],
  '/coins/erg.jpg': [path.join('coins', 'erg.jpg'), 'image/jpeg'],
  '/coins/vela.svg': [path.join('coins', 'vela.svg'), 'image/svg+xml'],
};

function send(res, code, body) {
  res.writeHead(code, { ...HEADERS, 'Content-Type': 'application/json; charset=utf-8' });
  res.end(JSON.stringify(body));
}

function readRaw(req, max) {
  return new Promise((resolve) => {
    let size = 0;
    const chunks = [];
    req.on('data', (c) => {
      size += c.length;
      if (size > max) {
        req.removeAllListeners('data');
        resolve(null);
      } else chunks.push(c);
    });
    req.on('end', () => resolve(Buffer.concat(chunks)));
    req.on('error', () => resolve(null));
  });
}

function parseBody(raw) {
  if (raw === null) return null;
  try {
    const value = raw.length ? JSON.parse(raw.toString('utf8')) : {};
    return value !== null && typeof value === 'object' && !Array.isArray(value) ? value : null;
  } catch {
    return null;
  }
}

function phonePage(staticDir) {
  return fs.readFileSync(path.join(staticDir, '..', 'renderer', 'index.html'), 'utf8')
    .replace('<html', '<html class="phone"')
    .replace('../assets/icon-96.png', 'icon.png')
    .replace('<link rel="stylesheet" href="style.css" />', '<link rel="stylesheet" href="style.css" /><link rel="stylesheet" href="remote.css" />')
    .replace('<script src="app.js"></script>', '<script src="remote.js"></script><script src="app.js"></script>');
}

function handler(o) {
  const requireIdentity = o.requireIdentity !== false;
  const nonces = new attestation.NonceCache();
  const challenges = new Map();
  return async (req, res) => {
    try {
      if (!hostAllowed(req.headers.host, o.httpsPort)) return send(res, 421, { error: 'host' });
      const login = typeof req.headers['tailscale-user-login'] === 'string' && req.headers['tailscale-user-login'] !== '' ? req.headers['tailscale-user-login'] : null;

      if (requireIdentity && login === null) return send(res, 403, { error: 'tailnetOnly' });
      const url = new URL(req.url || '/', 'http://x');
      const method = req.method || 'GET';

      if (method === 'GET' && (url.pathname === '/' || url.pathname === '/index.html')) {
        let page;
        try {
          page = phonePage(o.staticDir);
        } catch {
          return send(res, 404, { error: 'notFound' });
        }
        res.writeHead(200, { ...HEADERS, 'Content-Type': 'text/html; charset=utf-8' });
        return res.end(page);
      }
      if (method === 'GET' && STATIC[url.pathname]) {
        const [file, type] = STATIC[url.pathname];
        let data;
        try {
          data = fs.readFileSync(path.join(o.staticDir, file));
        } catch {
          return send(res, 404, { error: 'notFound' });
        }
        res.writeHead(200, { ...HEADERS, 'Content-Type': type });
        return res.end(data);
      }
      if (!url.pathname.startsWith('/api/')) return send(res, 404, { error: 'notFound' });

      const key = req.socket.remoteAddress || 'unknown';
      const device = o.devices.authenticate(req.headers.authorization);
      if (device === null) {
        if (o.limiter.blocked(key)) return send(res, 429, { error: 'tooManyTries' });
        o.limiter.fail(key);
        o.onDenied?.(bearerOf(req.headers.authorization) === null ? 'none' : 'unknown', login);
        return send(res, 401, { error: 'notPaired' });
      }
      if (login !== null && o.devices.bindLogin(device.id, login) === 'mismatch') return send(res, 403, { error: 'otherAccount' });
      o.devices.touch(device.id);

      if (method === 'GET' && url.pathname === '/api/state') return send(res, 200, { ...o.state(device.attestedKey !== undefined), device: { attested: device.attestedKey !== undefined } });
      if (o.page && method === 'GET' && url.pathname === '/api/app/init') return send(res, 200, { ...o.page.init(device.attestedKey !== undefined), device: { attested: device.attestedKey !== undefined } });
      if (o.page && method === 'GET' && url.pathname === '/api/app/status') return send(res, 200, { ...o.page.status(device.attestedKey !== undefined), device: { attested: device.attestedKey !== undefined } });
      if (method !== 'POST') return send(res, 404, { error: 'notFound' });
      if (!originAllowed(req.headers.origin, req.headers.host, o.httpsPort)) return send(res, 403, { error: 'origin' });
      if (!/^application\/json\b/i.test(req.headers['content-type'] || '')) return send(res, 415, { error: 'json' });
      const raw = await readRaw(req, url.pathname === '/api/attest' ? MAX_ATTEST_BODY : url.pathname === '/api/manage/config' ? MAX_CONFIG_BODY : MAX_BODY);
      if (raw === null && req.readable) {

        req.resume();
        res.once('finish', () => setTimeout(() => req.destroy(), 2000).unref());
        return send(res, 413, { error: 'tooBig' });
      }
      const body = parseBody(raw);
      if (body === null) return send(res, 400, { error: 'body' });

      let signed = false;
      if (device.attestedKey !== undefined && req.headers['x-dori-signature'] !== undefined) {
        const why = attestation.verifyRequestSignature(device.attestedKey, { method, pathAndQuery: req.url, timestamp: req.headers['x-dori-timestamp'], nonce: req.headers['x-dori-nonce'], signature: req.headers['x-dori-signature'], body: raw, deviceId: device.id }, nonces);
        if (why !== null) return send(res, 401, { error: why });
        signed = true;
      }
      const view = () => o.state(device.attestedKey !== undefined);

      if (url.pathname === '/api/attest/challenge' || url.pathname === '/api/attest') {

        if (device.attestedKey !== undefined && !signed) return send(res, 403, { error: 'alreadyVerified' });
        if (url.pathname === '/api/attest/challenge') {
          const challenge = (o.attestation?.challenge ?? (() => crypto.randomBytes(32)))();
          challenges.set(device.id, { challenge, until: Date.now() + ATTEST_CHALLENGE_MS });
          return send(res, 200, { challenge: challenge.toString('base64') });
        }
        const pending = challenges.get(device.id);
        challenges.delete(device.id);
        if (pending === undefined || pending.until < Date.now()) return send(res, 400, { error: 'attestLate' });
        try {
          const key = attestation.verifyAttestation(body.chain, {
            challenge: pending.challenge,
            signingDigests: [...(o.attestation?.signingDigests ?? attestation.RELEASE_SIGNING_DIGESTS), ...attestation.devApkDigests()],
            ...(o.attestation?.rootKeys ? { rootKeys: o.attestation.rootKeys } : {}),
            revoked: await (o.attestation?.revoked ?? attestation.revokedKeys)(),
          });
          const attested = o.devices.setAttested(device.id, key.publicKeyPem, key.level);
          o.onAttested?.(device, key.level);
          return send(res, 200, { attested });
        } catch (error) {
          if (error instanceof attestation.AttestError) return send(res, 400, { error: error.code });
          throw error;
        }
      }

      const manage = /^\/api\/manage\/(wallet|wallet-delete|profile|profile-delete|config|bench-plan|bench-start|bench-cancel)$/.exec(url.pathname);
      if (manage) {
        if (device.attestedKey === undefined || !signed) return send(res, 403, { error: 'notVerified', verify: true });
        if (!o.manage) return send(res, 404, { error: 'notFound' });
        const r = await o.manage(manage[1], body, device);
        if (r.errors) return send(res, 400, { error: 'invalid', errors: r.errors });
        return send(res, r.ok ? 200 : 400, r.ok ? { ok: true, ...(r.reply ?? {}), state: { ...view(), device: { attested: true } } } : { error: r.code || 'failed' });
      }

      let r;
      if (url.pathname === '/api/start') r = await o.act('start', null, device);
      else if (url.pathname === '/api/stop') r = await o.act('stop', null, device);
      else if (url.pathname === '/api/without-oc') r = await o.act('without-oc', null, device);
      else if (url.pathname === '/api/profile') {
        if (typeof body.id !== 'string' || !/^[a-z0-9-]{1,40}$/.test(body.id)) return send(res, 400, { error: 'badProfile' });
        r = await o.act('profile', body.id, device);
      } else return send(res, 404, { error: 'notFound' });
      return send(res, r.ok ? 200 : 400, r.ok ? { ok: true, state: { ...view(), device: { attested: device.attestedKey !== undefined } } } : { error: r.code || 'failed' });
    } catch {
      if (!res.headersSent) send(res, 500, { error: 'internal' });
      else res.end();
    }
  };
}

function tailscaleBinary(platform = process.platform) {
  if (platform === 'win32') return 'C:\\Program Files\\Tailscale\\tailscale.exe';
  if (platform === 'darwin') return '/Applications/Tailscale.app/Contents/MacOS/Tailscale';
  return '/usr/bin/tailscale';
}

function parseTailscaleStatus(json) {
  const dns = json && json.Self && json.Self.DNSName;
  if (typeof dns !== 'string') return null;
  const host = dns.trim().toLowerCase().replace(/\.$/, '');
  return TS_NET.test(host) ? host : null;
}

const run = (args, timeout) =>
  new Promise((resolve) => {
    execFile(tailscaleBinary(), args, { timeout, windowsHide: true, maxBuffer: 4 * 1024 * 1024 }, (error, stdout, stderr) => {
      resolve({ ok: !error, missing: error?.code === 'ENOENT', stdout: String(stdout || ''), stderr: String(stderr || error?.message || '') });
    });
  });

async function tailscaleHost() {
  const r = await run(['status', '--json'], 3000);
  if (!r.ok) return null;
  try {
    return parseTailscaleStatus(JSON.parse(r.stdout));
  } catch {
    return null;
  }
}

const serveCommand = (httpsPort, port) => `tailscale serve --bg --https=${httpsPort} http://127.0.0.1:${port}`;

async function tailscaleServe(httpsPort, port) {
  const r = await run(['serve', '--bg', `--https=${httpsPort}`, `http://127.0.0.1:${port}`], 20_000);
  if (r.missing) return { ok: false, code: 'noTailscale' };
  return r.ok ? { ok: true } : { ok: false, code: 'tailscaleRefused', detail: `${r.stdout} ${r.stderr}`.trim().split('\n').slice(0, 4).join(' ') };
}

async function tailscaleServeOff(httpsPort) {
  const r = await run(['serve', `--https=${httpsPort}`, 'off'], 20_000);
  return r.ok ? { ok: true } : { ok: false, code: r.missing ? 'noTailscale' : 'tailscaleRefused', detail: r.stderr.trim().split('\n')[0] };
}

async function tailscaleServing(host, httpsPort, port) {
  const r = await run(['serve', 'status', '--json'], 5000);
  if (!r.ok) return false;
  try {
    const j = JSON.parse(r.stdout);
    const site = j.Web && j.Web[`${host}:${httpsPort}`];
    const proxy = site && site.Handlers && site.Handlers['/'] && site.Handlers['/'].Proxy;
    return typeof proxy === 'string' && proxy.replace(/\/$/, '') === `http://127.0.0.1:${port}`;
  } catch {
    return false;
  }
}

const pairingUrl = (host, httpsPort, token) => `https://${host}${Number(httpsPort) === 443 ? '' : `:${httpsPort}`}/#t=${token}`;

function qrMatrix(text) {
  const { qrcodegen } = require('./vendor/qrcodegen');
  const qr = qrcodegen.QrCode.encodeText(text, qrcodegen.QrCode.Ecc.MEDIUM);
  let bits = '';
  for (let y = 0; y < qr.size; y += 1) for (let x = 0; x < qr.size; x += 1) bits += qr.getModule(x, y) ? '1' : '0';
  return { size: qr.size, bits };
}

module.exports = {
  DEFAULT_PORT,
  DEFAULT_HTTPS_PORT,
  PAIR_UNUSED_MS,
  DEVICE_IDLE_MS,
  MAX_DEVICES,
  hostnameOf,
  hostAllowed,
  originAllowed,
  FailureLimiter,
  redactWallet,
  scrubPaths,
  bearerOf,
  DeviceStore,
  remoteState,
  applyManage,
  applyPageSave,
  pageConfig,
  pageStatus,
  changesMining,
  describeChange,
  handler,
  tailscaleHost,
  tailscaleServe,
  tailscaleServeOff,
  tailscaleServing,
  parseTailscaleStatus,
  serveCommand,
  pairingUrl,
  qrMatrix,
};
