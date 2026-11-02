
use sha2::{Digest, Sha256};
use std::ffi::{c_char, c_int, c_void, CString};

pub const MODEL_FILE: &str = "sd-turbo-int.poisd";
pub const MODEL_URLS: [&str; 2] = ["https://vela.suprnova.cc/dl/sd-turbo-int.poisd", "https://www.suprnova.cc/dl/sd-turbo-int.poisd"];
pub const MODEL_SHA256: &str = "ba38ab96b3398e22ea631d8a457350f91833e57463199d063663308513651f36";
pub const MODEL_HASH: &str = "a7fd64b04c8bf36c1b74a629e32f093ce10e364787292f53fa1337557eb9776a";
pub const X0_BYTES: usize = 32_768;
pub const MAX_TOKENS: usize = 75;
pub const DEFAULT_BATCH: usize = 8;

extern "C" {
    fn vela_open(device: c_int, model_path: *const c_char, batch: c_int, err: *mut c_char, errlen: c_int) -> *mut c_void;
    fn vela_prompt(h: *mut c_void, tokens: *const c_int, n: c_int) -> c_int;
    fn vela_run(h: *mut c_void, seeds: *const u8, x0: *mut u8) -> c_int;
    fn vela_close(h: *mut c_void);
}

pub struct Engine {
    h: *mut c_void,
    batch: usize,
    prompt: Option<Vec<i32>>,
}

impl Engine {
    pub fn open(device: usize, model: &std::path::Path, batch: usize) -> Result<Self, String> {
        let path = CString::new(model.to_string_lossy().as_bytes()).map_err(|_| "a model path with a NUL byte")?;
        let mut err = [0 as c_char; 256];
        let h = unsafe { vela_open(device as c_int, path.as_ptr(), batch as c_int, err.as_mut_ptr(), err.len() as c_int) };
        if h.is_null() {
            let msg = unsafe { std::ffi::CStr::from_ptr(err.as_ptr()) }.to_string_lossy().into_owned();
            return Err(if msg.is_empty() { "the Vela engine did not open".into() } else { msg });
        }
        Ok(Self { h, batch, prompt: None })
    }

    pub fn batch(&self) -> usize {
        self.batch
    }

    pub fn set_prompt(&mut self, tokens: &[i32]) -> Result<(), String> {
        if self.prompt.as_deref() == Some(tokens) {
            return Ok(());
        }
        if tokens.len() > MAX_TOKENS {
            return Err(format!("a prompt of {} tokens (at most {MAX_TOKENS})", tokens.len()));
        }
        let r = unsafe { vela_prompt(self.h, tokens.as_ptr(), tokens.len() as c_int) };
        if r != 0 {
            return Err(format!("the text encoder failed ({r})"));
        }
        self.prompt = Some(tokens.to_vec());
        Ok(())
    }

    pub fn run(&mut self, seeds: &[[u8; 32]], x0: &mut Vec<u8>) -> Result<(), String> {
        if seeds.len() != self.batch {
            return Err(format!("{} seeds for a batch of {}", seeds.len(), self.batch));
        }
        x0.resize(self.batch * X0_BYTES, 0);
        let flat: Vec<u8> = seeds.iter().flatten().copied().collect();
        let r = unsafe { vela_run(self.h, flat.as_ptr(), x0.as_mut_ptr()) };
        if r != 0 {
            return Err(format!("the UNet step failed ({r})"));
        }
        Ok(())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { vela_close(self.h) }
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

pub fn sha256d(data: &[u8]) -> [u8; 32] {
    sha256(&sha256(data))
}

pub fn tagged_hash(tag: &str, data: &[u8]) -> [u8; 32] {
    let t = sha256(tag.as_bytes());
    let mut h = Sha256::new();
    h.update(t);
    h.update(t);
    h.update(data);
    h.finalize().into()
}

pub fn model_hash_le() -> [u8; 32] {
    let mut b: [u8; 32] = hex_bytes(MODEL_HASH).try_into().expect("32 bytes");
    b.reverse();
    b
}

fn hex_bytes(h: &str) -> Vec<u8> {
    (0..h.len() / 2).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap_or(0)).collect()
}

pub fn seed(header80: &[u8; 80]) -> [u8; 32] {
    let mut d = [0u8; 112];
    d[..80].copy_from_slice(header80);
    d[80..].copy_from_slice(&model_hash_le());
    tagged_hash("POI/seed", &d)
}

pub fn hash_poi(seed: &[u8; 32], x0: &[u8]) -> [u8; 32] {
    let mut d = Vec::with_capacity(32 + x0.len());
    d.extend_from_slice(seed);
    d.extend_from_slice(x0);
    tagged_hash("POI/out", &d)
}

pub fn pow_hash(header80: &[u8; 80], poi: &[u8; 32]) -> [u8; 32] {
    let mut d = [0u8; 112];
    d[..80].copy_from_slice(header80);
    d[80..].copy_from_slice(poi);
    sha256d(&d)
}

pub fn target_from_difficulty(d: f64) -> [u8; 32] {
    let mut t = [0xffu8; 32];
    if !(d >= 1.0) {
        return t;
    }
    let dv = d.ceil() as u128;
    let mut rem: u128 = 0;
    for i in (0..32).rev() {
        let cur = (rem << 8) | 0xff;
        t[i] = (cur / dv) as u8;
        rem = cur % dv;
    }
    t
}

pub fn meets(hash: &[u8; 32], target: &[u8; 32]) -> bool {
    for i in (0..32).rev() {
        if hash[i] != target[i] {
            return hash[i] < target[i];
        }
    }
    true
}

pub const VECTOR_TOKENS: [i32; 3] = [320, 736, 3240];
pub const VECTOR_X0_SHA256: &str = "bef682cc9651fe8d42f075e98b907432f86a0b9e9a40e772fef99fc424f5c518";
pub const VECTOR_HASH_POI: &str = "44b0b93a02bcda5ab3043d86bada0e9b027903aa36ea3b79a30e72f68d6a95b0";

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn self_check(engine: &mut Engine) -> Result<(), String> {
    engine.set_prompt(&VECTOR_TOKENS)?;
    let s = sha256(b"x");
    let seeds = vec![s; engine.batch()];
    let mut x0 = Vec::new();
    engine.run(&seeds, &mut x0)?;
    for b in 0..engine.batch() {
        let one = &x0[b * X0_BYTES..(b + 1) * X0_BYTES];
        let (hx, hp) = (hex(&sha256(one)), hex(&hash_poi(&s, one)));
        if hx != VECTOR_X0_SHA256 || hp != VECTOR_HASH_POI {
            return Err(format!("the Vela engine does NOT match the official vector (slot {b}: sha256(x0) {hx}, hashPoI {hp})"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_and_meets() {
        let t = target_from_difficulty(1.0);
        assert_eq!(t, [0xff; 32]);
        let t = target_from_difficulty(256.0);
        assert_eq!(t[31], 0);
        assert_eq!(t[30], 0xff);
        let mut h = [0u8; 32];
        assert!(meets(&h, &t));
        h[31] = 1;
        assert!(!meets(&h, &t));
        let t = target_from_difficulty(2048.0);
        assert_eq!((t[31], t[30]), (0x00, 0x1f));
    }

    #[test]
    fn tagged_hash_matches_its_definition() {
        let t = sha256(b"POI/out");
        let mut d = t.to_vec();
        d.extend_from_slice(&t);
        d.extend_from_slice(b"abc");
        assert_eq!(tagged_hash("POI/out", b"abc"), sha256(&d));
        assert_eq!(model_hash_le()[0], 0x6a);
    }
}
