//! The kernel images as they sit inside the binary and in the public source
//! tree: scrambled, and opened in memory just before the driver loads them.
//!
//! This is not strong protection and does not pretend to be: the miner has to
//! open the images to run them, so the code that does it is here. It keeps the
//! kernel from falling out of the executable with `strings` or a section dump.
//! What asks you not to open the images outside the running miner is the
//! kernel's licence (`KERNEL-LICENSE`); whoever wants to know how the kernel
//! works is asked to talk to DoriByte rather than take it apart.
//!
//! Format: `DKN1`, 8 bytes of nonce, 8 bytes of FNV-1a 64 of the plain image
//! (little endian), then the image XORed with a keyed stream.

pub const MAGIC: &[u8; 4] = b"DKN1";
const KEY: u64 = 0x4472_6942_7974_6521;
const HEADER: usize = 4 + 8 + 8;

fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// splitmix64 from a seed: eight stream bytes a step.
fn stream(nonce: u64) -> impl FnMut() -> u64 {
    let mut state = KEY ^ nonce.rotate_left(29);
    move || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

fn apply(nonce: u64, data: &mut [u8]) {
    let mut next = stream(nonce);
    for chunk in data.chunks_mut(8) {
        let word = next().to_le_bytes();
        for (byte, key) in chunk.iter_mut().zip(word) {
            *byte ^= key;
        }
    }
}

/// The image as it is stored. The nonce only has to differ between images.
#[allow(dead_code)]
pub fn seal(plain: &[u8], nonce: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + plain.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce.to_le_bytes());
    out.extend_from_slice(&fnv1a(plain).to_le_bytes());
    out.extend_from_slice(plain);
    apply(nonce, &mut out[HEADER..]);
    out
}

/// The image the driver loads. `None` when the bytes are not a sealed image
/// or do not open to what was sealed.
#[allow(dead_code)]
pub fn unseal(sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < HEADER || &sealed[..4] != MAGIC {
        return None;
    }
    let nonce = u64::from_le_bytes(sealed[4..12].try_into().ok()?);
    let sum = u64::from_le_bytes(sealed[12..20].try_into().ok()?);
    let mut plain = sealed[HEADER..].to_vec();
    apply(nonce, &mut plain);
    (fnv1a(&plain) == sum).then_some(plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_what_it_sealed() {
        for size in [0usize, 1, 7, 8, 9, 4096, 100_003] {
            let plain: Vec<u8> = (0..size).map(|i| (i * 31 + size) as u8).collect();
            let sealed = seal(&plain, size as u64);
            assert_eq!(unseal(&sealed).as_deref(), Some(plain.as_slice()));
            if (64..=4096).contains(&size) {
                assert!(!sealed.windows(16).any(|w| plain.windows(16).any(|p| p == w)));
            }
        }
    }

    #[test]
    fn refuses_what_it_did_not_seal() {
        assert!(unseal(b"\x7fELF plain image").is_none());
        let mut sealed = seal(b"mining_main and the rest of an image", 3);
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(unseal(&sealed).is_none());
    }
}
