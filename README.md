# DoriMiner

An NVIDIA miner for Quantus (QTC, the QPoW algorithm: Poseidon2 over Goldilocks), by DoriByte.

**Status: test release for the RTX 50 series.** Measured on one card so far, an RTX 5070 Ti. It runs on older
NVIDIA cards too, but the fast kernel is only in for the 50 series.

## Where we stand

**Transparency and trust.** The miner is free, and its source is here for anyone to read and build: the pool
client, what is sent to the pool and when, the dev fee and the wallet it goes to, the local API, the exact
hash on the CPU, and the code that drives the card. Please audit it. If you find something wrong, tell us.

**The kernel is closed, for now.** The program that runs on the GPU is in this repository as compiled, sealed
images, not as source. It is where the speed comes from, and we would rather not hand it to closed, paid
miners. This is a position for the present, not a principle.

**We are not hiding it from you.** The sealing only keeps the kernel from falling out of the executable with
a one-line command. It is not strong protection and we do not pretend it is: the code that unseals the images
is in this repository (`crates/qtc-cuda/src/seal.rs`), because the miner has to do it to run them. What asks
you not to open them outside the running miner is the kernel's licence (`KERNEL-LICENSE`), not the seal. If you want to understand the kernel, you do not need to take it apart:
talk to us ([Discord](https://discord.gg/QBzHYGadE)). We want to contribute to a better mining community for everyone, and we answer.

**You can check what the kernel does without its source.** `dori-miner bench` starts by hashing 4096 nonces on
the card and running a search on it, and compares both with the exact hash computed on the CPU by the code in
`crates/qtc-pow`; while mining, every share is checked on the CPU before it is sent, and `--self-check` measures the card against the CPU for as
long as you like. A kernel can only produce hashes: it has no network, no files, no wallet.

## Dev fee

Optional, and 1% unless you say otherwise: just under one minute in every hundred is mined for the DoriByte wallet, on
the pool you chose. It is never more than 1% of a run: the first minute comes after 99 minutes of mining for
you, a shorter run pays nothing, and your pool password is not sent on the fee's link. `--dev-fee 0` turns
it off. All of it is in `crates/doriworker/src/devfee.rs`, wallet included; the log says at the start what the fee is, and
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
recommend [DoriRig](https://github.com/DoriByte/dori-rig-releases/releases) (beta), DoriByte's own rig manager, which runs DoriMiner and takes care
of fans, limits and temperatures; any tool you already use works too. Either way, watch the temperatures (memory too) on the
first run.

## Build

Rust is all you need: the kernel images are in the tree, so there is no CUDA Toolkit to install. At run time
the miner needs only the NVIDIA driver.

```
cargo build --release         # gives target/release/doriworker(.exe): the release package calls it dori-miner
cargo test --workspace        # CPU only; never touches a GPU
```

## Tell us what your card does

We have measured one card. Yours is worth a lot to us:

- the card (model and maker) and the driver version
- the last lines of `dori-miner bench --duration 60`
- your settings (stock, or core clock / offset / power limit) and the watts you read
- if you can, the hash rate of the miner you use today at the same settings

Post it in the #hashrates channel of our Discord: https://discord.gg/QBzHYGadE

## Layout

| Path | What |
|---|---|
| `crates/qtc-pow` | Exact QPoW hash on the CPU, and the sponge states the kernel resumes from |
| `crates/qtc-cuda` | Loading the kernel and the search engine on each card; `kernels/sealed` holds the images |
| `crates/doriworker` | The program: pool client, GPU workers, dev fee, local API, benchmark |

## Credits and licence

Parts of the host code, and the kernel the closed one started from, derive from
[Quantus-Network/quantus-miner](https://github.com/Quantus-Network/quantus-miner) (Apache-2.0); see
[NOTICE](NOTICE). 

In short, and the files are what counts:

- **Free to use, to read, to build and to change for yourself.** The source is under the
  [PolyForm Noncommercial License 1.0.0](LICENSE), and [MINING-PERMISSION](MINING-PERMISSION) adds that mining
  for your own account, at any scale and for profit, is allowed.
- **Not for sale.** You may not sell it, put it in a paid product or service, or use its code in another
  miner that charges a fee or keeps its source closed.
- **The kernel images** are free to use with DoriMiner and may not be taken apart: [KERNEL-LICENSE](KERNEL-LICENSE).

Want something these terms do not allow? Talk to us.
