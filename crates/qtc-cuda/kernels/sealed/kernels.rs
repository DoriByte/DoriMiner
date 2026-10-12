pub const KERNELS: &[(u32, &[u8])] = &[
    (75, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/kernels/sealed/mining_sm75_0.dkn"))),
    (86, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/kernels/sealed/mining_sm86_0.dkn"))),
    (86, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/kernels/sealed/mining_sm86_1.dkn"))),
    (120, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/kernels/sealed/mining_sm120_0.dkn"))),
    (120, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/kernels/sealed/mining_sm120_1.dkn"))),
];
pub const KERNEL_WALK2: &[(u32, bool)] = &[
    (75, false),
    (86, true),
    (120, true),
];
