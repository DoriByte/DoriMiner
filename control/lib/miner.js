const { spawn, execFile } = require('node:child_process');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const { system32 } = require('./winpaths');

const EXE = process.platform === 'win32' ? 'dori-miner.exe' : 'dori-miner';

const CLEAN_EXIT = new Set([0, 0xc000013a, -1073741510]);
const LOG_LINES = 300;

const SAFE_ARG = /^[A-Za-z0-9_+.:/,-]+$/;

function candidates(controlDir) {
  const up = path.resolve(controlDir, '..');
  return [
    path.join(up, '..', 'miner', EXE),
    path.join(up, EXE),
    path.join(up, '..', EXE),
    path.join(up, 'target', 'release', process.platform === 'win32' ? 'dori-miner.exe' : 'dori-miner'),
  ];
}

function find(configured, controlDir) {
  if (configured) return fs.existsSync(configured) ? configured : null;
  return candidates(controlDir).find((f) => fs.existsSync(f)) ?? null;
}

function resolveMiner({ configured, controlDir, elevated, chosen = null, packaged = false, exists = fs.existsSync }) {
  if (!elevated) return { path: find(configured, controlDir), ignored: false };
  const same = (a, b) => {
    const norm = (p) => (process.platform === 'win32' ? path.resolve(p).toLowerCase() : path.resolve(p));
    return norm(a) === norm(b);
  };
  const all = candidates(controlDir);
  const own = packaged ? [all[0]] : all;
  const first = own.find((f) => exists(f)) ?? null;
  if (!configured) return { path: first, ignored: false };
  if (((chosen && same(chosen, configured)) || own.some((f) => same(f, configured))) && exists(configured)) return { path: configured, ignored: false };
  return { path: first, ignored: true };
}

function windowLine(exe, args, codeFile = null, cmdExe = null) {
  if (/[%"\u0000-\u001f]/.test(exe)) throw new Error('the miner path cannot contain % or "');

  if (!codeFile || !cmdExe || CODE_UNSAFE.test(codeFile) || CODE_UNSAFE.test(cmdExe) || CODE_UNSAFE.test(exe) || CODE_UNSAFE.test(path.win32.dirname(exe))) {
    return ['start', '"DoriMiner"', '/D', `"${path.win32.dirname(exe)}"`, '/wait', `"${exe}"`, ...args].join(' ');
  }

  return ['start', '"DoriMiner"', '/D', `"${path.win32.dirname(exe)}"`, '/wait', `"${cmdExe}"`, '/d', '/v:off', '/c',
    `""${exe}" ${args.join(' ')} & call echo %^errorlevel%>"${codeFile}""`].join(' ');
}

const CODE_UNSAFE = /[%"&^<>|!\u0000-\u001f]/;

function summary(port, timeoutMs = 1500) {
  return new Promise((resolve) => {
    const req = http.get({ host: '127.0.0.1', port, path: '/summary', timeout: timeoutMs }, (res) => {
      let body = '';
      res.setEncoding('utf8');
      res.on('data', (d) => {
        body += d;
      });
      res.on('end', () => {
        try {
          resolve(res.statusCode === 200 ? JSON.parse(body) : null);
        } catch {
          resolve(null);
        }
      });
    });
    req.on('timeout', () => req.destroy());
    req.on('error', () => resolve(null));
  });
}

const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === 'EPERM';
  }
};

function imageName(pid) {
  return new Promise((resolve) => {
    if (process.platform !== 'win32') {
      fs.readFile(`/proc/${pid}/comm`, 'utf8', (err, text) => resolve(err ? null : text.trim()));
      return;
    }
    execFile(system32('tasklist.exe'), ['/FI', `PID eq ${pid}`, '/FO', 'CSV', '/NH'], { windowsHide: true }, (err, out) => {
      const m = !err && /^"([^"]+)","(\d+)"/m.exec(out);
      resolve(m && Number(m[2]) === pid ? m[1] : null);
    });
  });
}

function descendants(parent) {
  return new Promise((resolve) => {
    if (process.platform !== 'win32' || !Number.isInteger(parent)) return resolve([]);
    const ps = system32('WindowsPowerShell', 'v1.0', 'powershell.exe');
    const cmd = 'Get-CimInstance Win32_Process | ForEach-Object { "$($_.ProcessId)|$($_.ParentProcessId)|$($_.Name)" }';
    execFile(ps, ['-NoProfile', '-NonInteractive', '-Command', cmd], { windowsHide: true, timeout: 10000, maxBuffer: 8 * 1024 * 1024 }, (err, out) => {
      if (err) return resolve([]);
      const all = out.split(/\r?\n/).map((l) => l.trim().split('|')).filter((f) => f.length === 3 && /^\d+$/.test(f[0]) && /^\d+$/.test(f[1]))
        .map(([pid, ppid, name]) => ({ pid: Number(pid), parent: Number(ppid), name }));
      const found = [];
      let level = [parent];
      for (let depth = 0; depth < 3 && level.length; depth++) {
        const next = all.filter((p) => level.includes(p.parent) && p.pid !== parent);
        found.push(...next);
        level = next.map((p) => p.pid);
      }
      resolve(found);
    });
  });
}

class Miner {

  constructor(pidFile) {
    this.pidFile = pidFile;
    this.child = null;
    this.adopted = null;
    this.startedAt = null;
    this.lastExit = null;
    this.log = [];
    this.wantStop = false;
    this.codeFile = null;
    this.onExit = null;
  }

  get running() {
    return this.child !== null || this.adopted !== null;
  }

  get pid() {
    return this.child?.pid ?? this.adopted?.pid ?? null;
  }

  remember(pid, showWindow, exe, meta) {
    try {
      fs.writeFileSync(this.pidFile, JSON.stringify({ pid, showWindow, exe, startedAt: this.startedAt, meta }));
    } catch {

    }
  }

  forget() {
    fs.rm(this.pidFile, { force: true }, () => {});
  }

  async adopt(apiPort) {
    let saved;
    try {
      saved = JSON.parse(fs.readFileSync(this.pidFile, 'utf8'));
    } catch {
      return false;
    }
    if (!Number.isInteger(saved?.pid) || saved.pid <= 0) return false;

    const exeName = EXE;
    const viaCmd = saved.showWindow && process.platform === 'win32';
    let pid = saved.pid;
    let name = alive(pid) ? await imageName(pid) : null;
    if (viaCmd && name?.toLowerCase() !== 'cmd.exe') {

      const tree = await descendants(saved.pid);
      const isExe = (p) => p.name.toLowerCase() === exeName.toLowerCase();
      const inner = tree.find((p) => p.name.toLowerCase() === 'cmd.exe' && tree.some((q) => q.parent === p.pid && isExe(q)));
      pid = (inner ?? tree.find(isExe))?.pid ?? null;
      name = pid && alive(pid) ? await imageName(pid) : null;
    }
    const expected = viaCmd ? ['cmd.exe', exeName] : [exeName];
    if (!name || !expected.some((e) => e.toLowerCase() === name.toLowerCase()) || !(await summary(apiPort))) {
      this.forget();
      return false;
    }
    this.adopted = { pid };
    this.meta = saved.meta ?? null;
    this.startedAt = saved.startedAt ?? Date.now();
    this.showWindow = Boolean(saved.showWindow);
    return true;
  }

  checkAdopted() {
    if (this.adopted && !alive(this.adopted.pid)) this.ended(null, null, true);
  }

  ended(code, signal, clean) {
    const byUser = this.wantStop || clean || CLEAN_EXIT.has(code);
    this.lastExit = { code, signal, at: Date.now(), byUser };
    this.child = null;
    this.adopted = null;
    this.startedAt = null;
    this.forget();
    if (this.onExit) this.onExit(this.lastExit);
  }

  push(text) {
    for (const line of text.split(/\r?\n/)) {
      if (line.trim()) this.log.push(line);
    }
    if (this.log.length > LOG_LINES) this.log.splice(0, this.log.length - LOG_LINES);
  }

  start(exe, args, showWindow, meta = null) {
    this.meta = meta;
    if (this.running) return;
    this.sweepCodeDirs();

    if (!args.every((a) => typeof a === 'string' && SAFE_ARG.test(a))) throw new Error('the miner command line has a value that is not allowed');
    this.wantStop = false;
    this.log = [];
    const env = { ...process.env };
    const opts = { cwd: path.dirname(exe), env };
    let child;
    if (showWindow && process.platform === 'win32') {

      let codeFile = null;
      try { codeFile = path.join(fs.mkdtempSync(path.join(path.dirname(this.pidFile), 'exit-')), 'code.txt'); } catch {  }
      const line = windowLine(exe, args, codeFile, system32('cmd.exe'));
      if (codeFile && line.includes(codeFile)) this.codeFile = codeFile;
      else if (codeFile) try { fs.rmSync(path.dirname(codeFile), { recursive: true, force: true }); } catch {  }
      child = spawn(system32('cmd.exe'), ['/d', '/v:off', '/s', '/c', `"${line}"`], { ...opts, windowsHide: true, windowsVerbatimArguments: true, stdio: 'ignore' });
    } else {

      child = spawn(exe, args, { ...opts, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
      child.stdout.setEncoding('utf8').on('data', (d) => this.push(d));
      child.stderr.setEncoding('utf8').on('data', (d) => this.push(d));
    }
    this.child = child;
    this.startedAt = Date.now();

    this.showWindow = showWindow && process.platform === 'win32';
    child.on('error', (err) => {
      this.push(`could not start the miner: ${err.message}`);
    });
    child.on('exit', (code, signal) => this.ended(this.windowCode(code), signal, false));
    if (child.pid) this.remember(child.pid, this.showWindow, exe, meta);
  }

  sweepCodeDirs() {
    try {
      const base = path.dirname(this.pidFile);
      for (const name of fs.readdirSync(base)) if (/^exit-/.test(name)) fs.rmSync(path.join(base, name), { recursive: true, force: true });
    } catch {  }
  }

  windowCode(code) {
    const file = this.codeFile;
    this.codeFile = null;
    if (!file) return code;
    try {

      if (!fs.lstatSync(path.dirname(file)).isDirectory() || !fs.lstatSync(file).isFile()) return code;
      const text = fs.readFileSync(file, 'utf8');
      return /^-?\d{1,10}\s*$/.test(text) ? Number.parseInt(text, 10) : code;
    } catch {
      return code;
    } finally {
      const dir = path.dirname(file);
      if (path.basename(dir).startsWith('exit-')) try { fs.rmSync(dir, { recursive: true, force: true }); } catch {  }
    }
  }

  stop() {
    const pid = this.pid;
    if (!pid) return Promise.resolve();
    this.wantStop = true;
    return new Promise((resolve) => {
      const done = setTimeout(resolve, 8000);
      const gone = () => {
        clearTimeout(done);
        resolve();
      };
      if (this.child) {
        this.child.once('exit', gone);
      } else {
        const poll = setInterval(() => {
          if (!alive(pid)) {
            clearInterval(poll);
            this.ended(null, null, true);
            gone();
          }
        }, 250);
      }
      if (process.platform === 'win32') {

        execFile(system32('taskkill.exe'), ['/PID', String(pid), '/T', '/F'], { windowsHide: true }, () => {});
      } else {
        process.kill(pid, 'SIGTERM');
      }
    });
  }
}

module.exports = { Miner, find, resolveMiner, windowLine, candidates, summary, EXE };
