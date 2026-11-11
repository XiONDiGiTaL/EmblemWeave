//! The C math library the module imports.
//!
//! nnSdk's C library is musl, so every function here is computed by the `libm`
//! crate, a line-by-line port of musl's libm: the results are the bits the
//! console's own library would produce, not merely "IEEE-correct to an ulp".
//!
//! Calling convention (AAPCS64): floating point arguments arrive in `s0`/`d0`,
//! `s1`/`d1`, ... (the low lanes of `v0`, `v1`, ...), integer and pointer
//! arguments in `x0`, `x1`, ... independently of the float ones. The result is
//! written to `v0` with the rest of the register zeroed, which is what a scalar
//! floating point write does; `x0` is left alone.

use crate::hle::{write_bytes, HostResult, HostServices};
use ew_lift::State;

fn fa(state: &State, i: usize) -> f32 {
    f32::from_bits(state.v[i][0] as u32)
}

fn da(state: &State, i: usize) -> f64 {
    f64::from_bits(state.v[i][0])
}

fn fr(state: &mut State, v: f32) -> HostResult {
    state.v[0] = [v.to_bits() as u64, 0];
    HostResult::Returned(state.x[0])
}

fn dr(state: &mut State, v: f64) -> HostResult {
    state.v[0] = [v.to_bits(), 0];
    HostResult::Returned(state.x[0])
}

macro_rules! f1 {
    ($($name:ident),* $(,)?) => {$(
        fn $name(s: &mut State) -> HostResult {
            let a = fa(s, 0);
            fr(s, libm::$name(a))
        }
    )*};
}
macro_rules! f2 {
    ($($name:ident),* $(,)?) => {$(
        fn $name(s: &mut State) -> HostResult {
            let (a, b) = (fa(s, 0), fa(s, 1));
            fr(s, libm::$name(a, b))
        }
    )*};
}
macro_rules! d1 {
    ($($name:ident),* $(,)?) => {$(
        fn $name(s: &mut State) -> HostResult {
            let a = da(s, 0);
            dr(s, libm::$name(a))
        }
    )*};
}
macro_rules! d2 {
    ($($name:ident),* $(,)?) => {$(
        fn $name(s: &mut State) -> HostResult {
            let (a, b) = (da(s, 0), da(s, 1));
            dr(s, libm::$name(a, b))
        }
    )*};
}

f1!(
    cosf, sinf, tanf, acosf, asinf, atanf, sqrtf, fabsf, floorf, ceilf, truncf, roundf, rintf, expf,
    logf, log2f, log10f, cbrtf, exp2f, sinhf, coshf, tanhf, asinhf, acoshf, atanhf, expm1f, log1pf,
    erff, erfcf, tgammaf, lgammaf, j0f, j1f, y0f, y1f
);
f2!(powf, atan2f, fmodf, fmaxf, fminf, copysignf, hypotf, remainderf, nextafterf, fdimf);
d1!(cos, sin, tan, acos, asin, atan, sqrt, fabs, floor, ceil, trunc, round, exp, exp2, log, log2, log10);
d2!(pow, fmod, atan2);

/// `logbf`, written as musl writes it (the libm crate does not carry it).
fn logbf(s: &mut State) -> HostResult {
    let x = fa(s, 0);
    let r = if !x.is_finite() {
        x * x
    } else if x == 0.0 {
        -1.0 / (x * x)
    } else {
        libm::ilogbf(x) as f32
    };
    fr(s, r)
}

fn fmaf(s: &mut State) -> HostResult {
    let (a, b, c) = (fa(s, 0), fa(s, 1), fa(s, 2));
    fr(s, libm::fmaf(a, b, c))
}

/// `ldexpf(float, int)` and `scalbnf(float, int)`: the integer is in `w0`.
fn ldexpf(s: &mut State) -> HostResult {
    let (a, e) = (fa(s, 0), s.x[0] as i32);
    fr(s, libm::ldexpf(a, e))
}
fn scalbnf(s: &mut State) -> HostResult {
    let (a, e) = (fa(s, 0), s.x[0] as i32);
    fr(s, libm::scalbnf(a, e))
}

/// `int ilogbf(float)` returns in `w0`.
fn ilogbf(s: &mut State) -> HostResult {
    HostResult::Returned(libm::ilogbf(fa(s, 0)) as u32 as u64)
}

/// `frexpf(float, int* exp)`.
fn frexpf(s: &mut State) -> HostResult {
    let (m, e) = libm::frexpf(fa(s, 0));
    if let Err(err) = write_bytes(s, s.x[0], &e.to_le_bytes()) {
        return err;
    }
    fr(s, m)
}

/// `modff(float, float* iptr)`.
fn modff(s: &mut State) -> HostResult {
    let (frac, int) = libm::modff(fa(s, 0));
    if let Err(err) = write_bytes(s, s.x[0], &int.to_bits().to_le_bytes()) {
        return err;
    }
    fr(s, frac)
}

/// `remquof(float, float, int* quo)`.
fn remquof(s: &mut State) -> HostResult {
    let (r, q) = libm::remquof(fa(s, 0), fa(s, 1));
    if let Err(err) = write_bytes(s, s.x[0], &q.to_le_bytes()) {
        return err;
    }
    fr(s, r)
}

/// `nanf(const char*)`: musl ignores the tag and returns the quiet NaN.
fn nanf(s: &mut State) -> HostResult {
    fr(s, f32::from_bits(0x7FC0_0000))
}

/// `jnf(int, float)` / `ynf(int, float)`.
fn jnf(s: &mut State) -> HostResult {
    let (n, x) = (s.x[0] as i32, fa(s, 0));
    fr(s, libm::jnf(n, x))
}
fn ynf(s: &mut State) -> HostResult {
    let (n, x) = (s.x[0] as i32, fa(s, 0));
    fr(s, libm::ynf(n, x))
}

/// Register the C math family the module imports.
pub fn register(services: &mut HostServices) {
    let table: &[(&str, fn(&mut State) -> HostResult)] = &[
        ("cosf", cosf), ("sinf", sinf), ("tanf", tanf), ("acosf", acosf), ("asinf", asinf),
        ("atanf", atanf), ("sqrtf", sqrtf), ("fabsf", fabsf), ("floorf", floorf), ("ceilf", ceilf),
        ("truncf", truncf), ("roundf", roundf), ("rintf", rintf), ("expf", expf), ("logf", logf),
        ("log2f", log2f), ("log10f", log10f), ("cbrtf", cbrtf), ("exp2f", exp2f), ("sinhf", sinhf),
        ("coshf", coshf), ("tanhf", tanhf), ("asinhf", asinhf), ("acoshf", acoshf), ("atanhf", atanhf),
        ("expm1f", expm1f), ("log1pf", log1pf), ("logbf", logbf), ("erff", erff), ("erfcf", erfcf),
        ("tgammaf", tgammaf), ("lgammaf", lgammaf), ("j0f", j0f), ("j1f", j1f), ("y0f", y0f),
        ("y1f", y1f), ("jnf", jnf), ("ynf", ynf), ("powf", powf), ("atan2f", atan2f), ("fmodf", fmodf),
        ("fmaxf", fmaxf), ("fminf", fminf), ("copysignf", copysignf), ("hypotf", hypotf),
        ("remainderf", remainderf), ("nextafterf", nextafterf), ("fdimf", fdimf), ("fmaf", fmaf),
        ("ldexpf", ldexpf), ("scalbnf", scalbnf), ("ilogbf", ilogbf), ("frexpf", frexpf),
        ("modff", modff), ("remquof", remquof), ("nanf", nanf), ("cos", cos), ("sin", sin),
        ("tan", tan), ("acos", acos), ("asin", asin), ("atan", atan), ("sqrt", sqrt), ("fabs", fabs),
        ("floor", floor), ("ceil", ceil), ("trunc", trunc), ("round", round), ("exp", exp),
        ("exp2", exp2), ("log", log), ("log2", log2), ("log10", log10), ("pow", pow), ("fmod", fmod),
        ("atan2", atan2),
    ];
    for (name, handler) in table {
        services.register(name, *handler, "musl libm (via the libm crate), arguments in s/d registers, result in v0");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_f(args: &[f32]) -> State {
        let mut s = State::default();
        for (i, a) in args.iter().enumerate() {
            s.v[i] = [a.to_bits() as u64, 0];
        }
        s
    }

    #[test]
    fn arguments_come_from_s_registers_not_x0() {
        let mut s = with_f(&[0.0]);
        s.x[0] = 0x4049_0FDB; // pi as bits: must be ignored
        cosf(&mut s);
        assert_eq!(f32::from_bits(s.v[0][0] as u32), 1.0);
        assert_eq!(s.v[0][1], 0, "a scalar write clears the upper half");
    }

    #[test]
    fn binary_and_mixed_signatures() {
        let mut s = with_f(&[1.5, -2.0]);
        fmaxf(&mut s);
        assert_eq!(f32::from_bits(s.v[0][0] as u32), 1.5);
        let mut s = with_f(&[3.0]);
        s.x[0] = 4;
        ldexpf(&mut s);
        assert_eq!(f32::from_bits(s.v[0][0] as u32), 48.0);
        let mut s = with_f(&[8.0]);
        s.x[0] = 0x100;
        frexpf(&mut s);
        assert_eq!(f32::from_bits(s.v[0][0] as u32), 0.5);
        assert_eq!(i32::from_le_bytes(s.mem[0x100..0x104].try_into().unwrap()), 4);
    }

    #[test]
    fn double_results_land_in_d0() {
        let mut s = State::default();
        s.v[0] = [9.0f64.to_bits(), 0];
        sqrt(&mut s);
        assert_eq!(f64::from_bits(s.v[0][0]), 3.0);
    }

    #[test]
    fn results_match_musl() {
        // musl's sinf(1.0f) is 0x3f576aa4.
        let mut s = with_f(&[1.0]);
        sinf(&mut s);
        assert_eq!(s.v[0][0] as u32, 0x3F57_6AA4);
    }
}
