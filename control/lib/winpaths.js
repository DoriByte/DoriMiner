const fs = require('node:fs');
const path = require('node:path');

function windowsDir(env = process.env, exists = fs.existsSync) {
  const usual = 'C:\\Windows';
  return exists(path.win32.join(usual, 'System32', 'kernel32.dll')) ? usual : env.SystemRoot || usual;
}

const system32 = (...parts) => path.win32.join(windowsDir(), 'System32', ...parts);

module.exports = { windowsDir, system32 };
