//! Quantus QPoW on the CPU: the exact reference hash, and the sponge states the
//! GPU kernel resumes from.
//!
//! Derived from `crates/pow-core/src/lib.rs` of Quantus-Network/quantus-miner
//! (commit c7838cbc, MIT OR Apache-2.0). Changes: the reference hash is built
//! directly on `qp-poseidon-core` instead of the `qpow-math` git dependency, and
//! the node-only helpers (job context, CPU mining loop) are gone.

use primitive_types::U512;
use qp_poseidon_core::poseidon2::INITIAL_EXTERNAL_CONSTANTS;
use qp_poseidon_core::{Goldilocks, Poseidon2};

pub use qp_poseidon_core::SPONGE_WIDTH;

/// The QPoW hash: Poseidon2 squeezed twice over `header || nonce`, read as a
/// big-endian 512-bit integer. This is the exact function the chain verifies.
pub fn get_nonce_hash(header: [u8; 32], nonce_be: [u8; 64]) -> U512 {
    let mut input = [0u8; 96];
    input[..32].copy_from_slice(&header);
    input[32..].copy_from_slice(&nonce_be);
    U512::from_big_endian(&qp_poseidon_core::hash_squeeze_twice(&input))
}

/// A nonce is valid when its hash is strictly below this.
pub fn target_from_difficulty(difficulty: U512) -> Option<U512> {
    (!difficulty.is_zero()).then(|| U512::MAX / difficulty)
}

/// Split a U512 into 16 little-endian u32 limbs (the layout the GPU kernel uses).
pub fn u512_to_le_u32s(n: U512) -> [u32; 16] {
    let bytes = n.to_little_endian();
    let mut out = [0u32; 16];
    for i in 0..16 {
        out[i] = u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
    }
    out
}

/// Rebuild a U512 from 16 little-endian u32 limbs.
pub fn u512_from_le_u32s(limbs: [u32; 16]) -> U512 {
    let mut bytes = [0u8; 64];
    for i in 0..16 {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&limbs[i].to_le_bytes());
    }
    U512::from_little_endian(&bytes)
}

/// 12 felts packed into 24 little-endian u32 limbs.
pub fn felts_to_u32s(felts: [u64; SPONGE_WIDTH]) -> [u32; 24] {
    let mut out = [0u32; 24];
    for (i, felt) in felts.iter().enumerate() {
        out[2 * i] = *felt as u32;
        out[2 * i + 1] = (*felt >> 32) as u32;
    }
    out
}

/// Sponge state (canonical u64 felts) after absorbing the 32-byte header and the
/// high 32 bytes of the big-endian nonce: the first two of the five Poseidon2
/// permutations of `get_nonce_hash`. It is the same for every nonce that shares
/// those 32 bytes.
pub fn mining_midstate(header: [u8; 32], nonce_high_be: [u8; 32]) -> [u64; SPONGE_WIDTH] {
    let poseidon2 = Poseidon2::new();
    let mut state = [Goldilocks::ZERO; SPONGE_WIDTH];
    for (i, chunk) in header.chunks_exact(4).enumerate() {
        state[i] += Goldilocks::from_u64(u32::from_le_bytes(chunk.try_into().unwrap()) as u64);
    }
    poseidon2.permute_mut(&mut state);
    for (i, chunk) in nonce_high_be.chunks_exact(4).enumerate() {
        state[i] += Goldilocks::from_u64(u32::from_le_bytes(chunk.try_into().unwrap()) as u64);
    }
    poseidon2.permute_mut(&mut state);
    state.map(|g| g.as_canonical_u64())
}

pub fn mining_prestate_low64(header: [u8; 32], nonce_be: [u8; 64]) -> [u64; SPONGE_WIDTH] {
    let mut state =
        mining_midstate(header, nonce_be[..32].try_into().unwrap()).map(Goldilocks::from_u64);
    for (i, chunk) in nonce_be[32..56].chunks_exact(4).enumerate() {
        state[i] += Goldilocks::from_u64(u32::from_le_bytes(chunk.try_into().unwrap()) as u64);
    }
    external_linear_layer(&mut state);
    for (felt, constant) in state.iter_mut().zip(INITIAL_EXTERNAL_CONSTANTS[0]) {
        *felt += Goldilocks::from_u64(constant);
    }
    state.map(|felt| felt.as_canonical_u64())
}

fn external_linear_layer(state: &mut [Goldilocks; SPONGE_WIDTH]) {
    for chunk in state.chunks_exact_mut(4) {
        let chunk: &mut [Goldilocks; 4] = chunk.try_into().unwrap();
        let t01 = chunk[0] + chunk[1];
        let t23 = chunk[2] + chunk[3];
        let t0123 = t01 + t23;
        let t01123 = t0123 + chunk[1];
        let t01233 = t0123 + chunk[3];
        chunk[3] = t01233 + chunk[0] + chunk[0];
        chunk[1] = t01123 + chunk[2] + chunk[2];
        chunk[0] = t01123 + t01;
        chunk[2] = t01233 + t23;
    }

    let sums: [Goldilocks; 4] =
        std::array::from_fn(|offset| (offset..SPONGE_WIDTH).step_by(4).map(|i| state[i]).sum());
    for (i, felt) in state.iter_mut().enumerate() {
        *felt += sums[i % 4];
    }
}

pub const SPARSE_DIR: [i64; 8] = [17, -11, 3, -4, -17, 11, -3, 4];
pub const SPARSE_STEP: u64 = 35;
pub const SPARSE_BASE: u32 = 0x4000_0000;
pub const SPARSE_MAX_INDEX: u32 = 60_000_000;
pub const SPARSE_MAX_LAUNCH: u32 = 1 << 30;

pub fn sparse_low_felts(launch: u32, index: u32) -> [u32; 8] {
    assert!(launch < SPARSE_MAX_LAUNCH && index < SPARSE_MAX_INDEX);
    let mut felts = [0u32; 8];
    for (felt, step) in felts.iter_mut().zip(SPARSE_DIR) {
        *felt = (SPARSE_BASE as i64 + step * index as i64) as u32;
    }
    felts[0] += launch;
    felts
}

pub fn sparse_nonce_low(launch: u32, index: u32) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for (chunk, felt) in bytes
        .chunks_exact_mut(4)
        .zip(sparse_low_felts(launch, index))
    {
        chunk.copy_from_slice(&felt.to_le_bytes());
    }
    bytes
}

pub fn launch_inputs(midstate: [u64; SPONGE_WIDTH], launch: u32) -> [u64; SPONGE_WIDTH] {
    let mut state = midstate.map(Goldilocks::from_u64);
    for (felt, low) in state.iter_mut().zip(sparse_low_felts(launch, 0)) {
        *felt += Goldilocks::from_u64(low as u64);
    }
    external_linear_layer(&mut state);
    for (felt, constant) in state.iter_mut().zip(INITIAL_EXTERNAL_CONSTANTS[0]) {
        *felt += Goldilocks::from_u64(constant);
    }
    state.map(|felt| felt.as_canonical_u64())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SparsePrestate {
    pub lane3: u64,
    pub lane7: u64,
    pub round1: [u64; SPONGE_WIDTH],
}

pub fn sparse_prestate(midstate: [u64; SPONGE_WIDTH], launch: u32) -> SparsePrestate {
    let mut state = midstate.map(Goldilocks::from_u64);
    for (felt, low) in state.iter_mut().zip(sparse_low_felts(launch, 0)) {
        *felt += Goldilocks::from_u64(low as u64);
    }
    external_linear_layer(&mut state);
    for (felt, constant) in state.iter_mut().zip(INITIAL_EXTERNAL_CONSTANTS[0]) {
        *felt += Goldilocks::from_u64(constant);
    }
    let (lane3, lane7) = (state[3].as_canonical_u64(), state[7].as_canonical_u64());
    for (i, felt) in state.iter_mut().enumerate() {
        *felt = if i == 3 || i == 7 {
            Goldilocks::ZERO
        } else {
            felt.exp7()
        };
    }
    external_linear_layer(&mut state);
    for (felt, constant) in state.iter_mut().zip(INITIAL_EXTERNAL_CONSTANTS[1]) {
        *felt += Goldilocks::from_u64(constant);
    }
    SparsePrestate {
        lane3,
        lane7,
        round1: state.map(|felt| felt.as_canonical_u64()),
    }
}

pub const WALK2_DIR: [i64; 8] = [-11, 3, -4, 17, 11, -3, 4, -17];
pub const WALK2_MAX_PER_THREAD: u32 = 32;
pub const WALK2_ROW: usize = 9;
pub const WALK2_ROW_LANES: [usize; WALK2_ROW] = [0, 2, 3, 4, 6, 7, 8, 10, 11];

pub fn walk2_low_felts(launch: u32, thread: u32, step: u32) -> [u32; 8] {
    assert!(launch < SPARSE_MAX_LAUNCH && thread < SPARSE_MAX_INDEX && step < SPARSE_MAX_INDEX);
    let mut felts = [0u32; 8];
    for (i, felt) in felts.iter_mut().enumerate() {
        let offset = SPARSE_DIR[i] * step as i64 + WALK2_DIR[i] * thread as i64;
        *felt = (SPARSE_BASE as i64 + offset) as u32;
    }
    felts[0] += launch;
    felts
}

pub fn walk2_nonce_low(launch: u32, thread: u32, step: u32) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for (chunk, felt) in bytes
        .chunks_exact_mut(4)
        .zip(walk2_low_felts(launch, thread, step))
    {
        chunk.copy_from_slice(&felt.to_le_bytes());
    }
    bytes
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Walk2Prestate {
    pub lane2: u64,
    pub lane6: u64,
    pub lane3: u64,
    pub lane7: u64,
    pub round1: [u64; SPONGE_WIDTH],
}

pub fn walk2_prestate(midstate: [u64; SPONGE_WIDTH], launch: u32) -> Walk2Prestate {
    let mut state = midstate.map(Goldilocks::from_u64);
    for (felt, low) in state.iter_mut().zip(walk2_low_felts(launch, 0, 0)) {
        *felt += Goldilocks::from_u64(low as u64);
    }
    external_linear_layer(&mut state);
    for (felt, constant) in state.iter_mut().zip(INITIAL_EXTERNAL_CONSTANTS[0]) {
        *felt += Goldilocks::from_u64(constant);
    }
    let input = state.map(|felt| felt.as_canonical_u64());
    for (i, felt) in state.iter_mut().enumerate() {
        *felt = if matches!(i, 2 | 3 | 6 | 7) {
            Goldilocks::ZERO
        } else {
            felt.exp7()
        };
    }
    external_linear_layer(&mut state);
    for (felt, constant) in state.iter_mut().zip(INITIAL_EXTERNAL_CONSTANTS[1]) {
        *felt += Goldilocks::from_u64(constant);
    }
    Walk2Prestate {
        lane2: input[2],
        lane6: input[6],
        lane3: input[3],
        lane7: input[7],
        round1: state.map(|felt| felt.as_canonical_u64()),
    }
}

impl Walk2Prestate {
    pub fn u(&self, step: u32) -> [u64; SPONGE_WIDTH] {
        let d = Goldilocks::from_u64(SPARSE_STEP * step as u64);
        let mut state = [Goldilocks::ZERO; SPONGE_WIDTH];
        state[3] = (Goldilocks::from_u64(self.lane3) + d).exp7();
        state[7] = (Goldilocks::from_u64(self.lane7) - d).exp7();
        external_linear_layer(&mut state);
        state.map(|felt| felt.as_canonical_u64())
    }

    pub fn row(&self, step: u32) -> [u64; WALK2_ROW] {
        let u = self.u(step);
        WALK2_ROW_LANES.map(|lane| u[lane])
    }
}

/// Format a hash rate with appropriate units (H/s, KH/s, MH/s, GH/s).
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

/// Hardcoded `get_nonce_hash` / midstate vectors, taken from upstream where they
/// are checked against the chain's own `qpow-math`.
pub struct NonceHashKv {
    pub header: &'static str,
    pub nonce: &'static str,
    pub hash: &'static str,
    pub mid: [u64; 12],
}

pub const NONCE_HASH_KVS: &[NonceHashKv] = &[
    NonceHashKv {
        header: "0000000000000000000000000000000000000000000000000000000000000000",
        nonce: "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        hash: "8e64e3d8e0f38f882e8501f9e525df0a95d2e91e9cfc32c9248d756fb07780e2f8fdca2c5a54441e6fcd8d774a5f6aae72f36d1c76bc19f691a0d4f6c607e8cc",
        mid: [
            0x8646d336b5a0fccd,
            0x818ca59548916345,
            0x49475e8bc9c928bc,
            0xc469163bcdc4900b,
            0xe4421fafacd59087,
            0x633f49b998698d66,
            0x4a9067e3883e76da,
            0x193039e85ea14472,
            0x203d2d3ee6a8eba8,
            0xef5b9ce39bcff072,
            0xc16a3c8dd5680003,
            0xd77c52ab5bf7cf1f,
        ],
    },
    NonceHashKv {
        header: "0101010101010101010101010101010101010101010101010101010101010101",
        nonce: "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001",
        hash: "e4ef79db80b642a093d7e38e6a6daac7ec1cca7293ddbe3710b4e6781be4add0b93f7e3440626df5bb23b757436787ba8d3fd0a2652e387f8af5bd8a5cc2ce2f",
        mid: [
            0x788cf743676a5c85,
            0xdce1d727e1189b53,
            0x36e0e019f5a0bf3a,
            0x806487dd9c3bf83f,
            0xcb6d7300de1ab5fe,
            0x7dbe191fa64fc6b7,
            0x95dedc7fe99861be,
            0xb1a3885303f7467d,
            0x891eb002a34a0f17,
            0x4be4cf057a87208a,
            0xe9113e7dc541745f,
            0xfaeda908097d00f1,
        ],
    },
    NonceHashKv {
        header: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        nonce: "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001234567890abcdef",
        hash: "7f6c410ce62e7fc54811ad3b8b92664ac0f8b1d2cd6eb163129251146e7ee34d27794aaae1b31eaaeb98ab835bae6a2b9ac4ad6f08c95903e1423f0b865816ff",
        mid: [
            0xf6fc848ab2dddc18,
            0xfd88d1f0ccda99fe,
            0x27cfa43b8ab9947a,
            0x6a505b37f02183d3,
            0x5f98f1d3524fb0c6,
            0xd6de4efd98793e2e,
            0xf3f205bd1e4d4526,
            0xc0c9046201e91dd6,
            0xcd2d28eef32b18de,
            0xa303ecbc8e98000c,
            0x3db245e7738f3aca,
            0xbc55f7553023863e,
        ],
    },
    NonceHashKv {
        header: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        nonce: "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000ffffffffffffffff",
        hash: "822f70999ee517f610b51bbb79ba88bfbb19cb77b316df25d8aeb4c2682673555e9fc6a588ee96dcce4ff0d2b8d97f4aa89d0f46c97f9027b386d42b14be5100",
        mid: [
            0xd1fe93daa1f07409,
            0x0ea440f2b57a0ecf,
            0x3b7a1f3cb2d48e39,
            0x43a5a1255658d1b2,
            0xebe00ad8f0fe28c0,
            0xb5618e7c6d6072d1,
            0x158b7b19f80ec62c,
            0x329a91c8de5575c2,
            0xd1637d851a474212,
            0x2d927735708f3b7d,
            0x6219c7cf8ef5af51,
            0x802028c39ccd8f13,
        ],
    },
    NonceHashKv {
        header: "0707070707070707070707070707070707070707070707070707070707070707",
        nonce: "00000000000000000000000000000000000000000deadbeefcafe000000000000000000000000000000000000000000000000000000000001234567890abcdef",
        hash: "eab81cdb33db293aef7e1f10f85cb08d17bc197c284d46a25d9dc57cdb7ba878adf3f5c354318121876795aab08c0941afda21d835383faa2f139c26be31a30a",
        mid: [
            0x41885398f5b05a0d,
            0xb7f17cee3f70d242,
            0x307aecc5bca54c7e,
            0xb74164c0d6de7e91,
            0x0c0a9b27281c8181,
            0xd84ddf12fdddf0ee,
            0x88f8cdf552c0a15e,
            0x88da4cfbfbf92d22,
            0x8c344261349149db,
            0x0c0fd0d6a8444652,
            0x7a57b00736d58568,
            0x9ad40597f4fe2abc,
        ],
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
            let high: [u8; 32] = nonce_be[..32].try_into().unwrap();
            assert_eq!(
                mining_midstate(header, high),
                v.mid,
                "kv {i}: midstate mismatch"
            );
        }
    }

    /// The first 32 bytes of the PoW hash are the plain Poseidon2 hash.
    #[test]
    fn first_squeeze_is_hash_bytes() {
        let input = [0x5au8; 96];
        let twice = qp_poseidon_core::hash_squeeze_twice(&input);
        assert_eq!(twice[..32], qp_poseidon_core::hash_bytes(&input));
    }

    /// Resuming from the midstate and from the prestate both reach the reference hash.
    #[test]
    fn midstate_and_prestate_resume_to_full_hash() {
        let poseidon2 = Poseidon2::new();
        for v in NONCE_HASH_KVS {
            let header = decode32(v.header);
            let nonce_be = decode64(v.nonce);

            let mut state = mining_midstate(header, nonce_be[..32].try_into().unwrap())
                .map(Goldilocks::from_u64);
            for (i, chunk) in nonce_be[32..].chunks_exact(4).enumerate() {
                state[i] +=
                    Goldilocks::from_u64(u32::from_le_bytes(chunk.try_into().unwrap()) as u64);
            }
            external_linear_layer(&mut state);
            for (felt, constant) in state.iter_mut().zip(INITIAL_EXTERNAL_CONSTANTS[0]) {
                *felt += Goldilocks::from_u64(constant);
            }

            let mut low = [Goldilocks::ZERO; SPONGE_WIDTH];
            for (i, chunk) in nonce_be[56..].chunks_exact(4).enumerate() {
                low[6 + i] =
                    Goldilocks::from_u64(u32::from_le_bytes(chunk.try_into().unwrap()) as u64);
            }
            external_linear_layer(&mut low);
            let prestate = mining_prestate_low64(header, nonce_be);
            for i in 0..SPONGE_WIDTH {
                assert_eq!(
                    (Goldilocks::from_u64(prestate[i]) + low[i]).as_canonical_u64(),
                    state[i].as_canonical_u64()
                );
            }

            let mut state = mining_midstate(header, nonce_be[..32].try_into().unwrap())
                .map(Goldilocks::from_u64);
            for (i, chunk) in nonce_be[32..].chunks_exact(4).enumerate() {
                state[i] +=
                    Goldilocks::from_u64(u32::from_le_bytes(chunk.try_into().unwrap()) as u64);
            }
            poseidon2.permute_mut(&mut state);
            state[0] += Goldilocks::ONE;
            state[1] += Goldilocks::ONE;
            poseidon2.permute_mut(&mut state);
            let mut hash = [0u8; 64];
            for i in 0..4 {
                hash[i * 8..(i + 1) * 8]
                    .copy_from_slice(&state[i].as_canonical_u64().to_le_bytes());
            }
            poseidon2.permute_mut(&mut state);
            for i in 0..4 {
                hash[32 + i * 8..32 + (i + 1) * 8]
                    .copy_from_slice(&state[i].as_canonical_u64().to_le_bytes());
            }
            assert_eq!(
                U512::from_big_endian(&hash),
                get_nonce_hash(header, nonce_be)
            );
        }
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
