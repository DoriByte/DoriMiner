// DoriMiner - gera17.cu
// Copyright (C) 2026 J. Costa (DoriByte). Licensed under the PolyForm Noncommercial License 1.0.0 with the additional
// permission in MINING-PERMISSION (see LICENSE and MINING-PERMISSION at the top of this tree).

#include <type_traits>

#include <cstdint>
#define WARPS 8
#define MMA_ASM asm volatile

#define MMA_TYPES "s8.s8"
#define MMA_K32(c,a,b) MMA_ASM("mma.sync.aligned.m16n8k32.row.col.s32." MMA_TYPES ".s32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};" \
  : "+r"(c[0]), "+r"(c[1]), "+r"(c[2]), "+r"(c[3]) : "r"(a.w[0]), "r"(a.w[1]), "r"(a.w[2]), "r"(a.w[3]), "r"(b[0]), "r"(b[1]))
__device__ __forceinline__ uint32_t rotr(uint32_t x, int n) { return (x >> n) | (x << (32 - n)); }
#define G(a,b,c,d,x,y) a = a + b + x; d = rotr(d ^ a, 16); c = c + d; b = rotr(b ^ c, 12); a = a + b + y; d = rotr(d ^ a, 8); c = c + d; b = rotr(b ^ c, 7);
#define PAIRQ (NA * 32)
#define BSTQ (PST * PAIRQ)

#define PSTQ (32 * PST)
#define OSTEP 256u
#define STAGEQ (BSTQ + PSTQ)
#define BARQ ((2 * RING * 8 + 15) / 16)
#define UTQ 256
struct alignas(16) Quad { uint32_t w[4]; };

__device__ __forceinline__ void transpose4(const uint32_t d[4], uint32_t w[4]) {
  const uint32_t t0 = __byte_perm(d[0], d[1], 0x5140), t1 = __byte_perm(d[0], d[1], 0x7362);
  const uint32_t t2 = __byte_perm(d[2], d[3], 0x5140), t3 = __byte_perm(d[2], d[3], 0x7362);
  w[0] = __byte_perm(t0, t2, 0x5410); w[1] = __byte_perm(t0, t2, 0x7632);
  w[2] = __byte_perm(t1, t3, 0x5410); w[3] = __byte_perm(t1, t3, 0x7632);
}

__device__ __forceinline__ void meia(const uint32_t* ut, uint32_t qp, uint32_t qm, uint32_t w[4]) {
  uint32_t d[4];
  #pragma unroll
  for (int j = 0; j < 4; ++j) {
    const uint32_t p = __byte_perm(qp, 0, 0x4440 + j), mm = __byte_perm(qm, 0, 0x4440 + j);
    d[j] = (ut[p * 8] - ut[mm * 8] + 0x80808080u) ^ 0x80808080u;
  }
  transpose4(d, w);
}

__device__ __forceinline__ void junta(const uint32_t l[4], const uint32_t h[4], Quad& av, Quad& aw) {
  av.w[0] = l[0]; av.w[1] = l[1]; av.w[2] = h[0]; av.w[3] = h[1];
  aw.w[0] = l[2]; aw.w[1] = l[3]; aw.w[2] = h[2]; aw.w[3] = h[3];
}

__device__ __forceinline__ void gera(const uint32_t* ut, const Quad q, Quad& av, Quad& aw) {
  uint32_t l[4], h[4];
  meia(ut, q.w[0], q.w[2], l);
  meia(ut, q.w[1], q.w[3], h);
  junta(l, h, av, aw);
}

__device__ __forceinline__ void poke(Quad& av, Quad& aw, int slot, int jb, int v) {
  const uint32_t add = (uint32_t)v << (8 * jb);
  #pragma unroll
  for (int s = 0; s < 8; ++s) {
    uint32_t& x = s < 4 ? av.w[s] : aw.w[s - 4];
    if (s == slot) x = ((x ^ 0x80808080u) + add) ^ 0x80808080u;
  }
}

__device__ __forceinline__ void finish_tile(uint32_t* tr, const uint32_t* key, const uint32_t* target, uint32_t* hits,
                                            uint32_t* dump, int band, int strip, int ncol, int lane, int row, int t, int col0) {
  static const int perm[16] = {2,6,3,10,7,0,4,13,1,11,12,5,9,14,15,8};
  (void)dump; (void)ncol; (void)strip;
  uint32_t v[16];
  #pragma unroll
  for (int i = 0; i < 8; ++i) v[i] = key[i];
  v[8] = 0x6A09E667u; v[9] = 0xBB67AE85u; v[10] = 0x3C6EF372u; v[11] = 0xA54FF53Au; v[12] = 0; v[13] = 0; v[14] = 64; v[15] = 0x1B;
  #pragma unroll
  for (int rr = 0; rr < 7; ++rr) {
    G(v[0],v[4],v[8],v[12],tr[0],tr[1])  G(v[1],v[5],v[9],v[13],tr[2],tr[3])
    G(v[2],v[6],v[10],v[14],tr[4],tr[5]) G(v[3],v[7],v[11],v[15],tr[6],tr[7])
    G(v[0],v[5],v[10],v[15],tr[8],tr[9]) G(v[1],v[6],v[11],v[12],tr[10],tr[11])
    G(v[2],v[7],v[8],v[13],tr[12],tr[13]) G(v[3],v[4],v[9],v[14],tr[14],tr[15])
    uint32_t qq[16];
    #pragma unroll
    for (int i = 0; i < 16; ++i) qq[i] = tr[perm[i]];
    #pragma unroll
    for (int i = 0; i < 16; ++i) tr[i] = qq[i];
  }
  bool win = true;
  for (int i = 7; i >= 0; --i) { const uint32_t hh = v[i] ^ v[i + 8], z = target[i]; if (hh != z) { win = hh < z; break; } }
  if (win) { const uint32_t s = atomicAdd(hits, 1u); if (s < 63) { hits[1 + s] = (uint32_t)row << 16 | (uint32_t)(col0 + 2 * t); } }
}

__device__ __forceinline__ void carrega_u(uint32_t dst, const Quad* U, int b, int lane) {
  const Quad* src = U + (size_t)b * UTQ + lane;
  #pragma unroll
  for (int q = 0; q < UTQ / 32; ++q)
    asm volatile("cp.async.cg.shared.global [%0], [%1], 16;" :: "r"(dst + (uint32_t)(q * 32 + lane) * 16), "l"(src + q * 32) : "memory");
  asm volatile("cp.async.commit_group;" ::: "memory");
}

__device__ __forceinline__ void mb_wait(uint32_t bar, uint32_t parity) {
  asm volatile("{ .reg .pred p; W: mbarrier.try_wait.parity.shared::cta.b64 p, [%0], %1; @!p bra W; }" :: "r"(bar), "r"(parity) : "memory");
}

__device__ __forceinline__ void mb_wait_c(uint32_t bar, uint32_t parity) {
  mb_wait(bar, parity);
}
__device__ __forceinline__ bool mb_test(uint32_t bar, uint32_t parity) {
  uint32_t ok;
  asm volatile("{ .reg .pred p; mbarrier.test_wait.parity.shared::cta.b64 p, [%1], %2; selp.u32 %0, 1, 0, p; }" : "=r"(ok) : "r"(bar), "r"(parity) : "memory");
  return ok != 0;
}

__device__ __forceinline__ void copia_em(uint32_t ring0, uint32_t full0, const Quad* bsrc, const Quad* psrc, uint32_t v) {
  const uint32_t slot = v % RING, bar = full0 + 8 * slot;
  asm volatile("mbarrier.arrive.expect_tx.shared::cta.b64 _, [%0], %1;" :: "r"(bar), "n"(STAGEQ * 16) : "memory");
  asm volatile("cp.async.bulk.shared::cluster.global.mbarrier::complete_tx::bytes [%0], [%1], %3, [%2];"
               :: "r"(ring0 + slot * STAGEQ * 16), "l"(bsrc), "r"(bar), "n"(BSTQ * 16) : "memory");
  asm volatile("cp.async.bulk.shared::cluster.global.mbarrier::complete_tx::bytes [%0], [%1], %3, [%2];"
               :: "r"(ring0 + (slot * STAGEQ + BSTQ) * 16), "l"(psrc), "r"(bar), "n"(PSTQ * 16) : "memory");
}
__device__ __forceinline__ void copia(uint32_t ring0, uint32_t full0, const Quad* bstrip, const Quad* pq, uint32_t v, int pairs) {

  const uint32_t slot = v % RING, bar = full0 + 8 * slot, sv = v & (uint32_t)(pairs / PST - 1);
  asm volatile("mbarrier.arrive.expect_tx.shared::cta.b64 _, [%0], %1;" :: "r"(bar), "n"(STAGEQ * 16) : "memory");
  asm volatile("cp.async.bulk.shared::cluster.global.mbarrier::complete_tx::bytes [%0], [%1], %3, [%2];"
               :: "r"(ring0 + slot * STAGEQ * 16), "l"(bstrip + (size_t)sv * STAGEQ), "r"(bar), "n"(STAGEQ * 16) : "memory");
  (void)pq; return;
  const Quad* src = bstrip + (size_t)(PST * sv) * PAIRQ;
  asm volatile("mbarrier.arrive.expect_tx.shared::cta.b64 _, [%0], %1;" :: "r"(bar), "n"(STAGEQ * 16) : "memory");
  asm volatile("cp.async.bulk.shared::cluster.global.mbarrier::complete_tx::bytes [%0], [%1], %3, [%2];"
               :: "r"(ring0 + slot * STAGEQ * 16), "l"(src), "r"(bar), "n"(BSTQ * 16) : "memory");
  asm volatile("cp.async.bulk.shared::cluster.global.mbarrier::complete_tx::bytes [%0], [%1], %3, [%2];"
               :: "r"(ring0 + (slot * STAGEQ + BSTQ) * 16), "l"(pq + (size_t)sv * PSTQ), "r"(bar), "n"(PSTQ * 16) : "memory");
}

__device__ __forceinline__ void copia_u(uint32_t ring0, uint32_t full0, const Quad* bstrip, uint32_t v, int pairs) {
  const uint32_t slot = v % RING, bar = full0 + 8 * slot, sv = v & (uint32_t)(pairs / PST - 1);
  asm volatile("{ .reg .pred p; elect.sync _|p, 0xffffffff;\n\t"
               "@p mbarrier.arrive.expect_tx.shared::cta.b64 _, [%2], %3;\n\t"
               "@p cp.async.bulk.shared::cta.global.mbarrier::complete_tx::bytes [%0], [%1], %3, [%2]; }"
               :: "r"(ring0 + slot * STAGEQ * 16), "l"(bstrip + (size_t)sv * STAGEQ), "r"(bar), "n"(STAGEQ * 16) : "memory");
}

__device__ __forceinline__ void meia6(uint32_t base, const uint32_t op[4], const uint32_t om[4], uint32_t w[4]) {
  uint32_t d[4];
  #pragma unroll
  for (int j = 0; j < 4; ++j) {
    uint32_t a, b;
    asm volatile("ld.shared.u32 %0, [%1];" : "=r"(a) : "r"(base + op[j]));
    asm volatile("ld.shared.u32 %0, [%1];" : "=r"(b) : "r"(base + om[j]));
    d[j] = (a - b + 0x80808080u) ^ 0x80808080u;
  }
  transpose4(d, w);
}
__device__ __forceinline__ void ldm8(uint32_t addr, uint32_t r[4]) {
  asm volatile("ldmatrix.sync.aligned.m16n16.x2.trans.shared.b8 {%0,%1,%2,%3}, [%4];" : "=r"(r[0]), "=r"(r[1]), "=r"(r[2]), "=r"(r[3]) : "r"(addr));
}
__device__ __forceinline__ uint32_t dif(uint32_t a, uint32_t b) { return (a - b + 0x80808080u) ^ 0x80808080u; }

__device__ __forceinline__ void dif2(uint32_t a0, uint32_t a1, uint32_t b0, uint32_t b1, uint32_t& r0, uint32_t& r1) {
  uint64_t a, b;
  asm("mov.b64 %0, {%1, %2};" : "=l"(a) : "r"(a0), "r"(a1));
  asm("mov.b64 %0, {%1, %2};" : "=l"(b) : "r"(b0), "r"(b1));
  const uint64_t d = a - b;
  asm("mov.b64 {%0, %1}, %2;" : "=r"(r0), "=r"(r1) : "l"(d));
  r0 ^= 0x80808080u; r1 ^= 0x80808080u;
}

__device__ __forceinline__ void gera7g(uint32_t hb, uint32_t w, uint32_t half, Quad& av, Quad& aw) {
  uint32_t o0, o1, o2, o3;
  asm("lop3.b32 %0, %1, 0xff0, %2, 0xea;" : "=r"(o0) : "r"(w << 4), "r"(half));
  asm("lop3.b32 %0, %1, 0xff0, %2, 0xea;" : "=r"(o1) : "r"(w >> 4), "r"(half));
  asm("lop3.b32 %0, %1, 0xff0, %2, 0xea;" : "=r"(o2) : "r"(w >> 12), "r"(half));
  asm("lop3.b32 %0, %1, 0xff0, %2, 0xea;" : "=r"(o3) : "r"(w >> 20), "r"(half));
  uint32_t pl[4], ph[4], ml[4], mh[4];
  ldm8(hb + o0, pl); ldm8(hb + o1, ph); ldm8(hb + o2, ml); ldm8(hb + o3, mh);
  av.w[0] = dif(pl[0], ml[0]); av.w[1] = dif(pl[1], ml[1]); av.w[2] = dif(ph[0], mh[0]); av.w[3] = dif(ph[1], mh[1]);
  aw.w[0] = dif(pl[2], ml[2]); aw.w[1] = dif(pl[3], ml[3]); aw.w[2] = dif(ph[2], mh[2]); aw.w[3] = dif(ph[3], mh[3]);
}
__device__ __forceinline__ void gera7(uint32_t hb, uint32_t offs, Quad& av, Quad& aw) {
  uint32_t o0, o1, o2, o3;
  asm volatile("ld.shared.v4.u32 {%0,%1,%2,%3}, [%4];" : "=r"(o0), "=r"(o1), "=r"(o2), "=r"(o3) : "r"(offs));
  uint32_t pl[4], ph[4], ml[4], mh[4];
  ldm8(hb + o0, pl); ldm8(hb + o1, ph); ldm8(hb + o2, ml); ldm8(hb + o3, mh);
  av.w[0] = dif(pl[0], ml[0]); av.w[1] = dif(pl[1], ml[1]); av.w[2] = dif(ph[0], mh[0]); av.w[3] = dif(ph[1], mh[1]);
  aw.w[0] = dif(pl[2], ml[2]); aw.w[1] = dif(pl[3], ml[3]); aw.w[2] = dif(ph[2], mh[2]); aw.w[3] = dif(ph[3], mh[3]);
}

struct Cru { uint32_t pl[4], ph[4], ml[4], mh[4]; };
__device__ __forceinline__ void gera7a(uint32_t hb, uint32_t offs, Cru& r) {
  uint32_t o0, o1, o2, o3;
  asm volatile("ld.shared.v4.u32 {%0,%1,%2,%3}, [%4];" : "=r"(o0), "=r"(o1), "=r"(o2), "=r"(o3) : "r"(offs));
  ldm8(hb + o0, r.pl); ldm8(hb + o1, r.ph); ldm8(hb + o2, r.ml); ldm8(hb + o3, r.mh);
}
__device__ __forceinline__ void gera7b(const Cru& r, Quad& av, Quad& aw) {
  av.w[0] = dif(r.pl[0], r.ml[0]); av.w[1] = dif(r.pl[1], r.ml[1]); av.w[2] = dif(r.ph[0], r.mh[0]); av.w[3] = dif(r.ph[1], r.mh[1]);
  aw.w[0] = dif(r.pl[2], r.ml[2]); aw.w[1] = dif(r.pl[3], r.ml[3]); aw.w[2] = dif(r.ph[2], r.mh[2]); aw.w[3] = dif(r.ph[3], r.mh[3]);
}

#define CLDIMS

__device__ __forceinline__ void dobra8t(uint32_t& x, const int32_t (&a)[4], const int32_t (&b)[4]) {
  asm("{ .reg .b32 t, u2;\n\t"
      "lop3.b32 t, %1, %2, %3, 0x96;\n\t"
      "lop3.b32 u2, %4, %5, %6, 0x96;\n\t"
      "lop3.b32 t, t, u2, %7, 0x96;\n\t"
      "lop3.b32 %0, %0, t, %8, 0x96; }"
      : "+r"(x) : "r"(a[0]), "r"(a[1]), "r"(a[2]), "r"(a[3]), "r"(b[0]), "r"(b[1]), "r"(b[2]), "r"(b[3]));
}

__device__ __forceinline__ void dobra8(uint32_t& x, const int32_t (&a)[4], const int32_t (&b)[4]) {
  asm volatile("{ .reg .b32 t, u2;\n\t"
               "lop3.b32 t, %1, %2, %3, 0x96;\n\t"
               "lop3.b32 u2, %4, %5, %6, 0x96;\n\t"
               "lop3.b32 t, t, u2, %7, 0x96;\n\t"
               "lop3.b32 %0, %0, t, %8, 0x96; }"
               : "+r"(x) : "r"(a[0]), "r"(a[1]), "r"(a[2]), "r"(a[3]), "r"(b[0]), "r"(b[1]), "r"(b[2]), "r"(b[3]));
}

__device__ __forceinline__ uint32_t dobra8v(const int32_t (&a)[4], const int32_t (&b)[4]) {
  uint32_t v;
  asm("{ .reg .b32 t, u2, w;\n\t"
      "lop3.b32 t, %1, %2, %3, 0x96;\n\t"
      "lop3.b32 u2, %4, %5, %6, 0x96;\n\t"
      "lop3.b32 w, t, u2, %7, 0x96;\n\t"
      "xor.b32 %0, w, %8; }"
      : "=r"(v) : "r"(a[0]), "r"(a[1]), "r"(a[2]), "r"(a[3]), "r"(b[0]), "r"(b[1]), "r"(b[2]), "r"(b[3]));
  return v;
}

#define G17BOUNDS __launch_bounds__(32 * WARPS, 1)
extern "C" __global__ void CLDIMS G17BOUNDS
prl_gera17(const uint32_t* Ut, const uint32_t* Bt, const uint32_t* key, const uint32_t* target,
          uint32_t* hits, int n, int k, uint32_t* dump, int m, int rows16, const uint32_t* Pt, uint32_t prc, int pv) {
  extern __shared__ Quad sm[];
  Quad* const ring = sm;
  uint64_t* const bars = (uint64_t*)(sm + RING * STAGEQ);
  Quad* const tab = sm + RING * STAGEQ + BARQ;
  Quad* const trq = tab + WARPS * UTQ;
  const int tid = threadIdx.x, wi = tid >> 5, lane = tid & 31, g = lane >> 2, t = lane & 3;
  const int ncol = n / (8 * NA), pairs = (k >> 7) * 2;
  const int PS = (m >> 16) & 0xffff, PR = m & 0xffff;
  int strip = blockIdx.x % ncol, rb = blockIdx.x / ncol;
  if (PS) { const int i = blockIdx.x % (PR * PS), panel = blockIdx.x / (PR * PS), across = ncol / PS; strip = (panel % across) * PS + i % PS; rb = (panel / across) * PR + i / PS; }
  const int NL = LAPS;
  auto lstrip = [&](int L) { (void)L; return strip; };
  auto lrow = [&](int L) { return rb * LAPS + L; };
  (void)rows16;
  auto bsv = [&](uint32_t v) { return (const Quad*)Bt + (size_t)lstrip((int)(v / (uint32_t)(pairs / PST))) * (pairs / PST) * STAGEQ; };
  const uint32_t ring0 = (uint32_t)__cvta_generic_to_shared(ring);
  const uint32_t full0 = (uint32_t)__cvta_generic_to_shared(bars), empty0 = full0 + 8 * RING;
  const uint32_t mytab = (uint32_t)__cvta_generic_to_shared(tab + wi * UTQ);
  const uint32_t hb = mytab + (lane >> 4) * 2048;
  const Quad* const pq = (const Quad*)Pt;
  if (tid == 0) {
    for (int q = 0; q < RING; ++q) {
      asm volatile("mbarrier.init.shared::cta.b64 [%0], 1;" :: "r"(full0 + 8 * q) : "memory");
      asm volatile("mbarrier.init.shared::cta.b64 [%0], %1;" :: "r"(empty0 + 8 * q), "n"(WARPS) : "memory");
    }
    asm volatile("fence.mbarrier_init.release.cluster;" ::: "memory");
  }
  carrega_u(mytab, (const Quad*)Ut, lrow(0) * WARPS + wi, lane);
  const int pr = (int)(prc >> 16), pc = (int)(prc & 0xffff);
  const int pgrp = pr >> 5, pe = (pr >> 3) & 3, pcc = pc & 31;
  const bool plane = pv != 0 && g == (pr & 7) && t == ((pcc & 15) >> 2);
  const int pstep = pc >> 5, pslot = (pe >> 1) * 4 + (pe & 1) + 2 * (pcc >> 4), pjb = pcc & 3;
  asm volatile("cp.async.wait_all;" ::: "memory");
  __syncthreads();
  const uint32_t stages = (uint32_t)NL * (pairs / PST);
  if (tid == 0)
    for (uint32_t v = 0; v < LA && v < stages; ++v) copia(ring0, full0, bsv(v), pq, v, pairs);
  uint32_t u = 0;
  for (int lap = 0; lap < NL; ++lap) {
    const int band0 = (lrow(lap) * WARPS + wi) * 2;
    const int grp = band0 >> 1;
    const bool mine = plane && grp == pgrp;
    if (lap > 0) { asm volatile("cp.async.wait_all;" ::: "memory"); __syncwarp(); }
    const uint32_t trs = (uint32_t)__cvta_generic_to_shared(trq) + (uint32_t)tid * 4;
    #pragma unroll
    for (int i = 0; i < 16; ++i) asm volatile("st.shared.u32 [%0], %1;" :: "r"(trs + i * 1024), "r"(0u) : "memory");
    uint32_t trl[16];
    int32_t c0[NA][4], c1[NA][4];
    #pragma unroll
    for (int j = 0; j < NA; ++j) { c0[j][0] = c0[j][1] = c0[j][2] = c0[j][3] = 0; c1[j][0] = c1[j][1] = c1[j][2] = c1[j][3] = 0; }
    #pragma unroll
    for (int i = 0; i < 16; ++i) trl[i] = 0;

    const uint32_t pob = ring0 + (uint32_t)BSTQ * 16 + (uint32_t)(lane & 15) * 16;
    Quad xa, xw;
    auto etapas = [&](auto pk) {
    for (int ps = 0; ps < pairs; ps += PST, ++u) {

      if (wi == (int)(u & 7) && u + LA < stages) {
        const uint32_t v = u + LA;
        if (v >= RING) mb_wait_c(empty0 + 8 * (v % RING), ((v / RING) - 1) & 1);
        copia_u(ring0, full0, bsv(v), v, pairs);
      }
      mb_wait(full0 + 8 * (u % RING), (u / RING) & 1);
      if (false)
        mb_wait(full0 + 8 * ((u + 1) % RING), ((u + 1) / RING) & 1);
      const Quad* const st = ring + (u % RING) * STAGEQ + lane;
      #pragma unroll
      for (int s = 0; s < 2 * PST; ++s) {
        Quad ca, cw;
        gera7(hb, pob + (uint32_t)((u % RING) * STAGEQ) * 16 + (uint32_t)s * OSTEP, ca, cw);
        if constexpr (decltype(pk)::value) { if (mine && pstep == 2 * ps + s) poke(ca, cw, pslot, pjb, pv); }
        const bool nova = s < 2 * PST - 1 || ps + PST < pairs;
        const uint32_t un = s < 2 * PST - 1 ? u : u + 1;
        const uint32_t po = pob + (uint32_t)((un % RING) * STAGEQ) * 16 + (uint32_t)((s + 1) & (2 * PST - 1)) * OSTEP;
        const uint32_t rb0 = ring0 + (uint32_t)((u % RING) * STAGEQ + (s >> 1) * PAIRQ + lane) * 16 + (uint32_t)(s & 1) * 8;
        uint2 bq[NA];
        uint32_t o0, o1, o2, o3, pl[4], ml[4], ph[4], mh[4];
        asm volatile("ld.shared.v2.u32 {%0,%1}, [%2];" : "=r"(bq[0].x), "=r"(bq[0].y) : "r"(rb0));
        asm volatile("ld.shared.v2.u32 {%0,%1}, [%2];" : "=r"(bq[1].x), "=r"(bq[1].y) : "r"(rb0 + 32 * 16));
        #pragma unroll
        for (int j = 0; j < NA; ++j) {
          if (j + 2 < NA)
            asm volatile("ld.shared.v2.u32 {%0,%1}, [%2];" : "=r"(bq[j + 2].x), "=r"(bq[j + 2].y) : "r"(rb0 + (uint32_t)(j + 2) * 32 * 16));
          uint32_t b[2] = {bq[j].x, bq[j].y};
          MMA_K32(c0[j], ca, b);
          MMA_K32(c1[j], cw, b);
        }
      }
      __syncwarp();
      if (lane == 0) asm volatile("mbarrier.arrive.shared::cta.b64 _, [%0];" :: "r"(empty0 + 8 * (u % RING)) : "memory");
      if (PST == 2 || (ps & 1)) {

        uint32_t xp[ARVORE];
        #pragma unroll
        for (int i = 0; i < ARVORE; ++i) xp[i] = 0;

        #pragma unroll
        for (int j = 0; j < NA; ++j) dobra8t(xp[j % ARVORE], c0[j], c1[j]);
        #pragma unroll
        for (int i = 1; i < ARVORE; ++i) xp[0] ^= xp[i];
        const uint32_t x = xp[0];
        const int w = (ps >> 1) & 15;
        #pragma unroll
        {
          uint32_t o;
          asm volatile("ld.shared.u32 %0, [%1];" : "=r"(o) : "r"(trs + w * 1024) : "memory");
          asm volatile("st.shared.u32 [%0], %1;" :: "r"(trs + w * 1024), "r"(((o << 13) | (o >> 19)) ^ x) : "memory");
        }
      }
    }
    };
    if (pv != 0 && grp == pgrp) etapas(std::true_type{}); else etapas(std::false_type{});
    if (lap + 1 < NL) {
      __syncwarp();
      carrega_u(mytab, (const Quad*)Ut, lrow(lap + 1) * WARPS + wi, lane);
    }
    uint32_t tr[16];
    #pragma unroll
    for (int i = 0; i < 16; ++i) asm volatile("ld.shared.u32 %0, [%1];" : "=r"(tr[i]) : "r"(trs + i * 1024) : "memory");
    finish_tile(tr, key, target, hits, dump, band0 >> 1, lstrip(lap), ncol, lane, band0 * 16 + g, t, lstrip(lap) * 8 * NA);
  }
}

