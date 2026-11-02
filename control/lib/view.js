const fs = require('node:fs');

const SIZES = {
  compact: { width: 380, height: 700, minWidth: 340, minHeight: 520 },
  window: { width: 980, height: 760, minWidth: 560, minHeight: 480 },
};

function defaults() {
  return { mode: 'compact', alwaysOnTop: false, bounds: {} };
}

function load(file) {
  try {
    const v = { ...defaults(), ...JSON.parse(fs.readFileSync(file, 'utf8').trimStart()) };
    if (!SIZES[v.mode]) v.mode = 'compact';
    v.alwaysOnTop = Boolean(v.alwaysOnTop);
    if (!v.bounds || typeof v.bounds !== 'object') v.bounds = {};
    return v;
  } catch {
    return defaults();
  }
}

function save(file, v) {
  try {
    fs.writeFileSync(file, `${JSON.stringify(v, null, 2)}\n`);
  } catch {

  }
}

function placement(v, mode, workAreas, primary) {
  const size = SIZES[mode];
  const saved = v.bounds[mode];
  const fits = (b) =>
    b &&
    [b.x, b.y, b.width, b.height].every(Number.isFinite) &&
    b.width >= size.minWidth &&
    b.height >= size.minHeight &&
    workAreas.some((a) => b.x + 60 <= a.x + a.width && b.x + b.width - 60 >= a.x && b.y >= a.y - 8 && b.y + 40 <= a.y + a.height);
  if (fits(saved)) return { ...saved };
  const height = Math.min(size.height, primary.height - 16);
  if (mode === 'compact') return { x: primary.x + primary.width - size.width - 16, y: primary.y + primary.height - height - 16, width: size.width, height };
  return { x: primary.x + Math.round((primary.width - size.width) / 2), y: primary.y + Math.round((primary.height - height) / 2), width: Math.min(size.width, primary.width), height };
}

module.exports = { SIZES, defaults, load, save, placement };
