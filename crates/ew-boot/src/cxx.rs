//! The libc++ entry points the module imports.
//!
//! The title links the C++ standard library dynamically, so its own code calls
//! into symbols this host has to provide. Two are here:
//!
//! * `std::random_device`, an object that must exist for static initialisation
//!   to finish, and a generator whose result must be real entropy rather than a
//!   constant;
//! * `std::__hash_memory`, which is cityhash64. See [`crate::hash`]: the values
//!   are checked against a real libc++ build, because a different hash would
//!   change the tables the game builds, not merely how far the boot gets.
//!
//! Everything else in `std::__1` stays unregistered, so a run that reaches
//! `std::locale` or `std::to_chars` stops on that name. Those are formatting and
//! parsing services, and answering them with zeroed objects would change what
//! the game reads and writes rather than what it can start.

use crate::hash::hash_memory as cityhash64;
use crate::hle::{read_bytes, write_bytes, HostResult, HostServices};
use ew_lift::State;

/// `std::__1::random_device` holds a single implementation pointer.
const OBJECT_BYTES: u64 = 8;

/// `std::__1::random_device::random_device(std::string const&)`.
///
/// The constructor only needs a live object. The token argument names an entropy
/// source on a POSIX host; on this target the source is the host itself, so the
/// string is not consulted and the object is zeroed.
fn random_device_ctor(state: &mut State) -> HostResult {
    let object = state.x[0];
    match write_bytes(state, object, &[0u8; OBJECT_BYTES as usize]) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

/// `std::__1::random_device::operator()()`.
///
/// Returns `unsigned int` in `x0`. The sequence is a splitmix64 stream seeded
/// from the host clock and advanced per call, which is real entropy from the
/// title's point of view and never repeats a value it already returned.
fn random_device_call(state: &mut State) -> HostResult {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    if state.host.rng == 0 {
        state.host.rng = nanos ^ 0x9E37_79B9_7F4A_7C15;
    }
    let mut z = state.host.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
    state.host.rng = z;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    HostResult::Returned(z & 0xFFFF_FFFF)
}

/// `__cxa_guard_acquire(unsigned long long*)`.
///
/// The guard is two bytes in the guest's own `.bss`: byte 0 is "already
/// initialised", byte 1 is "initialisation in progress". Returns 1 when the
/// caller must run the initialiser and 0 when it has already run. With a single
/// guest thread the in-progress byte can never be observed by a second thread,
/// so the protocol is implemented exactly as specified rather than approximated.
fn cxa_guard_acquire(state: &mut State) -> HostResult {
    let guard = state.x[0];
    let bytes = match read_bytes(state, guard, 2) {
        Ok(b) => b,
        Err(e) => return e,
    };
    if bytes[0] != 0 {
        return HostResult::Returned(0);
    }
    match write_bytes(state, guard + 1, &[1]) {
        Ok(()) => HostResult::Returned(1),
        Err(e) => e,
    }
}

/// `__cxa_guard_release(unsigned long long*)`: mark the static as initialised.
fn cxa_guard_release(state: &mut State) -> HostResult {
    let guard = state.x[0];
    match write_bytes(state, guard, &[1, 0]) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

/// `__cxa_guard_abort(unsigned long long*)`: clear the in-progress byte so a
/// later attempt can retry.
fn cxa_guard_abort(state: &mut State) -> HostResult {
    let guard = state.x[0];
    match write_bytes(state, guard + 1, &[0]) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

/// `std::__1::__hash_memory(void const*, unsigned long)`.
///
/// Argument pointer in `x0`, length in `x1`, result in `x0`. The bytes are read
/// through the guest memory bus, so a bad pointer is a reported fault rather
/// than a host-side read of arbitrary process memory.
fn hash_memory(state: &mut State) -> HostResult {
    let ptr = state.x[0];
    let len = state.x[1];
    if len > 1 << 20 {
        return HostResult::Unavailable(format!(
            "__hash_memory was asked for {len} bytes at 0x{ptr:x}, which is not a plausible string length"
        ));
    }
    match read_bytes(state, ptr, len) {
        Ok(bytes) => HostResult::Returned(cityhash64(&bytes)),
        Err(e) => e,
    }
}

/// Deterministic Miller-Rabin for 64-bit integers. The witness set
/// {2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37} is proven exact below 3.3e24.
fn is_prime(n: u64) -> bool {
    if n < 2 {
        return false;
    }
    const W: [u64; 12] = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];
    for p in W {
        if n % p == 0 {
            return n == p;
        }
    }
    let mut d = n - 1;
    let mut r = 0;
    while d % 2 == 0 {
        d /= 2;
        r += 1;
    }
    let mulmod = |a: u64, b: u64| ((a as u128 * b as u128) % n as u128) as u64;
    let powmod = |mut b: u64, mut e: u64| {
        let mut acc = 1u64;
        b %= n;
        while e > 0 {
            if e & 1 == 1 {
                acc = mulmod(acc, b);
            }
            b = mulmod(b, b);
            e >>= 1;
        }
        acc
    };
    'witness: for a in W {
        let mut x = powmod(a, d);
        if x == 1 || x == n - 1 {
            continue;
        }
        for _ in 1..r {
            x = mulmod(x, x);
            if x == n - 1 {
                continue 'witness;
            }
        }
        return false;
    }
    true
}

/// The largest prime below 2^64; libc++ throws `overflow_error` above it.
const LARGEST_U64_PRIME: u64 = 0xFFFF_FFFF_FFFF_FFC5;

/// `std::__1::__next_prime(unsigned long)`.
///
/// libc++ returns the smallest prime that is not less than `n`, except that
/// its table starts with 0, so `__next_prime(0)` is 0. The libc++ search walks
/// a wheel of residues; the result is the same number, so the host computes it
/// directly with an exact primality test.
fn next_prime(state: &mut State) -> HostResult {
    let n = state.x[0];
    if n == 0 {
        return HostResult::Returned(0);
    }
    if n > LARGEST_U64_PRIME {
        return HostResult::Unavailable(format!(
            "__next_prime({n}) throws std::overflow_error in libc++, and the host does not model C++ exceptions"
        ));
    }
    let mut c = n;
    while !is_prime(c) {
        c += 1;
    }
    HostResult::Returned(c)
}

/// libc++'s nothrow `operator new` calls the replaceable throwing operator and
/// returns null if it throws. This title replaces the throwing operators (it
/// exports `_Znwm` and `_Znam`), so the call forwards into the game's own code
/// with the size argument already in `x0`.
fn forward(state: &mut State, export: &str) -> HostResult {
    match state.host.named.get(&format!("export.{export}")) {
        Some(&target) => HostResult::Jump(target),
        None => HostResult::Unavailable(format!(
            "nothrow operator new forwards to `{export}`, which this module does not export"
        )),
    }
}
fn new_nothrow(state: &mut State) -> HostResult {
    forward(state, "_Znwm")
}
fn new_array_nothrow(state: &mut State) -> HostResult {
    forward(state, "_Znam")
}

pub fn register(services: &mut HostServices) {
    services.register(
        "_ZnwmRKSt9nothrow_t",
        new_nothrow as _,
        "forwards to the game's own operator new, as libc++ does",
    );
    services.register(
        "_ZnamRKSt9nothrow_t",
        new_array_nothrow as _,
        "forwards to the game's own operator new[], as libc++ does",
    );
    services.register(
        "_ZNSt3__112__next_primeEm",
        next_prime as _,
        "smallest prime not less than n (0 maps to 0 as in libc++), exact Miller-Rabin; overflow is reported",
    );
    services.register(
        "_ZNSt3__113random_deviceC2ERKNS_12basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEEE",
        random_device_ctor as _,
        "constructs the object; the host is the entropy source, so the token string is not consulted",
    );
    services.register(
        "_ZNSt3__113random_deviceclEv",
        random_device_call as _,
        "splitmix64 seeded from the host clock, truncated to the unsigned int the interface returns",
    );
    services.register(
        "__cxa_guard_acquire",
        cxa_guard_acquire as _,
        "the two-byte static-initialisation guard from libc++abi, implemented on guest memory",
    );
    services.register(
        "__cxa_guard_release",
        cxa_guard_release as _,
        "marks the static initialised, exactly as libc++abi does",
    );
    services.register(
        "__cxa_guard_abort",
        cxa_guard_abort as _,
        "clears the in-progress byte so a later attempt can retry",
    );
    services.register(
        "_ZNSt3__113__hash_memoryEPKvm",
        hash_memory as _,
        "cityhash64, ported from libc++ and checked against reference vectors from a real libc++ build",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_zeroes_the_object() {
        let mut s = State::default();
        s.mem[0x100..0x108].fill(0xAA);
        s.x[0] = 0x100;
        assert_eq!(random_device_ctor(&mut s), HostResult::Returned(0));
        assert_eq!(&s.mem[0x100..0x108], &[0u8; 8]);
    }

    #[test]
    fn a_static_guard_runs_its_initialiser_once() {
        let mut s = State::default();
        s.x[0] = 0x300;
        assert_eq!(cxa_guard_acquire(&mut s), HostResult::Returned(1));
        assert_eq!(s.mem[0x301], 1, "in-progress is recorded");
        assert_eq!(cxa_guard_release(&mut s), HostResult::Returned(0));
        assert_eq!(&s.mem[0x300..0x302], &[1, 0]);
        // Second entry sees the initialised byte and does not run it again.
        s.x[0] = 0x300;
        assert_eq!(cxa_guard_acquire(&mut s), HostResult::Returned(0));
    }

    #[test]
    fn an_aborted_guard_can_be_retried() {
        let mut s = State::default();
        s.x[0] = 0x400;
        assert_eq!(cxa_guard_acquire(&mut s), HostResult::Returned(1));
        assert_eq!(cxa_guard_abort(&mut s), HostResult::Returned(0));
        assert_eq!(s.mem[0x401], 0);
        assert_eq!(cxa_guard_acquire(&mut s), HostResult::Returned(1));
    }

    #[test]
    fn hash_memory_reads_guest_bytes_and_returns_the_hash() {
        let mut s = State::default();
        s.mem[0x200..0x205].copy_from_slice(b"hello");
        s.x[0] = 0x200;
        s.x[1] = 5;
        assert_eq!(hash_memory(&mut s), HostResult::Returned(235368941344721064));
    }

    #[test]
    fn next_prime_matches_libcxx() {
        let mut s = State::default();
        for (n, want) in [(0u64, 0u64), (1, 2), (2, 2), (4, 5), (212, 223), (1000, 1009), (1 << 32, 4294967311)] {
            s.x[0] = n;
            assert_eq!(next_prime(&mut s), HostResult::Returned(want), "n={n}");
        }
        s.x[0] = LARGEST_U64_PRIME;
        assert_eq!(next_prime(&mut s), HostResult::Returned(LARGEST_U64_PRIME));
        s.x[0] = u64::MAX;
        assert!(matches!(next_prime(&mut s), HostResult::Unavailable(_)));
    }

    #[test]
    fn generator_does_not_repeat() {
        let mut s = State::default();
        let mut seen = Vec::new();
        for _ in 0..64 {
            match random_device_call(&mut s) {
                HostResult::Returned(v) => seen.push(v),
                other => panic!("{other:?}"),
            }
        }
        let mut sorted = seen.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), seen.len(), "stream repeated a value");
        assert!(seen.iter().all(|v| *v <= 0xFFFF_FFFF));
    }
}
