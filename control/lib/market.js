const COINGECKO = { PRL: 'pearl-2', QTC: 'quantus', ERG: 'ergo' };

const WHATTOMINE = { PRL: { id: 469, tag: 'PRL' }, QTC: { id: 473, tag: 'QUANTUS' }, ERG: { id: 340, tag: 'ERG' } };
const EVERY_MS = 30 * 60_000;
const MAX_AGE_MS = 2 * 60 * 60_000;

const positive = (v) => {
  const n = typeof v === 'string' ? Number(v) : v;
  return typeof n === 'number' && Number.isFinite(n) && n > 0 ? n : null;
};

function plausible(value, last, now) {
  if (!last || now - last.at > 12 * 3_600_000 || !(last.value > 0)) return true;
  const ratio = value / last.value;
  return ratio <= 5 && ratio >= 0.2;
}

function parsePrices(json, now) {
  const out = {};
  for (const [coin, id] of Object.entries(COINGECKO)) {
    const eur = positive(json?.[id]?.eur);
    if (eur !== null) out[coin] = { eur, usd: positive(json?.[id]?.usd), at: now };
  }
  return out;
}

function parseNetwork(coin, json, now) {
  const w = WHATTOMINE[coin];
  if (!w || !json || json.tag !== w.tag || json.lagging === true) return null;
  const networkHps = positive(json.nethash);
  const blockTimeS = positive(json.block_time);
  const blockReward = positive(json.block_reward);
  const timestamp = positive(json.timestamp);
  if (networkHps === null || blockTimeS === null || blockReward === null || timestamp === null) return null;
  const at = timestamp * 1000;
  if (at > now + 10 * 60_000 || at < now - 24 * 3_600_000) return null;
  return { networkHps, blockTimeS, blockReward, at };
}

function estimate(price, network, { hps, watts }, { feeFraction = 0, kwhEur = null } = {}) {
  if (!price || !network || !(hps > 0)) return null;
  const share = Math.min(hps / network.networkHps, 1);
  const revenue = share * (86_400 / network.blockTimeS) * network.blockReward * price.eur * (1 - Math.min(Math.max(feeFraction, 0), 1));
  const energy = kwhEur > 0 && watts > 0 ? (watts / 1000) * 24 * kwhEur : 0;
  return { revenue, energy, net: revenue - energy, dataAt: Math.min(price.at, network.at), priceAt: price.at, networkAt: network.at };
}

class Market {
  constructor(fetchImpl = globalThis.fetch, now = Date.now) {
    this.fetch = fetchImpl;
    this.now = now;
    this.prices = {};
    this.networks = {};
    this.lastTry = 0;
  }

  async get(url) {
    const r = await this.fetch(url, { headers: { Accept: 'application/json', 'User-Agent': 'DoriMiner' }, redirect: 'error', signal: AbortSignal.timeout(15_000) });
    if (!r.ok) throw new Error(`${r.status}`);
    return r.json();
  }

  usdPerEur() {
    for (const p of Object.values(this.prices)) if (p.usd) return p.usd / p.eur;
    return null;
  }

  async refresh(force = false) {
    const now = this.now();
    if (!force && now - this.lastTry < EVERY_MS) return;
    this.lastTry = now;
    try {
      const fresh = parsePrices(await this.get(`https://api.coingecko.com/api/v3/simple/price?ids=${Object.values(COINGECKO).join(',')}&vs_currencies=eur,usd`), now);
      for (const [coin, p] of Object.entries(fresh)) {
        const last = this.prices[coin];
        if (plausible(p.eur, last && { value: last.eur, at: last.at }, now)) this.prices[coin] = p;
      }
    } catch {}
    for (const [coin, { id }] of Object.entries(WHATTOMINE)) {
      try {
        const n = parseNetwork(coin, await this.get(`https://whattomine.com/coins/${id}.json`), now);
        const last = this.networks[coin];
        if (n && plausible(n.networkHps, last && { value: last.networkHps, at: last.at }, now)) this.networks[coin] = n;
      } catch {}
    }
  }

  of(coin) {
    const now = this.now();
    const p = this.prices[coin];
    const n = this.networks[coin];
    if (!p || !n || now - p.at > MAX_AGE_MS || now - n.at > MAX_AGE_MS) return null;
    return { price: p, network: n };
  }
}

module.exports = { Market, estimate, parsePrices, parseNetwork, plausible, COINGECKO, WHATTOMINE, EVERY_MS, MAX_AGE_MS };
