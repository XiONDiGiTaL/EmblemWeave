// SPDX-License-Identifier: GPL-3.0-or-later
// Helpers for tier-2 code of one instruction family (see nxrecomp/lift/tier2/x_*.py). Included at the end of cxx.h.
#pragma once
namespace nx {

// Bit-level IEEE 754 helpers for the fp handlers (FMULX, FRECPX, FMAXV/FMINV, FCVTXN). Everything works on the raw
// bit pattern, so NaN payloads, signs of zero and signalling/quiet state are exact. FPCR.DN = 0 (NaNs propagate),
// round to nearest even, no flush-to-zero.

template <typename F>
struct fpm_fmt;
template <>
struct fpm_fmt<float> {
    using U = uint32_t;
    static constexpr int kExp = 8, kFrac = 23;
};
template <>
struct fpm_fmt<double> {
    using U = uint64_t;
    static constexpr int kExp = 11, kFrac = 52;
};

template <typename F>
using fpm_u = typename fpm_fmt<F>::U;

template <typename F>
inline fpm_u<F> fpm_raw(F x) {
    fpm_u<F> b;
    std::memcpy(&b, &x, sizeof b);
    return b;
}

template <typename F>
inline F fpm_from(fpm_u<F> b) {
    F x;
    std::memcpy(&x, &b, sizeof x);
    return x;
}

template <typename F>
constexpr fpm_u<F> fpm_sign_bit() { return fpm_u<F>(1) << (sizeof(F) * 8 - 1); }

template <typename F>
constexpr fpm_u<F> fpm_exp_mask() { return ((fpm_u<F>(1) << fpm_fmt<F>::kExp) - 1) << fpm_fmt<F>::kFrac; }

template <typename F>
constexpr fpm_u<F> fpm_frac_mask() { return (fpm_u<F>(1) << fpm_fmt<F>::kFrac) - 1; }

template <typename F>
constexpr fpm_u<F> fpm_quiet_bit() { return fpm_u<F>(1) << (fpm_fmt<F>::kFrac - 1); }

template <typename F>
inline bool fpm_is_nan(F x) {
    auto b = fpm_raw(x);
    return (b & fpm_exp_mask<F>()) == fpm_exp_mask<F>() && (b & fpm_frac_mask<F>()) != 0;
}

template <typename F>
inline bool fpm_is_snan(F x) { return fpm_is_nan(x) && !(fpm_raw(x) & fpm_quiet_bit<F>()); }

template <typename F>
inline bool fpm_is_qnan(F x) { return fpm_is_nan(x) && (fpm_raw(x) & fpm_quiet_bit<F>()) != 0; }

template <typename F>
inline bool fpm_is_inf(F x) {
    auto b = fpm_raw(x);
    return (b & (fpm_exp_mask<F>() | fpm_frac_mask<F>())) == fpm_exp_mask<F>();
}

template <typename F>
inline bool fpm_is_zero(F x) { return (fpm_raw(x) & ~fpm_sign_bit<F>()) == 0; }

template <typename F>
inline bool fpm_negative(F x) { return (fpm_raw(x) & fpm_sign_bit<F>()) != 0; }

template <typename F>
inline F fpm_quiet(F x) { return fpm_from<F>(fpm_raw(x) | fpm_quiet_bit<F>()); }

// FPProcessNaNs for two operands (FPCR.DN = 0), at least one of them NaN: the first signalling NaN wins (quietened),
// else the first quiet NaN, operand 1 before operand 2.
template <typename F>
inline F fpm_nan2(F a, F b) {
    if (fpm_is_snan(a)) return fpm_quiet(a);
    if (fpm_is_snan(b)) return fpm_quiet(b);
    if (fpm_is_qnan(a)) return a;
    return b;
}

// FMULX: as FMUL, except that infinity times zero gives +/-2.0 (sign = XOR of the operand signs).
template <typename F>
inline F fpm_mulx(F a, F b) {
    if (fpm_is_nan(a) || fpm_is_nan(b)) return fpm_nan2(a, b);
    if ((fpm_is_inf(a) && fpm_is_zero(b)) || (fpm_is_zero(a) && fpm_is_inf(b))) {
        bool neg = fpm_negative(a) != fpm_negative(b);
        F two = F(2.0);
        return neg ? -two : two;
    }
    return a * b;
}

// FPMax / FPMin: NaNs propagate; +0 is the larger zero and -0 the smaller.
template <typename F>
inline F fpm_max(F a, F b) {
    if (fpm_is_nan(a) || fpm_is_nan(b)) return fpm_nan2(a, b);
    if (fpm_is_zero(a) && fpm_is_zero(b)) return fpm_negative(a) ? b : a;
    return a > b ? a : b;
}

template <typename F>
inline F fpm_min(F a, F b) {
    if (fpm_is_nan(a) || fpm_is_nan(b)) return fpm_nan2(a, b);
    if (fpm_is_zero(a) && fpm_is_zero(b)) return fpm_negative(a) ? a : b;
    return a < b ? a : b;
}

// FRECPX (Arm ARM FPRecpX): NaN is quietened; a zero or denormal input gives the largest finite exponent, infinity
// gives a signed zero, a normal input has its exponent field inverted; the fraction is always zero, the sign is kept.
template <typename F>
inline F fpm_recpx(F x) {
    using U = fpm_u<F>;
    constexpr int kE = fpm_fmt<F>::kExp, kF = fpm_fmt<F>::kFrac;
    if (fpm_is_nan(x)) return fpm_quiet(x);
    U b = fpm_raw(x);
    U sign = b & fpm_sign_bit<F>();
    U e = (b & fpm_exp_mask<F>()) >> kF;
    U all = (U(1) << kE) - 1;
    U r;
    if (e == 0) r = sign | ((all - 1) << kF);
    else if (e == all) r = sign;
    else r = sign | ((all ^ e) << kF);
    return fpm_from<F>(r);
}

// FCVTXN element: double -> float with round-to-odd. Round to nearest, and when that is inexact take the truncated
// value (one step towards zero in magnitude) with its lowest bit set. NaN keeps its sign and top 23 payload bits and is
// quietened; an overflow gives the largest finite float with the sign.
inline float fpm_fcvtxn(double x) {
    uint32_t sign = uint32_t(fpm_raw(x) >> 32) & 0x80000000u;
    if (fpm_is_nan(x)) {
        uint32_t frac = uint32_t((fpm_raw(x) & fpm_frac_mask<double>()) >> 29);
        return fpm_from<float>(sign | 0x7f800000u | frac | 0x00400000u);
    }
    if (fpm_is_inf(x)) return fpm_from<float>(sign | 0x7f800000u);
    float t = static_cast<float>(x);
    if (std::isinf(t)) return fpm_from<float>(sign | 0x7f7fffffu);
    if (static_cast<double>(t) == x) return t;
    uint32_t mag = fpm_raw(t) & 0x7fffffffu;
    if (std::fabs(static_cast<double>(t)) > std::fabs(x)) mag -= 1;
    return fpm_from<float>(sign | (mag | 1u));
}

}  // namespace nx
