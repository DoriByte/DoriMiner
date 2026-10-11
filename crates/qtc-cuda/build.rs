//! The public tree's build: it carries the kernel images sealed (see `src/seal.rs` and the README) and
//! copies their table to where the crate includes it. No CUDA Toolkit, no compiler but Rust's.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=kernels/sealed");
    println!("cargo:rustc-check-cfg=cfg(qtc_sealed)");
    println!("cargo:rustc-cfg=qtc_sealed");
    let table = std::fs::read_to_string(Path::new("kernels/sealed").join("kernels.rs"))
        .expect("kernels/sealed/kernels.rs is missing: this tree carries no kernel");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    std::fs::write(out_dir.join("kernels.rs"), table).unwrap();
}
