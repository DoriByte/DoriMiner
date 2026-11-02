// DoriMiner - vela_seal.h
// Copyright (C) 2026 J. Costa (DoriByte). Licensed under the PolyForm Noncommercial License 1.0.0 with the additional
// permission in MINING-PERMISSION (see LICENSE and MINING-PERMISSION at the top of this tree).

#pragma once
#include <cstdint>
#include <cstring>
#include <mutex>
#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#else
#include <link.h>
#include <sys/mman.h>
#include <unistd.h>
#endif

namespace vela_seal {

inline uint64_t seal_key() {
    volatile uint64_t pa = 0xA0761D6478BD642Full, pb = 0xE7037ED1A0B428DBull;
    uint64_t x = pa;
    x ^= x >> 33; x *= 0xff51afd7ed558ccdull; x ^= x >> 33; x *= 0xc4ceb9fe1a85ec53ull; x ^= x >> 33;
    return x ^ (uint64_t)pb;
}
constexpr uint64_t NONCE = 0x56454c4146415442ull;
constexpr uint32_t FATBIN_MAGIC = 0xBA55ED50u;
constexpr int WRAPPER_MAGIC = 0x466243b1;

struct Wrapper {
    int magic;
    int version;
    const unsigned char* data;
    void* filename;
};

struct Stream {
    uint64_t state;
    explicit Stream(uint64_t nonce) : state(seal_key() ^ ((nonce << 29) | (nonce >> 35))) {}
    uint64_t next() {
        state += 0x9e3779b97f4a7c15ull;
        uint64_t z = state;
        z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ull;
        z = (z ^ (z >> 27)) * 0x94d049bb133111ebull;
        return z ^ (z >> 31);
    }
};

inline void apply(unsigned char* p, uint64_t n) {
    Stream s(NONCE);
    for (uint64_t i = 0; i < n; i += 8) {
        const uint64_t w = s.next();
        for (uint64_t j = 0; j < 8 && i + j < n; j++) p[i + j] ^= (unsigned char)(w >> (8 * j));
    }
}

inline uint64_t plain_size(const unsigned char* hdr) {
    uint32_t magic;
    uint64_t rest;
    std::memcpy(&magic, hdr, 4);
    std::memcpy(&rest, hdr + 8, 8);
    return magic == FATBIN_MAGIC ? 16 + rest : 0;
}

inline uint64_t sealed_size(const unsigned char* data, const unsigned char* lo, const unsigned char* hi) {
    if (data < lo || data + 16 > hi) return 0;
    unsigned char hdr[16];
    std::memcpy(hdr, data, 16);
    apply(hdr, 16);
    const uint64_t n = plain_size(hdr);
    return (n && data + n <= hi) ? n : 0;
}

#ifdef _WIN32
inline bool writable(void* p, uint64_t n, unsigned long* old) { return VirtualProtect(p, (SIZE_T)n, PAGE_READWRITE, old) != 0; }
inline void restore(void* p, uint64_t n, unsigned long old) { unsigned long x; VirtualProtect(p, (SIZE_T)n, old, &x); }
#else
inline bool writable(void* p, uint64_t n, unsigned long*) {
    const uintptr_t pg = (uintptr_t)sysconf(_SC_PAGESIZE), a = (uintptr_t)p & ~(pg - 1);
    return mprotect((void*)a, (uintptr_t)p + n - a, PROT_READ | PROT_WRITE) == 0;
}
inline void restore(void* p, uint64_t n, unsigned long) {
    const uintptr_t pg = (uintptr_t)sysconf(_SC_PAGESIZE), a = (uintptr_t)p & ~(pg - 1);
    mprotect((void*)a, (uintptr_t)p + n - a, PROT_READ);
}
#endif

template <class F>
void each_region(F f) {
#ifdef _WIN32
    HMODULE m = nullptr;
    if (!GetModuleHandleExA(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, (LPCSTR)&apply, &m)) return;
    auto* base = (const unsigned char*)m;
    auto* nt = (const IMAGE_NT_HEADERS*)(base + ((const IMAGE_DOS_HEADER*)base)->e_lfanew);
    const IMAGE_SECTION_HEADER* s = IMAGE_FIRST_SECTION(nt);
    for (unsigned i = 0; i < nt->FileHeader.NumberOfSections; i++, s++)
        if ((s->Characteristics & IMAGE_SCN_MEM_READ) && !(s->Characteristics & IMAGE_SCN_MEM_DISCARDABLE))
            f(base + s->VirtualAddress, base + s->VirtualAddress + s->Misc.VirtualSize, base, base + nt->OptionalHeader.SizeOfImage);
#else
    struct Ctx { const void* self; F* f; } ctx{(const void*)&apply, &f};
    dl_iterate_phdr([](dl_phdr_info* info, size_t, void* vp) -> int {
        auto* c = (Ctx*)vp;
        const unsigned char *mlo = nullptr, *mhi = nullptr;
        bool mine = false;
        for (int i = 0; i < info->dlpi_phnum; i++) {
            const ElfW(Phdr)& ph = info->dlpi_phdr[i];
            if (ph.p_type != PT_LOAD) continue;
            auto* lo = (const unsigned char*)(info->dlpi_addr + ph.p_vaddr);
            auto* hi = lo + ph.p_memsz;
            if (!mlo || lo < mlo) mlo = lo;
            if (!mhi || hi > mhi) mhi = hi;
            if ((const unsigned char*)c->self >= lo && (const unsigned char*)c->self < hi) mine = true;
        }
        if (!mine) return 0;
        for (int i = 0; i < info->dlpi_phnum; i++) {
            const ElfW(Phdr)& ph = info->dlpi_phdr[i];
            if (ph.p_type != PT_LOAD || !(ph.p_flags & PF_R)) continue;
            auto* lo = (const unsigned char*)(info->dlpi_addr + ph.p_vaddr);
            (*c->f)(lo, lo + ph.p_filesz, mlo, mhi);
        }
        return 1;
    }, &ctx);
#endif
}

inline const char* unseal_now() {
    const Wrapper* found = nullptr;
    uint64_t size = 0;
    int plain = 0, sealed = 0;
    each_region([&](const unsigned char* lo, const unsigned char* hi, const unsigned char* mlo, const unsigned char* mhi) {
        for (const unsigned char* p = (const unsigned char*)(((uintptr_t)lo + 7) & ~(uintptr_t)7); p + sizeof(Wrapper) <= hi; p += 8) {
            const Wrapper* w = (const Wrapper*)p;
            if (w->magic != WRAPPER_MAGIC || w->version != 1) continue;
            if (w->data >= mlo && w->data + 16 <= mhi && plain_size(w->data)) { plain++; continue; }
            const uint64_t n = sealed_size(w->data, mlo, mhi);
            if (n) { sealed++; found = w; size = n; }
        }
    });
    if (sealed == 0) return plain ? "" : "the Vela engine's device code was not found in the program";
    if (sealed > 1) return "more than one sealed device image in the program";
    auto* p = (unsigned char*)found->data;
    unsigned long old = 0;
    if (!writable(p, size, &old)) return "cannot open the Vela engine's device code (memory protection)";
    apply(p, size);
    restore(p, size, old);
    return plain_size(p) == size ? "" : "the Vela engine's device code did not open";
}

inline const char* unseal() {
    static std::once_flag once;
    static const char* why = "";
    std::call_once(once, [] { why = unseal_now(); });
    return why;
}

}

