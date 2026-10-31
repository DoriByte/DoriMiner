# Pearl's official proof and verifier

Three parts of [pearl-research-labs/pearl](https://github.com/pearl-research-labs/pearl), at commit
`2f8b770cac8f8b74be05c1fc51c2baeb76ce0701`, copied unchanged (the build folders and plonky2's `.github` and
`audits` left out). `crates/prl-pow` uses them to prove every Pearl share and check it with the official verifier
before the miner sends it.

| Folder | What | Licence |
|---|---|---|
| `zk-pow` | the plain proof and its verifier | ISC (`LICENSE` here, the repository's) |
| `pearl-blake3` | the Merkle tree over BLAKE3 the proof uses | ISC (`LICENSE` here) |
| `plonky2` | Pearl's copy of plonky2, which `zk-pow` builds on | MIT or Apache-2.0 (`plonky2/LICENSE-MIT`, `plonky2/LICENSE-APACHE`) |

Copied on 2026-10-18. They are here and not a git dependency because the repository's
CUTLASS submodule has paths longer than Windows allows, and so that the miner builds without the network and
against the commit its CPU twin was checked with. Nothing in these folders is changed; a change would be marked.
