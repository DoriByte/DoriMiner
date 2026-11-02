use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(qtc_sealed)");
    println!("cargo:rustc-cfg=qtc_sealed");
    let os = if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") { "windows" } else { "linux" };
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("kernels").join("sealed").join(os);
    println!("cargo:rerun-if-changed=kernels/sealed/{os}");
    let table = std::fs::read_to_string(dir.join("SHA256SUMS")).unwrap_or_else(|e| panic!("kernels/sealed/{os}/SHA256SUMS: {e}"));
    for line in table.lines().filter(|l| !l.trim().is_empty()) {
        let (sha, name) = line.split_once("  ").expect("a SHA256SUMS line");
        let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("kernels/sealed/{os}/{name}: {e}"));
        use sha2::Digest;
        let got: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(got, sha, "kernels/sealed/{os}/{name} is not the sealed file");
    }
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static=qtc");
    if os == "linux" {
        for l in ["dl", "pthread"] {
            println!("cargo:rustc-link-lib={l}");
        }
    }
}
