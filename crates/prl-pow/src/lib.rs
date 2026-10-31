//! Published without its comments: they describe the kernel, which is closed for now (see the README).
//! The code is all here.


pub mod twin;
pub use twin::*;

use blake3::hazmat::{merge_subtrees_non_root, merge_subtrees_root, HasherExt, Mode};
use std::sync::Arc;

pub const RANK: usize = 128;
const CHUNK: usize = 1024;
pub const MAX_BYTE: i8 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub m: usize,
    pub n: usize,
    pub k: usize,
    pub na: usize,
    pub h: u32,
}

impl Shape {
    pub fn config(&self) -> Configuracao {
        Configuracao {
            k: self.k as u32,
            rank: RANK as u16,
            linhas: Padrao::de_lista(&(0..self.h).map(|i| 8 * i).collect::<Vec<_>>()).expect("tile rows"),
            colunas: Padrao::de_lista(&(0..self.na as u32).flat_map(|j| [8 * j, 8 * j + 1]).collect::<Vec<_>>())
                .expect("tile columns"),
        }
    }

    pub fn check(&self) -> Result<(), String> {
        if self.m % 32 != 0 || self.m > 65536 || self.n % (8 * self.na) != 0 || self.n > 65536 || self.k % 2048 != 0 {
            return Err(format!("shape {self:?}: m a multiple of 32, n of 8 * na, k of 2048, m and n at most 65536"));
        }
        if (self.m * self.k) % CHUNK != 0 || self.m * self.k / CHUNK < 2 {
            return Err(format!("shape {self:?}: A must be at least two whole chunks of {CHUNK} bytes"));
        }
        forma_legal(&self.config(), self.m as u32, self.n as u32, true)
    }

    pub fn products(&self) -> u64 {
        (self.m * self.n * self.k) as u64
    }

    pub fn tile(&self) -> (Vec<u32>, Vec<u32>) {
        let c = self.config();
        (c.linhas.indices(), c.colunas.indices())
    }
}

fn de_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.trim().trim_start_matches("0x");
    if text.len() != 2 * N || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolJob {
    pub id: String,
    pub header: [u8; 76],
    pub target_be: [u8; 32],
}

impl PoolJob {
    pub fn parse(id: &str, header_hex: &str, target_hex: &str) -> Result<Self, String> {
        let header = de_hex::<76>(header_hex).ok_or("the job's header is not 152 hex characters")?;
        let target_be = de_hex::<32>(target_hex).ok_or("the job's target is not 64 hex characters")?;
        if id.is_empty() {
            return Err("the job has no id".into());
        }
        Ok(Self { id: id.to_string(), header, target_be })
    }
}

pub fn compact(be: &[u8; 32]) -> u32 {
    let i = be.iter().position(|&b| b != 0).unwrap_or(31);
    let mut size = (32 - i) as u32;
    let byte = |j: usize| *be.get(i + j).unwrap_or(&0) as u32;
    let mut mantissa = (byte(0) << 16) | (byte(1) << 8) | byte(2);
    if mantissa & 0x80_0000 != 0 {
        mantissa >>= 8;
        size += 1;
    }
    (size << 24) | mantissa
}

pub fn threads() -> usize {
    std::thread::available_parallelism().map_or(8, |n| n.get()).min(16)
}

fn in_parallel<T: Send>(total: usize, f: impl Fn(usize, usize) -> Vec<T> + Sync) -> Vec<T> {
    let (nt, f) = (threads(), &f);
    let each = total.div_ceil(nt).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..nt)
            .map(|i| {
                s.spawn(move || {
                    let (from, to) = ((i * each).min(total), ((i + 1) * each).min(total));
                    if from < to { f(from, to) } else { Vec::new() }
                })
            })
            .collect();
        let parts: Vec<Vec<T>> = handles.into_iter().map(|h| h.join().expect("a worker thread panicked")).collect();
        let mut out = Vec::with_capacity(parts.iter().map(Vec::len).sum());
        for mut part in parts {
            out.append(&mut part);
        }
        out
    })
}

fn noise_of_all(side_a: bool, seed: &[u8; 32], rows: usize, k: usize) -> Vec<i8> {
    in_parallel(rows, |from, to| ruido(side_a, seed, &(from..to).collect::<Vec<_>>(), k, RANK).valores)
}

pub fn words32(b: &[u8; 32]) -> [u32; 8] {
    let mut o = [0u32; 8];
    for (i, w) in o.iter_mut().enumerate() {
        *w = u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
    }
    o
}

pub struct JobSide {
    pub job: PoolJob,
    pub shape: Shape,
    pub limit: U256,
    pub key: [u8; 32],
    pub root_b: [u8; 32],
    pub seed_b: [u8; 32],
    pub b_noisy: Vec<i8>,
    pub pb: Vec<u32>,
}

impl JobSide {
    pub fn new(job: PoolJob, shape: Shape) -> Result<Self, String> {
        let h = &job.header;
        let reversed = |range: std::ops::Range<usize>| {
            let mut x: [u8; 32] = h[range].try_into().unwrap();
            x.reverse();
            x
        };
        let header = Cabecalho {
            versao: u32::from_le_bytes(h[0..4].try_into().unwrap()),
            bloco_anterior: reversed(4..36),
            raiz_transacoes: reversed(36..68),
            tempo: u32::from_le_bytes(h[68..72].try_into().unwrap()),
            nbits: u32::from_le_bytes(h[72..76].try_into().unwrap()),
        };
        if header.bytes() != *h {
            return Err("the twin's header is not the pool's 76 bytes".into());
        }
        zk_pow::api::proof::IncompleteBlockHeader::from_bytes(h).map_err(|e| format!("the job's header: {e}"))?;
        let cfg = shape.config();
        let mut le = job.target_be;
        le.reverse();
        let limit = U256::de_bytes_le(&le)
            .vezes(cfg.fator_penalizado())
            .ok_or("the pool's target times the tile's factor does not fit in 256 bits")?;
        let key = chave_do_trabalho(&header, &cfg);
        let root_b = raiz(&vec![0i8; shape.n * shape.k], &key);
        let (seed_b, _) = sementes(&key, &[0; 32], &root_b, shape.m as u32, shape.n as u32, true);
        let b_noisy = soma_ruido(&vec![0i8; shape.n * shape.k], &noise_of_all(false, &seed_b, shape.n, shape.k));
        let mut blocks = 0u64;
        let pb = pares(shape.k, RANK, b"B_tensor", &seed_b, &mut blocks).iter().map(|&(p, q)| p | (q << 16)).collect();
        Ok(Self { job, shape, limit, key, root_b, seed_b, b_noisy, pb })
    }

}

struct Node {
    from: usize,
    to: usize,
    children: Option<(usize, usize)>,
    cv: [u8; 32],
}

pub struct ASide {
    key: [u8; 32],
    unit: usize,
    leaves: usize,
    nodes: Vec<Node>,
    byte: Option<(usize, i8)>,
    zeros: Vec<u8>,
}

const UNIT: usize = 64;

impl ASide {
    pub fn new(shape: &Shape, key: &[u8; 32]) -> Self {
        let chunks = shape.m * shape.k / CHUNK;
        Self::with_unit(chunks, key, if chunks % UNIT == 0 && chunks >= 2 * UNIT { UNIT } else { 1 })
    }

    fn with_unit(chunks: usize, key: &[u8; 32], unit_chunks: usize) -> Self {
        let (unit, leaves) = (unit_chunks * CHUNK, chunks / unit_chunks);
        let cvs: Vec<[u8; 32]> = in_parallel(leaves, |from, to| {
            let mut zeros = vec![0u8; unit];
            (from..to).map(|i| Self::leaf(key, unit, i, None, &mut zeros)).collect()
        });
        let mut nodes = vec![Node { from: 0, to: leaves, children: None, cv: [0; 32] }];
        Self::build(&mut nodes, 0, &cvs, key);
        Self { key: *key, unit, leaves, nodes, byte: None, zeros: vec![0u8; unit] }
    }

    fn leaf(key: &[u8; 32], unit: usize, i: usize, byte: Option<(usize, i8)>, zeros: &mut [u8]) -> [u8; 32] {
        let at = byte.filter(|&(pos, _)| pos / unit == i).map(|(pos, val)| (pos % unit, val));
        if let Some((p, val)) = at {
            zeros[p] = val as u8;
        }
        let mut h = blake3::Hasher::new_keyed(key);
        h.set_input_offset((i * unit) as u64);
        h.update(zeros);
        if let Some((p, _)) = at {
            zeros[p] = 0;
        }
        h.finalize_non_root()
    }

    fn left(c: usize) -> usize {
        1 << (usize::BITS - 1 - (c - 1).leading_zeros())
    }

    fn build(nodes: &mut Vec<Node>, me: usize, leaves: &[[u8; 32]], key: &[u8; 32]) {
        let (from, to) = (nodes[me].from, nodes[me].to);
        if to - from == 1 {
            nodes[me].cv = leaves[from];
            return;
        }
        let mid = from + Self::left(to - from);
        let (l, r) = (nodes.len(), nodes.len() + 1);
        nodes.push(Node { from, to: mid, children: None, cv: [0; 32] });
        nodes.push(Node { from: mid, to, children: None, cv: [0; 32] });
        nodes[me].children = Some((l, r));
        Self::build(nodes, l, leaves, key);
        Self::build(nodes, r, leaves, key);
        nodes[me].cv = merge_subtrees_non_root(&nodes[l].cv, &nodes[r].cv, Mode::KeyedHash(key));
    }

    fn redo(&mut self, i: usize) {
        let mut path = vec![0usize];
        while let Some((l, r)) = self.nodes[*path.last().unwrap()].children {
            path.push(if i < self.nodes[l].to { l } else { r });
        }
        let leaf = path.pop().unwrap();
        self.nodes[leaf].cv = Self::leaf(&self.key, self.unit, i, self.byte, &mut self.zeros);
        for &node in path.iter().rev() {
            let (l, r) = self.nodes[node].children.unwrap();
            self.nodes[node].cv = merge_subtrees_non_root(&self.nodes[l].cv, &self.nodes[r].cv, Mode::KeyedHash(&self.key));
        }
    }

    pub fn set_byte(&mut self, pos: usize, val: i8) {
        assert!(pos < self.leaves * self.unit && (1..=MAX_BYTE).contains(&val));
        let old = self.byte.replace((pos, val));
        if let Some((q, _)) = old.filter(|&(q, _)| q / self.unit != pos / self.unit) {
            self.redo(q / self.unit);
        }
        self.redo(pos / self.unit);
    }

    pub fn root(&self) -> [u8; 32] {
        let (l, r) = self.nodes[0].children.unwrap();
        *merge_subtrees_root(&self.nodes[l].cv, &self.nodes[r].cv, Mode::KeyedHash(&self.key)).as_bytes()
    }
}

pub struct Pair {
    pub pos: usize,
    pub val: i8,
    pub seed_a: [u8; 32],
    pub pm: Vec<u32>,
}

impl Pair {
    pub fn new(job: &JobSide, a: &mut ASide, pos: usize, val: i8) -> Self {
        let s = &job.shape;
        a.set_byte(pos, val);
        let (seed_b, seed_a) = sementes(&job.key, &a.root(), &job.root_b, s.m as u32, s.n as u32, true);
        debug_assert_eq!(seed_b, job.seed_b, "the seed of B moved with A");
        let mut blocks = 0u64;
        let pm = pares(s.k, RANK, b"A_tensor", &seed_a, &mut blocks).iter().map(|&(p, q)| p | (q << 16)).collect();
        Self { pos, val, seed_a, pm }
    }

}

pub fn uniform_rows(seed: &[u8; 32], label: &[u8; 8], first: u32, count: u32, out: &mut [u8]) {
    for (r, row) in (first as usize..).zip(out.chunks_exact_mut(RANK).take(count as usize)) {
        for q in 0..4 {
            let mut msg = [0u8; 64];
            msg[0..4].copy_from_slice(&((r * 4 + q + 1) as i32).to_le_bytes());
            msg[32..40].copy_from_slice(label);
            for (x, &v) in blake3::keyed_hash(seed, &msg).as_bytes().iter().enumerate() {
                row[q * 32 + x] = v & 63;
            }
        }
    }
}

#[derive(Clone)]
pub struct Candidate {
    pub job: Arc<JobSide>,
    pub pos: usize,
    pub val: i8,
    pub seed_a: [u8; 32],
    pub row: u32,
    pub col: u32,
}

#[derive(Debug)]
pub enum ShareError {
    TwinSaysNo,
    VerifierRefuses(String),
    Other(String),
}

impl std::fmt::Display for ShareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShareError::TwinSaysNo => write!(f, "the CPU twin says the tile does not win"),
            ShareError::VerifierRefuses(e) => write!(f, "the official verifier refuses the proof: {e}"),
            ShareError::Other(e) => write!(f, "{e}"),
        }
    }
}

pub fn twin_wins(c: &Candidate) -> bool {
    let (job, s) = (&c.job, &c.job.shape);
    let (rows, cols) = s.tile();
    let a_rows: Vec<usize> = rows.iter().map(|&d| (c.row + d) as usize).collect();
    let noise = ruido(true, &c.seed_a, &a_rows, s.k, RANK).valores;
    let a: Vec<Vec<i8>> = a_rows
        .iter()
        .enumerate()
        .map(|(x, &l)| {
            let mut row = vec![0i8; s.k];
            if c.pos / s.k == l {
                row[c.pos % s.k] = c.val;
            }
            soma_ruido(&row, &noise[x * s.k..(x + 1) * s.k])
        })
        .collect();
    let la: Vec<&[i8]> = a.iter().map(|v| &v[..]).collect();
    let lb: Vec<&[i8]> = cols.iter().map(|&d| (c.col + d) as usize).map(|x| &job.b_noisy[x * s.k..(x + 1) * s.k]).collect();
    let mut count = Contagem::default();
    let h = hash_do_transcrito(&transcrito(&la, &lb, s.k, RANK, &mut count), &c.seed_a, &mut count);
    ganha(&h, &job.limit)
}

fn merkle_proof(plain: &[u8], key: &[u8; 32], rows: &[usize], num_rows: usize, k: usize) -> zk_pow::ffi::plain_proof::MatrixMerkleProof {
    let full = pearl_blake3::pad_to_chunk_boundary(plain);
    let tree = pearl_blake3::MerkleTree::new(&full, *key);
    let leaves = pearl_blake3::MerkleTree::compute_leaf_indices_from_rows(rows, (num_rows, k));
    zk_pow::ffi::plain_proof::MatrixMerkleProof { proof: tree.get_multileaf_proof(&leaves), row_indices: rows.to_vec() }
}

pub fn check_share(c: &Candidate) -> Result<Vec<u8>, ShareError> {
    if !twin_wins(c) {
        return Err(ShareError::TwinSaysNo);
    }
    let (job, s) = (&c.job, &c.job.shape);
    let (rows, cols) = s.tile();
    let a_rows: Vec<usize> = rows.iter().map(|&d| (c.row + d) as usize).collect();
    let b_rows: Vec<usize> = cols.iter().map(|&d| (c.col + d) as usize).collect();
    let mut a = vec![0u8; s.m * s.k];
    a[c.pos] = c.val as u8;
    let proof = zk_pow::ffi::plain_proof::PlainProof {
        m: s.m,
        n: s.n,
        k: s.k,
        noise_rank: RANK,
        a: merkle_proof(&a, &job.key, &a_rows, s.m, s.k),
        bt: merkle_proof(&vec![0u8; s.n * s.k], &job.key, &b_rows, s.n, s.k),
        moe: None,
    };
    drop(a);
    let header = zk_pow::api::proof::IncompleteBlockHeader::from_bytes(&job.job.header).map_err(|e| ShareError::Other(format!("header: {e}")))?;
    zk_pow::api::verify::verify_plain_proof(&header, &proof, Some(compact(&job.job.target_be)), zk_pow::api::proof::SeedDerivation::Salted)
        .map_err(|e| ShareError::VerifierRefuses(e.to_string()))?;
    bincode::serialize(&proof).map_err(|e| ShareError::Other(format!("serialising the proof: {e}")))
}

pub fn base64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: Shape = Shape { m: 64, n: 256, k: 2048, na: 16, h: 4 };

    fn header(nbits: u32) -> String {
        let mut h = [0u8; 76];
        h[0..4].copy_from_slice(&1u32.to_le_bytes());
        for (i, b) in h[4..68].iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(37).wrapping_add(11);
        }
        h[68..72].copy_from_slice(&1_760_000_000u32.to_le_bytes());
        h[72..76].copy_from_slice(&nbits.to_le_bytes());
        h.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn the_tree_gives_the_twins_root_as_the_byte_moves() {
        let key = [7u8; 32];
        let mut a = ASide::new(&SMALL, &key);
        let mut plain = vec![0i8; SMALL.m * SMALL.k];
        assert_eq!(a.root(), raiz(&plain, &key));
        let mut last: Option<usize> = None;
        for (pos, val) in [(5usize, 1i8), (5, 2), (3000, 64), (3001, 9), (SMALL.m * SMALL.k - 1, 33)] {
            if let Some(q) = last {
                plain[q] = 0;
            }
            plain[pos] = val;
            last = Some(pos);
            a.set_byte(pos, val);
            assert_eq!(a.root(), raiz(&plain, &key), "byte at {pos}");
        }
    }

    #[test]
    fn leaves_of_64_chunks_give_the_same_tree() {
        let (chunks, key) = (448usize, [9u8; 32]);
        let mut by_unit = ASide::with_unit(chunks, &key, UNIT);
        let mut by_chunk = ASide::with_unit(chunks, &key, 1);
        let mut plain = vec![0i8; chunks * CHUNK];
        assert_eq!(by_unit.root(), by_chunk.root());
        assert_eq!(by_unit.root(), raiz(&plain, &key));
        let mut last: Option<usize> = None;
        for (pos, val) in [(0usize, 1i8), (100, 2), (65_535, 3), (65_536, 64), (300_000, 5), (chunks * CHUNK - 1, 33)] {
            if let Some(q) = last {
                plain[q] = 0;
            }
            plain[pos] = val;
            last = Some(pos);
            by_unit.set_byte(pos, val);
            by_chunk.set_byte(pos, val);
            assert_eq!(by_unit.root(), by_chunk.root(), "byte at {pos}");
            assert_eq!(by_unit.root(), raiz(&plain, &key), "byte at {pos}");
        }
    }

    #[test]
    fn a_winning_tile_of_this_shape_makes_a_share_the_official_verifier_accepts() {
        const SMALL6: Shape = Shape { m: 96, n: 256, k: 2048, na: 16, h: 6 };
        SMALL6.check().unwrap();
        assert_eq!(SMALL6.tile().0, vec![0, 8, 16, 24, 32, 40]);
        let target = format!("00000f{}", "ff".repeat(29));
        let job = Arc::new(JobSide::new(PoolJob::parse("j6", &header(0x1d00ffff), &target).unwrap(), SMALL6).unwrap());
        let mut a = ASide::new(&SMALL6, &job.key);
        let pair = Pair::new(&job, &mut a, 3 * 2048 + 77, 9);
        let (mut won, mut lost) = (0, 0);
        for row in [0u32, 1, 7, 48, 50, 55] {
            for col in (0..SMALL6.n as u32).step_by(2) {
                let c = Candidate { job: job.clone(), pos: pair.pos, val: pair.val, seed_a: pair.seed_a, row, col };
                if !twin_wins(&c) {
                    lost += 1;
                    continue;
                }
                let bytes = check_share(&c).unwrap_or_else(|e| panic!("tile {row},{col}: {e}"));
                assert!(!bytes.is_empty());
                won += 1;
                if won >= 2 && lost >= 1 {
                    return;
                }
            }
        }
        panic!("too few tiles to test: {won} won, {lost} lost");
    }

    #[test]
    fn compact_targets() {
        let mut be = [0u8; 32];
        be[6] = 0x03;
        be[7..].fill(0xff);
        assert_eq!(compact(&be), 0x1a03_ffff);
        let mut hi = [0u8; 32];
        hi[3] = 0x80;
        assert_eq!(compact(&hi), 0x1e00_8000);
    }

    #[test]
    fn jobs_are_read_and_bad_ones_refused() {
        let target = format!("000000{}", "ff".repeat(29));
        let job = PoolJob::parse("j1", &header(0x1d00ffff), &target).unwrap();
        assert_eq!(job.target_be[3], 0xff);
        assert!(PoolJob::parse("j1", "00", &target).is_err());
        assert!(PoolJob::parse("j1", &header(1), "zz").is_err());
        assert!(PoolJob::parse("", &header(1), &target).is_err());
    }

    #[test]
    fn a_winning_tile_makes_a_share_the_official_verifier_accepts() {
        SMALL.check().unwrap();
        let target = format!("00000f{}", "ff".repeat(29));
        let job = Arc::new(JobSide::new(PoolJob::parse("j1", &header(0x1d00ffff), &target).unwrap(), SMALL).unwrap());
        let mut a = ASide::new(&SMALL, &job.key);
        let pair = Pair::new(&job, &mut a, 12_345, 7);
        let (mut won, mut lost) = (0, 0);
        for row in (0..SMALL.m as u32).step_by(32) {
            for col in (0..SMALL.n as u32).step_by(8 * SMALL.na) {
                for (dr, dc) in [(0u32, 0u32), (1, 2), (2, 4), (3, 6), (4, 1)] {
                    let c = Candidate { job: job.clone(), pos: pair.pos, val: pair.val, seed_a: pair.seed_a, row: row + dr, col: col + dc };
                    match check_share(&c) {
                        Ok(bytes) => {
                            won += 1;
                            assert!(!bytes.is_empty());
                        }
                        Err(ShareError::TwinSaysNo) => lost += 1,
                        Err(e) => panic!("tile {},{}: {e}", c.row, c.col),
                    }
                    if won >= 2 && lost >= 1 {
                        return;
                    }
                }
            }
        }
        panic!("too few tiles to test: {won} won, {lost} lost");
    }
}
