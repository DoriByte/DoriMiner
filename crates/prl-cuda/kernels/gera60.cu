// DoriMiner - gera60.cu
// Copyright (C) 2026 J. Costa (DoriByte). Licensed under the PolyForm Noncommercial License 1.0.0 with the additional
// permission in MINING-PERMISSION (see LICENSE and MINING-PERMISSION at the top of this tree).

#include <cstdint>
#define NA 16
#define WARPS 8
#define RING 4
#define LEAD 2
#define NAP 100
#define GJ 4
#define XCH 4
#define BPOS 0

#define FASE 0
#define MT 3
#define MMA_K32(c,a,b) asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};" \
  : "+r"(c[0]), "+r"(c[1]), "+r"(c[2]), "+r"(c[3]) : "r"(a.w[0]), "r"(a.w[1]), "r"(a.w[2]), "r"(a.w[3]), "r"(b[0]), "r"(b[1]))
__device__ __forceinline__ uint32_t rotr(uint32_t x, int n) { return (x >> n) | (x << (32 - n)); }
#define G(a,b,c,d,x,y) a = a + b + x; d = rotr(d ^ a, 16); c = c + d; b = rotr(b ^ c, 12); a = a + b + y; d = rotr(d ^ a, 8); c = c + d; b = rotr(b ^ c, 7);
#define PAIRQ (NA * 32)
#define UTQ (128 * MT)
struct alignas(16) Quad { uint32_t w[4]; };

__device__ __forceinline__ void ldm(uint32_t addr, uint32_t r[4]) {
  asm volatile("ldmatrix.sync.aligned.m8n8.x4.trans.shared.b16 {%0,%1,%2,%3}, [%4];" : "=r"(r[0]), "=r"(r[1]), "=r"(r[2]), "=r"(r[3]) : "r"(addr));
}
__device__ __forceinline__ uint32_t dif(uint32_t a, uint32_t b) { return (a - b + 0x80808080u) ^ 0x80808080u; }

__device__ __forceinline__ void frag(uint32_t p, uint32_t m, Quad& a) {
  uint32_t x[4], y[4];
  ldm(p, x); ldm(m, y);
  const uint32_t d0 = dif(x[0], y[0]), d1 = dif(x[1], y[1]), d2 = dif(x[2], y[2]), d3 = dif(x[3], y[3]);
  a.w[0] = __byte_perm(d0, d1, 0x6420); a.w[1] = __byte_perm(d0, d1, 0x7531);
  a.w[2] = __byte_perm(d2, d3, 0x6420); a.w[3] = __byte_perm(d2, d3, 0x7531);
}

__device__ __forceinline__ void gera3(uint32_t base, uint32_t w, Quad (&a)[MT]) {
  const uint32_t p = base + (w >> 16), m = base + (w & 0xffffu);
  #pragma unroll
  for (int i = 0; i < MT; ++i) frag(p + 2048 * i, m + 2048 * i, a[i]);
}

#define R_SL 0
__device__ __forceinline__ void gera_eo(uint32_t vb, uint2 v, Quad& e, Quad& o) {
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
__device__ __forceinline__ void escreve_eo(Quad* ring, uint32_t slot, int wi, int lane, const Quad& e, const Quad& o) {
  uint2* const r2 = (uint2*)(ring + slot * PAIRQ);
  r2[(4 * wi) * 32 + lane] = make_uint2(e.w[0], e.w[1]); r2[(4 * wi + 1) * 32 + lane] = make_uint2(e.w[2], e.w[3]);
  r2[(4 * wi + 2) * 32 + lane] = make_uint2(o.w[0], o.w[1]); r2[(4 * wi + 3) * 32 + lane] = make_uint2(o.w[2], o.w[3]);
}

__device__ __forceinline__ void poe_b(Quad* ring, uint32_t slot, int wi, int lane, uint32_t vb, uint2 v) {
  Quad e, o;
  #pragma unroll
  for (int s = 0; s < 2; ++s) {
    const uint32_t w = s ? v.y : v.x;
    uint32_t x[4], y[4];
    ldm(vb + (w >> 16), x); ldm(vb + (w & 0xffffu), y);
    const uint32_t d0 = dif(x[0], y[0]), d1 = dif(x[1], y[1]), d2 = dif(x[2], y[2]), d3 = dif(x[3], y[3]);
    e.w[2 * s] = __byte_perm(d0, d1, 0x6420); e.w[2 * s + 1] = __byte_perm(d2, d3, 0x6420);
    o.w[2 * s] = __byte_perm(d0, d1, 0x7531); o.w[2 * s + 1] = __byte_perm(d2, d3, 0x7531);
  }
  uint2* const r2 = (uint2*)(ring + slot * PAIRQ);
  r2[(4 * wi) * 32 + lane] = make_uint2(e.w[0], e.w[1]); r2[(4 * wi + 1) * 32 + lane] = make_uint2(e.w[2], e.w[3]);
  r2[(4 * wi + 2) * 32 + lane] = make_uint2(o.w[0], o.w[1]); r2[(4 * wi + 3) * 32 + lane] = make_uint2(o.w[2], o.w[3]);
}

__device__ __forceinline__ void poke(Quad (&a)[MT], int slot, int jb, int v) {
  const uint32_t add = (uint32_t)v << (8 * jb), sbit = 1u << slot;
  #pragma unroll
  for (int s = 0; s < 4 * MT; ++s) {
    uint32_t& x = a[s >> 2].w[s & 3];
    if ((sbit >> s) & 1u) x = ((x ^ 0x80808080u) + add) ^ 0x80808080u;
  }
}
#define CBPOS 0
#define BMIX 0
#define FOLDI 0
#define DOBRA_ASM asm

__device__ __forceinline__ void dobra12t(uint32_t& x, const int32_t (&a)[4], const int32_t (&b)[4], const int32_t (&c)[4]) {
  DOBRA_ASM("{ .reg .b32 t, u2;\n\t"
      "lop3.b32 t, %1, %2, %3, 0x96;\n\t"
      "lop3.b32 u2, %4, %5, %6, 0x96;\n\t"
      "lop3.b32 t, t, u2, %7, 0x96;\n\t"
      "lop3.b32 u2, %8, %9, %10, 0x96;\n\t"
      "lop3.b32 t, t, u2, %11, 0x96;\n\t"
      "lop3.b32 %0, %0, t, %12, 0x96; }"
      : "+r"(x) : "r"(a[0]), "r"(a[1]), "r"(a[2]), "r"(a[3]), "r"(b[0]), "r"(b[1]), "r"(b[2]), "r"(b[3]),
                  "r"(c[0]), "r"(c[1]), "r"(c[2]), "r"(c[3]));
}

#define DOBRAJ(x, j) dobra12t(x, c[0][j], c[1][j], c[2][j])

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
  uint64_t pol;
  asm volatile("createpolicy.fractional.L2::evict_first.b64 %0, 1.0;" : "=l"(pol));
  #pragma unroll
  for (int q = 0; q < UTQ / 32; ++q)
    asm volatile("cp.async.cg.shared.global.L2::cache_hint [%0], [%1], 16, %2;" :: "r"(dst + (uint32_t)(q * 32 + lane) * 16), "l"(src + q * 32), "l"(pol) : "memory");
  asm volatile("cp.async.commit_group;" ::: "memory");
}

extern "C" __global__ void __launch_bounds__(32 * WARPS, 1)
prl_gera60(const uint32_t* Ut, const uint32_t* Bt, const uint32_t* key, const uint32_t* target,
           uint32_t* hits, int n, int k, uint32_t* dump, int m, int rows16, const uint32_t* Pt, uint32_t prc, int pv) {
  extern __shared__ Quad sm[];
  Quad* const tab = sm;
  Quad* const vtab = tab + WARPS * UTQ;
  Quad* const ring = vtab + 8 * 128;
  volatile uint32_t* const done = (volatile uint32_t*)(ring + RING * PAIRQ);
  const int tid = threadIdx.x, wi = tid >> 5, lane = tid & 31, g = lane >> 2, t = lane & 3;
  const int ncol = n / (8 * NA), pairs = (k >> 7) * 2;
  const int PS = (m >> 16) & 0xffff, PR = m & 0xffff;
  int strip = blockIdx.x % ncol, rb = blockIdx.x / ncol;
  if (PS) { const int i = blockIdx.x % (PR * PS), panel = blockIdx.x / (PR * PS), across = ncol / PS; strip = (panel % across) * PS + i % PS; rb = (panel / across) * PR + i / PS; }
  const int col0 = strip * 8 * NA;
  (void)rows16;
  const uint32_t mytab = (uint32_t)__cvta_generic_to_shared(tab + wi * UTQ);
  const uint32_t myv = (uint32_t)__cvta_generic_to_shared(vtab + (WARPS == 8 ? wi : wi & 7) * 128);
  const uint32_t morto = (uint32_t)__cvta_generic_to_shared(vtab);
  const uint2* const pa = (const uint2*)Pt + lane;
  const uint2* const pb = (const uint2*)(Pt + k) + lane;
  carrega_u(mytab, (const Quad*)Ut, (rb * LAPS) * WARPS + wi, lane);
  const bool fatia = WARPS == 8 || wi < 8;
  if (fatia) {
    const Quad* src = (const Quad*)Bt + (size_t)(strip * WARPS + wi) * 128 + lane;
    #pragma unroll
    for (int q = 0; q < 4; ++q)
      asm volatile("cp.async.cg.shared.global [%0], [%1], 16;" :: "r"(myv + (uint32_t)(q * 32 + lane) * 16), "l"(src + q * 32) : "memory");
    asm volatile("cp.async.commit_group;" ::: "memory");
  }
  done[wi] = 0;

  const int pr = (int)(prc >> 16), pc = (int)(prc & 0xffff);
  const int pgrp = pr / (16 * MT), pe = (pr % (16 * MT)) >> 3, pcc = pc & 31;
  const bool plane = pv != 0 && g == (pr & 7) && t == ((pcc & 15) >> 2);
  const int pstep = pc >> 5, pslot = (pe >> 1) * 4 + (pe & 1) + 2 * (pcc >> 4), pjb = pcc & 3;
  asm volatile("cp.async.wait_all;" ::: "memory");
  __syncwarp();
  #pragma unroll 1
  for (int q = 0; q < (R_SL >= 2 ? RING : LEAD); ++q) if (fatia) poe_b(ring, q, wi, lane, myv, pb[q * 32]);
  __syncthreads();
  uint32_t r = 0;
  uint2 vbn = pb[LEAD * 32];
  for (int lap = 0; lap < LAPS; ++lap) {
    const int grp = (rb * LAPS + lap) * WARPS + wi;
    const bool mine = plane && grp == pgrp;
    if (lap > 0) { asm volatile("cp.async.wait_all;" ::: "memory"); __syncwarp(); }
    volatile uint32_t trl[16];
    #pragma unroll
    for (int i = 0; i < 16; ++i) trl[i] = 0;
    int32_t c[MT][NA][4];
    #pragma unroll
    for (int i = 0; i < MT; ++i)
      #pragma unroll
      for (int j = 0; j < NA; ++j) { c[i][j][0] = c[i][j][1] = c[i][j][2] = c[i][j][3] = 0; }
    Quad ca[MT], cb[MT];
    uint2 vc = pa[0];
    gera3(mytab, vc.x, ca);
    if (mine && pstep == 0) poke(ca, pslot, pjb, pv);
    auto pair_ = [&](const int pair, const bool odd, const int rsl, const int wsl) {
      for (;;) {
        const uint32_t d = done[lane % WARPS];
        if (__all_sync(0xffffffffu, (int32_t)(r - d) < LEAD)) break;
        __nanosleep(NAP);
      }
      const bool last = pair + 1 == pairs;
      const uint2 vn = pa[((pair + 1) & (pairs - 1)) * 32];
      const uint2 vb = vbn;
      vbn = pb[((pair + LEAD + 1) & (pairs - 1)) * 32];
      const Quad* const rd = ring + rsl * PAIRQ + lane;
      auto gera_cb = [&]() { gera3(mytab, vc.y, cb); if (mine && pstep == 2 * pair + 1) poke(cb, pslot, pjb, pv); };

      auto gera_ca = [&]() { gera3(last ? morto : mytab, vn.x, ca); if (mine && pstep == 2 * pair + 2) poke(ca, pslot, pjb, pv); };

      uint32_t xc[XCH];
      #pragma unroll
      for (int i = 0; i < XCH; ++i) xc[i] = 0;
      auto dobra_grupo = [&](const int j0) {
        #pragma unroll
        for (int i = 0; i < GJ; ++i) DOBRAJ(xc[(j0 + i) % XCH], j0 + i);
      };
      auto passo = [&](const int s, const Quad (&a)[MT], const int j1, const int j2) {
        const bool dob = FOLDI && s == 1 && odd;
        #pragma unroll
        for (int j0 = j1; j0 < j2; j0 += GJ) {
          if (CBPOS && s == 0 && j0 == CBPOS * GJ) gera_cb();
          uint2 b[GJ];
          #pragma unroll
          for (int i = 0; i < GJ; ++i) {
            b[i] = ((const uint2*)(ring + rsl * PAIRQ))[(2 * (j0 + i) + s) * 32 + lane];
          }
          #pragma unroll
          for (int mi = 0; mi < MT; ++mi)
            #pragma unroll
            for (int i = 0; i < GJ; ++i) { uint32_t x[2] = {b[i].x, b[i].y}; MMA_K32(c[mi][j0 + i], a[mi], x); }
          if (dob && j0 > j1) dobra_grupo(j0 - GJ);
        }
        if (dob) dobra_grupo(j2 - GJ);
      };
      const bool depois = FASE && (wi & 4) != 0;
      const bool bmeio = BPOS == 1 || (BMIX && (wi & 4) != 0);
      if (!depois) {
        if (!CBPOS) gera_cb();
        passo(0, ca, 0, NA); gera_ca();
        if (bmeio) if (fatia) poe_b(ring, wsl, wi, lane, myv, vb);
        passo(1, cb, 0, NA);
        if (!bmeio) if (fatia) poe_b(ring, wsl, wi, lane, myv, vb);
      } else {
        passo(0, ca, 0, NA / 2); gera_cb(); passo(0, ca, NA / 2, NA);
        if (fatia) poe_b(ring, wsl, wi, lane, myv, vb);
        passo(1, cb, 0, NA / 2); gera_ca(); passo(1, cb, NA / 2, NA);
      }
      if (last && lap + 1 < LAPS) {

        __syncwarp();
        carrega_u(mytab, (const Quad*)Ut, grp + WARPS, lane);
      }
      vc = vn;
      if (odd) {

        #pragma unroll
        for (int j = 0; j < NA; ++j) DOBRAJ(xc[j % XCH], j);
        #pragma unroll
        for (int st = 1; st < XCH; st *= 2)
          #pragma unroll
          for (int i = 0; i + st < XCH; i += 2 * st) xc[i] ^= xc[i + st];
        const int w = (pair >> 1) & 15;
        const uint32_t o = trl[w];
        trl[w] = ((o << 13) | (o >> 19)) ^ xc[0];
      }
      done[wi] = r + 1;
      ++r;
    };

    for (int pair = 0; pair < pairs; pair += 4) {
      pair_(pair, false, 0, LEAD % RING);
      pair_(pair + 1, true, 1, (1 + LEAD) % RING);
      pair_(pair + 2, false, 2, (2 + LEAD) % RING);
      pair_(pair + 3, true, 3, (3 + LEAD) % RING);
    }
    uint32_t tr[16];
    #pragma unroll
    for (int i = 0; i < 16; ++i) tr[i] = trl[i];
    finish_tile(tr, key, target, hits, dump, grp, strip, ncol, lane, grp * 16 * MT + g, t, col0);
  }
}

