// DoriMiner - gera40.cu
// Copyright (C) 2026 J. Costa (DoriByte). Licensed under the PolyForm Noncommercial License 1.0.0 with the additional
// permission in MINING-PERMISSION (see LICENSE and MINING-PERMISSION at the top of this tree).

#include <cstdint>
#include <type_traits>
#define NA 16
#define WARPS 8
#define RING 4
#define LA 3
#define PST 1
#define MT 3
#define MMAVOL volatile
#define MMA_K32(c,a,b) asm MMAVOL("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};" \
  : "+r"(c[0]), "+r"(c[1]), "+r"(c[2]), "+r"(c[3]) : "r"(a.w[0]), "r"(a.w[1]), "r"(a.w[2]), "r"(a.w[3]), "r"(b[0]), "r"(b[1]))
__device__ __forceinline__ uint32_t rotr(uint32_t x, int n) { return (x >> n) | (x << (32 - n)); }
#define G(a,b,c,d,x,y) a = a + b + x; d = rotr(d ^ a, 16); c = c + d; b = rotr(b ^ c, 12); a = a + b + y; d = rotr(d ^ a, 8); c = c + d; b = rotr(b ^ c, 7);
#define PAIRQ (NA * 32)
#define BSTQ (PST * PAIRQ)
#define PSTQ (32 * PST)
#define STAGEQ (BSTQ + PSTQ)
#define BARQ ((2 * RING * 8 + 15) / 16)
#define UTQ (128 * MT)
struct alignas(16) Quad { uint32_t w[4]; };

__device__ __forceinline__ void poke(Quad (&a)[MT], int slot, int jb, int v) {
  const uint32_t add = (uint32_t)v << (8 * jb), sbit = 1u << slot;
  #pragma unroll
  for (int s = 0; s < 4 * MT; ++s) {
    uint32_t& x = a[s >> 2].w[s & 3];
    if ((sbit >> s) & 1u) x = ((x ^ 0x80808080u) + add) ^ 0x80808080u;
  }
}

__device__ __forceinline__ void finish_tile(uint32_t* tr, const uint32_t* key, const uint32_t* target, uint32_t* hits,
                                            uint32_t* dump, int band, int strip, int ncol, int lane, int row, int t, int col0) {
  static const int perm[16] = {2,6,3,10,7,0,4,13,1,11,12,5,9,14,15,8};
  (void)dump; (void)ncol; (void)strip; (void)band;
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

__device__ __forceinline__ void copia(uint32_t ring0, uint32_t full0, const Quad* bstrip, uint32_t v, int pairs) {
  const uint32_t slot = v % RING, bar = full0 + 8 * slot, sv = v & (uint32_t)(pairs / PST - 1);
  asm volatile("mbarrier.arrive.expect_tx.shared::cta.b64 _, [%0], %1;" :: "r"(bar), "n"(STAGEQ * 16) : "memory");
  asm volatile("cp.async.bulk.shared::cluster.global.mbarrier::complete_tx::bytes [%0], [%1], %3, [%2];"
               :: "r"(ring0 + slot * STAGEQ * 16), "l"(bstrip + (size_t)sv * STAGEQ), "r"(bar), "n"(STAGEQ * 16) : "memory");
}

__device__ __forceinline__ void copia_u(uint32_t ring0, uint32_t full0, const Quad* bstrip, uint32_t v, int pairs) {
  const uint32_t slot = v % RING, bar = full0 + 8 * slot, sv = v & (uint32_t)(pairs / PST - 1);
  asm volatile("{ .reg .pred p; elect.sync _|p, 0xffffffff;\n\t"
               "@p mbarrier.arrive.expect_tx.shared::cta.b64 _, [%2], %3;\n\t"
               "@p cp.async.bulk.shared::cta.global.mbarrier::complete_tx::bytes [%0], [%1], %3, [%2]; }"
               :: "r"(ring0 + slot * STAGEQ * 16), "l"(bstrip + (size_t)sv * STAGEQ), "r"(bar), "n"(STAGEQ * 16) : "memory");
}
__device__ __forceinline__ void ldm8(uint32_t addr, uint32_t r[4]) {
  asm volatile("ldmatrix.sync.aligned.m16n16.x2.trans.shared.b8 {%0,%1,%2,%3}, [%4];" : "=r"(r[0]), "=r"(r[1]), "=r"(r[2]), "=r"(r[3]) : "r"(addr));
}
__device__ __forceinline__ uint32_t dif(uint32_t a, uint32_t b) { return (a - b + 0x80808080u) ^ 0x80808080u; }

__device__ __forceinline__ void gera40(uint32_t hb, uint32_t hb3, bool hi, uint32_t offs, Quad (&a)[MT]) {
  uint32_t o0, o1, o2, o3;
  asm volatile("ld.shared.v4.u32 {%0,%1,%2,%3}, [%4];" : "=r"(o0), "=r"(o1), "=r"(o2), "=r"(o3) : "r"(offs));
  uint32_t pl[4], ph[4], ml[4], mh[4], p3[4], m3[4];
  ldm8(hb + o0, pl); ldm8(hb + o1, ph); ldm8(hb + o2, ml); ldm8(hb + o3, mh);
  ldm8(hb3 + (hi ? o1 : o0), p3); ldm8(hb3 + (hi ? o3 : o2), m3);
  a[0].w[0] = dif(pl[0], ml[0]); a[0].w[1] = dif(pl[1], ml[1]); a[0].w[2] = dif(ph[0], mh[0]); a[0].w[3] = dif(ph[1], mh[1]);
  a[1].w[0] = dif(pl[2], ml[2]); a[1].w[1] = dif(pl[3], ml[3]); a[1].w[2] = dif(ph[2], mh[2]); a[1].w[3] = dif(ph[3], mh[3]);
  a[2].w[0] = dif(p3[0], m3[0]); a[2].w[1] = dif(p3[1], m3[1]); a[2].w[2] = dif(p3[2], m3[2]); a[2].w[3] = dif(p3[3], m3[3]);
}

__device__ __forceinline__ void dobra12t(uint32_t& x, const int32_t (&a)[4], const int32_t (&b)[4], const int32_t (&c)[4]) {
  asm("{ .reg .b32 t, u2;\n\t"
      "lop3.b32 t, %1, %2, %3, 0x96;\n\t"
      "lop3.b32 u2, %4, %5, %6, 0x96;\n\t"
      "lop3.b32 t, t, u2, %7, 0x96;\n\t"
      "lop3.b32 u2, %8, %9, %10, 0x96;\n\t"
      "lop3.b32 t, t, u2, %11, 0x96;\n\t"
      "lop3.b32 %0, %0, t, %12, 0x96; }"
      : "+r"(x) : "r"(a[0]), "r"(a[1]), "r"(a[2]), "r"(a[3]), "r"(b[0]), "r"(b[1]), "r"(b[2]), "r"(b[3]),
                  "r"(c[0]), "r"(c[1]), "r"(c[2]), "r"(c[3]));
}
#define ARVORE 8

#define LDB "ld.volatile.shared.v2.u32"
#define LDQ "ld.volatile.shared.v4.u32"
#define BADI 2

extern "C" __global__ void __launch_bounds__(32 * WARPS, 1)
prl_gera40(const uint32_t* Ut, const uint32_t* Bt, const uint32_t* key, const uint32_t* target,
           uint32_t* hits, int n, int k, uint32_t* dump, int m, int rows16, const uint32_t* Pt, uint32_t prc, int pv) {
  extern __shared__ Quad sm[];
  Quad* const ring = sm;
  uint64_t* const bars = (uint64_t*)(sm + RING * STAGEQ);
  Quad* const tab = sm + RING * STAGEQ + BARQ;
  const int tid = threadIdx.x, wi = tid >> 5, lane = tid & 31, g = lane >> 2, t = lane & 3;
  const int ncol = n / (8 * NA), pairs = (k >> 7) * 2;
  const int PS = (m >> 16) & 0xffff, PR = m & 0xffff;
  int strip = blockIdx.x % ncol, rb = blockIdx.x / ncol;
  if (PS) { const int i = blockIdx.x % (PR * PS), panel = blockIdx.x / (PR * PS), across = ncol / PS; strip = (panel % across) * PS + i % PS; rb = (panel / across) * PR + i / PS; }
  const int NL = LAPS;
  auto lrow = [&](int L) { return rb * LAPS + L; };

  auto repos = [&](int& st, int& r) {
    uint32_t b; asm volatile("mov.u32 %0, %%ctaid.x;" : "=r"(b));
    int nc; asm volatile("mov.b32 %0, %1;" : "=r"(nc) : "r"(n)); nc /= 8 * NA;
    st = (int)b % nc; r = (int)b / nc;
    if (PS) { const int i = (int)b % (PR * PS), panel = (int)b / (PR * PS), across = nc / PS; st = (panel % across) * PS + i % PS; r = (panel / across) * PR + i / PS; }
  };
  (void)rows16; (void)Pt;
  auto bsv = [&](uint32_t v) { (void)v; return (const Quad*)Bt + (size_t)strip * (pairs / PST) * STAGEQ; };
  const uint32_t ring0 = (uint32_t)__cvta_generic_to_shared(ring);
  const uint32_t full0 = (uint32_t)__cvta_generic_to_shared(bars), empty0 = full0 + 8 * RING;
  const uint32_t mytab = (uint32_t)__cvta_generic_to_shared(tab + wi * UTQ);
  const uint32_t hb = mytab + (lane >> 4) * 2048, hb3 = mytab + 4096;
  const bool hi = lane >= 16;
  if (tid == 0) {
    for (int q = 0; q < RING; ++q) {
      asm volatile("mbarrier.init.shared::cta.b64 [%0], 1;" :: "r"(full0 + 8 * q) : "memory");
      asm volatile("mbarrier.init.shared::cta.b64 [%0], %1;" :: "r"(empty0 + 8 * q), "n"(WARPS) : "memory");
    }
    asm volatile("fence.mbarrier_init.release.cluster;" ::: "memory");
  }
  carrega_u(mytab, (const Quad*)Ut, lrow(0) * WARPS + wi, lane);

  const int pr = (int)(prc >> 16), pc = (int)(prc & 0xffff);
  const int pgrp = pr / (16 * MT), pe = (pr % (16 * MT)) >> 3, pcc = pc & 31;
  const bool plane = pv != 0 && g == (pr & 7) && t == ((pcc & 15) >> 2);
  const int pstep = pc >> 5, pslot = (pe >> 1) * 4 + (pe & 1) + 2 * (pcc >> 4), pjb = pcc & 3;
  asm volatile("cp.async.wait_all;" ::: "memory");
  __syncthreads();
  const uint32_t stages = (uint32_t)NL * (pairs / PST);
  if (tid == 0)
    for (uint32_t v = 0; v < LA && v < stages; ++v) copia(ring0, full0, bsv(v), v, pairs);
  uint32_t u = 0;
  for (int lap = 0; lap < NL; ++lap) {
    const int grp = lrow(lap) * WARPS + wi;
    uint32_t lid; asm volatile("mov.u32 %0, %%laneid;" : "=r"(lid));
    const bool mine = pv != 0 && (int)(lid >> 2) == (pr & 7) && (int)(lid & 3) == ((pcc & 15) >> 2) && (unsigned)(pr - grp * 16 * MT) < 16 * MT;
    (void)plane;
    (void)mine;
    if (lap > 0) { asm volatile("cp.async.wait_all;" ::: "memory"); __syncwarp(); }
    const uint32_t trs = (uint32_t)__cvta_generic_to_shared(tab + WARPS * UTQ) + (uint32_t)tid * 4;
    #pragma unroll
    for (int i = 0; i < 16; ++i) asm volatile("st.shared.u32 [%0], %1;" :: "r"(trs + i * 1024), "r"(0u) : "memory");
    int32_t c[MT][NA][4];
    #pragma unroll
    for (int i = 0; i < MT; ++i)
      #pragma unroll
      for (int j = 0; j < NA; ++j) { c[i][j][0] = c[i][j][1] = c[i][j][2] = c[i][j][3] = 0; }
    const uint32_t pob = ring0 + (uint32_t)BSTQ * 16 + (uint32_t)(lane & 15) * 16;
    auto etapas = [&](auto pk) {
    for (int ps = 0; ps < pairs; ps += PST, ++u) {

      if (wi == (int)(u & 7) && u + LA < stages) {
        const uint32_t v = u + LA;
        if (v >= RING) mb_wait(empty0 + 8 * (v % RING), ((v / RING) - 1) & 1);
        copia_u(ring0, full0, bsv(v), v, pairs);
      }
      mb_wait(full0 + 8 * (u % RING), (u / RING) & 1);
      {

        Quad ca[2][MT];
        #pragma unroll
        for (int s = 0; s < 2; ++s) {
          gera40(hb, hb3, hi, pob + (uint32_t)((u % RING) * STAGEQ) * 16 + (uint32_t)s * 256, ca[s]);
          if constexpr (decltype(pk)::value) {
            const int pe2 = (pr - grp * 16 * MT) >> 3;
            if (mine && pstep == 2 * ps + s) poke(ca[s], (pe2 >> 1) * 4 + (pe2 & 1) + 2 * (pcc >> 4), pjb, pv);
          }
        }
        const uint32_t rq = ring0 + (uint32_t)((u % RING) * STAGEQ + lane) * 16;
        uint4 bq[NA];
        #pragma unroll
        for (int j = 0; j < JADI; ++j)
          asm volatile(LDQ " {%0,%1,%2,%3}, [%4];" : "=r"(bq[j].x), "=r"(bq[j].y), "=r"(bq[j].z), "=r"(bq[j].w) : "r"(rq + (uint32_t)j * 512));
        #pragma unroll
        for (int j0 = 0; j0 < NA; j0 += JB) {
          #pragma unroll
          for (int jj = 0; jj < JB; ++jj)
            if (j0 + jj + JADI < NA)
              asm volatile(LDQ " {%0,%1,%2,%3}, [%4];" : "=r"(bq[j0 + jj + JADI].x), "=r"(bq[j0 + jj + JADI].y),
                           "=r"(bq[j0 + jj + JADI].z), "=r"(bq[j0 + jj + JADI].w) : "r"(rq + (uint32_t)(j0 + jj + JADI) * 512));
          #pragma unroll
          for (int s = 0; s < 2; ++s)
            #pragma unroll
            for (int jj = 0; jj < JB; ++jj) {
              const uint4 q = bq[j0 + jj];
              uint32_t b[2] = {s ? q.z : q.x, s ? q.w : q.y};
              #pragma unroll
              for (int i = 0; i < MT; ++i) MMA_K32(c[i][j0 + jj], ca[s][i], b);
            }
        }
      }
      __syncwarp();
      if (lane == 0) asm volatile("mbarrier.arrive.shared::cta.b64 _, [%0];" :: "r"(empty0 + 8 * (u % RING)) : "memory");
      if (ps & 1) {
        uint32_t xp[ARVORE];
        #pragma unroll
        for (int i = 0; i < ARVORE; ++i) xp[i] = 0;
        #pragma unroll
        for (int j = 0; j < NA; ++j) dobra12t(xp[j % ARVORE], c[0][j], c[1][j], c[2][j]);
        #pragma unroll
        for (int i = 1; i < ARVORE; ++i) xp[0] ^= xp[i];
        const uint32_t x = xp[0];
        const int w = (ps >> 1) & 15;
        uint32_t o;
        asm volatile("ld.shared.u32 %0, [%1];" : "=r"(o) : "r"(trs + w * 1024) : "memory");
        asm volatile("st.shared.u32 [%0], %1;" :: "r"(trs + w * 1024), "r"(((o << 13) | (o >> 19)) ^ x) : "memory");
      }
    }
    };
    if (pv != 0 && (unsigned)(pr - grp * 16 * MT) < 16 * MT) etapas(std::true_type{}); else etapas(std::false_type{});
    lap = (int)(u >> (31 - __clz(pairs))) - 1;
    { int st2, rb2; repos(st2, rb2); strip = st2; rb = rb2; }
    const int grp2 = lrow(lap) * WARPS + wi;
    if (lap + 1 < NL) {
      __syncwarp();
      carrega_u(mytab, (const Quad*)Ut, lrow(lap + 1) * WARPS + wi, lane);
    }
    uint32_t tr[16];
    #pragma unroll
    for (int i = 0; i < 16; ++i) asm volatile("ld.shared.u32 %0, [%1];" : "=r"(tr[i]) : "r"(trs + i * 1024) : "memory");
    finish_tile(tr, key, target, hits, dump, grp2, strip, ncol, lane, grp2 * 16 * MT + g, t, strip * 8 * NA);
  }
}

