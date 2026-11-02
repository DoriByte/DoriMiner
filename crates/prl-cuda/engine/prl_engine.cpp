// DoriMiner - prl_engine.cpp
// Copyright (C) 2026 J. Costa (DoriByte). Licensed under the PolyForm Noncommercial License 1.0.0 with the additional
// permission in MINING-PERMISSION (see LICENSE and MINING-PERMISSION at the top of this tree).

#include "../../qtc-cuda/engine/motor.h"
#include "prl_images.inc"

namespace {

enum Series : uint32_t { RTX50 = 0, RTX40 = 1, RTX30 = 2, RTX50_T6 = 3, RTX30_T6 = 4 };

struct Shape {
    uint32_t m, n, k, na, h;
};
const uint32_t BLOCK = 256;
const int MAX_HITS = 63;

Shape series_shape(uint32_t s) {
    switch (s) {
        case RTX50_T6: return {67200, 4096, 8192, 16, 6};
        case RTX50: return {62720, 4096, 8192, 16, 4};
        case RTX30_T6: return {58368, 4096, 8192, 16, 6};
        default: return {65536, 4096, 8192, 16, 4};
    }
}
uint32_t bands_per_block(uint32_t s) {
    switch (s) {
        case RTX50_T6: return 120;
        case RTX30_T6: return 192;
        case RTX50: return 112;
        default: return 128;
    }
}
uint32_t shared_bytes(uint32_t s) {
    switch (s) {
        case RTX50: return 84032;
        case RTX50_T6: return 100416;
        default: return 98560;
    }
}
const uint8_t* series_image(uint32_t s, size_t* len) {
    switch (s) {
        case RTX50: *len = sizeof IMG_SM120; return IMG_SM120;
        case RTX40: *len = sizeof IMG_SM89; return IMG_SM89;
        case RTX30: *len = sizeof IMG_SM86; return IMG_SM86;
        case RTX50_T6: *len = sizeof IMG_SM120_6X32; return IMG_SM120_6X32;
        default: *len = sizeof IMG_SM86_6X32; return IMG_SM86_6X32;
    }
}
bool is_rtx50(uint32_t s) { return s == RTX50 || s == RTX50_T6; }
bool is_rtx30(uint32_t s) { return s == RTX30 || s == RTX30_T6; }

uint32_t blocks_of(uint32_t s, const Shape& sh) {
    const uint32_t bands = bands_per_block(s), rows16 = sh.m / 16;
    return (sh.n / (8 * sh.na)) * ((rows16 + bands - 1) / bands);
}

bool grid_covers(uint32_t s, const Shape& sh, int32_t panel, uint32_t* missed_out, uint32_t* past_out) {
    const size_t ncol = sh.n / (8 * sh.na), bands = bands_per_block(s), rbs = (sh.m / 16 + bands - 1) / bands;
    uint8_t* seen = (uint8_t*)calloc(ncol * rbs, 1);
    if (!seen) return false;
    const size_t ps = (size_t)((panel >> 16) & 0xffff), pr = (size_t)(panel & 0xffff), all = blocks_of(s, sh);
    uint32_t past = 0, missed = 0;
    for (size_t b = 0; b < all; b++) {
        size_t strip, rb;
        if (ps == 0) {
            strip = b % ncol;
            rb = b / ncol;
        } else {
            const size_t i = b % (pr * ps), p = b / (pr * ps), across = ncol / ps;
            strip = (p % across) * ps + i % ps;
            rb = (p / across) * pr + i / ps;
        }
        if (strip >= ncol || rb >= rbs) past++;
        else seen[rb * ncol + strip]++;
    }
    for (size_t i = 0; i < ncol * rbs; i++)
        if (seen[i] != 1) missed++;
    free(seen);
    if (missed_out) *missed_out = missed;
    if (past_out) *past_out = past;
    return missed == 0 && past == 0 && sh.m % (16 * bands) == 0;
}

typedef void (*RowsFn)(const uint8_t* seed, const uint8_t* label, uint32_t first, uint32_t count, uint8_t* out);

struct UArgs {
    const uint8_t* seed;
    const uint8_t* label;
    size_t gr, gw;
    bool sixteen;
    uint32_t* out;
    RowsFn rows;
};

void u_stretch(void* p, size_t from, size_t to) {
    const UArgs* a = (const UArgs*)p;
    uint8_t rows[48][128];
    const size_t parts = a->gr / 16;
    size_t order[4][4];
    for (size_t w = 0; w < 4; w++)
        for (size_t j = 0; j < 4; j++) order[w][j] = a->sixteen ? (4 * w + j) / 2 + 8 * ((4 * w + j) % 2) : 4 * w + j;
    for (size_t g = from; g < to; g++) {
        a->rows(a->seed, a->label, (uint32_t)(a->gr * g), (uint32_t)a->gr, &rows[0][0]);
        uint32_t* dst = a->out + g * a->gw;
        for (size_t part = 0; part < parts; part++) {
            const uint8_t(*base)[128] = rows + 16 * part;
            for (size_t l = 0; l < 128; l++) {
                for (size_t w = 0; w < 4; w++) {
                    dst[(part * 128 + l) * 4 + w] = (uint32_t)base[order[w][0]][l] | (uint32_t)base[order[w][1]][l] << 8 |
                                                    (uint32_t)base[order[w][2]][l] << 16 | (uint32_t)base[order[w][3]][l] << 24;
                }
            }
        }
    }
}

inline size_t kappa(size_t lane) {
    const size_t i = lane >> 3, r = lane & 7;
    return 16 * (i >> 1) + 4 * (r >> 1) + 2 * (i & 1) + (r & 1);
}

void pairs_table(const uint32_t* pm, size_t k, uint32_t* o) {
    for (size_t s = 0; s < k / 32; s++) {
        for (size_t i = 0; i < 16; i++) {
            const uint32_t a = pm[s * 32 + i], b = pm[s * 32 + 16 + i];
            *o++ = 16 * (a & 0xffff);
            *o++ = 16 * (b & 0xffff);
            *o++ = 16 * (a >> 16);
            *o++ = 16 * (b >> 16);
        }
    }
}

void pairs_table16(const uint32_t* pm, size_t k, const uint32_t* perm, uint32_t* o) {
    for (size_t q = 0; q < k / 64; q++) {
        for (size_t lane = 0; lane < 32; lane++) {
            for (size_t s = 0; s < 2; s++) {
                const size_t at = 32 * (2 * q + s) + kappa(lane);
                const uint32_t v = pm[perm ? perm[at] : at];
                *o++ = (16 * (v & 0xffff)) << 16 | 16 * (v >> 16);
            }
        }
    }
}

struct PermArgs {
    const uint32_t* pa;
    const uint32_t* pb;
    size_t voltas;
    uint32_t* out;
};

inline uint32_t group_cost(const uint8_t (*cg)[8]) {
    uint32_t total = 0;
    for (int side = 0; side < 4; side++) {
        uint8_t best = cg[side][0];
        for (int b = 1; b < 8; b++)
            if (cg[side][b] > best) best = cg[side][b];
        total += best;
    }
    return total;
}

void perm_stretch(void* p, size_t from, size_t to) {
    const PermArgs* a = (const PermArgs*)p;
    size_t fases[4][8];
    for (size_t i = 0; i < 4; i++)
        for (size_t r = 0; r < 8; r++) fases[i][r] = kappa(8 * i + r);
    for (size_t st = from; st < to; st++) {
        uint64_t x = 0x9E3779B97F4A7C15ull ^ ((uint64_t)st * 0xD1B54A32D192ED03ull);
        size_t cols[128];
        for (size_t c = 0; c < 128; c++) cols[c] = st * 128 + c;
        uint8_t counts[16][4][8];
        memset(counts, 0, sizeof counts);
        for (size_t i = 0; i < 128; i++) {
            const uint32_t pa = a->pa[cols[i]], pb = a->pb[cols[i]];
            counts[i / 8][0][(pa & 0xffff) % 8]++;
            counts[i / 8][1][(pa >> 16) % 8]++;
            counts[i / 8][2][(pb & 0xffff) % 8]++;
            counts[i / 8][3][(pb >> 16) % 8]++;
        }
        const auto rnd = [&x](size_t n) -> size_t {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            return (size_t)(x % (uint64_t)n);
        };
        for (size_t v = 0; v < a->voltas; v++) {
            const size_t ga = rnd(16), gb = rnd(16);
            if (ga == gb) continue;
            const size_t ia = ga * 8 + rnd(8);
            const size_t ib = gb * 8 + rnd(8);
            const uint32_t paa = a->pa[cols[ia]], pba = a->pb[cols[ia]], pab = a->pa[cols[ib]], pbb = a->pb[cols[ib]];
            const uint8_t la[4] = {(uint8_t)((paa & 0xffff) % 8), (uint8_t)((paa >> 16) % 8), (uint8_t)((pba & 0xffff) % 8), (uint8_t)((pba >> 16) % 8)};
            const uint8_t lb[4] = {(uint8_t)((pab & 0xffff) % 8), (uint8_t)((pab >> 16) % 8), (uint8_t)((pbb & 0xffff) % 8), (uint8_t)((pbb >> 16) % 8)};
            const uint32_t antes = group_cost(counts[ga]) + group_cost(counts[gb]);
            uint8_t na[4][8], nb[4][8];
            memcpy(na, counts[ga], sizeof na);
            memcpy(nb, counts[gb], sizeof nb);
            for (int side = 0; side < 4; side++) {
                na[side][la[side]]--;
                na[side][lb[side]]++;
                nb[side][lb[side]]--;
                nb[side][la[side]]++;
            }
            if (group_cost(na) + group_cost(nb) <= antes) {
                const size_t t = cols[ia];
                cols[ia] = cols[ib];
                cols[ib] = t;
                memcpy(counts[ga], na, sizeof na);
                memcpy(counts[gb], nb, sizeof nb);
            }
        }
        uint32_t* out = a->out + st * 128;
        for (size_t g = 0; g < 16; g++) {
            for (size_t r = 0; r < 8; r++) {
                const size_t at = 32 * (g / 4) + fases[g % 4][r];
                out[at % 128] = (uint32_t)cols[g * 8 + r];
            }
        }
    }
}

struct BArgs {
    const int8_t* b;
    size_t k, na, strip_words;
    bool room;
    uint32_t* out;
};

void b_stretch(void* p, size_t from, size_t to) {
    const BArgs* a = (const BArgs*)p;
    const size_t kw = a->k / 4, na = a->na;
    const uint8_t* b = (const uint8_t*)a->b;
    for (size_t strip = from; strip < to; strip++) {
        uint32_t* o = a->out + strip * a->strip_words;
        size_t made = 0;
        for (size_t stretch = 0; stretch < a->k / 128; stretch++) {
            for (size_t x = 0; x < 2 * na; x++) {
                const size_t j = x % na, half = x / na;
                for (size_t lane = 0; lane < 32; lane++) {
                    const size_t base = (strip * 8 * na + lane / 4 + 8 * j) * kw + stretch * 32 + half * 16 + lane % 4;
                    for (size_t t = 0; t < 4; t++) {
                        memcpy(o, b + 4 * (base + 4 * t), 4);
                        o++;
                    }
                    made += 4;
                    if (a->room && made % 2048 == 0) {
                        memset(o, 0, 128 * 4);
                        o += 128;
                    }
                }
            }
        }
    }
}

struct Tables {
    Slice u, pairs;
    uint32_t* perm;
};
struct Queued {
    Slice hits, key, limit;
    Tables tables;
};

struct Engine {
    Ctx ctx;
    uint32_t series;
    size_t threads;
    RowsFn rows;
    void* stream;
    void* side;
    void* module;
    void* fn;
    Slice b;
    bool has_b;
    Slice dump;
    Shape shape;
    int32_t panel;
    uint32_t grid;
    uint32_t* u_host;
    size_t u_cap;
    uint32_t* pairs_host;
    size_t pairs_cap;
    uint32_t* pb;
    size_t pb_len;
    int refs;
};

void tables_free(Ctx* c, Tables* t) {
    slice_free(c, &t->u);
    slice_free(c, &t->pairs);
    free(t->perm);
    t->perm = nullptr;
}

void engine_unref(Engine* e) {
    if (--e->refs > 0) return;
    Ctx* c = &e->ctx;
    keep(c, bind(c));
    if (e->side) stream_destroy(c, e->side);
    if (e->module) {
        keep(c, bind(c));
        keep(c, cu.cuModuleUnload(e->module));
    }
    if (e->has_b) slice_free(c, &e->b);
    slice_free(c, &e->dump);
    ctx_close(c);
    free(e->u_host);
    free(e->pairs_host);
    free(e->pb);
    free(e);
}

bool grow(uint32_t** p, size_t* cap, size_t words) {
    if (*cap >= words) return true;
    free(*p);
    *p = (uint32_t*)malloc(words * 4);
    *cap = *p ? words : 0;
    return *p != nullptr;
}

}

ENGINE_EXPORT int prl_series(uint32_t capability, uint32_t* series, char* err, int errlen) {
    int base;
    switch (capability) {
        case 120: base = RTX50; break;
        case 89: base = RTX40; break;
        case 86: base = RTX30; break;
        default: {
            char n[24];
            text_set(err, errlen, "no Pearl kernel for sm_", text_num(n, sizeof n, capability));
            return 1;
        }
    }
    int tile = 0;
    if (tile != 4 && base == RTX50) *series = RTX50_T6;
    else if (tile != 4 && base == RTX30) *series = RTX30_T6;
    else *series = (uint32_t)base;
    return 0;
}

ENGINE_EXPORT int prl_supported(uint32_t capability) { return capability == 120 || capability == 89 || capability == 86; }

ENGINE_EXPORT const char* prl_series_name(uint32_t series) {
    switch (series) {
        case RTX50: return "Rtx50";
        case RTX40: return "Rtx40";
        case RTX30: return "Rtx30";
        case RTX50_T6: return "Rtx50";
        default: return "Rtx30";
    }
}

ENGINE_EXPORT int prl_measured(void) { return PEARL_MEASURED; }

ENGINE_EXPORT void* prl_open(int ordinal, uint32_t series, uint32_t threads, RowsFn rows, char* err, int errlen) {
    if (!driver_load(err, errlen)) return nullptr;
    Engine* e = (Engine*)calloc(1, sizeof(Engine));
    if (!e) {
        text_set(err, errlen, "out of memory");
        return nullptr;
    }
    e->series = series;
    e->threads = threads ? threads : 1;
    e->rows = rows;
    e->refs = 1;
    Ctx* c = &e->ctx;
    const auto fail = [&](const char* what, cu_r r) -> void* {
        driver_error(err, errlen, what, r);
        if (c->ctx) engine_unref(e);
        else free(e);
        return nullptr;
    };
    cu_r r = ctx_open(c, ordinal);
    if (r) return fail("opening the card", r);
    size_t stored_len = 0;
    const uint8_t* stored = series_image(series, &stored_len);
    Unsealed plain = unseal(stored, stored_len);
    if (!plain.data) {
        text_set(err, errlen, "the embedded Pearl image does not open");
        engine_unref(e);
        return nullptr;
    }
    keep(c, bind(c));
    r = cu.cuModuleLoadData(&e->module, plain.data);
    free(plain.data);
    if (r) return fail("loading the Pearl kernel", r);
    switch (series) {
        case RTX50: r = cu.cuModuleGetFunction(&e->fn, e->module, HS("prl_gera17").c); break;
        case RTX40: r = cu.cuModuleGetFunction(&e->fn, e->module, HS("prl_gera29").c); break;
        case RTX30: r = cu.cuModuleGetFunction(&e->fn, e->module, HS("prl_gera34x").c); break;
        case RTX50_T6: r = cu.cuModuleGetFunction(&e->fn, e->module, HS("prl_gera40").c); break;
        default: r = cu.cuModuleGetFunction(&e->fn, e->module, HS("prl_gera60").c); break;
    }
    if (r) return fail("the Pearl kernel's function", r);
    keep(c, bind(c));
    if ((r = cu.cuFuncSetAttribute(e->fn, FUNC_DYNAMIC_SHARED, (int)shared_bytes(series)))) return fail("the card refuses the kernel's settings", r);
    e->stream = nullptr;
    if ((r = stream_new(c, &e->side))) return fail("opening the card", r);
    if ((r = slice_zeros(c, e->stream, 32 * 4, &e->dump))) return fail("opening the card", r);
    e->panel = is_rtx50(series) || series == RTX30_T6 ? (4 << 16) | 1 : (4 << 16) | 4;
    int processors = 0;
    if ((r = bind(c)) || (r = cu.cuDeviceGetAttribute(&processors, ATTR_MULTIPROCESSORS, c->device))) return fail("asking the card", r);
    e->shape = series_shape(series);
    if (is_rtx50(series) && processors == 84) e->shape.m = 21 * 16 * bands_per_block(series);
    if (is_rtx50(series) || series == RTX30_T6) {
        uint32_t missed = 0, past = 0;
        if (!grid_covers(series, e->shape, e->panel, &missed, &past)) {
            (void)missed;
            (void)past;
            text_set(err, errlen, "these options do not fit this card");
            engine_unref(e);
            return nullptr;
        }
    }
    e->grid = blocks_of(series, e->shape);
    return e;
}

ENGINE_EXPORT void prl_close(void* h) { engine_unref((Engine*)h); }

ENGINE_EXPORT void prl_shape(void* h, uint32_t* out) {
    const Shape& s = ((Engine*)h)->shape;
    out[0] = s.m;
    out[1] = s.n;
    out[2] = s.k;
    out[3] = s.na;
    out[4] = s.h;
}

ENGINE_EXPORT uint64_t prl_info_a(void* h) {
    const Shape& s = ((Engine*)h)->shape;
    return (uint64_t)(s.m < 65536 ? s.m : 65536) * s.k;
}

ENGINE_EXPORT int prl_info_b(void* h, uint32_t row, uint32_t* out) {
    const uint64_t m = ((Engine*)h)->shape.m;
    if ((uint64_t)row + 65536 < m) {
        *out = row + 65536;
        return 1;
    }
    return 0;
}

ENGINE_EXPORT int prl_job(void* h, const int8_t* b, const uint32_t* pb, size_t pb_len, const uint8_t* seed_b, char* err, int errlen) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    const Shape& s = e->shape;
    uint32_t* data = nullptr;
    size_t words = 0;
    const size_t strips = s.n / (8 * s.na);
    if (is_rtx30(e->series)) {
        if (!grow(&e->pb, &e->pb_len, pb_len)) {
            text_set(err, errlen, "out of memory");
            return 1;
        }
        memcpy(e->pb, pb, pb_len * 4);
        const size_t gr = 32, gw = 1024;
        const size_t groups = s.n / gr;
        words = groups * gw;
        data = (uint32_t*)malloc(words * 4);
        if (!data) {
            text_set(err, errlen, "out of memory");
            return 1;
        }
        UArgs a = {seed_b, (const uint8_t*)"B_tensor", gr, gw, true, data, e->rows};
        in_parallel(groups, e->threads, u_stretch, &a);
    } else {
        const bool room = is_rtx50(e->series);
        const size_t strip_words = (s.k / 64) * (2048 + (room ? 128 : 0));
        words = strips * strip_words;
        data = (uint32_t*)malloc(words * 4);
        if (!data) {
            text_set(err, errlen, "out of memory");
            return 1;
        }
        BArgs a = {b, s.k, s.na, strip_words, room, data};
        in_parallel(strips, e->threads, b_stretch, &a);
    }
    Slice fresh;
    const cu_r r = slice_from_host(c, e->stream, data, words * 4, &fresh);
    free(data);
    if (r) {
        driver_error(err, errlen, "preparing the job", r);
        return 1;
    }
    if (e->has_b) slice_free(c, &e->b);
    e->b = fresh;
    e->has_b = true;
    return 0;
}

ENGINE_EXPORT void* prl_prepare(void* h, const uint32_t* pm, const uint8_t* seed_a, char* err, int errlen) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    const Shape& s = e->shape;
    const size_t k = s.k, gr = e->series == RTX50_T6 ? 48 : 32, gw = gr / 16 * 512;
    if (s.m % gr != 0) {
        text_set(err, errlen, "these options do not fit this card");
        return nullptr;
    }
    Tables* t = (Tables*)calloc(1, sizeof(Tables));
    const size_t groups = s.m / gr;
    if (!t || !grow(&e->u_host, &e->u_cap, groups * gw)) {
        free(t);
        text_set(err, errlen, "out of memory");
        return nullptr;
    }
    size_t pair_words;
    if (is_rtx50(e->series)) pair_words = 2 * k * (s.n / (8 * s.na));
    else if (is_rtx30(e->series)) pair_words = 2 * k;
    else pair_words = k;
    if (!grow(&e->pairs_host, &e->pairs_cap, pair_words)) {
        free(t);
        text_set(err, errlen, "out of memory");
        return nullptr;
    }
    UArgs ua = {seed_a, (const uint8_t*)"A_tensor", gr, gw, e->series != RTX50 && e->series != RTX50_T6, e->u_host, e->rows};
    in_parallel(groups, e->threads, u_stretch, &ua);
    if (is_rtx50(e->series)) {
        const size_t strips = s.n / (8 * s.na), one = 2 * k;
        pairs_table(pm, k, e->pairs_host);
        for (size_t i = 1; i < strips; i++) memcpy(e->pairs_host + i * one, e->pairs_host, one * 4);
    } else if (e->series == RTX40) {
        pairs_table16(pm, k, nullptr, e->pairs_host);
    } else {
        if (e->pb_len < k) {
            free(t);
            text_set(err, errlen, "no job on the card");
            return nullptr;
        }
        t->perm = (uint32_t*)malloc(k * 4);
        if (!t->perm) {
            free(t);
            text_set(err, errlen, "out of memory");
            return nullptr;
        }
        PermArgs pa = {pm, e->pb, 40000, t->perm};
        in_parallel(k / 128, e->threads, perm_stretch, &pa);
        pairs_table16(pm, k, t->perm, e->pairs_host);
        pairs_table16(e->pb, k, t->perm, e->pairs_host + k);
    }
    cu_r r = slice_from_host(c, e->side, e->u_host, groups * gw * 4, &t->u);
    const char* what = "preparing a pair";
    if (!r) r = slice_from_host(c, e->side, e->pairs_host, pair_words * 4, &t->pairs);
    if (r) {
        driver_error(err, errlen, what, r);
        tables_free(c, t);
        free(t);
        return nullptr;
    }
    e->refs++;
    return t;
}

ENGINE_EXPORT void prl_prepare_drop(void* h, void* tables) {
    Engine* e = (Engine*)h;
    tables_free(&e->ctx, (Tables*)tables);
    free(tables);
    engine_unref(e);
}

ENGINE_EXPORT void* prl_launch(void* h, void* tables, const uint8_t* seed_a, uint64_t pos, int val, const uint32_t* limit, char* err, int errlen) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    const Shape s = e->shape;
    Tables* t = (Tables*)tables;
    Queued* q = (Queued*)calloc(1, sizeof(Queued));
    const auto drop = [&]() {
        tables_free(c, t);
        free(t);
        engine_unref(e);
    };
    if (!q) {
        text_set(err, errlen, "out of memory");
        drop();
        return nullptr;
    }
    if (pos / s.k > 0xffff) {
        text_set(err, errlen, "a pair out of range for this card");
        free(q);
        drop();
        return nullptr;
    }
    if (!e->has_b) {
        text_set(err, errlen, "no job on the card");
        free(q);
        drop();
        return nullptr;
    }
    const auto failed = [&](const char* what, cu_r r) -> void* {
        driver_error(err, errlen, what, r);
        slice_free(c, &q->hits);
        slice_free(c, &q->limit);
        slice_free(c, &q->key);
        free(q);
        drop();
        return nullptr;
    };
    cu_r r;
    if (is_rtx50(e->series)) {
        before_read(c, &t->pairs, e->stream);
        before_read(c, &e->b, e->stream);
        const size_t width = 512, height = s.n / (8 * s.na) * (s.k / 64);
        Copy2D copy;
        memset(&copy, 0, sizeof copy);
        copy.srcMemoryType = MEM_DEVICE;
        copy.srcDevice = t->pairs.p;
        copy.srcPitch = width;
        copy.dstMemoryType = MEM_DEVICE;
        copy.dstDevice = e->b.p + 8192;
        copy.dstPitch = 8704;
        copy.WidthInBytes = width;
        copy.Height = height;
        r = cu.cuMemcpy2DAsync_v2(&copy, e->stream);
        if (r) return failed("queuing the search", r);
        after_read(c, &e->b, e->stream);
        after_read(c, &t->pairs, e->stream);
    }
    uint32_t key[8];
    for (int i = 0; i < 8; i++) key[i] = (uint32_t)seed_a[4 * i] | (uint32_t)seed_a[4 * i + 1] << 8 | (uint32_t)seed_a[4 * i + 2] << 16 | (uint32_t)seed_a[4 * i + 3] << 24;
    if ((r = slice_from_host(c, e->side, key, sizeof key, &q->key))) return failed("queuing the search", r);
    if ((r = slice_from_host(c, e->side, limit, 32, &q->limit))) return failed("queuing the search", r);
    if ((r = slice_zeros(c, e->side, (size_t)(MAX_HITS + 1) * 4, &q->hits))) return failed("queuing the search", r);
    const int32_t n = (int32_t)s.n, k = (int32_t)s.k, rows16 = (int32_t)(s.m / 16);
    const size_t row = (size_t)(pos / s.k);
    size_t col = (size_t)(pos % s.k);
    if (t->perm) {
        size_t at = 0;
        while (at < s.k && t->perm[at] != col) at++;
        col = at;
    }
    const uint32_t byte_at = (uint32_t)((row << 16) | col);
    const int32_t byte = val;
    Launch l;
    launch_start(&l, c);
    arg_read(&l, &t->u);
    arg_read(&l, &e->b);
    arg_read(&l, &q->key);
    arg_read(&l, &q->limit);
    arg_write(&l, &q->hits);
    arg_value(&l, &n);
    arg_value(&l, &k);
    arg_write(&l, &e->dump);
    arg_value(&l, &e->panel);
    arg_value(&l, &rows16);
    arg_read(&l, &t->pairs);
    arg_value(&l, &byte_at);
    arg_value(&l, &byte);
    if ((r = launch_go(&l, e->fn, e->grid, BLOCK, shared_bytes(e->series), e->stream))) return failed("launching the Pearl kernel", r);
    q->tables = *t;
    free(t);
    return q;
}

ENGINE_EXPORT int64_t prl_read(void* h, void* queued, uint32_t* out, uint32_t* kept, char* err, int errlen) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    Queued* q = (Queued*)queued;
    uint32_t raw[MAX_HITS + 1];
    const cu_r r = slice_to_host(c, e->side, &q->hits, raw);
    int64_t won = -1;
    if (r) {
        driver_error(err, errlen, "reading the tiles found", r);
    } else {
        won = raw[0];
        const uint32_t count = raw[0] < (uint32_t)MAX_HITS ? raw[0] : (uint32_t)MAX_HITS;
        memcpy(out, raw + 1, count * 4);
        *kept = count;
    }
    slice_free(c, &q->hits);
    slice_free(c, &q->key);
    slice_free(c, &q->limit);
    tables_free(c, &q->tables);
    free(q);
    engine_unref(e);
    return won;
}

ENGINE_EXPORT void prl_read_drop(void* h, void* queued) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    Queued* q = (Queued*)queued;
    slice_free(c, &q->hits);
    slice_free(c, &q->key);
    slice_free(c, &q->limit);
    tables_free(c, &q->tables);
    free(q);
    engine_unref(e);
}

