// SPDX-License-Identifier: GPL-3.0-or-later
// Helpers for tier-2 code of one instruction family (see nxrecomp/lift/tier2/x_*.py). Included at the end of cxx.h.
// Byte order: the host is little-endian, so lane k of a V128 is the bytes at offset k*ES of the union's u8 view.
#pragma once
namespace nx {

// LD1..ST1 without a lane: N bytes of memory are the low N bytes of one register (upper bytes zero).
template <int N>
NX_INLINE V128 vld_n(uint64_t a) {
    V128 r = vzero();
    std::memcpy(r.u8, reinterpret_cast<const void*>(a), N);
    return r;
}

template <int N>
NX_INLINE void vst_n(uint64_t a, const V128& v) {
    std::memcpy(reinterpret_cast<void*>(a), v.u8, N);
}

// LD2..LD4 / ST2..ST4 without a lane (interleaved): element e of structure s is at a + (e*NS + s)*ES, and
// register s holds it in lane e. L lanes per register; the bytes above L*ES of each register are zero.
template <int NS, int ES, int L>
NX_INLINE void vld_il(V128 (&t)[4], uint64_t a) {
    for (int s = 0; s < NS; s++) t[s] = vzero();
    for (int e = 0; e < L; e++)
        for (int s = 0; s < NS; s++)
            std::memcpy(t[s].u8 + e * ES, reinterpret_cast<const void*>(a + (e * NS + s) * ES), ES);
}

template <int NS, int ES, int L>
NX_INLINE void vst_il(uint64_t a, const V128 (&t)[4]) {
    for (int e = 0; e < L; e++)
        for (int s = 0; s < NS; s++)
            std::memcpy(reinterpret_cast<void*>(a + (e * NS + s) * ES), t[s].u8 + e * ES, ES);
}

// Single-lane LDn / STn: only lane `lane` of the register changes; the other lanes are kept.
template <int ES>
NX_INLINE void vld_lane(V128& v, uint64_t a, int lane) {
    std::memcpy(v.u8 + lane * ES, reinterpret_cast<const void*>(a), ES);
}

template <int ES>
NX_INLINE void vst_lane(uint64_t a, const V128& v, int lane) {
    std::memcpy(reinterpret_cast<void*>(a), v.u8 + lane * ES, ES);
}

// LDnR: one element of ES bytes, copied into all L lanes of the register (upper bytes zero).
template <int ES, int L>
NX_INLINE V128 vld_repl(uint64_t a) {
    V128 r = vzero();
    for (int j = 0; j < L; j++) std::memcpy(r.u8 + j * ES, reinterpret_cast<const void*>(a), ES);
    return r;
}

}  // namespace nx
