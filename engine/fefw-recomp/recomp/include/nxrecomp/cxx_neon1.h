// SPDX-License-Identifier: GPL-3.0-or-later
// Helpers for tier-2 code of one instruction family (see nxrecomp/lift/tier2/x_*.py). Included at the end of cxx.h.
// Family neon1: AdvSIMD integer narrowing, saturating and shift forms (x_neon1.py).
//
// Lane values travel as zero-extended uint64_t holding the element's bit pattern; E is the element width in bits
// (8, 16, 32 or 64) unless a helper says otherwise. Intermediate arithmetic is done in __int128 where a 64-bit lane
// could overflow. The FPSR.QC flag is not modelled.
#pragma once
#include <cstdint>

namespace nx {

// Low E bits set.
template <int E>
inline constexpr uint64_t n1_ones = E >= 64 ? ~0ull : ((1ull << E) - 1);

// The low E bits of v, sign-extended to 64 bits.
template <int E>
inline int64_t n1_sext(uint64_t v) {
    if constexpr (E >= 64) {
        return int64_t(v);
    } else {
        return int64_t(v << (64 - E)) >> (64 - E);
    }
}

// Saturate an exact value to a W-bit signed / unsigned range; the result is the W-bit pattern.
template <int W>
inline uint64_t n1_sat_s(__int128 v) {
    const __int128 hi = (__int128(1) << (W - 1)) - 1;
    const __int128 lo = -(__int128(1) << (W - 1));
    if (v > hi) v = hi;
    if (v < lo) v = lo;
    return uint64_t(v) & n1_ones<W>;
}

template <int W>
inline uint64_t n1_sat_u(__int128 v) {
    const __int128 hi = (__int128(1) << W) - 1;
    if (v > hi) v = hi;
    if (v < 0) v = 0;
    return uint64_t(v);
}

// Non-rounding right shift by k >= 1. Signed: arithmetic (sign fill once k reaches the width). Unsigned: zero fill.
template <int E, bool Sg>
inline uint64_t n1_shr(uint64_t a, int k) {
    if constexpr (Sg) {
        const int64_t x = n1_sext<E>(a);
        return uint64_t(x >> (k > 63 ? 63 : k)) & n1_ones<E>;
    } else {
        return k >= 64 ? 0 : (a >> k);
    }
}

// Rounding right shift by k >= 1: (x + 2^(k-1)) >> k computed exactly (the SRSHR / URSHR / SRSHL / URSHL rule).
// For k > E + 1 the result is 0 for both signednesses; the clamp keeps the shift in range.
template <int E, bool Sg>
inline uint64_t n1_rshr(uint64_t a, int k) {
    if (k > E + 1) return 0;
    if constexpr (Sg) {
        const __int128 x = n1_sext<E>(a);
        return uint64_t((x + (__int128(1) << (k - 1))) >> k) & n1_ones<E>;
    } else {
        return uint64_t((__int128(a) + (__int128(1) << (k - 1))) >> k) & n1_ones<E>;
    }
}

// Saturating left shift by k >= 0 of an E-bit value (SQSHL / UQSHL immediate and register forms).
template <int E, bool Sg>
inline uint64_t n1_lsl_sat(uint64_t a, int k) {
    if constexpr (Sg) {
        const int64_t x = n1_sext<E>(a);
        if (k >= E) {
            if (x == 0) return 0;
            return x > 0 ? n1_sat_s<E>(__int128(1) << 100) : n1_sat_s<E>(-(__int128(1) << 100));
        }
        return n1_sat_s<E>(__int128(x) * (__int128(1) << k));
    } else {
        if (k >= E) return a == 0 ? 0 : n1_ones<E>;
        return n1_sat_u<E>(__int128(a) * (__int128(1) << k));
    }
}

// Register-count shifts: the count is the signed low byte of the count element.
//   Sat: saturating left shift (SQSHL / UQSHL) instead of a truncating one (SSHL / USHL / SRSHL / URSHL).
//   Rnd: rounding right shift (SRSHL / URSHL / SQRSHL / UQRSHL) instead of a plain one.
template <int E, bool Sg, bool Sat, bool Rnd>
inline uint64_t n1_shl_reg(uint64_t a, uint64_t cnt) {
    const int k = int8_t(uint8_t(cnt));
    if (k >= 0) {
        if constexpr (Sat) {
            return n1_lsl_sat<E, Sg>(a, k);
        } else {
            return k >= E ? 0 : ((a << k) & n1_ones<E>);
        }
    }
    if constexpr (Rnd) {
        return n1_rshr<E, Sg>(a, -k);
    } else {
        return n1_shr<E, Sg>(a, -k);
    }
}

// SQSHL / UQSHL by immediate (0 <= n < E).
template <int E, bool Sg>
inline uint64_t n1_qshl_imm(uint64_t a, int n) {
    return n1_lsl_sat<E, Sg>(a, n);
}

// SQSHLU by immediate: signed input, unsigned saturated result (0 <= n < E).
template <int E>
inline uint64_t n1_qshlu_imm(uint64_t a, int n) {
    const int64_t x = n1_sext<E>(a);
    if (x < 0) return 0;
    return n1_sat_u<E>(__int128(x) * (__int128(1) << n));
}

// SLI: shift left by n (0 <= n < E) and insert; the low n bits of the destination are kept.
template <int E>
inline uint64_t n1_sli(uint64_t a, uint64_t d, int n) {
    const uint64_t hi = (n1_ones<E> << n) & n1_ones<E>;
    return (((a << n) & n1_ones<E>) & hi) | (d & ~hi & n1_ones<E>);
}

// SRI: shift right by n (1 <= n <= E) and insert; the top n bits of the destination are kept.
template <int E>
inline uint64_t n1_sri(uint64_t a, uint64_t d, int n) {
    if (n >= E) return d & n1_ones<E>;
    const uint64_t lo = n1_ones<E> >> n;
    return ((a >> n) & lo) | (d & ~lo & n1_ones<E>);
}

// ADDHN / SUBHN / RADDHN / RSUBHN: bits [2E-1:E] of the 2E-bit sum or difference (plus 2^(E-1) when rounding).
// E is the destination width; the sources are 2E bits, zero-extended in a and b. Bits [2E-1:E] of a 64-bit
// modular sum equal those of the exact sum, so uint64 arithmetic is enough.
template <int E>
inline uint64_t n1_hn(uint64_t a, uint64_t b, bool sub, bool rnd) {
    uint64_t t = sub ? a - b : a + b;
    if (rnd) t += uint64_t(1) << (E - 1);
    return (t >> E) & n1_ones<E>;
}

// SQDMULL (mode 0), SQDMLAL (mode 1), SQDMLSL (mode 2). E is the source element width; the result is 2E bits:
// sat_2E( [acc +/-] 2 * a * b ), with a, b signed E-bit and acc signed 2E-bit.
template <int E>
inline uint64_t n1_dmul(uint64_t a, uint64_t b, uint64_t acc, int mode) {
    const __int128 p0 = __int128(n1_sext<E>(a)) * __int128(n1_sext<E>(b)) * 2;
    if (mode == 0) return n1_sat_s<2 * E>(p0);
    // the doubled product saturates on its own FIRST (INT_MIN * INT_MIN * 2 = 2^(2E-1) becomes 2^(2E-1) - 1), then the
    // accumulate saturates again (measured on Unicorn, matches the Arm ARM SQDMLAL / SQDMLSL pseudocode)
    const __int128 p = n1_sext<2 * E>(n1_sat_s<2 * E>(p0));
    const __int128 s = n1_sext<2 * E>(acc);
    return n1_sat_s<2 * E>(mode == 1 ? s + p : s - p);
}

// Narrowing right shift, exact: (x [+ 2^(n-1)]) >> n for a W-bit source x (1 <= n <= W/2). The caller saturates.
template <int W, bool Sg, bool Rnd>
inline __int128 n1_wide_shr(uint64_t a, int n) {
    __int128 x = Sg ? __int128(n1_sext<W>(a)) : __int128(a);
    if (Rnd) x += __int128(1) << (n - 1);
    return x >> n;
}

}  // namespace nx
