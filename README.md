# DoriMiner

An NVIDIA miner for Quantus (QTC, the QPoW algorithm: Poseidon2 over Goldilocks), Pearl (PRL, RTX 50, 40 and 30 cards) and
Ergo (ERG, Autolykos v2),
Vela (VELA, Proof-of-Inference, RTX 50, 40 and 30 cards),
by DoriByte.

**Status: test release for the RTX 30, 40 and 50 series.** Measured on a few cards so far: an RTX 5070 Ti, an
RTX 5080, an RTX 4090, an RTX 3090, an RTX 3070, an RTX 3060 Ti and an RTX 2070 Super. It also runs on the other
RTX 20 cards, with a slower kernel.

## Where we stand

**Transparency and trust.** The miner is free, and its source is here for anyone to read and build: the pool
client, what is sent to the pool and when, the dev fee and the wallet it goes to, the local API, the exact
hash on the CPU and every check of a share against it, and the coin's protocol. Please audit it. If you find something wrong, tell us.

**The kernels are open, from 0.1.10.** The programs that run on the GPU and the engines that prepare and launch them, for all
four coins, are here as source: `crates/qtc-cuda/kernels` and `engine`, `crates/prl-cuda/kernels` and `engine`,
`crates/vela-cuda/engine`, `crates/erg-cuda/engine`. Read them and audit them. They are the production code only.

**What runs is in the tree too.** `crates/qtc-cuda/kernels/images` and `crates/prl-cuda/kernels/images` hold the final GPU
images, unsealed, each with its SHA-256 in `SHA256SUMS`: the exact programs the miner loads. Some of them are the compiler's
output from these sources with a tuning step applied afterwards (instruction order, register names, scheduling bits), so a
build from source gives the same results but not the same bytes. Nor is the source alone enough to rebuild an image: the Quantus kernel
is compiled with per-architecture options (the `QTC_*` switches in `mining.cu`) that are not in this tree, and some of its code only
serves variants the release does not use. The hashes on the CPU (below) are the same either way.

**The build does not need a CUDA Toolkit.** It links the engines as prebuilt static libraries, one per system
(`crates/*/kernels/sealed`), built from this source and carrying these images; the sealing there is only packaging.

**You can check what the kernel does.** `dori-miner bench` starts by hashing 4096 nonces on
the card and running a search on it, and compares both with the exact hash computed on the CPU by the code in
`crates/qtc-pow`; while mining, every share is checked on the CPU before it is sent, and `--self-check` measures the card against the CPU for as
long as you like. On Pearl every tile the card reports is checked twice on the CPU before it is sent: by the miner's
own exact model of the proof of work (`crates/prl-pow`), then with the official proof and verifier of
[pearl-research-labs/pearl](https://github.com/pearl-research-labs/pearl), against the pool's target. A share that does
not pass these open checks does not leave the miner, whatever the engine says.

## Dev fee

Optional, and 1% unless you say otherwise: just under one minute in every hundred is mined for the DoriByte wallet of
the coin you mine, on the pool you chose. It is never more than 1% of a run: the first minute comes after 99 minutes of mining for
you, a shorter run pays nothing, and your pool password is not sent on the fee's link. `--dev-fee 0` turns
it off. All of it is in `crates/dori-miner/src/devfee.rs`, wallet included; the log says at the start what the fee is, and
when a window starts and ends, and `/summary` counts its shares apart from yours.

## Use

```
dori-miner devices
dori-miner bench --duration 60
dori-miner -o stratum+ssl://qtc-eu.kryptex.network:8049 -u <wallet> -w <worker> --api-port 4068
```

| Option | What |
|---|---|
| `-o <url>` | Pool, `host:port`, `stratum+tcp://` or `stratum+ssl://` |
| `-u <wallet>` | Your wallet (Quantus addresses start with "qz") |
| `-w <worker>` | Worker name (sent as `WALLET.WORKER`; as `WALLET/WORKER` on Kryptex) |
| `-d <list>` | Cards to use, nvidia-smi indexes separated by commas (default: all) |
| `--api-port <port>` | Local API: `GET http://127.0.0.1:<port>/summary` |
| `--tls-fingerprint <sha256>` | Require this pool certificate |
| `--self-check <bits>` | Measure the card against the CPU while mining |
| `--dev-fee <percent>` | Dev fee, 0 to 5. **1 when not given; `--dev-fee 0` turns it off** |

`dori-miner --help` lists the other options.

### Pearl

```
dori-miner --coin pearl -o stratum+ssl://prl.kryptex.network:8048 -u <wallet> -w <worker> --api-port 4068
```

- RTX 50, 40 and 30 cards (compute capability 12.0, 8.9 and 8.6). Other cards are refused with a message.
- Pearl addresses start with "prl1". The hash rate is in products a second (TH/s), as Pearl miners give it.
- Two machines on the same wallet do not repeat each other's work.
- Pools: mined on Kryptex (`stratum+ssl://prl.kryptex.network:8048` and its regions, `-u WALLET -w WORKER`), shares
  accepted. Other Pearl pools are not tried yet; tell us how it goes.

### Ergo

```
dori-miner --coin ergo -o stratum+ssl://erg-eu.kryptex.network:8021 -u <wallet> -w <worker> --api-port 4068
```

- Ergo addresses start with "9". Each card is checked against the CPU before it mines and at every new height.
- When the table grows past a card's memory, the card keeps mining, more slowly, and the miner tells you. Today it still fits on 8 GB cards, so this has not yet run for real: tell us how it goes.
- `--lithos` connects to a local Lithos client (no dev fee on a local pool).

### Vela

```
dori-miner --coin vela -o stratum+ssl://vela.suprnova.cc:7604 -u <wallet>.<worker> --api-port 4068
```

- RTX 50, 40 and 30 cards (12.0, 8.9 and 8.6), NVIDIA driver 580 or newer (CUDA 13).
- Proof-of-Inference: each attempt is one step of the sd-turbo integer UNet, exact to the official `sdint.py`, and the
  hash of its output is the share. The rate is in inferences a second (inf/s).
- The model (`sd-turbo-int.poisd`, about 1.2 GB) is downloaded on the first start from the pool's server (then its
  mirror), checked by its SHA-256 and kept in the data folder (`$DORI_DATA_DIR`, else `~/.local/share/dori-miner` or
  `%LOCALAPPDATA%\DoriMiner`); `--vela-model <file>` uses another copy. `GET /summary` shows the download as `model`.
  The model is Stability AI's sd-turbo, under the Stability AI Community License: it is not in this tree.
- Each card is checked against the official test vector before it mines; every attempt is hashed on the CPU.
- Pools: Suprnova (`vela.suprnova.cc`, `stratum-eu2`, `stratum-ord`, `stratum-hkg.suprnova.cc`, TLS port 7604,
  `-u WALLET.WORKER`), shares accepted on `vela.suprnova.cc`.

### Pools

| Pool | Example | What we have seen |
|---|---|---|
| Kryptex | `stratum+ssl://qtc-eu.kryptex.network:8049`, `-u WALLET -w WORKER` (sent as `WALLET/WORKER`) | mined on, shares accepted |
| Suprnova | `stratum+ssl://quantus.suprnova.cc:7074`, `-u WALLET.WORKER` | mined on, shares accepted |
| LuckyPool | `stratum+ssl://eu.lproute.com:5660`, `-u WALLET.WORKER` | logs in and gets jobs; not mined on yet |
| Himpool | `stratum+ssl://eu.himpool.com:1336`, `-u WALLET.WORKER` | logs in and gets jobs; not mined on yet |
| K1Pool | `stratum+ssl://eu.quantus.k1pool.com:5660`, `-u ACCOUNT.WORKER` | logs in and gets jobs; not mined on yet |

Pools that speak QUIC instead of stratum (LongCipher, NurseryPool) do not work. If you mine on a pool marked
"not mined on yet", tell us how it went.

**Is it really the pool?** Pools use certificates no authority vouches for, so a miner cannot check them the way
a browser does. This one knows the certificate of each pool above (seen on 2026-10-11) and says so in the log;
if the pool answers with another one it warns in capitals and goes on mining: either the pool renewed it or
something between you and the pool is answering in its place. `--tls-fingerprint` makes it refuse instead.
On any other pool it prints the certificate's fingerprint for you to pin.

The miner does not touch your card's settings: no overclock, no power limit, no fan control. For that we
recommend [DoriRig](https://gitlab.com/doribyte-releases/downloads) (beta), DoriByte's own rig manager, which runs DoriMiner and takes care
of fans, limits and temperatures; any tool you already use works too. Either way, watch the temperatures (memory too) on the
first run.

## DoriMiner Control

Optional: a small window for one miner on one computer, in `control/`. It writes the miner's command line, starts and
stops it, and shows what it is doing. On Windows a profile can also set a card's power limit, clocks and fans, as
administrator. It comes as its own package on the downloads page, with this same miner in it.

## Build

Rust and its system linker are all you need (on Linux, libstdc++ for the Ergo and Vela engines): the engines are in the tree as
static libraries, so there is no CUDA Toolkit to install. At run time the miner needs only the NVIDIA driver.

```
cargo build --release         # gives target/release/dori-miner(.exe): the release package calls it dori-miner
cargo test --workspace        # CPU only; never touches a GPU
```

## Tell us what your card does

We have measured only a few cards. Yours is worth a lot to us:

- the card (model and maker) and the driver version
- the last lines of `dori-miner bench --duration 60`
- your settings (stock, or core clock / offset / power limit) and the watts you read
- if you can, the hash rate of the miner you use today at the same settings

Post it in the #hashrates channel of our Discord: https://discord.gg/pEpuWh7D89

## Layout

| Path | What |
|---|---|
| `crates/qtc-pow` | Exact QPoW hash on the CPU |
| `crates/qtc-cuda` | Quantus on each card: the kernel and the engine in source (`kernels`, `engine`), the final images (`kernels/images`), the engine as a prebuilt library (`kernels/sealed`), candidates checked on the CPU |
| `crates/prl-pow` | Pearl on the CPU: the exact model of the proof of work, the job and pair sides, shares checked with the official verifier |
| `crates/prl-cuda` | Pearl on each card: the kernels and the engine in source (`kernels`, `engine`), the final images (`kernels/images`), the engine as a prebuilt library (`kernels/sealed`) |
| `crates/erg-cuda` | Ergo on each card: the engine in source (`engine`), as a prebuilt library (`kernels/sealed`, with NVIDIA's static CUDA runtime), and the protocol's hashes |
| `crates/vela-cuda` | Vela on each card: the engine in source (`engine`), as a prebuilt library (`kernels/sealed`, with NVIDIA's static CUDA runtime), and the protocol's hashes |
| `crates/dori-miner` | The program: pool clients, GPU workers, dev fee, local API, benchmark |
| `third_party/pearl` | Pearl's official proof and verifier (`zk-pow`, `pearl-blake3`, `plonky2`), unchanged, with their licences |

## Credits and licence

Parts of the host code, and the Quantus kernel it started from, derive from
[Quantus-Network/quantus-miner](https://github.com/Quantus-Network/quantus-miner) (Apache-2.0); see
[NOTICE](NOTICE). Pearl's official proof and verifier are in `third_party/pearl`, unchanged, under their own
licences (ISC; plonky2 MIT or Apache-2.0). `crates/vela-cuda/kernels/sealed` and `crates/erg-cuda/kernels/sealed` carry NVIDIA's static CUDA runtime
(`libcudart_static.a`), redistributed under the NVIDIA CUDA Toolkit licence.

In short, and the files are what counts:

- **Free to use, to read, to build and to change for yourself.** All of it, the kernels and engines included, is under the
  [PolyForm Noncommercial License 1.0.0](LICENSE), and [MINING-PERMISSION](MINING-PERMISSION) adds that mining for your
  own account, as a person, is allowed. Mining by or for a company needs a written agreement with us.
- **Not for sale.** You may not sell it, put it in a paid product or service, or use its code in another
  miner that charges a fee or keeps its source closed.
- **Source-available, not "open source" in the OSI sense:** the noncommercial terms are the difference. Releases before 0.1.10
  keep the terms they were published with.

Want something these terms do not allow? Talk to us.
