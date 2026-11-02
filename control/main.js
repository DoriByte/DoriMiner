const { app, BrowserWindow, Menu, Tray, dialog, ipcMain, nativeImage, screen, session, shell } = require('electron');
const { spawn } = require('node:child_process');
const { pathToFileURL } = require('node:url');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const { refusal } = require('./lib/startguard');
{
  const refused = refusal({
    argv: process.argv,
    packaged: app.isPackaged,
    testBuild: require('node:fs').existsSync(path.join(__dirname, 'control-test-build')),
    tmpDir: require('node:os').tmpdir(),
  });
  if (refused) {
    console.error(`DoriMiner Control: ${refused}`);
    app.exit(1);
    process.exit(1);
  }
}
const config = require('./lib/config');
const { COINS } = require('./lib/coins');
const gpu = require('./lib/gpu');
const { GpuControl, changesCards, elevateCall } = require('./lib/gpuctl');
const { Miner, resolveMiner, summary } = require('./lib/miner');
const { wantsMining, profileNow } = require('./lib/schedule');
const thermal = require('./lib/thermal');
const { EnergyLog } = require('./lib/energy');

if (app.isPackaged) delete process.env.DORI_DEV_APK_DIGESTS;
const failoverRules = require('./lib/failover');
const view = require('./lib/view');
const remote = require('./lib/remote');
const { Market, estimate } = require('./lib/market');
const autoswitch = require('./lib/autoswitch');
const bench = require('./lib/bench');
const fs = require('node:fs');

const ICON = path.join(__dirname, 'assets', process.platform === 'win32' ? 'doriminer.ico' : 'icon-192.png');
const DATA = (name) => path.join(app.getPath('userData'), name);
const CONFIG_FILE = () => DATA('config.json');
const VIEW_FILE = () => DATA('view.json');
const POLL_MS = 2000;
const MEMTEMP_MS = 10_000;
const RESTART_MIN_MS = 10_000;
const RESTART_MAX_MS = 300_000;

let win = null;
let tray = null;
let quitting = false;
let cfg = null;

let cfgErrors = {};

let chosenMiner = null;
let viewState = null;
let miner = null;
let cards = null;
let admin = false;

let armed = false;
let restartAt = null;
let restartDelay = RESTART_MIN_MS;
let lastSummary = null;
let lastGpus = [];
let memTemps = new Map();
let memTempAt = 0;
let note = null;

let running = { profileId: null, poolIndex: 0, poolSince: 0, downSince: null };

let ocState = null;

let skipOc = false;

let keepCards = false;
let guardState = {};

let blindState = {};
let blindWarned = false;
let thermalStop = null;
let rescueActive = false;
let assistActive = false;
let fanTarget = null;

let devices = null;
let remoteServer = null;
let remotePort = null;
let remoteError = null;
const remoteLimiter = new remote.FailureLimiter();
const deniedNoted = new Map();
let tsCache = { at: 0, host: null };

const market = new Market();
const RATES_FILE = () => DATA('rates.json');
const LEARN_AFTER_MS = 180_000;
const AUTO_EVERY_MS = 60_000;
let rates = {};
let ratesSavedAt = 0;
let autoState = null;
let autoDecision = null;
let autoCheckedAt = 0;

const BENCH_FILE = () => DATA('bench.json');
let benchRun = null;
let energy = null;

let phoneSeenAt = 0;
const watched = () => Boolean(win && win.isVisible()) || Date.now() - phoneSeenAt < 15_000;
let benchLast = null;
let benchResults = {};

const lang = () => (cfg.language === 'auto' ? (app.getLocale().toLowerCase().startsWith('pt') ? 'pt' : 'en') : cfg.language);
const L = (pt, en) => (lang() === 'pt' ? pt : en);
const profileById = (id) => cfg.profiles.find((p) => p.id === id) ?? null;

function minerChoice() {
  return resolveMiner({ configured: cfg.minerPath, controlDir: __dirname, elevated: admin, chosen: chosenMiner, packaged: app.isPackaged });
}

function minerPath() {
  return minerChoice().path;
}

function standingNote() {
  if (blindWarned) return L('Sem leitura de temperatura de uma placa: se continuar, paro o miner dentro de menos de um minuto.', 'No temperature reading from a card: if this goes on, the miner stops in under a minute.');
  const bad = Object.keys(cfgErrors);
  if (bad.length > 0) return L(`As definições gravadas têm valores inválidos (${bad.slice(0, 3).join(', ')}), por isso nada arranca. Corrija-as em Definições e grave.`, `The saved settings have invalid values (${bad.slice(0, 3).join(', ')}), so nothing starts. Correct them in Settings and save.`);
  if (cards.stateRejected) return L('O registo do que foi mudado nas placas (cards.json) não é válido e não foi usado. Se as placas ficaram alteradas, reinicie o computador para as repor.', 'The record of what was changed on the cards (cards.json) is not valid and was not used. If the cards stayed changed, restart the computer to put them back.');
  if (minerChoice().ignored) return L('Como administrador, o caminho do miner nas definições não foi usado: uso o do pacote. Para usar outro, escolha-o em Definições → Avançado.', 'As administrator, the miner path in the settings was not used: the package\'s one runs. To use another, pick it in Settings → Advanced.');
  return null;
}

function cardIndexes() {
  if (cfg.devices) return cfg.devices.split(',').map(Number);
  return lastGpus.map((g) => g.index);
}

const limitsOf = (p) => ({ coreC: p?.oc?.coreLimitC ?? thermal.POLICY.coreLimitC, memC: p?.oc?.memLimitC ?? thermal.POLICY.memoryLimitC });

async function launch(profileId, poolIndex) {
  return launchProfile(profileById(profileId), poolIndex);
}

async function launchProfile(profile, poolIndex, c = cfg, { noOc = false } = {}) {
  const profileId = profile?.id ?? null;
  if (Object.keys(cfgErrors).length > 0) {
    note = standingNote();
    return false;
  }
  const exe = minerPath();
  const problem = config.argsProblem(c, profile, poolIndex);
  if (problem && problem !== 'missing') {
    note = L(`O perfil "${profile?.name ?? '?'}" tem um valor inválido (${problem}). Não lancei o miner.`, `Profile "${profile?.name ?? '?'}" has an invalid value (${problem}). The miner was not started.`);
    return false;
  }
  const args = config.minerArgs(c, profile, poolIndex);
  if (!exe) {
    note = L('Não encontrei o miner: escolha o dori-miner em Definições → Avançado.', 'Miner not found: pick dori-miner in Settings → Advanced.');
    return false;
  }
  if (!args) {
    note = L(`O perfil "${profile?.name ?? '?'}" não tem carteira ou pool.`, `Profile "${profile?.name ?? '?'}" has no wallet or pool.`);
    return false;
  }

  const coin = COINS[profile.coin];
  if (coin?.cards) {
    if (lastGpus.length === 0) lastGpus = await gpu.read();
    const fit = lastGpus.filter((g) => coin.cards.test(g.name));
    if (lastGpus.length > 0 && fit.length === 0) {
      note = L(`${coin.short} só se mina em placas ${coin.cardsNote}: nenhuma placa deste computador serve.`, `${coin.short} is mined on ${coin.cardsNote} cards only: no card in this computer fits.`);
      return false;
    }
  }
  ocState = null;
  fanTarget = null;
  if (changesCards(profile.oc) && !skipOc && !keepCards && !noOc) {
    if (!admin) {
      note = L(
        `O perfil "${profile.name}" muda as placas (potência, relógios ou ventoinhas), e isso precisa de administrador.`,
        `Profile "${profile.name}" changes the cards (power, clocks or fans), which needs administrator.`,
      );
      ocState = { code: 'needAdmin' };
      return false;
    }
    if (lastGpus.length === 0) lastGpus = await gpu.read();
    const r = await cards.apply(cardIndexes(), profile.oc);
    if (!r.ok) {
      note = L(`Não consegui aplicar as definições das placas (${r.code}${r.detail ? `: ${r.detail}` : ''}). Não lancei o miner.`, `Could not apply the card settings (${r.code}${r.detail ? `: ${r.detail}` : ''}). The miner was not started.`);
      ocState = { code: r.code, detail: r.detail };
      return false;
    }
    ocState = { code: r.steps.some((s) => s.startsWith('memLockRefused')) ? 'partial' : 'ok', steps: r.steps };
    if (profile.oc.fan.mode === 'fixed') await setFans(profile.oc.fan.pct);
  }
  keepCards = false;
  note = null;
  try {
    miner.start(exe, args, cfg.showMinerWindow, { profileId, poolIndex });
  } catch (err) {
    note = err.message;
    await cards.restore();
    return false;
  }
  running = { profileId, poolIndex, poolSince: Date.now(), downSince: null };
  return true;
}

async function stopMiner({ keep = false } = {}) {
  await miner.stop();
  if (!keep) await restoreCards();
}

async function restoreCards() {
  fanTarget = null;
  rescueActive = false;
  assistActive = false;
  if (!cards.dirty) return;
  const r = await cards.restore();
  if (!r.ok) {
    note = L(
      `Não consegui repor tudo nas placas (${r.failed.map((f) => f.what).join(', ')}). ${admin ? '' : 'Abra o Control como administrador para as repor.'}`,
      `Could not put everything back on the cards (${r.failed.map((f) => f.what).join(', ')}). ${admin ? '' : 'Open Control as administrator to put them back.'}`,
    );
  }
}

async function setFans(pct) {
  if (!admin || fanTarget === pct) return;
  const r = await cards.setFans(cardIndexes(), pct);
  if (r.ok) fanTarget = pct;
}

function onMinerExit(exit) {
  if (quitting) return;
  if (benchRun && !miner.wantStop) {

    const it = benchRun.items[benchRun.i];
    if (it && it.state === 'running') {
      it.state = 'failed';
      it.reason = L(`O miner parou (código ${exit.code}).`, `The miner stopped (code ${exit.code}).`);
      restoreCards().then(() => benchNext());
    }
    return;
  }
  if (exit.byUser) {

    if (!miner.wantStop) {
      armed = false;
      restoreCards();
    }
    return;
  }
  restoreCards();
  if (armed && cfg.restartOnCrash) {
    restartAt = Date.now() + restartDelay;
    note = L(`O miner parou (código ${exit.code}). Volto a lançá-lo em ${Math.round(restartDelay / 1000)} s.`, `The miner stopped (code ${exit.code}). Starting it again in ${Math.round(restartDelay / 1000)} s.`);
    restartDelay = Math.min(restartDelay * 2, RESTART_MAX_MS);
  } else {
    armed = false;
    note = L(`O miner parou (código ${exit.code}).`, `The miner stopped (code ${exit.code}).`);
  }
}

async function reconcile() {
  if (benchRun) return;
  const inHours = wantsMining(cfg.schedule);
  const wanted = profileNow(cfg);
  if (thermalStop && !thermalStop.mayResume) {
    if (miner.running) await stopMiner();
    return;
  }

  if (thermalStop && !armed) thermalStop = null;
  if (armed && inHours && !miner.running) {
    if (restartAt && Date.now() < restartAt) return;
    if (ocState?.code === 'needAdmin' && !skipOc) return;
    restartAt = null;

    if (await summary(cfg.apiPort, 800)) {
      note = L(`Já há um miner a responder na porta ${cfg.apiPort}, lançado fora do Control.`, `A miner started outside Control already answers on port ${cfg.apiPort}.`);
      return;
    }
    thermalStop = null;
    await launch(wanted, wanted === running.profileId ? running.poolIndex : 0);
  } else if (miner.running && (!armed || !inHours)) {
    await stopMiner();
  } else if (miner.running && wanted !== running.profileId) {

    skipOc = false;
    await stopMiner();
    await launch(wanted, 0);
  }
  if (miner.running && miner.startedAt && Date.now() - miner.startedAt > 600_000) restartDelay = RESTART_MIN_MS;
}

async function failover() {
  if (!miner.running || benchRun) return;
  const profile = profileById(running.profileId);
  if (!profile || profile.pools.length < 2) return;
  const d = failoverRules.decide(running, profile.pools.length, lastSummary?.pool?.connected === true, Date.now());
  running.downSince = d.downSince;
  const next = d.next;
  if (next === null) return;
  const name = (i) => config.resolvePool(cfg, profile.pools[i])?.name ?? '?';
  const why = next === 0 ? L('volto à primeira pool', 'back to the first pool') : L('a pool não responde há 2 minutos', 'the pool has not answered for 2 minutes');
  note = L(`Mudei de ${name(running.poolIndex)} para ${name(next)}: ${why}.`, `Moved from ${name(running.poolIndex)} to ${name(next)}: ${why}.`);
  const keepNote = note;

  await stopMiner({ keep: true });
  const prevOc = ocState;
  keepCards = true;
  const started = await launch(running.profileId, next);
  keepCards = false;

  if (!started) await restoreCards();
  ocState = prevOc;
  note = keepNote;
}

async function watchTemps() {
  if (!miner.running && !thermalStop) return;
  const profile = profileById(running.profileId);
  const limits = limitsOf(profile);
  const used = new Set(cardIndexes());
  const watched = lastGpus.filter((g) => used.has(g.index));
  const max = (f) => watched.reduce((m, g) => (g[f] === null || g[f] === undefined ? m : m === null ? g[f] : Math.max(m, g[f])), null);
  const coreC = max('coreC');
  const memC = max('memC');
  const core = { tempC: coreC, limitC: limits.coreC };
  const memory = { tempC: memC, limitC: limits.memC };

  const seesAll = used.size > 0 && watched.length === used.size && watched.every((x) => typeof x.coreC === 'number');
  const blind = miner.running ? thermal.blindness(blindState, Date.now(), seesAll) : thermal.blindness({}, Date.now(), true);
  blindState = blind.state;
  blindWarned = blind.warn;
  if (blind.stop) {
    blindState = {};
    blindWarned = false;
    guardState = { stoppedAt: Date.now(), stoppedPart: 'core' };
    thermalStop = { at: Date.now(), reason: { part: 'none' }, mayResume: false };
    if (benchRun) await benchFinish(true);
    note = L(
      `Sem leitura de temperatura há ${Math.round(thermal.POLICY.blindStopMs / 1000)} s: parei o miner por segurança. Volta sozinho quando houver leitura, 10 °C abaixo do limite e passados 5 minutos.`,
      `No temperature reading for ${Math.round(thermal.POLICY.blindStopMs / 1000)} s: miner stopped to be safe. It comes back by itself once there is a reading, 10 °C under the limit and after 5 minutes.`,
    );
    await stopMiner();
    return;
  }

  const g = thermal.guard(guardState, Date.now(), core, memory);
  guardState = g.state;
  if (g.stop) {
    const r = g.reason;
    thermalStop = { at: Date.now(), reason: r, mayResume: false };
    if (benchRun) await benchFinish(true);
    note = L(
      `${r.part === 'core' ? 'A placa' : 'A memória'} esteve a ${r.tempC} °C (limite ${r.limitC} °C) durante mais de um minuto: parei o miner para arrefecer. Volta sozinho quando descer 10 °C e passarem 5 minutos.`,
      `The ${r.part === 'core' ? 'card' : 'memory'} was at ${r.tempC} °C (limit ${r.limitC} °C) for over a minute: miner stopped to cool down. It comes back by itself 10 °C lower and after 5 minutes.`,
    );
    await stopMiner();
    return;
  }
  if (thermalStop) {
    if (g.mayResume) {
      thermalStop.mayResume = true;
      guardState = {};
    }
    return;
  }

  if (!seesAll || !admin) return;
  rescueActive = thermal.rescue(rescueActive, core, memory);
  const fan = profile?.oc?.fan ?? { mode: 'auto' };
  let want = null;
  if (rescueActive) want = 100;
  else if (fan.mode === 'fixed') want = fan.pct;
  else if (fan.mode === 'curve') want = thermal.curvePct(fan.curve, thermal.fanTempOf(coreC, memC));
  else {
    const assist = thermal.memoryAssist(assistActive, coreC, memory);
    assistActive = assist !== null;
    want = assist;
  }
  if (want === null) {
    if (fanTarget !== null) {
      const r = await cards.setFans(cardIndexes(), 'auto');
      if (r.ok) fanTarget = null;
    }
  } else if (fanTarget === null || Math.abs(want - fanTarget) >= 3 || want === 100) {
    await setFans(want);
  }
}

function minedWatts() {
  const used = new Set(cardIndexes());
  const w = lastGpus.filter((g) => used.has(g.index) && typeof g.watts === 'number').map((g) => g.watts);
  return w.length ? w.reduce((a, b) => a + b, 0) : null;
}

function learnRate() {
  if (benchRun || !miner.running || !lastSummary?.hashrate || !miner.startedAt || Date.now() - miner.startedAt < LEARN_AFTER_MS) return;
  const id = running.profileId;
  const watts = minedWatts();
  const old = rates[id];
  const mix = (a, b) => (a === null || a === undefined ? b : b === null ? a : a * 0.9 + b * 0.1);
  rates[id] = { hps: mix(old?.hps, lastSummary.hashrate), watts: mix(old?.watts ?? null, watts), at: Date.now() };
  if (Date.now() - ratesSavedAt > 300_000) {
    ratesSavedAt = Date.now();
    try {
      fs.writeFileSync(`${RATES_FILE()}.tmp`, JSON.stringify(rates));
      fs.renameSync(`${RATES_FILE()}.tmp`, RATES_FILE());
    } catch {}
  }
}

function optionOf(profile) {
  if (!profile) return null;
  const m = market.of(profile.coin);
  if (!m) return null;
  const live = miner.running && running.profileId === profile.id && lastSummary?.hashrate > 0;
  const perf = live ? { hps: lastSummary.hashrate, watts: minedWatts() } : rates[profile.id];
  if (!perf) return null;
  const e = estimate(m.price, m.network, perf, { feeFraction: (cfg.devFee || 0) / 100, kwhEur: cfg.auto.kwhEur });
  if (!e) return null;
  return { ...e, coin: profile.coin, profileId: profile.id, live, coinsPerDay: e.revenue / m.price.eur, priceEur: m.price.eur };
}

async function autoStep() {
  if (Date.now() - autoCheckedAt < AUTO_EVERY_MS) return;
  autoCheckedAt = Date.now();
  if (!cfg.auto.enabled || !armed || !miner.running || benchRun) {
    autoDecision = null;
    return;
  }
  const profile = profileById(running.profileId);
  if (!profile || !cfg.auto.profiles.includes(profile.id)) {
    autoDecision = { kind: 'manual' };
    return;
  }
  const now = Date.now();
  const options = cfg.auto.profiles.map((id) => optionOf(profileById(id))).filter(Boolean);
  const candidates = autoswitch.bestPerCoin(options).map((o) => ({ coin: o.coin, profileId: o.profileId, net: o.net, priceAt: o.priceAt, networkAt: o.networkAt }));
  if (!autoState || autoState.current !== profile.coin) autoState = autoswitch.initial(profile.coin, miner.startedAt || now);
  const r = autoswitch.decide(autoState, candidates, now);
  autoState = r.state;
  autoDecision = r.decision;
  if (r.decision.kind === 'switch' && r.decision.profileId !== running.profileId) {
    const to = profileById(r.decision.profileId);
    const eur = (x) => `${x.toFixed(2)} €`;
    const keep = L(
      `Troca automática: ${to?.name} rende ${eur(r.decision.to.net)} por dia contra ${eur(r.decision.from.net)} de ${profile.name}.`,
      `Automatic switch: ${to?.name} pays ${eur(r.decision.to.net)} a day against ${eur(r.decision.from.net)} for ${profile.name}.`,
    );
    await applyConfig({ ...cfg, activeProfile: r.decision.profileId });
    note = keep;
  }
}

function autoInfo() {
  return {
    enabled: cfg.auto.enabled,
    decision: autoDecision,
    profiles: cfg.profiles.map((p) => {
      const o = optionOf(p);
      return { id: p.id, name: p.name, coin: p.coin, inSwitch: cfg.auto.profiles.includes(p.id), measured: Boolean(rates[p.id]) || (miner.running && running.profileId === p.id), net: o?.net ?? null, revenue: o?.revenue ?? null, coinsPerDay: o?.coinsPerDay ?? null };
    }),
  };
}

function saveJson(file, value) {
  try {
    fs.writeFileSync(`${file}.tmp`, JSON.stringify(value));
    fs.renameSync(`${file}.tmp`, file);
  } catch {}
}

async function benchPlan(scope) {
  if (lastGpus.length === 0) lastGpus = await gpu.read();
  const items = bench.plan(cfg, scope, lastGpus);
  return {
    items: items.map((x) => ({ coin: x.coin, name: x.profile.name, profileId: x.profileId, hasWallet: Boolean(x.wallet), supported: x.supported })),
    missing: bench.missingWallets(items),
    seconds: { own: bench.plannedSeconds(items, false), dev: bench.plannedSeconds(items, true) },
  };
}

async function benchStart(scope, useDev) {
  if (benchRun) return { ok: false, code: 'running' };
  if (status().state === 'external') return { ok: false, code: 'external' };
  if (lastGpus.length === 0) lastGpus = await gpu.read();
  const items = bench.plan(cfg, scope, lastGpus).map((x) => {
    const wallet = bench.walletFor(x, useDev);
    const state = !x.supported ? 'unsupported' : !wallet ? 'noWallet' : 'waiting';
    return { ...x, address: wallet, dev: Boolean(wallet && !x.wallet), state, hps: null, watts: null };
  });
  if (!items.some((x) => x.state === 'waiting')) return { ok: false, code: 'nothing' };
  benchRun = { scope, items, i: -1, phase: null, since: 0, samples: [], wasArmed: armed };
  armed = false;
  restartAt = null;
  note = null;
  await stopMiner();
  await benchNext();
  return { ok: true };
}

async function benchNext() {
  const b = benchRun;
  for (b.i += 1; b.i < b.items.length; b.i += 1) {
    const it = b.items[b.i];
    if (it.state !== 'waiting') continue;
    const c = { ...cfg, wallets: [...cfg.wallets, { id: '__bench', coin: it.coin, name: 'bench', address: it.address }] };
    const profile = { ...it.profile, walletId: '__bench', worker: it.dev ? 'doribench' : it.profile.worker };

    it.noOc = changesCards(profile.oc) && !admin;
    const started = await launchProfile(profile, 0, c, { noOc: it.noOc });
    if (!started) {
      it.state = 'failed';
      it.reason = note;
      note = null;
      continue;
    }
    it.state = 'running';
    b.phase = 'warm';
    b.since = Date.now();
    b.samples = [];
    return;
  }
  await benchFinish(false);
}

async function benchStep() {
  const b = benchRun;
  if (!b || b.i < 0 || b.i >= b.items.length) return;
  const it = b.items[b.i];
  if (it.state !== 'running') return;
  if (b.phase === 'warm' && Date.now() - b.since >= bench.WARM_MS) {
    b.phase = 'measure';
    b.since = Date.now();
  }
  if (b.phase === 'measure') {
    if (lastSummary?.hashrate > 0) b.samples.push({ hps: lastSummary.hashrate, watts: minedWatts() });
    if (Date.now() - b.since >= bench.MEASURE_MS) {
      it.hps = bench.median(b.samples.map((s) => s.hps));
      it.watts = bench.median(b.samples.map((s) => s.watts));
      it.state = it.hps ? 'done' : 'failed';
      if (!it.hps) it.reason = L('O miner não mostrou hashrate.', 'The miner showed no hash rate.');
      if (it.hps) {
        const r = { hps: it.hps, watts: it.watts, at: Date.now() };
        benchResults[it.coin] = { ...r, profileId: it.profileId, dev: it.dev, noOc: Boolean(it.noOc) };
        if (it.profileId && !it.noOc) rates[it.profileId] = r;
        saveJson(BENCH_FILE(), benchResults);
        saveJson(RATES_FILE(), rates);
      }
      await stopMiner();
      await benchNext();
    }
  }
}

async function benchFinish(cancelled) {
  const b = benchRun;
  if (!b) return;
  for (const it of b.items) if (it.state === 'running' || (cancelled && it.state === 'waiting')) it.state = 'cancelled';
  benchRun = null;
  benchLast = b.items;
  if (miner.running) await stopMiner();
  armed = b.wasArmed;
  note = cancelled ? L('Benchmark cancelado.', 'Benchmark cancelled.') : L('Benchmark terminado.', 'Benchmark finished.');
}

function benchView() {
  const priced = Object.entries(benchResults).map(([coin, r]) => {
    const m = market.of(coin);
    const e = m ? estimate(m.price, m.network, r, { feeFraction: (cfg.devFee || 0) / 100, kwhEur: cfg.auto.kwhEur }) : null;
    return { coin, hps: r.hps, watts: r.watts, at: r.at, dev: r.dev, noOc: r.noOc, profileId: r.profileId && profileById(r.profileId) ? r.profileId : null, net: e?.net ?? null, revenue: e?.revenue ?? null };
  });
  priced.sort((a, b) => (b.net ?? -1) - (a.net ?? -1));
  const run = benchRun ?? (benchLast ? { items: benchLast, i: -1, phase: null } : null);
  return {
    active: Boolean(benchRun),
    phase: benchRun?.phase ?? null,
    left: benchRun && benchRun.i >= 0 && benchRun.i < benchRun.items.length ? Math.max(0, Math.round(((benchRun.phase === 'warm' ? bench.WARM_MS + bench.MEASURE_MS : bench.MEASURE_MS) - (Date.now() - benchRun.since)) / 1000)) : null,
    items: run ? run.items.map((x) => ({ coin: x.coin, name: x.profile.name, state: x.state, dev: x.dev, noOc: Boolean(x.noOc), hps: x.hps, watts: x.watts, reason: x.reason ?? null })) : [],
    results: priced,
    unsupported: Object.keys(COINS).filter((c) => !bench.supported(c, lastGpus)),
  };
}

const machineLabel = () => (typeof cfg.machineName === 'string' && cfg.machineName) || os.hostname();

function status() {
  const inHours = wantsMining(cfg.schedule);
  let state = 'stopped';
  if (benchRun) state = 'bench';
  else if (miner.running) state = 'mining';
  else if (thermalStop) state = 'cooling';
  else if (lastSummary) state = 'external';
  else if (armed && ocState?.code === 'needAdmin') state = 'needAdmin';
  else if (armed && !inHours) state = 'waiting';
  else if (armed && restartAt) state = 'restarting';
  const profile = profileById(miner.running ? running.profileId : profileNow(cfg));
  return {
    state,
    armed,
    admin,
    pid: miner.pid,
    startedAt: miner.startedAt,
    showWindow: miner.running ? miner.showWindow : cfg.showMinerWindow,
    lastExit: miner.lastExit,
    note: (blindWarned ? standingNote() : note) ?? standingNote(),
    minerPath: minerPath(),
    summary: lastSummary,
    gpus: lastGpus,
    profileId: profile?.id ?? null,
    pool: miner.running && profile ? { index: running.poolIndex, of: profile.pools.length, name: config.resolvePool(cfg, profile.pools[running.poolIndex])?.name ?? null } : null,
    oc: ocState,
    cardsChanged: cards.dirty,
    fans: { target: fanTarget, rescue: rescueActive, assist: assistActive },
    limits: limitsOf(profile),
    log: miner.running && !miner.showWindow ? miner.log.slice(-120) : miner.log.slice(-40),
    earn: miner.running ? optionOf(profileById(running.profileId)) : null,
    auto: autoInfo(),
    bench: benchView(),
    energy: energy ? energy.view(cfg.auto.kwhEur) : null,
    usdPerEur: market.usdPerEur(),
    machine: machineLabel(),
    systemName: os.hostname(),
  };
}

async function tick() {
  try {
    miner.checkAdopted();
    await reconcile();

    const watch = miner.running || thermalStop;
    const [s, g] = await Promise.all([summary(cfg.apiPort), watch || watched() ? gpu.read() : Promise.resolve(lastGpus)]);
    lastSummary = s;
    if (Date.now() - memTempAt >= MEMTEMP_MS && (watch || watched())) {
      memTempAt = Date.now();
      memTemps = await cards.readMemoryTemps();
    }

    const others = !miner.running && !benchRun && watched() ? await gpu.users() : null;
    lastGpus = g.map((card) => ({ ...card, memC: card.bus !== null ? memTemps.get(card.bus) ?? null : null, usedBy: others ? gpu.usedBy(card, others.get(card.index)) : null }));

    if (energy) {
      const usedCards = new Set((s?.gpus ?? []).map((m) => m.id));
      energy.step(miner.running ? lastGpus.filter((c) => usedCards.size === 0 || usedCards.has(c.index)).reduce((a, c) => a + (c.watts ?? 0), 0) : null);
    }
    await watchTemps();
    await benchStep();
    await failover();
    learnRate();

    if (miner.running || benchRun || cfg.auto.enabled || watched()) market.refresh().catch(() => {});
    await autoStep();
    updateTray();
    if (win && !win.isDestroyed()) win.webContents.send('status', status());
  } catch (err) {
    note = `tick: ${err.message}`;
  } finally {
    setTimeout(tick, POLL_MS);
  }
}

function hashrate(h) {
  if (!h) return '0 H/s';
  const units = ['H/s', 'kH/s', 'MH/s', 'GH/s', 'TH/s'];
  let i = 0;
  while (h >= 1000 && i < units.length - 1) {
    h /= 1000;
    i += 1;
  }
  return `${h.toFixed(1)} ${units[i]}`;
}

function updateTray() {
  if (!tray) return;
  const tip = miner.running ? `DoriMiner · ${hashrate(lastSummary?.hashrate)}` : L('DoriMiner · parado', 'DoriMiner · stopped');
  tray.setToolTip(tip);
}

function trayMenu() {
  return Menu.buildFromTemplate([
    { label: L('Abrir', 'Open'), click: showWindow },
    { type: 'separator' },
    { label: L('Minerar', 'Mine'), click: () => ipcStart() },
    { label: L('Parar', 'Stop'), click: () => ipcStop() },
    { type: 'separator' },
    { label: L('Sair (para o miner)', 'Quit (stops the miner)'), click: quit },
  ]);
}

function showWindow() {
  if (!win || win.isDestroyed()) createWindow();
  win.show();
  win.focus();
}

async function quit() {
  quitting = true;
  energy?.save();
  await stopMiner();
  app.exit(0);
}

async function elevate() {
  if (process.platform !== 'win32') return false;
  const call = elevateCall(process.execPath, app.isPackaged ? null : app.getAppPath(), armed);
  quitting = true;
  await stopMiner();
  app.releaseSingleInstanceLock();
  const child = spawn(call.file, call.args, { windowsHide: true, stdio: 'ignore', env: { ...process.env, ...call.env } });
  child.on('exit', (code) => {
    if (code === 0) app.exit(0);
    else {

      quitting = false;
      app.requestSingleInstanceLock();
      note = L('Ficou sem administrador.', 'Still without administrator.');
    }
  });
  return true;
}

function placementFor(mode) {
  const areas = screen.getAllDisplays().map((d) => d.workArea);
  return view.placement(viewState, mode, areas, screen.getPrimaryDisplay().workArea);
}

let boundsTimer = null;
function rememberBounds() {
  clearTimeout(boundsTimer);
  boundsTimer = setTimeout(() => {
    if (!win || win.isDestroyed() || win.isMinimized() || win.isMaximized()) return;
    viewState.bounds[viewState.mode] = win.getBounds();
    view.save(VIEW_FILE(), viewState);
  }, 500);
}

function applyView(next) {
  const old = viewState.mode;
  if (next.mode && next.mode !== old && view.SIZES[next.mode]) {
    if (!win.isMinimized() && !win.isMaximized()) viewState.bounds[old] = win.getBounds();
    if (win.isMaximized()) win.unmaximize();
    viewState.mode = next.mode;
    const size = view.SIZES[next.mode];
    win.setMinimumSize(size.minWidth, size.minHeight);
    win.setBounds(placementFor(next.mode));
  }
  if (typeof next.alwaysOnTop === 'boolean') viewState.alwaysOnTop = next.alwaysOnTop;

  win.setAlwaysOnTop(viewState.mode === 'compact' && viewState.alwaysOnTop);
  view.save(VIEW_FILE(), viewState);
  return { mode: viewState.mode, alwaysOnTop: viewState.alwaysOnTop };
}

function createWindow() {
  const size = view.SIZES[viewState.mode];
  win = new BrowserWindow({
    ...placementFor(viewState.mode),
    minWidth: size.minWidth,
    minHeight: size.minHeight,
    alwaysOnTop: viewState.mode === 'compact' && viewState.alwaysOnTop,
    title: 'DoriMiner Control',
    icon: ICON,
    backgroundColor: '#0e141d',
    autoHideMenuBar: true,
    webPreferences: { preload: path.join(__dirname, 'preload.js'), contextIsolation: true, sandbox: true, nodeIntegration: false },
  });
  win.loadFile(path.join(__dirname, 'renderer', 'index.html'));

  win.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https:\/\//.test(url) || /^mailto:[\w.+-]+@[\w.-]+$/.test(url)) shell.openExternal(url);
    return { action: 'deny' };
  });
  win.webContents.on('will-navigate', (e) => e.preventDefault());
  win.on('move', rememberBounds);
  win.on('resize', rememberBounds);
  win.on('close', (e) => {
    if (!quitting) {
      e.preventDefault();
      win.hide();
    }
  });
}

function ipcStart() {
  if (benchRun) {
    benchRun.wasArmed = true;
    return Promise.resolve();
  }
  armed = true;
  restartAt = null;
  restartDelay = RESTART_MIN_MS;
  note = null;
  if (ocState?.code === 'needAdmin') ocState = null;
  return reconcile();
}

async function ipcStop() {
  if (benchRun) await benchFinish(true);
  armed = false;
  restartAt = null;
  skipOc = false;

  if (ocState?.code === 'needAdmin') ocState = null;
  await stopMiner();
}

async function applyConfig(next) {
  const c = config.normalize(next);
  const errors = config.validate(c);
  if (Object.keys(errors).length > 0) return { ok: false, errors };
  const badAddress = config.newAddressProblem(cfg, c);
  if (badAddress) return { ok: false, errors: badAddress };
  const key = (x) => {
    const p = x.profiles.find((q) => q.id === (miner.running ? running.profileId : profileNow(x)));
    return JSON.stringify([config.minerArgs(x, p, running.poolIndex), p?.oc, x.showMinerWindow, x.minerPath]);
  };
  const before = key(cfg);
  const remoteBefore = cfg.remote;

  config.save(CONFIG_FILE(), c);
  cfg = c;
  cfgErrors = {};

  if (win && !win.isDestroyed()) win.webContents.send('config', cfg);
  tray?.setContextMenu(trayMenu());

  if (remoteBefore.enabled && (!cfg.remote.enabled || remoteBefore.httpsPort !== cfg.remote.httpsPort)) await remote.tailscaleServeOff(remoteBefore.httpsPort);
  await applyRemote();

  let restarted = false;
  if (miner.running && (before !== key(cfg) || !profileById(running.profileId))) {
    await stopMiner();
    running.poolIndex = 0;
    restarted = true;
    await reconcile();
  }
  if (ocState?.code === 'needAdmin') ocState = null;
  return { ok: true, config: cfg, restarted, lang: lang() };
}

async function applyRemote() {
  const want = cfg.remote.enabled ? `${cfg.remote.port}:${cfg.remote.httpsPort}` : null;
  if (remoteServer && remoteServer.key === want) return;
  if (remoteServer) {
    const old = remoteServer;
    remoteServer = null;
    remotePort = null;
    await new Promise((resolve) => {
      old.close(() => resolve());
      old.closeAllConnections();
    });
  }
  remoteError = null;
  if (want === null) return;
  const srv = http.createServer(
    remote.handler({
      devices,
      limiter: remoteLimiter,
      httpsPort: cfg.remote.httpsPort,
      staticDir: path.join(__dirname, 'remote'),
      state: (manage) => remote.remoteState(status(), cfg, machineLabel(), manage),
      act: remoteAct,
      manage: remoteManage,
      page: {
        init: (manage) => ({
          config: remote.pageConfig(cfg, manage),
          coins: COINS,
          lang: lang(),
          platform: process.platform,
          version: app.getVersion(),
          view: { mode: 'window', alwaysOnTop: false },
          policy: thermal.POLICY,
          defaultCurve: thermal.DEFAULT_CURVE,
        }),
        status: (manage) => {
          phoneSeenAt = Date.now();
          return { status: remote.pageStatus(status(), cfg, manage), config: remote.pageConfig(cfg, manage) };
        },
      },
      onAttested: (device, level) => {
        recordRemoteChange(device, 'attested', level);
        note = L(`Telemóvel "${device.name}" verificado como aplicação DoriRig: passa a poder mudar carteiras e perfis.`, `Phone "${device.name}" verified as the DoriRig app: it can now change wallets and profiles.`);
      },
      onDenied: noteDenied,
    }),
  );
  srv.key = want;
  await new Promise((resolve) => {
    srv.once('error', (err) => {
      remoteError = err.code === 'EADDRINUSE' ? 'portInUse' : err.code || 'listenFailed';
      resolve();
    });
    srv.listen(cfg.remote.port, '127.0.0.1', () => {
      remoteServer = srv;
      remotePort = cfg.remote.port;
      resolve();
    });
  });
}

function noteDenied(why, login) {
  const last = deniedNoted.get(why);
  if (last !== undefined && Date.now() - last < 600_000) return;
  deniedNoted.set(why, Date.now());
  const who = login ?? L('conta Tailscale desconhecida', 'unknown Tailscale account');
  note =
    why === 'none'
      ? L(`Acesso remoto recusado: um dispositivo (${who}) pediu sem token; emparelhe-o outra vez.`, `Remote access refused: a device (${who}) asked without a token; pair it again.`)
      : L(`Acesso remoto recusado: um dispositivo (${who}) usou um token que este Control não conhece (revogado, caducado ou de outra máquina).`, `Remote access refused: a device (${who}) used a token this Control does not know (revoked, expired or another machine's).`);
}

function recordRemoteChange(device, what, detail) {
  try {
    fs.appendFileSync(DATA('remote-changes.log'), `${JSON.stringify({ at: new Date().toISOString(), device: device.name, what, detail })}\n`, { mode: 0o600 });
  } catch {

  }
}

async function remoteManage(what, body, device) {

  const scope = body.scope && body.scope.kind === 'profile' && typeof body.scope.id === 'string' ? { kind: 'profile', id: body.scope.id } : { kind: 'all' };
  if (what === 'bench-plan') return { ok: true, reply: { plan: await benchPlan(scope) } };
  if (what === 'bench-cancel') {
    if (!benchRun) return { ok: true };
    await benchFinish(true);
    recordRemoteChange(device, 'bench-cancel', scope);
    note = L(`Benchmark cancelado pelo telemóvel "${device.name}".`, `Benchmark cancelled from the phone "${device.name}".`);
    return { ok: true };
  }
  if (what === 'bench-start') {
    const r = await benchStart(scope, body.useDev === true);
    if (r.ok !== false) {
      recordRemoteChange(device, 'bench', scope);
      note = L(`Benchmark iniciado pelo telemóvel "${device.name}".`, `Benchmark started from the phone "${device.name}".`);
    }
    return { ok: true, reply: { bench: r } };
  }
  if (benchRun) return { ok: false, code: 'benchRunning' };
  const change = what === 'config' ? remote.applyPageSave(cfg, body.config) : remote.applyManage(cfg, what, body);
  if (!change.ok) return change;

  if (miner.running && body.restart !== true && remote.changesMining(cfg, change.next, running.profileId, running.poolIndex)) return { ok: false, code: 'restartNeeded' };
  const snapshot = (c) => ({ wallets: c.wallets, pools: c.pools, profiles: c.profiles.map((p) => ({ id: p.id, name: p.name, walletId: p.walletId, worker: p.worker, pools: p.pools })), activeProfile: c.activeProfile });
  const before = what === 'config' ? snapshot(cfg) : remote.describeChange(cfg, what, body);

  const changedItems = (now, was, show) => now.filter((x) => { const b = was.find((y) => y.id === x.id); return !b || JSON.stringify(b) !== JSON.stringify(x); }).map(show);
  const configLabel = () => [
    ...changedItems(change.next.wallets, cfg.wallets, (w) => `${w.name} ${remote.redactWallet(w.address)}`),
    ...changedItems(change.next.pools, cfg.pools, (p) => `${p.name} ${p.url}`),
    ...changedItems(change.next.profiles, cfg.profiles, (p) => p.name),
    ...[...cfg.wallets, ...cfg.pools, ...cfg.profiles].filter((x) => ![...change.next.wallets, ...change.next.pools, ...change.next.profiles].some((y) => y.id === x.id)).map((x) => L(`${x.name} apagado`, `${x.name} deleted`)),
  ].join(', ') || L('definições', 'settings');
  const label = what === 'config' ? configLabel() : what === 'wallet' ? `${text(body.name)} ${remote.redactWallet(text(body.address))}` : what === 'profile' ? text(body.name) : String(body.id);
  const old = cfg;
  let r;
  try {
    r = await applyConfig(change.next);
  } catch (error) {

    if (cfg === old) throw error;
    r = { ok: true, restarted: true, failed: true };
  }
  if (cfg !== old) recordRemoteChange(device, what, { label, before, after: what === 'config' ? snapshot(cfg) : remote.describeChange(cfg, what, { ...body, id: body.id ?? change.newId }) });
  if (!r.ok) return { ok: false, code: Object.values(r.errors)[0] || 'failed' };
  const by = L(`pelo telemóvel "${device.name}"`, `from the phone "${device.name}"`);
  const changed = what === 'config' ? L(`Alterado ${by}: ${label}.`, `Changed ${by}: ${label}.`) : what.startsWith('wallet') ? L(`Carteiras alteradas ${by}: ${label}.`, `Wallets changed ${by}: ${label}.`) : L(`Perfis alterados ${by}: ${label}.`, `Profiles changed ${by}: ${label}.`);

  note = r.restarted && armed && !miner.running && note ? `${changed} ${note}` : changed;
  if (r.failed) return { ok: false, code: 'savedRestartFailed' };
  return { ok: true, reply: what === 'config' ? { config: remote.pageConfig(cfg, true), restarted: r.restarted, lang: lang() } : undefined };
}
const text = (v) => (typeof v === 'string' ? v.trim() : '');

async function remoteAct(what, arg, device) {
  const by = L(`pelo telemóvel "${device.name}"`, `from the phone "${device.name}"`);
  if (status().state === 'external') return { ok: false, code: 'external' };
  if (what === 'start') {
    await ipcStart();
    if (!note) note = L(`Ligado ${by}.`, `Switched on ${by}.`);
    return { ok: true };
  }
  if (what === 'stop') {
    await ipcStop();
    note = L(`Desligado ${by}.`, `Switched off ${by}.`);
    return { ok: true };
  }
  if (what === 'profile') {
    const p = profileById(arg);
    if (!p) return { ok: false, code: 'unknownProfile' };
    if (cfg.activeProfile !== arg) {
      const r = await applyConfig({ ...cfg, activeProfile: arg });
      if (!r.ok) return { ok: false, code: 'badProfile' };
    }
    if (!note) note = L(`Perfil "${p.name}" escolhido ${by}.`, `Profile "${p.name}" chosen ${by}.`);
    return { ok: true };
  }
  if (what === 'without-oc') {
    skipOc = true;
    ocState = null;
    note = null;
    await reconcile();
    return { ok: true };
  }
  return { ok: false, code: 'unknown' };
}

async function tailscaleHost() {
  if (Date.now() - tsCache.at > 30_000) tsCache = { at: Date.now(), host: await remote.tailscaleHost() };
  return tsCache.host;
}

async function remoteInfo() {
  const host = await tailscaleHost();
  const r = cfg.remote;
  return {
    settings: r,
    listening: remotePort !== null,
    error: remoteError,
    host,
    serving: host !== null && remotePort !== null ? await remote.tailscaleServing(host, r.httpsPort, r.port) : false,
    command: remote.serveCommand(r.httpsPort, r.port),
    address: host ? `https://${host}${r.httpsPort === 443 ? '' : `:${r.httpsPort}`}` : null,
    devices: devices.list(),
  };
}

function handle(name, fn) {
  const page = pathToFileURL(path.join(__dirname, 'renderer', 'index.html')).href;
  ipcMain.handle(name, (e, ...args) => {
    if (e.senderFrame?.url.split(/[?#]/)[0] !== page) throw new Error('refused');
    return fn(e, ...args);
  });
}

function registerIpc() {
  handle('remote-info', () => remoteInfo());
  handle('remote-serve', async () => {
    const r = await remote.tailscaleServe(cfg.remote.httpsPort, cfg.remote.port);
    return { ...r, info: await remoteInfo() };
  });
  handle('remote-pair', async (_e, name) => {
    const host = await tailscaleHost();
    if (host === null) return { ok: false, code: 'noTailscaleHost' };
    if (remotePort === null) return { ok: false, code: 'remoteOff' };
    const r = devices.pair(name);
    if (!r.ok) return r;
    const url = remote.pairingUrl(host, cfg.remote.httpsPort, r.token);
    return { ok: true, device: r.device, url, qr: remote.qrMatrix(url), info: await remoteInfo() };
  });
  handle('remote-revoke', async (_e, id) => {
    devices.revoke(String(id));
    return remoteInfo();
  });
  handle('init', () => ({
    config: cfg,
    coins: COINS,
    lang: lang(),
    platform: process.platform,
    version: app.getVersion(),
    view: { mode: viewState.mode, alwaysOnTop: viewState.alwaysOnTop },
    policy: thermal.POLICY,
    defaultCurve: thermal.DEFAULT_CURVE,
  }));
  handle('view', (_e, next) => applyView(next && typeof next === 'object' ? next : {}));
  handle('status', () => status());
  handle('start', async () => {
    await ipcStart();
    return status();
  });
  handle('stop', async () => {
    await ipcStop();
    return status();
  });
  handle('elevate', () => elevate());
  handle('mine-without-oc', async () => {
    skipOc = true;
    ocState = null;
    note = null;
    await reconcile();
    return status();
  });
  handle('restore-cards', async () => {
    await restoreCards();
    return status();
  });
  handle('save', (_e, next) => applyConfig(next));
  const scopeOf = (x) => (x && x.kind === 'profile' && typeof x.id === 'string' ? { kind: 'profile', id: x.id } : { kind: 'all' });
  handle('bench-plan', (_e, scope) => benchPlan(scopeOf(scope)));
  handle('bench-start', async (_e, scope, useDev) => {
    const r = await benchStart(scopeOf(scope), useDev === true);
    return { ...r, status: status() };
  });
  handle('bench-cancel', async () => {
    await benchFinish(true);
    return status();
  });
  handle('browse-miner', async () => {
    const r = await dialog.showOpenDialog(win, {
      title: L('Onde está o dori-miner?', 'Where is dori-miner?'),
      properties: ['openFile'],
      filters: process.platform === 'win32' ? [{ name: 'dori-miner', extensions: ['exe'] }] : [],
    });
    if (r.canceled) return null;
    if (admin) {

      const sure = await dialog.showMessageBox(win, {
        type: 'warning',
        title: L('Executar como administrador', 'Run as administrator'),
        message: L('O Control está a correr como administrador, e este programa também vai correr assim.', 'Control is running as administrator, and this program will run that way too.'),
        detail: `${r.filePaths[0]}\n\n${L('Use só um programa em que confie, numa pasta onde só um administrador pode escrever.', 'Use only a program you trust, in a folder only an administrator can write to.')}`,
        buttons: [L('Cancelar', 'Cancel'), L('Usar este programa', 'Use this program')],
        defaultId: 0,
        cancelId: 0,
      });
      if (sure.response !== 1) return null;
      chosenMiner = r.filePaths[0];
    }
    return r.filePaths[0];
  });
}

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on('second-instance', showWindow);
  app.whenReady().then(async () => {
    const loaded = config.loadChecked(CONFIG_FILE());
    cfg = loaded.config;
    cfgErrors = loaded.errors;
    viewState = view.load(VIEW_FILE());
    cards = new GpuControl({ stateFile: DATA('cards.json') });
    admin = await cards.isAdmin();
    devices = new remote.DeviceStore(DATA('devices.json'));
    energy = new EnergyLog(DATA('energy.json'));
    try {
      const saved = JSON.parse(fs.readFileSync(RATES_FILE(), 'utf8'));
      if (saved && typeof saved === 'object') rates = saved;
    } catch {}
    try {
      const saved = JSON.parse(fs.readFileSync(BENCH_FILE(), 'utf8'));
      if (saved && typeof saved === 'object') benchResults = saved;
    } catch {}
    miner = new Miner(DATA('miner.json'));
    miner.onExit = onMinerExit;

    const adopted = await miner.adopt(cfg.apiPort);
    if (adopted) {
      const meta = miner.meta ?? {};
      running = { profileId: profileById(meta.profileId) ? meta.profileId : cfg.activeProfile, poolIndex: meta.poolIndex ?? 0, poolSince: Date.now(), downSince: null };
      ocState = cards.dirty ? { code: 'ok', steps: [] } : null;
    } else if (cards.dirty) {
      await restoreCards();
    }
    Menu.setApplicationMenu(null);
    session.defaultSession.setPermissionRequestHandler((_wc, _permission, answer) => answer(false));
    session.defaultSession.setPermissionCheckHandler(() => false);
    registerIpc();
    await applyRemote();
    tray = new Tray(nativeImage.createFromPath(ICON));
    tray.setContextMenu(trayMenu());
    tray.on('click', showWindow);
    createWindow();
    if (cfg.startWithControl || adopted || process.argv.includes('--arm')) armed = true;
    tick();
  });
  app.on('window-all-closed', () => {

  });
  app.on('before-quit', (e) => {
    if (!quitting) {
      e.preventDefault();
      quit();
    }
  });
}
