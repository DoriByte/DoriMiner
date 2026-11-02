const { execFile } = require('node:child_process');
const { system32 } = require('./winpaths');

const SMI = process.platform === 'win32' ? system32('nvidia-smi.exe') : 'nvidia-smi';
const FIELDS = ['index', 'name', 'temperature.gpu', 'power.draw', 'power.limit', 'fan.speed', 'clocks.sm', 'utilization.gpu', 'pci.bus'];

const num = (text) => {
  const n = Number.parseFloat(text);
  return Number.isFinite(n) ? n : null;
};

function parse(csv) {
  return csv
    .split(/\r?\n/)
    .map((line) => line.split(',').map((f) => f.trim()))
    .filter((f) => f.length === FIELDS.length)
    .map(([index, name, temp, watts, limit, fan, clock, util, bus]) => ({
      index: Number(index),
      name,
      coreC: num(temp),
      watts: num(watts),
      limitW: num(limit),
      fanPct: num(fan),
      clockMHz: num(clock),
      utilPct: num(util),

      bus: /^0x[0-9a-f]+$/i.test(bus) ? Number.parseInt(bus, 16) : null,
      memC: null,
    }));
}

function read() {
  return new Promise((resolve) => {
    execFile(SMI, [`--query-gpu=${FIELDS.join(',')}`, '--format=csv,noheader,nounits'], { timeout: 5000, windowsHide: true }, (err, stdout) => {
      resolve(err ? [] : parse(stdout));
    });
  });
}

const KNOWN_MINERS = [
  [/^dori-?miner/, 'DoriMiner'],
  [/^srbminer/, 'SRBMiner'],
  [/^peakminer/, 'PeakMiner'],
  [/^bzminer/, 'BzMiner'],
  [/^wildrig/, 'WildRig'],
  [/^rigel/, 'Rigel'],
  [/^t-?rex/, 'T-Rex'],
  [/^lolminer/, 'lolMiner'],
  [/^nbminer/, 'NBMiner'],
  [/^gminer/, 'GMiner'],
  [/^teamredminer/, 'TeamRedMiner'],
  [/^onezerominer/, 'OneZeroMiner'],
  [/^nanominer/, 'nanominer'],
  [/^xmrig/, 'XMRig'],
  [/^excavator/, 'Excavator'],
];

const BUSY_PCT = 50;

function programName(text) {
  const base = String(text).trim().split(/[\\/]/).pop().replace(/\.exe$/i, '').replace(/[\u0000-\u001f]/g, '').slice(0, 40);
  return !base || /^\[.*\]$/.test(base) || base === 'N/A' ? null : base;
}

function parseUsers(cardsCsv, appsCsv) {
  const indexOf = new Map(
    cardsCsv
      .split(/\r?\n/)
      .map((line) => line.split(',').map((f) => f.trim()))
      .filter((f) => f.length === 2)
      .map(([index, uuid]) => [uuid, Number(index)]),
  );
  const users = new Map();
  for (const line of appsCsv.split(/\r?\n/)) {
    const at = line.indexOf(',');
    if (at < 0) continue;
    const index = indexOf.get(line.slice(0, at).trim());
    if (index === undefined) continue;
    users.set(index, [...(users.get(index) ?? []), programName(line.slice(at + 1))]);
  }
  return users;
}

function usedBy(card, names = []) {
  const named = names.filter(Boolean);
  for (const name of named) {
    const known = KNOWN_MINERS.find(([re]) => re.test(name.toLowerCase()));
    if (known) return { miner: known[1] };
    if (/miner/i.test(name)) return { miner: name };
  }
  if (named.length) return { program: named[0] };
  return names.length || (card.utilPct ?? 0) >= BUSY_PCT ? { unknown: true } : null;
}

function users() {
  const smi = (query) =>
    new Promise((resolve, reject) => {
      execFile(SMI, [query, '--format=csv,noheader'], { timeout: 5000, windowsHide: true }, (err, stdout) => (err ? reject(err) : resolve(stdout)));
    });
  return Promise.all([smi('--query-gpu=index,uuid'), smi('--query-compute-apps=gpu_uuid,process_name')]).then(([c, a]) => parseUsers(c, a), () => new Map());
}

module.exports = { read, parse, parseUsers, usedBy, users };
