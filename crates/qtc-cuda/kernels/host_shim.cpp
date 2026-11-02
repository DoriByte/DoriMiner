// DoriMiner - host_shim.cpp
// Copyright (C) 2026 J. Costa (DoriByte). Licensed under the PolyForm Noncommercial License 1.0.0 with the additional
// permission in MINING-PERMISSION (see LICENSE and MINING-PERMISSION at the top of this tree).

#ifdef _MSC_VER
#pragma warning(disable : 4068)
#endif

#define QTC_HOST
#include "mining.cu"

#define QTC_SHIM_SUFFIX
#define QTC_SHIM_PASTE(a, b) a##b
#define QTC_SHIM_EXPAND(a, b) QTC_SHIM_PASTE(a, b)
#define QTC_SHIM(name) QTC_SHIM_EXPAND(name, QTC_SHIM_SUFFIX)

extern "C" {

void QTC_SHIM(qtc_host_hash_from_midstate)(const u64 *mid, const u32 *nonce_le, u32 *hash_le) {
    hash_from_midstate(mid, nonce_le, hash_le);
}

u32 QTC_SHIM(qtc_host_mine)(const MiningParams *params, u32 index, u64 *first_out) {
    u64 round1[12];
    for (int i = 0; i < 12; i++) {
        round1[i] = ((u64)params->round1[2 * i + 1] << 32) | (u64)params->round1[2 * i];
    }

    u64 lane_a = ((u64)params->lanes[1] << 32) | (u64)params->lanes[0];
    u64 lane_b = ((u64)params->lanes[3] << 32) | (u64)params->lanes[2];
    return mine_one(round1, lane_a, lane_b, index, params->difficulty_target + 8, first_out) ? 1u : 0u;
}

u32 QTC_SHIM(qtc_host_params_size)(void) {
    return (u32)sizeof(MiningParams);
}

}

