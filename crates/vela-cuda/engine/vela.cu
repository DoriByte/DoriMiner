// DoriMiner - vela.cu
// Copyright (C) 2026 J. Costa (DoriByte). Licensed under the PolyForm Noncommercial License 1.0.0 with the additional
// permission in MINING-PERMISSION (see LICENSE and MINING-PERMISSION at the top of this tree).

#include "attn_kernels.cuh"
#include <map>
#include <string>
#include <cstring>
#include <cmath>
#include <chrono>
#include <unordered_map>
#include <thread>
#include <atomic>

static int envnum(const char* s) {
    if (!s) return 0;
    int sign = 1, v = 0;
    if (*s == '-') { sign = -1; s++; }
    while (*s >= '0' && *s <= '9') v = v * 10 + (*s++ - '0');
    return sign * v;
}
#include <algorithm>

#define getenv(name) ((const char *)nullptr)

__device__ __forceinline__ i64 floordiv(i64 a, i64 b) {
    i64 q = a / b;
    if ((a % b) != 0 && a < 0) q--;
    return q;
}

__device__ __forceinline__ int rshr64(i64 v, int s) {
    if (s > 0) {
        int pos = s > 62 ? 62 : s;
        return sat32((v + (1LL << (pos - 1))) >> pos);
    }
    if (s == 0) return sat32(v);
    int n = -s > 31 ? 31 : -s;
    i64 lim = 1LL << (62 - n);
    v = v > lim ? lim : (v < -lim ? -lim : v);
    return sat32(v << n);
}

__device__ __forceinline__ i64 floordiv_m(i64 a, unsigned long long dv, unsigned long long minv) {
    const unsigned long long n = a < 0 ? 0ull - (unsigned long long)a : (unsigned long long)a;
    unsigned long long q = __umul64hi(n, minv), r = n - q * dv;
    if (r >= dv) { q++; r -= dv; }
    if (r >= dv) { q++; r -= dv; }
    return a < 0 ? -(i64)(q + (r != 0)) : (i64)q;
}

__device__ __forceinline__ i64 isqrt64(i64 v) {
    i64 s = (i64)floor(sqrt((double)v));
    while (s * s > v) s--;
    while ((s + 1) * (s + 1) <= v) s++;
    return s;
}

__device__ __forceinline__ int q16f(int v, int e) {
    int r = e ? (v >> e) + ((v >> (e - 1)) & 1) : v;
    return max(-32767, min(32767, r));
}

__device__ __forceinline__ i64 lut_q16(const int* L, int x) {
    i64 z = (i64)x + (16LL << 16);
    if (z < 0) return 0;
    if (z >= (32LL << 16)) return 65536;
    int i = (int)(z >> 10), f = (int)(z & 1023);
    i64 a = L[i], b = L[i + 1];
    return a + (((b - a) * f) >> 10);
}

__device__ __forceinline__ int gelu_f(const int* __restrict__ L, int g) {
    int zi = g + (16 << 16);
    int ph;
    if (g < -(16 << 16)) ph = 0;
    else if (zi >= (32 << 16)) ph = 65536;
    else { int i = zi >> 10, f = zi & 1023, a = __ldg(L + i), b = __ldg(L + i + 1); ph = a + (((b - a) * f) >> 10); }
    i64 v = (i64)g * ph;
    return sat32((v + (1LL << 15)) >> 16);
}

__device__ __forceinline__ int gelu_s(const int2* __restrict__ T, int g) {
    int zi = g + (16 << 16);
    int ph;
    if (g < -(16 << 16)) ph = 0;
    else if (zi >= (32 << 16)) ph = 65536;
    else { int2 e = T[zi >> 10]; ph = e.x + ((e.y * (zi & 1023)) >> 10); }
    i64 v = (i64)g * ph;
    return sat32((v + (1LL << 15)) >> 16);
}

__device__ __forceinline__ i64 block_max(i64 v, i64* sh) {
    for (int o = 16; o > 0; o >>= 1) { i64 w = __shfl_xor_sync(~0u, v, o); v = w > v ? w : v; }
    int wid = threadIdx.x >> 5, nw = blockDim.x >> 5;
    __syncthreads();
    if ((threadIdx.x & 31) == 0) sh[wid] = v;
    __syncthreads();
    i64 r = sh[0];
    for (int i = 1; i < nw; i++) r = sh[i] > r ? sh[i] : r;
    return r;
}
__device__ __forceinline__ i64 block_sum(i64 v, i64* sh) {
    for (int o = 16; o > 0; o >>= 1) v += __shfl_xor_sync(~0u, v, o);
    int wid = threadIdx.x >> 5, nw = blockDim.x >> 5;
    __syncthreads();
    if ((threadIdx.x & 31) == 0) sh[wid] = v;
    __syncthreads();
    i64 r = 0;
    for (int i = 0; i < nw; i++) r += sh[i];
    return r;
}

__global__ void quant_rows(const int* __restrict__ x, int M, int K, int ldx, int Kp, int8_t* hi, uint8_t* lo, int* e) {
    __shared__ i64 sh[32];
    int r = blockIdx.x;
    const int* xr = x + (i64)r * ldx;
    i64 m = 0;
    for (int k = threadIdx.x; k < K; k += blockDim.x) { i64 a = xr[k]; a = a < 0 ? -a : a; m = a > m ? a : m; }
    int ee = exp_of(block_max(m, sh));
    for (int k = threadIdx.x; k < Kp; k += blockDim.x) {
        int q = k < K ? quant16(xr[k], ee) : 0;
        hi[(i64)r * Kp + k] = (int8_t)(q >> 8);
        lo[(i64)r * Kp + k] = (uint8_t)(q & 255);
    }
    if (threadIdx.x == 0) e[r] = ee;
}

__global__ void quant_rows_m(const int* __restrict__ x, int K, int Kp, const unsigned* __restrict__ rmax, int8_t* hi, uint8_t* lo, int* e) {
    const int r = blockIdx.x, ee = exp_of((i64)rmax[r]);
    const int* xr = x + (i64)r * K;
    for (int k4 = threadIdx.x; k4 * 4 < Kp; k4 += blockDim.x) {
        int k = k4 * 4;
        int4 v = k < K ? *(const int4*)(xr + k) : make_int4(0, 0, 0, 0);
        int q0 = q16f(v.x, ee), q1 = q16f(v.y, ee), q2 = q16f(v.z, ee), q3 = q16f(v.w, ee);
        unsigned w01 = (q0 & 0xffff) | ((unsigned)q1 << 16), w23 = (q2 & 0xffff) | ((unsigned)q3 << 16);
        *(unsigned*)(hi + (i64)r * Kp + k) = __byte_perm(w01, w23, 0x7531);
        *(unsigned*)(lo + (i64)r * Kp + k) = __byte_perm(w01, w23, 0x6420);
    }
    if (threadIdx.x == 0) e[r] = ee;
}

__global__ void pix_absmax(const int* __restrict__ x, int P, int C, unsigned* out) {
    __shared__ i64 sh[32];
    int p = blockIdx.x;
    i64 m = 0;
    for (int c = threadIdx.x; c < C; c += blockDim.x) { i64 a = x[(i64)p * C + c]; a = a < 0 ? -a : a; m = a > m ? a : m; }
    m = block_max(m, sh);
    if (threadIdx.x == 0) out[p] = (unsigned)m;
}

__global__ void im2col3(const int* __restrict__ x, const unsigned* __restrict__ pmax, int Hi, int Wi, int C, int up, int stride,
                        int Ho, int Wo, int Kp, int8_t* hi, uint8_t* lo, int* e) {
    __shared__ i64 sh[32];
    const int Po = Ho * Wo, b = blockIdx.x / Po, p = blockIdx.x % Po, oy = p / Wo, ox = p % Wo;
    const i64 ibase = (i64)b * Hi * Wi;
    int Hu = Hi * up, Wu = Wi * up;

    __shared__ int src[9];
    if (threadIdx.x < 9) {
        int ky = threadIdx.x / 3, kx = threadIdx.x % 3;
        int uy = oy * stride + ky - 1, ux = ox * stride + kx - 1;
        src[threadIdx.x] = (uy < 0 || ux < 0 || uy >= Hu || ux >= Wu) ? -1 : (int)(ibase + (uy / up) * Wi + (ux / up));
    }
    __syncthreads();
    i64 m = 0;
    if (threadIdx.x < 9 && src[threadIdx.x] >= 0) m = pmax[src[threadIdx.x]];
    int ee = exp_of(block_max(m, sh));
    for (int k = threadIdx.x; k < Kp; k += blockDim.x) {
        int q = 0;
        if (k < 9 * C) {
            int tap = k / C, c = k % C, s = src[tap];
            if (s >= 0) q = quant16(x[(i64)s * C + c], ee);
        }
        hi[(i64)blockIdx.x * Kp + k] = (int8_t)(q >> 8);
        lo[(i64)blockIdx.x * Kp + k] = (uint8_t)(q & 255);
    }
    if (threadIdx.x == 0) e[blockIdx.x] = ee;
}

constexpr int GS = 80;
__global__ void __launch_bounds__(256) gemm_q(const int8_t* __restrict__ Ah, const uint8_t* __restrict__ Al, const int* __restrict__ ea,
                                              const int8_t* __restrict__ W, const int* __restrict__ mm, const int* __restrict__ ss,
                                              const int* __restrict__ bb, int M, int N, int Kp, int* __restrict__ out, int ldo) {
    extern __shared__ __align__(16) unsigned char smem[];
    int8_t* sAh = (int8_t*)smem;
    uint8_t* sAl = (uint8_t*)(smem + 2 * 128 * GS);
    int8_t* sW = (int8_t*)(smem + 4 * 128 * GS);
    const int m0 = blockIdx.x * 128, n0 = blockIdx.y * 64;
    const int warp = threadIdx.x >> 5, lane = threadIdx.x & 31, g = lane >> 2, q = lane & 3;
    const int wm = (warp & 3) * 32, wn = (warp >> 2) * 32;
    int chh[2][4][4], cll[2][4][4];
    for (int a = 0; a < 2; a++) for (int b = 0; b < 4; b++) for (int i = 0; i < 4; i++) chh[a][b][i] = cll[a][b][i] = 0;
    auto load = [&](int stage, int k0) {
        for (int c = threadIdx.x; c < 128 * 4; c += 256) {
            int r = c >> 2, w = (c & 3) * 16, gm = m0 + r;
            bool ok = gm < M;
            const int8_t* srch = Ah + (i64)(ok ? gm : 0) * Kp + k0 + w;
            const uint8_t* srcl = Al + (i64)(ok ? gm : 0) * Kp + k0 + w;
            cp16(sAh + (stage * 128 + r) * GS + w, srch, ok);
            cp16(sAl + (stage * 128 + r) * GS + w, srcl, ok);
        }
        for (int c = threadIdx.x; c < 64 * 4; c += 256) {
            int r = c >> 2, w = (c & 3) * 16;
            cp16(sW + (stage * 64 + r) * GS + w, W + (i64)(n0 + r) * Kp + k0 + w, true);
        }
        asm volatile("cp.async.commit_group;");
    };
    const int nk = Kp / 64;
    load(0, 0);
    for (int kt = 0; kt < nk; kt++) {
        if (kt + 1 < nk) { load((kt + 1) & 1, (kt + 1) * 64); asm volatile("cp.async.wait_group 1;"); }
        else asm volatile("cp.async.wait_group 0;");
        __syncthreads();
        const int st = kt & 1;
#pragma unroll
        for (int ks = 0; ks < 2; ks++) {
            unsigned ah[2][4], al[2][4], bw[4][2];
#pragma unroll
            for (int mt = 0; mt < 2; mt++) {
                const int r = st * 128 + wm + mt * 16 + g, c = ks * 32 + q * 4;
                ah[mt][0] = *(const unsigned*)(sAh + r * GS + c);
                ah[mt][1] = *(const unsigned*)(sAh + (r + 8) * GS + c);
                ah[mt][2] = *(const unsigned*)(sAh + r * GS + c + 16);
                ah[mt][3] = *(const unsigned*)(sAh + (r + 8) * GS + c + 16);
                al[mt][0] = *(const unsigned*)(sAl + r * GS + c);
                al[mt][1] = *(const unsigned*)(sAl + (r + 8) * GS + c);
                al[mt][2] = *(const unsigned*)(sAl + r * GS + c + 16);
                al[mt][3] = *(const unsigned*)(sAl + (r + 8) * GS + c + 16);
            }
#pragma unroll
            for (int nt = 0; nt < 4; nt++) {
                const int r = st * 64 + wn + nt * 8 + g, c = ks * 32 + q * 4;
                bw[nt][0] = *(const unsigned*)(sW + r * GS + c);
                bw[nt][1] = *(const unsigned*)(sW + r * GS + c + 16);
            }
#pragma unroll
            for (int mt = 0; mt < 2; mt++)
#pragma unroll
                for (int nt = 0; nt < 4; nt++) {
                    MMA("s8", "s8", chh[mt][nt], ah[mt], bw[nt]);
                    MMA("u8", "s8", cll[mt][nt], al[mt], bw[nt]);
                }
        }
        __syncthreads();
    }
#pragma unroll
    for (int mt = 0; mt < 2; mt++)
#pragma unroll
        for (int nt = 0; nt < 4; nt++)
#pragma unroll
            for (int i = 0; i < 4; i++) {
                int row = m0 + wm + mt * 16 + g + (i >> 1) * 8, col = n0 + wn + nt * 8 + q * 2 + (i & 1);
                if (row >= M || col >= N) continue;
                i64 acc = 256LL * chh[mt][nt][i] + cll[mt][nt][i];
                int y = rshr64(acc * mm[col], ss[col] - ea[row]);
                out[(i64)row * ldo + col] = sat32((i64)y + bb[col]);
            }
}

constexpr int G1M = 128, G1N = 128, G1S = 3;
constexpr int G1STAGE = (2 * G1M + G1N) * 64;
__device__ __forceinline__ int swz(int row, int chunk) { return row * 64 + ((chunk ^ ((row >> 1) & 3)) << 4); }
__device__ __forceinline__ void ldsm4(unsigned r[4], const void* p) {
    unsigned a = (unsigned)__cvta_generic_to_shared(p);
    asm volatile("ldmatrix.sync.aligned.m8n8.x4.shared.b16 {%0,%1,%2,%3}, [%4];" : "=r"(r[0]), "=r"(r[1]), "=r"(r[2]), "=r"(r[3]) : "r"(a));
}

__device__ __forceinline__ int layer_val(int hh, int ll, int m256, int m, int sh, int b) {
    sh = sh < 1 ? 1 : (sh > 62 ? 62 : sh);
    const i64 x = (i64)hh * m256 + (i64)ll * m;
    const i64 v = ((x >> (sh - 1)) + 1) >> 1;
    int r, y = v > INT_MAX ? INT_MAX : (v < INT_MIN ? INT_MIN : (int)v);
    asm("add.sat.s32 %0, %1, %2;" : "=r"(r) : "r"(y), "r"(b));
    return r;
}

template <int EMODE, int MT = 4>
__device__ __forceinline__ void gemm_epilogue(int (&chh)[MT][4][4], int (&cll)[MT][4][4], int m0, int n0, int wm, int wn, int lane,
                                              int M, int N, const int* __restrict__ ea, const int* __restrict__ mm, const int* __restrict__ ss,
                                              const int* __restrict__ bb, int* __restrict__ out, int ldo, const int* __restrict__ res,
                                              const int* __restrict__ lphi, unsigned* __restrict__ rmax = nullptr, const int2* __restrict__ phis = nullptr,
                                              unsigned* __restrict__ cmax = nullptr, int p1 = 1) {
    unsigned am[MT][2] = {};
    unsigned cm[4][2] = {};
    const int g = lane >> 2, q = lane & 3;

    int er[MT][2];
#pragma unroll
    for (int mt = 0; mt < MT; mt++)
#pragma unroll
        for (int hh = 0; hh < 2; hh++) { int row = m0 + wm + mt * 16 + g + hh * 8; er[mt][hh] = row < M ? ea[row] : 0; }
#pragma unroll
    for (int nt = 0; nt < 4; nt++) {
        const int col = n0 + wn + nt * 8 + q * 2;
        if (col >= N) continue;
        const bool two = col + 1 < N;
        const int mc0 = mm[col], mc1 = two ? mm[col + 1] : 0, m2560 = mc0 * 256, m2561 = mc1 * 256;
        const int sc0 = ss[col], sc1 = two ? ss[col + 1] : 0, bc0 = bb[col], bc1 = two ? bb[col + 1] : 0;
#pragma unroll
        for (int mt = 0; mt < MT; mt++)
#pragma unroll
            for (int hh = 0; hh < 2; hh++) {
                int row = m0 + wm + mt * 16 + g + hh * 8;
                if (row >= M) continue;
                int o[2];
#pragma unroll
                for (int j = 0; j < 2; j++) {
                    int y = layer_val(chh[mt][nt][hh * 2 + j], cll[mt][nt][hh * 2 + j], j ? m2561 : m2560, j ? mc1 : mc0,
                                      (j ? sc1 : sc0) - er[mt][hh], j ? bc1 : bc0);
                    if constexpr (EMODE == 1) y = sat32((i64)y + res[(i64)row * ldo + col + j]);
                    else if constexpr (EMODE == 2) {
                        int ge = sat32((i64)rshr64((i64)y * lut_q16(lphi, y), 16));
                        y = sat32((i64)rshr64((i64)res[(i64)row * ldo + col + j] * ge, 16));
                    }
                    o[j] = y;
                }
                if constexpr (EMODE == 3) {
                    i64 v = (i64)o[0] * (phis ? gelu_s(phis, o[1]) : gelu_f(lphi, o[1]));
                    const int agv = sat32((v + (1LL << 15)) >> 16);
                    out[(i64)row * ldo + (col >> 1)] = agv;
                    const unsigned a = agv < 0 ? 0u - (unsigned)agv : (unsigned)agv;
                    am[mt][hh] = a > am[mt][hh] ? a : am[mt][hh];
                    continue;
                }
                {
                    const unsigned a0 = o[0] < 0 ? 0u - (unsigned)o[0] : (unsigned)o[0], a1 = two ? (o[1] < 0 ? 0u - (unsigned)o[1] : (unsigned)o[1]) : 0u;
                    am[mt][hh] = max(am[mt][hh], max(a0, a1));
                    cm[nt][0] = max(cm[nt][0], a0); cm[nt][1] = max(cm[nt][1], a1);
                }
                int* dst = out + (i64)row * ldo + col;
                if (two && ((((size_t)dst) & 7) == 0)) *(int2*)dst = make_int2(o[0], o[1]);
                else { dst[0] = o[0]; if (two) dst[1] = o[1]; }
            }
    }
    if (cmax) {
        const int img = m0 / p1;
#pragma unroll
        for (int nt = 0; nt < 4; nt++)
#pragma unroll
            for (int j = 0; j < 2; j++) {
                unsigned a = cm[nt][j];
                a = max(a, __shfl_xor_sync(~0u, a, 4)); a = max(a, __shfl_xor_sync(~0u, a, 8)); a = max(a, __shfl_xor_sync(~0u, a, 16));
                const int col = n0 + wn + nt * 8 + (lane & 3) * 2 + j;
                if ((lane >> 2) == 0 && col < N) atomicMax(&cmax[(i64)img * N + col], a);
            }
    }
    if (rmax) {
        const int g = lane >> 2;
#pragma unroll
        for (int mt = 0; mt < MT; mt++)
#pragma unroll
            for (int hh = 0; hh < 2; hh++) {
                unsigned a = am[mt][hh];
                a = max(a, __shfl_xor_sync(~0u, a, 1));
                a = max(a, __shfl_xor_sync(~0u, a, 2));
                const int row = m0 + wm + mt * 16 + g + hh * 8;
                if ((lane & 3) == 0 && row < M) atomicMax(&rmax[row], a);
            }
    }
}

struct HQ { int8_t* hi; uint8_t* lo; int* e; int T; };

#define G1MB2K 2560
#define G1MINB 1
template <int EMODE, int BN = 128, int BM = 128, int WTM = 64, int G1S = ::G1S, int MINB = G1MINB>
__global__ void __launch_bounds__(BM * BN / (WTM * 32) * 32, (BM * BN / (WTM * 32) * 32 <= 256) ? MINB : 1) gemm_q1(const int8_t* __restrict__ Ah, const uint8_t* __restrict__ Al, const int* __restrict__ ea,
                                               const int8_t* __restrict__ W, const int* __restrict__ mm, const int* __restrict__ ss,
                                               const int* __restrict__ bb, int M, int N, int Np, int Kp, int* __restrict__ out, int ldo,
                                               const int* __restrict__ res, const int* __restrict__ lphi, HQ hq = HQ{}, unsigned* __restrict__ rmaxp = nullptr,
                                               unsigned* __restrict__ cmaxp = nullptr, int p1 = 1) {
    extern __shared__ __align__(128) unsigned char smem[];
    constexpr int G1M = BM, MT = WTM / 16, WMW = BM / WTM, NT = WMW * (BN / 32) * 32, STAGE = (2 * G1M + BN) * 64;
    const int n0 = blockIdx.x * BN, m0 = blockIdx.y * G1M;
    const int warp = threadIdx.x >> 5, lane = threadIdx.x & 31;
    const int wm = (warp % WMW) * WTM, wn = (warp / WMW) * 32;
    int chh[MT][4][4], cll[MT][4][4];
#pragma unroll
    for (int a = 0; a < MT; a++)
#pragma unroll
        for (int b = 0; b < 4; b++)
#pragma unroll
            for (int i = 0; i < 4; i++) chh[a][b][i] = cll[a][b][i] = 0;

    constexpr int AIT = (G1M * 4 + NT - 1) / NT, WIT = (BN * 4 + NT - 1) / NT;
    const int8_t* pah[AIT]; const uint8_t* pal[AIT]; int oa[AIT]; bool va[AIT], ia[AIT];
    const int8_t* pw[WIT]; int ow[WIT]; bool vw[WIT], iw[WIT];
#pragma unroll
    for (int it = 0; it < AIT; it++) {
        const int c = threadIdx.x + it * NT, r = c >> 2, ch = c & 3, gm = m0 + r;
        ia[it] = c < G1M * 4;
        va[it] = gm < M;
        pah[it] = Ah + (i64)(va[it] ? gm : 0) * Kp + ch * 16;
        pal[it] = Al + (i64)(va[it] ? gm : 0) * Kp + ch * 16;
        oa[it] = swz(r, ch);
    }
#pragma unroll
    for (int it = 0; it < WIT; it++) {
        const int c = threadIdx.x + it * NT, r = c >> 2, ch = c & 3, gn = n0 + r;
        iw[it] = c < BN * 4;
        vw[it] = gn < Np;
        pw[it] = W + (i64)(vw[it] ? gn : 0) * Kp + ch * 16;
        ow[it] = 2 * G1M * 64 + swz(r, ch);
    }
    auto load = [&](int stage, int k0) {
        unsigned char* base = smem + stage * STAGE;
#pragma unroll
        for (int it = 0; it < AIT; it++) {
            if ((G1M * 4) % NT && !ia[it]) break;
            cp16(base + oa[it], pah[it] + k0, va[it]);
            cp16(base + G1M * 64 + oa[it], pal[it] + k0, va[it]);
        }
#pragma unroll
        for (int it = 0; it < WIT; it++) {
            if ((BN * 4) % NT && !iw[it]) break;
            cp16(base + ow[it], pw[it] + k0, vw[it]);
        }
    };
    const int nk = Kp / 64;
#pragma unroll
    for (int st = 0; st < G1S - 1; st++) {
        if (st < nk) load(st, st * 64);
        asm volatile("cp.async.commit_group;");
    }
    asm volatile("cp.async.wait_group %0;" ::"n"(G1S - 2));
    __syncthreads();

    unsigned ah[2][4], al[2][4], bw[2][4][2];

    const int fa = ((lane & 15) >> 1) & 3, fb = ((lane & 7) >> 1) & 3;
    const int abase = (wm + (lane & 15)) * 64, ax0 = ((0 + (lane >> 4)) ^ fa) << 4, ax1 = ((2 + (lane >> 4)) ^ fa) << 4;
    const int bbase = 2 * G1M * 64 + (wn + (lane & 7) + ((lane >> 4) << 3)) * 64, bx0 = ((0 + ((lane >> 3) & 1)) ^ fb) << 4,
              bx1 = ((2 + ((lane >> 3) & 1)) ^ fb) << 4;
    auto ldb = [&](const unsigned char* base, int ks, unsigned (&d)[4][2]) {
#pragma unroll
        for (int np = 0; np < 2; np++) {
            unsigned t4[4];
            ldsm4(t4, base + bbase + np * 1024 + (ks ? bx1 : bx0));
            d[np * 2][0] = t4[0]; d[np * 2][1] = t4[1]; d[np * 2 + 1][0] = t4[2]; d[np * 2 + 1][1] = t4[3];
        }
    };
    auto lda = [&](const unsigned char* base, int ks, int mt, unsigned (&dh)[4], unsigned (&dl)[4]) {
        const unsigned char* pa = base + abase + mt * 1024 + (ks ? ax1 : ax0);
        ldsm4(dh, pa);
        ldsm4(dl, pa + G1M * 64);
    };
    const unsigned char* base = smem;
    ldb(base, 0, bw[0]);
    lda(base, 0, 0, ah[0], al[0]);
    for (int kt = 0; kt < nk; kt++) {
#pragma unroll
        for (int j = 0; j < 2 * MT; j++) {
            const int ks = j / MT, mt = j % MT, cur = j & 1;
            if (j == MT / 2) ldb(base, 1, bw[1]);
            if (j < 2 * MT - 1) lda(base, (j + 1) / MT, (j + 1) % MT, ah[cur ^ 1], al[cur ^ 1]);
            else {
                if (kt + G1S - 1 < nk) load((kt + G1S - 1) % G1S, (kt + G1S - 1) * 64);
                asm volatile("cp.async.commit_group;");
                asm volatile("cp.async.wait_group %0;" ::"n"(G1S - 2));
                __syncthreads();
                base = smem + ((kt + 1) % G1S) * STAGE;
                if (kt + 1 < nk) { ldb(base, 0, bw[0]); lda(base, 0, 0, ah[0], al[0]); }
            }
#pragma unroll
            for (int nt = 0; nt < 4; nt++) {
                MMA("s8", "s8", chh[mt][nt], ah[cur], bw[ks][nt]);
                MMA("u8", "s8", cll[mt][nt], al[cur], bw[ks][nt]);
            }
        }
    }
    asm volatile("cp.async.wait_group 0;");
    if constexpr (EMODE == 4) {

        asm volatile("cp.async.wait_group 0;");
        __syncthreads();
        constexpr int WN = BN / 32;
        unsigned* red = (unsigned*)smem;
        const int g = lane >> 2, q = lane & 3;
        int er[MT][2];
#pragma unroll
        for (int mt = 0; mt < MT; mt++)
#pragma unroll
            for (int hh = 0; hh < 2; hh++) { int row = m0 + wm + mt * 16 + g + hh * 8; er[mt][hh] = row < M ? ea[row] : 0; }
        unsigned am[MT][2] = {};
#pragma unroll
        for (int nt = 0; nt < 4; nt++) {
            const int col = n0 + wn + nt * 8 + q * 2;
            const bool ok = col < N;
            const int mc0 = ok ? mm[col] : 0, mc1 = ok ? mm[col + 1] : 0, m2560 = mc0 * 256, m2561 = mc1 * 256;
            const int sc0 = ok ? ss[col] : 2, sc1 = ok ? ss[col + 1] : 2, bc0 = ok ? bb[col] : 0, bc1 = ok ? bb[col + 1] : 0;
#pragma unroll
            for (int mt = 0; mt < MT; mt++)
#pragma unroll
                for (int hh = 0; hh < 2; hh++)
#pragma unroll
                    for (int j = 0; j < 2; j++) {
                        int y = ok ? layer_val(chh[mt][nt][hh * 2 + j], cll[mt][nt][hh * 2 + j], j ? m2561 : m2560, j ? mc1 : mc0,
                                               (j ? sc1 : sc0) - er[mt][hh], j ? bc1 : bc0) : 0;
                        chh[mt][nt][hh * 2 + j] = y;
                        unsigned a = y < 0 ? 0u - (unsigned)y : (unsigned)y;
                        am[mt][hh] = a > am[mt][hh] ? a : am[mt][hh];
                    }
        }
#pragma unroll
        for (int mt = 0; mt < MT; mt++)
#pragma unroll
            for (int hh = 0; hh < 2; hh++) {
                unsigned a = am[mt][hh];
                a = max(a, __shfl_xor_sync(~0u, a, 1));
                a = max(a, __shfl_xor_sync(~0u, a, 2));
                if (q == 0) red[(wm + mt * 16 + g + hh * 8) * WN + (wn >> 5)] = a;
            }
        __syncthreads();
        const int head = (n0 + wn) >> 6, wpair = (wn >> 5) & ~1;
#pragma unroll
        for (int mt = 0; mt < MT; mt++)
#pragma unroll
            for (int hh = 0; hh < 2; hh++) {
                const int rl = wm + mt * 16 + g + hh * 8, row = m0 + rl;
                if (row >= M || (n0 + wn) >= N) continue;
                const unsigned hm = max(red[rl * WN + wpair], red[rl * WN + wpair + 1]);
                const int e = exp_of((i64)hm);
                const i64 base = ((i64)head * hq.T + row) * 64;
#pragma unroll
                for (int nt = 0; nt < 4; nt++) {
                    const int c = ((wn + nt * 8) & 63) + q * 2;
                    const int q0 = q16f(chh[mt][nt][hh * 2], e), q1 = q16f(chh[mt][nt][hh * 2 + 1], e);
                    *(unsigned short*)(hq.hi + base + c) = (unsigned short)(((q0 >> 8) & 255) | ((q1 >> 8) << 8));
                    *(unsigned short*)(hq.lo + base + c) = (unsigned short)((q0 & 255) | ((q1 & 255) << 8));
                }
                if (((wn & 63) == 0) && q == 0) hq.e[(i64)head * hq.T + row] = e;
            }
        return;
    }
#define GELU_SMEM 0
    if constexpr (EMODE == 3 && GELU_SMEM) {
        asm volatile("cp.async.wait_group 0;");
        __syncthreads();
        int2* T = (int2*)smem;
        for (int i = threadIdx.x; i < 2048; i += NT) { int a = __ldg(lphi + i), b2 = __ldg(lphi + i + 1); T[i] = make_int2(a, b2 - a); }
        __syncthreads();
        gemm_epilogue<EMODE, MT>(chh, cll, m0, n0, wm, wn, lane, M, N, ea, mm, ss, bb, out, ldo, res, lphi, rmaxp, T);
        return;
    }
    gemm_epilogue<EMODE, MT>(chh, cll, m0, n0, wm, wn, lane, M, N, ea, mm, ss, bb, out, ldo, res, lphi, rmaxp, nullptr, cmaxp, p1);
}

__global__ void conv_e(const unsigned* __restrict__ pmax, int Bn, int Hi, int Wi, int up, int stride, int Ho, int Wo, int* __restrict__ e) {
    int m = blockIdx.x * blockDim.x + threadIdx.x, Po = Ho * Wo;
    if (m >= Bn * Po) return;
    int b = m / Po, p = m % Po, oy = p / Wo, ox = p % Wo, Hu = Hi * up, Wu = Wi * up;
    i64 mx = 0;
    for (int t = 0; t < 9; t++) {
        int uy = oy * stride + t / 3 - 1, ux = ox * stride + t % 3 - 1;
        if (uy < 0 || ux < 0 || uy >= Hu || ux >= Wu) continue;
        i64 v = pmax[(i64)b * Hi * Wi + (uy / up) * Wi + ux / up];
        mx = v > mx ? v : mx;
    }
    e[m] = exp_of(mx);
}

__global__ void __launch_bounds__(128) im2col4(const int* __restrict__ x, const int* __restrict__ ea, int Hi, int Wi, int C, int up, int stride,
                                               int Ho, int Wo, int Kp, int8_t* __restrict__ hi, uint8_t* __restrict__ lo) {
    const int m = blockIdx.x, Po = Ho * Wo, b = m / Po, p = m % Po, oy = p / Wo, ox = p % Wo, Hu = Hi * up, Wu = Wi * up;
    const int e = ea[m], C4 = C >> 2;
    int8_t* hr = hi + (i64)m * Kp;
    uint8_t* lr = lo + (i64)m * Kp;
    for (int tap = 0; tap < 9; tap++) {
        int uy = oy * stride + tap / 3 - 1, ux = ox * stride + tap % 3 - 1;
        bool ok = uy >= 0 && ux >= 0 && uy < Hu && ux < Wu;
        const int4* src = (const int4*)(x + ((i64)b * Hi * Wi + (ok ? (uy / up) * Wi + ux / up : 0)) * C);
        for (int c4 = threadIdx.x; c4 < C4; c4 += blockDim.x) {
            int4 v = ok ? __ldg(src + c4) : make_int4(0, 0, 0, 0);
            int q0 = q16f(v.x, e), q1 = q16f(v.y, e), q2 = q16f(v.z, e), q3 = q16f(v.w, e);
            unsigned w01 = (q0 & 0xffff) | ((unsigned)q1 << 16), w23 = (q2 & 0xffff) | ((unsigned)q3 << 16);
            *(unsigned*)(hr + tap * C + c4 * 4) = __byte_perm(w01, w23, 0x7531);
            *(unsigned*)(lr + tap * C + c4 * 4) = __byte_perm(w01, w23, 0x6420);
        }
    }
    for (int k = 9 * C + threadIdx.x * 4; k < Kp; k += blockDim.x * 4) { *(unsigned*)(hr + k) = 0; *(unsigned*)(lr + k) = 0; }
}

constexpr int CV_RAW = 128 * 64 * 4, CV_Q = 2 * 128 * 64, CV_W = 128 * 64;
constexpr int CV_SMEM = 2 * CV_RAW + CV_Q + 2 * CV_W + 128 * 16;
template <int EMODE>
__global__ void __launch_bounds__(256) conv_q(const int* __restrict__ x, const int* __restrict__ ea, int Hi, int Wi, int C, int up, int stride,
                                              int Ho, int Wo, const int8_t* __restrict__ W, const int* __restrict__ mm, const int* __restrict__ ss,
                                              const int* __restrict__ bb, int M, int N, int Np, int Kp, int* __restrict__ out, int ldo,
                                              const int* __restrict__ res, const int* __restrict__ lphi) {
    extern __shared__ __align__(128) unsigned char smem[];
    unsigned char* raw = smem;
    unsigned char* qb = smem + 2 * CV_RAW;
    unsigned char* wb = qb + CV_Q;
    int4* rinfo = (int4*)(wb + 2 * CV_W);
    __shared__ int erow[128];
    const int n0 = blockIdx.x * G1N, m0 = blockIdx.y * G1M;
    const int warp = threadIdx.x >> 5, lane = threadIdx.x & 31;
    const int wm = (warp & 1) * 64, wn = (warp >> 1) * 32;
    const int Hu = Hi * up, Wu = Wi * up, Po = Ho * Wo, cpt = C / 64;
    if (threadIdx.x < 128) {
        int m = m0 + threadIdx.x;
        int4 ri = make_int4(0, 0, 0, 0);
        if (m < M) { int b = m / Po, p = m % Po; ri = make_int4(b * Hi * Wi, (p / Wo) * stride - 1, (p % Wo) * stride - 1, 1); }
        rinfo[threadIdx.x] = ri;
        erow[threadIdx.x] = m < M ? ea[m] : 0;
    }
    __syncthreads();
    int chh[4][4][4], cll[4][4][4];
#pragma unroll
    for (int a = 0; a < 4; a++)
#pragma unroll
        for (int b = 0; b < 4; b++)
#pragma unroll
            for (int i = 0; i < 4; i++) chh[a][b][i] = cll[a][b][i] = 0;
    const int nk = Kp / 64;
    auto load = [&](int stage, int kt) {
        if (kt < nk) {
            const int tap = kt / cpt, c0 = (kt % cpt) * 64, ky = tap / 3, kx = tap % 3;
            unsigned char* rb = raw + stage * CV_RAW;
#pragma unroll
            for (int it = 0; it < 8; it++) {
                int c = threadIdx.x + it * 256, r = c >> 4, ch = c & 15;
                int4 ri = rinfo[r];
                int uy = ri.y + ky, ux = ri.z + kx;
                bool ok = ri.w && uy >= 0 && ux >= 0 && uy < Hu && ux < Wu;
                i64 src = ok ? (i64)ri.x + (uy / up) * Wi + ux / up : 0;
                cp16(rb + r * 256 + ((ch ^ (r & 15)) << 4), x + src * C + c0 + ch * 4, ok);
            }
            unsigned char* wbs = wb + stage * CV_W;
#pragma unroll
            for (int it = 0; it < 2; it++) {
                int c = threadIdx.x + it * 256, r = c >> 2, ch = c & 3, gn = n0 + r;
                bool ok = gn < Np;
                cp16(wbs + swz(r, ch), W + (i64)(ok ? gn : 0) * Kp + (i64)kt * 64 + ch * 16, ok);
            }
        }
        asm volatile("cp.async.commit_group;");
    };
    load(0, 0);
    load(1, 1);
    for (int kt = 0; kt < nk; kt++) {
        asm volatile("cp.async.wait_group 1;");
        __syncthreads();
        {
            const unsigned char* rb = raw + (kt & 1) * CV_RAW;
#pragma unroll
            for (int it = 0; it < 8; it++) {
                int c = threadIdx.x + it * 256, r = c >> 4, ch = c & 15;
                int4 v = *(const int4*)(rb + r * 256 + ((ch ^ (r & 15)) << 4));
                int e = erow[r];
                int q0 = q16f(v.x, e), q1 = q16f(v.y, e), q2 = q16f(v.z, e), q3 = q16f(v.w, e);
                unsigned w01 = (q0 & 0xffff) | ((unsigned)q1 << 16), w23 = (q2 & 0xffff) | ((unsigned)q3 << 16);
                int off = swz(r, ch >> 2) + (ch & 3) * 4;
                *(unsigned*)(qb + off) = __byte_perm(w01, w23, 0x7531);
                *(unsigned*)(qb + 128 * 64 + off) = __byte_perm(w01, w23, 0x6420);
            }
        }
        __syncthreads();
        const unsigned char* wbs = wb + (kt & 1) * CV_W;
#pragma unroll
        for (int ks = 0; ks < 2; ks++) {
            unsigned bw[4][2];
#pragma unroll
            for (int np = 0; np < 2; np++) {
                int r = wn + np * 16 + (lane & 7) + ((lane >> 4) << 3), ch = ks * 2 + ((lane >> 3) & 1);
                unsigned t4[4];
                ldsm4(t4, wbs + swz(r, ch));
                bw[np * 2][0] = t4[0]; bw[np * 2][1] = t4[1]; bw[np * 2 + 1][0] = t4[2]; bw[np * 2 + 1][1] = t4[3];
            }
#pragma unroll
            for (int mt = 0; mt < 4; mt++) {
                int r = wm + mt * 16 + (lane & 15), ch = ks * 2 + (lane >> 4);
                unsigned ah[4], al[4];
                ldsm4(ah, qb + swz(r, ch));
                ldsm4(al, qb + 128 * 64 + swz(r, ch));
#pragma unroll
                for (int nt = 0; nt < 4; nt++) {
                    MMA("s8", "s8", chh[mt][nt], ah, bw[nt]);
                    MMA("u8", "s8", cll[mt][nt], al, bw[nt]);
                }
            }
        }
        __syncthreads();
        load(kt & 1, kt + 2);
    }
    asm volatile("cp.async.wait_group 0;");
    gemm_epilogue<EMODE>(chh, cll, m0, n0, wm, wn, lane, M, N, ea, mm, ss, bb, out, ldo, res, lphi);
}

constexpr int CV2_SMEM = 2 * CV_Q + 3 * CV_W + 128 * 16;
template <int EMODE>
__global__ void __launch_bounds__(256) conv_q2(const int* __restrict__ x, const int* __restrict__ ea, int Hi, int Wi, int C, int up, int stride,
                                               int Ho, int Wo, const int8_t* __restrict__ W, const int* __restrict__ mm, const int* __restrict__ ss,
                                               const int* __restrict__ bb, int M, int N, int Np, int Kp, int* __restrict__ out, int ldo,
                                               const int* __restrict__ res, const int* __restrict__ lphi) {
    extern __shared__ __align__(128) unsigned char smem[];
    unsigned char* qb = smem;
    unsigned char* wb = smem + 2 * CV_Q;
    int4* rinfo = (int4*)(wb + 3 * CV_W);
    __shared__ int erow[128];
    const int n0 = blockIdx.x * G1N, m0 = blockIdx.y * G1M;
    const int warp = threadIdx.x >> 5, lane = threadIdx.x & 31;
    const int wm = (warp & 1) * 64, wn = (warp >> 1) * 32;
    const int Hu = Hi * up, Wu = Wi * up, Po = Ho * Wo, cpt = C / 64;
    if (threadIdx.x < 128) {
        int m = m0 + threadIdx.x;
        int4 ri = make_int4(0, 0, 0, 0);
        if (m < M) { int b = m / Po, p = m % Po; ri = make_int4(b * Hi * Wi, (p / Wo) * stride - 1, (p % Wo) * stride - 1, 1); }
        rinfo[threadIdx.x] = ri;
        erow[threadIdx.x] = m < M ? ea[m] : 0;
    }
    __syncthreads();
    int chh[4][4][4], cll[4][4][4];
#pragma unroll
    for (int a = 0; a < 4; a++)
#pragma unroll
        for (int b = 0; b < 4; b++)
#pragma unroll
            for (int i = 0; i < 4; i++) chh[a][b][i] = cll[a][b][i] = 0;
    const int nk = Kp / 64;
    int4 rv[8];

    auto ldg = [&](int kt) {
        const int tap = kt / cpt, c0 = (kt % cpt) * 64, ky = tap / 3, kx = tap % 3;
#pragma unroll
        for (int it = 0; it < 8; it++) {
            int c = threadIdx.x + it * 256, r = c >> 4, ch = c & 15;
            int4 ri = rinfo[r];
            int uy = ri.y + ky, ux = ri.z + kx;
            bool ok = kt < nk && ri.w && uy >= 0 && ux >= 0 && uy < Hu && ux < Wu;
            rv[it] = ok ? __ldg((const int4*)(x + ((i64)ri.x + (uy / up) * Wi + ux / up) * C + c0 + ch * 4)) : make_int4(0, 0, 0, 0);
        }
    };
    auto quant_one = [&](int it, unsigned char* q) {
        int c = threadIdx.x + it * 256, r = c >> 4, ch = c & 15, e = erow[r];
        int4 v = rv[it];
        int q0 = q16f(v.x, e), q1 = q16f(v.y, e), q2 = q16f(v.z, e), q3 = q16f(v.w, e);
        unsigned w01 = (q0 & 0xffff) | ((unsigned)q1 << 16), w23 = (q2 & 0xffff) | ((unsigned)q3 << 16);
        int off = swz(r, ch >> 2) + (ch & 3) * 4;
        *(unsigned*)(q + off) = __byte_perm(w01, w23, 0x7531);
        *(unsigned*)(q + 128 * 64 + off) = __byte_perm(w01, w23, 0x6420);
    };
    auto loadw = [&](int stage, int kt) {
        if (kt < nk) {
            unsigned char* wbs = wb + stage * CV_W;
#pragma unroll
            for (int it = 0; it < 2; it++) {
                int c = threadIdx.x + it * 256, r = c >> 2, ch = c & 3, gn = n0 + r;
                bool ok = gn < Np;
                cp16(wbs + swz(r, ch), W + (i64)(ok ? gn : 0) * Kp + (i64)kt * 64 + ch * 16, ok);
            }
        }
        asm volatile("cp.async.commit_group;");
    };
    loadw(0, 0);
    loadw(1, 1);
    ldg(0);
#pragma unroll
    for (int it = 0; it < 8; it++) quant_one(it, qb);
    for (int kt = 0; kt < nk; kt++) {
        asm volatile("cp.async.wait_group 1;");
        __syncthreads();
        loadw((kt + 2) % 3, kt + 2);
        ldg(kt + 1);
        const unsigned char* qc = qb + (kt & 1) * CV_Q;
        unsigned char* qn = qb + ((kt + 1) & 1) * CV_Q;
        const unsigned char* wbs = wb + (kt % 3) * CV_W;
#pragma unroll
        for (int ks = 0; ks < 2; ks++) {
            unsigned bw[4][2];
#pragma unroll
            for (int np = 0; np < 2; np++) {
                int r = wn + np * 16 + (lane & 7) + ((lane >> 4) << 3), ch = ks * 2 + ((lane >> 3) & 1);
                unsigned t4[4];
                ldsm4(t4, wbs + swz(r, ch));
                bw[np * 2][0] = t4[0]; bw[np * 2][1] = t4[1]; bw[np * 2 + 1][0] = t4[2]; bw[np * 2 + 1][1] = t4[3];
            }
#pragma unroll
            for (int mt = 0; mt < 4; mt++) {
                int r = wm + mt * 16 + (lane & 15), ch = ks * 2 + (lane >> 4);
                unsigned ah[4], al[4];
                ldsm4(ah, qc + swz(r, ch));
                ldsm4(al, qc + 128 * 64 + swz(r, ch));
#pragma unroll
                for (int nt = 0; nt < 4; nt++) {
                    MMA("s8", "s8", chh[mt][nt], ah, bw[nt]);
                    MMA("u8", "s8", cll[mt][nt], al, bw[nt]);
                }
                if (kt + 1 < nk) quant_one(ks * 4 + mt, qn);
            }
        }
    }
    asm volatile("cp.async.wait_group 0;");
    gemm_epilogue<EMODE>(chh, cll, m0, n0, wm, wn, lane, M, N, ea, mm, ss, bb, out, ldo, res, lphi);
}

__global__ void __launch_bounds__(1024) norm_k(const int* __restrict__ x, int P, int C, int cg, int layer, const int* __restrict__ gam,
                                               const int* __restrict__ bet, int den, int* __restrict__ y) {
    __shared__ i64 sh[32];

    const int c0 = layer ? 0 : blockIdx.x * cg, ncols = layer ? C : cg, r0 = layer ? blockIdx.x : blockIdx.y * P, nrows = layer ? 1 : P;
    const i64 n = (i64)ncols * nrows;
    auto at = [&](i64 i) -> i64 { int r = r0 + (int)(i / ncols), c = c0 + (int)(i % ncols); return (i64)r * C + c; };
    i64 s = 0;
    for (i64 i = threadIdx.x; i < n; i += blockDim.x) s += x[at(i)];
    s = block_sum(s, sh);
    const i64 mean = floordiv(s + n / 2, n);
    i64 m = 0;
    for (i64 i = threadIdx.x; i < n; i += blockDim.x) { i64 d = x[at(i)] - mean; d = d < 0 ? -d : d; m = d > m ? d : m; }
    int e = exp_of(block_max(m, sh)) - 5;
    e = e < 0 ? 0 : e;
    i64 v2 = 0;
    for (i64 i = threadIdx.x; i < n; i += blockDim.x) { i64 dd = rshr64(x[at(i)] - mean, e); v2 += dd * dd; }
    v2 = block_sum(v2, sh);
    const i64 var = floordiv(v2, n);
    const i64 eps = floordiv((1LL << (32 - 2 * e)) + den / 2, den);
    const i64 sd = isqrt64((var + eps) << 16);
    for (i64 i = threadIdx.x; i < n; i += blockDim.x) {
        i64 idx = at(i);
        i64 d = x[idx] - mean;
        i64 yy = floordiv((d << (24 - e)) * 2 + sd, 2 * sd);
        int c = (int)(idx % C);
        y[idx] = sat32((i64)rshr64(yy * gam[c], 16) + bet[c]);
    }
}

struct GnSt { unsigned long long sum, mx, ss; int xmax, xmin; };
__device__ __forceinline__ void gn_group_par(const GnSt& st, i64 n, i64& mean, int& e) {
    mean = floordiv((i64)st.sum + n / 2, n);

    i64 a = (i64)st.xmax - mean, b2 = mean - (i64)st.xmin, m = a > b2 ? a : b2;
    e = exp_of(m) - 5;
    e = e < 0 ? 0 : e;
}
__global__ void gn_init(GnSt* st, int n) {
    int i = threadIdx.x;
    if (i < n) { st[i].sum = 0; st[i].mx = 0; st[i].ss = 0; st[i].xmax = INT_MIN; st[i].xmin = INT_MAX; }
}
template <int PASS>
__global__ void __launch_bounds__(256) gn_pass(const int* __restrict__ x, int P, int C, int cg, GnSt* __restrict__ st) {
    __shared__ unsigned long long acc[32];
    __shared__ int amx[32], amn[32];
    __shared__ i64 means[32];
    __shared__ int es[32];
    const int b = blockIdx.y, r0 = blockIdx.x * 32;
    const i64 n = (i64)cg * P;
    if (threadIdx.x < 32) {
        acc[threadIdx.x] = 0;
        amx[threadIdx.x] = INT_MIN; amn[threadIdx.x] = INT_MAX;
        if (PASS > 0) { i64 m; int e; gn_group_par(st[b * 32 + threadIdx.x], n, m, e); means[threadIdx.x] = m; es[threadIdx.x] = e; }
    }
    __syncthreads();
    const int rows = min(32, P - r0);
    const int* xb = x + ((i64)b * P + r0) * C;

    for (int c = threadIdx.x; c < C; c += 256) {
        const int gi = c / cg;
        i64 a = 0;
        int vmx = INT_MIN, vmn = INT_MAX;
        for (int r = 0; r < rows; r++) {
            i64 v = xb[(i64)r * C + c];
            if (PASS == 0) { a += v; vmx = max(vmx, (int)v); vmn = min(vmn, (int)v); }
            else if (PASS == 1) { i64 d = v - means[gi]; d = d < 0 ? -d : d; a = d > a ? d : a; }
            else { i64 dd = rshr64(v - means[gi], es[gi]); a += dd * dd; }
        }
        if (PASS == 1) atomicMax(&acc[gi], (unsigned long long)a); else atomicAdd(&acc[gi], (unsigned long long)a);
        if (PASS == 0) { atomicMax(&amx[gi], vmx); atomicMin(&amn[gi], vmn); }
    }
    __syncthreads();
    if (threadIdx.x < 32) {
        unsigned long long* dst = PASS == 0 ? &st[b * 32 + threadIdx.x].sum : (PASS == 1 ? &st[b * 32 + threadIdx.x].mx : &st[b * 32 + threadIdx.x].ss);
        if (PASS == 1) atomicMax(dst, acc[threadIdx.x]); else atomicAdd(dst, acc[threadIdx.x]);
        if (PASS == 0) { atomicMax(&st[b * 32 + threadIdx.x].xmax, amx[threadIdx.x]); atomicMin(&st[b * 32 + threadIdx.x].xmin, amn[threadIdx.x]); }
    }
}
__global__ void gn_apply(const int* __restrict__ x, int P, int C, int cg, const GnSt* __restrict__ st, int den, const int* __restrict__ gam,
                         const int* __restrict__ bet, int* __restrict__ y, i64 total, const int* __restrict__ lsig,
                         unsigned* __restrict__ pmax = nullptr) {
    __shared__ i64 means[32], sds[32];
    __shared__ unsigned long long minvs[32];
    __shared__ int es[32];
    __shared__ unsigned rmax[16];
    const i64 row0 = ((i64)blockIdx.x * 256 * 8) / C;
    if (pmax && threadIdx.x < 16) rmax[threadIdx.x] = 0;
    const i64 i0 = (i64)blockIdx.x * 256 * 8;
    const int b = (int)(i0 / ((i64)P * C));
    const i64 n = (i64)cg * P;
    if (threadIdx.x < 32) {
        const GnSt& s = st[b * 32 + threadIdx.x];
        i64 m; int e;
        gn_group_par(s, n, m, e);
        i64 var = floordiv((i64)s.ss, n), eps = floordiv((1LL << (32 - 2 * e)) + den / 2, den);
        means[threadIdx.x] = m; es[threadIdx.x] = e; sds[threadIdx.x] = isqrt64((var + eps) << 16);
        minvs[threadIdx.x] = ~0ull / (unsigned long long)(2 * sds[threadIdx.x]);
    }
    __syncthreads();
    for (int k = 0; k < 8; k++) {
        i64 i = i0 + k * 256 + threadIdx.x;
        if (i >= total) break;
        int c = (int)(i % C), gi = c / cg;
        i64 d = x[i] - means[gi], sd = sds[gi];
        i64 yy = floordiv_m((d << (24 - es[gi])) * 2 + sd, (unsigned long long)(2 * sd), minvs[gi]);
        int o = sat32((i64)rshr64(yy * gam[c], 16) + bet[c]);
        if (lsig) o = sat32((i64)rshr64((i64)o * lut_q16(lsig, o), 16));
        y[i] = o;
        if (pmax) { unsigned a = o < 0 ? 0u - (unsigned)o : (unsigned)o; atomicMax(&rmax[(int)(i / C - row0)], a); }
    }
    if (pmax) {
        __syncthreads();
        const i64 rows = total / C, rlast = (min(i0 + 2048, total) - 1) / C;
        if (threadIdx.x < 16 && row0 + threadIdx.x <= rlast && row0 + threadIdx.x < rows) atomicMax(&pmax[row0 + threadIdx.x], rmax[threadIdx.x]);
    }
}

__global__ void __launch_bounds__(256) prep_vq(const int* __restrict__ v, const unsigned* __restrict__ cmax, int T1, int Tp1, int Bn, int C,
                                               int8_t* __restrict__ hi, uint8_t* __restrict__ lo, int* __restrict__ ev) {
    __shared__ unsigned short tq[64][66];
    const int tile = blockIdx.x, h = blockIdx.y, b = blockIdx.z, H = C / 64;
    const int* vb = v + (i64)b * T1 * C;
    for (int i = threadIdx.x; i < 64 * 64; i += 256) {
        const int key = i >> 6, c = i & 63, t = tile * 64 + key;
        const int e = exp_of((i64)cmax[(i64)b * C + h * 64 + c]);
        const int q = t < T1 ? quant16(vb[(i64)t * C + h * 64 + c], e) : 0;
        tq[c][key] = (unsigned short)q;
    }
    if (tile == 0 && threadIdx.x < 64) ev[(b * H + h) * 64 + threadIdx.x] = exp_of((i64)cmax[(i64)b * C + h * 64 + threadIdx.x]);
    __syncthreads();
    const i64 Tp = (i64)Bn * Tp1;
    for (int i = threadIdx.x; i < 64 * 64; i += 256) {
        const int c = i >> 6, pz = i & 63;
        const int q = (short)tq[c][PERM[pz]];
        const i64 at = ((i64)h * 64 + c) * Tp + (i64)b * Tp1 + tile * 64 + pz;
        hi[at] = (int8_t)(q >> 8);
        lo[at] = (uint8_t)(q & 255);
    }
}

__global__ void __launch_bounds__(256) gn_apply2(const int* __restrict__ x, int P, int C, int cg, const GnSt* __restrict__ st, int den,
                                                 const int* __restrict__ gam, const int* __restrict__ bet, int* __restrict__ y,
                                                 const int* __restrict__ lsig, unsigned* __restrict__ pmax) {
    __shared__ i64 means[32], sds[32];
    __shared__ unsigned long long minvs[32];
    __shared__ int es[32];
    __shared__ unsigned rmax[8];
    const i64 r0 = (i64)blockIdx.x * 8;
    const int b = (int)(r0 / P);
    const i64 n = (i64)cg * P;
    if (threadIdx.x < 32) {
        const GnSt& s = st[b * 32 + threadIdx.x];
        i64 m; int e;
        gn_group_par(s, n, m, e);
        i64 var = floordiv((i64)s.ss, n), eps = floordiv((1LL << (32 - 2 * e)) + den / 2, den);
        means[threadIdx.x] = m; es[threadIdx.x] = e; sds[threadIdx.x] = isqrt64((var + eps) << 16);
        minvs[threadIdx.x] = ~0ull / (unsigned long long)(2 * sds[threadIdx.x]);
    }
    if (threadIdx.x < 8) rmax[threadIdx.x] = 0;
    __syncthreads();
    unsigned am[8] = {0, 0, 0, 0, 0, 0, 0, 0};
    for (int c = threadIdx.x; c < C; c += 256) {
        const int gi = c / cg, e = es[gi];
        const i64 mean = means[gi], sd = sds[gi];
        const unsigned long long dv = (unsigned long long)(2 * sd), minv = minvs[gi];
        const i64 g = gam[c];
        const int be = bet[c];
#pragma unroll
        for (int r = 0; r < 8; r++) {
            const i64 i = (r0 + r) * C + c;
            const i64 d = x[i] - mean;
            const i64 yy = floordiv_m((d << (24 - e)) * 2 + sd, dv, minv);
            int o = sat32((i64)rshr64(yy * g, 16) + be);
            if (lsig) o = sat32((i64)rshr64((i64)o * lut_q16(lsig, o), 16));
            y[i] = o;
            const unsigned a = o < 0 ? 0u - (unsigned)o : (unsigned)o;
            am[r] = a > am[r] ? a : am[r];
        }
    }
    if (pmax) {
#pragma unroll
        for (int r = 0; r < 8; r++) {
            unsigned a = am[r];
            for (int o2 = 16; o2 > 0; o2 >>= 1) a = max(a, __shfl_xor_sync(~0u, a, o2));
            if ((threadIdx.x & 31) == 0) atomicMax(&rmax[r], a);
        }
        __syncthreads();
        if (threadIdx.x < 8) atomicMax(&pmax[r0 + threadIdx.x], rmax[threadIdx.x]);
    }
}

template <int NPL>
__global__ void __launch_bounds__(256) gn_rows_q(const int* __restrict__ x, int P, int cg, const GnSt* __restrict__ st, int den,
                                                 const int* __restrict__ gam, const int* __restrict__ bet, int R, int8_t* __restrict__ qhi,
                                                 uint8_t* __restrict__ qlo, int* __restrict__ qe) {
    constexpr int C = 32 * NPL;
    __shared__ i64 means[32], sds[32];
    __shared__ unsigned long long minvs[32];
    __shared__ int es[32];
    const int row = blockIdx.x * 8 + (threadIdx.x >> 5), lane = threadIdx.x & 31, b = (blockIdx.x * 8) / P;
    const i64 n = (i64)cg * P;
    if (threadIdx.x < 32) {
        const GnSt& s = st[b * 32 + threadIdx.x];
        i64 m; int e;
        gn_group_par(s, n, m, e);
        i64 var = floordiv((i64)s.ss, n), eps = floordiv((1LL << (32 - 2 * e)) + den / 2, den);
        means[threadIdx.x] = m; es[threadIdx.x] = e; sds[threadIdx.x] = isqrt64((var + eps) << 16);
        minvs[threadIdx.x] = ~0ull / (unsigned long long)(2 * sds[threadIdx.x]);
    }
    __syncthreads();
    if (row >= R) return;
    const int* xr = x + (i64)row * C;
    int o[NPL];
    i64 am = 0;
#pragma unroll
    for (int k = 0; k < NPL; k++) {
        const int c = k * 32 + lane, gi = c / cg;
        i64 d = xr[c] - means[gi], sd = sds[gi];
        i64 yy = floordiv_m((d << (24 - es[gi])) * 2 + sd, (unsigned long long)(2 * sd), minvs[gi]);
        o[k] = sat32((i64)rshr64(yy * gam[c], 16) + bet[c]);
        i64 a = o[k] < 0 ? -(i64)o[k] : o[k];
        am = a > am ? a : am;
    }
    for (int s2 = 16; s2 > 0; s2 >>= 1) { i64 w = __shfl_xor_sync(~0u, am, s2); am = w > am ? w : am; }
    const int ee = exp_of(am);
#pragma unroll
    for (int k = 0; k < NPL; k++) {
        int q = q16f(o[k], ee), c = k * 32 + lane;
        qhi[(i64)row * C + c] = (int8_t)(q >> 8);
        qlo[(i64)row * C + c] = (uint8_t)(q & 255);
    }
    if (lane == 0) qe[row] = ee;
}

template <int NPL, bool QO = false>
__global__ void __launch_bounds__(256) ln_warp(const int* __restrict__ x, int R, const int* __restrict__ gam, const int* __restrict__ bet, int den,
                                               int* __restrict__ y, int8_t* __restrict__ qhi = nullptr, uint8_t* __restrict__ qlo = nullptr,
                                               int* __restrict__ qe = nullptr) {
    const int row = blockIdx.x * 8 + (threadIdx.x >> 5), lane = threadIdx.x & 31;
    if (row >= R) return;
    constexpr int C = 32 * NPL;
    const int* xr = x + (i64)row * C;
    int v[NPL];
    i64 s = 0;
#pragma unroll
    for (int k = 0; k < NPL; k++) { v[k] = xr[k * 32 + lane]; s += v[k]; }
    for (int o = 16; o > 0; o >>= 1) s += __shfl_xor_sync(~0u, s, o);
    const i64 mean = floordiv(s + C / 2, C);
    i64 m = 0;
#pragma unroll
    for (int k = 0; k < NPL; k++) { i64 d = v[k] - mean; d = d < 0 ? -d : d; m = d > m ? d : m; }
    for (int o = 16; o > 0; o >>= 1) { i64 w = __shfl_xor_sync(~0u, m, o); m = w > m ? w : m; }
    int e = exp_of(m) - 5;
    e = e < 0 ? 0 : e;
    i64 v2 = 0;
#pragma unroll
    for (int k = 0; k < NPL; k++) { i64 dd = rshr64(v[k] - mean, e); v2 += dd * dd; }
    for (int o = 16; o > 0; o >>= 1) v2 += __shfl_xor_sync(~0u, v2, o);
    const i64 var = floordiv(v2, C), eps = floordiv((1LL << (32 - 2 * e)) + den / 2, den), sd = isqrt64((var + eps) << 16);
    const unsigned long long dv = (unsigned long long)(2 * sd), minv = ~0ull / dv;
    int o[NPL];
    i64 am = 0;
#pragma unroll
    for (int k = 0; k < NPL; k++) {
        int c = k * 32 + lane;
        i64 yy = floordiv_m(((i64)(v[k] - mean) << (24 - e)) * 2 + sd, dv, minv);
        o[k] = sat32((i64)rshr64(yy * gam[c], 16) + bet[c]);
        if constexpr (!QO) y[(i64)row * C + c] = o[k];
        else { i64 a = o[k] < 0 ? -(i64)o[k] : o[k]; am = a > am ? a : am; }
    }
    if constexpr (QO) {
        for (int s2 = 16; s2 > 0; s2 >>= 1) { i64 w = __shfl_xor_sync(~0u, am, s2); am = w > am ? w : am; }
        const int ee = exp_of(am);
#pragma unroll
        for (int k = 0; k < NPL; k++) {
            int q = q16f(o[k], ee), c = k * 32 + lane;
            qhi[(i64)row * C + c] = (int8_t)(q >> 8);
            qlo[(i64)row * C + c] = (uint8_t)(q & 255);
        }
        if (lane == 0) qe[row] = ee;
    }
}

__global__ void ewise(const int* __restrict__ x, const int* __restrict__ x2, i64 n, int mode, const int* __restrict__ L, int* __restrict__ y) {
    i64 i = blockIdx.x * (i64)blockDim.x + threadIdx.x;
    if (i >= n) return;
    int v = x[i];
    if (mode == 0 || mode == 1) y[i] = sat32((i64)rshr64((i64)v * lut_q16(L, v), 16));
    else if (mode == 2) { int gv = x2[i]; int ge = sat32((i64)rshr64((i64)gv * lut_q16(L, gv), 16)); y[i] = sat32((i64)rshr64((i64)v * ge, 16)); }
    else y[i] = sat32((i64)v + x2[i]);
}

__global__ void concat_k(const int* a, int C1, const int* b, int C2, int P, int* y) {
    i64 i = blockIdx.x * (i64)blockDim.x + threadIdx.x, C = C1 + C2;
    if (i >= (i64)P * C) return;
    int p = (int)(i / C), c = (int)(i % C);
    y[i] = c < C1 ? a[(i64)p * C1 + c] : b[(i64)p * C2 + c - C1];
}

__device__ __forceinline__ unsigned rotl32(unsigned v, int c) { return (v << c) | (v >> (32 - c)); }
#define QR(a, b, c, d) a += b; d ^= a; d = rotl32(d, 16); c += d; b ^= c; b = rotl32(b, 12); a += b; d ^= a; d = rotl32(d, 8); c += d; b ^= c; b = rotl32(b, 7);
__global__ void noise_k(const unsigned char* __restrict__ seeds, int Bn, const int* __restrict__ gauss, int cin, int* __restrict__ ntok,
                        int* __restrict__ xin) {
    const int b = blockIdx.y, bi = blockIdx.x * blockDim.x + threadIdx.x;
    if (bi >= 512 || b >= Bn) return;
    const unsigned char* sd = seeds + b * 32;
    unsigned st[16];
    st[0] = 0x61707865; st[1] = 0x3320646e; st[2] = 0x79622d32; st[3] = 0x6b206574;
    for (int i = 0; i < 8; i++) st[4 + i] = sd[4 * i] | (sd[4 * i + 1] << 8) | (sd[4 * i + 2] << 16) | ((unsigned)sd[4 * i + 3] << 24);
    st[12] = bi; st[13] = st[14] = st[15] = 0;
    unsigned x[16];
    for (int i = 0; i < 16; i++) x[i] = st[i];
    for (int r = 0; r < 10; r++) {
        QR(x[0], x[4], x[8], x[12]) QR(x[1], x[5], x[9], x[13]) QR(x[2], x[6], x[10], x[14]) QR(x[3], x[7], x[11], x[15])
        QR(x[0], x[5], x[10], x[15]) QR(x[1], x[6], x[11], x[12]) QR(x[2], x[7], x[8], x[13]) QR(x[3], x[4], x[9], x[14])
    }
    for (int w = 0; w < 16; w++) {
        const unsigned v = x[w] + st[w];
        for (int h = 0; h < 2; h++) {
            const int j = bi * 32 + w * 2 + h, c = j >> 12, p = j & 4095;
            const int n = gauss[(v >> (16 * h)) & 0xffff];
            const i64 at = ((i64)b * 4096 + p) * 4 + c;
            ntok[at] = n;
            xin[at] = rshr64((i64)n * cin, 30);
        }
    }
}
#undef QR

__global__ void x0_k(const int* __restrict__ ntok, const int* __restrict__ eps, int Bn, int sig, unsigned char* __restrict__ outb) {
    const i64 i = blockIdx.x * (i64)blockDim.x + threadIdx.x;
    if (i >= (i64)Bn * 16384) return;
    const int b = (int)(i / 16384), r = (int)(i % 16384), p = r >> 2, c = r & 3;
    const int x0 = rshr64(((i64)ntok[i] - eps[i]) * sig, 16);
    int q = rshr64(x0, 8);
    q = q > 32767 ? 32767 : (q < -32767 ? -32767 : q);
    unsigned char* o = outb + (i64)b * 32768 + ((c << 12) + p) * 2;
    o[0] = (unsigned char)(q & 255); o[1] = (unsigned char)((q >> 8) & 255);
}

__global__ void temb_k(const int* __restrict__ ids, const int8_t* __restrict__ w, const int* __restrict__ m, const int* __restrict__ sv,
                       const int* __restrict__ pos, int D, int* __restrict__ x) {
    const int t = blockIdx.x, id = ids[t];
    for (int d = threadIdx.x; d < D; d += blockDim.x)
        x[(i64)t * D + d] = sat32((i64)rshr64((i64)w[(i64)id * D + d] * m[id], sv[id] - 16) + pos[(i64)t * D + d]);
}

__global__ void attn_naive(const int* __restrict__ q, const int* __restrict__ k, const int* __restrict__ v, int T, int C, int causal,
                           const int* __restrict__ L, int* __restrict__ out) {
    __shared__ i64 sh[32];
    __shared__ int s_s[128], ev_s[64];
    __shared__ i64 E_s[128];
    const int h = blockIdx.y, i = blockIdx.x, d = threadIdx.x;
    const i64 qv = q[(i64)i * C + h * 64 + d];
    const int eq = exp_of(block_max(qv < 0 ? -qv : qv, sh));
    const i64 qq = quant16(qv, eq);
    for (int j = 0; j < T; j++) {
        const i64 kv = k[(i64)j * C + h * 64 + d];
        const int ek = exp_of(block_max(kv < 0 ? -kv : kv, sh));
        const i64 S = block_sum(qq * quant16(kv, ek), sh);
        if (d == 0) s_s[j] = (causal && j > i) ? INT_MIN : rshr(S, 19 - eq - ek);
    }
    {
        i64 m = 0;
        for (int j = 0; j < T; j++) { i64 a = v[(i64)j * C + h * 64 + d]; a = a < 0 ? -a : a; m = a > m ? a : m; }
        ev_s[d] = exp_of(m);
    }
    __syncthreads();
    int mx = INT_MIN;
    for (int j = 0; j < T; j++) mx = max(mx, s_s[j]);
    i64 sumE = 0;
    for (int j = 0; j < T; j++) {
        const i64 z = (i64)mx - s_s[j];
        i64 E = 0;
        if (z < (16LL << 16)) { const int zc = (int)(z < 0 ? 0 : z), ix = zc >> 10, f = zc & 1023; E = (i64)L[ix] + ((((i64)L[ix + 1] - L[ix]) * f) >> 10); }
        if (d == 0) E_s[j] = E;
        sumE += E;
    }
    __syncthreads();
    const i64 r = (1LL << 62) / sumE;
    i64 O = 0;
    for (int j = 0; j < T; j++) {
        i64 P = (E_s[j] * r + (1LL << 46)) >> 47;
        P = P > 32767 ? 32767 : P;
        O += P * quant16(v[(i64)j * C + h * 64 + d], ev_s[d]);
    }
    out[(i64)i * C + h * 64 + d] = rshr(O, 15 - ev_s[d]);
}

struct T { int dt; std::vector<uint32_t> dims; const unsigned char* data; size_t n; };
struct Act { int* p; int P, C; };

struct Engine {
    int B = 1;
    std::vector<unsigned char> file;
    std::map<std::string, T> t;
    std::map<std::string, void*> dev;
    int *lut_sig, *lut_phi;
    int4* lut4;

    bool check = false;
    std::vector<std::pair<std::string, std::string>> man;
    std::vector<std::vector<int>> manshape;
    size_t seq = 0, bad = 0;
    std::vector<void*> tmp;

    void load(const char* path) {
        FILE* f = fopen(path, "rb");
        fseek(f, 0, SEEK_END);
        file.resize(ftell(f));
        fseek(f, 0, SEEK_SET);
        if (fread(file.data(), 1, file.size(), f) != file.size()) exit(1);
        fclose(f);
        size_t pos = 12;
        uint32_t cnt;
        memcpy(&cnt, &file[8], 4);
        static const int sz[4] = {1, 2, 4, 1};
        for (uint32_t i = 0; i < cnt; i++) {
            uint16_t nl;
            memcpy(&nl, &file[pos], 2); pos += 2;
            std::string name((const char*)&file[pos], nl); pos += nl;
            T x; x.dt = file[pos]; int nd = file[pos + 1]; pos += 2;
            x.n = 1;
            for (int d = 0; d < nd; d++) { uint32_t v; memcpy(&v, &file[pos], 4); pos += 4; x.dims.push_back(v); x.n *= v; }
            x.data = &file[pos];
            pos += x.n * sz[x.dt];
            t[name] = x;
        }
    }
    template <class TT> TT* up(const std::string& name, const void* src, size_t bytes) {
        auto it = dev.find(name);
        if (it != dev.end()) return (TT*)it->second;
        void* d;
        CK(cudaMalloc(&d, bytes));
        CK(cudaMemcpy(d, src, bytes, cudaMemcpyHostToDevice));
        dev[name] = d;
        return (TT*)d;
    }
    const int* i32(const std::string& n) { const T& x = t.at(n); return up<int>(n, x.data, x.n * 4); }

    void make_ag(const std::string& base) {
        if (t.count(base + ".ag.w")) return;
        const T &wa = t.at(base + ".a.w"), &wg = t.at(base + ".g.w");
        int N = wa.dims[0], K = wa.dims[1];
        auto* w = new std::vector<int8_t>((size_t)2 * N * K);
        for (int j = 0; j < N; j++) {
            memcpy(&(*w)[(size_t)(2 * j) * K], wa.data + (size_t)j * K, K);
            memcpy(&(*w)[(size_t)(2 * j + 1) * K], wg.data + (size_t)j * K, K);
        }
        t[base + ".ag.w"] = T{0, {(uint32_t)(2 * N), (uint32_t)K}, (const unsigned char*)w->data(), (size_t)2 * N * K};
        for (const char* sfx : {".m", ".s", ".b"}) {
            const int *pa = (const int*)t.at(base + ".a" + sfx).data, *pg = (const int*)t.at(base + ".g" + sfx).data;
            auto* v = new std::vector<int>(2 * N);
            for (int j = 0; j < N; j++) { (*v)[2 * j] = pa[j]; (*v)[2 * j + 1] = pg[j]; }
            t[base + ".ag" + sfx] = T{2, {(uint32_t)(2 * N)}, (const unsigned char*)v->data(), (size_t)2 * N};
        }
    }
    const int8_t* weight(const std::string& base, int C3, int& N, int& K, int& Kp) {
        const T& w = t.at(base + ".w");
        N = w.dims[0]; K = w.dims[1];
        int Np = (N + 63) / 64 * 64;
        Kp = (K + 63) / 64 * 64;
        std::string key = base + ".w#" + std::to_string(C3);
        auto it = dev.find(key);
        if (it != dev.end()) return (const int8_t*)it->second;
        std::vector<int8_t> h((size_t)Np * Kp, 0);
        const int8_t* src = (const int8_t*)w.data;
        for (int n = 0; n < N; n++)
            for (int k = 0; k < K; k++) {
                int kk = k;
                if (C3) { int c = k / 9, tap = k % 9; kk = tap * C3 + c; }
                h[(size_t)n * Kp + kk] = src[(size_t)n * K + k];
            }
        return up<int8_t>(key, h.data(), h.size());
    }

    std::unordered_map<void*, int> rc;
    int* alloc(size_t n) { int* p; CK(cudaMallocAsync((void**)&p, n * 4, 0)); rc[p] = 1; return p; }
    void keep(const Act& a) { rc[a.p]++; }
    struct QB { int8_t* hi; uint8_t* lo; int* e; int Kp; };
    std::unordered_map<void*, QB> qcache;
    std::unordered_map<void*, unsigned*> pcache;
    std::unordered_map<void*, unsigned*> rmcache;
    std::unordered_map<void*, unsigned*> cmcache;
    bool fuse = true;
    void drop(const Act& a) {
        auto it = rc.find(a.p);
        if (it == rc.end()) return;
        if (--it->second == 0) {
            auto q = qcache.find(a.p);
            if (q != qcache.end()) { cudaFreeAsync(q->second.hi, 0); cudaFreeAsync(q->second.lo, 0); cudaFreeAsync(q->second.e, 0); qcache.erase(q); }
            auto pq = pcache.find(a.p);
            if (pq != pcache.end()) { cudaFreeAsync(pq->second, 0); pcache.erase(pq); }
            auto rq = rmcache.find(a.p);
            if (rq != rmcache.end()) { cudaFreeAsync(rq->second, 0); rmcache.erase(rq); }
            auto cq = cmcache.find(a.p);
            if (cq != cmcache.end()) { cudaFreeAsync(cq->second, 0); cmcache.erase(cq); }
            cudaFreeAsync(a.p, 0);
            rc.erase(it);
        }
    }
    Act act(int P, int C) { return Act{alloc((size_t)P * C), P, C}; }

    void verify(const char* kind, const std::string& name, const Act& a, bool chw) {
        if (!check) return;
        CK(cudaDeviceSynchronize());
        std::vector<int> h((size_t)a.P * a.C), o(h.size());
        CK(cudaMemcpy(h.data(), a.p, h.size() * 4, cudaMemcpyDeviceToHost));
        if (chw) { for (int p = 0; p < a.P; p++) for (int c = 0; c < a.C; c++) o[(size_t)c * a.P + p] = h[(size_t)p * a.C + c]; }
        else o = h;
        unsigned char md[32];
        char hex[65];
        for (int i = 0; i < 32; i++) sprintf(hex + 2 * i, "%02x", md[i]);
        std::string want = seq < man.size() ? man[seq].second : "?";
        std::string what = std::string(kind) + " " + name;
        bool ok = want == hex && man[seq].first == what;
        if (!ok) {
            bad++;
        }
        seq++;
    }

    Act lin(const Act& x, const std::string& name, int emode = 0, const int* res = nullptr, bool rm = false, int cmImgs = 0) {
        int N, K, Kp;
        const int8_t* W = weight(name, 0, N, K, Kp);
        if (K != x.C) { fprintf(stderr, "vela engine: model shape error (E11)\n"); exit(1); }
        QB qb;
        auto it = qcache.find(x.p);
        bool cached = it != qcache.end() && rc.count(x.p);
        if (cached) qb = it->second;
        else {
            qb.Kp = Kp;
            CK(cudaMallocAsync((void**)&qb.hi, (size_t)x.P * Kp, 0)); CK(cudaMallocAsync((void**)&qb.lo, (size_t)x.P * Kp, 0)); CK(cudaMallocAsync((void**)&qb.e, 4ull * x.P, 0));
            auto rq = rmcache.find(x.p);
            if (rq != rmcache.end() && rc.count(x.p) && K % 4 == 0) quant_rows_m<<<x.P, 128>>>(x.p, K, Kp, rq->second, qb.hi, qb.lo, qb.e);
            else quant_rows<<<x.P, 256>>>(x.p, x.P, K, K, Kp, qb.hi, qb.lo, qb.e);
            if (rc.count(x.p)) qcache[x.p] = qb;
        }
        Act y = act(x.P, emode == 3 ? N / 2 : N);
        unsigned* cmp = nullptr;
        if (cmImgs) {
            CK(cudaMallocAsync((void**)&cmp, 4ull * N * cmImgs, 0));
            CK(cudaMemsetAsync(cmp, 0, 4ull * N * cmImgs, 0));
            cmcache[y.p] = cmp;
        }
        unsigned* rmp = nullptr;
        if (rm) {
            CK(cudaMallocAsync((void**)&rmp, 4ull * x.P, 0));
            CK(cudaMemsetAsync(rmp, 0, 4ull * x.P, 0));
            rmcache[y.p] = rmp;
        }
        gemm(qb.hi, qb.lo, qb.e, W, name, x.P, N, Kp, y.p, emode, res, HQ{}, rmp, cmp, cmImgs ? x.P / cmImgs : 1);
        if (!rc.count(x.p)) { cudaFreeAsync(qb.hi, 0); cudaFreeAsync(qb.lo, 0); cudaFreeAsync(qb.e, 0); }
        verify("lin", name, y, false);
        return y;
    }
    void gemm(const int8_t* hi, const uint8_t* lo, const int* e, const int8_t* W, const std::string& name, int M, int N, int Kp, int* out,
              int emode = 0, const int* res = nullptr, HQ hq = HQ{}, unsigned* rmp = nullptr, unsigned* cmp = nullptr, int p1 = 1) {
        static int gv = getenv(nullptr) ? envnum(getenv(nullptr)) : 1;
        static thread_local bool attr = false;
        if (!attr) {
            CK(cudaFuncSetAttribute(gemm_q, cudaFuncAttributeMaxDynamicSharedMemorySize, (4 * 128 + 2 * 64) * GS));
            for (auto f : {(const void*)gemm_q1<0>, (const void*)gemm_q1<1>, (const void*)gemm_q1<2>, (const void*)gemm_q1<3>, (const void*)gemm_q1<4>})
                CK(cudaFuncSetAttribute(f, cudaFuncAttributeMaxDynamicSharedMemorySize, G1S * G1STAGE));
            for (auto f : {(const void*)gemm_q1<0, 160>, (const void*)gemm_q1<1, 160>, (const void*)gemm_q1<2, 160>, (const void*)gemm_q1<3, 160>})
                CK(cudaFuncSetAttribute(f, cudaFuncAttributeMaxDynamicSharedMemorySize, G1S * (2 * G1M + 160) * 64));
            attr = true;
        }
        const int Np = (N + 63) / 64 * 64;
        if (gv == 0) gemm_q<<<dim3((M + 127) / 128, (N + 63) / 64), 256, (4 * 128 + 2 * 64) * GS>>>(hi, lo, e, W, i32(name + ".m"), i32(name + ".s"), i32(name + ".b"), M, N, Kp, out, N);
        else {
            static int bn160 = getenv(nullptr) ? envnum(getenv(nullptr)) == 160 : 0;
            const int *mm = i32(name + ".m"), *ss = i32(name + ".s"), *bb = i32(name + ".b");
            if (bn160 && N % 160 == 0 && emode != 4) {
                dim3 g2(N / 160, (M + G1M - 1) / G1M);
                const int sm2 = G1S * (2 * G1M + 160) * 64;
                if (emode == 0) gemm_q1<0, 160><<<g2, 320, sm2>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
                else if (emode == 1) gemm_q1<1, 160><<<g2, 320, sm2>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
                else if (emode == 2) gemm_q1<2, 160><<<g2, 320, sm2>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
                else gemm_q1<3, 160><<<g2, 320, sm2>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N / 2, res, lut_phi);
                return;
            }
            static int smallk = getenv(nullptr) ? envnum(getenv(nullptr)) : 1 << 30;
            if (Kp <= smallk) {
                static thread_local bool attr64 = false;
                const int sm64 = G1S * (2 * 64 + 128) * 64;
                if (!attr64) {
                    for (auto f : {(const void*)gemm_q1<0, 128, 64, 32>, (const void*)gemm_q1<1, 128, 64, 32>, (const void*)gemm_q1<2, 128, 64, 32>, (const void*)gemm_q1<3, 128, 64, 32>, (const void*)gemm_q1<4, 128, 64, 32>})
                        CK(cudaFuncSetAttribute(f, cudaFuncAttributeMaxDynamicSharedMemorySize, sm64));
                    attr64 = true;
                }
                static int g128 = getenv(nullptr) ? envnum(getenv(nullptr)) : 0;
                if (g128) {
                    static thread_local bool a2 = false;
                    const int sm2 = G1S * (2 * 128 + 128) * 64;
                    if (!a2) {
                        for (auto f : {(const void*)gemm_q1<0, 128, 128, 32>, (const void*)gemm_q1<1, 128, 128, 32>, (const void*)gemm_q1<2, 128, 128, 32>, (const void*)gemm_q1<3, 128, 128, 32>, (const void*)gemm_q1<4, 128, 128, 32>})
                            CK(cudaFuncSetAttribute(f, cudaFuncAttributeMaxDynamicSharedMemorySize, sm2));
                        a2 = true;
                    }
                    dim3 g2((N + 127) / 128, (M + 127) / 128);
                    if (emode == 0) gemm_q1<0, 128, 128, 32><<<g2, 512, sm2>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
                    else if (emode == 1) gemm_q1<1, 128, 128, 32><<<g2, 512, sm2>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
                    else if (emode == 2) gemm_q1<2, 128, 128, 32><<<g2, 512, sm2>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
                    else if (emode == 3) gemm_q1<3, 128, 128, 32><<<g2, 512, sm2>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N / 2, res, lut_phi);
                    else gemm_q1<4, 128, 128, 32><<<g2, 512, sm2>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi, hq);
                    return;
                }
                static int gcfg = getenv(nullptr) ? envnum(getenv(nullptr)) : 0;
                if (gcfg) {
                    static thread_local bool a3 = false;
                    const int smc = gcfg == 1 ? 2 * (2 * 64 + 128) * 64 : 3 * (2 * 64 + 64) * 64;
                    if (!a3) {
                        if (gcfg == 1) for (auto f : {(const void*)gemm_q1<0, 128, 64, 32, 2>, (const void*)gemm_q1<1, 128, 64, 32, 2>, (const void*)gemm_q1<2, 128, 64, 32, 2>, (const void*)gemm_q1<3, 128, 64, 32, 2>, (const void*)gemm_q1<4, 128, 64, 32, 2>}) CK(cudaFuncSetAttribute(f, cudaFuncAttributeMaxDynamicSharedMemorySize, smc));
                        else for (auto f : {(const void*)gemm_q1<0, 64, 64, 32, 3>, (const void*)gemm_q1<1, 64, 64, 32, 3>, (const void*)gemm_q1<2, 64, 64, 32, 3>, (const void*)gemm_q1<3, 64, 64, 32, 3>}) CK(cudaFuncSetAttribute(f, cudaFuncAttributeMaxDynamicSharedMemorySize, smc));
                        a3 = true;
                    }
                    if (gcfg == 1) {
                        dim3 gc((N + 127) / 128, (M + 63) / 64);
                        if (emode == 0) gemm_q1<0, 128, 64, 32, 2><<<gc, 256, smc>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
                        else if (emode == 1) gemm_q1<1, 128, 64, 32, 2><<<gc, 256, smc>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
                        else if (emode == 2) gemm_q1<2, 128, 64, 32, 2><<<gc, 256, smc>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
                        else if (emode == 3) gemm_q1<3, 128, 64, 32, 2><<<gc, 256, smc>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N / 2, res, lut_phi);
                        else gemm_q1<4, 128, 64, 32, 2><<<gc, 256, smc>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi, hq);
                        return;
                    }
                    if (emode != 4) {
                        dim3 gc((N + 63) / 64, (M + 63) / 64);
                        if (emode == 0) gemm_q1<0, 64, 64, 32, 3><<<gc, 128, smc>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
                        else if (emode == 1) gemm_q1<1, 64, 64, 32, 3><<<gc, 128, smc>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
                        else if (emode == 2) gemm_q1<2, 64, 64, 32, 3><<<gc, 128, smc>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
                        else gemm_q1<3, 64, 64, 32, 3><<<gc, 128, smc>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N / 2, res, lut_phi);
                        return;
                    }
                }
                dim3 g6((N + 127) / 128, (M + 63) / 64);

                static thread_local int mb2k = getenv(nullptr) ? envnum(getenv(nullptr)) : [] { int d = 0, mj = 0; cudaGetDevice(&d);
                    cudaDeviceGetAttribute(&mj, cudaDevAttrComputeCapabilityMajor, d); return mj == 12 ? G1MB2K : 1 << 30; }();
                if (Kp >= mb2k) {
                    static thread_local bool a2b = false;
                    if (!a2b) {
                        for (auto f : {(const void*)gemm_q1<0, 128, 64, 32, 3, 2>, (const void*)gemm_q1<1, 128, 64, 32, 3, 2>, (const void*)gemm_q1<2, 128, 64, 32, 3, 2>, (const void*)gemm_q1<3, 128, 64, 32, 3, 2>, (const void*)gemm_q1<4, 128, 64, 32, 3, 2>})
                            CK(cudaFuncSetAttribute(f, cudaFuncAttributeMaxDynamicSharedMemorySize, sm64));
                        a2b = true;
                    }
                    if (emode == 0) gemm_q1<0, 128, 64, 32, 3, 2><<<g6, 256, sm64>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi, HQ{}, rmp, cmp, p1);
                    else if (emode == 1) gemm_q1<1, 128, 64, 32, 3, 2><<<g6, 256, sm64>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi, HQ{}, rmp, cmp, p1);
                    else if (emode == 2) gemm_q1<2, 128, 64, 32, 3, 2><<<g6, 256, sm64>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi, HQ{}, rmp, cmp, p1);
                    else if (emode == 3) gemm_q1<3, 128, 64, 32, 3, 2><<<g6, 256, sm64>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N / 2, res, lut_phi, HQ{}, rmp);
                    else gemm_q1<4, 128, 64, 32, 3, 2><<<g6, 256, sm64>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi, hq);
                    return;
                }
                if (emode == 0) gemm_q1<0, 128, 64, 32><<<g6, 256, sm64>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi, HQ{}, rmp, cmp, p1);
                else if (emode == 1) gemm_q1<1, 128, 64, 32><<<g6, 256, sm64>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi, HQ{}, rmp, cmp, p1);
                else if (emode == 2) gemm_q1<2, 128, 64, 32><<<g6, 256, sm64>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi, HQ{}, rmp, cmp, p1);
                else if (emode == 3) gemm_q1<3, 128, 64, 32><<<g6, 256, sm64>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N / 2, res, lut_phi, HQ{}, rmp);
                else gemm_q1<4, 128, 64, 32><<<g6, 256, sm64>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi, hq);
                return;
            }
            dim3 gr((N + G1N - 1) / G1N, (M + G1M - 1) / G1M);
            if (emode == 0) gemm_q1<0><<<gr, 256, G1S * G1STAGE>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
            else if (emode == 1) gemm_q1<1><<<gr, 256, G1S * G1STAGE>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
            else if (emode == 2) gemm_q1<2><<<gr, 256, G1S * G1STAGE>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi);
            else if (emode == 3) gemm_q1<3><<<gr, 256, G1S * G1STAGE>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N / 2, res, lut_phi);
            else gemm_q1<4><<<gr, 256, G1S * G1STAGE>>>(hi, lo, e, W, mm, ss, bb, M, N, Np, Kp, out, N, res, lut_phi, hq);
        }
        CK(cudaGetLastError());
    }

    Act conv3(const Act& x, int H, int W, const std::string& name, int up = 1, int stride = 1, const int* res = nullptr) {
        int N, K, Kp;
        const int8_t* Wt = weight(name, x.C, N, K, Kp);
        int Hu = H * up, Wu = W * up, Ho = (Hu - 1) / stride + 1, Wo = (Wu - 1) / stride + 1, P = Ho * Wo * B;
        static int iv = getenv(nullptr) ? envnum(getenv(nullptr)) : 0;
        if (iv && x.C % 64 == 0) {
            static thread_local bool attr = false;
            if (!attr) {
                for (auto f : {(const void*)conv_q<0>, (const void*)conv_q<1>})
                    CK(cudaFuncSetAttribute(f, cudaFuncAttributeMaxDynamicSharedMemorySize, CV_SMEM));
                for (auto f : {(const void*)conv_q2<0>, (const void*)conv_q2<1>})
                    CK(cudaFuncSetAttribute(f, cudaFuncAttributeMaxDynamicSharedMemorySize, CV2_SMEM));
                attr = true;
            }
            unsigned* pm; int* e;
            CK(cudaMallocAsync((void**)&pm, 4ull * x.P, 0)); CK(cudaMallocAsync((void**)&e, 4ull * P, 0));
            pix_absmax<<<x.P, 128>>>(x.p, x.P, x.C, pm);
            conv_e<<<(P + 255) / 256, 256>>>(pm, B, H, W, up, stride, Ho, Wo, e);
            Act y = act(P, N);
            dim3 gr((N + G1N - 1) / G1N, (P + G1M - 1) / G1M);
            const int Np = (N + 63) / 64 * 64;
            const int *mm = i32(name + ".m"), *ss = i32(name + ".s"), *bb = i32(name + ".b");
            if (iv == 2 && res) conv_q2<1><<<gr, 256, CV2_SMEM>>>(x.p, e, H, W, x.C, up, stride, Ho, Wo, Wt, mm, ss, bb, P, N, Np, Kp, y.p, N, res, lut_phi);
            else if (iv == 2) conv_q2<0><<<gr, 256, CV2_SMEM>>>(x.p, e, H, W, x.C, up, stride, Ho, Wo, Wt, mm, ss, bb, P, N, Np, Kp, y.p, N, res, lut_phi);
            else if (res) conv_q<1><<<gr, 256, CV_SMEM>>>(x.p, e, H, W, x.C, up, stride, Ho, Wo, Wt, mm, ss, bb, P, N, Np, Kp, y.p, N, res, lut_phi);
            else conv_q<0><<<gr, 256, CV_SMEM>>>(x.p, e, H, W, x.C, up, stride, Ho, Wo, Wt, mm, ss, bb, P, N, Np, Kp, y.p, N, res, lut_phi);
            CK(cudaGetLastError());
            cudaFreeAsync(pm, 0); cudaFreeAsync(e, 0);
            verify("lin", name, y, false);
            verify("conv", name, y, true);
            return y;
        }
        unsigned* pm;
        auto pc = pcache.find(x.p);
        const bool havepm = pc != pcache.end() && rc.count(x.p);
        if (havepm) pm = pc->second;
        else {
            CK(cudaMallocAsync((void**)&pm, 4ull * x.P, 0));
            pix_absmax<<<x.P, 128>>>(x.p, x.P, x.C, pm);
        }
        int8_t* hi; uint8_t* lo; int* e;
        CK(cudaMallocAsync((void**)&hi, (size_t)P * Kp, 0)); CK(cudaMallocAsync((void**)&lo, (size_t)P * Kp, 0)); CK(cudaMallocAsync((void**)&e, 4ull * P, 0));
        static int i2c = getenv(nullptr) ? envnum(getenv(nullptr)) : 4;
        if (i2c == 4 && x.C % 4 == 0) {
            conv_e<<<(P + 255) / 256, 256>>>(pm, B, H, W, up, stride, Ho, Wo, e);
            im2col4<<<P, x.C >= 512 ? 128 : 96>>>(x.p, e, H, W, x.C, up, stride, Ho, Wo, Kp, hi, lo);
        } else
        im2col3<<<P, 256>>>(x.p, pm, H, W, x.C, up, stride, Ho, Wo, Kp, hi, lo, e);
        Act y = act(P, N);
        gemm(hi, lo, e, Wt, name, P, N, Kp, y.p, res ? 1 : 0, res);
        if (!havepm) cudaFreeAsync(pm, 0);
        cudaFreeAsync(hi, 0); cudaFreeAsync(lo, 0); cudaFreeAsync(e, 0);
        verify("lin", name, y, false);
        verify("conv", name, y, true);
        return y;
    }
    Act conv1(const Act& x, const std::string& name) {
        Act y = lin(x, name);
        verify("conv", name, y, true);
        return y;
    }

    Act norm_q(const Act& x, const std::string& name) {
        if (!fuse || (x.C != 320 && x.C != 640 && x.C != 1280)) return norm(x, name, false);
        int den = ((const int*)t.at(name + ".eps").data)[0];
        Act y = act(1, 1);
        y.P = x.P; y.C = x.C;
        QB qb; qb.Kp = x.C;
        CK(cudaMallocAsync((void**)&qb.hi, (size_t)x.P * x.C, 0)); CK(cudaMallocAsync((void**)&qb.lo, (size_t)x.P * x.C, 0)); CK(cudaMallocAsync((void**)&qb.e, 4ull * x.P, 0));
        const int* g = i32(name + ".g"); const int* b = i32(name + ".beta");
        unsigned gr = (x.P + 7) / 8;
        if (x.C == 320) ln_warp<10, true><<<gr, 256>>>(x.p, x.P, g, b, den, nullptr, qb.hi, qb.lo, qb.e);
        else if (x.C == 640) ln_warp<20, true><<<gr, 256>>>(x.p, x.P, g, b, den, nullptr, qb.hi, qb.lo, qb.e);
        else ln_warp<40, true><<<gr, 256>>>(x.p, x.P, g, b, den, nullptr, qb.hi, qb.lo, qb.e);
        qcache[y.p] = qb;
        return y;
    }

    Act gnorm_q(const Act& x, const std::string& name) {
        if (!fuse || (x.C != 320 && x.C != 640 && x.C != 1280)) return norm(x, name, true);
        int den = ((const int*)t.at(name + ".eps").data)[0];
        const int P1 = x.P / B, cg = x.C / 32;
        GnSt* st;
        CK(cudaMallocAsync((void**)&st, sizeof(GnSt) * 32 * B, 0));
        gn_init<<<1, 32 * B>>>(st, 32 * B);
        dim3 gr((P1 + 31) / 32, B);
        gn_pass<0><<<gr, 256>>>(x.p, P1, x.C, cg, st);
        gn_pass<2><<<gr, 256>>>(x.p, P1, x.C, cg, st);
        Act y = act(1, 1);
        y.P = x.P; y.C = x.C;
        QB qb; qb.Kp = x.C;
        CK(cudaMallocAsync((void**)&qb.hi, (size_t)x.P * x.C, 0)); CK(cudaMallocAsync((void**)&qb.lo, (size_t)x.P * x.C, 0)); CK(cudaMallocAsync((void**)&qb.e, 4ull * x.P, 0));
        const int* g = i32(name + ".g"); const int* b = i32(name + ".beta");
        const unsigned blocks = (x.P + 7) / 8;
        if (x.C == 320) gn_rows_q<10><<<blocks, 256>>>(x.p, P1, cg, st, den, g, b, x.P, qb.hi, qb.lo, qb.e);
        else if (x.C == 640) gn_rows_q<20><<<blocks, 256>>>(x.p, P1, cg, st, den, g, b, x.P, qb.hi, qb.lo, qb.e);
        else gn_rows_q<40><<<blocks, 256>>>(x.p, P1, cg, st, den, g, b, x.P, qb.hi, qb.lo, qb.e);
        cudaFreeAsync(st, 0);
        qcache[y.p] = qb;
        return y;
    }
    Act norm(const Act& x, const std::string& name, bool group, bool silu = false) {
        Act y = act(x.P, x.C);
        int den = ((const int*)t.at(name + ".eps").data)[0];
        static int gnv = getenv(nullptr) ? envnum(getenv(nullptr)) : 1;
        if (group && gnv) {
            const int P1 = x.P / B, cg = x.C / 32;
            if (((i64)P1 * x.C) % 2048) { fprintf(stderr, "vela engine: model shape error (E12)\n"); exit(1); }
            GnSt* st;
            CK(cudaMallocAsync((void**)&st, sizeof(GnSt) * 32 * B, 0));
            gn_init<<<1, 32 * B>>>(st, 32 * B);
            dim3 gr((P1 + 31) / 32, B);
            gn_pass<0><<<gr, 256>>>(x.p, P1, x.C, cg, st);
            gn_pass<2><<<gr, 256>>>(x.p, P1, x.C, cg, st);
            i64 total = (i64)x.P * x.C;
            unsigned* pm = nullptr;
            if (silu && x.C >= 160) {
                CK(cudaMallocAsync((void**)&pm, 4ull * x.P, 0));
                CK(cudaMemsetAsync(pm, 0, 4ull * x.P, 0));
                pcache[y.p] = pm;
            }
            static int ga2 = getenv(nullptr) ? envnum(getenv(nullptr)) : 1;
            if (ga2 && P1 % 8 == 0) gn_apply2<<<(unsigned)(x.P / 8), 256>>>(x.p, P1, x.C, cg, st, den, i32(name + ".g"), i32(name + ".beta"), y.p, silu ? lut_sig : nullptr, pm);
            else gn_apply<<<(unsigned)((total + 2047) / 2048), 256>>>(x.p, P1, x.C, cg, st, den, i32(name + ".g"), i32(name + ".beta"), y.p, total, silu ? lut_sig : nullptr, pm);
            cudaFreeAsync(st, 0);
        } else if (group) norm_k<<<dim3(32, B), 1024>>>(x.p, x.P / B, x.C, x.C / 32, 0, i32(name + ".g"), i32(name + ".beta"), den, y.p);
        else if (x.C == 320) ln_warp<10><<<(x.P + 7) / 8, 256>>>(x.p, x.P, i32(name + ".g"), i32(name + ".beta"), den, y.p);
        else if (x.C == 640) ln_warp<20><<<(x.P + 7) / 8, 256>>>(x.p, x.P, i32(name + ".g"), i32(name + ".beta"), den, y.p);
        else if (x.C == 1280) ln_warp<40><<<(x.P + 7) / 8, 256>>>(x.p, x.P, i32(name + ".g"), i32(name + ".beta"), den, y.p);
        else if (x.C == 1024) ln_warp<32><<<(x.P + 7) / 8, 256>>>(x.p, x.P, i32(name + ".g"), i32(name + ".beta"), den, y.p);
        else norm_k<<<x.P, 256>>>(x.p, x.P, x.C, 0, 1, i32(name + ".g"), i32(name + ".beta"), den, y.p);
        CK(cudaGetLastError());
        if (!silu) verify("norm", name, y, group);
        return y;
    }
    Act ew(const Act& x, const Act* x2, int mode, const char* kind, bool chw) {
        Act y = act(x.P, x.C);
        i64 n = (i64)x.P * x.C;
        ewise<<<(unsigned)((n + 255) / 256), 256>>>(x.p, x2 ? x2->p : nullptr, n, mode, mode == 0 ? lut_sig : lut_phi, y.p);
        if (kind) verify(kind, "", y, chw);
        return y;
    }
    Act add(const Act& a, const Act& b) { return ew(a, &b, 3, nullptr, false); }

    HQ linq(const Act& x, const std::string& name, int Tp) {
        int N, K, Kp;
        const int8_t* W = weight(name, 0, N, K, Kp);
        const int H = N / 64;
        HQ hq; hq.T = Tp;
        CK(cudaMallocAsync((void**)&hq.hi, (size_t)H * Tp * 64, 0)); CK(cudaMallocAsync((void**)&hq.lo, (size_t)H * Tp * 64, 0)); CK(cudaMallocAsync((void**)&hq.e, 4ull * H * Tp, 0));
        if (Tp > x.P) { CK(cudaMemsetAsync(hq.hi, 0, (size_t)H * Tp * 64, 0)); CK(cudaMemsetAsync(hq.lo, 0, (size_t)H * Tp * 64, 0)); CK(cudaMemsetAsync(hq.e, 0, 4ull * H * Tp, 0)); }
        QB qb;
        auto it = qcache.find(x.p);
        bool cached = it != qcache.end() && rc.count(x.p);
        if (cached) qb = it->second;
        else {
            qb.Kp = Kp;
            CK(cudaMallocAsync((void**)&qb.hi, (size_t)x.P * Kp, 0)); CK(cudaMallocAsync((void**)&qb.lo, (size_t)x.P * Kp, 0)); CK(cudaMallocAsync((void**)&qb.e, 4ull * x.P, 0));
            quant_rows<<<x.P, 256>>>(x.p, x.P, K, K, Kp, qb.hi, qb.lo, qb.e);
            if (rc.count(x.p)) qcache[x.p] = qb;
        }
        gemm(qb.hi, qb.lo, qb.e, W, name, x.P, N, Kp, nullptr, 4, nullptr, hq);
        if (!rc.count(x.p)) { cudaFreeAsync(qb.hi, 0); cudaFreeAsync(qb.lo, 0); cudaFreeAsync(qb.e, 0); }
        return hq;
    }
    void freehq(HQ& h) { cudaFreeAsync(h.hi, 0); cudaFreeAsync(h.lo, 0); cudaFreeAsync(h.e, 0); }

    Act attention_q(const HQ& q, const HQ& k, const Act& v, int Tq, int Tk1, bool shared, int C) {
        const int H = C / 64, Tq1 = Tq / B, Tkp1 = (Tk1 + 63) / 64 * 64, Bk = shared ? 1 : B, Tkp = Bk * Tkp1;
        int8_t *vh, *vhp; uint8_t *vl, *vlp; int *ev, *mx; unsigned long long* rr;
        CK(cudaMallocAsync((void**)&vh, (size_t)H * 64 * Tkp, 0)); CK(cudaMallocAsync((void**)&vl, (size_t)H * 64 * Tkp, 0)); CK(cudaMallocAsync((void**)&ev, 4ull * Bk * H * 64, 0));
        CK(cudaMallocAsync((void**)&vhp, (size_t)H * 64 * Tkp, 0)); CK(cudaMallocAsync((void**)&vlp, (size_t)H * 64 * Tkp, 0));
        CK(cudaMallocAsync((void**)&mx, 4ull * H * Tq, 0)); CK(cudaMallocAsync((void**)&rr, 8ull * H * Tq, 0));
        auto cq = cmcache.find(v.p);
        if (cq != cmcache.end()) prep_vq<<<dim3(Tkp1 / 64, H, Bk), 256>>>(v.p, cq->second, Tk1, Tkp1, Bk, C, vhp, vlp, ev);
        else prep_vbp<<<dim3(H * 64, Bk), 256>>>(v.p, Tk1, Tkp1, Bk, C, vhp, vlp, ev);
        Act o = act(Tq, C);
        unsigned* rmo;
        CK(cudaMallocAsync((void**)&rmo, 4ull * Tq, 0));
        CK(cudaMemsetAsync(rmo, 0, 4ull * Tq, 0));
        rmcache[o.p] = rmo;
        const int ks = shared ? 1 : 0;
        static int av = getenv(nullptr) ? envnum(getenv(nullptr)) : 4;
        if (av == 4) {
            if (Tq1 % 128 == 0) attn_k4<0, 8><<<dim3(Tq1 / 128, H * B), 256, k4smem<0>()>>>(q.hi, q.lo, q.e, k.hi, k.lo, k.e, vhp, vlp, ev, lut4, Tq, Tk1, Tkp, C, o.p, mx, rr, Tq1, Tkp1, H, ks);
            else attn_k4<0, 4><<<dim3(Tq1 / 64, H * B), 128, k4smem<0>()>>>(q.hi, q.lo, q.e, k.hi, k.lo, k.e, vhp, vlp, ev, lut4, Tq, Tk1, Tkp, C, o.p, mx, rr, Tq1, Tkp1, H, ks);
            static int pv5 = getenv(nullptr) ? envnum(getenv(nullptr)) : 0;
            if (pv5) {
                attn_pv5<<<dim3(Tq1 / 64, H * B), 128>>>(q.hi, q.lo, q.e, k.hi, k.lo, k.e, vhp, vlp, ev, i32("lut.exp"), Tq, Tk1, Tkp, C, o.p, mx, rr, Tq1, Tkp1, H, ks, rmo);
                attn_k3<1, 4><<<dim3(Tq1 / 64, H * B), 128>>>(q.hi, q.lo, q.e, k.hi, k.lo, k.e, vhp, vlp, ev, lut4, Tq, Tk1, Tkp, C, o.p, mx, rr, Tq1, Tkp1, H, ks, rmo, 1);
            } else
            attn_k3<1, 4><<<dim3(Tq1 / 64, H * B), 128>>>(q.hi, q.lo, q.e, k.hi, k.lo, k.e, vhp, vlp, ev, lut4, Tq, Tk1, Tkp, C, o.p, mx, rr, Tq1, Tkp1, H, ks, rmo);
        } else {
        if (Tq1 % 128 == 0) attn_k3<0, 8><<<dim3(Tq1 / 128, H * B), 256>>>(q.hi, q.lo, q.e, k.hi, k.lo, k.e, vhp, vlp, ev, lut4, Tq, Tk1, Tkp, C, o.p, mx, rr, Tq1, Tkp1, H, ks);
        else attn_k3<0, 4><<<dim3(Tq1 / 64, H * B), 128>>>(q.hi, q.lo, q.e, k.hi, k.lo, k.e, vhp, vlp, ev, lut4, Tq, Tk1, Tkp, C, o.p, mx, rr, Tq1, Tkp1, H, ks);
        attn_k3<1, 4><<<dim3(Tq1 / 64, H * B), 128>>>(q.hi, q.lo, q.e, k.hi, k.lo, k.e, vhp, vlp, ev, lut4, Tq, Tk1, Tkp, C, o.p, mx, rr, Tq1, Tkp1, H, ks);
        }
        CK(cudaGetLastError());
        for (void* p : {(void*)vh, (void*)vl, (void*)ev, (void*)vhp, (void*)vlp, (void*)mx, (void*)rr}) cudaFreeAsync(p, 0);
        return o;
    }
    Act attention(const Act& q, const Act& k, const Act& v) {
        const bool shared = k.P == 77 && B > 1;
        const int C = q.C, H = C / 64, Tq1 = q.P / B, Tk1 = shared ? k.P : k.P / B, Tkp1 = (Tk1 + 63) / 64 * 64;
        const int Bk = shared ? 1 : B, Tq = q.P, Tkp = Bk * Tkp1;
        if (!shared && B > 1 && Tk1 % 64) { fprintf(stderr, "vela engine: model shape error (E13)\n"); exit(1); }
        int8_t *qh, *kh, *vh, *vhp; uint8_t *ql, *kl, *vl, *vlp; int *eq, *ek, *ev, *mx; unsigned long long* rr;
        CK(cudaMallocAsync((void**)&qh, (size_t)H * Tq * 64, 0)); CK(cudaMallocAsync((void**)&ql, (size_t)H * Tq * 64, 0)); CK(cudaMallocAsync((void**)&eq, 4ull * H * Tq, 0));
        CK(cudaMallocAsync((void**)&kh, (size_t)H * Tkp * 64, 0)); CK(cudaMallocAsync((void**)&kl, (size_t)H * Tkp * 64, 0)); CK(cudaMallocAsync((void**)&ek, 4ull * H * Tkp, 0));
        CK(cudaMallocAsync((void**)&vh, (size_t)H * 64 * Tkp, 0)); CK(cudaMallocAsync((void**)&vl, (size_t)H * 64 * Tkp, 0)); CK(cudaMallocAsync((void**)&ev, 4ull * Bk * H * 64, 0));
        CK(cudaMallocAsync((void**)&vhp, (size_t)H * 64 * Tkp, 0)); CK(cudaMallocAsync((void**)&vlp, (size_t)H * 64 * Tkp, 0));
        CK(cudaMallocAsync((void**)&mx, 4ull * H * Tq, 0)); CK(cudaMallocAsync((void**)&rr, 8ull * H * Tq, 0));
        prep_rows<<<dim3(Tq, H), 64>>>(q.p, Tq, Tq, C, qh, ql, eq);
        prep_rows<<<dim3(Tkp, H), 64>>>(k.p, Bk * Tk1, Tkp, C, kh, kl, ek);
        prep_vb<<<dim3(H * 64, Bk), 256>>>(v.p, Tk1, Tkp1, Bk, C, vh, vl, ev);
        permute_v<<<(H * 64 * Tkp + 255) / 256, 256>>>(vh, vl, Tkp, vhp, vlp, H * 64 * Tkp);
        Act o = act(Tq, C);
        const int ks = shared ? 1 : 0;
        if (Tq1 % 128 == 0) attn_k3<0, 8><<<dim3(Tq1 / 128, H * B), 256>>>(qh, ql, eq, kh, kl, ek, vhp, vlp, ev, lut4, Tq, Tk1, Tkp, C, o.p, mx, rr, Tq1, Tkp1, H, ks);
        else attn_k3<0, 4><<<dim3(Tq1 / 64, H * B), 128>>>(qh, ql, eq, kh, kl, ek, vhp, vlp, ev, lut4, Tq, Tk1, Tkp, C, o.p, mx, rr, Tq1, Tkp1, H, ks);
        attn_k3<1, 4><<<dim3(Tq1 / 64, H * B), 128>>>(qh, ql, eq, kh, kl, ek, vhp, vlp, ev, lut4, Tq, Tk1, Tkp, C, o.p, mx, rr, Tq1, Tkp1, H, ks);
        CK(cudaGetLastError());
        for (void* p : {(void*)qh, (void*)ql, (void*)eq, (void*)kh, (void*)kl, (void*)ek, (void*)vh, (void*)vl, (void*)ev, (void*)vhp, (void*)vlp, (void*)mx, (void*)rr}) cudaFreeAsync(p, 0);
        verify("attn", std::to_string(Tq1) + "x" + std::to_string(Tk1), o, false);
        return o;
    }
    Act normsilu(const Act& x, const std::string& name) {
        if (fuse) return norm(x, name, true, true);
        Act n = norm(x, name, true), h = ew(n, nullptr, 0, "silu", true);
        drop(n);
        return h;
    }
    Act resnet(const Act& x, int H, int W, const std::string& p) {
        Act h = normsilu(x, p + ".norm1");
        Act c = conv3(h, H, W, p + ".conv1");
        drop(h);
        h = normsilu(c, p + ".norm2"); drop(c);
        bool sc = t.count(p + ".conv_shortcut.w");
        Act xs = x, y;
        if (fuse) { if (sc) xs = conv1(x, p + ".conv_shortcut"); y = conv3(h, H, W, p + ".conv2", 1, 1, xs.p); }
        else { c = conv3(h, H, W, p + ".conv2"); if (sc) xs = conv1(x, p + ".conv_shortcut"); y = add(xs, c); drop(c); }
        drop(h);
        if (sc) drop(xs);
        verify("resnet", p, y, true);
        return y;
    }
    std::map<std::string, std::pair<Act, Act>> kv;
    std::map<std::string, HQ> kvq;
    Act transformer(const Act& x, const std::string& p) {
        Act gn = fuse ? gnorm_q(x, p + ".norm") : norm(x, p + ".norm", true), h = lin(gn, p + ".proj_in");
        drop(gn);
        std::string b = p + ".transformer_blocks.0";
        Act n = norm_q(h, b + ".norm1");
        Act a;
        if (fuse) {
            HQ hq = linq(n, b + ".attn1.to_q", n.P), hk = linq(n, b + ".attn1.to_k", n.P);
            Act v = lin(n, b + ".attn1.to_v", 0, nullptr, false, (n.P / B) % 64 == 0 ? B : 0);
            drop(n);
            a = attention_q(hq, hk, v, n.P, n.P / B, false, n.C);
            freehq(hq); freehq(hk); drop(v);
        } else {
            Act q = lin(n, b + ".attn1.to_q"), k = lin(n, b + ".attn1.to_k"), v = lin(n, b + ".attn1.to_v");
            drop(n);
            a = attention(q, k, v);
            drop(q); drop(k); drop(v);
        }
        Act h2 = fuse ? lin(a, b + ".attn1.to_out.0", 1, h.p) : Act{};
        if (!fuse) { Act o = lin(a, b + ".attn1.to_out.0"); h2 = add(h, o); drop(o); }
        drop(a); drop(h);
        n = norm_q(h2, b + ".norm2");
        auto& c = kv.at(b);
        if (fuse) {
            HQ hq = linq(n, b + ".attn2.to_q", n.P);
            drop(n);
            a = attention_q(hq, kvq.at(b), c.second, n.P, 77, B > 1, n.C);
            freehq(hq);
        } else {
            Act q = lin(n, b + ".attn2.to_q"); drop(n);
            a = attention(q, c.first, c.second); drop(q);
        }
        if (fuse) h = lin(a, b + ".attn2.to_out.0", 1, h2.p);
        else { Act o = lin(a, b + ".attn2.to_out.0"); h = add(h2, o); drop(o); }
        drop(a); drop(h2);
        n = fuse ? norm_q(h, b + ".norm3") : norm(h, b + ".norm3", false);
        Act A{}, ag;
        if (fuse) { make_ag(b + ".ff.net.0.proj"); ag = lin(n, b + ".ff.net.0.proj.ag", 3, nullptr, true); }
        else {
            A = lin(n, b + ".ff.net.0.proj.a");
            Act G = lin(n, b + ".ff.net.0.proj.g");
            if (check) drop(ew(G, nullptr, 1, "gelu", false));
            ag = ew(A, &G, 2, nullptr, false);
            drop(G);
        }
        drop(n);
        if (!fuse) drop(A);
        if (fuse) h2 = lin(ag, b + ".ff.net.2", 1, h.p, true);
        else { Act o = lin(ag, b + ".ff.net.2"); h2 = add(h, o); drop(o); }
        drop(ag); drop(h);
        Act y;
        if (fuse) y = lin(h2, p + ".proj_out", 1, x.p);
        else { Act o = lin(h2, p + ".proj_out"); y = add(x, o); drop(o); }
        drop(h2);
        verify("transformer", p, y, true);
        return y;
    }

    Act text(const std::vector<int>& ids) {
        int* dids;
        CK(cudaMallocAsync((void**)&dids, 4 * ids.size(), 0));
        CK(cudaMemcpyAsync(dids, ids.data(), 4 * ids.size(), cudaMemcpyHostToDevice, 0));
        const int T = (int)ids.size(), D = 1024;
        Act x = act(T, D);
        temb_k<<<T, 256>>>(dids, (const int8_t*)up<int8_t>("te.tok.w", t.at("te.tok.w").data, t.at("te.tok.w").n), i32("te.tok.m"), i32("te.tok.s"), i32("te.pos"), D, x.p);
        const bool f = fuse;
        fuse = false;
        for (int l = 0; l < 23; l++) {
            const std::string p = "te.layers." + std::to_string(l);
            Act n = norm(x, p + ".layer_norm1", false);
            Act qa = lin(n, p + ".self_attn.q_proj"), ka = lin(n, p + ".self_attn.k_proj"), va = lin(n, p + ".self_attn.v_proj");
            drop(n);
            Act a = act(T, D);
            attn_naive<<<dim3(T, 16), 64>>>(qa.p, ka.p, va.p, T, D, 1, i32("lut.exp"), a.p);
            drop(qa); drop(ka); drop(va);
            Act o = lin(a, p + ".self_attn.out_proj"); drop(a);
            Act x2 = add(x, o); drop(x); drop(o); x = x2;
            n = norm(x, p + ".layer_norm2", false);
            Act h1 = lin(n, p + ".mlp.fc1"); drop(n);
            Act g = ew(h1, nullptr, 1, nullptr, false); drop(h1);
            Act h2 = lin(g, p + ".mlp.fc2"); drop(g);
            x2 = add(x, h2); drop(x); drop(h2); x = x2;
        }
        Act c = norm(x, "te.final_layer_norm", false);
        drop(x);
        fuse = f;
        cudaFreeAsync(dids, 0);
        return c;
    }
    Act unet(const Act& x, const Act& ctx) {
        for (auto& it : t) {
            const std::string& n = it.first;
            const std::string suf = ".attn2.to_k.w";
            if (n.rfind("unet.", 0) == 0 && n.size() > suf.size() && n.compare(n.size() - suf.size(), suf.size(), suf) == 0) {
                std::string b = n.substr(0, n.size() - suf.size());
                Act k = fuse ? Act{} : lin(ctx, b + ".attn2.to_k");
                if (fuse) kvq[b] = linq(ctx, b + ".attn2.to_k", 128);
                Act v = lin(ctx, b + ".attn2.to_v", 0, nullptr, false, fuse ? 1 : 0);
                kv[b] = {k, v};
            }
        }
        int H = 64, W = 64;
        Act h = conv3(x, H, W, "unet.conv_in");
        std::vector<Act> skips{h};
        keep(h);
        auto step = [&](Act nh) { drop(h); h = nh; };
        for (int i = 0; i < 4; i++) {
            for (int j = 0; j < 2; j++) {
                step(resnet(h, H, W, "unet.down_blocks." + std::to_string(i) + ".resnets." + std::to_string(j)));
                if (i < 3) step(transformer(h, "unet.down_blocks." + std::to_string(i) + ".attentions." + std::to_string(j)));
                skips.push_back(h); keep(h);
            }
            if (i < 3) {
                step(conv3(h, H, W, "unet.down_blocks." + std::to_string(i) + ".downsamplers.0.conv", 1, 2));
                H /= 2; W /= 2;
                skips.push_back(h); keep(h);
            }
        }
        step(resnet(h, H, W, "unet.mid_block.resnets.0"));
        step(transformer(h, "unet.mid_block.attentions.0"));
        step(resnet(h, H, W, "unet.mid_block.resnets.1"));
        for (int i = 0; i < 4; i++) {
            for (int j = 0; j < 3; j++) {
                Act s = skips.back(); skips.pop_back();
                Act cat = act(h.P, h.C + s.C);
                i64 n = (i64)h.P * cat.C;
                concat_k<<<(unsigned)((n + 255) / 256), 256>>>(h.p, h.C, s.p, s.C, h.P, cat.p);
                drop(s);
                step(cat);
                step(resnet(h, H, W, "unet.up_blocks." + std::to_string(i) + ".resnets." + std::to_string(j)));
                if (i > 0) step(transformer(h, "unet.up_blocks." + std::to_string(i) + ".attentions." + std::to_string(j)));
            }
            if (i < 3) {
                step(conv3(h, H, W, "unet.up_blocks." + std::to_string(i) + ".upsamplers.0.conv", 2, 1));
                H *= 2; W *= 2;
            }
        }
        Act sl = normsilu(h, "unet.conv_norm_out");
        drop(h);
        Act eps = conv3(sl, H, W, "unet.conv_out");
        drop(sl);
        for (auto& it : kv) { if (!fuse) drop(it.second.first); drop(it.second.second); }
        kv.clear();
        for (auto& it : kvq) freehq(it.second);
        kvq.clear();
        return eps;
    }

};

static std::vector<int> readall(const std::string& p, size_t n) {
    std::vector<int> v(n);
    FILE* f = fopen(p.c_str(), "rb");
    if (!f || fread(v.data(), 4, n, f) != n) { fprintf(stderr, "não li %s\n", p.c_str()); exit(1); }
    fclose(f);
    return v;
}


