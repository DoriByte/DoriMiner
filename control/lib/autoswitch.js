const POLICY = { minGainFraction: 0.1, holdMs: 15 * 60_000, minDwellMs: 15 * 60_000, minGainEur: 0.02, staleMs: 15 * 60_000 };

function initial(current, now) {
  return { current, since: now, beating: {}, evaluatedAt: now };
}

function decide(state, candidates, now, policy = POLICY) {
  let s = { ...state, beating: { ...(state.beating || {}) } };
  if (s.evaluatedAt !== undefined && now - s.evaluatedAt > policy.staleMs) s.beating = {};
  s.evaluatedAt = now;
  const cur = candidates.find((c) => c.coin === s.current);
  if (!cur) return { state: { ...s, beating: {} }, decision: { kind: 'no-data', coin: s.current } };
  const beats = (c) => {
    const gain = c.net - cur.net;
    return c.coin !== cur.coin && gain >= policy.minGainEur && gain >= Math.abs(cur.net) * policy.minGainFraction;
  };
  const beating = {};
  for (const c of candidates) {
    if (!beats(c)) continue;
    beating[c.coin] = s.beating[c.coin] ?? { since: now, priceAt: c.priceAt, networkAt: c.networkAt };
  }
  s.beating = beating;
  const ahead = candidates.filter((c) => beating[c.coin]).sort((a, b) => b.net - a.net);
  if (ahead.length === 0) return { state: s, decision: { kind: 'hold', coin: cur.coin } };
  const confirmed = (c) => c.priceAt > beating[c.coin].priceAt && c.networkAt > beating[c.coin].networkAt;
  const held = (c) => now - beating[c.coin].since;
  const ready = ahead.find((c) => held(c) >= policy.holdMs && confirmed(c));
  if (ready && now - s.since >= policy.minDwellMs) {
    return { state: initial(ready.coin, now), decision: { kind: 'switch', coin: ready.coin, profileId: ready.profileId, from: cur, to: ready } };
  }
  const leader = ahead[0];
  const waitMin = Math.max(0, Math.ceil(Math.max(policy.holdMs - held(leader), policy.minDwellMs - (now - s.since)) / 60_000));
  return { state: s, decision: { kind: 'hold', coin: cur.coin, ahead: { coin: leader.coin, net: leader.net, waitMin, needsSample: !confirmed(leader) } } };
}

function bestPerCoin(options) {
  const best = new Map();
  for (const o of options) if (!best.has(o.coin) || o.net > best.get(o.coin).net) best.set(o.coin, o);
  return [...best.values()].sort((a, b) => b.net - a.net);
}

module.exports = { POLICY, initial, decide, bestPerCoin };
