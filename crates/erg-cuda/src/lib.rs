//! Published without its comments: they describe the kernel, which is closed for now (see the README).
//! The code is all here.


use primitive_types::U256;
use std::ffi::{c_char, c_int, c_void};

extern "C" {
    fn erg_open(device: c_int, err: *mut c_char, errlen: c_int) -> *mut c_void;
    fn erg_table(h: *mut c_void, height: u32, made: *mut c_int) -> c_int;
    fn erg_mine(h: *mut c_void, msg: *const u8, target_top: u64, nonce0: u64, size: u32, found: *mut u64, max: c_int) -> c_int;
    fn erg_per_launch(h: *mut c_void, size: u32) -> u64;
    fn erg_hits(h: *mut c_void, msg: *const u8, nonces: *const u64, count: c_int, hits: *mut u8) -> c_int;
    fn erg_close(h: *mut c_void);
    fn erg_info_a(h: *mut c_void) -> c_int;
    fn erg_info_b(h: *mut c_void) -> c_int;
    fn erg_table_bytes(h: *mut c_void) -> u64;
    fn erg_info_d(h: *mut c_void) -> u32;
}

pub struct Engine {
    h: *mut c_void,
    height: u32,
}

impl Engine {
    pub fn open(device: usize) -> Result<Self, String> {
        let mut err = [0 as c_char; 256];
        let h = unsafe { erg_open(device as c_int, err.as_mut_ptr(), err.len() as c_int) };
        if h.is_null() {
            let msg = unsafe { std::ffi::CStr::from_ptr(err.as_ptr()) }.to_string_lossy().into_owned();
            return Err(if msg.is_empty() { "the ERG engine did not open".into() } else { msg });
        }
        Ok(Self { h, height: 0 })
    }

    pub fn set_height(&mut self, height: u32) -> Result<bool, String> {
        if height == self.height {
            return Ok(false);
        }
        let mut made: c_int = 0;
        match unsafe { erg_table(self.h, height, &mut made) } {
            0 => {
                self.height = height;
                Ok(made != 0)
            }
            1 => Err(format!("the table for height {height} does not fit on this card, even in compact form")),
            r => Err(format!("making the table for height {height} failed ({r})")),
        }
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn compact(&self) -> bool {
        unsafe { erg_info_a(self.h) != 0 }
    }

    pub fn level(&self) -> f64 {
        unsafe { erg_info_b(self.h) as f64 / 1000.0 }
    }

    pub fn table_bytes(&self) -> u64 {
        unsafe { erg_table_bytes(self.h) }
    }

    pub fn per_launch(&self, size: u32) -> u64 {
        unsafe { erg_per_launch(self.h, size) }
    }

    pub fn launch_size(&self) -> u32 {
        unsafe { erg_info_d(self.h) }
    }

    pub fn mine(&mut self, msg: &[u8; 32], target: &U256, nonce0: u64, size: u32) -> Result<Vec<u64>, String> {
        let mut found = [0u64; 32];
        let r = unsafe { erg_mine(self.h, msg.as_ptr(), target_top(target), nonce0, size, found.as_mut_ptr(), found.len() as c_int) };
        if r < 0 {
            return Err(format!("the mining kernel failed ({r})"));
        }
        Ok(found[..r as usize].to_vec())
    }

    pub fn hits(&mut self, msg: &[u8; 32], nonces: &[u64]) -> Result<Vec<[u8; 32]>, String> {
        let mut out = vec![0u8; 32 * nonces.len()];
        let r = unsafe { erg_hits(self.h, msg.as_ptr(), nonces.as_ptr(), nonces.len() as c_int, out.as_mut_ptr()) };
        if r != 0 {
            return Err(format!("the self-check kernel failed ({r})"));
        }
        Ok(out.chunks(32).map(|c| c.try_into().unwrap()).collect())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { erg_close(self.h) }
    }
}

const IV: [u64; 8] = [
    0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1, 0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b,
    0x5be0cd19137e2179,
];
const SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

fn compress(h: &mut [u64; 8], block: &[u8; 128], t: u64, last: bool) {
    let mut m = [0u64; 16];
    for (k, w) in m.iter_mut().enumerate() {
        *w = u64::from_le_bytes(block[8 * k..8 * k + 8].try_into().unwrap());
    }
    let mut v = [0u64; 16];
    v[..8].copy_from_slice(h);
    v[8..].copy_from_slice(&IV);
    v[12] ^= t;
    if last {
        v[14] = !v[14];
    }
    let g = |v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize, x: u64, y: u64| {
        v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
        v[d] = (v[d] ^ v[a]).rotate_right(32);
        v[c] = v[c].wrapping_add(v[d]);
        v[b] = (v[b] ^ v[c]).rotate_right(24);
        v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
        v[d] = (v[d] ^ v[a]).rotate_right(16);
        v[c] = v[c].wrapping_add(v[d]);
        v[b] = (v[b] ^ v[c]).rotate_right(63);
    };
    for r in 0..12 {
        let s = &SIGMA[r % 10];
        g(&mut v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
        g(&mut v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
        g(&mut v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
        g(&mut v, 3, 7, 11, 15, m[s[6]], m[s[7]]);
        g(&mut v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
        g(&mut v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
        g(&mut v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
        g(&mut v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
    }
    for k in 0..8 {
        h[k] ^= v[k] ^ v[k + 8];
    }
}

pub fn blake2b256(data: &[u8]) -> [u8; 32] {
    let mut h = IV;
    h[0] ^= 0x0101_0020;
    let mut block = [0u8; 128];
    let mut t = 0u64;
    let mut chunks = data.chunks(128).peekable();
    if data.is_empty() {
        compress(&mut h, &block, 0, true);
    }
    while let Some(c) = chunks.next() {
        block = [0u8; 128];
        block[..c.len()].copy_from_slice(c);
        t += c.len() as u64;
        compress(&mut h, &block, t, chunks.peek().is_none());
    }
    let mut out = [0u8; 32];
    for k in 0..4 {
        out[8 * k..8 * k + 8].copy_from_slice(&h[k].to_le_bytes());
    }
    out
}

pub fn calc_n(height: u32) -> u32 {
    let mut n: u32 = 1 << 26;
    if height < 600 * 1024 {
        return n;
    }
    for _ in 0..(height.min(4_198_400) - 600 * 1024) / (50 * 1024) + 1 {
        n = n / 100 * 105;
    }
    n
}

pub fn table_bytes(height: u32) -> u64 {
    calc_n(height) as u64 * 32
}

fn m_const() -> &'static [u8] {
    static M: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    M.get_or_init(|| (0u64..1024).flat_map(|i| i.to_be_bytes()).collect())
}

pub fn element(j: u32, height: u32) -> U256 {
    let mut d = Vec::with_capacity(8 + 8192);
    d.extend_from_slice(&j.to_be_bytes());
    d.extend_from_slice(&height.to_be_bytes());
    d.extend_from_slice(m_const());
    U256::from_big_endian(&blake2b256(&d)[1..])
}

pub fn hit(msg: &[u8; 32], nonce: u64, height: u32) -> [u8; 32] {
    let n = calc_n(height) as u64;
    let mut a = [0u8; 40];
    a[..32].copy_from_slice(msg);
    a[32..].copy_from_slice(&nonce.to_be_bytes());
    let i = (u64::from_be_bytes(blake2b256(&a)[24..].try_into().unwrap()) % n) as u32;
    let e = element(i, height);
    let eb = e.to_big_endian();
    let mut seed = Vec::with_capacity(71);
    seed.extend_from_slice(&eb[1..]);
    seed.extend_from_slice(&a);
    let j = blake2b256(&seed);
    let mut x = [0u8; 35];
    x[..32].copy_from_slice(&j);
    x[32..].copy_from_slice(&j[..3]);
    let mut f = U256::zero();
    for k in 0..32 {
        let idx = (u32::from_be_bytes(x[k..k + 4].try_into().unwrap()) as u64 % n) as u32;
        f += element(idx, height);
    }
    let fb = f.to_big_endian();
    blake2b256(&fb)
}

pub fn target_from_decimal(text: &str) -> Result<U256, String> {
    let t = text.trim();
    if t.is_empty() || !t.bytes().all(|c| c.is_ascii_digit()) {
        return Err(format!("not a decimal target: {t:.80}"));
    }
    U256::from_dec_str(t).map_err(|e| format!("a target that does not fit 256 bits ({e:?})"))
}

pub fn target_top(target: &U256) -> u64 {
    (*target >> 192).low_u64()
}

pub fn meets(hit: &[u8; 32], target: &U256) -> bool {
    U256::from_big_endian(hit) < *target
}

pub fn difficulty(target: &U256) -> f64 {
    const Q: &str = "115792089237316195423570985008687907852837564279074904382605163141518161494337";
    if target.is_zero() {
        return f64::INFINITY;
    }
    let q = U256::from_dec_str(Q).unwrap();
    let (d, _) = q.div_mod(*target);
    let lz = d.leading_zeros();
    if lz >= 192 {
        d.low_u64() as f64
    } else {
        ((d >> (192 - lz)).low_u64() as f64) * 2f64.powi(192 - lz as i32)
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub const VECTOR_MSG: &str = "548c3e602a8f36f8f2738f5f643b02425038044d98543a51cabaa9785e7e864f";
pub const VECTOR_HEIGHT: u32 = 614_400;
pub const VECTOR_NONCE: u64 = 0x3105;
pub const VECTOR_HIT: &str = "0002fcb113fe65e5754959872dfdbffea0489bf830beb4961ddc0e9e66a1412a";

pub fn self_check(engine: &mut Engine, msg: &[u8; 32], from: u64, count: usize) -> Result<(), String> {
    let nonces: Vec<u64> = (0..count as u64).map(|k| from.wrapping_add(k.wrapping_mul(0x9e37_79b9_7f4a_7c15))).collect();
    let card = engine.hits(msg, &nonces)?;
    for (k, &nonce) in nonces.iter().enumerate() {
        let cpu = hit(msg, nonce, engine.height());
        if card[k] != cpu {
            return Err(format!("the ERG engine does NOT match the CPU (height {}, nonce {nonce:016x}: card {}, CPU {})", engine.height(), hex(&card[k]), hex(&cpu)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(h: &str) -> Vec<u8> {
        (0..h.len() / 2).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn blake2b_matches_rfc7693() {
        assert_eq!(hex(&blake2b256(b"")), "0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8");
        assert_eq!(hex(&blake2b256(b"abc")), "bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319");
        assert_eq!(blake2b256(&[7u8; 256]), blake2b256(&[7u8; 256]));
    }

    #[test]
    fn calc_n_matches_the_reference() {
        assert_eq!(calc_n(500_000), 1 << 26);
        assert_eq!(calc_n(600 * 1024), 70_464_240);
        assert_eq!(calc_n(650 * 1024), 73_987_410);
        assert_eq!(calc_n(788_400), 81_571_035);
        assert_eq!(calc_n(1_051_200), 104_107_290);
        assert_eq!(calc_n(4_198_400), 2_143_944_600);
        assert_eq!(calc_n(41_984_000), 2_143_944_600);
        assert_eq!(calc_n(1_904_531), 238_614_390);
    }

    #[test]
    fn the_official_vector() {
        let msg: [u8; 32] = unhex(VECTOR_MSG).try_into().unwrap();
        assert_eq!(hex(&hit(&msg, VECTOR_NONCE, VECTOR_HEIGHT)), VECTOR_HIT);
    }

    #[test]
    fn target_and_meets() {
        let b = target_from_decimal("7067388259113537318333190002971674063283542741642755394446115914399301849").unwrap();
        let h: [u8; 32] = unhex(VECTOR_HIT).try_into().unwrap();
        assert!(meets(&h, &b));
        assert!(!meets(&[0xff; 32], &b));
        assert_eq!(target_top(&(U256::one() << 255)), 1 << 63);
        assert!(target_from_decimal("12a").is_err());
        assert!((difficulty(&b) - 16384.0).abs() < 1.0, "{}", difficulty(&b));
    }
}
