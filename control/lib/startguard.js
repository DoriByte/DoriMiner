const path = require('node:path');

const DEBUG = /^--(remote-debugging|inspect|debug|js-flags)/;

function refusal({ argv, packaged, testBuild, tmpDir }) {
  const debug = argv.slice(1).find((a) => DEBUG.test(a));
  if (testBuild) {
    const data = argv.find((a) => a.startsWith('--user-data-dir='))?.slice('--user-data-dir='.length);
    const rel = data ? path.relative(path.resolve(tmpDir), path.resolve(data)) : '..';
    if (rel === '' || rel.startsWith('..') || path.isAbsolute(rel)) return 'a test build runs only with --user-data-dir in the temporary folder';
    return null;
  }
  if (packaged && debug) return `this package does not start with ${debug.split('=')[0]}`;
  return null;
}

module.exports = { refusal };
