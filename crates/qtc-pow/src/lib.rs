//! Published without its comments: they describe the kernel, which is closed for now (see the README).
//! The code is all here.


use primitive_types::U512;
use qp_poseidon_core::{Goldilocks, Poseidon2};

pub use qp_poseidon_core::SPONGE_WIDTH;

pub fn get_nonce_hash(header: [u8; 32], nonce_be: [u8; 64]) -> U512 {
    let mut input = [0u8; 96];
    input[..32].copy_from_slice(&header);
    input[32..].copy_from_slice(&nonce_be);
    U512::from_big_endian(&qp_poseidon_core::hash_squeeze_twice(&input))
}

pub fn permute(state: &mut [u64; SPONGE_WIDTH]) {
    static POSEIDON2: std::sync::OnceLock<Poseidon2> = std::sync::OnceLock::new();
    let mut felts = state.map(Goldilocks::from_u64);
    POSEIDON2.get_or_init(Poseidon2::new).permute_mut(&mut felts);
    *state = felts.map(|g| g.as_canonical_u64());
}

pub fn target_from_difficulty(difficulty: U512) -> Option<U512> {
    (!difficulty.is_zero()).then(|| U512::MAX / difficulty)
}

pub fn u512_to_le_u32s(n: U512) -> [u32; 16] {
    let bytes = n.to_little_endian();
    let mut out = [0u32; 16];
    for i in 0..16 {
        out[i] = u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
    }
    out
}

pub fn u512_from_le_u32s(limbs: [u32; 16]) -> U512 {
    let mut bytes = [0u8; 64];
    for i in 0..16 {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&limbs[i].to_le_bytes());
    }
    U512::from_little_endian(&bytes)
}

pub fn format_hashrate(hashes_per_sec: f64) -> String {
    if hashes_per_sec >= 1_000_000_000.0 {
        format!("{:.2} GH/s", hashes_per_sec / 1_000_000_000.0)
    } else if hashes_per_sec >= 1_000_000.0 {
        format!("{:.2} MH/s", hashes_per_sec / 1_000_000.0)
    } else if hashes_per_sec >= 1_000.0 {
        format!("{:.2} KH/s", hashes_per_sec / 1_000.0)
    } else {
        format!("{:.2} H/s", hashes_per_sec)
    }
}

pub struct NonceHashKv {
    pub header: &'static str,
    pub nonce: &'static str,
    pub hash: &'static str,
}

pub const NONCE_HASH_KVS: &[NonceHashKv] = &[
    NonceHashKv {
        header: "0000000000000000000000000000000000000000000000000000000000000000",
        nonce: "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        hash: "8e64e3d8e0f38f882e8501f9e525df0a95d2e91e9cfc32c9248d756fb07780e2f8fdca2c5a54441e6fcd8d774a5f6aae72f36d1c76bc19f691a0d4f6c607e8cc",
    },
    NonceHashKv {
        header: "0101010101010101010101010101010101010101010101010101010101010101",
        nonce: "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001",
        hash: "e4ef79db80b642a093d7e38e6a6daac7ec1cca7293ddbe3710b4e6781be4add0b93f7e3440626df5bb23b757436787ba8d3fd0a2652e387f8af5bd8a5cc2ce2f",
    },
    NonceHashKv {
        header: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        nonce: "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001234567890abcdef",
        hash: "7f6c410ce62e7fc54811ad3b8b92664ac0f8b1d2cd6eb163129251146e7ee34d27794aaae1b31eaaeb98ab835bae6a2b9ac4ad6f08c95903e1423f0b865816ff",
    },
    NonceHashKv {
        header: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        nonce: "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000ffffffffffffffff",
        hash: "822f70999ee517f610b51bbb79ba88bfbb19cb77b316df25d8aeb4c2682673555e9fc6a588ee96dcce4ff0d2b8d97f4aa89d0f46c97f9027b386d42b14be5100",
    },
    NonceHashKv {
        header: "0707070707070707070707070707070707070707070707070707070707070707",
        nonce: "00000000000000000000000000000000000000000deadbeefcafe000000000000000000000000000000000000000000000000000000000001234567890abcdef",
        hash: "eab81cdb33db293aef7e1f10f85cb08d17bc197c284d46a25d9dc57cdb7ba878adf3f5c354318121876795aab08c0941afda21d835383faa2f139c26be31a30a",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn decode32(s: &str) -> [u8; 32] {
        hex::decode(s).unwrap().try_into().unwrap()
    }

    fn decode64(s: &str) -> [u8; 64] {
        hex::decode(s).unwrap().try_into().unwrap()
    }

    #[test]
    fn nonce_hash_golden_vectors() {
        for (i, v) in NONCE_HASH_KVS.iter().enumerate() {
            let header = decode32(v.header);
            let nonce_be = decode64(v.nonce);
            assert_eq!(
                get_nonce_hash(header, nonce_be).to_big_endian(),
                decode64(v.hash),
                "kv {i}: get_nonce_hash mismatch"
            );
        }
    }

    #[test]
    fn permute_is_the_crates() {
        let mut words = [7u64; SPONGE_WIDTH];
        let mut felts = words.map(Goldilocks::from_u64);
        Poseidon2::new().permute_mut(&mut felts);
        permute(&mut words);
        assert_eq!(words, felts.map(|g| g.as_canonical_u64()));
    }

    #[test]
    fn first_squeeze_is_hash_bytes() {
        let input = [0x5au8; 96];
        let twice = qp_poseidon_core::hash_squeeze_twice(&input);
        assert_eq!(twice[..32], qp_poseidon_core::hash_bytes(&input));
    }

    #[test]
    fn u512_limb_roundtrip() {
        let n = (U512::from(0xdeadbeefcafeu64) << 300) | U512::from(0x1234567890abcdefu64);
        assert_eq!(u512_from_le_u32s(u512_to_le_u32s(n)), n);
    }

    #[test]
    fn target_is_max_over_difficulty() {
        assert_eq!(target_from_difficulty(U512::zero()), None);
        assert_eq!(target_from_difficulty(U512::one()), Some(U512::MAX));
        assert_eq!(
            target_from_difficulty(U512::from(18_253_611_008u64)),
            Some(U512::MAX / U512::from(18_253_611_008u64))
        );
    }
}
