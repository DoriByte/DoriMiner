const crypto = require('node:crypto');
const { X509Certificate } = crypto;

const APP_PACKAGE = 'pt.doribyte.rig';
const RELEASE_SIGNING_DIGESTS = ['7295b96574c9ce25bffc68fabf5b2ebd3bb915a4101943556c57487df14b8664'];

const GOOGLE_ROOT_KEYS = [
  'feb2ea7551ee316ed4bb443c8293b884dbfdea40b603ee3e4f4a897e4580fbae',
  '3ee44512a1af2beb39c889490c60ea3f82e43f5d5a5532f5ab9419f676cd07ec',
];
const KEY_DESCRIPTION_OID = '1.3.6.1.4.1.11129.2.1.17';
const MAX_AUTH_TIMEOUT_SEC = 60;
const SIGNATURE_SKEW_MS = 60_000;
const REVOKED_URL = 'https://android.googleapis.com/attestation/status';

class AttestError extends Error {
  constructor(code) {
    super(code);
    this.code = code;
  }
}
const fail = (code) => {
  throw new AttestError(code);
};

function readTlv(buf, offset) {
  if (offset + 2 > buf.length) throw new Error('DER truncated');
  const first = buf[offset];
  let pos = offset + 1;
  let tag = first & 0x1f;
  if (tag === 0x1f) {
    tag = 0;
    for (let i = 0; ; i += 1) {
      if (pos >= buf.length || i > 4) throw new Error('DER tag');
      const b = buf[pos];
      pos += 1;
      tag = tag * 128 + (b & 0x7f);
      if ((b & 0x80) === 0) break;
    }
  }
  if (pos >= buf.length) throw new Error('DER truncated');
  let length = buf[pos];
  pos += 1;
  if (length & 0x80) {
    const bytes = length & 0x7f;
    if (bytes === 0 || bytes > 4) throw new Error('DER length');
    length = 0;
    for (let i = 0; i < bytes; i += 1) {
      if (pos >= buf.length) throw new Error('DER truncated');
      length = length * 256 + buf[pos];
      pos += 1;
    }
  }
  const end = pos + length;
  if (end > buf.length) throw new Error('DER truncated');
  return { tlv: { cls: first >> 6, constructed: (first & 0x20) !== 0, tag, value: buf.subarray(pos, end) }, next: end };
}

function children(value) {
  const out = [];
  let pos = 0;
  while (pos < value.length) {
    const { tlv, next } = readTlv(value, pos);
    out.push(tlv);
    pos = next;
  }
  return out;
}

function one(buf) {
  const { tlv, next } = readTlv(buf, 0);
  if (next !== buf.length) throw new Error('DER extra');
  return tlv;
}

function int(tlv) {
  if (tlv.cls !== 0 || (tlv.tag !== 2 && tlv.tag !== 10) || tlv.value.length === 0 || tlv.value.length > 6) throw new Error('DER int');
  let n = tlv.value[0] & 0x80 ? -1 : 0;
  for (const b of tlv.value) n = n * 256 + b;
  return n;
}

function oid(tlv) {
  if (tlv.cls !== 0 || tlv.tag !== 6) throw new Error('DER oid');
  const parts = [];
  let n = 0;
  for (const b of tlv.value) {
    n = n * 128 + (b & 0x7f);
    if ((b & 0x80) === 0) {
      if (parts.length === 0) parts.push(n < 80 ? Math.floor(n / 40) : 2, n < 80 ? n % 40 : n - 80);
      else parts.push(n);
      n = 0;
    }
  }
  return parts.join('.');
}

function certExtension(certDer, wanted) {
  const cert = children(one(certDer).value);
  const tbs = children(cert[0].value);
  const ext = tbs.find((x) => x.cls === 2 && x.tag === 3);
  if (!ext) return null;
  for (const e of children(one(ext.value).value)) {
    const parts = children(e.value);
    if (oid(parts[0]) === wanted) {
      const octets = parts[parts.length - 1];
      if (octets.tag !== 4) throw new Error('DER extnValue');
      return octets.value;
    }
  }
  return null;
}

function authorizationList(tlv) {
  const out = new Map();
  for (const item of children(tlv.value)) if (item.cls === 2) out.set(item.tag, one(item.value));
  return out;
}

function parseKeyDescription(ext) {
  const seq = children(one(ext).value);
  if (seq.length < 8) throw new Error('KeyDescription');
  return {
    attestationSecurityLevel: int(seq[1]),
    keyMintSecurityLevel: int(seq[3]),
    challenge: seq[4].value,
    softwareEnforced: authorizationList(seq[6]),
    hardwareEnforced: authorizationList(seq[7]),
  };
}

function parseApplicationId(octets) {
  if (octets.tag !== 4) throw new Error('attestationApplicationId');
  const seq = children(one(octets.value).value);
  return {
    packages: children(seq[0].value).map((info) => children(info.value)[0].value.toString('utf8')),
    digests: children(seq[1].value).map((d) => d.value.toString('hex')),
  };
}

const serialOf = (cert) => cert.serialNumber.toLowerCase().replace(/^0+(?=.)/, '');
const spkiSha256 = (cert) => crypto.createHash('sha256').update(cert.publicKey.export({ type: 'spki', format: 'der' })).digest('hex');

function verifyAttestation(chainB64, policy) {
  if (!Array.isArray(chainB64) || chainB64.length < 2 || chainB64.length > 10 || chainB64.some((c) => typeof c !== 'string' || c.length > 20_000)) fail('attestChain');
  let certs;
  try {
    certs = chainB64.map((c) => new X509Certificate(Buffer.from(c, 'base64')));
  } catch {
    fail('attestChain');
  }
  const now = policy.now ?? new Date();
  for (let i = 0; i < certs.length; i += 1) {
    const cert = certs[i];
    if (!cert.verify((certs[i + 1] ?? cert).publicKey)) fail('attestChain');
    if (i > 0) {

      let description;
      try {
        description = certExtension(Buffer.from(cert.raw), KEY_DESCRIPTION_OID);
      } catch {
        fail('attestChain');
      }
      if (cert.ca !== true || description !== null) fail('attestChain');
      if (now < new Date(cert.validFrom) || now > new Date(cert.validTo)) fail('attestExpired');
    }
    if (policy.revoked?.has(serialOf(cert))) fail('attestRevoked');
  }
  if (!(policy.rootKeys ?? GOOGLE_ROOT_KEYS).includes(spkiSha256(certs[certs.length - 1]))) fail('attestRoot');

  const leaf = certs[0];
  let desc;
  try {
    const ext = certExtension(Buffer.from(leaf.raw), KEY_DESCRIPTION_OID);
    if (ext === null) fail('attestDescription');
    desc = parseKeyDescription(ext);
  } catch (error) {
    if (error instanceof AttestError) throw error;
    fail('attestDescription');
  }
  if (desc.attestationSecurityLevel < 1 || desc.keyMintSecurityLevel < 1) fail('attestSoftware');
  if (desc.challenge.length !== policy.challenge.length || !desc.challenge.equals(policy.challenge)) fail('attestChallenge');

  const hw = desc.hardwareEnforced;
  try {

    const rot = hw.get(704);
    if (!rot) fail('attestBoot');
    const parts = children(rot.value);
    if (!(parts[1].tag === 1 && parts[1].value[0] !== 0)) fail('attestUnlocked');
    if (int(parts[2]) !== 0) fail('attestModified');

    if (!children(hw.get(1)?.value ?? Buffer.alloc(0)).some((p) => int(p) === 2)) fail('attestKey');
    if (!hw.get(2) || int(hw.get(2)) !== 3) fail('attestKey');

    if (hw.has(503)) fail('attestNoAuth');
    const timeout = hw.get(505);
    if (!hw.has(504) || timeout === undefined || int(timeout) > MAX_AUTH_TIMEOUT_SEC) fail('attestNoAuth');

    const appId = desc.softwareEnforced.get(709) ?? hw.get(709);
    if (!appId) fail('attestApp');
    const app = parseApplicationId(appId);
    if (app.packages.length !== 1 || app.packages[0] !== APP_PACKAGE) fail('attestApp');
    if (!app.digests.some((d) => policy.signingDigests.includes(d))) fail('attestSigner');
  } catch (error) {
    if (error instanceof AttestError) throw error;
    fail('attestDescription');
  }
  return { publicKeyPem: leaf.publicKey.export({ type: 'spki', format: 'pem' }).toString(), level: desc.attestationSecurityLevel === 2 ? 'strongbox' : 'tee' };
}

function signedPayload(method, pathAndQuery, timestamp, nonce, body) {
  return [method.toUpperCase(), pathAndQuery, timestamp, nonce, crypto.createHash('sha256').update(body).digest('hex')].join('\n');
}

class NonceCache {
  constructor(now = Date.now) {
    this.now = now;
    this.seen = new Map();
  }

  use(key) {
    const now = this.now();
    for (const [k, at] of this.seen) if (now - at > 2 * SIGNATURE_SKEW_MS) this.seen.delete(k);
    if (this.seen.has(key)) return false;
    this.seen.set(key, now);
    return true;
  }
}

function verifyRequestSignature(publicKeyPem, input, nonces, now = Date.now()) {
  const { timestamp, nonce, signature } = input;
  if (typeof timestamp !== 'string' || typeof nonce !== 'string' || typeof signature !== 'string') return 'signatureMissing';
  if (!/^\d{10,16}$/.test(timestamp) || Math.abs(now - Number(timestamp)) > SIGNATURE_SKEW_MS) return 'signatureClock';
  if (!/^[A-Za-z0-9_-]{16,64}$/.test(nonce)) return 'signatureBad';
  let ok = false;
  try {
    ok = crypto.verify('sha256', Buffer.from(signedPayload(input.method, input.pathAndQuery, timestamp, nonce, input.body), 'utf8'), { key: crypto.createPublicKey(publicKeyPem), dsaEncoding: 'der' }, Buffer.from(signature, 'base64'));
  } catch {
    ok = false;
  }
  if (!ok) return 'signatureBad';
  if (!nonces.use(`${input.deviceId}:${nonce}`)) return 'signatureReused';
  return null;
}

function devApkDigests(env = process.env) {
  return String(env.DORI_DEV_APK_DIGESTS ?? '').split(',').map((d) => d.trim().toLowerCase()).filter((d) => /^[0-9a-f]{64}$/.test(d));
}

let revokedCache = null;

async function revokedKeys(fetchImpl = fetch) {
  if (revokedCache !== null && Date.now() - revokedCache.at < 3_600_000) return revokedCache.keys;
  try {
    const response = await fetchImpl(REVOKED_URL, { signal: AbortSignal.timeout(10_000) });
    if (!response.ok) throw new Error(String(response.status));
    const json = await response.json();
    const keys = new Set(Object.keys(json.entries ?? {}).map((k) => k.toLowerCase().replace(/^0+(?=.)/, '')));
    revokedCache = { at: Date.now(), keys };
    return keys;
  } catch {
    throw new AttestError('attestNoList');
  }
}

module.exports = { APP_PACKAGE, RELEASE_SIGNING_DIGESTS, GOOGLE_ROOT_KEYS, AttestError, NonceCache, verifyAttestation, signedPayload, verifyRequestSignature, spkiSha256, devApkDigests, revokedKeys };
