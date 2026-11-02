const fs = require('node:fs');
const path = require('node:path');

const KEEP_DAYS = 400;

const MAX_STEP_MS = 60_000;
const SAVE_EVERY_MS = 60_000;

const dayOf = (ms) => {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
};

class EnergyLog {
  constructor(file, now = Date.now) {
    this.file = file;
    this.now = now;
    this.startedAt = now();
    this.session = { wh: 0, ms: 0 };
    this.days = this.load();
    this.lastAt = null;
    this.savedAt = 0;
  }

  load() {
    try {
      const raw = JSON.parse(fs.readFileSync(this.file, 'utf8'));
      const days = {};
      for (const [day, v] of Object.entries(raw.days ?? {})) {
        if (/^\d{4}-\d{2}-\d{2}$/.test(day) && Number.isFinite(v?.wh) && v.wh >= 0 && Number.isFinite(v?.ms) && v.ms >= 0) days[day] = { wh: v.wh, ms: v.ms };
      }
      return days;
    } catch {
      return {};
    }
  }

  save() {
    try {
      fs.mkdirSync(path.dirname(this.file), { recursive: true });
      const tmp = `${this.file}.tmp`;
      fs.writeFileSync(tmp, `${JSON.stringify({ version: 1, days: this.days })}\n`);
      fs.renameSync(tmp, this.file);
      this.savedAt = this.now();
    } catch {

    }
  }

  step(watts) {
    const now = this.now();
    const dt = this.lastAt === null ? 0 : now - this.lastAt;
    if (watts === null || !Number.isFinite(watts) || watts <= 0) {
      this.lastAt = null;
      return;
    }
    this.lastAt = now;
    if (dt <= 0 || dt > MAX_STEP_MS) return;
    const wh = (watts * dt) / 3_600_000;
    const day = dayOf(now);
    const d = this.days[day] ?? (this.days[day] = { wh: 0, ms: 0 });
    d.wh += wh;
    d.ms += dt;
    this.session.wh += wh;
    this.session.ms += dt;
    if (now - this.savedAt >= SAVE_EVERY_MS) {
      const names = Object.keys(this.days).sort();
      for (const old of names.slice(0, Math.max(0, names.length - KEEP_DAYS))) delete this.days[old];
      this.save();
    }
  }

  lastDays(n) {
    const out = { wh: 0, ms: 0 };
    const today = new Date(this.now());
    for (let i = 0; i < n; i += 1) {

      const d = this.days[dayOf(new Date(today.getFullYear(), today.getMonth(), today.getDate() - i, 12).getTime())];
      if (d) {
        out.wh += d.wh;
        out.ms += d.ms;
      }
    }
    return out;
  }

  view(kwhEur) {
    const of = (x) => ({ kwh: x.wh / 1000, hours: x.ms / 3_600_000, eur: typeof kwhEur === 'number' ? (x.wh / 1000) * kwhEur : null });
    return { since: new Date(this.startedAt).toISOString(), session: of(this.session), today: of(this.lastDays(1)), days7: of(this.lastDays(7)), days30: of(this.lastDays(30)) };
  }
}

module.exports = { EnergyLog };
