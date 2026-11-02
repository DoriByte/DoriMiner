const { createHash } = require('node:crypto');

const IV = ['6a09e667f3bcc908', 'bb67ae8584caa73b', '3c6ef372fe94f82b', 'a54ff53a5f1d36f1', '510e527fade682d1', '9b05688c2b3e6c1f', '1f83d9abfb41bd6b', '5be0cd19137e2179'].map((h) => BigInt(`0x${h}`));
const SIGMA = ['0123456789abcdef', 'ea489fd61c02b753', 'b8c052fdae367194', '7931dcbe265a40f8', '905724afe1bc683d', '2c6a0b834d75fe19', 'c51fed4a0763928b', 'db7ec13950f4862a', '6fe9b308c2d714a5', 'a2847615fb9e3cd0'].map((r) => [...r].map((c) => parseInt(c, 16)));
const MASK = (1n << 64n) - 1n;
const rotr = (x, n) => ((x >> n) | (x << (64n - n))) & MASK;

function blake2b(input, outLen) {
  const h = IV.slice();
  h[0] ^= 0x01010000n | BigInt(outLen);
  const block = (chunk, total, last) => {
    const m = Array.from({ length: 16 }, (_, i) => chunk.readBigUInt64LE(i * 8));
    const v = [...h, ...IV];
    v[12] ^= total & MASK;
    v[13] ^= total >> 64n;
    if (last) v[14] ^= MASK;
    const g = (a, b, c, d, x, y) => {
      v[a] = (v[a] + v[b] + x) & MASK;
      v[d] = rotr(v[d] ^ v[a], 32n);
      v[c] = (v[c] + v[d]) & MASK;
      v[b] = rotr(v[b] ^ v[c], 24n);
      v[a] = (v[a] + v[b] + y) & MASK;
      v[d] = rotr(v[d] ^ v[a], 16n);
      v[c] = (v[c] + v[d]) & MASK;
      v[b] = rotr(v[b] ^ v[c], 63n);
    };
    for (let r = 0; r < 12; r += 1) {
      const s = SIGMA[r % 10];
      g(0, 4, 8, 12, m[s[0]], m[s[1]]);
      g(1, 5, 9, 13, m[s[2]], m[s[3]]);
      g(2, 6, 10, 14, m[s[4]], m[s[5]]);
      g(3, 7, 11, 15, m[s[6]], m[s[7]]);
      g(0, 5, 10, 15, m[s[8]], m[s[9]]);
      g(1, 6, 11, 12, m[s[10]], m[s[11]]);
      g(2, 7, 8, 13, m[s[12]], m[s[13]]);
      g(3, 4, 9, 14, m[s[14]], m[s[15]]);
    }
    for (let i = 0; i < 8; i += 1) h[i] ^= v[i] ^ v[i + 8];
  };
  let pos = 0;
  while (input.length - pos > 128) {
    block(input.subarray(pos, pos + 128), BigInt(pos + 128), false);
    pos += 128;
  }
  const lastBlock = Buffer.alloc(128);
  input.copy(lastBlock, 0, pos);
  block(lastBlock, BigInt(input.length), true);
  const out = Buffer.alloc(64);
  h.forEach((x, i) => out.writeBigUInt64LE(x, i * 8));
  return out.subarray(0, outLen);
}
const blake2b512 = (input) => blake2b(input, 64);

const BASE58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';

function base58Decode(text        )                    {
  let n = 0n;
  for (const ch of text) {
    const v = BASE58.indexOf(ch);
    if (v < 0) return null;
    n = n * 58n + BigInt(v);
  }
  const bytes           = [];
  while (n > 0n) {
    bytes.unshift(Number(n & 0xffn));
    n >>= 8n;
  }
  for (const ch of text) {
    if (ch !== '1') break;
    bytes.unshift(0);
  }
  return Uint8Array.from(bytes);
}

function base58CheckOk(text        )          {
  const raw = base58Decode(text);
  if (raw === null || raw.length < 5) return false;
  const sum = createHash('sha256').update(createHash('sha256').update(raw.subarray(0, -4)).digest()).digest();
  return raw.subarray(-4).every((b, i) => b === sum[i]);
}

function ss58Ok(text        )          {
  const raw = base58Decode(text);
  if (raw === null || raw.length < 35) return false;
  const sum = blake2b512(Buffer.concat([Buffer.from('SS58PRE'), raw.subarray(0, -2)]));
  return raw[raw.length - 2] === sum[0] && raw[raw.length - 1] === sum[1];
}

const BECH32 = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l';
const BECH32M_CONST = 0x2bc830a3;

function bech32mOk(text        , hrp        )          {
  const lower = text.toLowerCase();
  if (text !== lower && text !== text.toUpperCase()) return false;
  const pos = lower.lastIndexOf('1');
  if (lower.slice(0, pos) !== hrp || lower.length - pos - 1 < 6) return false;
  const data = [...lower.slice(pos + 1)].map((c) => BECH32.indexOf(c));
  if (data.includes(-1)) return false;
  const generators = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
  let chk = 1;
  for (const v of [...[...hrp].map((c) => c.charCodeAt(0) >> 5), 0, ...[...hrp].map((c) => c.charCodeAt(0) & 31), ...data]) {
    const top = chk >> 25;
    chk = ((chk & 0x1ffffff) << 5) ^ v;
    for (let i = 0; i < 5; i += 1) if ((top >> i) & 1) chk ^= generators[i]          ;
  }
  return chk === BECH32M_CONST;
}

function segwitOk(text        , hrp        )          {
  const lower = text.toLowerCase();
  if (text !== lower && text !== text.toUpperCase()) return false;
  const pos = lower.lastIndexOf('1');
  if (lower.slice(0, pos) !== hrp || lower.length > 90 || lower.length - pos - 1 < 7) return false;
  const data = [...lower.slice(pos + 1)].map((c) => BECH32.indexOf(c));
  if (data.includes(-1)) return false;
  const generators = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
  let chk = 1;
  for (const v of [...[...hrp].map((c) => c.charCodeAt(0) >> 5), 0, ...[...hrp].map((c) => c.charCodeAt(0) & 31), ...data]) {
    const top = chk >> 25;
    chk = ((chk & 0x1ffffff) << 5) ^ v;
    for (let i = 0; i < 5; i += 1) if ((top >> i) & 1) chk ^= generators[i]          ;
  }
  const version = data[0]          ;
  if (version > 16 || chk !== (version === 0 ? 1 : BECH32M_CONST)) return false;

  const groups = data.slice(1, -6);
  const bits = groups.length * 5;
  const bytes = Math.floor(bits / 8);
  if (bits - bytes * 8 > 4) return false;
  let rest = 0;
  for (const g of groups) rest = ((rest << 5) | g) & 0xff;
  if ((rest & ((1 << (bits - bytes * 8)) - 1)) !== 0) return false;
  return version === 0 ? bytes === 20 || bytes === 32 : bytes >= 2 && bytes <= 40;
}

function base58VersionOk(text        , versions                   )          {
  const raw = base58Decode(text);
  return raw !== null && raw.length === 25 && versions.includes(raw[0]          ) && base58CheckOk(text);
}

function ergoOk(text) {
  const raw = base58Decode(text);
  if (raw === null || raw.length !== 38 || raw[0] !== 1) return false;
  const buf = Buffer.from(raw);
  return blake2b(buf.subarray(0, 34), 32).subarray(0, 4).equals(buf.subarray(34));
}

const CHECKS                                                         = {
  PRL: (a) => a.length === 63 && bech32mOk(a, 'prl'),
  QTC: ss58Ok,
  RVN: (a) => a.length === 34 && base58CheckOk(a),
  XNA: (a) => a.length === 34 && base58CheckOk(a),
  ERG: ergoOk,

  VELA: (a) => (/^vela1/i.test(a) ? segwitOk(a, 'vela') : base58VersionOk(a, [0x46, 0x4b])),
};

function addressCheck(coinId        , address        )                 {
  const check = Object.hasOwn(CHECKS, coinId) ? CHECKS[coinId] : undefined;
  return check === undefined ? null : check(address);
}

module.exports = { addressCheck, blake2b, blake2b512 };
