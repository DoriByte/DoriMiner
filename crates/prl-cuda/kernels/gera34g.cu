// DoriMiner - gera34g.cu
// Copyright (C) 2026 J. Costa (DoriByte). Licensed under the PolyForm Noncommercial License 1.0.0 with the additional
// permission in MINING-PERMISSION (see LICENSE and MINING-PERMISSION at the top of this tree).

#include <cstdint>
#define NA 16
#define WARPS 8
#define RING 4
#define LEAD (RING / 2)
#define LAPS 8
#define NAP 100

#define GA_MODO 2
#define GJ 8
#define MMA_K32(c,a,b) asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};" \
  : "+r"(c[0]), "+r"(c[1]), "+r"(c[2]), "+r"(c[3]) : "r"(a.w[0]), "r"(a.w[1]), "r"(a.w[2]), "r"(a.w[3]), "r"(b[0]), "r"(b[1]))
__device__ __forceinline__ uint32_t rotr(uint32_t x, int n) { return (x >> n) | (x << (32 - n)); }
#define G(a,b,c,d,x,y) a = a + b + x; d = rotr(d ^ a, 16); c = c + d; b = rotr(b ^ c, 12); a = a + b + y; d = rotr(d ^ a, 8); c = c + d; b = rotr(b ^ c, 7);
#define PAIRQ (NA * 32)
#define QP (PAIRQ / (32 * WARPS))
#define UTQ 256
#if WARPS > 32 || RING < 2 * LEAD || QP < 1 || QP * 32 * WARPS != PAIRQ
#error
#endif
struct alignas(16) Quad { uint32_t w[4]; };

__device__ __forceinline__ void ldm(uint32_t addr, uint32_t r[4]) {
  asm volatile("ldmatrix.sync.aligned.m8n8.x4.trans.shared.b16 {%0,%1,%2,%3}, [%4];" : "=r"(r[0]), "=r"(r[1]), "=r"(r[2]), "=r"(r[3]) : "r"(addr));
}

__device__ __forceinline__ void xork(uint32_t& x, uint32_t w, int s) {
  if (s) asm volatile("lop3.b32 %0, %0, %1, 0x3c3c3c3c, 0x96;" : "+r"(x) : "r"(w));
  else asm volatile("lop3.b32 %0, %0, %1, 0x5a5a5a5a, 0x96;" : "+r"(x) : "r"(w));
}
__device__ __forceinline__ uint32_t dif(uint32_t a, uint32_t b) { return (a - b + 0x80808080u) ^ 0x80808080u; }

__device__ __forceinline__ void frag(uint32_t p, uint32_t m, Quad& a) {
  uint32_t x[4], y[4];
  ldm(p, x); ldm(m, y);
  const uint32_t d0 = dif(x[0], y[0]), d1 = dif(x[1], y[1]), d2 = dif(x[2], y[2]), d3 = dif(x[3], y[3]);
  a.w[0] = __byte_perm(d0, d1, 0x6420); a.w[1] = __byte_perm(d0, d1, 0x7531);
  a.w[2] = __byte_perm(d2, d3, 0x6420); a.w[3] = __byte_perm(d2, d3, 0x7531);
}

__device__ __forceinline__ void gera(uint32_t base, uint32_t v, Quad& av, Quad& aw) {
  const uint32_t p = base + (v >> 16), m = base + (v & 0xffffu);
  frag(p, m, av);
  frag(p + 2048, m + 2048, aw);
}

__device__ __forceinline__ void gera_b(uint32_t vb, uint2 v, Quad& e, Quad& o) {
  #pragma unroll
  for (int s = 0; s < 2; ++s) {
    const uint32_t w = s ? v.y : v.x;
    uint32_t x[4], y[4];
    ldm(vb + (w >> 16), x); ldm(vb + (w & 0xffffu), y);
    const uint32_t d0 = dif(x[0], y[0]), d1 = dif(x[1], y[1]), d2 = dif(x[2], y[2]), d3 = dif(x[3], y[3]);
    e.w[2 * s] = __byte_perm(d0, d1, 0x6420); e.w[2 * s + 1] = __byte_perm(d2, d3, 0x6420);
    o.w[2 * s] = __byte_perm(d0, d1, 0x7531); o.w[2 * s + 1] = __byte_perm(d2, d3, 0x7531);
  }
}

__device__ __forceinline__ void gera_b_ld(uint32_t vb, uint2 v, uint32_t gb[16]) {
  ldm(vb + (v.x >> 16), gb); ldm(vb + (v.x & 0xffffu), gb + 4); ldm(vb + (v.y >> 16), gb + 8); ldm(vb + (v.y & 0xffffu), gb + 12);
}
__device__ __forceinline__ void poe_b_fim(Quad* ring, uint32_t slot, int wi, int lane, const uint32_t gb[16]) {
  Quad e, o;
  #pragma unroll
  for (int s = 0; s < 2; ++s) {
    const uint32_t* x = gb + 8 * s; const uint32_t* y = gb + 8 * s + 4;
    const uint32_t d0 = dif(x[0], y[0]), d1 = dif(x[1], y[1]), d2 = dif(x[2], y[2]), d3 = dif(x[3], y[3]);
    e.w[2 * s] = __byte_perm(d0, d1, 0x6420); e.w[2 * s + 1] = __byte_perm(d2, d3, 0x6420);
    o.w[2 * s] = __byte_perm(d0, d1, 0x7531); o.w[2 * s + 1] = __byte_perm(d2, d3, 0x7531);
  }
  ring[slot * PAIRQ + (2 * wi) * 32 + lane] = e;
  ring[slot * PAIRQ + (2 * wi + 1) * 32 + lane] = o;
}
__device__ __forceinline__ void poe_b(Quad* ring, uint32_t slot, int wi, int lane, uint32_t vb, uint2 v) {
  Quad e, o;
  gera_b(vb, v, e, o);
  ring[slot * PAIRQ + (2 * wi) * 32 + lane] = e;
  ring[slot * PAIRQ + (2 * wi + 1) * 32 + lane] = o;
}

__device__ __forceinline__ void gera_ld(uint32_t base, uint32_t v, uint32_t g[16]) {
  const uint32_t p = base + (v >> 16), m = base + (v & 0xffffu);
  ldm(p, g); ldm(m, g + 4); ldm(p + 2048, g + 8); ldm(m + 2048, g + 12);
}
__device__ __forceinline__ void gera_fim(const uint32_t g[16], Quad& av, Quad& aw) {
  #pragma unroll
  for (int h = 0; h < 2; ++h) {
    const uint32_t* x = g + 8 * h; const uint32_t* y = g + 8 * h + 4;
    Quad& a = h ? aw : av;
    const uint32_t d0 = dif(x[0], y[0]), d1 = dif(x[1], y[1]), d2 = dif(x[2], y[2]), d3 = dif(x[3], y[3]);
    a.w[0] = __byte_perm(d0, d1, 0x6420); a.w[1] = __byte_perm(d0, d1, 0x7531);
    a.w[2] = __byte_perm(d2, d3, 0x6420); a.w[3] = __byte_perm(d2, d3, 0x7531);
  }
}

__device__ __forceinline__ uint32_t lin_p(uint32_t h) { return ((h >> 8) & 0xffu) << 4; }
__device__ __forceinline__ uint32_t lin_m(uint32_t h) { return (h & 0xffu) << 4; }
__device__ __forceinline__ void gera_h(uint32_t base, uint32_t h, Quad& av, Quad& aw) {
  const uint32_t p = base + lin_p(h), m = base + lin_m(h);
  frag(p, m, av);
  frag(p + 2048, m + 2048, aw);
}
__device__ __forceinline__ void gera_ldh(uint32_t base, uint32_t h, uint32_t g[16]) {
  const uint32_t p = base + lin_p(h), m = base + lin_m(h);
  ldm(p, g); ldm(m, g + 4); ldm(p + 2048, g + 8); ldm(m + 2048, g + 12);
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

  uint64_t pol;
  asm volatile("createpolicy.fractional.L2::evict_first.b64 %0, 1.0;" : "=l"(pol));
  #pragma unroll
  for (int q = 0; q < UTQ / 32; ++q)
    asm volatile("cp.async.cg.shared.global.L2::cache_hint [%0], [%1], 16, %2;" :: "r"(dst + (uint32_t)(q * 32 + lane) * 16), "l"(src + q * 32), "l"(pol) : "memory");
  asm volatile("cp.async.commit_group;" ::: "memory");
}

extern "C" __global__ void __launch_bounds__(32 * WARPS, 1)
prl_gera34x(const uint32_t* Ut, const uint32_t* Bt, const uint32_t* key, const uint32_t* target,
           uint32_t* hits, int n, int k, uint32_t* dump, int m, int rows16, const uint32_t* Pt, uint32_t prc, int pv) {
  extern __shared__ Quad sm[];
  Quad* const tab = sm;
  Quad* const vtab = tab + WARPS * UTQ;
  Quad* const par = vtab + WARPS * 128;
  Quad* const ring = par + k / 8;
  volatile uint32_t* const done = (volatile uint32_t*)(ring + RING * PAIRQ);
  const int tid = threadIdx.x, wi = tid >> 5, lane = tid & 31, g = lane >> 2, t = lane & 3;
  const int ncol = n / (8 * NA), pairs = (k >> 7) * 2;
  const int PS = (m >> 16) & 0xffff, PR = m & 0xffff;
  int strip = blockIdx.x % ncol, rb = blockIdx.x / ncol;
  if (PS) { const int i = blockIdx.x % (PR * PS), panel = blockIdx.x / (PR * PS), across = ncol / PS; strip = (panel % across) * PS + i % PS; rb = (panel / across) * PR + i / PS; }
  const int col0 = strip * 8 * NA;
  (void)rows16;
  const uint32_t mytab = (uint32_t)__cvta_generic_to_shared(tab + wi * UTQ);
  const uint32_t myv = (uint32_t)__cvta_generic_to_shared(vtab + wi * 128);
  const uint32_t morto = (uint32_t)__cvta_generic_to_shared(vtab);
  const uint32_t* const pa = (const uint32_t*)par + lane;
  const uint2* const pb = (const uint2*)(Pt + k) + lane;
  carrega_u(mytab, (const Quad*)Ut, (rb * LAPS) * WARPS + wi, lane);
  {
    const Quad* src = (const Quad*)Bt + (size_t)(strip * WARPS + wi) * 128 + lane;
    #pragma unroll
    for (int q = 0; q < 4; ++q)
      asm volatile("cp.async.cg.shared.global [%0], [%1], 16;" :: "r"(myv + (uint32_t)(q * 32 + lane) * 16), "l"(src + q * 32) : "memory");
    asm volatile("cp.async.commit_group;" ::: "memory");
  }
  for (int q = tid; q < k / 2; q += 32 * WARPS) {
    const uint2 x = ((const uint2*)Pt)[q];
    ((uint32_t*)par)[q] = (x.x >> 20) << 8 | (x.x & 0xffffu) >> 4 | ((x.y >> 20) << 8 | (x.y & 0xffffu) >> 4) << 16;
  }
  done[wi] = 0;

  const int pr = (int)(prc >> 16), pc = (int)(prc & 0xffff);
  const int pgrp = pr >> 5, pe = (pr >> 3) & 3, pcc = pc & 31;
  const bool plane = pv != 0 && g == (pr & 7) && t == ((pcc & 15) >> 2);
  const int pstep = pc >> 5, pslot = (pe >> 1) * 4 + (pe & 1) + 2 * (pcc >> 4), pjb = pcc & 3;
  asm volatile("cp.async.wait_all;" ::: "memory");
  __syncwarp();
  #pragma unroll 1
  for (int q = 0; q < LEAD; ++q) poe_b(ring, q, wi, lane, myv, pb[q * 32]);
  __syncthreads();
  const uint32_t total = (uint32_t)LAPS * pairs;
  uint32_t r = 0;
  uint2 vbn = pb[LEAD * 32];
  for (int lap = 0; lap < LAPS; ++lap) {
    const int band0 = ((rb * LAPS + lap) * WARPS + wi) * 2;
    const int grp = band0 >> 1;
    const bool mine = plane && grp == pgrp;
    const int ppair = mine ? (pstep >> 1) : -1;
    const int pvl = pv;
    if (lap > 0) { asm volatile("cp.async.wait_all;" ::: "memory"); __syncwarp(); }
    uint32_t trr[16], xlast = 0;
    #pragma unroll
    for (int i = 0; i < 16; ++i) trr[i] = 0;
    int32_t c0[NA][4], c1[NA][4];
    #pragma unroll
    for (int j = 0; j < NA; ++j) { c0[j][0] = c0[j][1] = c0[j][2] = c0[j][3] = 0; c1[j][0] = c1[j][1] = c1[j][2] = c1[j][3] = 0; }
    Quad av0, av1, aw0, aw1;
    {
      const uint32_t v = pa[0];
      gera_h(mytab, v, av0, aw0);
      gera_h(mytab, v >> 16, av1, aw1);
    }
    uint32_t van = pa[32];
    if (mine && pstep == 0) poke(av0, aw0, pslot, pjb, pv);
    if (mine && pstep == 1) poke(av1, aw1, pslot, pjb, pv);

    auto half = [&](const Quad& av0, const Quad& av1, const Quad& aw0, const Quad& aw1, Quad& an0, Quad& an1, Quad& bn0,
                    Quad& bn1, const int pair, const bool first, const int rsl, const int wsl) {
      if (!first && pair + 1 == pairs && lap + 1 < LAPS) {

        __syncwarp();
        carrega_u(mytab, (const Quad*)Ut, grp + WARPS, lane);
      }
      for (;;) {
        const uint32_t d = done[lane % WARPS];
        if (__all_sync(0xffffffffu, (int32_t)(r - d) < LEAD)) break;
        __nanosleep(NAP);
      }
      const uint32_t up = first || pair + 1 < pairs ? mytab : morto;
      const int pn = (pair + 1) & (pairs - 1), s0 = 2 * pn;
      const uint32_t v = van; const uint2 vb = vbn;
      van = pa[((pair + 2) & (pairs - 1)) * 32];
      vbn = pb[((pair + LEAD + 1) & (pairs - 1)) * 32];
      const Quad* const rd = ring + rsl * PAIRQ + lane;
      const bool depois = GA_MODO == 2 && (wi & (WARPS / 2)) != 0;
      if (!depois) { gera_h(up, v, an0, bn0); gera_h(up, v >> 16, an1, bn1); }
#define POE_SLICE poe_b(ring, wsl, wi, lane, myv, vb);

      #pragma unroll
      for (int j0 = 0; j0 < NA; j0 += GJ) {
        Quad b[GJ];
        #pragma unroll
        for (int i = 0; i < GJ; ++i) b[i] = rd[(j0 + i) * 32];
        #pragma unroll
        for (int i = 0; i < GJ; ++i) { uint32_t x[2] = {b[i].w[0], b[i].w[1]}; MMA_K32(c0[j0 + i], av0, x); }
        #pragma unroll
        for (int i = 0; i < GJ; ++i) { uint32_t x[2] = {b[i].w[0], b[i].w[1]}; MMA_K32(c1[j0 + i], aw0, x); }
        #pragma unroll
        for (int i = 0; i < GJ; ++i) { uint32_t x[2] = {b[i].w[2], b[i].w[3]}; MMA_K32(c0[j0 + i], av1, x); }
        #pragma unroll
        for (int i = 0; i < GJ; ++i) { uint32_t x[2] = {b[i].w[2], b[i].w[3]}; MMA_K32(c1[j0 + i], aw1, x); }
      }
      if (depois) {

        uint32_t g0[16], g1[16];
        gera_ldh(up, v, g0); gera_ldh(up, v >> 16, g1);
        POE_SLICE
        gera_fim(g0, an0, bn0); gera_fim(g1, an1, bn1);
      } else {
        POE_SLICE
      }
      if (ppair == pair + 1) { if (pstep == s0) poke(an0, bn0, pslot, pjb, pvl); else poke(an1, bn1, pslot, pjb, pvl); }
      if (!first) {
        uint32_t xc[XCH];
        #pragma unroll
        for (int i = 0; i < XCH; ++i) xc[i] = 0;
        #pragma unroll
        for (int j = 0; j < NA; ++j) {
          uint32_t& y = xc[j % XCH];
          y ^= (uint32_t)c0[j][0] ^ (uint32_t)c0[j][1] ^ (uint32_t)c0[j][2] ^ (uint32_t)c0[j][3]
             ^ (uint32_t)c1[j][0] ^ (uint32_t)c1[j][1] ^ (uint32_t)c1[j][2] ^ (uint32_t)c1[j][3];
        }
        #pragma unroll
        for (int st = 1; st < XCH; st *= 2)
          #pragma unroll
          for (int i = 0; i + st < XCH; i += 2 * st) xc[i] ^= xc[i + st];
        const uint32_t x = xc[0];
        xlast = x;
      }
      done[wi] = r + 1;
      ++r;
    };
    Quad xv0, xv1, xw0, xw1;

    for (int pair = 0; pair < pairs; pair += 4) {
      half(av0, av1, aw0, aw1, xv0, xv1, xw0, xw1, pair, true, 0, LEAD % RING);
      half(xv0, xv1, xw0, xw1, av0, av1, aw0, aw1, pair + 1, false, 1, (1 + LEAD) % RING);
      const uint32_t n0 = ((trr[0] << 13) | (trr[0] >> 19)) ^ xlast;
      half(av0, av1, aw0, aw1, xv0, xv1, xw0, xw1, pair + 2, true, 2, (2 + LEAD) % RING);
      half(xv0, xv1, xw0, xw1, av0, av1, aw0, aw1, pair + 3, false, 3, (3 + LEAD) % RING);
      const uint32_t n1 = ((trr[1] << 13) | (trr[1] >> 19)) ^ xlast;
      #pragma unroll
      for (int i = 0; i < 14; ++i) trr[i] = trr[i + 2];
      trr[14] = n0; trr[15] = n1;
    }
    uint32_t tr[16];
    #pragma unroll
    for (int i = 0; i < 16; ++i) tr[i] = trr[i];
    finish_tile(tr, key, target, hits, dump, band0 >> 1, strip, ncol, lane, band0 * 16 + g, t, col0);
  }
}

