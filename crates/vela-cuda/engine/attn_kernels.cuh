// DoriMiner - attn_kernels.cuh
// Copyright (C) 2026 J. Costa (DoriByte). Licensed under the PolyForm Noncommercial License 1.0.0 with the additional
// permission in MINING-PERMISSION (see LICENSE and MINING-PERMISSION at the top of this tree).

#pragma once
#define KMINB 3

#include <cstdio>
#include <cstdlib>
#include <cstdint>
#include <climits>
#include <vector>
#include <type_traits>
#include <cuda_runtime.h>

typedef long long i64;

#define CK(x) do { cudaError_t e_ = (x); if (e_ != cudaSuccess) { fprintf(stderr, "vela engine: CUDA error %d\n", (int)e_); exit(1); } } while (0)

__device__ __forceinline__ int sat32(i64 v) { return v > INT_MAX ? INT_MAX : (v < INT_MIN ? INT_MIN : (int)v); }

__device__ __forceinline__ int rshr(i64 v, int s) {
    if (s > 0) return sat32((v + (1LL << (s - 1))) >> s);
    if (s == 0) return sat32(v);
    int n = -s;
    if (v > (i64)(INT_MAX >> n)) return INT_MAX;
    if (v < (i64)(INT_MIN >> n)) return INT_MIN;
    return (int)(v << n);
}

__device__ __forceinline__ int exp_of(i64 m) {
    int e = 0;
    for (int k = 15; k < 32; k++) e += m >= (1LL << k);
    return e;
}

__device__ __forceinline__ int quant16(i64 v, int e) {
    int q = rshr(v, e);
    return q > 32767 ? 32767 : (q < -32767 ? -32767 : q);
}

__global__ void prep_rows(const int* __restrict__ x, int T, int Tp, int C, int8_t* hi, uint8_t* lo, int* e) {
    int t = blockIdx.x, h = blockIdx.y, d = threadIdx.x;
    i64 v = t < T ? (i64)x[(i64)t * C + h * 64 + d] : 0;
    __shared__ i64 sm[64];
    sm[d] = v < 0 ? -v : v;
    __syncthreads();
    for (int o = 32; o > 0; o >>= 1) { if (d < o && sm[d + o] > sm[d]) sm[d] = sm[d + o]; __syncthreads(); }
    int ee = exp_of(sm[0]);
    int q = quant16(v, ee);
    i64 at = ((i64)h * Tp + t) * 64 + d;
    hi[at] = (int8_t)(q >> 8);
    lo[at] = (uint8_t)(q & 255);
    if (d == 0) e[h * Tp + t] = ee;
}

__global__ void prep_v(const int* __restrict__ v, int T, int Tp, int C, int8_t* hi, uint8_t* lo, int* ev) {
    int hc = blockIdx.x, h = hc / 64, c = hc % 64, tid = threadIdx.x;
    i64 m = 0;
    for (int t = tid; t < T; t += blockDim.x) { i64 a = v[(i64)t * C + h * 64 + c]; a = a < 0 ? -a : a; if (a > m) m = a; }
    __shared__ i64 sm[256];
    sm[tid] = m;
    __syncthreads();
    for (int o = 128; o > 0; o >>= 1) { if (tid < o && sm[tid + o] > sm[tid]) sm[tid] = sm[tid + o]; __syncthreads(); }
    int ee = exp_of(sm[0]);
    for (int t = tid; t < Tp; t += blockDim.x) {
        int q = t < T ? quant16(v[(i64)t * C + h * 64 + c], ee) : 0;
        hi[(i64)hc * Tp + t] = (int8_t)(q >> 8);
        lo[(i64)hc * Tp + t] = (uint8_t)(q & 255);
    }
    if (tid == 0) ev[hc] = ee;
}

#define MMA(TA, TB, c, a, b)                                                                                 \
    asm volatile("mma.sync.aligned.m16n8k32.row.col.s32." TA "." TB ".s32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};" \
                 : "+r"(c[0]), "+r"(c[1]), "+r"(c[2]), "+r"(c[3])                                          \
                 : "r"(a[0]), "r"(a[1]), "r"(a[2]), "r"(a[3]), "r"(b[0]), "r"(b[1]))

__device__ __forceinline__ int s_of(int H, int R, int t) {
    if (t >= 16) return (H + ((R + (1 << (t - 1))) >> 16)) >> (t - 16);
    if (t >= 1) {
        int sh = 16 - t, hi = H >> (31 - sh);
        if (hi == 0 || hi == -1) {
            int low = (R + (1 << (t - 1))) >> t, r;
            asm("add.sat.s32 %0, %1, %2;" : "=r"(r) : "r"(H << sh), "r"(low));
            return r;
        }
    }
    return rshr((i64)H * 65536 + R, t);
}

__device__ __forceinline__ unsigned E_of(const int2* lut2, int mx, int s) {
    unsigned z = (unsigned)mx - (unsigned)s;
    if (z >= (1u << 20)) return 0;
    int2 e = lut2[z >> 10];
    unsigned f = z & 1023;
    return (unsigned)e.x - (unsigned)(e.y >> 10) * f - ((((unsigned)e.y & 1023) * f + 1023) >> 10);
}

constexpr int KS = 80;

struct Smem {
    int8_t kh[64 * KS];
    uint8_t kl[64 * KS];
    int8_t vh[64 * KS];
    uint8_t vl[64 * KS];
    int ek[64];
    int lut[1025];
};

__device__ __forceinline__ void ldsm4a(unsigned& r0, unsigned& r1, unsigned& r2, unsigned& r3, const void* p) {
    unsigned a = (unsigned)__cvta_generic_to_shared(p);
    asm volatile("ldmatrix.sync.aligned.m8n8.x4.shared.b16 {%0,%1,%2,%3}, [%4];" : "=r"(r0), "=r"(r1), "=r"(r2), "=r"(r3) : "r"(a));
}

__constant__ unsigned char PERM[64];
__constant__ unsigned char INVPERM[64];

__global__ void __launch_bounds__(128) attn_k(const int8_t* __restrict__ qh, const uint8_t* __restrict__ ql, const int* __restrict__ eqv,
                                              const int8_t* __restrict__ kh, const uint8_t* __restrict__ kl, const int* __restrict__ ekv,
                                              const int8_t* __restrict__ vh, const uint8_t* __restrict__ vl, const int* __restrict__ evv,
                                              const int* __restrict__ lutg, int Tq, int Tk, int Tkp, int C, int* __restrict__ out) {
    __shared__ Smem sm;
    const int h = blockIdx.y, warp = threadIdx.x >> 5, lane = threadIdx.x & 31, g = lane >> 2, q = lane & 3;
    const int r0 = blockIdx.x * 64 + warp * 16 + g;
    for (int i = threadIdx.x; i < 1025; i += 128) sm.lut[i] = lutg[i];

    unsigned aqh[2][4], aql[2][4];
    {
        const int8_t* Qh = qh + ((i64)h * Tq) * 64;
        const uint8_t* Ql = ql + ((i64)h * Tq) * 64;
        for (int ks = 0; ks < 2; ks++) {
            int c0 = ks * 32 + q * 4;
            aqh[ks][0] = *(const unsigned*)(Qh + (i64)r0 * 64 + c0);
            aqh[ks][1] = *(const unsigned*)(Qh + (i64)(r0 + 8) * 64 + c0);
            aqh[ks][2] = *(const unsigned*)(Qh + (i64)r0 * 64 + c0 + 16);
            aqh[ks][3] = *(const unsigned*)(Qh + (i64)(r0 + 8) * 64 + c0 + 16);
            aql[ks][0] = *(const unsigned*)(Ql + (i64)r0 * 64 + c0);
            aql[ks][1] = *(const unsigned*)(Ql + (i64)(r0 + 8) * 64 + c0);
            aql[ks][2] = *(const unsigned*)(Ql + (i64)r0 * 64 + c0 + 16);
            aql[ks][3] = *(const unsigned*)(Ql + (i64)(r0 + 8) * 64 + c0 + 16);
        }
    }
    const int eq0 = eqv[h * Tq + r0], eq1 = eqv[h * Tq + r0 + 8];
    const int ntiles = Tkp / 64;

    auto load_k = [&](int tile) {
        const int8_t* Kh = kh + ((i64)h * Tkp + tile * 64) * 64;
        const uint8_t* Kl = kl + ((i64)h * Tkp + tile * 64) * 64;
        for (int i = threadIdx.x; i < 64 * 16; i += 128) {
            int row = i >> 4, w = i & 15;
            *(unsigned*)(sm.kh + row * KS + w * 4) = *(const unsigned*)(Kh + row * 64 + w * 4);
            *(unsigned*)(sm.kl + row * KS + w * 4) = *(const unsigned*)(Kl + row * 64 + w * 4);
        }
        if (threadIdx.x < 64) sm.ek[threadIdx.x] = ekv[h * Tkp + tile * 64 + threadIdx.x];
    };

    auto scores = [&](int nt, int s[4]) {
        int chh[4] = {0, 0, 0, 0}, cmid[4] = {0, 0, 0, 0}, cll[4] = {0, 0, 0, 0};
        const int n = nt * 8 + g;
        for (int ks = 0; ks < 2; ks++) {
            unsigned bh[2], bl[2];
            bh[0] = *(const unsigned*)(sm.kh + n * KS + ks * 32 + q * 4);
            bh[1] = *(const unsigned*)(sm.kh + n * KS + ks * 32 + 16 + q * 4);
            bl[0] = *(const unsigned*)(sm.kl + n * KS + ks * 32 + q * 4);
            bl[1] = *(const unsigned*)(sm.kl + n * KS + ks * 32 + 16 + q * 4);
            MMA("s8", "s8", chh, aqh[ks], bh);
            MMA("s8", "u8", cmid, aqh[ks], bl);
            MMA("u8", "s8", cmid, aql[ks], bh);
            MMA("u8", "u8", cll, aql[ks], bl);
        }
        for (int i = 0; i < 4; i++) {
            i64 S = 65536LL * chh[i] + 256LL * cmid[i] + cll[i];
            int col = nt * 8 + q * 2 + (i & 1);
            int eqr = i < 2 ? eq0 : eq1;
            s[i] = rshr(S, 19 - eqr - sm.ek[col]);
        }
    };
    auto valid = [&](int tile, int nt, int i) { return tile * 64 + nt * 8 + q * 2 + (i & 1) < Tk; };
    auto Eof = [&](int mx, int s) -> i64 {
        i64 z = (i64)mx - s;
        if (z >= (16 << 16)) return 0;
        int zc = (int)z, ix = zc >> 10, f = zc & 1023;
        i64 L0 = sm.lut[ix], L1 = sm.lut[ix + 1];
        return L0 + (((L1 - L0) * f) >> 10);
    };

    int mx0 = INT_MIN, mx1 = INT_MIN;
    for (int tile = 0; tile < ntiles; tile++) {
        __syncthreads();
        load_k(tile);
        __syncthreads();
        for (int nt = 0; nt < 8; nt++) {
            int s[4];
            scores(nt, s);
            for (int i = 0; i < 4; i++)
                if (valid(tile, nt, i)) { if (i < 2) mx0 = max(mx0, s[i]); else mx1 = max(mx1, s[i]); }
        }
    }
    for (int o = 1; o < 4; o <<= 1) { mx0 = max(mx0, __shfl_xor_sync(~0u, mx0, o)); mx1 = max(mx1, __shfl_xor_sync(~0u, mx1, o)); }

    i64 se0 = 0, se1 = 0;
    for (int tile = 0; tile < ntiles; tile++) {
        __syncthreads();
        load_k(tile);
        __syncthreads();
        for (int nt = 0; nt < 8; nt++) {
            int s[4];
            scores(nt, s);
            for (int i = 0; i < 4; i++)
                if (valid(tile, nt, i)) { if (i < 2) se0 += Eof(mx0, s[i]); else se1 += Eof(mx1, s[i]); }
        }
    }
    for (int o = 1; o < 4; o <<= 1) { se0 += __shfl_xor_sync(~0u, se0, o); se1 += __shfl_xor_sync(~0u, se1, o); }
    const i64 rr0 = (1LL << 62) / se0, rr1 = (1LL << 62) / se1;

    int ohh[8][4], omid[8][4], oll[8][4];
    for (int c = 0; c < 8; c++) for (int i = 0; i < 4; i++) ohh[c][i] = omid[c][i] = oll[c][i] = 0;
    for (int tile = 0; tile < ntiles; tile++) {
        __syncthreads();
        load_k(tile);
        {
            const int8_t* Vh = vh + (i64)h * 64 * Tkp + tile * 64;
            const uint8_t* Vl = vl + (i64)h * 64 * Tkp + tile * 64;
            for (int i = threadIdx.x; i < 64 * 64; i += 128) {
                int ch = i >> 6, p = i & 63, key = PERM[p];
                sm.vh[ch * KS + p] = Vh[(i64)ch * Tkp + key];
                sm.vl[ch * KS + p] = Vl[(i64)ch * Tkp + key];
            }
        }
        __syncthreads();
        for (int kg = 0; kg < 2; kg++) {
            unsigned ph[4] = {0, 0, 0, 0}, pl[4] = {0, 0, 0, 0};
            for (int t4 = 0; t4 < 4; t4++) {
                int nt = kg * 4 + t4, s[4];
                scores(nt, s);
                for (int i = 0; i < 4; i++) {
                    int P = 0;
                    if (valid(tile, nt, i)) {
                        i64 E = Eof(i < 2 ? mx0 : mx1, s[i]);
                        i64 v = (E * (i < 2 ? rr0 : rr1) + (1LL << 46)) >> 47;
                        P = v > 32767 ? 32767 : (int)v;
                    }

                    int reg = (t4 >> 1) * 2 + (i >> 1), byte = (t4 & 1) * 2 + (i & 1);
                    ph[reg] |= (unsigned)(P >> 8) << (8 * byte);
                    pl[reg] |= (unsigned)(P & 255) << (8 * byte);
                }
            }
            for (int ct = 0; ct < 8; ct++) {
                const int ch = ct * 8 + g;
                unsigned bh[2], bl[2];
                bh[0] = *(const unsigned*)(sm.vh + ch * KS + kg * 32 + q * 4);
                bh[1] = *(const unsigned*)(sm.vh + ch * KS + kg * 32 + 16 + q * 4);
                bl[0] = *(const unsigned*)(sm.vl + ch * KS + kg * 32 + q * 4);
                bl[1] = *(const unsigned*)(sm.vl + ch * KS + kg * 32 + 16 + q * 4);
                MMA("s8", "s8", ohh[ct], ph, bh);
                MMA("s8", "u8", omid[ct], ph, bl);
                MMA("u8", "s8", omid[ct], pl, bh);
                MMA("u8", "u8", oll[ct], pl, bl);
            }
        }
    }
    for (int ct = 0; ct < 8; ct++)
        for (int i = 0; i < 4; i++) {
            int row = i < 2 ? r0 : r0 + 8, ch = ct * 8 + q * 2 + (i & 1);
            i64 O = 65536LL * ohh[ct][i] + 256LL * omid[ct][i] + oll[ct][i];
            out[(i64)row * C + h * 64 + ch] = rshr(O, 15 - evv[h * 64 + ch]);
        }
}

struct Smem1 {
    int8_t kh[64 * KS];
    uint8_t kl[64 * KS];
    int8_t vh[64 * KS];
    uint8_t vl[64 * KS];
    int tk[64];
    int2 lut2[1024];
};

__global__ void __launch_bounds__(128) attn_k1(const int8_t* __restrict__ qh, const uint8_t* __restrict__ ql, const int* __restrict__ eqv,
                                               const int8_t* __restrict__ kh, const uint8_t* __restrict__ kl, const int* __restrict__ ekv,
                                               const int8_t* __restrict__ vh, const uint8_t* __restrict__ vl, const int* __restrict__ evv,
                                               const int2* __restrict__ lut2g, int Tq, int Tk, int Tkp, int C, int* __restrict__ out) {
    __shared__ Smem1 sm;
    const int h = blockIdx.y, warp = threadIdx.x >> 5, lane = threadIdx.x & 31, g = lane >> 2, q = lane & 3;
    const int r0 = blockIdx.x * 64 + warp * 16 + g;
    for (int i = threadIdx.x; i < 1024; i += 128) sm.lut2[i] = lut2g[i];

    unsigned aqh[2][4], aql[2][4];
    {
        const int8_t* Qh = qh + ((i64)h * Tq) * 64;
        const uint8_t* Ql = ql + ((i64)h * Tq) * 64;
#pragma unroll
        for (int ks = 0; ks < 2; ks++) {
            int c0 = ks * 32 + q * 4;
            aqh[ks][0] = *(const unsigned*)(Qh + (i64)r0 * 64 + c0);
            aqh[ks][1] = *(const unsigned*)(Qh + (i64)(r0 + 8) * 64 + c0);
            aqh[ks][2] = *(const unsigned*)(Qh + (i64)r0 * 64 + c0 + 16);
            aqh[ks][3] = *(const unsigned*)(Qh + (i64)(r0 + 8) * 64 + c0 + 16);
            aql[ks][0] = *(const unsigned*)(Ql + (i64)r0 * 64 + c0);
            aql[ks][1] = *(const unsigned*)(Ql + (i64)(r0 + 8) * 64 + c0);
            aql[ks][2] = *(const unsigned*)(Ql + (i64)r0 * 64 + c0 + 16);
            aql[ks][3] = *(const unsigned*)(Ql + (i64)(r0 + 8) * 64 + c0 + 16);
        }
    }
    const int eq0 = eqv[h * Tq + r0], eq1 = eqv[h * Tq + r0 + 8];
    const int ntiles = Tkp / 64, nfull = Tk / 64;

    auto load_k = [&](int tile) {
        const int8_t* Kh = kh + ((i64)h * Tkp + tile * 64) * 64;
        const uint8_t* Kl = kl + ((i64)h * Tkp + tile * 64) * 64;
        for (int i = threadIdx.x; i < 64 * 16; i += 128) {
            int row = i >> 4, w = i & 15;
            *(unsigned*)(sm.kh + row * KS + w * 4) = *(const unsigned*)(Kh + row * 64 + w * 4);
            *(unsigned*)(sm.kl + row * KS + w * 4) = *(const unsigned*)(Kl + row * 64 + w * 4);
        }
        if (threadIdx.x < 64) sm.tk[threadIdx.x] = 19 - ekv[h * Tkp + tile * 64 + threadIdx.x];
    };
    auto scores = [&](int nt, int s[4]) {
        int chh[4] = {0, 0, 0, 0}, cmid[4] = {0, 0, 0, 0}, cll[4] = {0, 0, 0, 0};
        const int n = nt * 8 + g;
#pragma unroll
        for (int ks = 0; ks < 2; ks++) {
            unsigned bh[2], bl[2];
            bh[0] = *(const unsigned*)(sm.kh + n * KS + ks * 32 + q * 4);
            bh[1] = *(const unsigned*)(sm.kh + n * KS + ks * 32 + 16 + q * 4);
            bl[0] = *(const unsigned*)(sm.kl + n * KS + ks * 32 + q * 4);
            bl[1] = *(const unsigned*)(sm.kl + n * KS + ks * 32 + 16 + q * 4);
            MMA("s8", "s8", chh, aqh[ks], bh);
            MMA("s8", "u8", cmid, aqh[ks], bl);
            MMA("u8", "s8", cmid, aql[ks], bh);
            MMA("u8", "u8", cll, aql[ks], bl);
        }
        const int2 tt = *(const int2*)(sm.tk + nt * 8 + q * 2);
#pragma unroll
        for (int i = 0; i < 4; i++) s[i] = s_of(chh[i], cmid[i] * 256 + cll[i], ((i & 1) ? tt.y : tt.x) - (i < 2 ? eq0 : eq1));
    };

    int mx0 = INT_MIN, mx1 = INT_MIN;
    for (int tile = 0; tile < ntiles; tile++) {
        __syncthreads();
        load_k(tile);
        __syncthreads();
        const bool full = tile < nfull;
#pragma unroll
        for (int nt = 0; nt < 8; nt++) {
            int s[4];
            scores(nt, s);
#pragma unroll
            for (int i = 0; i < 4; i++) {
                if (!full && tile * 64 + nt * 8 + q * 2 + (i & 1) >= Tk) continue;
                if (i < 2) mx0 = max(mx0, s[i]); else mx1 = max(mx1, s[i]);
            }
        }
    }
#pragma unroll
    for (int o = 1; o < 4; o <<= 1) { mx0 = max(mx0, __shfl_xor_sync(~0u, mx0, o)); mx1 = max(mx1, __shfl_xor_sync(~0u, mx1, o)); }

    unsigned long long se0 = 0, se1 = 0;
    for (int tile = 0; tile < ntiles; tile++) {
        __syncthreads();
        load_k(tile);
        __syncthreads();
        const bool full = tile < nfull;
#pragma unroll
        for (int nt = 0; nt < 8; nt++) {
            int s[4];
            scores(nt, s);
#pragma unroll
            for (int i = 0; i < 4; i++) {
                if (!full && tile * 64 + nt * 8 + q * 2 + (i & 1) >= Tk) continue;
                if (i < 2) se0 += E_of(sm.lut2, mx0, s[i]); else se1 += E_of(sm.lut2, mx1, s[i]);
            }
        }
    }
#pragma unroll
    for (int o = 1; o < 4; o <<= 1) { se0 += __shfl_xor_sync(~0u, se0, o); se1 += __shfl_xor_sync(~0u, se1, o); }

    const unsigned long long rr0 = (1ULL << 62) / se0, rr1 = (1ULL << 62) / se1;
    const unsigned r0lo = (unsigned)rr0, r1lo = (unsigned)rr1;
    const bool r0big = rr0 >> 32, r1big = rr1 >> 32;

    int ohh[8][4], omid[8][4], oll[8][4];
#pragma unroll
    for (int c = 0; c < 8; c++)
#pragma unroll
        for (int i = 0; i < 4; i++) ohh[c][i] = omid[c][i] = oll[c][i] = 0;
    for (int tile = 0; tile < ntiles; tile++) {
        __syncthreads();
        load_k(tile);
        {
            const int8_t* Vh = vh + (i64)h * 64 * Tkp + tile * 64;
            const uint8_t* Vl = vl + (i64)h * 64 * Tkp + tile * 64;
            for (int i = threadIdx.x; i < 64 * 64; i += 128) {
                int ch = i >> 6, p = i & 63, key = PERM[p];
                sm.vh[ch * KS + p] = Vh[(i64)ch * Tkp + key];
                sm.vl[ch * KS + p] = Vl[(i64)ch * Tkp + key];
            }
        }
        __syncthreads();
        const bool full = tile < nfull;
#pragma unroll
        for (int kg = 0; kg < 2; kg++) {
            unsigned ph[4] = {0, 0, 0, 0}, pl[4] = {0, 0, 0, 0};
#pragma unroll
            for (int t4 = 0; t4 < 4; t4++) {
                int nt = kg * 4 + t4, s[4];
                scores(nt, s);
#pragma unroll
                for (int i = 0; i < 4; i++) {
                    unsigned P = 0;
                    if (full || tile * 64 + nt * 8 + q * 2 + (i & 1) < Tk) {
                        unsigned E = E_of(sm.lut2, i < 2 ? mx0 : mx1, s[i]);
                        bool big = i < 2 ? r0big : r1big;
                        unsigned long long x = big ? ((unsigned long long)E << 32) : (unsigned long long)E * (i < 2 ? r0lo : r1lo);
                        P = (unsigned)((x + (1ULL << 46)) >> 47);
                        P = P > 32767 ? 32767 : P;
                    }
                    int reg = (t4 >> 1) * 2 + (i >> 1), byte = (t4 & 1) * 2 + (i & 1);
                    ph[reg] |= (P >> 8) << (8 * byte);
                    pl[reg] |= (P & 255) << (8 * byte);
                }
            }
#pragma unroll
            for (int ct = 0; ct < 8; ct++) {
                const int ch = ct * 8 + g;
                unsigned bh[2], bl[2];
                bh[0] = *(const unsigned*)(sm.vh + ch * KS + kg * 32 + q * 4);
                bh[1] = *(const unsigned*)(sm.vh + ch * KS + kg * 32 + 16 + q * 4);
                bl[0] = *(const unsigned*)(sm.vl + ch * KS + kg * 32 + q * 4);
                bl[1] = *(const unsigned*)(sm.vl + ch * KS + kg * 32 + 16 + q * 4);
                MMA("s8", "s8", ohh[ct], ph, bh);
                MMA("s8", "u8", omid[ct], ph, bl);
                MMA("u8", "s8", omid[ct], pl, bh);
                MMA("u8", "u8", oll[ct], pl, bl);
            }
        }
    }
#pragma unroll
    for (int ct = 0; ct < 8; ct++)
#pragma unroll
        for (int i = 0; i < 4; i++) {
            int row = i < 2 ? r0 : r0 + 8, ch = ct * 8 + q * 2 + (i & 1);
            i64 O = 65536LL * ohh[ct][i] + 256LL * omid[ct][i] + oll[ct][i];
            out[(i64)row * C + h * 64 + ch] = rshr(O, 15 - evv[h * 64 + ch]);
        }
}

struct Smem2 {
    int8_t kh[64 * KS];
    uint8_t kl[64 * KS];
    int8_t vh[64 * KS];
    uint8_t vl[64 * KS];
    int tk[64];
    int flagw[2];
    int4 lut4[1024];
};

__device__ __forceinline__ int s_fast(int H, int R, int t) {
    return (H << (16 - t)) + ((R + (1 << (t - 1))) >> t);
}

__device__ __forceinline__ unsigned E_fast(const int4* lut4, int mx, int s) {
    unsigned z = (unsigned)mx - (unsigned)s;
    unsigned zc = min(z, (1u << 20) - 1);
    int4 e = lut4[zc >> 10];
    unsigned f = zc & 1023;
    unsigned E = (unsigned)(e.x + e.y * (int)f) - (((unsigned)e.z * f + 1023) >> 10);
    return z >= (1u << 20) ? 0u : E;
}

__global__ void __launch_bounds__(128) attn_k2(const int8_t* __restrict__ qh, const uint8_t* __restrict__ ql, const int* __restrict__ eqv,
                                               const int8_t* __restrict__ kh, const uint8_t* __restrict__ kl, const int* __restrict__ ekv,
                                               const int8_t* __restrict__ vh, const uint8_t* __restrict__ vl, const int* __restrict__ evv,
                                               const int4* __restrict__ lut4g, int Tq, int Tk, int Tkp, int C, int* __restrict__ out) {
    __shared__ Smem2 sm;
    __shared__ int eqmin_s, eqmax_s;
    const int h = blockIdx.y, warp = threadIdx.x >> 5, lane = threadIdx.x & 31, g = lane >> 2, q = lane & 3;
    const int r0 = blockIdx.x * 64 + warp * 16 + g;
    for (int i = threadIdx.x; i < 1024; i += 128) sm.lut4[i] = lut4g[i];
    if (threadIdx.x == 0) { eqmin_s = INT_MAX; eqmax_s = INT_MIN; }

    unsigned aqh[2][4], aql[2][4];
    {
        const int8_t* Qh = qh + ((i64)h * Tq) * 64;
        const uint8_t* Ql = ql + ((i64)h * Tq) * 64;
#pragma unroll
        for (int ks = 0; ks < 2; ks++) {
            int c0 = ks * 32 + q * 4;
            aqh[ks][0] = *(const unsigned*)(Qh + (i64)r0 * 64 + c0);
            aqh[ks][1] = *(const unsigned*)(Qh + (i64)(r0 + 8) * 64 + c0);
            aqh[ks][2] = *(const unsigned*)(Qh + (i64)r0 * 64 + c0 + 16);
            aqh[ks][3] = *(const unsigned*)(Qh + (i64)(r0 + 8) * 64 + c0 + 16);
            aql[ks][0] = *(const unsigned*)(Ql + (i64)r0 * 64 + c0);
            aql[ks][1] = *(const unsigned*)(Ql + (i64)(r0 + 8) * 64 + c0);
            aql[ks][2] = *(const unsigned*)(Ql + (i64)r0 * 64 + c0 + 16);
            aql[ks][3] = *(const unsigned*)(Ql + (i64)(r0 + 8) * 64 + c0 + 16);
        }
    }
    const int eq0 = eqv[h * Tq + r0], eq1 = eqv[h * Tq + r0 + 8];
    __syncthreads();
    atomicMin(&eqmin_s, min(eq0, eq1));
    atomicMax(&eqmax_s, max(eq0, eq1));
    __syncthreads();
    const int eqmin = eqmin_s, eqmax = eqmax_s;
    const int ntiles = Tkp / 64, nfull = Tk / 64;

    auto load_k = [&](int tile) {
        const int8_t* Kh = kh + ((i64)h * Tkp + tile * 64) * 64;
        const uint8_t* Kl = kl + ((i64)h * Tkp + tile * 64) * 64;
        for (int i = threadIdx.x; i < 64 * 4; i += 128) {
            int row = i >> 2, w = i & 3;
            *(uint4*)(sm.kh + row * KS + w * 16) = *(const uint4*)(Kh + row * 64 + w * 16);
            *(uint4*)(sm.kl + row * KS + w * 16) = *(const uint4*)(Kl + row * 64 + w * 16);
        }
        if (threadIdx.x < 64) {
            int key = tile * 64 + threadIdx.x;
            int tkv = 19 - ekv[h * Tkp + key];
            sm.tk[threadIdx.x] = tkv;
            bool ok = key >= Tk || (tkv - eqmax >= 6 && tkv - eqmin <= 16);
            ok = __all_sync(~0u, ok);
            if ((threadIdx.x & 31) == 0) sm.flagw[threadIdx.x >> 5] = ok;
        }
    };
    auto load_v = [&](int tile) {
        const int8_t* Vh = vh + (i64)h * 64 * Tkp + tile * 64;
        const uint8_t* Vl = vl + (i64)h * 64 * Tkp + tile * 64;
        for (int i = threadIdx.x; i < 64 * 4; i += 128) {
            int ch = i >> 2, w = i & 3;
            *(uint4*)(sm.vh + ch * KS + w * 16) = *(const uint4*)(Vh + (i64)ch * Tkp + w * 16);
            *(uint4*)(sm.vl + ch * KS + w * 16) = *(const uint4*)(Vl + (i64)ch * Tkp + w * 16);
        }
    };

    auto mma_qk = [&](int nt, int H[4], int R[4]) {
        int chh[4] = {0, 0, 0, 0}, cmid[4] = {0, 0, 0, 0}, cll[4] = {0, 0, 0, 0};
        const int n = nt * 8 + g;
#pragma unroll
        for (int ks = 0; ks < 2; ks++) {
            unsigned bh[2], bl[2];
            bh[0] = *(const unsigned*)(sm.kh + n * KS + ks * 32 + q * 4);
            bh[1] = *(const unsigned*)(sm.kh + n * KS + ks * 32 + 16 + q * 4);
            bl[0] = *(const unsigned*)(sm.kl + n * KS + ks * 32 + q * 4);
            bl[1] = *(const unsigned*)(sm.kl + n * KS + ks * 32 + 16 + q * 4);
            MMA("s8", "s8", chh, aqh[ks], bh);
            MMA("s8", "u8", cmid, aqh[ks], bl);
            MMA("u8", "s8", cmid, aql[ks], bh);
            MMA("u8", "u8", cll, aql[ks], bl);
        }
#pragma unroll
        for (int i = 0; i < 4; i++) { H[i] = chh[i]; R[i] = cmid[i] * 256 + cll[i]; }
    };
    auto scores = [&](auto fastc, int nt, int s[4]) {
        constexpr bool FAST = decltype(fastc)::value;
        int H[4], R[4];
        mma_qk(nt, H, R);
        const int2 tt = *(const int2*)(sm.tk + nt * 8 + q * 2);
#pragma unroll
        for (int i = 0; i < 4; i++) {
            int t = ((i & 1) ? tt.y : tt.x) - (i < 2 ? eq0 : eq1);
            s[i] = FAST ? s_fast(H[i], R[i], t) : s_of(H[i], R[i], t);
        }
    };
    auto inval = [&](int tile, int nt, int i) { return tile * 64 + nt * 8 + q * 2 + (i & 1) >= Tk; };

    int mx0 = INT_MIN, mx1 = INT_MIN;
    for (int tile = 0; tile < ntiles; tile++) {
        __syncthreads();
        load_k(tile);
        __syncthreads();
        auto body = [&](auto fastc) {
#pragma unroll
            for (int nt = 0; nt < 8; nt++) {
                int s[4];
                scores(fastc, nt, s);
                if (tile >= nfull)
#pragma unroll
                    for (int i = 0; i < 4; i++) if (inval(tile, nt, i)) s[i] = INT_MIN;
                mx0 = max(mx0, max(s[0], s[1]));
                mx1 = max(mx1, max(s[2], s[3]));
            }
        };
        if (sm.flagw[0] & sm.flagw[1]) body(std::true_type{}); else body(std::false_type{});
    }
#pragma unroll
    for (int o = 1; o < 4; o <<= 1) { mx0 = max(mx0, __shfl_xor_sync(~0u, mx0, o)); mx1 = max(mx1, __shfl_xor_sync(~0u, mx1, o)); }

    unsigned long long se0 = 0, se1 = 0;
    for (int tile = 0; tile < ntiles; tile++) {
        __syncthreads();
        load_k(tile);
        __syncthreads();
        auto body = [&](auto fastc) {
#pragma unroll
            for (int nt = 0; nt < 8; nt++) {
                int s[4];
                scores(fastc, nt, s);
                unsigned E[4];
#pragma unroll
                for (int i = 0; i < 4; i++) {
                    E[i] = E_fast(sm.lut4, i < 2 ? mx0 : mx1, s[i]);
                    if (tile >= nfull && inval(tile, nt, i)) E[i] = 0;
                }
                se0 += (unsigned long long)E[0] + E[1];
                se1 += (unsigned long long)E[2] + E[3];
            }
        };
        if (sm.flagw[0] & sm.flagw[1]) body(std::true_type{}); else body(std::false_type{});
    }
#pragma unroll
    for (int o = 1; o < 4; o <<= 1) { se0 += __shfl_xor_sync(~0u, se0, o); se1 += __shfl_xor_sync(~0u, se1, o); }
    const unsigned long long rr0 = (1ULL << 62) / se0, rr1 = (1ULL << 62) / se1;

    const unsigned rm0 = rr0 >> 32 ? 0u : (unsigned)rr0, rm1 = rr1 >> 32 ? 0u : (unsigned)rr1;
    const unsigned rb0 = rr0 >> 32 ? ~0u : 0u, rb1 = rr1 >> 32 ? ~0u : 0u;

    int ohh[8][4], omid[8][4], oll[8][4];
#pragma unroll
    for (int c = 0; c < 8; c++)
#pragma unroll
        for (int i = 0; i < 4; i++) ohh[c][i] = omid[c][i] = oll[c][i] = 0;
    for (int tile = 0; tile < ntiles; tile++) {
        __syncthreads();
        load_k(tile);
        load_v(tile);
        __syncthreads();
        auto body = [&](auto fastc) {
#pragma unroll
            for (int kg = 0; kg < 2; kg++) {
                unsigned w[4][2];
#pragma unroll
                for (int t4 = 0; t4 < 4; t4++) {
                    int nt = kg * 4 + t4, s[4];
                    scores(fastc, nt, s);
                    unsigned P[4];
#pragma unroll
                    for (int i = 0; i < 4; i++) {
                        unsigned E = E_fast(sm.lut4, i < 2 ? mx0 : mx1, s[i]);
                        unsigned x = __umulhi(E, i < 2 ? rm0 : rm1) + (E & (i < 2 ? rb0 : rb1));
                        P[i] = min((x + 16384u) >> 15, 32767u);
                        if (tile >= nfull && inval(tile, nt, i)) P[i] = 0;
                    }
                    w[t4][0] = P[0] | (P[1] << 16);
                    w[t4][1] = P[2] | (P[3] << 16);
                }
                unsigned ph[4], pl[4];
#pragma unroll
                for (int reg = 0; reg < 4; reg++) {
                    unsigned a = w[(reg >> 1) * 2][reg & 1], b = w[(reg >> 1) * 2 + 1][reg & 1];
                    pl[reg] = __byte_perm(a, b, 0x6420);
                    ph[reg] = __byte_perm(a, b, 0x7531);
                }
#pragma unroll
                for (int ct = 0; ct < 8; ct++) {
                    const int ch = ct * 8 + g;
                    unsigned bh[2], bl[2];
                    bh[0] = *(const unsigned*)(sm.vh + ch * KS + kg * 32 + q * 4);
                    bh[1] = *(const unsigned*)(sm.vh + ch * KS + kg * 32 + 16 + q * 4);
                    bl[0] = *(const unsigned*)(sm.vl + ch * KS + kg * 32 + q * 4);
                    bl[1] = *(const unsigned*)(sm.vl + ch * KS + kg * 32 + 16 + q * 4);
                    MMA("s8", "s8", ohh[ct], ph, bh);
                    MMA("s8", "u8", omid[ct], ph, bl);
                    MMA("u8", "s8", omid[ct], pl, bh);
                    MMA("u8", "u8", oll[ct], pl, bl);
                }
            }
        };
        if (sm.flagw[0] & sm.flagw[1]) body(std::true_type{}); else body(std::false_type{});
    }
#pragma unroll
    for (int ct = 0; ct < 8; ct++)
#pragma unroll
        for (int i = 0; i < 4; i++) {
            int row = i < 2 ? r0 : r0 + 8, ch = ct * 8 + q * 2 + (i & 1);
            i64 O = 65536LL * ohh[ct][i] + 256LL * omid[ct][i] + oll[ct][i];
            out[(i64)row * C + h * 64 + ch] = rshr(O, 15 - evv[h * 64 + ch]);
        }
}

struct Smem3 {
    int8_t kh[64 * KS];
    uint8_t kl[64 * KS];
    int tk[64];
    int flagw[2];
    int4 lut4[1024];
};
struct Smem3p {
    int8_t kh[64 * KS];
    uint8_t kl[64 * KS];
    int tk[64];
    int flagw[2];
    int2 lut4[1024];
};
__device__ __forceinline__ unsigned E_fast(const int2* lut2, int mx, int s) {
    unsigned z = (unsigned)mx - (unsigned)s;
    unsigned zc = min(z, (1u << 20) - 1);
    int2 e = lut2[zc >> 10];
    unsigned f = zc & 1023;
    unsigned E = (unsigned)e.x - ((unsigned)e.y >> 10) * f - ((((unsigned)e.y & 1023) * f + 1023) >> 10);
    return z >= (1u << 20) ? 0u : E;
}
struct __align__(16) SmemV {
    int8_t vh[64 * KS];
    uint8_t vl[64 * KS];
};

#define LUT2 1
template <int MODE, int WARPS>
__global__ void __launch_bounds__(WARPS * 32, (MODE == 1 && WARPS == 4) ? KMINB : 1) attn_k3(const int8_t* __restrict__ qh, const uint8_t* __restrict__ ql, const int* __restrict__ eqv,
                                               const int8_t* __restrict__ kh, const uint8_t* __restrict__ kl, const int* __restrict__ ekv,
                                               const int8_t* __restrict__ vh, const uint8_t* __restrict__ vl, const int* __restrict__ evv,
                                               const int4* __restrict__ lut4g, int Tq, int Tk, int Tkp, int C, int* __restrict__ out,
                                               int* __restrict__ mxg, unsigned long long* __restrict__ rrg,
                                               int Tq1, int Tkp1, int H, int kshared, unsigned* __restrict__ rmaxo = nullptr, int onlyBig = 0) {

    __shared__ std::conditional_t<(MODE != 2 && LUT2), Smem3p, Smem3> sm;
    constexpr int NT = WARPS * 32;
    __shared__ int eqmin_s, eqmax_s;
    const int bimg = blockIdx.y / H, h = blockIdx.y % H, warp = threadIdx.x >> 5, lane = threadIdx.x & 31, g = lane >> 2, q = lane & 3;
    const int kb = kshared ? 0 : bimg * Tkp1, eb = kshared ? 0 : bimg;
    const int r0 = bimg * Tq1 + blockIdx.x * (WARPS * 16) + warp * 16 + g;
    for (int i = threadIdx.x; i < 1024; i += NT) {
        int4 e = lut4g[i];
        if constexpr (MODE != 2 && LUT2) sm.lut4[i] = make_int2(e.x, ((-e.y) << 10) | e.z);
        else sm.lut4[i] = e;
    }
    if (threadIdx.x == 0) { eqmin_s = INT_MAX; eqmax_s = INT_MIN; }

    unsigned aqh[2][4], aql[2][4];
    {
        const int8_t* Qh = qh + ((i64)h * Tq) * 64;
        const uint8_t* Ql = ql + ((i64)h * Tq) * 64;
#pragma unroll
        for (int ks = 0; ks < 2; ks++) {
            int c0 = ks * 32 + q * 4;
            aqh[ks][0] = *(const unsigned*)(Qh + (i64)r0 * 64 + c0);
            aqh[ks][1] = *(const unsigned*)(Qh + (i64)(r0 + 8) * 64 + c0);
            aqh[ks][2] = *(const unsigned*)(Qh + (i64)r0 * 64 + c0 + 16);
            aqh[ks][3] = *(const unsigned*)(Qh + (i64)(r0 + 8) * 64 + c0 + 16);
            aql[ks][0] = *(const unsigned*)(Ql + (i64)r0 * 64 + c0);
            aql[ks][1] = *(const unsigned*)(Ql + (i64)(r0 + 8) * 64 + c0);
            aql[ks][2] = *(const unsigned*)(Ql + (i64)r0 * 64 + c0 + 16);
            aql[ks][3] = *(const unsigned*)(Ql + (i64)(r0 + 8) * 64 + c0 + 16);
        }
    }
    const int eq0 = eqv[h * Tq + r0], eq1 = eqv[h * Tq + r0 + 8];
    __syncthreads();
    atomicMin(&eqmin_s, min(eq0, eq1));
    atomicMax(&eqmax_s, max(eq0, eq1));
    __syncthreads();
    const int eqmin = eqmin_s, eqmax = eqmax_s;
    const int ntiles = Tkp1 / 64, nfull = Tk / 64;

    auto load_k = [&](int tile) {
        const int8_t* Kh = kh + ((i64)h * Tkp + kb + tile * 64) * 64;
        const uint8_t* Kl = kl + ((i64)h * Tkp + kb + tile * 64) * 64;
        for (int i = threadIdx.x; i < 64 * 4; i += NT) {
            int row = i >> 2, w = i & 3;
            *(uint4*)(sm.kh + row * KS + w * 16) = *(const uint4*)(Kh + row * 64 + w * 16);
            *(uint4*)(sm.kl + row * KS + w * 16) = *(const uint4*)(Kl + row * 64 + w * 16);
        }
        if (threadIdx.x < 64) {
            int key = tile * 64 + threadIdx.x;
            int tkv = 19 - ekv[h * Tkp + kb + key];
            sm.tk[threadIdx.x] = tkv;
            bool ok = key >= Tk || (tkv - eqmax >= 6 && tkv - eqmin <= 16);
            ok = __all_sync(~0u, ok);
            if ((threadIdx.x & 31) == 0) sm.flagw[threadIdx.x >> 5] = ok;
        }
    };
    auto load_v = [&](SmemV& smv, int tile) {
        const int8_t* Vh = vh + (i64)h * 64 * Tkp + kb + tile * 64;
        const uint8_t* Vl = vl + (i64)h * 64 * Tkp + kb + tile * 64;
        for (int i = threadIdx.x; i < 64 * 4; i += NT) {
            int ch = i >> 2, w = i & 3;
            *(uint4*)(smv.vh + ch * KS + w * 16) = *(const uint4*)(Vh + (i64)ch * Tkp + w * 16);
            *(uint4*)(smv.vl + ch * KS + w * 16) = *(const uint4*)(Vl + (i64)ch * Tkp + w * 16);
        }
    };

    auto mma_qk = [&](int nt, int H[4], int R[4]) {
        int chh[4] = {0, 0, 0, 0}, cmid[4] = {0, 0, 0, 0}, cll[4] = {0, 0, 0, 0};
        const int n = nt * 8 + g;
#pragma unroll
        for (int ks = 0; ks < 2; ks++) {
            unsigned bh[2], bl[2];
            {
                const int mi = lane >> 3, rr = nt * 8 + (lane & 7);
                const void* src = (mi < 2 ? (const void*)(sm.kh) : (const void*)(sm.kl)) ;
                ldsm4a(bh[0], bh[1], bl[0], bl[1], (const unsigned char*)src + rr * KS + ks * 32 + (mi & 1) * 16);
            }
            MMA("s8", "s8", chh, aqh[ks], bh);
            MMA("s8", "u8", cmid, aqh[ks], bl);
            MMA("u8", "s8", cmid, aql[ks], bh);
            MMA("u8", "u8", cll, aql[ks], bl);
        }
#pragma unroll
        for (int i = 0; i < 4; i++) { H[i] = chh[i]; R[i] = cmid[i] * 256 + cll[i]; }
    };
    auto scores = [&](auto fastc, int nt, int s[4]) {
        constexpr bool FAST = decltype(fastc)::value;
        int H[4], R[4];
        mma_qk(nt, H, R);
        const int2 tt = *(const int2*)(sm.tk + nt * 8 + q * 2);
#pragma unroll
        for (int i = 0; i < 4; i++) {
            int t = ((i & 1) ? tt.y : tt.x) - (i < 2 ? eq0 : eq1);
            s[i] = FAST ? s_fast(H[i], R[i], t) : s_of(H[i], R[i], t);
        }
    };
    auto inval = [&](int tile, int nt, int i) { return tile * 64 + nt * 8 + q * 2 + (i & 1) >= Tk; };

    int mx0 = INT_MIN, mx1 = INT_MIN;
    unsigned long long rr0, rr1;
    if constexpr (MODE != 1) {

    for (int tile = 0; tile < ntiles; tile++) {
        __syncthreads();
        load_k(tile);
        __syncthreads();
        auto body = [&](auto fastc) {
#pragma unroll
            for (int nt = 0; nt < 8; nt++) {
                int s[4];
                scores(fastc, nt, s);
                if (tile >= nfull)
#pragma unroll
                    for (int i = 0; i < 4; i++) if (inval(tile, nt, i)) s[i] = INT_MIN;
                mx0 = max(mx0, max(s[0], s[1]));
                mx1 = max(mx1, max(s[2], s[3]));
            }
        };
        if (sm.flagw[0] & sm.flagw[1]) body(std::true_type{}); else body(std::false_type{});
    }
#pragma unroll
    for (int o = 1; o < 4; o <<= 1) { mx0 = max(mx0, __shfl_xor_sync(~0u, mx0, o)); mx1 = max(mx1, __shfl_xor_sync(~0u, mx1, o)); }

    unsigned long long se0 = 0, se1 = 0;
    for (int tile = 0; tile < ntiles; tile++) {
        __syncthreads();
        load_k(tile);
        __syncthreads();
        auto body = [&](auto fastc) {
#pragma unroll
            for (int nt = 0; nt < 8; nt++) {
                int s[4];
                scores(fastc, nt, s);
                unsigned E[4];
#pragma unroll
                for (int i = 0; i < 4; i++) {
                    E[i] = E_fast(sm.lut4, i < 2 ? mx0 : mx1, s[i]);
                    if (tile >= nfull && inval(tile, nt, i)) E[i] = 0;
                }
                se0 += (unsigned long long)E[0] + E[1];
                se1 += (unsigned long long)E[2] + E[3];
            }
        };
        if (sm.flagw[0] & sm.flagw[1]) body(std::true_type{}); else body(std::false_type{});
    }
#pragma unroll
    for (int o = 1; o < 4; o <<= 1) { se0 += __shfl_xor_sync(~0u, se0, o); se1 += __shfl_xor_sync(~0u, se1, o); }
    rr0 = (1ULL << 62) / se0; rr1 = (1ULL << 62) / se1;
    if constexpr (MODE == 0) {
        if (q == 0) { mxg[h * Tq + r0] = mx0; mxg[h * Tq + r0 + 8] = mx1; rrg[h * Tq + r0] = rr0; rrg[h * Tq + r0 + 8] = rr1; }
        return;
    }
    } else {
        mx0 = mxg[h * Tq + r0]; mx1 = mxg[h * Tq + r0 + 8]; rr0 = rrg[h * Tq + r0]; rr1 = rrg[h * Tq + r0 + 8];
    }

    const unsigned rm0 = rr0 >> 32 ? 0u : (unsigned)rr0, rm1 = rr1 >> 32 ? 0u : (unsigned)rr1;
    const unsigned rb0 = rr0 >> 32 ? ~0u : 0u, rb1 = rr1 >> 32 ? ~0u : 0u;

    const bool smallP = __all_sync(~0u, (((1ULL << 30) * rr0 + (1ULL << 46)) >> 47) <= 255 && (((1ULL << 30) * rr1 + (1ULL << 46)) >> 47) <= 255);

    if constexpr (MODE != 0) {
    if (onlyBig) {
        __shared__ int allsm;
        if (threadIdx.x == 0) allsm = 1;
        __syncthreads();
        if (!smallP) allsm = 0;
        __syncthreads();
        if (allsm) return;
    }
    __shared__ SmemV smv;

    int ohh[8][4], omid[8][4], oll[8][4];
#pragma unroll
    for (int c = 0; c < 8; c++)
#pragma unroll
        for (int i = 0; i < 4; i++) ohh[c][i] = omid[c][i] = oll[c][i] = 0;
    for (int tile = 0; tile < ntiles; tile++) {
        __syncthreads();
        load_k(tile);
        load_v(smv, tile);
        __syncthreads();
        auto body = [&](auto fastc) {
#pragma unroll
            for (int kg = 0; kg < 2; kg++) {
                unsigned w[4][2];
#pragma unroll
                for (int t4 = 0; t4 < 4; t4++) {
                    int nt = kg * 4 + t4, s[4];
                    scores(fastc, nt, s);
                    unsigned P[4];
#pragma unroll
                    for (int i = 0; i < 4; i++) {
                        unsigned E = E_fast(sm.lut4, i < 2 ? mx0 : mx1, s[i]);
                        unsigned x = __umulhi(E, i < 2 ? rm0 : rm1) + (E & (i < 2 ? rb0 : rb1));
                        P[i] = min((x + 16384u) >> 15, 32767u);
                        if (tile >= nfull && inval(tile, nt, i)) P[i] = 0;
                    }
                    w[t4][0] = P[0] | (P[1] << 16);
                    w[t4][1] = P[2] | (P[3] << 16);
                }
                unsigned ph[4], pl[4];
#pragma unroll
                for (int reg = 0; reg < 4; reg++) {
                    unsigned a = w[(reg >> 1) * 2][reg & 1], b = w[(reg >> 1) * 2 + 1][reg & 1];
                    pl[reg] = __byte_perm(a, b, 0x6420);
                    ph[reg] = __byte_perm(a, b, 0x7531);
                }
                if (smallP || !__any_sync(~0u, (ph[0] | ph[1] | ph[2] | ph[3]) != 0)) {
#pragma unroll
                    for (int ct = 0; ct < 8; ct++) {
                        const int ch = ct * 8 + g;
                        unsigned bh[2], bl[2];
                        {
                            const int mi = lane >> 3, rr = ct * 8 + (lane & 7);
                            const void* src = mi < 2 ? (const void*)smv.vh : (const void*)smv.vl;
                            ldsm4a(bh[0], bh[1], bl[0], bl[1], (const unsigned char*)src + rr * KS + kg * 32 + (mi & 1) * 16);
                        }
                        MMA("u8", "s8", omid[ct], pl, bh);
                        MMA("u8", "u8", oll[ct], pl, bl);
                    }
                } else {
#pragma unroll
                for (int ct = 0; ct < 8; ct++) {
                    const int ch = ct * 8 + g;
                    unsigned bh[2], bl[2];
                    {
                        const int mi = lane >> 3, rr = ct * 8 + (lane & 7);
                        const void* src = mi < 2 ? (const void*)smv.vh : (const void*)smv.vl;
                        ldsm4a(bh[0], bh[1], bl[0], bl[1], (const unsigned char*)src + rr * KS + kg * 32 + (mi & 1) * 16);
                    }
                    MMA("s8", "s8", ohh[ct], ph, bh);
                    MMA("s8", "u8", omid[ct], ph, bl);
                    MMA("u8", "s8", omid[ct], pl, bh);
                    MMA("u8", "u8", oll[ct], pl, bl);
                }
                }
            }
        };
        if (sm.flagw[0] & sm.flagw[1]) body(std::true_type{}); else body(std::false_type{});
    }
    unsigned am0 = 0, am1 = 0;
#pragma unroll
    for (int ct = 0; ct < 8; ct++)
#pragma unroll
        for (int i = 0; i < 4; i++) {
            int row = i < 2 ? r0 : r0 + 8, ch = ct * 8 + q * 2 + (i & 1);
            i64 O = 65536LL * ohh[ct][i] + 256LL * omid[ct][i] + oll[ct][i];
            const int ov = rshr(O, 15 - evv[(eb * H + h) * 64 + ch]);
            out[(i64)row * C + h * 64 + ch] = ov;
            const unsigned a = ov < 0 ? 0u - (unsigned)ov : (unsigned)ov;
            if (i < 2) am0 = max(am0, a); else am1 = max(am1, a);
        }
    if (rmaxo) {
        am0 = max(am0, __shfl_xor_sync(~0u, am0, 1)); am0 = max(am0, __shfl_xor_sync(~0u, am0, 2));
        am1 = max(am1, __shfl_xor_sync(~0u, am1, 1)); am1 = max(am1, __shfl_xor_sync(~0u, am1, 2));
        if (q == 0) { atomicMax(&rmaxo[r0], am0); atomicMax(&rmaxo[r0 + 8], am1); }
    }
    }
}

#define KMINB4 2
struct __align__(16) Smem4 {
    int8_t kh[2][64 * KS];
    uint8_t kl[2][64 * KS];
    int ek[2][64];
    int2 lut4[1024];
};
struct __align__(16) SmemV4 {
    int8_t vh[2][64 * KS];
    uint8_t vl[2][64 * KS];
};
template <int MODE> constexpr int k4smem() { return (int)sizeof(Smem4) + (MODE != 0 ? (int)sizeof(SmemV4) : 0); }
__device__ __forceinline__ void cp16(void* dst, const void* src, bool ok) {
    unsigned d = (unsigned)__cvta_generic_to_shared(dst);
    asm volatile("cp.async.cg.shared.global [%0], [%1], 16, %2;" ::"r"(d), "l"(src), "r"(ok ? 16 : 0));
}

template <int MODE, int WARPS>

#if defined(__CUDA_ARCH__) && __CUDA_ARCH__ >= 1200
#define K4MINB8 4
#else
#define K4MINB8 1
#endif
__global__ void __launch_bounds__(WARPS * 32, (MODE == 1 && WARPS == 4) ? KMINB4 : (MODE == 0 && WARPS == 8) ? K4MINB8 : 1) attn_k4(const int8_t* __restrict__ qh, const uint8_t* __restrict__ ql, const int* __restrict__ eqv,
                                               const int8_t* __restrict__ kh, const uint8_t* __restrict__ kl, const int* __restrict__ ekv,
                                               const int8_t* __restrict__ vh, const uint8_t* __restrict__ vl, const int* __restrict__ evv,
                                               const int4* __restrict__ lut4g, int Tq, int Tk, int Tkp, int C, int* __restrict__ out,
                                               int* __restrict__ mxg, unsigned long long* __restrict__ rrg,
                                               int Tq1, int Tkp1, int H, int kshared) {

    extern __shared__ __align__(16) unsigned char dsm4[];
    Smem4& sm = *reinterpret_cast<Smem4*>(dsm4);
    constexpr int NT = WARPS * 32;
    __shared__ int eqmin_s, eqmax_s;
    const int bimg = blockIdx.y / H, h = blockIdx.y % H, warp = threadIdx.x >> 5, lane = threadIdx.x & 31, g = lane >> 2, q = lane & 3;
    const int kb = kshared ? 0 : bimg * Tkp1, eb = kshared ? 0 : bimg;
    const int r0 = bimg * Tq1 + blockIdx.x * (WARPS * 16) + warp * 16 + g;
    for (int i = threadIdx.x; i < 1024; i += NT) {
        int4 e = lut4g[i];
        sm.lut4[i] = make_int2(e.x, ((-e.y) << 10) | e.z);
    }
    if (threadIdx.x == 0) { eqmin_s = INT_MAX; eqmax_s = INT_MIN; }

    unsigned aqh[2][4], aql[2][4];
    {
        const int8_t* Qh = qh + ((i64)h * Tq) * 64;
        const uint8_t* Ql = ql + ((i64)h * Tq) * 64;
#pragma unroll
        for (int ks = 0; ks < 2; ks++) {
            int c0 = ks * 32 + q * 4;
            aqh[ks][0] = *(const unsigned*)(Qh + (i64)r0 * 64 + c0);
            aqh[ks][1] = *(const unsigned*)(Qh + (i64)(r0 + 8) * 64 + c0);
            aqh[ks][2] = *(const unsigned*)(Qh + (i64)r0 * 64 + c0 + 16);
            aqh[ks][3] = *(const unsigned*)(Qh + (i64)(r0 + 8) * 64 + c0 + 16);
            aql[ks][0] = *(const unsigned*)(Ql + (i64)r0 * 64 + c0);
            aql[ks][1] = *(const unsigned*)(Ql + (i64)(r0 + 8) * 64 + c0);
            aql[ks][2] = *(const unsigned*)(Ql + (i64)r0 * 64 + c0 + 16);
            aql[ks][3] = *(const unsigned*)(Ql + (i64)(r0 + 8) * 64 + c0 + 16);
        }
    }
    const int eq0 = eqv[h * Tq + r0], eq1 = eqv[h * Tq + r0 + 8];
    __syncthreads();
    atomicMin(&eqmin_s, min(eq0, eq1));
    atomicMax(&eqmax_s, max(eq0, eq1));
    __syncthreads();
    const int eqmin = eqmin_s, eqmax = eqmax_s;
    const int ntiles = Tkp1 / 64, nfull = Tk / 64;

    auto load_k = [&](int tile, int bf) {
        const int8_t* Kh = kh + ((i64)h * Tkp + kb + tile * 64) * 64;
        const uint8_t* Kl = kl + ((i64)h * Tkp + kb + tile * 64) * 64;
        for (int i = threadIdx.x; i < 64 * 4; i += NT) {
            int row = i >> 2, w = i & 3;
            cp16(sm.kh[bf] + row * KS + w * 16, Kh + row * 64 + w * 16, true);
            cp16(sm.kl[bf] + row * KS + w * 16, Kl + row * 64 + w * 16, true);
        }
        if (threadIdx.x < 16) cp16(sm.ek[bf] + threadIdx.x * 4, ekv + h * Tkp + kb + tile * 64 + threadIdx.x * 4, true);
    };
    auto load_v = [&](SmemV4& smv, int tile, int bf) {
        const int8_t* Vh = vh + (i64)h * 64 * Tkp + kb + tile * 64;
        const uint8_t* Vl = vl + (i64)h * 64 * Tkp + kb + tile * 64;
        for (int i = threadIdx.x; i < 64 * 4; i += NT) {
            int ch = i >> 2, w = i & 3;
            cp16(smv.vh[bf] + ch * KS + w * 16, Vh + (i64)ch * Tkp + w * 16, true);
            cp16(smv.vl[bf] + ch * KS + w * 16, Vl + (i64)ch * Tkp + w * 16, true);
        }
    };

    auto fast_of = [&](int tile, int bf) {
        bool ok = true;
#pragma unroll
        for (int j = 0; j < 2; j++) {
            int key = lane + 32 * j, tkv = 19 - sm.ek[bf][key];
            ok = ok && (tile * 64 + key >= Tk || (tkv - eqmax >= 6 && tkv - eqmin <= 16));
        }
        return __all_sync(~0u, ok);
    };
    int curb = 0;

    auto mma_qk = [&](int nt, int H[4], int R[4]) {
        int chh[4] = {0, 0, 0, 0}, cmid[4] = {0, 0, 0, 0}, cll[4] = {0, 0, 0, 0};
        const int n = nt * 8 + g;
#pragma unroll
        for (int ks = 0; ks < 2; ks++) {
            unsigned bh[2], bl[2];
            {
                const int mi = lane >> 3, rr = nt * 8 + (lane & 7);
                const void* src = (mi < 2 ? (const void*)(sm.kh[curb]) : (const void*)(sm.kl[curb])) ;
                ldsm4a(bh[0], bh[1], bl[0], bl[1], (const unsigned char*)src + rr * KS + ks * 32 + (mi & 1) * 16);
            }
            MMA("s8", "s8", chh, aqh[ks], bh);
            MMA("s8", "u8", cmid, aqh[ks], bl);
            MMA("u8", "s8", cmid, aql[ks], bh);
            MMA("u8", "u8", cll, aql[ks], bl);
        }
#pragma unroll
        for (int i = 0; i < 4; i++) { H[i] = chh[i]; R[i] = cmid[i] * 256 + cll[i]; }
    };
    auto scores = [&](auto fastc, int nt, int s[4]) {
        constexpr bool FAST = decltype(fastc)::value;
        int H[4], R[4];
        mma_qk(nt, H, R);
        const int2 ek2 = *(const int2*)(sm.ek[curb] + nt * 8 + q * 2);
        const int2 tt = make_int2(19 - ek2.x, 19 - ek2.y);
#pragma unroll
        for (int i = 0; i < 4; i++) {
            int t = ((i & 1) ? tt.y : tt.x) - (i < 2 ? eq0 : eq1);
            s[i] = FAST ? s_fast(H[i], R[i], t) : s_of(H[i], R[i], t);
        }
    };
    auto inval = [&](int tile, int nt, int i) { return tile * 64 + nt * 8 + q * 2 + (i & 1) >= Tk; };

    int mx0 = INT_MIN, mx1 = INT_MIN;
    unsigned long long rr0, rr1;
    if constexpr (MODE != 1) {

    load_k(0, 0);
    asm volatile("cp.async.commit_group;");
    for (int tile = 0; tile < ntiles; tile++) {
        if (tile + 1 < ntiles) load_k(tile + 1, (tile + 1) & 1);
        asm volatile("cp.async.commit_group;");
        asm volatile("cp.async.wait_group 1;");
        __syncthreads();
        curb = tile & 1;
        auto body = [&](auto fastc) {
#pragma unroll
            for (int nt = 0; nt < 8; nt++) {
                int s[4];
                scores(fastc, nt, s);
                if (tile >= nfull)
#pragma unroll
                    for (int i = 0; i < 4; i++) if (inval(tile, nt, i)) s[i] = INT_MIN;
                mx0 = max(mx0, max(s[0], s[1]));
                mx1 = max(mx1, max(s[2], s[3]));
            }
        };
        if (fast_of(tile, curb)) body(std::true_type{}); else body(std::false_type{});
        __syncthreads();
    }
#pragma unroll
    for (int o = 1; o < 4; o <<= 1) { mx0 = max(mx0, __shfl_xor_sync(~0u, mx0, o)); mx1 = max(mx1, __shfl_xor_sync(~0u, mx1, o)); }

    unsigned long long se0 = 0, se1 = 0;
    load_k(0, 0);
    asm volatile("cp.async.commit_group;");
    for (int tile = 0; tile < ntiles; tile++) {
        if (tile + 1 < ntiles) load_k(tile + 1, (tile + 1) & 1);
        asm volatile("cp.async.commit_group;");
        asm volatile("cp.async.wait_group 1;");
        __syncthreads();
        curb = tile & 1;
        auto body = [&](auto fastc) {
#pragma unroll
            for (int nt = 0; nt < 8; nt++) {
                int s[4];
                scores(fastc, nt, s);
                unsigned E[4];
#pragma unroll
                for (int i = 0; i < 4; i++) {
                    E[i] = E_fast(sm.lut4, i < 2 ? mx0 : mx1, s[i]);
                    if (tile >= nfull && inval(tile, nt, i)) E[i] = 0;
                }
                se0 += (unsigned long long)E[0] + E[1];
                se1 += (unsigned long long)E[2] + E[3];
            }
        };
        if (fast_of(tile, curb)) body(std::true_type{}); else body(std::false_type{});
        __syncthreads();
    }
#pragma unroll
    for (int o = 1; o < 4; o <<= 1) { se0 += __shfl_xor_sync(~0u, se0, o); se1 += __shfl_xor_sync(~0u, se1, o); }
    rr0 = (1ULL << 62) / se0; rr1 = (1ULL << 62) / se1;
    if constexpr (MODE == 0) {
        if (q == 0) { mxg[h * Tq + r0] = mx0; mxg[h * Tq + r0 + 8] = mx1; rrg[h * Tq + r0] = rr0; rrg[h * Tq + r0 + 8] = rr1; }
        return;
    }
    } else {
        mx0 = mxg[h * Tq + r0]; mx1 = mxg[h * Tq + r0 + 8]; rr0 = rrg[h * Tq + r0]; rr1 = rrg[h * Tq + r0 + 8];
    }

    const unsigned rm0 = rr0 >> 32 ? 0u : (unsigned)rr0, rm1 = rr1 >> 32 ? 0u : (unsigned)rr1;
    const unsigned rb0 = rr0 >> 32 ? ~0u : 0u, rb1 = rr1 >> 32 ? ~0u : 0u;

    const bool smallP = __all_sync(~0u, (((1ULL << 30) * rr0 + (1ULL << 46)) >> 47) <= 255 && (((1ULL << 30) * rr1 + (1ULL << 46)) >> 47) <= 255);

    if constexpr (MODE != 0) {
    SmemV4& smv = *reinterpret_cast<SmemV4*>(dsm4 + sizeof(Smem4));

    int ohh[8][4], omid[8][4], oll[8][4];
#pragma unroll
    for (int c = 0; c < 8; c++)
#pragma unroll
        for (int i = 0; i < 4; i++) ohh[c][i] = omid[c][i] = oll[c][i] = 0;
    load_k(0, 0);
    load_v(smv, 0, 0);
    asm volatile("cp.async.commit_group;");
    for (int tile = 0; tile < ntiles; tile++) {
        if (tile + 1 < ntiles) { load_k(tile + 1, (tile + 1) & 1); load_v(smv, tile + 1, (tile + 1) & 1); }
        asm volatile("cp.async.commit_group;");
        asm volatile("cp.async.wait_group 1;");
        __syncthreads();
        curb = tile & 1;
        auto body = [&](auto fastc) {
#pragma unroll
            for (int kg = 0; kg < 2; kg++) {
                unsigned w[4][2];
#pragma unroll
                for (int t4 = 0; t4 < 4; t4++) {
                    int nt = kg * 4 + t4, s[4];
                    scores(fastc, nt, s);
                    unsigned P[4];
#pragma unroll
                    for (int i = 0; i < 4; i++) {
                        unsigned E = E_fast(sm.lut4, i < 2 ? mx0 : mx1, s[i]);
                        unsigned x = __umulhi(E, i < 2 ? rm0 : rm1) + (E & (i < 2 ? rb0 : rb1));
                        P[i] = min((x + 16384u) >> 15, 32767u);
                        if (tile >= nfull && inval(tile, nt, i)) P[i] = 0;
                    }
                    w[t4][0] = P[0] | (P[1] << 16);
                    w[t4][1] = P[2] | (P[3] << 16);
                }
                unsigned ph[4], pl[4];
#pragma unroll
                for (int reg = 0; reg < 4; reg++) {
                    unsigned a = w[(reg >> 1) * 2][reg & 1], b = w[(reg >> 1) * 2 + 1][reg & 1];
                    pl[reg] = __byte_perm(a, b, 0x6420);
                    ph[reg] = __byte_perm(a, b, 0x7531);
                }
                if (smallP) {
#pragma unroll
                    for (int ct = 0; ct < 8; ct++) {
                        const int ch = ct * 8 + g;
                        unsigned bh[2], bl[2];
                        bh[0] = *(const unsigned*)(smv.vh[curb] + ch * KS + kg * 32 + q * 4);
                        bh[1] = *(const unsigned*)(smv.vh[curb] + ch * KS + kg * 32 + 16 + q * 4);
                        bl[0] = *(const unsigned*)(smv.vl[curb] + ch * KS + kg * 32 + q * 4);
                        bl[1] = *(const unsigned*)(smv.vl[curb] + ch * KS + kg * 32 + 16 + q * 4);
                        MMA("u8", "s8", omid[ct], pl, bh);
                        MMA("u8", "u8", oll[ct], pl, bl);
                    }
                } else {
#pragma unroll
                for (int ct = 0; ct < 8; ct++) {
                    const int ch = ct * 8 + g;
                    unsigned bh[2], bl[2];
                    bh[0] = *(const unsigned*)(smv.vh[curb] + ch * KS + kg * 32 + q * 4);
                    bh[1] = *(const unsigned*)(smv.vh[curb] + ch * KS + kg * 32 + 16 + q * 4);
                    bl[0] = *(const unsigned*)(smv.vl[curb] + ch * KS + kg * 32 + q * 4);
                    bl[1] = *(const unsigned*)(smv.vl[curb] + ch * KS + kg * 32 + 16 + q * 4);
                    MMA("s8", "s8", ohh[ct], ph, bh);
                    MMA("s8", "u8", omid[ct], ph, bl);
                    MMA("u8", "s8", omid[ct], pl, bh);
                    MMA("u8", "u8", oll[ct], pl, bl);
                }
                }
            }
        };
        if (fast_of(tile, curb)) body(std::true_type{}); else body(std::false_type{});
        __syncthreads();
    }
#pragma unroll
    for (int ct = 0; ct < 8; ct++)
#pragma unroll
        for (int i = 0; i < 4; i++) {
            int row = i < 2 ? r0 : r0 + 8, ch = ct * 8 + q * 2 + (i & 1);
            i64 O = 65536LL * ohh[ct][i] + 256LL * omid[ct][i] + oll[ct][i];
            out[(i64)row * C + h * 64 + ch] = rshr(O, 15 - evv[(eb * H + h) * 64 + ch]);
        }
    }
}

__global__ void prep_vbp(const int* __restrict__ v, int T1, int Tp1, int Bn, int C, int8_t* hi, uint8_t* lo, int* ev) {
    int hc = blockIdx.x, b = blockIdx.y, h = hc / 64, c = hc % 64, tid = threadIdx.x, H = C / 64;
    const int* vb = v + (i64)b * T1 * C;
    i64 m = 0;
    for (int t = tid; t < T1; t += blockDim.x) { i64 a = vb[(i64)t * C + h * 64 + c]; a = a < 0 ? -a : a; if (a > m) m = a; }
    __shared__ i64 sm[256];
    sm[tid] = m;
    __syncthreads();
    for (int o = 128; o > 0; o >>= 1) { if (tid < o && sm[tid + o] > sm[tid]) sm[tid] = sm[tid + o]; __syncthreads(); }
    int ee = exp_of(sm[0]);
    const i64 Tp = (i64)Bn * Tp1;
    for (int t = tid; t < Tp1; t += blockDim.x) {
        int q = t < T1 ? quant16(vb[(i64)t * C + h * 64 + c], ee) : 0;
        const i64 at = (i64)hc * Tp + (i64)b * Tp1 + (t & ~63) + INVPERM[t & 63];
        hi[at] = (int8_t)(q >> 8);
        lo[at] = (uint8_t)(q & 255);
    }
    if (tid == 0) ev[(b * H + h) * 64 + c] = ee;
}

__global__ void prep_vb(const int* __restrict__ v, int T1, int Tp1, int Bn, int C, int8_t* hi, uint8_t* lo, int* ev) {
    int hc = blockIdx.x, b = blockIdx.y, h = hc / 64, c = hc % 64, tid = threadIdx.x, H = C / 64;
    const int* vb = v + (i64)b * T1 * C;
    i64 m = 0;
    for (int t = tid; t < T1; t += blockDim.x) { i64 a = vb[(i64)t * C + h * 64 + c]; a = a < 0 ? -a : a; if (a > m) m = a; }
    __shared__ i64 sm[256];
    sm[tid] = m;
    __syncthreads();
    for (int o = 128; o > 0; o >>= 1) { if (tid < o && sm[tid + o] > sm[tid]) sm[tid] = sm[tid + o]; __syncthreads(); }
    int ee = exp_of(sm[0]);
    const i64 Tp = (i64)Bn * Tp1;
    for (int t = tid; t < Tp1; t += blockDim.x) {
        int q = t < T1 ? quant16(vb[(i64)t * C + h * 64 + c], ee) : 0;
        hi[(i64)hc * Tp + (i64)b * Tp1 + t] = (int8_t)(q >> 8);
        lo[(i64)hc * Tp + (i64)b * Tp1 + t] = (uint8_t)(q & 255);
    }
    if (tid == 0) ev[(b * H + h) * 64 + c] = ee;
}

__global__ void permute_v(const int8_t* hi, const uint8_t* lo, int Tkp, int8_t* hp, uint8_t* lp, int n) {
    int i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= n) return;
    int row = i / Tkp, t = i % Tkp, base = t & ~63, p = t & 63;
    hp[i] = hi[(i64)row * Tkp + base + PERM[p]];
    lp[i] = lo[(i64)row * Tkp + base + PERM[p]];
}

__device__ __forceinline__ int swz64(int row, int byte) { return row * 64 + ((((byte >> 4) ^ ((row >> 1) & 3))) << 4) + (byte & 15); }
struct __align__(16) Smem5 {
    int8_t kh[64 * 64];
    uint8_t kl[64 * 64];
    int8_t vh[64 * 64];
    uint8_t vl[64 * 64];
    int ek[64];
    int lut[1025];
    int allsmall;
};
__global__ void __launch_bounds__(128, 4) attn_pv5(const int8_t* __restrict__ qh, const uint8_t* __restrict__ ql, const int* __restrict__ eqv,
                                                   const int8_t* __restrict__ kh, const uint8_t* __restrict__ kl, const int* __restrict__ ekv,
                                                   const int8_t* __restrict__ vh, const uint8_t* __restrict__ vl, const int* __restrict__ evv,
                                                   const int* __restrict__ lutg, int Tq, int Tk, int Tkp, int C, int* __restrict__ out,
                                                   const int* __restrict__ mxg, const unsigned long long* __restrict__ rrg,
                                                   int Tq1, int Tkp1, int H, int kshared, unsigned* __restrict__ rmaxo) {
    __shared__ Smem5 sm;
    const int bimg = blockIdx.y / H, h = blockIdx.y % H, warp = threadIdx.x >> 5, lane = threadIdx.x & 31, g = lane >> 2, q = lane & 3;
    const int kb = kshared ? 0 : bimg * Tkp1, eb = kshared ? 0 : bimg;
    const int r0 = bimg * Tq1 + blockIdx.x * 64 + warp * 16 + g;
    const int mx0 = mxg[h * Tq + r0], mx1 = mxg[h * Tq + r0 + 8];
    const unsigned long long rr0 = rrg[h * Tq + r0], rr1 = rrg[h * Tq + r0 + 8];
    const bool small = (((1ULL << 30) * rr0 + (1ULL << 46)) >> 47) <= 255 && (((1ULL << 30) * rr1 + (1ULL << 46)) >> 47) <= 255;
    if (threadIdx.x == 0) sm.allsmall = 1;
    __syncthreads();
    if (!small) sm.allsmall = 0;
    for (int i = threadIdx.x; i < 1025; i += 128) sm.lut[i] = lutg[i];
    __syncthreads();
    if (!sm.allsmall) return;
    const unsigned rm0 = rr0 >> 32 ? 0u : (unsigned)rr0, rm1 = rr1 >> 32 ? 0u : (unsigned)rr1;
    const unsigned rb0 = rr0 >> 32 ? ~0u : 0u, rb1 = rr1 >> 32 ? ~0u : 0u;
    unsigned aqh[2][4], aql[2][4];
    {
        const int8_t* Qh = qh + ((i64)h * Tq) * 64;
        const uint8_t* Ql = ql + ((i64)h * Tq) * 64;
#pragma unroll
        for (int ks = 0; ks < 2; ks++) {
            int c0 = ks * 32 + q * 4;
            aqh[ks][0] = *(const unsigned*)(Qh + (i64)r0 * 64 + c0);
            aqh[ks][1] = *(const unsigned*)(Qh + (i64)(r0 + 8) * 64 + c0);
            aqh[ks][2] = *(const unsigned*)(Qh + (i64)r0 * 64 + c0 + 16);
            aqh[ks][3] = *(const unsigned*)(Qh + (i64)(r0 + 8) * 64 + c0 + 16);
            aql[ks][0] = *(const unsigned*)(Ql + (i64)r0 * 64 + c0);
            aql[ks][1] = *(const unsigned*)(Ql + (i64)(r0 + 8) * 64 + c0);
            aql[ks][2] = *(const unsigned*)(Ql + (i64)r0 * 64 + c0 + 16);
            aql[ks][3] = *(const unsigned*)(Ql + (i64)(r0 + 8) * 64 + c0 + 16);
        }
    }
    const int eq0 = eqv[h * Tq + r0], eq1 = eqv[h * Tq + r0 + 8];

    __shared__ int eqmin_s, eqmax_s;
    if (threadIdx.x == 0) { eqmin_s = INT_MAX; eqmax_s = INT_MIN; }
    __syncthreads();
    atomicMin(&eqmin_s, min(eq0, eq1));
    atomicMax(&eqmax_s, max(eq0, eq1));
    __syncthreads();
    const int eqmin = eqmin_s, eqmax = eqmax_s;
    const int ntiles = Tkp1 / 64, nfull = Tk / 64;
    int omid[8][4], oll[8][4];
#pragma unroll
    for (int c = 0; c < 8; c++)
#pragma unroll
        for (int i = 0; i < 4; i++) omid[c][i] = oll[c][i] = 0;
    for (int tile = 0; tile < ntiles; tile++) {
        __syncthreads();
        {
            const int8_t* Kh = kh + ((i64)h * Tkp + kb + tile * 64) * 64;
            const uint8_t* Kl = kl + ((i64)h * Tkp + kb + tile * 64) * 64;
            const int8_t* Vh = vh + (i64)h * 64 * Tkp + kb + tile * 64;
            const uint8_t* Vl = vl + (i64)h * 64 * Tkp + kb + tile * 64;
            for (int i = threadIdx.x; i < 64 * 4; i += 128) {
                int row = i >> 2, w = i & 3, o = row * 64 + ((w ^ ((row >> 1) & 3)) << 4);
                *(uint4*)(sm.kh + o) = *(const uint4*)(Kh + row * 64 + w * 16);
                *(uint4*)(sm.kl + o) = *(const uint4*)(Kl + row * 64 + w * 16);
                *(uint4*)(sm.vh + o) = *(const uint4*)(Vh + (i64)row * Tkp + w * 16);
                *(uint4*)(sm.vl + o) = *(const uint4*)(Vl + (i64)row * Tkp + w * 16);
            }
            if (threadIdx.x < 64) sm.ek[threadIdx.x] = ekv[h * Tkp + kb + tile * 64 + threadIdx.x];
        }
        __syncthreads();
        bool fast = true;
#pragma unroll
        for (int j = 0; j < 2; j++) {
            int key = lane + 32 * j, tkv = 19 - sm.ek[key];
            fast = fast && (tile * 64 + key >= Tk || (tkv - eqmax >= 6 && tkv - eqmin <= 16));
        }
        fast = __all_sync(~0u, fast);
        auto body = [&](auto fastc) {
            constexpr bool FAST = decltype(fastc)::value;
#pragma unroll
            for (int kg = 0; kg < 2; kg++) {
                unsigned w[4][2];
#pragma unroll
                for (int t4 = 0; t4 < 4; t4++) {
                    const int nt = kg * 4 + t4, n = nt * 8 + g;
                    int chh[4] = {0, 0, 0, 0}, cmid[4] = {0, 0, 0, 0}, cll[4] = {0, 0, 0, 0};
#pragma unroll
                    for (int ks = 0; ks < 2; ks++) {
                        unsigned bh[2], bl[2];
                        bh[0] = *(const unsigned*)(sm.kh + swz64(n, ks * 32 + q * 4));
                        bh[1] = *(const unsigned*)(sm.kh + swz64(n, ks * 32 + 16 + q * 4));
                        bl[0] = *(const unsigned*)(sm.kl + swz64(n, ks * 32 + q * 4));
                        bl[1] = *(const unsigned*)(sm.kl + swz64(n, ks * 32 + 16 + q * 4));
                        MMA("s8", "s8", chh, aqh[ks], bh);
                        MMA("s8", "u8", cmid, aqh[ks], bl);
                        MMA("u8", "s8", cmid, aql[ks], bh);
                        MMA("u8", "u8", cll, aql[ks], bl);
                    }
                    const int2 ek2 = *(const int2*)(sm.ek + nt * 8 + q * 2);
                    unsigned P[4];
#pragma unroll
                    for (int i = 0; i < 4; i++) {
                        const int t = 19 - ((i & 1) ? ek2.y : ek2.x) - (i < 2 ? eq0 : eq1), R = cmid[i] * 256 + cll[i];
                        const int s = FAST ? s_fast(chh[i], R, t) : s_of(chh[i], R, t);
                        const unsigned z = (unsigned)(i < 2 ? mx0 : mx1) - (unsigned)s, zc = min(z, (1u << 20) - 1);
                        const int ix = zc >> 10;
                        const unsigned L0 = (unsigned)sm.lut[ix], nd = L0 - (unsigned)sm.lut[ix + 1], f = zc & 1023;
                        unsigned E = L0 - (unsigned)(((unsigned long long)nd * f + 1023) >> 10);
                        E = z >= (1u << 20) ? 0u : E;
                        const unsigned x = __umulhi(E, i < 2 ? rm0 : rm1) + (E & (i < 2 ? rb0 : rb1));
                        P[i] = min((x + 16384u) >> 15, 32767u);
                        if (tile >= nfull && tile * 64 + nt * 8 + q * 2 + (i & 1) >= Tk) P[i] = 0;
                    }
                    w[t4][0] = P[0] | (P[1] << 16);
                    w[t4][1] = P[2] | (P[3] << 16);
                }
                unsigned pl[4];
#pragma unroll
                for (int reg = 0; reg < 4; reg++) pl[reg] = __byte_perm(w[(reg >> 1) * 2][reg & 1], w[(reg >> 1) * 2 + 1][reg & 1], 0x6420);
#pragma unroll
                for (int ct = 0; ct < 8; ct++) {
                    const int ch = ct * 8 + g;
                    unsigned bh[2], bl[2];
                    bh[0] = *(const unsigned*)(sm.vh + swz64(ch, kg * 32 + q * 4));
                    bh[1] = *(const unsigned*)(sm.vh + swz64(ch, kg * 32 + 16 + q * 4));
                    bl[0] = *(const unsigned*)(sm.vl + swz64(ch, kg * 32 + q * 4));
                    bl[1] = *(const unsigned*)(sm.vl + swz64(ch, kg * 32 + 16 + q * 4));
                    MMA("u8", "s8", omid[ct], pl, bh);
                    MMA("u8", "u8", oll[ct], pl, bl);
                }
            }
        };
        if (fast) body(std::true_type{}); else body(std::false_type{});
    }
    unsigned am0 = 0, am1 = 0;
#pragma unroll
    for (int ct = 0; ct < 8; ct++)
#pragma unroll
        for (int i = 0; i < 4; i++) {
            int row = i < 2 ? r0 : r0 + 8, ch = ct * 8 + q * 2 + (i & 1);
            i64 O = 256LL * omid[ct][i] + oll[ct][i];
            const int ov = rshr(O, 15 - evv[(eb * H + h) * 64 + ch]);
            out[(i64)row * C + h * 64 + ch] = ov;
            const unsigned a = ov < 0 ? 0u - (unsigned)ov : (unsigned)ov;
            if (i < 2) am0 = max(am0, a); else am1 = max(am1, a);
        }
    if (rmaxo) {
        am0 = max(am0, __shfl_xor_sync(~0u, am0, 1)); am0 = max(am0, __shfl_xor_sync(~0u, am0, 2));
        am1 = max(am1, __shfl_xor_sync(~0u, am1, 1)); am1 = max(am1, __shfl_xor_sync(~0u, am1, 2));
        if (q == 0) { atomicMax(&rmaxo[r0], am0); atomicMax(&rmaxo[r0 + 8], am1); }
    }
}

