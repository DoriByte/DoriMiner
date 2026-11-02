// DoriMiner - qtc_engine.cpp
// Copyright (C) 2026 J. Costa (DoriByte). Licensed under the PolyForm Noncommercial License 1.0.0 with the additional
// permission in MINING-PERMISSION (see LICENSE and MINING-PERMISSION at the top of this tree).

#include "motor.h"
#include "qtc_images.inc"
#ifdef _MSC_VER
#include <intrin.h>
#endif

namespace {

const uint64_t P = 0xffffffff00000001ull, EPS = 0xffffffffull;

inline uint64_t fadd(uint64_t a, uint64_t b) {
    uint64_t s = a + b;
    if (s < a) s += EPS;
    return s >= P ? s - P : s;
}
inline uint64_t fsub(uint64_t a, uint64_t b) { return a >= b ? a - b : a + (P - b); }
inline uint64_t fmul(uint64_t a, uint64_t b) {
#ifdef _MSC_VER
    uint64_t hi, lo = _umul128(a, b, &hi);
#else
    const unsigned __int128 w = (unsigned __int128)a * b;
    const uint64_t lo = (uint64_t)w, hi = (uint64_t)(w >> 64);
#endif
    const uint64_t hh = hi >> 32, hl = hi & EPS;
    uint64_t t0 = lo - hh;
    if (lo < hh) t0 -= EPS;
    const uint64_t t1 = hl * EPS;
    uint64_t r = t0 + t1;
    if (r < t0) r += EPS;
    return r >= P ? r - P : r;
}
inline uint64_t fexp7(uint64_t x) {
    const uint64_t x2 = fmul(x, x), x3 = fmul(x2, x), x4 = fmul(x2, x2);
    return fmul(x3, x4);
}

const uint64_t RC0[12] = {0xc002e770975b1607ull, 0xbca51a8dfe14593aull, 0x72938dfbe774f7f9ull, 0xe4f2fe29e03234acull,
                          0xd5e0ba2f541b6449ull, 0xec33b868f3cc46c1ull, 0x486dcb55419d475aull, 0x6c1cb2a358cc24f1ull,
                          0xe3f30d509a1436bbull, 0xd9a64f068dca7c29ull, 0xe59b3f57aabba1aeull, 0x2a3dd4505b478fdcull};
const uint64_t RC1[12] = {0xada1f8dc7676ed25ull, 0x2711aa8b5509d516ull, 0x4ae6acd0c9c92897ull, 0x56eb3d6b5256d67aull,
                          0x1f7a9d55923bf51eull, 0x3600427d397a7f68ull, 0xe5076df75b72c3d0ull, 0xfcd59aa12c6090adull,
                          0xcd895e8c68b57a9eull, 0x41df7ef9d730ae3eull, 0xee3e2b889abe977dull, 0xd29bb7edbeb9c405ull};

void layer(uint64_t* s) {
    for (int c = 0; c < 3; c++) {
        uint64_t* v = s + 4 * c;
        const uint64_t t01 = fadd(v[0], v[1]), t23 = fadd(v[2], v[3]), t0123 = fadd(t01, t23);
        const uint64_t t01123 = fadd(t0123, v[1]), t01233 = fadd(t0123, v[3]);
        v[3] = fadd(fadd(t01233, v[0]), v[0]);
        v[1] = fadd(fadd(t01123, v[2]), v[2]);
        v[0] = fadd(t01123, t01);
        v[2] = fadd(t01233, t23);
    }
    uint64_t sums[4];
    for (int o = 0; o < 4; o++) sums[o] = fadd(fadd(s[o], s[o + 4]), s[o + 8]);
    for (int i = 0; i < 12; i++) s[i] = fadd(s[i], sums[i % 4]);
}

enum Scheme : uint32_t { SCHEME_ONE = 0, SCHEME_TWO = 1 };
const int64_t DIR[8] = {17, -11, 3, -4, -17, 11, -3, 4};
const int64_t DIR2[8] = {-11, 3, -4, 17, 11, -3, 4, -17};
const uint64_t STEP = 35;
const uint32_t BASE = 0x40000000u;
const uint32_t MAX_INDEX = 60000000u;
const uint32_t MAX_LAUNCH = 1u << 30;
const uint32_t TWO_PER_THREAD = 32;
const int TWO_ROW = 9;
const int ROW_LANES[TWO_ROW] = {0, 2, 3, 4, 6, 7, 8, 10, 11};
const uint32_t TWO_DEFAULT = 16;
const uint32_t MAX_BLOCKS = 4096;

inline void low_felts(uint32_t scheme, uint32_t launch, uint32_t thread, uint32_t step, uint32_t* f) {
    for (int i = 0; i < 8; i++) {
        const int64_t off = scheme == SCHEME_TWO ? DIR[i] * (int64_t)step + DIR2[i] * (int64_t)thread : DIR[i] * (int64_t)step;
        f[i] = (uint32_t)((int64_t)BASE + off);
    }
    f[0] += launch;
}

void first_inputs(const uint64_t* mid, uint32_t launch, uint64_t* s) {
    uint32_t low[8];
    low_felts(SCHEME_ONE, launch, 0, 0, low);
    for (int i = 0; i < 12; i++) s[i] = i < 8 ? fadd(mid[i], low[i]) : mid[i];
    layer(s);
    for (int i = 0; i < 12; i++) s[i] = fadd(s[i], RC0[i] >= P ? RC0[i] - P : RC0[i]);
}

struct Params {
    uint32_t round1[24];
    uint32_t lanes[4];
    uint32_t target[16];
    uint32_t config[3];
    uint64_t rows[TWO_PER_THREAD][TWO_ROW];
};

void make_params(uint32_t scheme, const uint64_t* mid, const uint32_t* target, uint32_t epoch, uint32_t launch, const uint32_t* config, Params* p) {
    (void)epoch;
    memset(p, 0, sizeof *p);
    uint64_t s[12];
    first_inputs(mid, launch, s);
    uint64_t lane_a, lane_b;
    if (scheme == SCHEME_TWO) {
        lane_a = s[2];
        lane_b = s[6];
        const uint64_t l3 = s[3], l7 = s[7];
        for (int i = 0; i < 12; i++) s[i] = (i == 2 || i == 3 || i == 6 || i == 7) ? 0 : fexp7(s[i]);
        layer(s);
        const uint32_t rows = config[1] < TWO_PER_THREAD ? config[1] : TWO_PER_THREAD;
        for (uint32_t step = 0; step < rows; step++) {
            uint64_t u[12] = {0};
            const uint64_t d = STEP * step;
            u[3] = fexp7(fadd(l3, d));
            u[7] = fexp7(fsub(l7, d));
            layer(u);
            for (int k = 0; k < TWO_ROW; k++) p->rows[step][k] = u[ROW_LANES[k]];
        }
    } else {
        lane_a = s[3];
        lane_b = s[7];
        for (int i = 0; i < 12; i++) s[i] = (i == 3 || i == 7) ? 0 : fexp7(s[i]);
        layer(s);
    }
    for (int i = 0; i < 12; i++) {
        const uint64_t v = fadd(s[i], RC1[i] >= P ? RC1[i] - P : RC1[i]);
        p->round1[2 * i] = (uint32_t)v;
        p->round1[2 * i + 1] = (uint32_t)(v >> 32);
    }
    p->lanes[0] = (uint32_t)lane_a;
    p->lanes[1] = (uint32_t)(lane_a >> 32);
    p->lanes[2] = (uint32_t)lane_b;
    p->lanes[3] = (uint32_t)(lane_b >> 32);
    memcpy(p->target, target, sizeof p->target);
    memcpy(p->config, config, sizeof p->config);
}

inline uint32_t fit_per_thread(uint32_t scheme, uint32_t n) {
    const uint32_t most = scheme == SCHEME_TWO ? TWO_PER_THREAD : 0xffffffffu;
    return n < 1 ? 1 : n > most ? most : n;
}
inline uint32_t ceil_div(uint32_t a, uint32_t b) { return a / b + (a % b != 0); }

uint32_t default_per_thread(uint32_t scheme, uint32_t count, uint32_t block_threads) {
    const uint32_t grid = ceil_div(count, MAX_BLOCKS * block_threads);
    if (scheme == SCHEME_TWO) return fit_per_thread(scheme, grid > TWO_DEFAULT ? grid : TWO_DEFAULT);
    return grid > 1 ? grid : 1;
}

void dispatch(uint32_t scheme, uint32_t count, bool asked, uint32_t per, uint32_t block_threads, uint32_t* out) {
    bool fixed = asked;
    uint32_t want = per;
    if (!fixed && scheme == SCHEME_TWO) {
        fixed = true;
        want = default_per_thread(scheme, count, block_threads);
    }
    if (fixed) {
        uint32_t n = fit_per_thread(scheme, want);
        const uint32_t cap = (0xffffffffu - count) / block_threads;
        if (n > cap) n = cap;
        if (n < 1) n = 1;
        uint32_t blocks = ceil_div(ceil_div(count, n), block_threads);
        if (blocks < 1) blocks = 1;
        out[0] = blocks;
        out[1] = blocks * block_threads;
        out[2] = n;
        return;
    }
    uint32_t blocks = ceil_div(count, block_threads);
    if (blocks < 1) blocks = 1;
    if (blocks > MAX_BLOCKS) blocks = MAX_BLOCKS;
    const uint32_t threads = blocks * block_threads;
    uint32_t n = ceil_div(count, threads);
    if (n < 1) n = 1;
    out[0] = blocks;
    out[1] = threads;
    out[2] = n;
}

uint32_t arch_block_threads(uint32_t arch) {
    for (size_t i = 0; i < IMG_ARCHS; i++)
        if (IMG_ARCH[i].arch == arch) return IMG_ARCH[i].block_threads;
    return 256;
}
uint32_t arch_scheme(uint32_t arch) {
    for (size_t i = 0; i < IMG_ARCHS; i++)
        if (IMG_ARCH[i].arch == arch && IMG_ARCH[i].two) return SCHEME_TWO;
    return SCHEME_ONE;
}

uint64_t skipped_nonces(uint32_t count, uint32_t per, uint32_t blocks, uint32_t block_threads) {
    const uint64_t block = (uint64_t)block_threads * (per > 1 ? per : 1), full = count / block, rest = count % block;
    return (blocks < full ? blocks : full) * block + (blocks > full ? rest : 0);
}

#define QTC_HASH_BLOCK 256
const size_t RESULT_WORDS = 1 + 8;
const size_t MAX_HITS = 8;

typedef void (*PermuteFn)(uint64_t* state);

void make_mid(const uint8_t* header, const uint8_t* high, PermuteFn permute, uint64_t* s) {
    memset(s, 0, 12 * 8);
    for (int i = 0; i < 8; i++) s[i] = fadd(s[i], (uint64_t)header[4 * i] | (uint64_t)header[4 * i + 1] << 8 | (uint64_t)header[4 * i + 2] << 16 | (uint64_t)header[4 * i + 3] << 24);
    permute(s);
    for (int i = 0; i < 8; i++) s[i] = fadd(s[i], (uint64_t)high[4 * i] | (uint64_t)high[4 * i + 1] << 8 | (uint64_t)high[4 * i + 2] << 16 | (uint64_t)high[4 * i + 3] << 24);
    permute(s);
}

struct SlotTiming {
    void* start;
    void* cleared;
    void* end;
};
struct Slot {
    void* stream;
    Slice results;
    void* host;
    void* host_event;
    void* done;
    SlotTiming timing;
    bool timed;
    int lane;
};
struct Flight {
    int slot;
    uint32_t launch, count, per_thread;
    uint32_t scheme;
};
struct Abort {
    void* stream;
    void* host;
    void* host_event;
    uint32_t newest;
    bool on;
};
struct GpuTiming {
    void* start;
    void* cleared;
    void* end;
    void* previous_end;
    bool has_previous;
    bool on;
};

struct Engine {
    Ctx ctx;
    uint32_t arch, capability, scheme, block_threads;
    void* stream;
    void* module;
    void* mine;
    void* hash;
    int registers, local_bytes;
    Slice results;
    Slice abort_word;
    Abort abort;
    GpuTiming timing;
    Slot slots[8];
    int nslots, nlanes, next_slot;
    void* lanes[8];
    Flight flights[8];
    int fhead, fcount;
    bool asked;
    uint32_t asked_per;
    unsigned wait_flag;
    PermuteFn permute;
    Params params;
    uint32_t stage_blocks, stage_per, stage_launch, stage_count;
    Flight waiting;
    bool has_waiting;
    uint64_t gpu_gap, gpu_clear, gpu_kernel, gpu_launches;
};

void pinned_free(Ctx* c, void* host, void* ev) {
    keep(c, bind(c));
    keep(c, cu.cuEventSynchronize(ev));
    keep(c, cu.cuMemFreeHost(host));
    event_destroy(c, ev);
}

cu_r pinned_new(Ctx* c, size_t bytes, void** host, void** ev) {
    cu_r r = bind(c);
    if (r) return r;
    if ((r = event_new(c, ev, EVENT_BLOCKING_SYNC))) return r;
    if ((r = cu.cuMemHostAlloc(host, bytes, 0))) return r;
    return cu.cuEventSynchronize(*ev);
}

void slots_clear(Engine* e) {
    Ctx* c = &e->ctx;
    for (int i = 0; i < e->nslots; i++) {
        Slot* s = &e->slots[i];

        bool last = s->lane > 0;
        for (int j = i + 1; j < e->nslots; j++)
            if (e->slots[j].lane == s->lane) last = false;
        if (last) stream_destroy(c, s->stream);
        slice_free(c, &s->results);
        pinned_free(c, s->host, s->host_event);
        event_destroy(c, s->done);
        if (s->timed) {
            event_destroy(c, s->timing.start);
            event_destroy(c, s->timing.cleared);
            event_destroy(c, s->timing.end);
        }
    }
    e->nslots = 0;
    e->next_slot = 0;
}

cu_r elapsed_ms(Ctx* c, void* a, void* b, float* out) {
    cu_r r = bind(c);
    if (r) return r;
    if ((r = cu.cuEventSynchronize(a))) return r;
    if ((r = cu.cuEventSynchronize(b))) return r;
    return cu.cuEventElapsedTime(out, a, b);
}
inline uint64_t ms_ns(float ms) {
    if (ms < 0) ms = 0;
    return (uint64_t)((double)ms * 1e6);
}

struct Outcome {
    uint64_t skipped;
    uint32_t reported;
    uint32_t kept;
    uint32_t per_thread;
};

int read_out(Engine* e, const Flight* f, const uint32_t* raw, Outcome* o, uint8_t* lows, char* err, int errlen) {
    const uint32_t reported = raw[0];
    (void)e;
    o->skipped = 0;
    o->reported = reported;
    o->per_thread = f->per_thread;
    const uint32_t kept = reported < MAX_HITS ? reported : (uint32_t)MAX_HITS;
    o->kept = kept;
    for (uint32_t i = 0; i < kept; i++) {
        const uint32_t index = raw[1 + i];
        if (index >= f->count) {
            char a[24], b[24];
            text_set(err, errlen, "the kernel returned candidate index ", text_num(a, sizeof a, index), " for a launch of ", text_num(b, sizeof b, f->count));
            return 1;
        }
        uint32_t felts[8];
        const uint32_t per = f->per_thread > 1 ? f->per_thread : 1;
        if (f->scheme == SCHEME_TWO) low_felts(SCHEME_TWO, f->launch, index / per, index % per, felts);
        else low_felts(SCHEME_ONE, f->launch, 0, index, felts);
        memcpy(lows + 32 * i, felts, 32);
    }
    return 0;
}

}

#define TXT(s) "the engine could not run the launch"

ENGINE_EXPORT uint32_t qtc_const(int which) {
    switch (which) {
        case 0: return MAX_INDEX;
        case 1: return MAX_LAUNCH;
        case 3: return 2;
        case 4: return 2;
        default: return 0;
    }
}

ENGINE_EXPORT uint32_t qtc_arch_for(uint32_t capability) {
    uint32_t best = 0;
    for (size_t i = 0; i < IMG_ARCHS; i++)
        if (IMG_ARCH[i].arch <= capability && IMG_ARCH[i].arch > best) best = IMG_ARCH[i].arch;
    return best;
}
ENGINE_EXPORT uint32_t qtc_lim_a(uint32_t scheme) { return scheme == SCHEME_TWO ? TWO_PER_THREAD : 0xffffffffu; }
ENGINE_EXPORT uint32_t qtc_lim_b(uint32_t scheme, uint32_t n) { return fit_per_thread(scheme, n); }
ENGINE_EXPORT uint32_t qtc_lim_c(uint32_t scheme, uint32_t count, uint32_t block_threads) { return default_per_thread(scheme, count, block_threads); }

ENGINE_EXPORT void qtc_init(const uint8_t* header, const uint8_t* high, PermuteFn permute, uint64_t* out) { make_mid(header, high, permute, out); }

ENGINE_EXPORT int qtc_chk_a(const uint64_t* state, uint32_t scheme, uint32_t launch, uint32_t count, uint32_t per) {
    uint64_t in[12];
    first_inputs(state, launch, in);
    const uint64_t SMALL = 1ull << 11;
    uint64_t up, down;
    uint32_t steps;
    if (scheme == SCHEME_TWO) {
        up = in[2];
        down = in[6];
        steps = ceil_div(count, per > 1 ? per : 1);
    } else {
        up = in[3];
        down = in[7];
        steps = count;
    }
    const uint64_t last = STEP * (uint64_t)(steps ? steps - 1 : 0);
    const auto reaches = [&](uint64_t low, uint64_t high) { return down >= low && down <= high + last; };
    const bool lands_small = up < SMALL || up >= P - last;
    return reaches(2, SMALL) || reaches((1ull << 31) - 1, (1ull << 32) - 1) || lands_small;
}

ENGINE_EXPORT void qtc_val_a(uint32_t scheme, uint32_t launch, uint32_t per, uint32_t index, uint8_t* out) {
    uint32_t f[8];
    if (per < 1) per = 1;
    if (scheme == SCHEME_TWO) low_felts(SCHEME_TWO, launch, index / per, index % per, f);
    else low_felts(SCHEME_ONE, launch, 0, index, f);
    for (int i = 0; i < 8; i++) {
        out[4 * i] = (uint8_t)f[i];
        out[4 * i + 1] = (uint8_t)(f[i] >> 8);
        out[4 * i + 2] = (uint8_t)(f[i] >> 16);
        out[4 * i + 3] = (uint8_t)(f[i] >> 24);
    }
}

ENGINE_EXPORT uint32_t qtc_opt_f(uint32_t count, uint32_t per, uint32_t wave, uint32_t block_threads) {
    const uint64_t block = (uint64_t)block_threads * (per > 1 ? per : 1), w = wave > 1 ? wave : 1;
    uint64_t blocks = ((uint64_t)count + block * w / 2) / (block * w);
    if (blocks < 1) blocks = 1;
    blocks *= w;
    while (blocks > w && blocks * block > MAX_INDEX) blocks -= w;
    return blocks * block <= MAX_INDEX ? (uint32_t)(blocks * block) : count;
}

ENGINE_EXPORT void* qtc_open(int ordinal, PermuteFn permute, uint32_t* info, char* err, int errlen) {
    if (!driver_load(err, errlen)) return nullptr;
    Engine* e = (Engine*)calloc(1, sizeof(Engine));
    if (!e) {
        text_set(err, errlen, "out of memory");
        return nullptr;
    }
    e->permute = permute;
    e->wait_flag = EVENT_BLOCKING_SYNC;
    Ctx* c = &e->ctx;
    const auto fail = [&](const char* what, cu_r r) -> void* {
        driver_error(err, errlen, what, r);
        if (c->ctx) ctx_close(c);
        free(e);
        return nullptr;
    };
    char head[64], num[24];
    text_set(head, sizeof head, "could not open CUDA device ", text_num(num, sizeof num, ordinal));
    cu_r r = ctx_open(c, ordinal);
    if (r) return fail(head, r);
    if ((r = cu.cuCtxSetFlags(CTX_BLOCKING))) return fail(TXT("could not enable blocking synchronisation"), r);
    int major = 0, minor = 0;
    if ((r = cu.cuDeviceGetAttribute(&major, ATTR_CC_MAJOR, c->device)) || (r = cu.cuDeviceGetAttribute(&minor, ATTR_CC_MINOR, c->device))) return fail("could not query the CUDA device", r);
    e->capability = (uint32_t)(major * 10 + minor);
    e->arch = qtc_arch_for(e->capability);
    if (!e->arch) {
        text_set(err, errlen, "this card (sm_", text_num(num, sizeof num, e->capability), ") is older than any kernel this build carries");
        ctx_close(c);
        free(e);
        return nullptr;
    }
    e->scheme = arch_scheme(e->arch);
    e->block_threads = arch_block_threads(e->arch);
    cu_r last = 0;
    {
        last = -1;
        for (size_t i = 0; i < IMG_COUNT; i++) {
            if (IMG_LIST[i].arch != e->arch) continue;
            Unsealed plain = unseal(IMG_LIST[i].data, IMG_LIST[i].len);
            if (!plain.data) continue;
            keep(c, bind(c));
            last = cu.cuModuleLoadData(&e->module, plain.data);
            free(plain.data);
            if (!last) break;
        }
    }
    if (last) {
        if (last == -1) {
            text_set(err, errlen, "no image embedded for this card");
            ctx_close(c);
            free(e);
            return nullptr;
        }
        char what[64];
        text_set(what, sizeof what, "could not load the sm_", text_num(num, sizeof num, e->arch), " kernel");
        return fail(what, last);
    }
    if ((r = cu.cuModuleGetFunction(&e->mine, e->module, HS("mining_main").c))) return fail("could not find the kernel", r);
    if ((r = cu.cuModuleGetFunction(&e->hash, e->module, HS("hash_nonces").c))) return fail("could not find the kernel", r);
    keep(c, bind(c));
    if (cu.cuFuncGetAttribute(&e->registers, FUNC_REGS, e->mine)) e->registers = -1;
    keep(c, bind(c));
    if (cu.cuFuncGetAttribute(&e->local_bytes, FUNC_LOCAL_BYTES, e->mine)) e->local_bytes = -1;
    e->stream = nullptr;
    if ((r = slice_zeros(c, e->stream, RESULT_WORDS * 4, &e->results))) return fail("could not allocate a buffer", r);
    if ((r = slice_zeros(c, e->stream, 4, &e->abort_word))) return fail("could not allocate a buffer", r);
    info[0] = e->arch;
    info[1] = e->capability;
    info[2] = e->scheme;
    info[3] = e->block_threads;
    return e;
}

ENGINE_EXPORT void qtc_close(void* h) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    if (e->fcount > 0) {
        keep(c, bind(c));
        keep(c, cu.cuCtxSynchronize());
    }
    slice_free(c, &e->results);
    keep(c, bind(c));
    keep(c, cu.cuModuleUnload(e->module));
    if (e->timing.on) {
        event_destroy(c, e->timing.start);
        event_destroy(c, e->timing.cleared);
        event_destroy(c, e->timing.end);
        event_destroy(c, e->timing.previous_end);
    }
    slots_clear(e);
    slice_free(c, &e->abort_word);
    if (e->abort.on) {
        stream_destroy(c, e->abort.stream);
        pinned_free(c, e->abort.host, e->abort.host_event);
    }
    ctx_close(c);
    free(e);
}

ENGINE_EXPORT int qtc_opt_a(void* h, int mode, char* err, int errlen) {
    Engine* e = (Engine*)h;
    const unsigned flags = mode == 1 ? CTX_YIELD : mode == 2 ? CTX_SPIN : CTX_BLOCKING;
    cu_r r = bind(&e->ctx);
    if (!r) r = cu.cuCtxSetFlags(flags);
    if (r) {
        driver_error(err, errlen, TXT("could not change how the thread waits for the card"), r);
        return 1;
    }
    e->wait_flag = mode == 0 ? EVENT_BLOCKING_SYNC : EVENT_DEFAULT;
    return 0;
}

ENGINE_EXPORT void qtc_opt_b(void* h, int asked, uint32_t per) {
    Engine* e = (Engine*)h;
    e->asked = asked != 0;
    e->asked_per = fit_per_thread(e->scheme, per);
}

ENGINE_EXPORT uint32_t qtc_get_b(void* h, uint32_t count) {
    Engine* e = (Engine*)h;
    uint32_t d[3];
    dispatch(e->scheme, count, e->asked, e->asked_per, e->block_threads, d);
    return d[2];
}

ENGINE_EXPORT int qtc_get_c(void* h, uint32_t* out, char* err, int errlen) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    int processors = 0, per = 0;
    cu_r r = bind(c);
    if (!r) r = cu.cuDeviceGetAttribute(&processors, ATTR_MULTIPROCESSORS, c->device);
    if (!r) r = cu.cuOccupancyMaxActiveBlocksPerMultiprocessorWithFlags(&per, e->mine, (int)e->block_threads, 0, 0);
    if (r) {
        driver_error(err, errlen, "could not query the kernel", r);
        return 1;
    }
    *out = (uint32_t)(processors > 1 ? processors : 1) * (uint32_t)(per > 1 ? per : 1);
    return 0;
}

ENGINE_EXPORT int qtc_opt_t(void* h, char* err, int errlen) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    if (e->timing.on) return 0;
    cu_r r = event_new(c, &e->timing.start, EVENT_DEFAULT);
    if (!r) r = event_new(c, &e->timing.cleared, EVENT_DEFAULT);
    if (!r) r = event_new(c, &e->timing.end, EVENT_DEFAULT);
    if (!r) r = event_new(c, &e->timing.previous_end, EVENT_DEFAULT);
    if (r) {
        driver_error(err, errlen, TXT("could not create a timing event"), r);
        return 1;
    }
    e->timing.on = true;
    return 0;
}

ENGINE_EXPORT void qtc_get_t(void* h, uint64_t* out) {
    Engine* e = (Engine*)h;
    out[0] = e->gpu_gap;
    out[1] = e->gpu_clear;
    out[2] = e->gpu_kernel;
    out[3] = e->gpu_launches;
}

ENGINE_EXPORT int qtc_opt_c(void* h, int depth, int streams, char* err, int errlen) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    if (e->fcount) {
        text_set(err, errlen, TXT("launches are still in flight"));
        return 1;
    }
    if (depth < 1 || depth > 8 || streams < 1 || streams > depth) {
        char a[24], b[24];
        text_set(err, errlen, "cannot queue ", text_num(a, sizeof a, depth), " launches on ", text_num(b, sizeof b, streams));
        return 1;
    }
    const auto failed = [&](cu_r r) {
        driver_error(err, errlen, TXT("could not prepare the launch queue"), r);
        return 1;
    };
    void* lanes[8];
    lanes[0] = e->stream;
    if (streams > 1) c->tracking = false;
    for (int i = 1; i < streams; i++) {
        const cu_r r = stream_new(c, &lanes[i]);
        if (r) return failed(r);
    }
    slots_clear(e);
    for (int i = 0; i < depth; i++) {
        Slot* s = &e->slots[i];
        memset(s, 0, sizeof *s);
        s->lane = i % streams;
        s->stream = lanes[s->lane];
        cu_r r = slice_zeros(c, e->stream, RESULT_WORDS * 4, &s->results);
        if (r) return failed(r);
        before_read(c, &s->results, e->stream);
        after_read(c, &s->results, e->stream);
        if ((r = pinned_new(c, RESULT_WORDS * 4, &s->host, &s->host_event))) return failed(r);
        memset(s->host, 0, RESULT_WORDS * 4);
        if (e->timing.on) {
            s->timed = true;
            if ((r = event_new(c, &s->timing.start, EVENT_DEFAULT))) return failed(r);
            if ((r = event_new(c, &s->timing.cleared, EVENT_DEFAULT))) return failed(r);
            if ((r = event_new(c, &s->timing.end, EVENT_DEFAULT))) return failed(r);
        }
        if ((r = event_new(c, &s->done, e->wait_flag))) return failed(r);
        e->nslots = i + 1;
    }
    cu_r r = bind(c);
    if (!r) r = cu.cuStreamSynchronize(e->stream);
    if (r) return failed(r);
    return 0;
}

ENGINE_EXPORT int qtc_get_d(void* h) { return ((Engine*)h)->fcount; }

ENGINE_EXPORT int qtc_opt_d(void* h, char* err, int errlen) {
    (void)h;
    text_set(err, errlen, TXT("this build's kernel has no abort check"));
    return 1;
}

ENGINE_EXPORT int qtc_opt_e(void* h, uint32_t epoch, char* err, int errlen) {
    Engine* e = (Engine*)h;
    if (!e->abort.on || epoch <= e->abort.newest) return 0;
    e->abort.newest = epoch;
    cu_r r = bind(&e->ctx);
    if (!r) {
        *(uint32_t*)e->abort.host = epoch;
        r = cu.cuMemcpyHtoDAsync_v2(e->abort_word.p, e->abort.host, 4, e->abort.stream);
    }
    if (r) {
        driver_error(err, errlen, TXT("could not tell the card its job is over"), r);
        return 1;
    }
    cu.cuStreamQuery(e->abort.stream);
    return 0;
}

ENGINE_EXPORT uint32_t qtc_put(void* h, const uint64_t* state, const uint32_t* target, uint32_t epoch, uint32_t launch, uint32_t count) {
    Engine* e = (Engine*)h;
    uint32_t d[3];
    dispatch(e->scheme, count, e->asked, e->asked_per, e->block_threads, d);
    const uint32_t config[3] = {d[1], d[2], count};
    make_params(e->scheme, state, target, epoch, launch, config, &e->params);
    e->stage_blocks = d[0];
    e->stage_per = d[2];
    e->stage_launch = launch;
    e->stage_count = count;
    return d[2];
}

static unsigned shared_pad() {
    unsigned pad = 0;
    return pad;
}

ENGINE_EXPORT int qtc_submit(void* h, char* err, int errlen) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    if (e->fcount >= e->nslots) {
        text_set(err, errlen, TXT("no free slot to queue a launch in"));
        return 1;
    }
    const int index = e->next_slot;
    e->next_slot = (index + 1) % e->nslots;
    Slot* s = &e->slots[index];
    void* st = s->stream;
    cu_r r = bind(c);
    const char* what = TXT("could not queue the launch");
    if (!r && s->timed) r = event_record(c, s->timing.start, st);
    if (!r) r = cu.cuMemsetD8Async(s->results.p, 0, RESULT_WORDS * 4, st);
    if (!r && s->timed) r = event_record(c, s->timing.cleared, st);
    if (r) {
        driver_error(err, errlen, what, r);
        return 1;
    }
    void* args[3];
    int n = 0;
    args[n++] = &s->results.p;
    args[n++] = &e->params;
    (void)n;
    r = cu.cuLaunchKernel(e->mine, e->stage_blocks, 1, 1, e->block_threads, 1, 1, shared_pad(), st, args, nullptr);
    if (r) {
        driver_error(err, errlen, "kernel launch failed", r);
        return 1;
    }
    if (s->timed) r = event_record(c, s->timing.end, st);
    if (!r) r = cu.cuMemcpyDtoHAsync_v2(s->host, s->results.p, RESULT_WORDS * 4, st);
    if (!r) r = event_record(c, s->done, st);
    if (r) {
        driver_error(err, errlen, what, r);
        return 1;
    }

    cu.cuStreamQuery(st);
    Flight* f = &e->flights[(e->fhead + e->fcount) % 8];
    e->fcount++;
    f->slot = index;
    f->launch = e->stage_launch;
    f->count = e->stage_count;
    f->per_thread = e->stage_per;
    f->scheme = e->scheme;
    return 0;
}

ENGINE_EXPORT int qtc_wait(void* h, char* err, int errlen) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    if (!e->fcount) {
        text_set(err, errlen, TXT("no launch is in flight"));
        return 1;
    }
    const Flight f = e->flights[e->fhead];
    e->fhead = (e->fhead + 1) % 8;
    e->fcount--;
    Slot* s = &e->slots[f.slot];
    cu_r r = bind(c);
    if (!r) r = cu.cuEventSynchronize(s->done);
    if (r) {
        driver_error(err, errlen, "kernel run failed", r);
        return 1;
    }
    e->waiting = f;
    e->has_waiting = true;
    return 0;
}

ENGINE_EXPORT int qtc_collect(void* h, uint64_t* skipped, uint32_t* reported, uint32_t* kept, uint8_t* lows, char* err, int errlen) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    if (!e->has_waiting) {
        text_set(err, errlen, TXT("no launch is in flight"));
        return 1;
    }
    e->has_waiting = false;
    const Flight f = e->waiting;
    Slot* s = &e->slots[f.slot];
    cu_r r = 0;
    uint32_t raw[RESULT_WORDS];
    memcpy(raw, s->host, sizeof raw);
    if (s->timed && e->fcount) {
        Slot* next = &e->slots[e->flights[e->fhead].slot];
        if (next->timed) {
            float clear = 0, kernel = 0, gap = 0;
            r = elapsed_ms(c, s->timing.start, s->timing.cleared, &clear);
            if (!r) r = elapsed_ms(c, s->timing.cleared, s->timing.end, &kernel);
            if (!r) r = elapsed_ms(c, s->timing.end, next->timing.start, &gap);
            if (r) {
                driver_error(err, errlen, TXT("could not time the launch"), r);
                return 1;
            }
            e->gpu_clear += ms_ns(clear);
            e->gpu_kernel += ms_ns(kernel);
            e->gpu_gap += ms_ns(gap);
            e->gpu_launches++;
        }
    }
    Outcome o;
    if (read_out(e, &f, raw, &o, lows, err, errlen)) return 1;
    *skipped = o.skipped;
    *reported = o.reported;
    *kept = o.kept;
    return 0;
}

ENGINE_EXPORT int qtc_search_start(void* h, char* err, int errlen) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    void* st = e->stream;
    GpuTiming* t = &e->timing;
    cu_r r = 0;
    if (t->on && (r = event_record(c, t->start, st))) {
        driver_error(err, errlen, TXT("could not time the launch"), r);
        return 1;
    }
    if ((r = slice_clear(c, &e->results, st))) {
        driver_error(err, errlen, TXT("could not clear the result buffer"), r);
        return 1;
    }
    if (t->on && (r = event_record(c, t->cleared, st))) {
        driver_error(err, errlen, TXT("could not time the launch"), r);
        return 1;
    }
    Launch l;
    launch_start(&l, c);
    arg_write(&l, &e->results);
    arg_value(&l, &e->params);
    if ((r = launch_go(&l, e->mine, e->stage_blocks, e->block_threads, shared_pad(), st))) {
        driver_error(err, errlen, "kernel launch failed", r);
        return 1;
    }
    if (t->on && (r = event_record(c, t->end, st))) {
        driver_error(err, errlen, TXT("could not time the launch"), r);
        return 1;
    }
    return 0;
}

ENGINE_EXPORT int qtc_search_wait(void* h, char* err, int errlen) {
    Engine* e = (Engine*)h;
    cu_r r = bind(&e->ctx);
    if (!r) r = cu.cuStreamSynchronize(e->stream);
    if (r) {
        driver_error(err, errlen, "kernel run failed", r);
        return 1;
    }
    return 0;
}

ENGINE_EXPORT int qtc_search_read(void* h, uint64_t* skipped, uint32_t* reported, uint32_t* kept, uint8_t* lows, char* err, int errlen) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    void* st = e->stream;
    GpuTiming* t = &e->timing;
    cu_r r = 0;
    uint32_t raw[RESULT_WORDS];
    if ((r = slice_to_host(c, st, &e->results, raw))) {
        driver_error(err, errlen, TXT("could not read the result buffer"), r);
        return 1;
    }
    if (t->on) {
        if (t->has_previous) {
            float gap = 0, clear = 0, kernel = 0;
            cu_r a = elapsed_ms(c, t->previous_end, t->start, &gap);
            cu_r b = elapsed_ms(c, t->start, t->cleared, &clear);
            cu_r d = elapsed_ms(c, t->cleared, t->end, &kernel);
            r = a ? a : b ? b : d;
            if (r) {
                driver_error(err, errlen, TXT("could not time the launch"), r);
                return 1;
            }
            e->gpu_gap += ms_ns(gap);
            e->gpu_clear += ms_ns(clear);
            e->gpu_kernel += ms_ns(kernel);
            e->gpu_launches++;
        }
        void* x = t->end;
        t->end = t->previous_end;
        t->previous_end = x;
        t->has_previous = true;
    }
    Flight f;
    f.slot = 0;
    f.launch = e->stage_launch;
    f.count = e->stage_count;
    f.per_thread = e->stage_per;
    f.scheme = e->scheme;
    Outcome o;
    if (read_out(e, &f, raw, &o, lows, err, errlen)) return 1;
    *skipped = o.skipped;
    *reported = o.reported;
    *kept = o.kept;
    return 0;
}

ENGINE_EXPORT int qtc_hash_many(void* h, const uint8_t* header, const uint8_t* start_be, const uint32_t* start_le, uint32_t count, uint32_t* out, char* err, int errlen) {
    Engine* e = (Engine*)h;
    Ctx* c = &e->ctx;
    const char* what = "hash self-test failed";
    uint64_t mid[12];
    make_mid(header, start_be, e->permute, mid);
    uint32_t words[24];
    for (int i = 0; i < 12; i++) {
        words[2 * i] = (uint32_t)mid[i];
        words[2 * i + 1] = (uint32_t)(mid[i] >> 32);
    }
    Slice md, st, hashes;
    memset(&md, 0, sizeof md);
    memset(&st, 0, sizeof st);
    memset(&hashes, 0, sizeof hashes);
    cu_r r = slice_from_host(c, e->stream, words, sizeof words, &md);
    if (!r) r = slice_from_host(c, e->stream, start_le, 64, &st);
    if (!r) r = slice_zeros(c, e->stream, (size_t)count * 16 * 4, &hashes);
    if (!r) {
        Launch l;
        launch_start(&l, c);
        arg_write(&l, &hashes);
        arg_read(&l, &md);
        arg_read(&l, &st);
        arg_value(&l, &count);
        const uint32_t grid = (count + QTC_HASH_BLOCK - 1) / QTC_HASH_BLOCK;
        r = launch_go(&l, e->hash, grid ? grid : 1, QTC_HASH_BLOCK, 0, e->stream);
    }
    if (!r) r = bind(c);
    if (!r) r = cu.cuStreamSynchronize(e->stream);
    if (!r) r = slice_to_host(c, e->stream, &hashes, out);
    if (r) driver_error(err, errlen, what, r);
    slice_free(c, &hashes);
    slice_free(c, &st);
    slice_free(c, &md);
    return r ? 1 : 0;
}


