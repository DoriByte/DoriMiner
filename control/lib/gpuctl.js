const { execFile } = require('node:child_process');
const fs = require('node:fs');
const { system32 } = require('./winpaths');

const SMI = process.platform === 'win32' ? system32('nvidia-smi.exe') : 'nvidia-smi';
const POWERSHELL = system32('WindowsPowerShell', 'v1.0', 'powershell.exe');

const defaultExec = (file, args, timeoutMs, env) =>
  new Promise((resolve) => {
    execFile(file, args, { timeout: timeoutMs, windowsHide: true, env: env ? { ...process.env, ...env } : process.env }, (error, stdout, stderr) => {
      const code = error === null ? 0 : typeof error.code === 'number' ? error.code : 1;
      resolve({ code, stdout: String(stdout), stderr: error !== null && !stderr ? error.message : String(stderr) });
    });
  });

const encoded = (script) => ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', Buffer.from(script, 'utf16le').toString('base64')];
const firstLine = (text, code) => text.trim().split('\n')[0] || `code ${code}`;
const validIndex = (i) => Number.isInteger(i) && i >= 0 && i <= 99;

const validOffset = (v) => Number.isInteger(v) && Math.abs(v) <= 20000;
const noAdmin = (text) => /administrator|permission|privilege|codigo 4\b/i.test(text);

const NVML_CLOCK_SCRIPT = `$ErrorActionPreference = 'Stop'
if (-not $env:DORI_OC_INDEX) { exit 3 }
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
[StructLayout(LayoutKind.Sequential)]
public struct DoriClockOffset { public uint version; public int type; public int pstate; public int offset; public int min; public int max; }
public static class DoriNvmlOc {
  [DllImport("nvml.dll")] public static extern int nvmlInit_v2();
  [DllImport("nvml.dll")] public static extern int nvmlDeviceGetHandleByIndex_v2(uint i, out IntPtr h);
  [DllImport("nvml.dll")] public static extern int nvmlDeviceGetClockOffsets(IntPtr h, ref DoriClockOffset info);
  [DllImport("nvml.dll")] public static extern int nvmlDeviceSetClockOffsets(IntPtr h, ref DoriClockOffset info);
}
"@
function Check($rc, $what) { if ($rc -ne 0) { [Console]::Error.WriteLine("NVML falhou em $what (codigo $rc)"); exit 4 } }
function ReadClock($h, $type) { $i = New-Object DoriClockOffset; $i.version = 0x01000018; $i.type = $type; $i.pstate = 0; Check ([DoriNvmlOc]::nvmlDeviceGetClockOffsets($h, [ref]$i)) "ler o relogio $type"; return $i }
function WriteClock($h, $type, $value) {
  $i = ReadClock $h $type
  if ($value -lt $i.min -or $value -gt $i.max) { [Console]::Error.WriteLine("offset fora dos limites desta placa ($($i.min) a $($i.max) MHz)"); exit 3 }
  $i.offset = $value
  Check ([DoriNvmlOc]::nvmlDeviceSetClockOffsets($h, [ref]$i)) "mudar o relogio $type"
}
$index = [uint32]$env:DORI_OC_INDEX
Check ([DoriNvmlOc]::nvmlInit_v2()) 'init'
$h = [IntPtr]::Zero
Check ([DoriNvmlOc]::nvmlDeviceGetHandleByIndex_v2($index, [ref]$h)) "GPU $index"
if ($env:DORI_OC_CORE) { WriteClock $h 0 ([int]$env:DORI_OC_CORE) }
if ($env:DORI_OC_MEM) { WriteClock $h 2 ([int]$env:DORI_OC_MEM) }
$c = ReadClock $h 0; $m = ReadClock $h 2
[Console]::Out.WriteLine("core=$($c.offset),$($c.min),$($c.max);mem=$($m.offset),$($m.min),$($m.max)")
exit 0
`;

const NVML_FAN_SCRIPT = `$ErrorActionPreference = 'Stop'
if (-not $env:DORI_OC_INDEX -or -not $env:DORI_OC_FAN) { exit 3 }
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class DoriNvml {
  [DllImport("nvml.dll")] public static extern int nvmlInit_v2();
  [DllImport("nvml.dll")] public static extern int nvmlDeviceGetHandleByIndex_v2(uint i, out IntPtr h);
  [DllImport("nvml.dll")] public static extern int nvmlDeviceGetNumFans(IntPtr h, out uint n);
  [DllImport("nvml.dll")] public static extern int nvmlDeviceGetMinMaxFanSpeed(IntPtr h, out uint lo, out uint hi);
  [DllImport("nvml.dll")] public static extern int nvmlDeviceSetFanSpeed_v2(IntPtr h, uint f, uint s);
  [DllImport("nvml.dll")] public static extern int nvmlDeviceSetDefaultFanSpeed_v2(IntPtr h, uint f);
}
"@
function Check($rc, $what) { if ($rc -ne 0) { [Console]::Error.WriteLine("NVML falhou em $what (codigo $rc)"); exit 4 } }
$index = [uint32]$env:DORI_OC_INDEX
$target = [int]$env:DORI_OC_FAN
Check ([DoriNvml]::nvmlInit_v2()) 'init'
$h = [IntPtr]::Zero
Check ([DoriNvml]::nvmlDeviceGetHandleByIndex_v2($index, [ref]$h)) "GPU $index"
$n = [uint32]0
Check ([DoriNvml]::nvmlDeviceGetNumFans($h, [ref]$n)) 'numero de ventoinhas'
if ($target -lt 0) {
  for ($f = 0; $f -lt $n; $f++) { Check ([DoriNvml]::nvmlDeviceSetDefaultFanSpeed_v2($h, [uint32]$f)) "ventoinha $f (automatico)" }
} else {
  $lo = [uint32]0; $hi = [uint32]0
  Check ([DoriNvml]::nvmlDeviceGetMinMaxFanSpeed($h, [ref]$lo, [ref]$hi)) 'limites das ventoinhas'
  if ($target -lt $lo -or $target -gt $hi) { [Console]::Error.WriteLine("percentagem fora dos limites desta placa ($lo a $hi %)"); exit 3 }
  for ($f = 0; $f -lt $n; $f++) { Check ([DoriNvml]::nvmlDeviceSetFanSpeed_v2($h, [uint32]$f, [uint32]$target)) "ventoinha $f" }
}
exit 0
`;

const scriptEnv = (index, core, mem, fan) => ({ DORI_OC_INDEX: String(index), DORI_OC_CORE: core === null ? '' : String(core), DORI_OC_MEM: mem === null ? '' : String(mem), DORI_OC_FAN: fan === null ? '' : String(fan) });

function clockCall(index, core, mem) {
  if (!validIndex(index) || !(core === null || validOffset(core)) || !(mem === null || validOffset(mem))) throw new RangeError('bad card index or clock offset');
  return { script: NVML_CLOCK_SCRIPT, env: scriptEnv(index, core, mem, null) };
}

function fanCall(index, mode) {
  if (!validIndex(index) || !(mode === 'auto' || (Number.isInteger(mode) && mode >= 0 && mode <= 100))) throw new RangeError('bad card index or fan percentage');
  return { script: NVML_FAN_SCRIPT, env: scriptEnv(index, null, null, mode === 'auto' ? -1 : mode) };
}

function elevateCall(execPath, appPath, arm) {
  const command = "$a = @(); if ($env:DORI_APP) { $a += $env:DORI_APP }; if ($env:DORI_ARM) { $a += '--arm' }; if ($a.Count -gt 0) { Start-Process -FilePath $env:DORI_EXE -Verb RunAs -ArgumentList $a } else { Start-Process -FilePath $env:DORI_EXE -Verb RunAs }";
  return { file: POWERSHELL, args: ['-NoProfile', '-NonInteractive', '-Command', command], env: { DORI_EXE: execPath, DORI_APP: appPath ?? '', DORI_ARM: arm ? '1' : '' } };
}

const NVAPI_THERMAL_SCRIPT = `
$ErrorActionPreference = 'Stop'
$src = @"
using System; using System.Text; using System.Globalization; using System.Runtime.InteropServices;
public static class DoriNvapi {
  [DllImport("nvapi64.dll", EntryPoint = "nvapi_QueryInterface", CallingConvention = CallingConvention.Cdecl)] static extern IntPtr Q(uint id);
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate int InitD();
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate int EnumD([Out] IntPtr[] handles, out int count);
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate int ThermD(IntPtr gpu, ref Therm t);
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate int BusD(IntPtr gpu, out int bus);
  [StructLayout(LayoutKind.Sequential)] struct Therm {
    public uint version; public uint mask;
    [MarshalAs(UnmanagedType.ByValArray, SizeConst = 8)] public uint[] reserved;
    [MarshalAs(UnmanagedType.ByValArray, SizeConst = 32)] public int[] temps;
  }
  static Delegate F(uint id, Type t) { IntPtr p = Q(id); return p == IntPtr.Zero ? null : Marshal.GetDelegateForFunctionPointer(p, t); }
  static Therm New(uint mask) { Therm t = new Therm(); t.version = 0x000200A8; t.mask = mask; t.reserved = new uint[8]; t.temps = new int[32]; return t; }
  public static string Read() {
    StringBuilder sb = new StringBuilder();
    InitD init = (InitD)F(0x0150E828, typeof(InitD)); if (init == null || init() != 0) return "";
    EnumD en = (EnumD)F(0xE5AC921F, typeof(EnumD)); IntPtr[] h = new IntPtr[64]; int n; if (en == null || en(h, out n) != 0) return "";
    BusD bus = (BusD)F(0x1BE0B8E5, typeof(BusD)); ThermD th = (ThermD)F(0x65FE3AAD, typeof(ThermD)); if (bus == null || th == null) return "";
    for (int g = 0; g < n; g++) {
      int b; if (bus(h[g], out b) != 0) continue;
      uint mask = 0;
      for (int bit = 0; bit < 32; bit++) { Therm p = New(1u << bit); if (th(h[g], ref p) == 0) mask |= 1u << bit; else break; }
      if (mask == 0) continue;
      Therm t = New(mask); if (th(h[g], ref t) != 0) continue;
      sb.Append("bus=" + b + ";t=");
      bool first = true;
      for (int i = 0; i < 32; i++) if ((mask & (1u << i)) != 0) { sb.Append((first ? "" : ",") + i + ":" + (t.temps[i] / 256.0).ToString("0.0", CultureInfo.InvariantCulture)); first = false; }
      sb.AppendLine();
    }
    return sb.ToString();
  }
}
"@
Add-Type -TypeDefinition $src
Write-Output ([DoriNvapi]::Read())
`;

function parseClockLine(text) {
  const m = /core=(-?\d+),(-?\d+),(-?\d+);mem=(-?\d+),(-?\d+),(-?\d+)/.exec(text);
  if (!m) return null;
  const n = m.slice(1).map(Number);
  return { core: { offset: n[0], min: n[1], max: n[2] }, mem: { offset: n[3], min: n[4], max: n[5] } };
}

function parseNvapiThermal(text) {
  const out = [];
  for (const line of text.split(/\r?\n/)) {
    const m = /^bus=(\d+);t=([0-9:.,]+)$/.exec(line.trim());
    if (!m) continue;
    const temps = new Map();
    for (const pair of m[2].split(',')) {
      const [i, v] = pair.split(':').map(Number);
      if (Number.isInteger(i) && Number.isFinite(v)) temps.set(i, v);
    }
    out.push({ bus: Number(m[1]), temps });
  }
  return out;
}

function memoryTempOf(temps) {
  const valid = (i) => {
    const v = temps.get(i);
    return v !== undefined && v > 0 && v < 150 ? v : null;
  };
  if (valid(0) !== null) return valid(9);
  if (valid(1) !== null && valid(2) !== null) return valid(2);
  return null;
}

function parseMemSteps(stdout) {
  const steps = new Set();
  for (const line of stdout.split('\n')) {
    const n = Number(line.replace(/MHz/i, '').trim());
    if (Number.isInteger(n) && n >= 100 && n <= 20000) steps.add(n);
  }
  return [...steps].sort((a, b) => a - b);
}

function checkOc(oc) {
  if (!oc) return null;
  if (typeof oc !== 'object') return 'badOc';
  const int = (v, lo, hi) => v === null || v === undefined || (Number.isInteger(v) && v >= lo && v <= hi);
  if (!int(oc.powerW, 50, 1000)) return 'badPowerW';
  if (!int(oc.coreLockMHz, 300, 4000)) return 'badCoreLock';
  if (!int(oc.memLockMHz, 100, 20000)) return 'badMemLock';
  if (!int(oc.coreOffsetMHz, -1000, 1000)) return 'badCoreOffset';
  if (!int(oc.memOffsetMHz, -3000, 6000)) return 'badMemOffset';
  const fan = oc.fan || { mode: 'auto' };
  if (typeof fan !== 'object') return 'badFan';
  if (!['auto', 'fixed', 'curve'].includes(fan.mode)) return 'badFan';
  if (fan.mode === 'fixed' && !int(fan.pct, 30, 100)) return 'badFanPct';
  if (fan.mode === 'curve') {
    const pts = fan.curve;
    if (!Array.isArray(pts) || pts.length < 2 || pts.length > 8) return 'badCurve';
    if (!pts.every((p) => p && Number.isInteger(p.tempC) && Number.isInteger(p.pct) && int(p.tempC, 20, 95) && int(p.pct, 30, 100))) return 'badCurve';
  }
  if (!int(oc.coreLimitC, 60, 90) || !int(oc.memLimitC, 70, 100)) return 'badLimit';
  return null;
}

const isMap = (x) => x !== null && typeof x === 'object' && !Array.isArray(x);
const indexKey = (k) => /^\d{1,2}$/.test(k);

function validState(s) {
  if (!isMap(s) || !isMap(s.limits) || !isMap(s.offsets)) return false;
  if (![s.coreLocked, s.memLocked, s.fans].every((l) => Array.isArray(l) && l.every(validIndex))) return false;
  if (!Object.entries(s.limits).every(([k, w]) => indexKey(k) && Number.isInteger(w) && w >= 1 && w <= 2000)) return false;
  return Object.entries(s.offsets).every(([k, o]) => indexKey(k) && isMap(o) && validOffset(o.core) && validOffset(o.mem));
}

const emptyState = () => ({ limits: {}, offsets: {}, coreLocked: [], memLocked: [], fans: [] });

function changesCards(oc) {
  if (!oc) return false;
  return [oc.powerW, oc.coreLockMHz, oc.memLockMHz, oc.coreOffsetMHz, oc.memOffsetMHz].some((v) => v !== null && v !== undefined) || (oc.fan && oc.fan.mode !== 'auto');
}

class GpuControl {

  constructor({ stateFile, exec = defaultExec, platform = process.platform }) {
    this.stateFile = stateFile;
    this.exec = exec;
    this.platform = platform;

    this.stateRejected = false;
    this.state = this.loadState();
    this.fanNow = new Map();
  }

  loadState() {
    let text;
    try {
      text = fs.readFileSync(this.stateFile, 'utf8');
    } catch {
      return emptyState();
    }
    try {
      const saved = JSON.parse(text);
      if (validState(saved)) return saved;
    } catch {}
    this.stateRejected = true;
    return emptyState();
  }

  saveState() {
    try {
      fs.writeFileSync(this.stateFile, JSON.stringify(this.state));
    } catch {

    }
  }

  get dirty() {
    const s = this.state;
    return Object.keys(s.limits).length + Object.keys(s.offsets).length + s.coreLocked.length + s.memLocked.length + s.fans.length > 0;
  }

  async isAdmin() {
    if (this.platform !== 'win32') return typeof process.getuid === 'function' && process.getuid() === 0;
    return (await this.exec(system32('net.exe'), ['session'], 10_000)).code === 0;
  }

  smi(args, timeout = 20_000) {
    return this.exec(SMI, args, timeout);
  }

  async ps(script, env, timeout = 30_000) {
    return this.exec(POWERSHELL, encoded(script), timeout, env);
  }

  async readClocks(index) {
    if (!validIndex(index) || this.platform !== 'win32') return null;
    const { script, env } = clockCall(index, null, null);
    const r = await this.ps(script, env);
    return r.code === 0 ? parseClockLine(r.stdout) : null;
  }

  async writeClocks(index, core, mem) {
    let call;
    try {
      call = clockCall(index, core, mem);
    } catch {
      return { ok: false, code: 'badValue' };
    }
    const r = await this.ps(call.script, call.env);
    if (r.code === 0) return { ok: true };
    const text = `${r.stdout} ${r.stderr}`;
    return { ok: false, code: noAdmin(text) ? 'noAdmin' : 'clockFailed', detail: firstLine(text, r.code) };
  }

  async readMemSteps(index) {
    if (!validIndex(index)) return [];
    const r = await this.smi(['-i', String(index), '--query-supported-clocks=mem', '--format=csv,noheader,nounits'], 15_000);
    return r.code === 0 ? parseMemSteps(r.stdout) : [];
  }

  async readLimits(index) {
    if (!validIndex(index)) return null;
    const r = await this.smi(['-i', String(index), '--query-gpu=power.limit,power.default_limit,power.min_limit,power.max_limit', '--format=csv,noheader,nounits'], 10_000);
    if (r.code !== 0) return null;
    const [limit, def, min, max] = r.stdout.trim().split('\n')[0].split(',').map((v) => Math.round(Number(v)));
    return [limit, def, min, max].every(Number.isFinite) ? { limit, def, min, max } : null;
  }

  async smiStep(args) {
    const r = await this.smi(args);
    if (r.code === 0) return { ok: true };
    const text = `${r.stdout} ${r.stderr}`;
    return { ok: false, code: noAdmin(text) ? 'noAdmin' : /not supported|not recognized/i.test(text) ? 'notSupported' : 'smiFailed', detail: firstLine(text, r.code) };
  }

  async apply(indexes, oc) {
    if (this.platform !== 'win32') return { ok: false, code: 'notSupported' };
    const bad = checkOc(oc);
    if (bad) return { ok: false, code: bad };
    if (!changesCards(oc)) return { ok: true, steps: [] };
    if (!Array.isArray(indexes) || !indexes.every(validIndex)) return { ok: false, code: 'badIndex' };
    const wantClocks = oc.coreOffsetMHz != null || oc.memOffsetMHz != null;
    const before = new Map();
    for (const i of indexes) {
      if (oc.powerW != null) {
        const lim = await this.readLimits(i);
        if (!lim) return { ok: false, code: 'readFailed' };
        if (oc.powerW < lim.min || oc.powerW > lim.max) return { ok: false, code: 'powerOutOfRange', detail: `${lim.min}-${lim.max} W` };
        before.set(`p${i}`, lim.limit);
      }
      if (wantClocks) {
        const now = await this.readClocks(i);
        if (!now) return { ok: false, code: 'readFailed' };
        if (oc.coreOffsetMHz != null && (oc.coreOffsetMHz < now.core.min || oc.coreOffsetMHz > now.core.max)) return { ok: false, code: 'coreOffsetOutOfRange', detail: `${now.core.min}..${now.core.max}` };
        if (oc.memOffsetMHz != null && (oc.memOffsetMHz < now.mem.min || oc.memOffsetMHz > now.mem.max)) return { ok: false, code: 'memOffsetOutOfRange', detail: `${now.mem.min}..${now.mem.max}` };
        before.set(`c${i}`, now);
      }
      if (oc.memLockMHz != null) {
        const steps = await this.readMemSteps(i);
        if (steps.length > 0 && !steps.includes(oc.memLockMHz)) return { ok: false, code: 'memStep', detail: steps.join(', ') };
      }
    }
    const done = [];
    const fail = async (r) => {
      await this.restore();
      return { ...r, ok: false, steps: done };
    };
    for (const i of indexes) {
      const key = String(i);
      if (oc.powerW != null) {
        if (this.state.limits[key] === undefined) this.state.limits[key] = before.get(`p${i}`);
        this.saveState();
        const r = await this.smiStep(['-i', key, '-pl', String(oc.powerW)]);
        if (!r.ok) return fail(r);
        done.push(`power:${i}`);
      }
      if (wantClocks) {
        const now = before.get(`c${i}`);
        if (this.state.offsets[key] === undefined) this.state.offsets[key] = { core: now.core.offset, mem: now.mem.offset };
        this.saveState();
        const r = await this.writeClocks(i, oc.coreOffsetMHz ?? null, oc.memOffsetMHz ?? null);
        if (!r.ok) return fail(r);
        done.push(`offsets:${i}`);
      }
      if (oc.memLockMHz != null) {
        if (!this.state.memLocked.includes(i)) this.state.memLocked.push(i);
        this.saveState();
        const r = await this.smiStep(['-i', key, '-lmc', String(oc.memLockMHz)]);
        if (r.ok) done.push(`memLock:${i}`);
        else {

          this.state.memLocked = this.state.memLocked.filter((x) => x !== i);
          this.saveState();
          done.push(`memLockRefused:${i}`);
          if (wantClocks) {
            const again = await this.writeClocks(i, oc.coreOffsetMHz ?? null, oc.memOffsetMHz ?? null);
            if (!again.ok) return fail(again);
          }
        }
      }
      if (oc.coreLockMHz != null) {
        if (!this.state.coreLocked.includes(i)) this.state.coreLocked.push(i);
        this.saveState();
        const r = await this.smiStep(['-i', key, '-lgc', `${oc.coreLockMHz},${oc.coreLockMHz}`]);
        if (!r.ok) return fail(r);
        done.push(`coreLock:${i}`);
      }
    }
    return { ok: true, steps: done };
  }

  async setFans(indexes, mode) {
    if (this.platform !== 'win32') return { ok: false, code: 'notSupported' };
    if (mode !== 'auto' && (!Number.isInteger(mode) || mode < 0 || mode > 100)) return { ok: false, code: 'badFanPct' };
    if (!Array.isArray(indexes) || !indexes.every(validIndex)) return { ok: false, code: 'badIndex' };
    for (const i of indexes) {
      if (this.fanNow.get(i) === mode) continue;
      if (mode !== 'auto' && !this.state.fans.includes(i)) {
        this.state.fans.push(i);
        this.saveState();
      }
      const call = fanCall(i, mode);
      const r = await this.ps(call.script, call.env);
      if (r.code !== 0) {
        const text = `${r.stdout} ${r.stderr}`;
        return { ok: false, code: noAdmin(text) ? 'noAdmin' : 'fanFailed', detail: firstLine(text, r.code) };
      }
      this.fanNow.set(i, mode);
      if (mode === 'auto') {
        this.state.fans = this.state.fans.filter((x) => x !== i);
        this.saveState();
      }
    }
    return { ok: true };
  }

  async restore() {
    if (this.platform !== 'win32' || !this.dirty) return { ok: true, failed: [] };
    const s = this.state;
    if (!validState(s)) return { ok: false, failed: [{ what: 'state', code: 'badState' }] };
    const failed = [];
    const step = (what, r) => {
      if (!r.ok) failed.push({ what, code: r.code, detail: r.detail });
      return r.ok;
    };
    for (const i of [...s.coreLocked]) if (step(`coreLock:${i}`, await this.smiStep(['-i', String(i), '-rgc']))) s.coreLocked = s.coreLocked.filter((x) => x !== i);
    for (const i of [...s.memLocked]) if (step(`memLock:${i}`, await this.smiStep(['-i', String(i), '-rmc']))) s.memLocked = s.memLocked.filter((x) => x !== i);
    for (const [key, o] of Object.entries(s.offsets)) if (step(`offsets:${key}`, await this.writeClocks(Number(key), o.core, o.mem))) delete s.offsets[key];
    for (const [key, w] of Object.entries(s.limits)) if (step(`power:${key}`, await this.smiStep(['-i', key, '-pl', String(w)]))) delete s.limits[key];
    for (const i of [...s.fans]) {
      this.fanNow.delete(i);
      step(`fans:${i}`, await this.setFans([i], 'auto'));
    }
    this.saveState();
    return { ok: failed.length === 0, failed };
  }

  async readMemoryTemps() {
    const out = new Map();
    if (this.platform !== 'win32') return out;
    const r = await this.ps(NVAPI_THERMAL_SCRIPT);
    if (r.code !== 0) return out;
    for (const gpu of parseNvapiThermal(r.stdout)) {
      const t = memoryTempOf(gpu.temps);
      if (t !== null) out.set(gpu.bus, Math.round(t));
    }
    return out;
  }
}

module.exports = { GpuControl, checkOc, changesCards, parseClockLine, parseNvapiThermal, memoryTempOf, parseMemSteps, clockCall, fanCall, elevateCall, validState, NVML_CLOCK_SCRIPT, NVML_FAN_SCRIPT };
