// SPDX-License-Identifier: GPL-3.0-or-later
// Helpers for tier-2 code of one instruction family (see nxrecomp/lift/tier2/x_*.py). Included at the end of cxx.h.
#pragma once
namespace nx {

// URECPE / URSQRTE on one 32-bit element (Arm ARM UnsignedRecipEstimate / UnsignedRecipSqrtEstimate): the table
// lookups are the ones of fpexact.h, applied to bits 31..23 of the operand; an operand below 2^31 (URECPE) or below
// 2^30 (URSQRTE) gives 0xFFFFFFFF.
inline uint32_t urecpe_(uint32_t x) { return (x & 0x80000000u) ? ::recip_estimate(x >> 23) << 23 : 0xFFFFFFFFu; }
inline uint32_t ursqrte_(uint32_t x) { return (x >> 30) ? ::recip_sqrt_estimate(x >> 23) << 23 : 0xFFFFFFFFu; }

// Carry-less (polynomial) product of two bytes, 15 bits significant (PMULL 8-bit, PMUL keeps the low 8 bits).
inline uint16_t pmull8_(uint64_t a, uint64_t b) {
    uint16_t r = 0;
    for (int k = 0; k < 8; k++)
        if ((b >> k) & 1) r ^= uint16_t(a << k);
    return r;
}

// RBIT on bytes: the bits of each byte in reverse order (vector RBIT).
inline uint8_t rbit8_(uint8_t v) {
    uint8_t r = 0;
    for (int k = 0; k < 8; k++) r |= uint8_t(((v >> k) & 1) << (7 - k));
    return r;
}

}  // namespace nx
