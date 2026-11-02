const POLICY = {
  coreLimitC: 85,

  memoryLimitC: 95,

  holdMs: 60_000,

  cooldownMs: 5 * 60_000,

  resumeMarginC: 10,

  memoryFanOffsetC: 20,

  blindWarnMs: 15_000,

  blindStopMs: 60_000,
};

const DEFAULT_CURVE = [
  { tempC: 50, pct: 40 },
  { tempC: 60, pct: 60 },
  { tempC: 70, pct: 80 },
  { tempC: 80, pct: 100 },
];

function fanTempOf(coreC, memoryC) {
  if (memoryC === null || memoryC === undefined) return coreC ?? null;
  const asCore = memoryC - POLICY.memoryFanOffsetC;
  return coreC === null || coreC === undefined ? asCore : Math.max(coreC, asCore);
}

function curvePct(points, tempC) {
  if (tempC === null || tempC === undefined || points.length === 0) return null;
  const p = [...points].sort((a, b) => a.tempC - b.tempC);
  if (tempC <= p[0].tempC) return p[0].pct;
  for (let i = 1; i < p.length; i += 1) {
    if (tempC <= p[i].tempC) {
      const a = p[i - 1];
      const b = p[i];
      return Math.round(a.pct + ((b.pct - a.pct) * (tempC - a.tempC)) / (b.tempC - a.tempC));
    }
  }
  return p[p.length - 1].pct;
}

function rescue(active, core, memory) {
  const near = (t, margin) => t.tempC !== null && t.tempC !== undefined && t.tempC >= t.limitC - margin;
  if (near(core, 5) || near(memory, 5)) return true;
  return active && (near(core, 10) || near(memory, 10));
}

function memoryAssist(active, coreC, memory) {
  if (memory.tempC === null || memory.tempC === undefined) return null;
  const gap = memory.limitC - memory.tempC;
  if (gap > (active ? 25 : 20)) return null;
  const steps = [
    [20, 60],
    [15, 70],
    [10, 85],
    [5, 100],
  ];
  let pct = 60;
  if (gap <= 5) pct = 100;
  else if (gap < 20) {
    for (let i = 1; i < steps.length; i += 1) {
      const [fromGap, fromPct] = steps[i - 1];
      const [toGap, toPct] = steps[i];
      if (gap >= toGap) {
        pct = fromPct + ((toPct - fromPct) * (fromGap - gap)) / (fromGap - toGap);
        break;
      }
    }
  }
  return Math.round(Math.max(pct, curvePct(DEFAULT_CURVE, coreC) ?? 0));
}

function guard(state, now, core, memory) {
  const coreHot = core.tempC !== null && core.tempC !== undefined && core.tempC >= core.limitC;
  const memHot = memory.tempC !== null && memory.tempC !== undefined && memory.tempC >= memory.limitC;
  const next = { ...state };
  let stop = false;
  let reason = null;
  if (next.stoppedAt) {

    const known = (t) => t.tempC !== null && t.tempC !== undefined;
    const cool =
      known(core) &&
      core.tempC <= core.limitC - POLICY.resumeMarginC &&
      (known(memory) ? memory.tempC <= memory.limitC - POLICY.resumeMarginC : next.stoppedPart !== 'memory');
    const waited = now - next.stoppedAt >= POLICY.cooldownMs;
    return { state: next, stop: false, mayResume: cool && waited, reason: null };
  }
  if (coreHot || memHot) {
    next.hotSince = next.hotSince ?? now;
    next.hotPart = coreHot ? 'core' : 'memory';
    next.hotC = coreHot ? core.tempC : memory.tempC;
  } else {

    const was = next.hotPart === 'memory' ? memory : core;
    if (next.hotSince != null && was.tempC !== null && was.tempC !== undefined) next.hotSince = null;
    if (next.hotSince == null) {
      next.hotSince = null;
      next.hotPart = null;
    }
  }
  if (next.hotSince != null && now - next.hotSince >= POLICY.holdMs) {
    stop = true;
    reason = { part: next.hotPart, tempC: next.hotC, limitC: next.hotPart === 'core' ? core.limitC : memory.limitC };
    next.stoppedAt = now;
    next.stoppedPart = next.hotPart;
    next.hotSince = null;
  }
  return { state: next, stop, mayResume: false, reason };
}

function blindness(state, now, seesAll) {
  if (seesAll) return { state: { blindSince: null }, warn: false, stop: false };
  const blindSince = state.blindSince ?? now;
  const gone = now - blindSince;
  return { state: { blindSince }, warn: gone >= POLICY.blindWarnMs, stop: gone >= POLICY.blindStopMs };
}

module.exports = { blindness, POLICY, DEFAULT_CURVE, fanTempOf, curvePct, rescue, memoryAssist, guard };
