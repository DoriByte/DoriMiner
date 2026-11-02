const STARTUP_GRACE_MS = 2 * 60_000;
const FAILOVER_MS = 2 * 60_000;
const RETURN_TO_PRIMARY_MS = 60 * 60_000;

function decide(r, poolsCount, connected, now) {
  let downSince = r.downSince;
  if (connected) downSince = null;
  else if (now - r.poolSince > STARTUP_GRACE_MS) downSince = downSince ?? now;
  let next = null;
  if (poolsCount > 1) {
    if (downSince !== null && now - downSince >= FAILOVER_MS) next = (r.poolIndex + 1) % poolsCount;
    else if (connected && r.poolIndex > 0 && now - r.poolSince >= RETURN_TO_PRIMARY_MS) next = 0;
  }
  return { downSince, next: next === r.poolIndex ? null : next };
}

module.exports = { decide, STARTUP_GRACE_MS, FAILOVER_MS, RETURN_TO_PRIMARY_MS };
