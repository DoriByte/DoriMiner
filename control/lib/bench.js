const { COINS } = require('./coins');

const WARM_MS = 45_000;
const MEASURE_MS = 60_000;

const DEV_WALLETS = {
  QTC: 'qzjcpLMMxF6D7d2vZeMwJDxdi5zRSe9xCJ1Dpmvf6Nt1zD4aM',
  PRL: 'prl1pgnhm0239cukx8wpkrze90axl887c4k62j4pnhjd0uulu4ynlj0vq020mtc',
  VELA: 'vela1qmd59xn3s9epg5trv8392vpmypmz0zkh03dz64t',
  ERG: '9gRarjaNH6e7AVWBBycVwYQk4tCKX6MQozKxiKxLzy3XnB4anvE',
};

const stockOc = () => ({ powerW: null, coreLockMHz: null, memLockMHz: null, coreOffsetMHz: null, memOffsetMHz: null, fan: { mode: 'auto', pct: 70, curve: null }, coreLimitC: null, memLimitC: null });

function supported(coin, gpus) {
  const c = COINS[coin];
  if (!c?.cards || !gpus || gpus.length === 0) return true;
  return gpus.some((g) => c.cards.test(g.name));
}

function plan(cfg, scope, gpus) {
  const walletOf = (p) => cfg.wallets.find((w) => w.id === p.walletId && w.coin === p.coin)?.address?.trim() || cfg.wallets.find((w) => w.coin === p.coin)?.address?.trim() || null;
  const item = (profile, real) => ({ coin: profile.coin, profile, profileId: real ? profile.id : null, wallet: walletOf(profile), supported: supported(profile.coin, gpus) });
  if (scope?.kind === 'profile') {
    const p = cfg.profiles.find((x) => x.id === scope.id);
    return p ? [item(p, true)] : [];
  }
  return Object.keys(COINS).map((coin) => {
    const active = cfg.profiles.find((p) => p.id === cfg.activeProfile && p.coin === coin);
    const p = active ?? cfg.profiles.find((x) => x.coin === coin);
    if (p) return item(p, true);
    return item({ id: `bench-${coin.toLowerCase()}`, name: COINS[coin].short, coin, walletId: null, pools: [`preset:${COINS[coin].defaultPool}`], worker: 'bench', oc: stockOc() }, false);
  });
}

function missingWallets(items) {
  return items.filter((x) => x.supported && !x.wallet).map((x) => x.coin);
}

function walletFor(item, useDev) {
  return item.wallet ?? (useDev ? DEV_WALLETS[item.coin] ?? null : null);
}

function median(xs) {
  const v = xs.filter((x) => typeof x === 'number' && Number.isFinite(x)).sort((a, b) => a - b);
  if (v.length === 0) return null;
  const m = v.length >> 1;
  return v.length % 2 ? v[m] : (v[m - 1] + v[m]) / 2;
}

function plannedSeconds(items, useDev) {
  return items.filter((x) => x.supported && walletFor(x, useDev)).length * Math.round((WARM_MS + MEASURE_MS) / 1000);
}

module.exports = { WARM_MS, MEASURE_MS, DEV_WALLETS, plan, missingWallets, walletFor, median, plannedSeconds, supported, stockOc };
