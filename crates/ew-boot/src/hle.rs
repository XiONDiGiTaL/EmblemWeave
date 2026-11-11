//! Host implementations for imported functions.
//!
//! Every entry here is a claim that the host can provide the exact semantics of
//! that symbol, so the set is deliberately small: the C memory and string
//! routines, whose behaviour is fully specified. Anything else stays
//! unimplemented and stops the run with its name.
//!
//! Two Nintendo runtime initialisers are also modelled, and both are marked as
//! assumptions rather than facts, because their exact side effects live in a
//! module this project does not translate yet.

use ew_lift::State;
use std::collections::BTreeMap;

/// Result of a host call.
#[derive(Debug, Clone)]
pub enum HostResult {
    /// Returned `x0` to the caller.
    Returned(u64),
    /// Cannot be provided; the run stops with this reason.
    Unavailable(String),
    /// Continue at a guest address with the registers as the handler left
    /// them and the caller's return address intact: a tail call into guest
    /// code, which is how a host vtable forwards to the game's own functions.
    Jump(u64),
    /// The calling guest thread must wait. The scheduler parks it with its
    /// registers as the handler left them and calls `resume` (with those
    /// registers loaded) whenever it next considers the thread; `resume`
    /// returns the call's result or blocks again. `timed` marks a wait that
    /// the passage of time alone can end, which is how a deadlock is told
    /// apart from a sleep.
    Block { resume: Handler, timed: bool },
}

/// Results compare by value; two blocks compare by their `timed` flag only,
/// because a function pointer has no stable identity to compare.
impl PartialEq for HostResult {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (HostResult::Returned(a), HostResult::Returned(b)) => a == b,
            (HostResult::Unavailable(a), HostResult::Unavailable(b)) => a == b,
            (HostResult::Jump(a), HostResult::Jump(b)) => a == b,
            (HostResult::Block { timed: a, .. }, HostResult::Block { timed: b, .. }) => a == b,
            _ => false,
        }
    }
}

pub type Handler = fn(&mut State) -> HostResult;

/// The set of imported symbols the host can serve.
///
/// A name can be registered exactly, or as a prefix so that C++ overload sets
/// such as the RELRO protection functions are covered by one entry instead of
/// one entry per mangled signature.
pub struct HostServices {
    handlers: BTreeMap<String, (Handler, &'static str)>,
    prefixes: Vec<(String, Handler, &'static str)>,
}

impl HostServices {
    pub fn new() -> HostServices {
        HostServices { handlers: BTreeMap::new(), prefixes: Vec::new() }
    }

    /// The functions EmblemWeave actually implements today.
    pub fn defaults() -> HostServices {
        let mut s = HostServices::new();
        for (name, handler, claim) in [
            ("memcpy", memcpy as Handler, "standard C memcpy semantics"),
            ("memmove", memmove as Handler, "standard C memmove semantics"),
            ("memset", memset as Handler, "standard C memset semantics"),
            ("memcmp", memcmp as Handler, "standard C memcmp semantics"),
            ("bcmp", memcmp as Handler, "memcmp semantics, boolean result unused"),
            ("memchr", memchr as Handler, "standard C memchr semantics"),
            ("strlen", strlen as Handler, "standard C strlen semantics"),
            ("strnlen", strnlen as Handler, "standard C strnlen semantics"),
            ("strcmp", strcmp as Handler, "standard C strcmp semantics"),
            ("strncmp", strncmp as Handler, "standard C strncmp semantics"),
            ("strcpy", strcpy as Handler, "standard C strcpy semantics"),
            ("strncpy", strncpy as Handler, "standard C strncpy semantics"),
            ("strchr", strchr as Handler, "standard C strchr semantics"),
            ("strrchr", strrchr as Handler, "standard C strrchr semantics"),
            ("strstr", strstr as Handler, "standard C strstr semantics"),
            (
                "__nnmusl_init_dso",
                init_dso as Handler,
                "assumption: registers TLS and atexit for a shared object, which this module is not",
            ),
            (
                "__nnmusl_fini_dso",
                init_dso as Handler,
                "assumption: paired teardown of the initialiser above",
            ),
        ] {
            s.register(name, handler, claim);
        }

        // nn::ro memory protection. The flat guest address space has no page
        // permissions, so protecting RELRO cannot fail and has nothing to do;
        // reporting success is the exact behaviour in this memory model, not a
        // shortcut around a missing feature.
        s.register_prefix(
            "_ZN2nn2ro12ProtectRelroE",
            protect_relro as Handler,
            "the flat guest address space has no page permissions, so protection succeeds trivially",
        );
        s.register_prefix(
            "_ZN2nn2ro14UnprotectRelroE",
            protect_relro as Handler,
            "paired with ProtectRelro in the flat guest address space",
        );

        // C runtime registration. The host never exits through the guest's
        // exit path, so registering a handler and returning success is the
        // complete behaviour available in this model.
        for name in ["atexit", "at_quick_exit", "__cxa_finalize"] {
            s.register(
                name,
                register_exit as Handler,
                "registers an exit handler; the host has no guest exit path to run it from yet",
            );
        }
        s.register_prefix(
            "__cxa_atexit",
            register_exit as Handler,
            "same contract as atexit for C++ static destructors",
        );

        // The single-threaded operating system services live in their own
        // module because every one of them has a stated concurrency contract.
        crate::services::register(&mut s);
        crate::math::register(&mut s);
        crate::cxx::register(&mut s);
        crate::alloc::register(&mut s);
        crate::pmr::register(&mut s);
        crate::fs::register(&mut s);
        crate::lmem::register(&mut s);
        // Multi-threaded nn::os supersedes the single-threaded handlers of the
        // same names, so it registers last.
        crate::threads::register(&mut s);
        crate::system::register(&mut s);
        crate::libc::register(&mut s);
        crate::gfx::register(&mut s);
        crate::nvn::register(&mut s);
        crate::nvn_mem::register(&mut s);
        crate::nvn_tex::register(&mut s);
        crate::nvn_win::register(&mut s);
        crate::nvn_cmd::register(&mut s);
        crate::nvn_state::register(&mut s);
        crate::nvn_prog::register(&mut s);
        crate::hid::register(&mut s);
        crate::kernel::register(&mut s);
        crate::audio::register(&mut s);
        crate::codec::register(&mut s);
        crate::prepo::register(&mut s);
        crate::movie::register(&mut s);
        s
    }

    pub fn register(&mut self, name: &str, handler: Handler, claim: &'static str) {
        self.handlers.insert(name.to_string(), (handler, claim));
    }

    /// Register a handler for every symbol starting with `prefix`. Lookup
    /// prefers the longest matching prefix, and re-registering the same prefix
    /// replaces the earlier handler.
    pub fn register_prefix(&mut self, prefix: &str, handler: Handler, claim: &'static str) {
        self.prefixes.retain(|(p, _, _)| p != prefix);
        self.prefixes.push((prefix.to_string(), handler, claim));
    }

    fn resolve(&self, name: &str) -> Option<(Handler, &'static str)> {
        if let Some((handler, claim)) = self.handlers.get(name) {
            return Some((*handler, *claim));
        }
        self.prefixes
            .iter()
            .filter(|(prefix, _, _)| name.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _, _)| prefix.len())
            .map(|(_, handler, claim)| (*handler, *claim))
    }


    pub fn get(&self, name: &str) -> Option<Handler> {
        self.resolve(name).map(|(h, _)| h)
    }

    pub fn claim(&self, name: &str) -> Option<&'static str> {
        self.resolve(name).map(|(_, c)| c)
    }

    pub fn names(&self) -> Vec<String> {
        self.handlers
            .keys()
            .cloned()
            .chain(self.prefixes.iter().map(|(p, _, _)| format!("{p}*")))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.handlers.len() + self.prefixes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

impl Default for HostServices {
    fn default() -> Self {
        HostServices::defaults()
    }
}

// Guest memory helpers. A fault is reported with the address, never masked.

/// The guest address of a host function created at run time, by name. The
/// same name always yields the same address. Calling it dispatches to the
/// service registered under `name`, or stops the run naming it.
pub(crate) fn dynamic_stub(state: &mut State, name: &str) -> Result<u64, HostResult> {
    if let Some(i) = state.host.dyn_stubs.iter().position(|n| n == name) {
        return Ok(state.host.dyn_base + i as u64 * 16);
    }
    let i = state.host.dyn_stubs.len() as u64;
    if state.host.dyn_base == 0 || i >= state.host.dyn_capacity {
        return Err(HostResult::Unavailable(format!(
            "no room for another run-time host function ({name}); {} already exist",
            state.host.dyn_stubs.len()
        )));
    }
    state.host.dyn_stubs.push(name.to_string());
    Ok(state.host.dyn_base + i * 16)
}

pub(crate) fn read_bytes(state: &State, addr: u64, len: u64) -> Result<Vec<u8>, HostResult> {
    let (Ok(start), Ok(end)) = (usize::try_from(addr), usize::try_from(addr.saturating_add(len))) else {
        return Err(HostResult::Unavailable(format!("host read of {len} bytes at 0x{addr:x} wraps")));
    };
    if end > state.mem.len() {
        return Err(HostResult::Unavailable(format!(
            "host read of {len} bytes at 0x{addr:x} is outside guest memory"
        )));
    }
    Ok(state.mem[start..end].to_vec())
}

pub(crate) fn write_bytes(state: &mut State, addr: u64, bytes: &[u8]) -> Result<(), HostResult> {
    let (Ok(start), Ok(end)) = (usize::try_from(addr), usize::try_from(addr.saturating_add(bytes.len() as u64))) else {
        return Err(HostResult::Unavailable(format!("host write at 0x{addr:x} wraps")));
    };
    if end > state.mem.len() {
        return Err(HostResult::Unavailable(format!(
            "host write of {} bytes at 0x{addr:x} is outside guest memory",
            bytes.len()
        )));
    }
    state.mem[start..end].copy_from_slice(bytes);
    Ok(())
}

fn memcpy(state: &mut State) -> HostResult {
    let (dst, src, n) = (state.x[0], state.x[1], state.x[2]);
    match read_bytes(state, src, n) {
        Ok(bytes) => match write_bytes(state, dst, &bytes) {
            Ok(()) => HostResult::Returned(dst),
            Err(e) => e,
        },
        Err(e) => e,
    }
}

fn memmove(state: &mut State) -> HostResult {
    memcpy(state)
}

fn memset(state: &mut State) -> HostResult {
    let (dst, value, n) = (state.x[0], state.x[1] as u8, state.x[2]);
    let bytes = vec![value; n as usize];
    match write_bytes(state, dst, &bytes) {
        Ok(()) => HostResult::Returned(dst),
        Err(e) => e,
    }
}

fn memcmp(state: &mut State) -> HostResult {
    let (a, b, n) = (state.x[0], state.x[1], state.x[2]);
    let (left, right) = match (read_bytes(state, a, n), read_bytes(state, b, n)) {
        (Ok(l), Ok(r)) => (l, r),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let order = match left.iter().zip(right.iter()).find(|(x, y)| x != y) {
        Some((x, y)) => x.cmp(y),
        None => std::cmp::Ordering::Equal,
    };
    let result = match order {
        std::cmp::Ordering::Less => -1i64,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    HostResult::Returned(result as u64)
}

fn memchr(state: &mut State) -> HostResult {
    let (addr, needle, n) = (state.x[0], state.x[1] as u8, state.x[2]);
    match read_bytes(state, addr, n) {
        Ok(bytes) => match bytes.iter().position(|b| *b == needle) {
            Some(i) => HostResult::Returned(addr + i as u64),
            None => HostResult::Returned(0),
        },
        Err(e) => e,
    }
}

/// Read a NUL-terminated string, with a hard cap so a bad pointer cannot walk
/// the whole address space.
pub(crate) fn c_string(state: &State, addr: u64, max: u64) -> Result<Vec<u8>, HostResult> {
    let mut out = Vec::new();
    for i in 0..max {
        let byte = read_bytes(state, addr + i, 1)?;
        if byte[0] == 0 {
            return Ok(out);
        }
        out.push(byte[0]);
    }
    Err(HostResult::Unavailable(format!(
        "host string read at 0x{addr:x} has no terminator within {max} bytes"
    )))
}

fn strlen(state: &mut State) -> HostResult {
    let addr = state.x[0];
    match c_string(state, addr, 1 << 20) {
        Ok(s) => HostResult::Returned(s.len() as u64),
        Err(e) => e,
    }
}

fn strnlen(state: &mut State) -> HostResult {
    let (addr, max) = (state.x[0], state.x[1]);
    match c_string(state, addr, max.max(1)) {
        Ok(s) => HostResult::Returned(s.len() as u64),
        // An unterminated string is exactly what strnlen reports as `max`.
        Err(_) => HostResult::Returned(max),
    }
}

fn strcmp(state: &mut State) -> HostResult {
    let (a, b) = (state.x[0], state.x[1]);
    let (left, right) = match (c_string(state, a, 1 << 20), c_string(state, b, 1 << 20)) {
        (Ok(l), Ok(r)) => (l, r),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let order = left.cmp(&right);
    let result = match order {
        std::cmp::Ordering::Less => -1i64,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    HostResult::Returned(result as u64)
}

fn strncmp(state: &mut State) -> HostResult {
    let (a, b, n) = (state.x[0], state.x[1], state.x[2]);
    let (left, right) = if n == 0 {
        (Vec::new(), Vec::new())
    } else {
        // At most n bytes, stopping at a terminator: a string needs no
        // terminator within the compared length.
        let bounded = |at: u64| -> Result<Vec<u8>, HostResult> {
            let mut out = Vec::new();
            for i in 0..n.min(1 << 20) {
                let c = read_bytes(state, at + i, 1)?[0];
                if c == 0 {
                    break;
                }
                out.push(c);
            }
            Ok(out)
        };
        match (bounded(a), bounded(b)) {
            (Ok(l), Ok(r)) => (l, r),
            (Err(e), _) | (_, Err(e)) => return e,
        }
    };
    let order = left.cmp(&right);
    let result = match order {
        std::cmp::Ordering::Less => -1i64,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    HostResult::Returned(result as u64)
}

fn strcpy(state: &mut State) -> HostResult {
    let (dst, src) = (state.x[0], state.x[1]);
    match c_string(state, src, 1 << 20) {
        Ok(mut s) => {
            s.push(0);
            match write_bytes(state, dst, &s) {
                Ok(()) => HostResult::Returned(dst),
                Err(e) => e,
            }
        }
        Err(e) => e,
    }
}

fn strncpy(state: &mut State) -> HostResult {
    let (dst, src, n) = (state.x[0], state.x[1], state.x[2]);
    let mut out = Vec::new();
    match c_string(state, src, 1 << 20) {
        Ok(s) => {
            out.extend_from_slice(&s);
            out.push(0);
            out.resize(n as usize, 0);
        }
        Err(e) => return e,
    }
    match write_bytes(state, dst, &out) {
        Ok(()) => HostResult::Returned(dst),
        Err(e) => e,
    }
}

fn strchr(state: &mut State) -> HostResult {
    let (addr, needle) = (state.x[0], state.x[1] as u8);
    match c_string(state, addr, 1 << 20) {
        Ok(s) => {
            // The terminator is a valid match for a zero needle.
            match s.iter().position(|b| *b == needle) {
                Some(i) => HostResult::Returned(addr + i as u64),
                None if needle == 0 => HostResult::Returned(addr + s.len() as u64),
                None => HostResult::Returned(0),
            }
        }
        Err(e) => e,
    }
}

fn strrchr(state: &mut State) -> HostResult {
    let (addr, needle) = (state.x[0], state.x[1] as u8);
    match c_string(state, addr, 1 << 20) {
        Ok(s) => match s.iter().rposition(|b| *b == needle) {
            Some(i) => HostResult::Returned(addr + i as u64),
            None if needle == 0 => HostResult::Returned(addr + s.len() as u64),
            None => HostResult::Returned(0),
        },
        Err(e) => e,
    }
}

fn strstr(state: &mut State) -> HostResult {
    let (hay, needle) = (state.x[0], state.x[1]);
    let (h, n) = match (c_string(state, hay, 1 << 20), c_string(state, needle, 1 << 20)) {
        (Ok(h), Ok(n)) => (h, n),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    if n.is_empty() {
        return HostResult::Returned(hay);
    }
    if n.len() > h.len() {
        return HostResult::Returned(0);
    }
    match h.windows(n.len()).position(|w| w == n.as_slice()) {
        Some(i) => HostResult::Returned(hay + i as u64),
        None => HostResult::Returned(0),
    }
}

/// `__nnmusl_init_dso` / `__nnmusl_fini_dso`.
///
/// Assumption stated in the claim: these set up per-shared-object C runtime
/// state. The module being translated is the program image, not a shared
/// object, so there is nothing to register. If the guest depends on something
/// this skips, the next stop will say so instead of hiding it.
fn init_dso(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

/// `nn::ro::ProtectRelro` and `nn::ro::UnprotectRelro`.
fn protect_relro(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

/// `atexit`, `at_quick_exit`, `__cxa_atexit` and `__cxa_finalize`.
fn register_exit(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(bytes: &[u8], at: u64) -> State {
        let mut s = State::default();
        s.mem[at as usize..at as usize + bytes.len()].copy_from_slice(bytes);
        s
    }

    #[test]
    fn memcpy_copies_and_returns_the_destination() {
        let mut s = state_with(b"hello", 0x100);
        s.x[0] = 0x200;
        s.x[1] = 0x100;
        s.x[2] = 5;
        assert_eq!(memcpy(&mut s), HostResult::Returned(0x200));
        assert_eq!(&s.mem[0x200..0x205], b"hello");
    }

    #[test]
    fn memset_fills_the_requested_range_only() {
        let mut s = State::default();
        s.x[0] = 0x300;
        s.x[1] = 0xAB;
        s.x[2] = 3;
        assert_eq!(memset(&mut s), HostResult::Returned(0x300));
        assert_eq!(&s.mem[0x300..0x303], &[0xAB, 0xAB, 0xAB]);
        assert_eq!(s.mem[0x303], 0);
    }

    #[test]
    fn strcmp_orders_by_first_difference() {
        let mut s = state_with(b"abc\0abd\0", 0x100);
        s.x[0] = 0x100;
        s.x[1] = 0x104;
        assert_eq!(strcmp(&mut s), HostResult::Returned((-1i64) as u64));
    }

    #[test]
    fn strlen_stops_at_the_terminator() {
        let mut s = state_with(b"fortune\0extra", 0x100);
        s.x[0] = 0x100;
        assert_eq!(strlen(&mut s), HostResult::Returned(7));
    }

    #[test]
    fn out_of_range_read_is_reported_with_the_address() {
        let mut s = State::default();
        s.x[0] = u64::MAX - 4;
        let result = memcpy(&mut s);
        match result {
            HostResult::Unavailable(msg) => assert!(msg.contains("0x"), "{msg}"),
            other => panic!("expected a reported fault, got {other:?}"),
        }
    }

    #[test]
    fn defaults_cover_the_c_memory_and_string_routines() {
        let s = HostServices::defaults();
        for name in ["memcpy", "memset", "strlen", "strcmp", "__nnmusl_init_dso"] {
            assert!(s.get(name).is_some(), "missing {name}");
            assert!(s.claim(name).is_some());
        }
        assert!(s.get("nn::os::CreateThread").is_none());
    }

    #[test]
    fn prefix_registration_covers_overloads() {
        let s = HostServices::defaults();
        assert!(s.get("_ZN2nn2ro12ProtectRelroEPKvS2_S2_S2_S2_").is_some());
        assert!(s.get("_ZN2nn2ro14UnprotectRelroEPKv").is_some());
        assert!(s.claim("_ZN2nn2ro12ProtectRelroEPKvS2_S2_").unwrap().contains("flat"));
    }
}
