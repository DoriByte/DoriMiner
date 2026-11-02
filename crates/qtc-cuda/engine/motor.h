// DoriMiner - motor.h
// Copyright (C) 2026 J. Costa (DoriByte). Licensed under the PolyForm Noncommercial License 1.0.0 with the additional
// permission in MINING-PERMISSION (see LICENSE and MINING-PERMISSION at the top of this tree).

#pragma once
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#define CU_API __stdcall
#else
#include <dlfcn.h>
#include <pthread.h>
#define CU_API
#endif

#define ENGINE_EXPORT extern "C"

namespace {

volatile uint8_t hide_mix = 0;
constexpr uint8_t hide_key(size_t i) { return (uint8_t)((0x5b + i * 0x95) ^ (i >> 3)); }
template <size_t N>
struct Hidden {
    char c[N];
    constexpr Hidden(const char (&s)[N]) : c{} {
        for (size_t i = 0; i < N; i++) c[i] = (char)(s[i] ^ hide_key(i));
    }
};
template <size_t N>
struct Shown {
    char c[N];
    explicit Shown(const Hidden<N>& h) {
        for (size_t i = 0; i < N; i++) c[i] = (char)(h.c[i] ^ hide_key(i) ^ hide_mix);
    }
};
#define HS(lit) ([]() { static constexpr Hidden<sizeof(lit)> h(lit); return Shown<sizeof(lit)>(h); }())

struct Unsealed {
    uint8_t* data;
    size_t len;
};

inline uint64_t seal_key() {
    volatile uint64_t pa = 0xD1B54A32D192ED03ull, pb = 0x2545F4914F6CDD1Dull;
    uint64_t x = pa;
    x ^= x >> 33; x *= 0xff51afd7ed558ccdull; x ^= x >> 33; x *= 0xc4ceb9fe1a85ec53ull; x ^= x >> 33;
    return x ^ (uint64_t)pb;
}

inline uint64_t le64(const uint8_t* p) {
    uint64_t v = 0;
    for (int i = 0; i < 8; i++) v |= (uint64_t)p[i] << (8 * i);
    return v;
}

inline Unsealed unseal(const uint8_t* stored, size_t len) {
    Unsealed out = {nullptr, 0};
    const size_t HEADER = 20;
    if (len < HEADER || memcmp(stored, "DKN1", 4) != 0) return out;
    const uint64_t nonce = le64(stored + 4), sum = le64(stored + 12);
    const size_t n = len - HEADER;
    uint8_t* p = (uint8_t*)malloc(n + 1);
    if (!p) return out;
    memcpy(p, stored + HEADER, n);
    uint64_t state = seal_key() ^ ((nonce << 29) | (nonce >> 35));
    for (size_t i = 0; i < n; i += 8) {
        state += 0x9e3779b97f4a7c15ull;
        uint64_t z = state;
        z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ull;
        z = (z ^ (z >> 27)) * 0x94d049bb133111ebull;
        z = z ^ (z >> 31);
        for (size_t j = 0; j < 8 && i + j < n; j++) p[i + j] ^= (uint8_t)(z >> (8 * j));
    }
    uint64_t hash = 0xcbf29ce484222325ull;
    for (size_t i = 0; i < n; i++) hash = (hash ^ p[i]) * 0x00000100000001b3ull;
    if (hash != sum) {
        free(p);
        return out;
    }
    p[n] = 0;
    out.data = p;
    out.len = n;
    return out;
}

inline void text_set(char* out, int cap, const char* a, const char* b = nullptr, const char* c = nullptr, const char* d = nullptr) {
    if (!out || cap <= 0) return;
    int at = 0;
    const char* all[4] = {a, b, c, d};
    for (int i = 0; i < 4; i++) {
        const char* s = all[i];
        if (!s) continue;
        while (*s && at + 1 < cap) out[at++] = *s++;
    }
    out[at] = 0;
}

inline const char* text_num(char* out, int cap, long long v) {
    char t[24];
    int n = 0;
    const bool neg = v < 0;
    unsigned long long u = neg ? 0ull - (unsigned long long)v : (unsigned long long)v;
    do {
        t[n++] = (char)('0' + u % 10);
        u /= 10;
    } while (u);
    int at = 0;
    if (neg && at + 1 < cap) out[at++] = '-';
    while (n && at + 1 < cap) out[at++] = t[--n];
    out[at] = 0;
    return out;
}

typedef void (*TaskFn)(void* arg, size_t from, size_t to);
struct Task {
    TaskFn fn;
    void* arg;
    size_t from, to;
};
#ifdef _WIN32
DWORD WINAPI task_main(void* p) {
    Task* t = (Task*)p;
    t->fn(t->arg, t->from, t->to);
    return 0;
}
#else
void* task_main(void* p) {
    Task* t = (Task*)p;
    t->fn(t->arg, t->from, t->to);
    return nullptr;
}
#endif

inline void in_parallel(size_t total, size_t threads, TaskFn fn, void* arg) {
    const size_t MAX_THREADS = 64;
    if (threads > MAX_THREADS) threads = MAX_THREADS;
    if (threads == 0) threads = 1;
    size_t each = (total + threads - 1) / threads;
    if (each == 0) each = 1;
    Task tasks[MAX_THREADS];
#ifdef _WIN32
    HANDLE handles[MAX_THREADS];
#else
    pthread_t handles[MAX_THREADS];
#endif
    bool started[MAX_THREADS];
    size_t n = 0;
    for (size_t from = 0; from < total; from += each, n++) {
        tasks[n].fn = fn;
        tasks[n].arg = arg;
        tasks[n].from = from;
        tasks[n].to = from + each < total ? from + each : total;
#ifdef _WIN32
        handles[n] = CreateThread(nullptr, 0, task_main, &tasks[n], 0, nullptr);
        started[n] = handles[n] != nullptr;
#else
        started[n] = pthread_create(&handles[n], nullptr, task_main, &tasks[n]) == 0;
#endif
        if (!started[n]) fn(arg, tasks[n].from, tasks[n].to);
    }
    for (size_t i = 0; i < n; i++) {
        if (!started[i]) continue;
#ifdef _WIN32
        WaitForSingleObject(handles[i], INFINITE);
        CloseHandle(handles[i]);
#else
        pthread_join(handles[i], nullptr);
#endif
    }
}

typedef int cu_r;
typedef unsigned long long cu_ptr;
struct Copy2D {
    size_t srcXInBytes, srcY;
    int srcMemoryType;
    const void* srcHost;
    cu_ptr srcDevice;
    void* srcArray;
    size_t srcPitch;
    size_t dstXInBytes, dstY;
    int dstMemoryType;
    void* dstHost;
    cu_ptr dstDevice;
    void* dstArray;
    size_t dstPitch;
    size_t WidthInBytes, Height;
};

#define DRIVER_CALLS(X)                                                                                                \
    X(cu_r, cuInit, (unsigned))                                                                                        \
    X(cu_r, cuDeviceGet, (int*, int))                                                                                  \
    X(cu_r, cuDeviceGetAttribute, (int*, int, int))                                                                    \
    X(cu_r, cuDevicePrimaryCtxRetain, (void**, int))                                                                   \
    X(cu_r, cuDevicePrimaryCtxRelease_v2, (int))                                                                       \
    X(cu_r, cuCtxGetCurrent, (void**))                                                                                 \
    X(cu_r, cuCtxSetCurrent, (void*))                                                                                  \
    X(cu_r, cuCtxSetFlags, (unsigned))                                                                                 \
    X(cu_r, cuCtxSynchronize, ())                                                                                      \
    X(cu_r, cuStreamCreate, (void**, unsigned))                                                                        \
    X(cu_r, cuStreamDestroy_v2, (void*))                                                                               \
    X(cu_r, cuStreamSynchronize, (void*))                                                                              \
    X(cu_r, cuStreamWaitEvent, (void*, void*, unsigned))                                                               \
    X(cu_r, cuStreamQuery, (void*))                                                                                    \
    X(cu_r, cuEventCreate, (void**, unsigned))                                                                         \
    X(cu_r, cuEventDestroy_v2, (void*))                                                                                \
    X(cu_r, cuEventRecord, (void*, void*))                                                                             \
    X(cu_r, cuEventSynchronize, (void*))                                                                               \
    X(cu_r, cuEventElapsedTime, (float*, void*, void*))                                                                \
    X(cu_r, cuMemAllocAsync, (cu_ptr*, size_t, void*))                                                                 \
    X(cu_r, cuMemAlloc_v2, (cu_ptr*, size_t))                                                                          \
    X(cu_r, cuMemFreeAsync, (cu_ptr, void*))                                                                           \
    X(cu_r, cuMemFree_v2, (cu_ptr))                                                                                    \
    X(cu_r, cuMemHostAlloc, (void**, size_t, unsigned))                                                                \
    X(cu_r, cuMemFreeHost, (void*))                                                                                    \
    X(cu_r, cuMemsetD8Async, (cu_ptr, unsigned char, size_t, void*))                                                   \
    X(cu_r, cuMemcpyHtoDAsync_v2, (cu_ptr, const void*, size_t, void*))                                                \
    X(cu_r, cuMemcpyDtoHAsync_v2, (void*, cu_ptr, size_t, void*))                                                      \
    X(cu_r, cuMemcpy2DAsync_v2, (const Copy2D*, void*))                                                                \
    X(cu_r, cuModuleLoadData, (void**, const void*))                                                                   \
    X(cu_r, cuModuleUnload, (void*))                                                                                   \
    X(cu_r, cuModuleGetFunction, (void**, void*, const char*))                                                         \
    X(cu_r, cuFuncSetAttribute, (void*, int, int))                                                                     \
    X(cu_r, cuFuncGetAttribute, (int*, int, void*))                                                                    \
    X(cu_r, cuOccupancyMaxActiveBlocksPerMultiprocessorWithFlags, (int*, void*, int, size_t, unsigned))                \
    X(cu_r, cuLaunchKernel,                                                                                            \
      (void*, unsigned, unsigned, unsigned, unsigned, unsigned, unsigned, unsigned, void*, void**, void**))            \
    X(cu_r, cuGetErrorName, (int, const char**))                                                                       \
    X(cu_r, cuGetErrorString, (int, const char**))

struct Driver {
#define X(ret, name, args) ret(CU_API* name) args;
    DRIVER_CALLS(X)
#undef X
};
Driver cu;
volatile int cu_loaded = 0;

inline void* load_system_library(const char* name) {
#ifdef _WIN32
    return (void*)LoadLibraryA(name);
#else
    return dlopen(name, RTLD_NOW | RTLD_LOCAL);
#endif
}
inline void* system_symbol(void* lib, const char* name) {
#ifdef _WIN32
    return (void*)GetProcAddress((HMODULE)lib, name);
#else
    return dlsym(lib, name);
#endif
}

inline bool driver_load(char* err, int errlen) {
    if (cu_loaded) return true;
#ifdef _WIN32
    void* lib = load_system_library(HS("nvcuda.dll").c);
#else
    void* lib = load_system_library(HS("libcuda.so.1").c);
    if (!lib) lib = load_system_library(HS("libcuda.so").c);
#endif
    if (!lib) {
        text_set(err, errlen, "NVIDIA driver not found (CUDA library missing)");
        return false;
    }
#define X(ret, name, args)                                                                                             \
    cu.name = (decltype(cu.name))system_symbol(lib, HS(#name).c);                                                      \
    if (!cu.name) {                                                                                                    \
        text_set(err, errlen, "this NVIDIA driver is too old for the miner");                                          \
        return false;                                                                                                  \
    }
    DRIVER_CALLS(X)
#undef X
    cu_loaded = 1;
    return true;
}

inline void driver_error(char* err, int errlen, const char* what, cu_r r) {
    const char *name = nullptr, *text = nullptr;
    if (cu_loaded) {
        cu.cuGetErrorName(r, &name);
        cu.cuGetErrorString(r, &text);
    }
    char num[24];
    text_num(num, sizeof num, r);
    text_set(err, errlen, what, ": DriverError(", name ? name : num, ", \"");
    if (!err || errlen <= 0) return;
    int at = 0;
    while (at < errlen && err[at]) at++;
    text_set(err + at, errlen - at, text ? text : "", "\")");
}

struct Ctx {
    int device;
    void* ctx;
    int ordinal;
    bool async_alloc;
    int streams;
    bool tracking;
    int pending;
};

const unsigned EVENT_DEFAULT = 0, EVENT_BLOCKING_SYNC = 1, EVENT_DISABLE_TIMING = 2;
const int ATTR_MEMORY_POOLS = 115, ATTR_MULTIPROCESSORS = 16, ATTR_CC_MAJOR = 75, ATTR_CC_MINOR = 76;
const int FUNC_LOCAL_BYTES = 3, FUNC_REGS = 4, FUNC_DYNAMIC_SHARED = 8;
const int MEM_DEVICE = 2;
const unsigned STREAM_NON_BLOCKING = 1;
const unsigned CTX_SPIN = 1, CTX_YIELD = 2, CTX_BLOCKING = 4;

inline bool managing(const Ctx* c) { return c->streams > 0 && c->tracking; }

inline cu_r bind(Ctx* c) {
    if (c->pending) {
        const cu_r r = c->pending;
        c->pending = 0;
        return r;
    }
    void* now = nullptr;
    const cu_r r = cu.cuCtxGetCurrent(&now);
    if (r) return r;
    return now != c->ctx ? cu.cuCtxSetCurrent(c->ctx) : 0;
}
inline void keep(Ctx* c, cu_r r) {
    if (r) c->pending = r;
}

inline cu_r ctx_open(Ctx* c, int ordinal) {
    memset(c, 0, sizeof *c);
    c->ordinal = ordinal;
    c->tracking = true;
    cu_r r = cu.cuInit(0);
    if (r) return r;
    if ((r = cu.cuDeviceGet(&c->device, ordinal))) return r;
    if ((r = cu.cuDevicePrimaryCtxRetain(&c->ctx, c->device))) return r;
    int pools = 0;
    if ((r = cu.cuDeviceGetAttribute(&pools, ATTR_MEMORY_POOLS, c->device))) return r;
    c->async_alloc = pools > 0;
    return bind(c);
}

inline void ctx_close(Ctx* c) {
    keep(c, bind(c));
    if (c->ctx) keep(c, cu.cuDevicePrimaryCtxRelease_v2(c->device));
    c->ctx = nullptr;
}

inline cu_r stream_new(Ctx* c, void** s) {
    cu_r r = bind(c);
    if (r) return r;
    const int prev = c->streams++;
    if (prev == 0 && c->tracking && (r = cu.cuCtxSynchronize())) return r;
    return cu.cuStreamCreate(s, STREAM_NON_BLOCKING);
}

inline void stream_destroy(Ctx* c, void* s) {
    keep(c, bind(c));
    if (s) {
        c->streams--;
        keep(c, cu.cuStreamDestroy_v2(s));
    }
}

inline cu_r stream_wait(Ctx* c, void* s, void* ev) {
    const cu_r r = bind(c);
    return r ? r : cu.cuStreamWaitEvent(s, ev, 0);
}

inline cu_r event_new(Ctx* c, void** ev, unsigned flags) {
    const cu_r r = bind(c);
    return r ? r : cu.cuEventCreate(ev, flags);
}

inline cu_r event_record(Ctx* c, void* ev, void* s) {
    const cu_r r = bind(c);
    return r ? r : cu.cuEventRecord(ev, s);
}

inline void event_destroy(Ctx* c, void* ev) {
    keep(c, bind(c));
    keep(c, cu.cuEventDestroy_v2(ev));
}

struct Slice {
    cu_ptr p;
    size_t bytes;
    void* rd;
    void* wr;
    void* stream;
    bool live;
};

inline cu_r slice_alloc(Ctx* c, void* s, size_t bytes, Slice* o) {
    memset(o, 0, sizeof *o);
    cu_r r = bind(c);
    if (r) return r;
    r = c->async_alloc ? cu.cuMemAllocAsync(&o->p, bytes, s) : cu.cuMemAlloc_v2(&o->p, bytes);
    if (r) return r;
    o->bytes = bytes;
    o->stream = s;
    o->live = true;
    if (c->tracking) {
        if ((r = event_new(c, &o->rd, EVENT_DISABLE_TIMING))) return r;
        if ((r = event_new(c, &o->wr, EVENT_DISABLE_TIMING))) return r;
    }
    return 0;
}

inline void before_write(Ctx* c, Slice* b, void* s) {
    if (managing(c)) {
        if (b->rd) keep(c, stream_wait(c, s, b->rd));
        if (b->wr) keep(c, stream_wait(c, s, b->wr));
    }
}
inline void after_write(Ctx* c, Slice* b, void* s) {
    if (managing(c) && b->wr) keep(c, event_record(c, b->wr, s));
}

inline void before_read(Ctx* c, Slice* b, void* s) {
    if (managing(c) && b->wr) keep(c, stream_wait(c, s, b->wr));
}
inline void after_read(Ctx* c, Slice* b, void* s) {
    if (managing(c) && b->rd) keep(c, event_record(c, b->rd, s));
}

inline cu_r slice_clear(Ctx* c, Slice* b, void* s) {
    cu_r r = bind(c);
    if (r) return r;
    before_write(c, b, s);
    r = cu.cuMemsetD8Async(b->p, 0, b->bytes, s);
    after_write(c, b, s);
    return r;
}

inline cu_r slice_zeros(Ctx* c, void* s, size_t bytes, Slice* o) {
    const cu_r r = slice_alloc(c, s, bytes, o);
    return r ? r : slice_clear(c, o, s);
}

inline cu_r slice_from_host(Ctx* c, void* s, const void* src, size_t bytes, Slice* o) {
    cu_r r = slice_alloc(c, s, bytes, o);
    if (r) return r;
    if ((r = bind(c))) return r;
    before_write(c, o, s);
    r = cu.cuMemcpyHtoDAsync_v2(o->p, src, bytes, s);
    after_write(c, o, s);
    return r;
}

inline cu_r slice_to_host(Ctx* c, void* s, Slice* b, void* dst) {
    cu_r r = bind(c);
    if (r) return r;
    before_read(c, b, s);
    r = cu.cuMemcpyDtoHAsync_v2(dst, b->p, b->bytes, s);
    after_read(c, b, s);
    return r;
}

inline void slice_free(Ctx* c, Slice* b) {
    if (!b->live) return;
    if (managing(c)) {
        if (b->rd) keep(c, stream_wait(c, b->stream, b->rd));
        if (b->wr) keep(c, stream_wait(c, b->stream, b->wr));
    }
    if (c->async_alloc) {
        keep(c, cu.cuMemFreeAsync(b->p, b->stream));
    } else {
        keep(c, bind(c));
        keep(c, cu.cuStreamSynchronize(b->stream));
        keep(c, cu.cuMemFree_v2(b->p));
    }
    if (b->rd) event_destroy(c, b->rd);
    if (b->wr) event_destroy(c, b->wr);
    b->live = false;
}

struct Launch {
    void* args[16];
    int n;
    void* waits[24];
    int nw;
    void* recs[16];
    int nr;
    Ctx* c;
};
inline void launch_start(Launch* l, Ctx* c) {
    l->n = l->nw = l->nr = 0;
    l->c = c;
}
inline void arg_value(Launch* l, const void* p) { l->args[l->n++] = (void*)p; }
inline void arg_read(Launch* l, Slice* b) {
    if (managing(l->c)) {
        if (b->wr) l->waits[l->nw++] = b->wr;
        if (b->rd) l->recs[l->nr++] = b->rd;
    }
    l->args[l->n++] = &b->p;
}
inline void arg_write(Launch* l, Slice* b) {
    if (managing(l->c)) {
        if (b->rd) l->waits[l->nw++] = b->rd;
        if (b->wr) {
            l->waits[l->nw++] = b->wr;
            l->recs[l->nr++] = b->wr;
        }
    }
    l->args[l->n++] = &b->p;
}
inline cu_r launch_go(Launch* l, void* fn, unsigned grid, unsigned block, unsigned shared, void* s) {
    Ctx* c = l->c;
    cu_r r = bind(c);
    if (r) return r;
    for (int i = 0; i < l->nw; i++)
        if ((r = stream_wait(c, s, l->waits[i]))) return r;
    if ((r = cu.cuLaunchKernel(fn, grid, 1, 1, block, 1, 1, shared, s, l->args, nullptr))) return r;
    for (int i = 0; i < l->nr; i++)
        if ((r = event_record(c, l->recs[i], s))) return r;
    return 0;
}

inline int numenv(const char* name, long long* out) {
    const char* s = getenv(name);
    if (!s) return 0;
    while (*s == ' ' || *s == '\t') s++;
    if (!*s) return 0;
    long long v = 0;
    const char* first = s;
    while (*s >= '0' && *s <= '9') v = v * 10 + (*s++ - '0');
    if (s == first) return -1;
    while (*s == ' ' || *s == '\t') s++;
    if (*s) return -1;
    *out = v;
    return 1;
}

}

