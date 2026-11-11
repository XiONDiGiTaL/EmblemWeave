//! The C runtime the module imports from nnSdk's musl: strings, character
//! classes, number parsing, formatted I/O, stdio over `nn::fs`, pthreads over
//! the guest scheduler, `rand`, the floating point environment, and the zstd
//! trace hooks.
//!
//! Observable behaviour follows musl where it differs from other C libraries:
//! `%p` prints sixteen hex digits, `rand` is musl's 64-bit LCG, `strto*`
//! saturate and set `errno` to `ERANGE`. Variadic arguments are read with the
//! AAPCS64 rules: general registers then the stack for integers, vector
//! registers then the stack for floating point, and `va_list` is the AAPCS64
//! five-field structure.

use crate::hle::{c_string, read_bytes, write_bytes, HostResult, HostServices};
use ew_lift::{State, ThreadInfo};

const ERANGE: u32 = 34;
const EINVAL: u32 = 22;
const ETIMEDOUT: u64 = 110;

macro_rules! t {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return e,
        }
    };
}

fn r64(s: &State, a: u64) -> Result<u64, HostResult> {
    Ok(u64::from_le_bytes(read_bytes(s, a, 8)?.try_into().unwrap()))
}
fn r32(s: &State, a: u64) -> Result<u32, HostResult> {
    Ok(u32::from_le_bytes(read_bytes(s, a, 4)?.try_into().unwrap()))
}
fn cstr(s: &State, a: u64) -> Result<Vec<u8>, HostResult> {
    c_string(s, a, 1 << 20)
}

// ---- errno -----------------------------------------------------------------------

/// Per-thread `errno`, allocated on first use from the host arena.
fn errno_addr(s: &mut State) -> Result<u64, HostResult> {
    let key = format!("errno:{:x}", s.host.thread_object);
    if let Some(&a) = s.host.named.get(&key) {
        return Ok(a);
    }
    let a = s.host.allocate(16, 16).map_err(HostResult::Unavailable)?;
    write_bytes(s, a, &[0u8; 4])?;
    s.host.named.insert(key, a);
    Ok(a)
}
fn set_errno(s: &mut State, e: u32) -> Result<(), HostResult> {
    let a = errno_addr(s)?;
    write_bytes(s, a, &e.to_le_bytes())
}
fn errno_location(s: &mut State) -> HostResult {
    match errno_addr(s) {
        Ok(a) => HostResult::Returned(a),
        Err(e) => e,
    }
}

// ---- strings and characters --------------------------------------------------------

fn strcat(s: &mut State) -> HostResult {
    let (dst, src) = (s.x[0], s.x[1]);
    let d = t!(cstr(s, dst));
    let mut b = t!(cstr(s, src));
    b.push(0);
    t!(write_bytes(s, dst + d.len() as u64, &b));
    HostResult::Returned(dst)
}

fn strncat(s: &mut State) -> HostResult {
    let (dst, src, n) = (s.x[0], s.x[1], s.x[2]);
    let d = t!(cstr(s, dst));
    let mut b = t!(c_string(s, src, n.saturating_add(1)).or_else(|_| read_bytes(s, src, n)));
    b.truncate(n as usize);
    if let Some(z) = b.iter().position(|&c| c == 0) {
        b.truncate(z);
    }
    b.push(0);
    t!(write_bytes(s, dst + d.len() as u64, &b));
    HostResult::Returned(dst)
}

fn toupper(s: &mut State) -> HostResult {
    let c = s.x[0] as i32;
    HostResult::Returned(if (b'a' as i32..=b'z' as i32).contains(&c) { c - 32 } else { c } as u32 as u64)
}
fn tolower(s: &mut State) -> HostResult {
    let c = s.x[0] as i32;
    HostResult::Returned(if (b'A' as i32..=b'Z' as i32).contains(&c) { c + 32 } else { c } as u32 as u64)
}
fn islower(s: &mut State) -> HostResult {
    HostResult::Returned((b'a' as u64..=b'z' as u64).contains(&(s.x[0] & 0xFFFF_FFFF)) as u64)
}
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}
fn isspace(s: &mut State) -> HostResult {
    let c = s.x[0] as u32;
    HostResult::Returned((c < 256 && is_space(c as u8)) as u64)
}
fn abs(s: &mut State) -> HostResult {
    HostResult::Returned((s.x[0] as i32).wrapping_abs() as u32 as u64)
}

fn strcasecmp(s: &mut State) -> HostResult {
    let (a, b) = (t!(cstr(s, s.x[0])), t!(cstr(s, s.x[1])));
    let mut i = 0;
    loop {
        let x = a.get(i).copied().unwrap_or(0).to_ascii_lowercase();
        let y = b.get(i).copied().unwrap_or(0).to_ascii_lowercase();
        if x != y || x == 0 {
            return HostResult::Returned((x as i32 - y as i32) as u32 as u64);
        }
        i += 1;
    }
}

/// `wmemchr(const wchar_t*, wchar_t, size_t)`: wchar_t is 32 bits on AArch64.
fn wmemchr(s: &mut State) -> HostResult {
    let (p, c, n) = (s.x[0], s.x[1] as u32, s.x[2]);
    for i in 0..n {
        if t!(r32(s, p + 4 * i)) == c {
            return HostResult::Returned(p + 4 * i);
        }
    }
    HostResult::Returned(0)
}

// ---- number parsing -------------------------------------------------------------------

/// C integer parse: whitespace, sign, base prefix, digits. Returns the value as
/// an unbounded magnitude with its sign, and the bytes consumed (0 if none).
fn parse_int(b: &[u8], base: u32) -> (bool, u128, bool, usize) {
    let mut i = 0;
    while i < b.len() && is_space(b[i]) {
        i += 1;
    }
    let mut neg = false;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        neg = b[i] == b'-';
        i += 1;
    }
    let mut base = base;
    let hex_prefix = i + 1 < b.len() && b[i] == b'0' && (b[i + 1] | 0x20) == b'x';
    if (base == 0 || base == 16) && hex_prefix && b.get(i + 2).is_some_and(|c| c.is_ascii_hexdigit()) {
        base = 16;
        i += 2;
    } else if base == 0 {
        base = if b.get(i) == Some(&b'0') { 8 } else { 10 };
    }
    let start = i;
    let mut v: u128 = 0;
    let mut overflow = false;
    while i < b.len() {
        let d = match (b[i] as char).to_digit(36) {
            Some(d) if d < base => d,
            _ => break,
        };
        v = v.checked_mul(base as u128).and_then(|x| x.checked_add(d as u128)).unwrap_or_else(|| {
            overflow = true;
            u128::MAX
        });
        i += 1;
    }
    if i == start {
        return (false, 0, false, 0);
    }
    (neg, v, overflow, i)
}

fn store_end(s: &mut State, endp: u64, at: u64) -> Result<(), HostResult> {
    if endp != 0 {
        write_bytes(s, endp, &at.to_le_bytes())?;
    }
    Ok(())
}

fn strtol_like(s: &mut State, signed: bool) -> HostResult {
    let (p, endp, base) = (s.x[0], s.x[1], s.x[2] as u32);
    if base == 1 || base > 36 {
        t!(set_errno(s, EINVAL));
        t!(store_end(s, endp, p));
        return HostResult::Returned(0);
    }
    let b = t!(cstr(s, p));
    let (neg, mag, overflow, used) = parse_int(&b, base);
    t!(store_end(s, endp, p + used as u64));
    if used == 0 {
        return HostResult::Returned(0);
    }
    let v = if signed {
        let limit = if neg { 1u128 << 63 } else { (1u128 << 63) - 1 };
        if overflow || mag > limit {
            t!(set_errno(s, ERANGE));
            if neg { i64::MIN as u64 } else { i64::MAX as u64 }
        } else if neg {
            (mag as i128).wrapping_neg() as u64
        } else {
            mag as u64
        }
    } else if overflow || mag > u64::MAX as u128 {
        t!(set_errno(s, ERANGE));
        u64::MAX
    } else if neg {
        (mag as u64).wrapping_neg()
    } else {
        mag as u64
    };
    HostResult::Returned(v)
}
fn strtol(s: &mut State) -> HostResult {
    strtol_like(s, true)
}
fn strtoul(s: &mut State) -> HostResult {
    strtol_like(s, false)
}
fn atoi(s: &mut State) -> HostResult {
    let b = t!(cstr(s, s.x[0]));
    let (neg, mag, _, _) = parse_int(&b, 10);
    let v = mag as u64 as i64;
    HostResult::Returned((if neg { v.wrapping_neg() } else { v }) as i32 as u32 as u64)
}

/// The longest prefix of `b` that is a C floating point literal, parsed.
/// Supports decimal and the `inf`/`infinity`/`nan` spellings; hexadecimal
/// floats are reported by returning `None` so the caller can stop.
fn parse_float(b: &[u8]) -> Option<(f64, usize)> {
    let mut i = 0;
    while i < b.len() && is_space(b[i]) {
        i += 1;
    }
    let start = i;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let rest: Vec<u8> = b[i..].iter().take(8).map(|c| c.to_ascii_lowercase()).collect();
    for (word, value) in [(&b"infinity"[..], f64::INFINITY), (b"inf", f64::INFINITY), (b"nan", f64::NAN)] {
        if rest.starts_with(word) {
            let neg = b[start] == b'-';
            return Some((if neg { -value } else { value }, i + word.len()));
        }
    }
    if rest.starts_with(b"0x") {
        return None;
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == digits_start || (i == digits_start + 1 && b[digits_start] == b'.') {
        return Some((0.0, 0));
    }
    if i < b.len() && (b[i] | 0x20) == b'e' {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    let text = std::str::from_utf8(&b[start..i]).ok()?;
    Some((text.parse::<f64>().ok()?, i))
}

fn strtod_like(s: &mut State, single: bool) -> HostResult {
    let (p, endp) = (s.x[0], s.x[1]);
    let b = t!(cstr(s, p));
    let Some((v, used)) = parse_float(&b) else {
        return HostResult::Unavailable("strtod of a hexadecimal float is not implemented".into());
    };
    t!(store_end(s, endp, p + used as u64));
    if single {
        let f = v as f32;
        if f.is_infinite() && v.is_finite() {
            t!(set_errno(s, ERANGE));
        }
        s.v[0] = [f.to_bits() as u64, 0];
    } else {
        s.v[0] = [v.to_bits(), 0];
    }
    HostResult::Returned(s.x[0])
}
fn strtod(s: &mut State) -> HostResult {
    strtod_like(s, false)
}
fn strtof(s: &mut State) -> HostResult {
    strtod_like(s, true)
}
fn atof(s: &mut State) -> HostResult {
    s.x[1] = 0;
    strtod_like(s, false)
}

// ---- variadic arguments ------------------------------------------------------------------

/// Where the next variadic argument comes from.
enum Va {
    /// Directly after the named arguments of a variadic call.
    Regs { gr: usize, vr: usize, stack: u64 },
    /// An AAPCS64 `va_list` in guest memory.
    List { at: u64 },
}

impl Va {
    fn int(&mut self, s: &mut State) -> Result<u64, HostResult> {
        match self {
            Va::Regs { gr, stack, .. } => {
                if *gr < 8 {
                    *gr += 1;
                    Ok(s.x[*gr - 1])
                } else {
                    let v = r64(s, *stack)?;
                    *stack += 8;
                    Ok(v)
                }
            }
            Va::List { at } => {
                let at = *at;
                let gr_offs = r32(s, at + 24)? as i32;
                if gr_offs < 0 {
                    let gr_top = r64(s, at + 8)?;
                    let v = r64(s, gr_top.wrapping_add(gr_offs as i64 as u64))?;
                    write_bytes(s, at + 24, &(gr_offs + 8).to_le_bytes())?;
                    Ok(v)
                } else {
                    let stack = r64(s, at)?;
                    let v = r64(s, stack)?;
                    write_bytes(s, at, &(stack + 8).to_le_bytes())?;
                    Ok(v)
                }
            }
        }
    }

    fn double(&mut self, s: &mut State) -> Result<f64, HostResult> {
        match self {
            Va::Regs { vr, stack, .. } => {
                if *vr < 8 {
                    *vr += 1;
                    Ok(f64::from_bits(s.v[*vr - 1][0]))
                } else {
                    let v = r64(s, *stack)?;
                    *stack += 8;
                    Ok(f64::from_bits(v))
                }
            }
            Va::List { at } => {
                let at = *at;
                let vr_offs = r32(s, at + 28)? as i32;
                if vr_offs < 0 {
                    let vr_top = r64(s, at + 16)?;
                    let v = r64(s, vr_top.wrapping_add(vr_offs as i64 as u64))?;
                    write_bytes(s, at + 28, &(vr_offs + 16).to_le_bytes())?;
                    Ok(f64::from_bits(v))
                } else {
                    let stack = r64(s, at)?;
                    let v = r64(s, stack)?;
                    write_bytes(s, at, &(stack + 8).to_le_bytes())?;
                    Ok(f64::from_bits(v))
                }
            }
        }
    }
}

// ---- printf ---------------------------------------------------------------------------------

fn pad(out: &mut Vec<u8>, body: &[u8], width: usize, left: bool, zero_after: Option<usize>) {
    let n = body.len();
    if n >= width {
        out.extend_from_slice(body);
    } else if left {
        out.extend_from_slice(body);
        out.extend(std::iter::repeat_n(b' ', width - n));
    } else if let Some(prefix) = zero_after {
        out.extend_from_slice(&body[..prefix]);
        out.extend(std::iter::repeat_n(b'0', width - n));
        out.extend_from_slice(&body[prefix..]);
    } else {
        out.extend(std::iter::repeat_n(b' ', width - n));
        out.extend_from_slice(body);
    }
}

/// `%e` digits for `v >= 0` with `prec` fraction digits: (mantissa, exponent).
fn exp_parts(v: f64, prec: usize) -> (String, i32) {
    let s = format!("{v:.prec$e}");
    let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
    (m.to_string(), e.parse().unwrap_or(0))
}

fn fmt_float(v: f64, conv: u8, prec: Option<usize>, alt: bool) -> String {
    let upper = conv.is_ascii_uppercase();
    let lower = conv.to_ascii_lowercase();
    let mag = v.abs();
    if !mag.is_finite() {
        let t = if mag.is_nan() { "nan" } else { "inf" };
        return if upper { t.to_uppercase() } else { t.to_string() };
    }
    let p = prec.unwrap_or(6);
    let exp_form = |p: usize| {
        let (m, e) = exp_parts(mag, p);
        let m = if alt && p == 0 { format!("{m}.") } else { m };
        format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
    };
    let mut s = match lower {
        b'f' => {
            let s = format!("{mag:.p$}");
            if alt && p == 0 { format!("{s}.") } else { s }
        }
        b'e' => exp_form(p),
        _ => {
            // %g: precision P (0 means 1); style from the %e exponent.
            let pp = if p == 0 { 1 } else { p };
            let (_, x) = exp_parts(mag, pp - 1);
            let mut s = if (pp as i32) > x && x >= -4 {
                format!("{mag:.*}", (pp as i32 - 1 - x) as usize)
            } else {
                exp_form(pp - 1)
            };
            if !alt {
                let (num, tail) = match s.find('e') {
                    Some(i) => (s[..i].to_string(), s[i..].to_string()),
                    None => (s.clone(), String::new()),
                };
                let num = if num.contains('.') {
                    num.trim_end_matches('0').trim_end_matches('.').to_string()
                } else {
                    num
                };
                s = num + &tail;
            }
            s
        }
    };
    if upper {
        s = s.to_uppercase();
    }
    s
}

/// Format `fmt` with arguments from `va`, as musl's printf does.
fn format(s: &mut State, fmt: &[u8], va: &mut Va) -> Result<Vec<u8>, HostResult> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' {
            out.push(fmt[i]);
            i += 1;
            continue;
        }
        i += 1;
        let (mut left, mut plus, mut space, mut alt, mut zero) = (false, false, false, false, false);
        while i < fmt.len() {
            match fmt[i] {
                b'-' => left = true,
                b'+' => plus = true,
                b' ' => space = true,
                b'#' => alt = true,
                b'0' => zero = true,
                _ => break,
            }
            i += 1;
        }
        let mut width = 0usize;
        if fmt.get(i) == Some(&b'*') {
            let w = va.int(s)? as i32;
            if w < 0 {
                left = true;
            }
            width = w.unsigned_abs() as usize;
            i += 1;
        } else {
            while i < fmt.len() && fmt[i].is_ascii_digit() {
                width = width * 10 + (fmt[i] - b'0') as usize;
                i += 1;
            }
        }
        let mut prec = None;
        if fmt.get(i) == Some(&b'.') {
            i += 1;
            if fmt.get(i) == Some(&b'*') {
                let p = va.int(s)? as i32;
                prec = (p >= 0).then_some(p as usize);
                i += 1;
            } else {
                let mut p = 0;
                while i < fmt.len() && fmt[i].is_ascii_digit() {
                    p = p * 10 + (fmt[i] - b'0') as usize;
                    i += 1;
                }
                prec = Some(p);
            }
        }
        let mut bits = 32;
        loop {
            match fmt.get(i) {
                Some(b'h') => bits = if bits == 16 { 8 } else { 16 },
                Some(b'l') | Some(b'j') | Some(b'z') | Some(b't') | Some(b'q') => bits = 64,
                Some(b'L') => bits = 128,
                _ => break,
            }
            i += 1;
        }
        let Some(&conv) = fmt.get(i) else { break };
        i += 1;
        let sign_of = |neg: bool| if neg { "-" } else if plus { "+" } else if space { " " } else { "" };
        match conv {
            b'%' => out.push(b'%'),
            b'd' | b'i' => {
                let raw = va.int(s)?;
                let v: i64 = match bits {
                    8 => raw as i8 as i64,
                    16 => raw as i16 as i64,
                    32 => raw as i32 as i64,
                    _ => raw as i64,
                };
                let mut digits = v.unsigned_abs().to_string();
                if let Some(p) = prec {
                    if p == 0 && v == 0 {
                        digits.clear();
                    }
                    while digits.len() < p {
                        digits.insert(0, '0');
                    }
                }
                let sign = sign_of(v < 0);
                let body = format!("{sign}{digits}");
                let z = (zero && !left && prec.is_none()).then_some(sign.len());
                pad(&mut out, body.as_bytes(), width, left, z);
            }
            b'u' | b'x' | b'X' | b'o' | b'p' => {
                let raw = va.int(s)?;
                let (conv, prec, alt) = if conv == b'p' { (b'x', Some(prec.unwrap_or(0).max(16)), true) } else { (conv, prec, alt) };
                let v = if conv == b'p' || bits >= 64 { raw } else if bits == 8 { raw as u8 as u64 } else if bits == 16 { raw as u16 as u64 } else { raw as u32 as u64 };
                let mut digits = match conv {
                    b'u' => v.to_string(),
                    b'x' => format!("{v:x}"),
                    b'X' => format!("{v:X}"),
                    _ => format!("{v:o}"),
                };
                if let Some(p) = prec {
                    if p == 0 && v == 0 {
                        digits.clear();
                    }
                    while digits.len() < p {
                        digits.insert(0, '0');
                    }
                }
                let prefix = match conv {
                    b'x' if alt && v != 0 => "0x",
                    b'X' if alt && v != 0 => "0X",
                    b'o' if alt && !digits.starts_with('0') => "0",
                    _ => "",
                };
                let body = format!("{prefix}{digits}");
                let z = (zero && !left && prec.is_none()).then_some(prefix.len());
                pad(&mut out, body.as_bytes(), width, left, z);
            }
            b'c' => {
                let c = va.int(s)? as u8;
                pad(&mut out, &[c], width, left, None);
            }
            b's' => {
                let p = va.int(s)?;
                let mut text = if p == 0 { b"(null)".to_vec() } else { cstr(s, p)? };
                if let Some(pr) = prec {
                    text.truncate(pr);
                }
                pad(&mut out, &text, width, left, None);
            }
            b'f' | b'F' | b'e' | b'E' | b'g' | b'G' => {
                if bits == 128 {
                    return Err(HostResult::Unavailable("printf of long double is not implemented".into()));
                }
                let v = va.double(s)?;
                let neg = v.is_sign_negative() && !v.is_nan();
                let sign = sign_of(neg);
                let body = format!("{sign}{}", fmt_float(v, conv, prec, alt));
                let z = (zero && !left && v.is_finite()).then_some(sign.len());
                pad(&mut out, body.as_bytes(), width, left, z);
            }
            b'n' => {
                let p = va.int(s)?;
                let n = out.len() as u64;
                let bytes = match bits {
                    8 => vec![n as u8],
                    16 => (n as u16).to_le_bytes().to_vec(),
                    32 => (n as u32).to_le_bytes().to_vec(),
                    _ => n.to_le_bytes().to_vec(),
                };
                write_bytes(s, p, &bytes)?;
            }
            other => {
                return Err(HostResult::Unavailable(format!(
                    "printf conversion %{} is not implemented",
                    other as char
                )))
            }
        }
    }
    Ok(out)
}

/// Write `text` into `buf` of `cap` bytes, C style; returns the full length.
fn emit(s: &mut State, buf: u64, cap: Option<u64>, text: &[u8]) -> HostResult {
    let n = text.len() as u64;
    match cap {
        None => {
            let mut b = text.to_vec();
            b.push(0);
            t!(write_bytes(s, buf, &b));
        }
        Some(0) => {}
        Some(cap) => {
            let k = n.min(cap - 1) as usize;
            let mut b = text[..k].to_vec();
            b.push(0);
            t!(write_bytes(s, buf, &b));
        }
    }
    HostResult::Returned(n as u32 as u64)
}

fn sprintf(s: &mut State) -> HostResult {
    let (buf, fmt) = (s.x[0], t!(cstr(s, s.x[1])));
    let mut va = Va::Regs { gr: 2, vr: 0, stack: s.sp };
    let text = t!(format(s, &fmt, &mut va));
    emit(s, buf, None, &text)
}
fn snprintf(s: &mut State) -> HostResult {
    let (buf, cap, fmt) = (s.x[0], s.x[1], t!(cstr(s, s.x[2])));
    let mut va = Va::Regs { gr: 3, vr: 0, stack: s.sp };
    let text = t!(format(s, &fmt, &mut va));
    emit(s, buf, Some(cap), &text)
}
fn vsnprintf(s: &mut State) -> HostResult {
    let (buf, cap, fmt, ap) = (s.x[0], s.x[1], t!(cstr(s, s.x[2])), s.x[3]);
    // The callee works on a copy of the va_list, as va_copy would.
    let copy = s.host.named.get("va_scratch").copied();
    let scratch = match copy {
        Some(a) => a,
        None => {
            let a = t!(s.host.allocate(32, 16).map_err(HostResult::Unavailable));
            s.host.named.insert("va_scratch".into(), a);
            a
        }
    };
    let list = t!(read_bytes(s, ap, 32));
    t!(write_bytes(s, scratch, &list));
    let mut va = Va::List { at: scratch };
    let text = t!(format(s, &fmt, &mut va));
    emit(s, buf, Some(cap), &text)
}

// ---- sscanf -------------------------------------------------------------------------------------

fn sscanf(s: &mut State) -> HostResult {
    let input = t!(cstr(s, s.x[0]));
    let fmt = t!(cstr(s, s.x[1]));
    let mut va = Va::Regs { gr: 2, vr: 0, stack: s.sp };
    let (mut i, mut k, mut assigned) = (0usize, 0usize, 0u64);
    let mut matched_any = false;
    while k < fmt.len() {
        let f = fmt[k];
        if is_space(f) {
            while i < input.len() && is_space(input[i]) {
                i += 1;
            }
            k += 1;
            continue;
        }
        if f != b'%' || fmt.get(k + 1) == Some(&b'%') {
            let want = if f == b'%' { k += 1; b'%' } else { f };
            if input.get(i) != Some(&want) {
                break;
            }
            i += 1;
            k += 1;
            continue;
        }
        k += 1;
        let suppress = fmt.get(k) == Some(&b'*');
        if suppress {
            k += 1;
        }
        let mut width = 0usize;
        while k < fmt.len() && fmt[k].is_ascii_digit() {
            width = width * 10 + (fmt[k] - b'0') as usize;
            k += 1;
        }
        let mut bits = 32;
        loop {
            match fmt.get(k) {
                Some(b'h') => bits = if bits == 16 { 8 } else { 16 },
                Some(b'l') | Some(b'j') | Some(b'z') | Some(b't') => bits = if bits == 64 { 64 } else { 64 },
                Some(b'L') => bits = 128,
                _ => break,
            }
            k += 1;
        }
        let Some(&conv) = fmt.get(k) else { break };
        k += 1;
        if !matches!(conv, b'c' | b'[' | b'n') {
            while i < input.len() && is_space(input[i]) {
                i += 1;
            }
        }
        let field_end = |i: usize| if width == 0 { input.len() } else { (i + width).min(input.len()) };
        let end = field_end(i);
        match conv {
            b'd' | b'i' | b'u' | b'x' | b'X' | b'o' => {
                let base = match conv {
                    b'd' | b'u' => 10,
                    b'x' | b'X' => 16,
                    b'o' => 8,
                    _ => 0,
                };
                let (neg, mag, _, used) = parse_int(&input[i..end], base);
                if used == 0 {
                    break;
                }
                i += used;
                matched_any = true;
                if !suppress {
                    let v = if neg { (mag as u64).wrapping_neg() } else { mag as u64 };
                    let p = t!(va.int(s));
                    let bytes = match bits {
                        8 => vec![v as u8],
                        16 => (v as u16).to_le_bytes().to_vec(),
                        32 => (v as u32).to_le_bytes().to_vec(),
                        _ => v.to_le_bytes().to_vec(),
                    };
                    t!(write_bytes(s, p, &bytes));
                    assigned += 1;
                }
            }
            b'f' | b'e' | b'g' | b'E' | b'G' | b'a' => {
                let Some((v, used)) = parse_float(&input[i..end]) else {
                    return HostResult::Unavailable("sscanf of a hexadecimal float is not implemented".into());
                };
                if used == 0 {
                    break;
                }
                i += used;
                matched_any = true;
                if !suppress {
                    let p = t!(va.int(s));
                    let bytes = if bits == 64 { v.to_le_bytes().to_vec() } else { (v as f32).to_le_bytes().to_vec() };
                    t!(write_bytes(s, p, &bytes));
                    assigned += 1;
                }
            }
            b's' => {
                let start = i;
                while i < end && !is_space(input[i]) {
                    i += 1;
                }
                if i == start {
                    break;
                }
                matched_any = true;
                if !suppress {
                    let p = t!(va.int(s));
                    let mut b = input[start..i].to_vec();
                    b.push(0);
                    t!(write_bytes(s, p, &b));
                    assigned += 1;
                }
            }
            b'c' => {
                let n = if width == 0 { 1 } else { width };
                if i + n > input.len() {
                    break;
                }
                matched_any = true;
                if !suppress {
                    let p = t!(va.int(s));
                    t!(write_bytes(s, p, &input[i..i + n]));
                    assigned += 1;
                }
                i += n;
            }
            b'[' => {
                let mut negate = false;
                if fmt.get(k) == Some(&b'^') {
                    negate = true;
                    k += 1;
                }
                let mut set = Vec::new();
                if fmt.get(k) == Some(&b']') {
                    set.push(b']');
                    k += 1;
                }
                while k < fmt.len() && fmt[k] != b']' {
                    if fmt[k] == b'-' && !set.is_empty() && fmt.get(k + 1).is_some_and(|&c| c != b']') {
                        let (a, z) = (*set.last().unwrap(), fmt[k + 1]);
                        set.extend(a..=z);
                        k += 2;
                    } else {
                        set.push(fmt[k]);
                        k += 1;
                    }
                }
                k += 1;
                let start = i;
                while i < end && (set.contains(&input[i]) != negate) {
                    i += 1;
                }
                if i == start {
                    break;
                }
                matched_any = true;
                if !suppress {
                    let p = t!(va.int(s));
                    let mut b = input[start..i].to_vec();
                    b.push(0);
                    t!(write_bytes(s, p, &b));
                    assigned += 1;
                }
            }
            b'n' => {
                if !suppress {
                    let p = t!(va.int(s));
                    t!(write_bytes(s, p, &(i as u32).to_le_bytes()));
                }
            }
            other => {
                return HostResult::Unavailable(format!("sscanf conversion %{} is not implemented", other as char))
            }
        }
    }
    if !matched_any && assigned == 0 && i >= input.len() {
        return HostResult::Returned(u32::MAX as u64); // EOF
    }
    HostResult::Returned(assigned)
}

// ---- stdio over nn::fs mounts --------------------------------------------------------------------

/// `FILE*` values are host handles; the position is kept per handle.
fn fopen(s: &mut State) -> HostResult {
    let path = String::from_utf8_lossy(&t!(cstr(s, s.x[0]))).into_owned();
    let mode = String::from_utf8_lossy(&t!(cstr(s, s.x[1]))).into_owned();
    if mode.contains(['w', 'a', '+']) {
        return HostResult::Unavailable(format!("fopen(\"{path}\", \"{mode}\"): writing through stdio is not implemented"));
    }
    let (host, _) = t!(crate::fs::resolve(s, &path));
    if !host.is_file() {
        let _ = set_errno(s, 2);
        return HostResult::Returned(0);
    }
    s.host.next_handle += 1;
    let h = 0xF1_0000_0000 + s.host.next_handle;
    s.host.files.insert(h, (host, 1));
    s.host.named.insert(format!("filepos:{h:x}"), 0);
    HostResult::Returned(h)
}

fn file_of(s: &State, h: u64) -> Result<(std::path::PathBuf, u64), HostResult> {
    let (path, _) = s.host.files.get(&h).cloned().ok_or_else(|| HostResult::Unavailable(format!("FILE* 0x{h:x} is not open")))?;
    let pos = s.host.named.get(&format!("filepos:{h:x}")).copied().unwrap_or(0);
    Ok((path, pos))
}

/// `fread(void* buf, size_t size, size_t n, FILE*)`.
fn fread(s: &mut State) -> HostResult {
    use std::io::{Read, Seek, SeekFrom};
    let (buf, size, n, h) = (s.x[0], s.x[1], s.x[2], s.x[3]);
    let (path, pos) = t!(file_of(s, h));
    let want = size.saturating_mul(n);
    let mut bytes = Vec::new();
    let r = std::fs::File::open(&path).and_then(|mut f| {
        f.seek(SeekFrom::Start(pos))?;
        f.take(want).read_to_end(&mut bytes)
    });
    if let Err(e) = r {
        return HostResult::Unavailable(format!("fread {}: {e}", path.display()));
    }
    let whole = if size == 0 { 0 } else { bytes.len() as u64 / size };
    bytes.truncate((whole * size) as usize);
    t!(write_bytes(s, buf, &bytes));
    s.host.named.insert(format!("filepos:{h:x}"), pos + bytes.len() as u64);
    HostResult::Returned(whole)
}

/// `fseek(FILE*, long, int whence)`.
fn fseek(s: &mut State) -> HostResult {
    let (h, off, whence) = (s.x[0], s.x[1] as i64, s.x[2] as u32);
    let (path, pos) = t!(file_of(s, h));
    let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) as i64;
    let base = match whence {
        0 => 0,
        1 => pos as i64,
        2 => len,
        _ => return HostResult::Returned(u32::MAX as u64),
    };
    let np = base + off;
    if np < 0 {
        let _ = set_errno(s, EINVAL);
        return HostResult::Returned(u32::MAX as u64);
    }
    s.host.named.insert(format!("filepos:{h:x}"), np as u64);
    HostResult::Returned(0)
}

fn ftell(s: &mut State) -> HostResult {
    let (_, pos) = t!(file_of(s, s.x[0]));
    HostResult::Returned(pos)
}

fn fclose(s: &mut State) -> HostResult {
    let h = s.x[0];
    s.host.files.remove(&h);
    s.host.named.remove(&format!("filepos:{h:x}"));
    HostResult::Returned(0)
}

// ---- pthreads -------------------------------------------------------------------------------------

fn pthread_mutex_lock(s: &mut State) -> HostResult {
    crate::threads::mutex_lock(s)
}
fn pthread_mutex_unlock(s: &mut State) -> HostResult {
    crate::threads::mutex_unlock(s)
}
fn pthread_cond_broadcast(s: &mut State) -> HostResult {
    crate::threads::cv_broadcast(s)
}

/// `pthread_cond_clockwait(cond, mutex, clockid_t, const timespec* abstime)`.
/// CLOCK_MONOTONIC (1) is the guest tick clock; CLOCK_REALTIME (0) is the host
/// wall clock. Returns 0 or ETIMEDOUT with the mutex held either way.
fn pthread_cond_clockwait(s: &mut State) -> HostResult {
    let (clock, ts) = (s.x[2] as u32, s.x[3]);
    let secs = t!(r64(s, ts));
    let nsec = t!(r64(s, ts + 8));
    let abs_ns = (secs as u128) * 1_000_000_000 + nsec as u128;
    let deadline = match clock {
        1 => ((abs_ns * crate::threads::TICK_FREQ as u128) / 1_000_000_000) as u64,
        0 => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let rel = abs_ns.saturating_sub(now);
            crate::threads::now_ticks() + ((rel * crate::threads::TICK_FREQ as u128) / 1_000_000_000) as u64
        }
        other => return HostResult::Unavailable(format!("pthread_cond_clockwait on clock {other}")),
    };
    t!(crate::threads::cv_enter(s));
    s.x[3] = deadline;
    clockwait_resume(s)
}

fn clockwait_resume(s: &mut State) -> HostResult {
    let (cv, ticket, deadline) = (s.x[0], s.x[2] as u32, s.x[3]);
    let (_, released) = t!(crate::threads::cv_fields(s, cv));
    if (ticket.wrapping_sub(released) as i32) < 0 {
        s.x[6] = 0;
        return crate::threads::cv_relock(s);
    }
    if crate::threads::now_ticks() >= deadline {
        s.x[6] = ETIMEDOUT;
        return crate::threads::cv_relock(s);
    }
    HostResult::Block { resume: clockwait_resume, timed: true }
}

/// `std::thread::detach()`: the thread runs on by itself. A `std::thread`
/// is one `pthread_t`, and threads made by the host's `pthread_create` need
/// no joining, so detaching clears the handle. (The SDK's own detach would
/// look the thread up in its internal list, which host threads are not in.)
fn std_thread_detach(s: &mut State) -> HostResult {
    let this = s.x[0];
    if t!(r64(s, this)) == 0 {
        return HostResult::Unavailable("std::thread::detach on a thread that is not joinable".into());
    }
    t!(write_bytes(s, this, &0u64.to_le_bytes()));
    HostResult::Returned(0)
}

/// `std::thread::~thread()`: destroying a joinable thread calls std::terminate.
fn std_thread_destroy(s: &mut State) -> HostResult {
    let this = s.x[0];
    if t!(r64(s, this)) != 0 {
        return HostResult::Unavailable("a joinable std::thread was destroyed, which calls std::terminate".into());
    }
    HostResult::Returned(0)
}

fn pthread_key_create(s: &mut State) -> HostResult {
    crate::services::alloc_tls_slot(s)
}
fn pthread_getspecific(s: &mut State) -> HostResult {
    let k = s.x[0] as usize;
    HostResult::Returned(s.host.tls.get(k).copied().unwrap_or(0))
}
fn pthread_setspecific(s: &mut State) -> HostResult {
    let (k, v) = (s.x[0] as usize, s.x[1]);
    match s.host.tls.get_mut(k) {
        Some(cell) => {
            *cell = v;
            HostResult::Returned(0)
        }
        None => HostResult::Returned(EINVAL as u64),
    }
}

/// `pthread_create(pthread_t*, attr, start, arg)`: a thread object and a
/// 1 MiB stack from the host arena, started immediately.
fn pthread_create(s: &mut State) -> HostResult {
    let (out, start, arg) = (s.x[0], s.x[2], s.x[3]);
    let object = t!(s.host.allocate(0x200, 0x100).map_err(HostResult::Unavailable));
    let stack = t!(s.host.allocate(1 << 20, 0x1000).map_err(HostResult::Unavailable));
    s.host.threads.insert(
        object,
        ThreadInfo { entry: start, arg, stack_top: stack + (1 << 20), priority: 44, started: true, ..Default::default() },
    );
    s.host.thread_starts += 1;
    t!(write_bytes(s, out, &object.to_le_bytes()));
    HostResult::Returned(0)
}

// ---- miscellany ---------------------------------------------------------------------------------------

/// musl's `rand`: a 64-bit LCG, `seed = 6364136223846793005*seed + 1`, result
/// `seed >> 33`, starting from seed 0 because `srand` is never called.
fn rand(s: &mut State) -> HostResult {
    let seed = s.host.named.get("rand_seed").copied().unwrap_or(0);
    let next = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    s.host.named.insert("rand_seed".into(), next);
    HostResult::Returned(next >> 33)
}

fn assert_fail(s: &mut State) -> HostResult {
    let text = |a: u64| c_string(s, a, 512).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    HostResult::Unavailable(format!(
        "the game failed an assertion: `{}` at {}:{} in {}",
        text(s.x[0]),
        text(s.x[1]),
        s.x[2] as u32,
        text(s.x[3])
    ))
}

fn setlocale(s: &mut State) -> HostResult {
    let wanted = if s.x[1] == 0 { Vec::new() } else { t!(cstr(s, s.x[1])) };
    if !(wanted.is_empty() || wanted == b"C" || wanted == b"POSIX") {
        return HostResult::Returned(0);
    }
    let at = match s.host.named.get("locale_c").copied() {
        Some(a) => a,
        None => {
            let a = t!(s.host.allocate(8, 8).map_err(HostResult::Unavailable));
            t!(write_bytes(s, a, b"C\0"));
            s.host.named.insert("locale_c".into(), a);
            a
        }
    };
    HostResult::Returned(at)
}

/// `fenv_t` on AArch64 is { fpcr, fpsr }; the host models round-to-nearest only.
fn fegetenv(s: &mut State) -> HostResult {
    t!(write_bytes(s, s.x[0], &[0u8; 8]));
    HostResult::Returned(0)
}
fn fesetenv(s: &mut State) -> HostResult {
    let p = s.x[0];
    if p != u64::MAX {
        let fpcr = t!(r32(s, p));
        if fpcr & (3 << 22) != 0 {
            return HostResult::Unavailable(format!("fesetenv selects rounding mode {}, and only round-to-nearest is modelled", (fpcr >> 22) & 3));
        }
    }
    HostResult::Returned(0)
}
/// The SDK's fesetround takes FPCR bits: the rounding mode in 23:22 and,
/// beside it, the AHP/DN/FZ mode bits (26:24). The game asks for
/// flush-to-zero (0x1000000) around some maths; flushing denormals changes
/// results only in the last bits of values below 2^-126, so round-to-nearest
/// with those mode bits is accepted. Any other rounding mode is not modelled.
fn fesetround(s: &mut State) -> HostResult {
    let mode = s.x[0] as u32;
    if mode & !0x0700_0000 != 0 {
        return HostResult::Unavailable(format!("fesetround(0x{mode:x}): only round-to-nearest is modelled"));
    }
    HostResult::Returned(0)
}

/// zstd's tracing hooks: with no tracer installed, begin returns a zero
/// context and end does nothing, which is exactly what zstd's weak defaults do.
fn zstd_trace(_s: &mut State) -> HostResult {
    HostResult::Returned(0)
}

pub fn register(s: &mut HostServices) {
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("__errno_location", errno_location, "a per-thread errno word"),
        ("strcat", strcat, "standard C"),
        ("strncat", strncat, "standard C"),
        ("toupper", toupper, "C locale"),
        ("tolower", tolower, "C locale"),
        ("islower", islower, "C locale"),
        ("isspace", isspace, "C locale"),
        ("abs", abs, "standard C, abs(INT_MIN) wraps as musl does"),
        ("strcasecmp", strcasecmp, "C locale"),
        ("wmemchr", wmemchr, "32-bit wchar_t"),
        ("strtol", strtol, "C semantics: base prefixes, saturation with ERANGE"),
        ("strtoul", strtoul, "C semantics: negation wraps, saturation with ERANGE"),
        ("strtoull", strtoul, "C semantics: negation wraps, saturation with ERANGE"),
        ("atoi", atoi, "strtol without error reporting"),
        ("strtod", strtod, "decimal, inf and nan; correctly rounded"),
        ("strtof", strtof, "decimal, inf and nan; ERANGE on overflow"),
        ("atof", atof, "strtod without an end pointer"),
        ("sprintf", sprintf, "musl printf semantics, AAPCS64 varargs"),
        ("snprintf", snprintf, "musl printf semantics, AAPCS64 varargs"),
        ("vsnprintf", vsnprintf, "musl printf semantics, AAPCS64 va_list"),
        ("sscanf", sscanf, "integers, floats, strings, sets and %n"),
        ("fopen", fopen, "read-only stdio over the nn::fs mounts"),
        ("fread", fread, "reads from the host file at the stream position"),
        ("fseek", fseek, "SEEK_SET, SEEK_CUR, SEEK_END"),
        ("ftell", ftell, "the stream position"),
        ("fclose", fclose, "releases the stream"),
        ("pthread_mutex_lock", pthread_mutex_lock, "the scheduler's blocking mutex"),
        ("pthread_mutex_unlock", pthread_mutex_unlock, "the scheduler's blocking mutex"),
        ("pthread_cond_broadcast", pthread_cond_broadcast, "the scheduler's condition variable"),
        ("pthread_cond_clockwait", pthread_cond_clockwait, "timed wait on the monotonic or realtime clock"),
        ("pthread_key_create", pthread_key_create, "a thread-local slot"),
        ("pthread_getspecific", pthread_getspecific, "the calling thread's slot value"),
        ("pthread_setspecific", pthread_setspecific, "the calling thread's slot value"),
        ("pthread_create", pthread_create, "a guest thread with a 1 MiB stack from the host arena"),
        ("_ZNSt3__16thread6detachEv", std_thread_detach, "host threads need no joining; the handle is cleared"),
        ("_ZNSt3__16threadD1Ev", std_thread_destroy, "a joinable thread terminates, as std::thread does"),
        ("rand", rand, "musl's 64-bit LCG"),
        ("__assert_fail", assert_fail, "a failed assertion stops the run with its text"),
        ("setlocale", setlocale, "only the C locale exists"),
        ("fegetenv", fegetenv, "default environment: round to nearest, no traps"),
        ("fesetenv", fesetenv, "accepted for round-to-nearest"),
        ("fesetround", fesetround, "accepted for round-to-nearest"),
        ("ZSTD_trace_compress_begin", zstd_trace, "no tracer installed"),
        ("ZSTD_trace_compress_end", zstd_trace, "no tracer installed"),
        ("ZSTD_trace_decompress_begin", zstd_trace, "no tracer installed"),
        ("ZSTD_trace_decompress_end", zstd_trace, "no tracer installed"),
    ];
    for (name, handler, claim) in table {
        s.register(name, *handler, claim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(s: &mut State, at: u64, text: &str) {
        s.mem[at as usize..at as usize + text.len()].copy_from_slice(text.as_bytes());
        s.mem[at as usize + text.len()] = 0;
    }
    fn get(s: &State, at: u64) -> String {
        String::from_utf8(c_string(s, at, 4096).unwrap()).unwrap()
    }

    #[test]
    fn printf_matches_musl() {
        let mut s = State::default();
        s.host.alloc_next = 0x80000;
        s.host.alloc_end = 0x90000;
        put(&mut s, 0x100, "%d|%5s|%-4x|%08.3f|%e|%g|%g|%p|%%|%c|%+i|%.0f");
        s.x[0] = 0x1000;
        s.x[1] = 0x100;
        s.x[2] = (-42i64) as u64;
        s.x[3] = 0x200;
        s.x[4] = 255;
        s.x[5] = 0xBEEF;
        s.x[6] = b'Z' as u64;
        s.x[7] = 7;
        put(&mut s, 0x200, "ab");
        s.v[0][0] = 3.14159f64.to_bits();
        s.v[1][0] = 1234.5f64.to_bits();
        s.v[2][0] = 0.0001f64.to_bits();
        s.v[3][0] = 1e20f64.to_bits();
        s.v[4][0] = 2.5f64.to_bits();
        let r = sprintf(&mut s);
        assert_eq!(
            get(&s, 0x1000),
            "-42|   ab|ff  |0003.142|1.234500e+03|0.0001|1e+20|0x000000000000beef|%|Z|+7|2"
        );
        assert_eq!(r, HostResult::Returned(77));
    }

    #[test]
    fn snprintf_truncates_and_reports_the_full_length() {
        let mut s = State::default();
        put(&mut s, 0x100, "hello %s");
        put(&mut s, 0x200, "world");
        s.x[0] = 0x1000;
        s.x[1] = 6;
        s.x[2] = 0x100;
        s.x[3] = 0x200;
        assert_eq!(snprintf(&mut s), HostResult::Returned(11));
        assert_eq!(get(&s, 0x1000), "hello");
    }

    #[test]
    fn strtol_handles_bases_and_saturates() {
        let mut s = State::default();
        s.host.alloc_next = 0x80000;
        s.host.alloc_end = 0x90000;
        s.host.thread_object = 1;
        for (text, base, want, used) in [
            ("  -0x1F rest", 0, -31i64, 7usize),
            ("0777", 0, 511, 4),
            ("99999999999999999999", 10, i64::MAX, 20),
            ("zz", 36, 1295, 2),
            ("x", 10, 0, 0),
        ] {
            put(&mut s, 0x100, text);
            s.x[0] = 0x100;
            s.x[1] = 0x300;
            s.x[2] = base;
            assert_eq!(strtol(&mut s), HostResult::Returned(want as u64), "{text}");
            let end = u64::from_le_bytes(s.mem[0x300..0x308].try_into().unwrap());
            assert_eq!(end, 0x100 + used as u64, "{text}");
        }
    }

    #[test]
    fn sscanf_reads_mixed_fields() {
        let mut s = State::default();
        put(&mut s, 0x100, "v2 12 3.5 name");
        put(&mut s, 0x200, "v%d %x %f %s");
        s.x[0] = 0x100;
        s.x[1] = 0x200;
        s.x[2] = 0x400;
        s.x[3] = 0x404;
        s.x[4] = 0x408;
        s.x[5] = 0x410;
        assert_eq!(sscanf(&mut s), HostResult::Returned(4));
        assert_eq!(u32::from_le_bytes(s.mem[0x400..0x404].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(s.mem[0x404..0x408].try_into().unwrap()), 0x12);
        assert_eq!(f32::from_le_bytes(s.mem[0x408..0x40C].try_into().unwrap()), 3.5);
        assert_eq!(get(&s, 0x410), "name");
    }

    #[test]
    fn rand_is_musls_sequence() {
        let mut s = State::default();
        // musl: seed 0 -> 1 -> 6364136223846793006 ...; outputs seed >> 33.
        assert_eq!(rand(&mut s), HostResult::Returned(0));
        assert_eq!(rand(&mut s), HostResult::Returned(6_364_136_223_846_793_006u64 >> 33));
    }
}
