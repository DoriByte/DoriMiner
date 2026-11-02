// DoriMiner - erg_api.cu
// Copyright (C) 2026 J. Costa (DoriByte). Licensed under the PolyForm Noncommercial License 1.0.0 with the additional
// permission in MINING-PERMISSION (see LICENSE and MINING-PERMISSION at the top of this tree).

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <cstdlib>
#include <cuda_runtime.h>
#include "erg_seal.h"

#define getenv(name) ((const char *)nullptr)

static long long envnum(const char *s) {
  long long sign = 1, v = 0;
  if (*s == '-') { sign = -1; s++; }
  while (*s >= '0' && *s <= '9') v = v * 10 + (*s++ - '0');
  return sign * v;
}

static bool envpair(const char *s, int *b, int *t) {
  if (*s < '0' || *s > '9') return false;
  *b = (int)envnum(s);
  while (*s >= '0' && *s <= '9') s++;
  if (*s != ',' || s[1] < '0' || s[1] > '9') return false;
  *t = (int)envnum(s + 1);
  return true;
}

namespace {

__constant__ uint32_t ERG_MSG[1024];
__constant__ uint64_t IV[8] = {
  0x6a09e667f3bcc908ull, 0xbb67ae8584caa73bull, 0x3c6ef372fe94f82bull, 0xa54ff53a5f1d36f1ull,
  0x510e527fade682d1ull, 0x9b05688c2b3e6c1full, 0x1f83d9abfb41bd6bull, 0x5be0cd19137e2179ull};
#define H0_256 (0x6a09e667f3bcc908ull ^ 0x01010020ull)

__device__ __forceinline__ uint64_t mk(uint32_t lo, uint32_t hi) { return ((uint64_t)hi << 32) | lo; }
__device__ __forceinline__ uint32_t LO(uint64_t x) { return (uint32_t)x; }
__device__ __forceinline__ uint32_t HI(uint64_t x) { return (uint32_t)(x >> 32); }
__device__ __forceinline__ uint64_t ror32(uint64_t x) { return mk(HI(x), LO(x)); }
__device__ __forceinline__ uint64_t ror24(uint64_t x) {
  return mk(__byte_perm(LO(x), HI(x), 0x6543), __byte_perm(LO(x), HI(x), 0x2107)); }
__device__ __forceinline__ uint64_t ror16(uint64_t x) {
  return mk(__byte_perm(LO(x), HI(x), 0x5432), __byte_perm(LO(x), HI(x), 0x1076)); }

template <bool FAST = false>
__device__ __forceinline__ uint64_t ror63(uint64_t x) {
#if __CUDA_ARCH__ >= 1000
  if (FAST) {
  uint32_t t, u, v;
  asm("add.cc.u32 %0, %3, %3;\n\taddc.cc.u32 %1, %4, %4;\n\taddc.u32 %2, %0, 0;" : "=&r"(t), "=&r"(u), "=r"(v) : "r"(LO(x)), "r"(HI(x)));
  return mk(v, u);
  }
#endif
  return mk(__funnelshift_l(HI(x), LO(x), 1), __funnelshift_l(LO(x), HI(x), 1)); }
__device__ __forceinline__ uint64_t bswap64(uint64_t x) {
  return mk(__byte_perm(HI(x), 0, 0x0123), __byte_perm(LO(x), 0, 0x0123)); }

template <bool SPLIT>
__device__ __forceinline__ uint64_t add2(uint64_t x, uint64_t y) {
#if __CUDA_ARCH__ >= 1000
  if (SPLIT) {
    uint32_t lo, hi;
    asm("add.cc.u32 %0, %2, %3;\n\taddc.u32 %1, %4, %5;" : "=r"(lo), "=r"(hi) : "r"(LO(x)), "r"(LO(y)), "r"(HI(x)), "r"(HI(y)));
    return mk(lo, hi);
  }
#endif
  return x + y;
}

#define G(a, b, c, d, x, y)                                       \
  a = add2<SPLIT>(add2<SPLIT>(a, b), (x)); d = ror32(d ^ a);      \
  c = add2<SPLIT>(c, d);  b = ror24(b ^ c);                       \
  a = add2<SPLIT>(add2<SPLIT>(a, b), (y)); d = ror16(d ^ a);      \
  c = add2<SPLIT>(c, d);  b = ror63<SPLIT>(b ^ c);

#define RONDA(s0, s1, s2, s3, s4, s5, s6, s7, s8, s9, s10, s11, s12, s13, s14, s15) \
  G(v[0], v[4], v[8], v[12], m[s0], m[s1]);   G(v[1], v[5], v[9], v[13], m[s2], m[s3]);  \
  G(v[2], v[6], v[10], v[14], m[s4], m[s5]);  G(v[3], v[7], v[11], v[15], m[s6], m[s7]); \
  G(v[0], v[5], v[10], v[15], m[s8], m[s9]);  G(v[1], v[6], v[11], v[12], m[s10], m[s11]); \
  G(v[2], v[7], v[8], v[13], m[s12], m[s13]); G(v[3], v[4], v[9], v[14], m[s14], m[s15]);
#define RONDAS \
  RONDA(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15); \
  RONDA(14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3); \
  RONDA(11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4); \
  RONDA(7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8); \
  RONDA(9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13); \
  RONDA(2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9); \
  RONDA(12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11); \
  RONDA(13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10); \
  RONDA(6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5); \
  RONDA(10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0); \
  RONDA(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15); \
  RONDA(14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3);

template <bool SPLIT = false>
__device__ __forceinline__ void comprime(uint64_t h[8], const uint64_t m[16], uint32_t t, bool last) {
  uint64_t v[16];
#pragma unroll
  for (int k = 0; k < 8; k++) { v[k] = h[k]; v[k + 8] = IV[k]; }
  v[12] ^= t;
  if (last) v[14] = ~v[14];
  RONDAS
#pragma unroll
  for (int k = 0; k < 8; k++) h[k] ^= v[k] ^ v[k + 8];
}

__device__ __forceinline__ void h_inicial(uint64_t h[8]) {
  h[0] = H0_256;
#pragma unroll
  for (int k = 1; k < 8; k++) h[k] = IV[k];
}

template <bool SPLIT>
__device__ __forceinline__ void elemento(uint32_t j, uint32_t hbe, uint64_t h[8]) {
    uint64_t m[16];
    h_inicial(h);
    m[0] = mk(__byte_perm(j, 0, 0x0123), hbe);
#pragma unroll
    for (int w = 1; w < 16; w++) m[w] = (uint64_t)__byte_perm(w - 1, 0, 0x0123) << 32;
    comprime<SPLIT>(h, m, 128, false);
#pragma unroll 1
    for (uint32_t k = 1; k < 64; k++) {
#pragma unroll
      for (int w = 0; w < 16; w++) {
#if __CUDA_ARCH__ < 1000
        m[w] = (uint64_t)ERG_MSG[16 * k + w - 1] << 32;
#else
        m[w] = (uint64_t)__byte_perm(16 * k + w - 1, 0, 0x0123) << 32;
#endif
      }
      comprime<SPLIT>(h, m, 128 * (k + 1), false);
    }
    m[0] = (uint64_t)__byte_perm(1023, 0, 0x0123) << 32;
#pragma unroll
    for (int w = 1; w < 16; w++) m[w] = 0;
    comprime<SPLIT>(h, m, 8200, true);
}

#define PACK_WARP 992
template <bool SPLIT, bool P = false>
__global__ void __launch_bounds__(256) gera(uint4 *__restrict__ tab, uint32_t n, uint32_t hbe, uint32_t k = 0) {
  if constexpr (!P) {
#pragma unroll 1
    for (uint32_t j = blockIdx.x * blockDim.x + threadIdx.x; j < n; j += gridDim.x * blockDim.x) {
      uint64_t h[8];
      elemento<SPLIT>(j, hbe, h);
      tab[2 * (uint64_t)j] = make_uint4(LO(h[0]), HI(h[0]), LO(h[1]), HI(h[1]));
      tab[2 * (uint64_t)j + 1] = make_uint4(LO(h[2]), HI(h[2]), LO(h[3]), HI(h[3]));
    }
  } else {

    __shared__ __align__(16) uint8_t st[8 * PACK_WARP];
    const uint32_t lane = threadIdx.x & 31;
    uint8_t *s = st + PACK_WARP * (threadIdx.x >> 5);
#pragma unroll 1
    for (uint32_t b = blockIdx.x * blockDim.x + (threadIdx.x & ~31u); b < n; b += gridDim.x * blockDim.x) {
      uint64_t h[8];
      elemento<SPLIT>(b + lane, hbe, h);
      if (b < k) {
        const uint64_t j = b + lane;
        tab[2 * j] = make_uint4(LO(h[0]), HI(h[0]), LO(h[1]), HI(h[1]));
        tab[2 * j + 1] = make_uint4(LO(h[2]), HI(h[2]), LO(h[3]), HI(h[3]));
        continue;
      }
#pragma unroll
      for (int c = 1; c < 32; c++) s[31 * lane + c - 1] = (uint8_t)(h[c / 8] >> (8 * (c % 8)));
      __syncwarp();
      const uint32_t bytes = 31 * (n - b < 32 ? n - b : 32);
      uint4 *dst = (uint4 *)((uint8_t *)tab + 32ull * k + 16 + 31ull * (b - k));
      for (uint32_t q = lane; 16 * q < bytes; q += 32) dst[q] = ((const uint4 *)s)[q];
      __syncwarp();
    }
  }
}

struct Job {
  uint64_t msg[4];
  uint64_t s4, s5, s6;
  uint64_t alvo_topo;
  uint64_t mN;
  uint32_t n;
  uint32_t k;
};
__constant__ Job T;

__device__ __forceinline__ uint32_t fastmod(uint32_t x) { return (uint32_t)__umul64hi(T.mN * x, T.n); }

template <int L>
__device__ __forceinline__ uint4 ld16(const uint4 *p) {
  if constexpr (L == 1) return __ldcs(p);
  if constexpr (L == 2) {
    uint4 v;
    asm volatile("ld.global.nc.L1::no_allocate.v4.u32 {%0,%1,%2,%3}, [%4];" : "=r"(v.x), "=r"(v.y), "=r"(v.z), "=r"(v.w) : "l"(p));
    return v;
  }
  return __ldg(p);
}

__device__ __forceinline__ uint64_t janela(uint32_t i) {
  return i < T.k ? 32ull * i : 32ull * T.k + 15 + 31ull * (i - T.k);
}

__device__ __forceinline__ void meia(const uint4 a, const uint4 b, uint32_t s, uint32_t x[4]) {
  const uint32_t w[8] = {a.x, a.y, a.z, a.w, b.x, b.y, b.z, b.w};
  const uint32_t q = s >> 2, r = 8 * (s & 3);
  uint32_t o[5];
#pragma unroll
  for (int k = 0; k < 5; k++) o[k] = q == 0 ? w[k] : q == 1 ? w[k + 1] : q == 2 ? w[k + 2] : w[k + 3];
#pragma unroll
  for (int k = 0; k < 4; k++) x[k] = __funnelshift_r(o[k], o[k + 1], r);
}

__device__ __forceinline__ void carrega_par(const uint4 *__restrict__ tab, uint32_t i, uint64_t e[4]) {
  const uint32_t r = threadIdx.x & 1;
  uint32_t out[8];
#pragma unroll
  for (uint32_t o = 0; o < 2; o++) {
    const uint32_t io = __shfl_sync(0xffffffffu, i, ((threadIdx.x & 31) & ~1u) | o);
    const uint64_t at = janela(io);
    const uint4 *c = tab + (at >> 4) + r;
    const uint32_t sh = (uint32_t)(at & 15);
    const uint4 a = ld16<0>(c), b = (r == 0 || sh) ? ld16<0>(c + 1) : make_uint4(0, 0, 0, 0);
    uint32_t x[4];
    meia(a, b, sh, x);
#pragma unroll
    for (int k = 0; k < 4; k++) {
      const uint32_t p = __shfl_xor_sync(0xffffffffu, x[k], 1);
      if (r == o) { out[4 * r + k] = x[k]; out[4 * (1 - r) + k] = p; }
    }
  }
  e[0] = mk(out[0], out[1]); e[1] = mk(out[2], out[3]); e[2] = mk(out[4], out[5]); e[3] = mk(out[6], out[7]);
}

template <bool P = false, int L = 0>
__device__ __forceinline__ void carrega(const uint4 *__restrict__ tab, uint32_t i, uint64_t e[4]) {
  if constexpr (P && L == 3) { carrega_par(tab, i, e); return; }
  if constexpr (P) {
    const uint64_t at = janela(i);
    const uint4 *c = tab + (at >> 4);
    const uint32_t sh = (uint32_t)(at & 15);

    constexpr bool SEM_SALTO = L >= 4;
    constexpr int LD = L == 4 || L == 6 ? 1 : L == 5 ? 0 : L;
    const uint32_t q = sh >> 2, r = 8 * (sh & 3);
    uint32_t o[9];
    if constexpr (SEM_SALTO) {
      const uint4 a = ld16<LD>(c), b = ld16<LD>(c + 1);
      uint4 d;
      if constexpr (L == 6) {

        d = sh ? ld16<LD>(c + 2) : make_uint4(0, 0, 0, 0);
      } else {
        d = ld16<LD>(c + 1 + (sh != 0));
      }
      const uint32_t w[12] = {a.x, a.y, a.z, a.w, b.x, b.y, b.z, b.w, d.x, d.y, d.z, d.w};
      const uint32_t m2 = 0u - ((q >> 1) & 1), m1 = 0u - (q & 1);
      uint32_t u[10];
#pragma unroll
      for (int k = 0; k < 10; k++) u[k] = (w[k] & ~m2) | (w[k + 2] & m2);
#pragma unroll
      for (int k = 0; k < 9; k++) o[k] = (u[k] & ~m1) | (u[k + 1] & m1);
    } else {
      const uint4 a = ld16<LD>(c), b = ld16<LD>(c + 1), d = sh ? ld16<LD>(c + 2) : make_uint4(0, 0, 0, 0);
      const uint32_t w[12] = {a.x, a.y, a.z, a.w, b.x, b.y, b.z, b.w, d.x, d.y, d.z, d.w};
#pragma unroll
      for (int k = 0; k < 9; k++) o[k] = q == 0 ? w[k] : q == 1 ? w[k + 1] : q == 2 ? w[k + 2] : w[k + 3];
    }
    uint32_t x[8];
#pragma unroll
    for (int k = 0; k < 8; k++) x[k] = __funnelshift_r(o[k], o[k + 1], r);
    e[0] = mk(x[0], x[1]); e[1] = mk(x[2], x[3]); e[2] = mk(x[4], x[5]); e[3] = mk(x[6], x[7]);
    return;
  }
#if __CUDA_ARCH__ >= 1000
  uint32_t w[8];
  asm volatile("ld.global.nc.L1::no_allocate.v8.b32 {%0,%1,%2,%3,%4,%5,%6,%7}, [%8];"
               : "=r"(w[0]), "=r"(w[1]), "=r"(w[2]), "=r"(w[3]), "=r"(w[4]), "=r"(w[5]), "=r"(w[6]), "=r"(w[7])
               : "l"(tab + 2 * (uint64_t)i));
  e[0] = mk(w[0], w[1]); e[1] = mk(w[2], w[3]); e[2] = mk(w[4], w[5]); e[3] = mk(w[6], w[7]);
#elif __CUDA_ARCH__ < 800
  const uint4 a = __ldcs(tab + 2 * (uint64_t)i), b = __ldcs(tab + 2 * (uint64_t)i + 1);
  e[0] = mk(a.x, a.y); e[1] = mk(a.z, a.w); e[2] = mk(b.x, b.y); e[3] = mk(b.z, b.w);
#else
  const uint4 a = __ldg(tab + 2 * (uint64_t)i), b = __ldg(tab + 2 * (uint64_t)i + 1);
  e[0] = mk(a.x, a.y); e[1] = mk(a.z, a.w); e[2] = mk(b.x, b.y); e[3] = mk(b.z, b.w);
#endif
}

__device__ __forceinline__ void soma(uint32_t f[8], const uint64_t e[4]) {
  const uint32_t w[8] = {LO(e[0]), HI(e[0]), LO(e[1]), HI(e[1]), LO(e[2]), HI(e[2]), LO(e[3]), HI(e[3])};
  uint32_t l[8];
#pragma unroll
  for (int k = 0; k < 8; k++) l[k] = __byte_perm(w[7 - k], 0, 0x0123);
  l[7] &= 0x00ffffffu;
  asm("add.cc.u32 %0, %0, %1;" : "+r"(f[0]) : "r"(l[0]));
#pragma unroll
  for (int k = 1; k < 7; k++) asm("addc.cc.u32 %0, %0, %1;" : "+r"(f[k]) : "r"(l[k]));
  asm("addc.u32 %0, %0, %1;" : "+r"(f[7]) : "r"(l[7]));
}

#define MAX_FOUND 32

template <bool P = false, int L = 0, int G = 8, int MB = 1>
__global__ void __launch_bounds__(256, MB) mina(const uint4 *__restrict__ tab, uint64_t nonce0, uint32_t voltas,
                                            uint32_t *__restrict__ res, uint64_t *__restrict__ dbg,
                                            const uint64_t *__restrict__ lista) {
  const uint32_t g = blockIdx.x * blockDim.x + threadIdx.x;
#pragma unroll 1
  for (uint32_t volta = 0; volta < voltas; volta++) {
    const uint64_t nonce = lista ? lista[g] : nonce0 + g + (uint64_t)volta * gridDim.x * blockDim.x;
    const uint64_t nw = bswap64(nonce);
    uint64_t h[8], m[16];

    h_inicial(h);
#pragma unroll
    for (int k = 0; k < 4; k++) m[k] = T.msg[k];
    m[4] = nw;
#pragma unroll
    for (int k = 5; k < 16; k++) m[k] = 0;
    comprime(h, m, 40, true);
    const uint32_t i = (uint32_t)(bswap64(h[3]) % T.n);

    uint64_t e[4];
    carrega<P, L>(tab, i, e);
    m[0] = (e[0] >> 8) | (e[1] << 56);
    m[1] = (e[1] >> 8) | (e[2] << 56);
    m[2] = (e[2] >> 8) | (e[3] << 56);
    m[3] = (e[3] >> 8) | (T.msg[0] << 56);
    m[4] = T.s4; m[5] = T.s5; m[6] = T.s6;
    m[7] = (T.msg[3] >> 8) | (nw << 56);
    m[8] = nw >> 8;
#pragma unroll
    for (int k = 9; k < 16; k++) m[k] = 0;
    h_inicial(h);
    comprime(h, m, 71, true);

    uint32_t be[9];
#pragma unroll
    for (int q = 0; q < 4; q++) { be[2 * q] = __byte_perm(LO(h[q]), 0, 0x0123); be[2 * q + 1] = __byte_perm(HI(h[q]), 0, 0x0123); }
    be[8] = be[0];
    uint32_t idx[32];
#pragma unroll
    for (int k = 0; k < 32; k++) idx[k] = fastmod(__funnelshift_l(be[k / 4 + 1], be[k / 4], 8 * (k % 4)));

#if __CUDA_ARCH__ < 1000
    constexpr int GRUPO = P ? G : 8;
#else
    constexpr int GRUPO = P ? G : 32;
#endif
    uint32_t f[8] = {0, 0, 0, 0, 0, 0, 0, 0};
#pragma unroll
    for (int k0 = 0; k0 < 32; k0 += GRUPO) {
      const uint32_t z = k0 ? (f[7] >> 30) : 0;
      uint64_t r[GRUPO][4];
#pragma unroll
      for (int k = 0; k < GRUPO; k++) carrega<P, L>(tab, idx[k0 + k] + z, r[k]);
#pragma unroll
      for (int k = 0; k < GRUPO; k++) soma(f, r[k]);
    }

    h_inicial(h);
    m[0] = mk(__byte_perm(f[7], 0, 0x0123), __byte_perm(f[6], 0, 0x0123));
    m[1] = mk(__byte_perm(f[5], 0, 0x0123), __byte_perm(f[4], 0, 0x0123));
    m[2] = mk(__byte_perm(f[3], 0, 0x0123), __byte_perm(f[2], 0, 0x0123));
    m[3] = mk(__byte_perm(f[1], 0, 0x0123), __byte_perm(f[0], 0, 0x0123));
#pragma unroll
    for (int k = 4; k < 16; k++) m[k] = 0;
    comprime(h, m, 32, true);

    if (dbg) {
#pragma unroll
      for (int k = 0; k < 4; k++) dbg[4 * g + k] = h[k];
    } else if (bswap64(h[0]) <= T.alvo_topo) {
      const uint32_t p = atomicAdd(res, 1);
      if (p < MAX_FOUND) { res[1 + 2 * p] = LO(nonce); res[2 + 2 * p] = HI(nonce); }
    }
  }
}

uint32_t calc_n(uint32_t height) {
  uint32_t n = 1u << 26;
  if (height < 600 * 1024) return n;
  const uint32_t a = height < 4198400 ? height : 4198400;
  for (uint32_t k = 0; k < (a - 600 * 1024) / (50 * 1024) + 1; k++) n = n / 100 * 105;
  return n;
}
uint64_t le64(const uint8_t *b) { uint64_t x = 0; for (int k = 7; k >= 0; k--) x = (x << 8) | b[k]; return x; }
uint32_t bs32(uint32_t x) { return (x >> 24) | ((x >> 8) & 0xff00u) | ((x << 8) & 0xff0000u) | (x << 24); }

struct Table {
  uint4 *p = nullptr;
  uint64_t bytes = 0;
  uint32_t height = 0;
  bool pending = false;
  cudaEvent_t done = nullptr;
  bool packed = false;
  uint32_t k = 0;
};

uint64_t table_bytes(uint32_t n, bool packed, uint32_t k = 0) {
  return packed ? 32ull * k + ((16 + 31ull * (n - k) + 15) & ~15ull) + 64 : 32ull * n;
}

static const uint64_t NONCES_PER_LAUNCH = 44040192ull;

uint32_t k_for(uint32_t n, uint64_t bytes) {
  const uint64_t base = table_bytes(n, true, 0);
  if (bytes <= base) return 0;
  const uint64_t k = bytes - base;
  return (uint32_t)((k < n ? k : n) & ~31ull);
}

struct Handle {
  int device = 0, sms = 0, l2max = 0;
  int mblocks = 0, mthreads = 0;
  int genthreads = 128;

  cudaStream_t smine = nullptr, sgen = nullptr;
  Table t[2];
  int cur = 0;
  bool two = false;
  bool packed = false;
  int pack_mode = -1;
  int pack_load = 0;
  bool pack_load_env = false;

  int pack_load_few = 5;
  int pack_switch = 500;
  uint64_t pack_cap = 0;
  int pack_grupo = 8;
  int pack_minb = 1;
  int pblocks = 0, pthreads = 0;

  uint32_t *res = nullptr;
  uint32_t *hres = nullptr;
  uint64_t *dlist = nullptr, *ddbg = nullptr;
};

void say(char *err, int n, const char *what) {
  if (err && n > 0) std::snprintf(err, (size_t)n, "%s", what);
}

bool ensure_alloc(Table &t, uint64_t bytes) {
  if (t.bytes >= bytes) return true;
  if (t.p) cudaFree(t.p);
  t.p = nullptr; t.bytes = 0; t.height = 0;
  if (cudaMalloc(&t.p, bytes) != cudaSuccess) { cudaGetLastError(); return false; }
  t.bytes = bytes;
  return true;
}

bool alloc_packed(Handle *h, Table &a, uint32_t n) {
  const uint64_t lo = table_bytes(n, true, 0);
  uint64_t hi = h->pack_mode == 1 && !h->pack_cap ? lo : table_bytes(n, true, n);
  if (h->pack_cap) hi = hi < h->pack_cap ? hi : (h->pack_cap > lo ? h->pack_cap : lo);
  if (a.p && a.bytes >= lo) return true;
  for (uint64_t sz = hi;; sz = sz > lo + (64ull << 20) ? sz - (64ull << 20) : lo) {
    if (ensure_alloc(a, sz)) return true;
    if (sz == lo) return false;
  }
}

void start_gen(Handle *h, Table &t, uint32_t height, cudaStream_t s, bool full) {
  const uint32_t n = calc_n(height);
  int per = 1;
  if (full && cudaOccupancyMaxActiveBlocksPerMultiprocessor(&per, t.packed ? gera<true, true> : gera<true>, 256, 0) != cudaSuccess) per = 1;

  if (t.packed) {
    if (full) gera<true, true><<<h->sms * (per > 0 ? per : 1), 256, 0, s>>>(t.p, n, bs32(height), t.k);
    else gera<false, true><<<h->sms, h->genthreads, 0, s>>>(t.p, n, bs32(height), t.k);
  } else {
    if (full) gera<true><<<h->sms * (per > 0 ? per : 1), 256, 0, s>>>(t.p, n, bs32(height));
    else gera<false><<<h->sms, h->genthreads, 0, s>>>(t.p, n, bs32(height));
  }
  t.height = height;
  t.pending = true;
  cudaEventRecord(t.done, s);
}

void pin_l2(Handle *h, const Table &t) {
  if (h->l2max <= 0) return;
  cudaStreamAttrValue at = {};
  at.accessPolicyWindow.base_ptr = t.p;
  at.accessPolicyWindow.num_bytes = (size_t)h->l2max < t.bytes ? (size_t)h->l2max : t.bytes;
  at.accessPolicyWindow.hitRatio = 1.0f;
  at.accessPolicyWindow.hitProp = cudaAccessPropertyPersisting;
  at.accessPolicyWindow.missProp = cudaAccessPropertyStreaming;
  cudaStreamSetAttribute(h->smine, cudaStreamAttributeAccessPolicyWindow, &at);
  cudaCtxResetPersistingL2Cache();
}

template <int L>
void mina_packed_l(Handle *h, int blocks, int threads, const uint4 *tab, uint64_t nonce0, uint32_t voltas, uint64_t *dbg, const uint64_t *lista) {
  if constexpr (L == 1) {
    if (h->pack_minb == 3) { mina<true, 1, 8, 3><<<blocks, threads, 0, h->smine>>>(tab, nonce0, voltas, h->res, dbg, lista); return; }
    if (h->pack_minb == 4) { mina<true, 1, 8, 4><<<blocks, threads, 0, h->smine>>>(tab, nonce0, voltas, h->res, dbg, lista); return; }
  }
  if (h->pack_grupo == 16) mina<true, L, 16><<<blocks, threads, 0, h->smine>>>(tab, nonce0, voltas, h->res, dbg, lista);
  else if (h->pack_grupo == 32) mina<true, L, 32><<<blocks, threads, 0, h->smine>>>(tab, nonce0, voltas, h->res, dbg, lista);
  else mina<true, L, 8><<<blocks, threads, 0, h->smine>>>(tab, nonce0, voltas, h->res, dbg, lista);
}

int pack_load_for(const Handle *h, const Table &t) {
  if (h->pack_load_env) return h->pack_load;
  const uint32_t n = calc_n(t.height);
  const uint64_t share = (uint64_t)(n - (t.k < n ? t.k : n)) * 1000 / n;
  return share <= (uint64_t)h->pack_switch ? h->pack_load_few : h->pack_load;
}

void mina_packed(Handle *h, const Table &t, int blocks, int threads, const uint4 *tab, uint64_t nonce0, uint32_t voltas, uint64_t *dbg, const uint64_t *lista) {
  switch (pack_load_for(h, t)) {
    case 1: mina_packed_l<1>(h, blocks, threads, tab, nonce0, voltas, dbg, lista); break;
    case 2: mina_packed_l<2>(h, blocks, threads, tab, nonce0, voltas, dbg, lista); break;
    case 3: mina_packed_l<3>(h, blocks, threads, tab, nonce0, voltas, dbg, lista); break;
    case 4: mina_packed_l<4>(h, blocks, threads, tab, nonce0, voltas, dbg, lista); break;
    case 5: mina_packed_l<5>(h, blocks, threads, tab, nonce0, voltas, dbg, lista); break;
    case 6: mina_packed_l<6>(h, blocks, threads, tab, nonce0, voltas, dbg, lista); break;
    default: mina_packed_l<0>(h, blocks, threads, tab, nonce0, voltas, dbg, lista);
  }
}

}

extern "C" {

void *erg_open(int device, char *err, int errlen) {
  if (const char *why = erg_seal::unseal(); *why) { say(err, errlen, why); return nullptr; }
  int drv = 0;
  cudaDriverGetVersion(&drv);
  if (drv < 13000) {
    char m[160];
    std::snprintf(m, sizeof m, "the NVIDIA driver supports CUDA %d.%d; ERG needs driver 580 or newer (CUDA 13)", drv / 1000, (drv % 1000) / 10);
    say(err, errlen, m);
    return nullptr;
  }
  if (cudaSetDevice(device) != cudaSuccess) { say(err, errlen, "cannot select the card"); return nullptr; }
  cudaDeviceProp pr;
  if (cudaGetDeviceProperties(&pr, device) != cudaSuccess) { say(err, errlen, "cannot read the card"); return nullptr; }
  if (pr.major * 10 + pr.minor < 75) { say(err, errlen, "ERG needs an RTX 20, 30, 40 or 50 card (sm_75 or newer)"); return nullptr; }
  auto *h = new Handle;
  h->device = device;
  h->sms = pr.multiProcessorCount;
  h->mblocks = pr.major >= 10 ? 2 * h->sms : h->sms;
  h->mthreads = pr.major >= 10 ? 256 : 128;

  if (const char *e = getenv(nullptr)) {
    int b = 0, t = 0;
    if (envpair(e, &b, &t) && b > 0 && t >= 32 && t <= 1024) { h->mblocks = b * h->sms; h->mthreads = t; }
  }
  if (const char *e = getenv(nullptr)) { const int t = (int)envnum(e); if (t >= 32 && t <= 256 && t % 32 == 0) h->genthreads = t; }
  if (const char *e = getenv(nullptr)) h->pack_mode = (int)envnum(e) ? 1 : 0;
  h->packed = h->pack_mode == 1;
  if (const char *e = getenv(nullptr)) h->pack_cap = (uint64_t)envnum(e) << 20;
  if (const char *e = getenv(nullptr)) { const int g = (int)envnum(e); if (g == 8 || g == 16 || g == 32) h->pack_grupo = g; }

  h->pack_load = pr.major >= 10 ? 0 : 1;
  h->pblocks = pr.major >= 10 ? h->mblocks : 3 * h->sms;
  h->pthreads = pr.major >= 10 ? h->mthreads : 128;

  h->pack_load_few = pr.major >= 10 ? 5 : 1;
  if (const char *e = getenv(nullptr)) { const int l = (int)envnum(e); if (l >= 0 && l <= 6) { h->pack_load = l; h->pack_load_env = true; } }
  if (const char *e = getenv(nullptr)) { const int m = (int)envnum(e); if (m == 1 || m == 3 || m == 4) h->pack_minb = m; }
  if (const char *e = getenv(nullptr)) { const int v = (int)envnum(e); if (v >= 0 && v <= 1000) h->pack_switch = v; }
  if (const char *e = getenv(nullptr)) {
    int b = 0, t = 0;
    if (envpair(e, &b, &t) && b > 0 && t >= 32 && t <= 256 && t % 32 == 0) { h->pblocks = b * h->sms; h->pthreads = t; }
  }
  {
    uint32_t mm[1024];
    for (uint32_t i = 0; i < 1024; i++) mm[i] = bs32(i);
    if (cudaMemcpyToSymbol(ERG_MSG, mm, sizeof mm) != cudaSuccess) { say(err, errlen, "cannot set up the card (constants)"); return nullptr; }
  }
  h->l2max = pr.persistingL2CacheMaxSize;
  if (h->l2max > 0 && cudaDeviceSetLimit(cudaLimitPersistingL2CacheSize, (size_t)h->l2max) != cudaSuccess) {
    cudaGetLastError();
    h->l2max = 0;
  }
  int lo = 0, hi = 0;
  cudaDeviceGetStreamPriorityRange(&lo, &hi);
  bool ok = cudaStreamCreateWithPriority(&h->smine, cudaStreamNonBlocking, hi) == cudaSuccess &&
            cudaStreamCreateWithPriority(&h->sgen, cudaStreamNonBlocking, lo) == cudaSuccess &&
            cudaEventCreateWithFlags(&h->t[0].done, cudaEventDisableTiming) == cudaSuccess &&
            cudaEventCreateWithFlags(&h->t[1].done, cudaEventDisableTiming) == cudaSuccess &&
            cudaMalloc(&h->res, 4 * (1 + 2 * MAX_FOUND)) == cudaSuccess &&
            cudaMallocHost(&h->hres, 4 * (1 + 2 * MAX_FOUND)) == cudaSuccess &&
            cudaMalloc(&h->dlist, 8 * 256) == cudaSuccess && cudaMalloc(&h->ddbg, 32 * 256) == cudaSuccess;
  if (!ok) { say(err, errlen, "cannot set up the card (streams, buffers)"); delete h; return nullptr; }
  return h;
}

int erg_table(void *hp, uint32_t height, int *made) {
  auto *h = (Handle *)hp;
  cudaSetDevice(h->device);
  uint64_t bytes = table_bytes(calc_n(height), h->packed);
  *made = 0;
  Table &a = h->t[h->cur], &b = h->t[1 - h->cur];
  if (b.height == height && b.bytes >= bytes && b.packed == h->packed) {
    if (b.pending && cudaEventSynchronize(b.done) != cudaSuccess) return 2;
    b.pending = false;
    h->cur = 1 - h->cur;
  } else if (!(a.height == height && !a.pending && a.bytes >= bytes && a.packed == h->packed)) {
    if (b.pending) { cudaEventSynchronize(b.done); b.pending = false; b.height = 0; }
    const uint32_t n = calc_n(height);
    if (!(h->packed ? alloc_packed(h, a, n) : ensure_alloc(a, bytes))) {

      if (b.p) { cudaFree(b.p); b = Table{nullptr, 0, 0, false, b.done}; }
      h->two = false;
      if (!(h->packed ? alloc_packed(h, a, n) : ensure_alloc(a, bytes))) {

        if (h->packed || h->pack_mode == 0) return 1;
        h->packed = true;
        if (!alloc_packed(h, a, n)) return 1;
      }
    }
    a.packed = h->packed;
    a.k = a.packed ? k_for(n, a.bytes) : 0;
    start_gen(h, a, height, h->smine, true);
    if (cudaEventSynchronize(a.done) != cudaSuccess) return 2;
    a.pending = false;
    *made = 1;
  }
  Table &now = h->t[h->cur], &spare = h->t[1 - h->cur];
  pin_l2(h, now);

  const uint64_t next_bytes = table_bytes(calc_n(height + 1), h->packed);
  if (!(spare.height == height + 1 && spare.packed == h->packed)) {
    if (!spare.p) {
      size_t fr = 0, tot = 0;
      cudaMemGetInfo(&fr, &tot);
      h->two = fr > next_bytes + (256ull << 20);
    }
    if (h->two && (h->packed ? alloc_packed(h, spare, calc_n(height + 1)) : ensure_alloc(spare, next_bytes))) {
      spare.packed = h->packed;
      spare.k = spare.packed ? k_for(calc_n(height + 1), spare.bytes) : 0;
      start_gen(h, spare, height + 1, h->sgen, false);
    }
    else if (h->two) { h->two = false; cudaGetLastError(); }
  }
  return cudaGetLastError() == cudaSuccess ? 0 : 2;
}

int erg_info_c(void *hp, uint32_t height) {
  auto *h = (Handle *)hp;
  const Table &s = h->t[1 - h->cur];
  if (s.height != height + 1) return -1;
  return s.pending && cudaEventQuery(s.done) == cudaErrorNotReady ? 0 : 1;
}

int erg_info_a(void *hp) {
  auto *h = (Handle *)hp;
  const Table &t = h->t[h->cur];
  return t.packed && t.height && t.k < calc_n(t.height) ? 1 : 0;
}

int erg_info_b(void *hp) {
  auto *h = (Handle *)hp;
  const Table &t = h->t[h->cur];
  if (!t.packed || !t.height) return 0;
  const uint32_t n = calc_n(t.height);
  return (int)((uint64_t)(n - (t.k < n ? t.k : n)) * 1000 / n);
}

uint64_t erg_table_bytes(void *hp) {
  auto *h = (Handle *)hp;
  return h->t[h->cur].bytes;
}

int erg_mine(void *hp, const uint8_t *msg, uint64_t alvo_topo, uint64_t nonce0, uint32_t voltas, uint64_t *found, int max) {
  auto *h = (Handle *)hp;
  const Table &t = h->t[h->cur];
  if (!t.height || t.pending) return -1;
  Job j = {};
  const uint32_t n = calc_n(t.height);
  for (int k = 0; k < 4; k++) j.msg[k] = le64(msg + 8 * k);
  j.s4 = le64(msg + 1); j.s5 = le64(msg + 9); j.s6 = le64(msg + 17);
  j.alvo_topo = alvo_topo;
  j.mN = UINT64_MAX / n + 1;
  j.n = n;
  j.k = t.k;
  if (cudaMemcpyToSymbolAsync(T, &j, sizeof j, 0, cudaMemcpyHostToDevice, h->smine) != cudaSuccess) return -2;
  if (cudaMemsetAsync(h->res, 0, 4, h->smine) != cudaSuccess) return -2;
  (void)cudaGetLastError();
  if (t.packed) mina_packed(h, t, h->pblocks, h->pthreads, t.p, nonce0, voltas, nullptr, nullptr);
  else mina<<<h->mblocks, h->mthreads, 0, h->smine>>>(t.p, nonce0, voltas, h->res, nullptr, nullptr);
  if (cudaGetLastError() != cudaSuccess) return -2;
  if (cudaMemcpyAsync(h->hres, h->res, 4 * (1 + 2 * MAX_FOUND), cudaMemcpyDeviceToHost, h->smine) != cudaSuccess) return -2;
  if (cudaStreamSynchronize(h->smine) != cudaSuccess) return -3;
  int c = (int)h->hres[0];
  if (c > MAX_FOUND) c = MAX_FOUND;
  if (c > max) c = max;
  for (int k = 0; k < c; k++) found[k] = ((uint64_t)h->hres[2 + 2 * k] << 32) | h->hres[1 + 2 * k];
  return c;
}

uint64_t erg_per_launch(void *hp, uint32_t voltas) {
  auto *h = (Handle *)hp;
  return h->t[h->cur].packed ? (uint64_t)h->pblocks * h->pthreads * voltas : (uint64_t)h->mblocks * h->mthreads * voltas;
}

uint32_t erg_info_d(void *hp) {
  uint64_t per1 = erg_per_launch(hp, 1);
  if (per1 < 1) per1 = 1;
  uint64_t r = NONCES_PER_LAUNCH / per1;
  if (r < 1) r = 1;
  if (r > (1ull << 16)) r = 1ull << 16;
  return (uint32_t)r;
}

int erg_hits(void *hp, const uint8_t *msg, const uint64_t *nonces, int count, uint8_t *hits) {
  auto *h = (Handle *)hp;
  const Table &t = h->t[h->cur];
  if (!t.height || t.pending || count < 1 || count > 256) return 1;
  Job j = {};
  const uint32_t n = calc_n(t.height);
  for (int k = 0; k < 4; k++) j.msg[k] = le64(msg + 8 * k);
  j.s4 = le64(msg + 1); j.s5 = le64(msg + 9); j.s6 = le64(msg + 17);
  j.mN = UINT64_MAX / n + 1;
  j.n = n;
  j.k = t.k;
  if (cudaMemcpyToSymbolAsync(T, &j, sizeof j, 0, cudaMemcpyHostToDevice, h->smine) != cudaSuccess) return 2;

  const int padded = (count + 31) & ~31;
  uint64_t list[256];
  for (int k = 0; k < padded; k++) list[k] = nonces[k < count ? k : count - 1];
  if (cudaMemcpyAsync(h->dlist, list, 8ull * padded, cudaMemcpyHostToDevice, h->smine) != cudaSuccess) return 2;
  if (cudaStreamSynchronize(h->smine) != cudaSuccess) return 2;

  if (t.packed) mina_packed(h, t, 1, padded, t.p, 0, 1, h->ddbg, h->dlist);
  else mina<<<1, count, 0, h->smine>>>(t.p, 0, 1, h->res, h->ddbg, h->dlist);
  if (cudaMemcpyAsync(hits, h->ddbg, 32ull * count, cudaMemcpyDeviceToHost, h->smine) != cudaSuccess) return 2;
  return cudaStreamSynchronize(h->smine) == cudaSuccess ? 0 : 3;
}

void erg_close(void *hp) {
  auto *h = (Handle *)hp;
  if (!h) return;
  cudaSetDevice(h->device);
  cudaStreamSynchronize(h->sgen);
  cudaStreamSynchronize(h->smine);
  for (auto &t : h->t) { if (t.p) cudaFree(t.p); if (t.done) cudaEventDestroy(t.done); }
  cudaFree(h->res); cudaFreeHost(h->hres); cudaFree(h->dlist); cudaFree(h->ddbg);
  cudaStreamDestroy(h->smine); cudaStreamDestroy(h->sgen);
  delete h;
}

}

