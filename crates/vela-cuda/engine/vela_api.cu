// DoriMiner - vela_api.cu
// Copyright (C) 2026 J. Costa (DoriByte). Licensed under the PolyForm Noncommercial License 1.0.0 with the additional
// permission in MINING-PERMISSION (see LICENSE and MINING-PERMISSION at the top of this tree).

#define VELA_LIB 1
#include "vela.cu"
#include "vela_seal.h"

namespace {
struct Handle {
    int device = 0;
    int batch = 8;
    Engine E;
    Act ctx{nullptr, 0, 0};
    bool haveCtx = false;
    int cin = 0, sig = 0;
    unsigned char* dseed = nullptr;
    int* ntok = nullptr;
    int* xin = nullptr;
    unsigned char* dx0 = nullptr;
};

void say(char* err, int n, const char* what) {
    if (err && n > 0) { std::snprintf(err, (size_t)n, "%s", what); }
}
}

extern "C" {

void* vela_open(int device, const char* model_path, int batch, char* err, int errlen) {
    if (batch < 1 || batch > 64) { say(err, errlen, "batch must be 1 to 64"); return nullptr; }

    if (const char* why = vela_seal::unseal(); *why) { say(err, errlen, why); return nullptr; }
    int drv = 0;
    cudaDriverGetVersion(&drv);
    if (drv < 13000) {
        char m[160];
        std::snprintf(m, sizeof m, "the NVIDIA driver supports CUDA %d.%d; Vela needs driver 580 or newer (CUDA 13)", drv / 1000, (drv % 1000) / 10);
        say(err, errlen, m);
        return nullptr;
    }
    if (cudaSetDevice(device) != cudaSuccess) { say(err, errlen, "cannot select the card"); return nullptr; }
    int major = 0;
    cudaDeviceGetAttribute(&major, cudaDevAttrComputeCapabilityMajor, device);
    if (major < 8) { say(err, errlen, "Vela needs an RTX 30, 40 or 50 card (sm_86 or newer)"); return nullptr; }
    FILE* f = std::fopen(model_path, "rb");
    if (!f) { say(err, errlen, "cannot open the model file"); return nullptr; }
    std::fclose(f);
    auto* h = new Handle;
    h->device = device;
    h->batch = batch;
    {
        cudaMemPool_t pool;
        CK(cudaDeviceGetDefaultMemPool(&pool, device));
        uint64_t thr = ~0ull;
        CK(cudaMemPoolSetAttribute(pool, cudaMemPoolAttrReleaseThreshold, &thr));
    }
    Engine& E = h->E;
    E.load(model_path);
    E.check = false;
    E.fuse = true;
    E.B = batch;

    unsigned char perm[64];
    for (int pp = 0; pp < 64; pp++) {
        int kg = pp / 32, r = pp % 32, hi16 = r / 16, qq = (r % 16) / 4, j = r % 4;
        perm[pp] = (unsigned char)(kg * 32 + hi16 * 16 + 8 * (j / 2) + 2 * qq + (j % 2));
    }
    CK(cudaMemcpyToSymbol(PERM, perm, 64));
    unsigned char inv[64];
    for (int pp = 0; pp < 64; pp++) inv[perm[pp]] = (unsigned char)pp;
    CK(cudaMemcpyToSymbol(INVPERM, inv, 64));
    const int* L = (const int*)E.t.at("lut.exp").data;
    std::vector<int4> L4(1024);
    for (int i = 0; i < 1024; i++) { int nd = L[i] - L[i + 1]; L4[i] = make_int4(L[i], -(nd >> 10), nd & 1023, 0); }
    E.lut4 = E.up<int4>("lut4", L4.data(), 16 * 1024);
    E.lut_sig = (int*)E.i32("lut.sigmoid");
    E.lut_phi = (int*)E.i32("lut.phi");
    {
        const int* P = (const int*)E.t.at("lut.phi").data;
        for (int i = 0; i < 2048; i++)
            if ((long long)(P[i + 1] - P[i]) * 1023 > INT_MAX || (long long)(P[i + 1] - P[i]) * 1023 < INT_MIN) {
                say(err, errlen, "vela engine: unsupported model (E14)");
                delete h; return nullptr; }
    }
    for (auto fn : {(const void*)attn_k3<0, 8>, (const void*)attn_k3<0, 4>, (const void*)attn_k3<1, 4>, (const void*)attn_k4<0, 8>, (const void*)attn_k4<0, 4>, (const void*)attn_k4<1, 4>})
        CK(cudaFuncSetAttribute(fn, cudaFuncAttributePreferredSharedMemoryCarveout, cudaSharedmemCarveoutMaxShared));
    CK(cudaFuncSetAttribute((const void*)attn_k4<1, 4>, cudaFuncAttributeMaxDynamicSharedMemorySize, k4smem<1>()));
    h->cin = ((const int*)E.t.at("c.cin").data)[0];
    h->sig = ((const int*)E.t.at("c.sig").data)[0];
    CK(cudaMalloc(&h->dseed, 32ull * batch));
    CK(cudaMalloc(&h->ntok, 4ull * 16384 * batch));
    CK(cudaMalloc(&h->xin, 4ull * 16384 * batch));
    CK(cudaMalloc(&h->dx0, 32768ull * batch));
    CK(cudaDeviceSynchronize());
    return h;
}

int vela_prompt(void* hp, const int* tokens, int n) {
    auto* h = (Handle*)hp;
    if (n < 0 || n > 75) return 1;
    std::vector<int> ids{49406};
    for (int i = 0; i < n; i++) ids.push_back(tokens[i]);
    ids.push_back(49407);
    ids.resize(77, 0);
    if (h->haveCtx) h->E.drop(h->ctx);
    h->ctx = h->E.text(ids);
    h->haveCtx = true;
    return cudaDeviceSynchronize() == cudaSuccess ? 0 : 2;
}

int vela_run(void* hp, const unsigned char* seeds, unsigned char* x0) {
    auto* h = (Handle*)hp;
    if (!h->haveCtx) return 1;
    Engine& E = h->E;
    const int B = h->batch;
    if (cudaMemcpy(h->dseed, seeds, 32ull * B, cudaMemcpyHostToDevice) != cudaSuccess) return 2;
    noise_k<<<dim3(4, B), 128>>>(h->dseed, B, E.i32("lut.gauss"), h->cin, h->ntok, h->xin);
    Act x{h->xin, 4096 * B, 4};
    Act eps = E.unet(x, h->ctx);
    x0_k<<<(unsigned)((16384ull * B + 255) / 256), 256>>>(h->ntok, eps.p, B, h->sig, h->dx0);
    const cudaError_t e = cudaMemcpy(x0, h->dx0, 32768ull * B, cudaMemcpyDeviceToHost);
    E.drop(eps);
    return e == cudaSuccess ? 0 : 3;
}

int vela_batch(void* hp) { return ((Handle*)hp)->batch; }

void vela_close(void* hp) {
    auto* h = (Handle*)hp;
    if (!h) return;
    cudaSetDevice(h->device);
    cudaFree(h->dseed); cudaFree(h->ntok); cudaFree(h->xin); cudaFree(h->dx0);
    delete h;
}

}

