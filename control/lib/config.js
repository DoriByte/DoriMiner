const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { COINS } = require('./coins');
const { addressCheck } = require('./wallet-check');
const { checkOc } = require('./gpuctl');

const TIME = /^([01]\d|2[0-3]):[0-5]\d$/;
const ID = /^[a-z0-9-]{1,40}$/;
const POOL_URL = /^(stratum\+(tcp|ssl):\/\/)?[A-Za-z0-9.-]+:\d{1,5}$/;

const WORKER = /^[A-Za-z0-9_-]{1,32}$/;
const DEVICES = /^\d{1,2}(,\d{1,2})*$/;

const BAD_PATH = /[%"\u0000-\u001f]/;
const str = (v) => (typeof v === 'string' ? v : '');
const isObject = (x) => x !== null && typeof x === 'object' && !Array.isArray(x);
const coinOf = (id) => (typeof id === 'string' && Object.hasOwn(COINS, id) ? COINS[id] : null);

const hostWorker = () => os.hostname().replace(/[^A-Za-z0-9_-]/g, '').slice(0, 32) || 'rig';

function newId(prefix, list) {
  const used = new Set(list.map((x) => x.id));
  for (let n = 1; ; n += 1) if (!used.has(`${prefix}${n}`)) return `${prefix}${n}`;
}

function defaultOc() {
  return { powerW: null, coreLockMHz: null, memLockMHz: null, coreOffsetMHz: null, memOffsetMHz: null, fan: { mode: 'auto', pct: 70, curve: null }, coreLimitC: null, memLimitC: null };
}

function defaultProfile(id = 'p1', coin = 'QTC') {
  return { id, name: COINS[coin]?.short ?? coin, coin, walletId: null, pools: [`preset:${COINS[coin]?.defaultPool}`], worker: hostWorker(), oc: defaultOc() };
}

function defaults() {
  return {
    version: 2,
    minerPath: '',
    apiPort: 4068,
    devFee: 1,
    devices: '',
    showMinerWindow: true,
    startWithControl: false,
    restartOnCrash: true,
    language: 'auto',

    remote: { enabled: false, port: 4782, httpsPort: 8443 },

    auto: { enabled: false, profiles: [], kwhEur: null },
    currency: 'eur',

    machineName: '',
    wallets: [],
    pools: [],
    profiles: [defaultProfile()],
    activeProfile: 'p1',
    schedule: { enabled: false, slots: [{ days: [1, 2, 3, 4, 5], from: '22:00', to: '08:00', profileId: null }] },
  };
}

function migrate1(v1) {
  const c = defaults();
  for (const k of ['minerPath', 'apiPort', 'devFee', 'devices', 'showMinerWindow', 'startWithControl', 'restartOnCrash', 'language']) if (v1[k] !== undefined) c[k] = v1[k];
  const coin = COINS[v1.coin] ? v1.coin : 'QTC';
  const p = defaultProfile('p1', coin);
  if (typeof v1.worker === 'string') p.worker = v1.worker;
  const address = v1.wallets && typeof v1.wallets[coin] === 'string' ? v1.wallets[coin].trim() : '';
  if (address) {
    c.wallets.push({ id: 'w1', coin, name: 'Carteira 1', address });
    p.walletId = 'w1';
  }
  if (v1.pool?.preset === 'custom' && v1.pool.custom) {
    c.pools.push({ id: 'pool1', coin, name: v1.pool.custom.replace(/^stratum\+(tcp|ssl):\/\//, ''), url: v1.pool.custom.trim() });
    p.pools = ['pool:pool1'];
  } else if (v1.pool?.preset) {
    p.pools = [`preset:${v1.pool.preset}`];
  }
  c.profiles = [p];
  if (v1.schedule) c.schedule = { enabled: Boolean(v1.schedule.enabled), slots: (v1.schedule.slots || []).map((s) => ({ ...s, profileId: null })) };
  return c;
}

function normalize(saved) {
  let s = saved && typeof saved === 'object' ? saved : {};
  if (s.version !== 2) s = migrate1(s);
  const base = defaults();
  const c = { ...base, ...s };
  c.wallets = Array.isArray(c.wallets) ? c.wallets.filter(isObject) : [];
  c.pools = Array.isArray(c.pools) ? c.pools.filter(isObject) : [];
  c.profiles = Array.isArray(c.profiles) ? c.profiles.filter(isObject) : [];
  if (c.profiles.length === 0) c.profiles = base.profiles;
  c.profiles = c.profiles.map((p) => ({ ...defaultProfile(p.id, p.coin), ...p, oc: { ...defaultOc(), ...(p.oc || {}), fan: { ...defaultOc().fan, ...(p.oc?.fan || {}) } } }));

  for (const p of c.profiles) if (Number.isInteger(p.oc.memLimitC) && p.oc.memLimitC > 100 && p.oc.memLimitC <= 105) p.oc.memLimitC = 100;
  if (!c.profiles.some((p) => p.id === c.activeProfile)) c.activeProfile = c.profiles[0].id;
  c.remote = { ...base.remote, ...(c.remote && typeof c.remote === 'object' ? c.remote : {}) };
  c.auto = { ...base.auto, ...(c.auto && typeof c.auto === 'object' ? c.auto : {}) };
  if (!Array.isArray(c.auto.profiles)) c.auto.profiles = [];

  c.auto.profiles = c.auto.profiles.filter((id) => c.profiles.some((p) => p.id === id));
  c.schedule = { ...base.schedule, ...(isObject(c.schedule) ? c.schedule : {}) };
  if (!Array.isArray(c.schedule.slots)) c.schedule.slots = [];
  c.schedule.slots = c.schedule.slots.map((x) => ({ profileId: null, ...x }));
  return c;
}

function resolvePool(c, ref) {
  if (typeof ref !== 'string') return null;
  const [kind, id] = ref.split(/:(.*)/s);
  if (kind === 'preset') {
    for (const coin of Object.values(COINS)) {
      const p = coin.pools.find((x) => x.id === id);
      if (p) return { name: p.name, url: p.url, coin: coin.id };
    }
    return null;
  }
  if (kind === 'pool') {
    const p = c.pools.find((x) => x.id === id);
    return p ? { name: p.name, url: str(p.url).trim(), coin: p.coin } : null;
  }
  return null;
}

function validate(c) {
  const errors = {};
  const ids = (list, prefix) => {
    const seen = new Set();
    list.forEach((x, i) => {
      if (!ID.test(x.id || '') || seen.has(x.id)) errors[`${prefix}.${i}.id`] = 'badId';
      seen.add(x.id);
    });
  };
  ids(c.wallets, 'wallets');
  ids(c.pools, 'pools');
  ids(c.profiles, 'profiles');
  c.wallets.forEach((w, i) => {
    const coin = coinOf(w.coin);
    if (!coin) errors[`wallets.${i}.coin`] = 'unknownCoin';
    else if (!coin.wallet.test(str(w.address).trim())) errors[`wallets.${i}.address`] = 'badWallet';
    if (!str(w.name).trim() || w.name.length > 40) errors[`wallets.${i}.name`] = 'badName';
  });
  c.pools.forEach((p, i) => {
    if (!coinOf(p.coin)) errors[`pools.${i}.coin`] = 'unknownCoin';
    if (!POOL_URL.test(str(p.url).trim())) errors[`pools.${i}.url`] = 'badPoolUrl';
    if (!str(p.name).trim() || p.name.length > 40) errors[`pools.${i}.name`] = 'badName';
  });
  c.profiles.forEach((p, i) => {
    const at = `profiles.${i}`;
    if (!coinOf(p.coin)) errors[`${at}.coin`] = 'unknownCoin';
    if (!str(p.name).trim() || p.name.length > 40) errors[`${at}.name`] = 'badName';
    if (p.walletId !== null) {
      const w = c.wallets.find((x) => x.id === p.walletId);
      if (!w) errors[`${at}.wallet`] = 'unknownWallet';
      else if (w.coin !== p.coin) errors[`${at}.wallet`] = 'walletOtherCoin';
    }
    if (!Array.isArray(p.pools) || p.pools.length === 0 || p.pools.length > 5) errors[`${at}.pools`] = 'badPools';
    else if (p.pools.some((ref) => resolvePool(c, ref)?.coin !== p.coin)) errors[`${at}.pools`] = 'unknownPool';
    if (!WORKER.test(str(p.worker))) errors[`${at}.worker`] = 'badWorker';
    const oc = checkOc(p.oc);
    if (oc) errors[`${at}.oc`] = oc;
  });
  if (!c.profiles.some((p) => p.id === c.activeProfile)) errors.activeProfile = 'unknownProfile';
  if (!Number.isInteger(c.devFee) || c.devFee < 0 || c.devFee > 5) errors.devFee = 'badDevFee';
  if (c.devices !== '' && !DEVICES.test(str(c.devices))) errors.devices = 'badDevices';
  if (typeof c.minerPath !== 'string' || c.minerPath.length > 1000 || BAD_PATH.test(c.minerPath) || (c.minerPath !== '' && !path.isAbsolute(c.minerPath))) errors.minerPath = 'badMinerPath';
  for (const k of ['showMinerWindow', 'startWithControl', 'restartOnCrash']) if (typeof c[k] !== 'boolean') errors[k] = 'badFlag';
  if (!Number.isInteger(c.apiPort) || c.apiPort < 1024 || c.apiPort > 65535) errors.apiPort = 'badApiPort';
  if (c.currency !== 'eur' && c.currency !== 'usd') errors.currency = 'badCurrency';
  if (typeof c.machineName !== 'string' || c.machineName.length > 40 || /[\p{Cc}\p{Cf}]/u.test(c.machineName)) errors.machineName = 'badMachineName';
  const r = c.remote;
  if (typeof r.enabled !== 'boolean') errors['remote.enabled'] = 'badRemote';
  if (!Number.isInteger(r.port) || r.port < 1024 || r.port > 65535 || r.port === c.apiPort) errors['remote.port'] = 'badRemotePort';
  if (!Number.isInteger(r.httpsPort) || ![443, 8443, 10000].includes(r.httpsPort)) errors['remote.httpsPort'] = 'badHttpsPort';
  const a = c.auto;
  if (typeof a.enabled !== 'boolean' || !a.profiles.every((id) => c.profiles.some((p) => p.id === id))) errors['auto.profiles'] = 'badAuto';
  if (a.kwhEur !== null && !(typeof a.kwhEur === 'number' && a.kwhEur >= 0 && a.kwhEur <= 2)) errors['auto.kwhEur'] = 'badKwh';
  c.schedule.slots.forEach((s, i) => {
    const days = Array.isArray(s.days) && s.days.length > 0 && s.days.every((d) => Number.isInteger(d) && d >= 0 && d <= 6);
    if (!days || !TIME.test(s.from) || !TIME.test(s.to) || s.from === s.to) errors[`slot${i}`] = 'badSlot';
    else if (s.profileId !== null && !c.profiles.some((p) => p.id === s.profileId)) errors[`slot${i}`] = 'unknownProfile';
  });
  return errors;
}

function newAddressProblem(before, after) {
  for (const [i, w] of after.wallets.entries()) {
    const same = before.wallets.some((b) => b.coin === w.coin && str(b.address).trim() === str(w.address).trim());
    if (!same && addressCheck(w.coin, str(w.address).trim()) === false) return { [`wallets.${i}.address`]: 'badWallet' };
  }
  return null;
}

function argsProblem(c, profile, poolIndex = 0) {
  if (!profile || !Array.isArray(profile.pools)) return 'missing';
  const wallet = c.wallets.find((w) => w.id === profile.walletId);
  const pool = resolvePool(c, profile.pools[poolIndex]);
  if (!wallet || !pool) return 'missing';
  const coin = coinOf(profile.coin);
  if (!coin) return 'unknownCoin';
  if (!coin.wallet.test(str(wallet.address).trim())) return 'badWallet';
  if (!POOL_URL.test(pool.url)) return 'badPoolUrl';
  if (!WORKER.test(str(profile.worker))) return 'badWorker';
  if (!Number.isInteger(c.apiPort) || c.apiPort < 1024 || c.apiPort > 65535) return 'badApiPort';
  if (!Number.isInteger(c.devFee) || c.devFee < 0 || c.devFee > 5) return 'badDevFee';
  if (c.devices !== '' && !DEVICES.test(str(c.devices))) return 'badDevices';
  return null;
}

function minerArgs(c, profile, poolIndex = 0) {
  if (argsProblem(c, profile, poolIndex)) return null;
  const wallet = c.wallets.find((w) => w.id === profile.walletId);
  const pool = resolvePool(c, profile.pools[poolIndex]);
  const args = ['-o', pool.url, '-u', wallet.address.trim(), '-w', profile.worker, '--api-port', String(c.apiPort), '--dev-fee', String(c.devFee)];
  if (c.devices) args.push('-d', c.devices);

  const coin = COINS[profile.coin]?.miner;
  if (coin && coin !== 'quantus') args.unshift('--coin', coin);
  return args;
}

function load(file) {
  try {

    return normalize(JSON.parse(fs.readFileSync(file, 'utf8').trimStart()));
  } catch {
    return defaults();
  }
}

function loadChecked(file) {
  const c = load(file);
  let errors;
  try {
    errors = validate(c);
  } catch {
    errors = { file: 'badFile' };
  }
  if (Object.keys(errors).length > 0) {
    const base = defaults();
    c.remote = base.remote;
    c.startWithControl = false;
    if (errors.apiPort) c.apiPort = base.apiPort;
    if (errors.file || Object.keys(errors).some((k) => k.startsWith('slot'))) c.schedule = base.schedule;
  }
  return { config: c, errors };
}

function save(file, c) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.tmp`;
  fs.writeFileSync(tmp, `${JSON.stringify(c, null, 2)}\n`);
  fs.renameSync(tmp, file);
}

module.exports = { newAddressProblem, defaults, defaultProfile, defaultOc, normalize, validate, minerArgs, resolvePool, newId, load, loadChecked, argsProblem, save };
