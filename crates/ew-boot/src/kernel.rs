//! Supervisor calls: the kernel interface under nnSdk's own code.
//!
//! Most of the title's system calls go through the host's `nn::*` services.
//! Library code that runs as the title's own code (the C++ runtime in `sdk`)
//! reaches the kernel directly with `svc`. The calls served here are the ones
//! that code needs: lock and condition-variable arbitration on guest memory,
//! the system tick, thread identity, sleeping and debug output.
//!
//! Lock words follow the kernel's protocol: 0 when free, otherwise the owner's
//! handle, with bit 30 set while other threads wait. A waiter blocks in the
//! host scheduler and takes the lock once its word is free; an unlock frees the
//! word, which wakes them.

use crate::hle::{read_bytes, write_bytes, HostResult, HostServices};
use ew_lift::State;

/// Bit 30 of a lock word: other threads are waiting.
const WAITERS: u32 = 0x4000_0000;
/// Kernel result codes.
const OK: u64 = 0;
const TIMED_OUT: u64 = 0xEA01;

fn r32(state: &State, a: u64) -> Result<u32, HostResult> {
    Ok(u32::from_le_bytes(read_bytes(state, a, 4)?.try_into().unwrap()))
}

fn w32(state: &mut State, a: u64, v: u32) -> Result<(), HostResult> {
    write_bytes(state, a, &v.to_le_bytes())
}

macro_rules! t {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return e,
        }
    };
}

/// The running thread's handle, from its thread-local region.
fn my_handle(state: &State) -> u32 {
    r32(state, state.tpidrro + crate::sched::TLS_HANDLE).unwrap_or(0)
}

/// `svc 0x1A ArbitrateLock(ownerHandle, address, tag)`: wait until the lock at
/// `address` is free, then take it with `tag`.
fn arbitrate_lock(state: &mut State) -> HostResult {
    let (addr, tag) = (state.x[1], state.x[2] as u32);
    let word = t!(r32(state, addr));
    if word & !WAITERS == 0 {
        t!(w32(state, addr, tag | (word & WAITERS)));
        return HostResult::Returned(OK);
    }
    if word & !WAITERS == tag {
        // Already ours (the owner released and handed it over).
        return HostResult::Returned(OK);
    }
    if state.x[0] != 0 {
        state.host.log.push(format!(
            "kernel: lock 0x{addr:x} held by 0x{:x} (word 0x{word:x}); thread 0x{tag:x} waits",
            state.x[0] as u32
        ));
        state.x[0] = 0;
    }
    HostResult::Block { resume: arbitrate_lock, timed: false }
}

/// `svc 0x1B ArbitrateUnlock(address)`: free the lock so a waiter can take it.
fn arbitrate_unlock(state: &mut State) -> HostResult {
    let addr = state.x[0];
    // Waiters re-check when the scheduler next considers them.
    t!(w32(state, addr, 0));
    HostResult::Returned(OK)
}

/// Condition variables are keyed by guest address; a signal counts as a
/// wake-up token the waiters take in turn.
#[derive(Default)]
struct Keys {
    tokens: std::collections::BTreeMap<u64, u64>,
}

fn keys<R>(state: &State, f: impl FnOnce(&mut Keys) -> R) -> R {
    state.host.ext.with::<Keys, R>(f)
}

/// `svc 0x1C WaitProcessWideKeyAtomic(mutexAddr, key, tag, timeoutNs)`:
/// release the mutex, wait for a signal on `key`, then retake the mutex.
fn wait_key(state: &mut State) -> HostResult {
    let (mutex, key) = (state.x[0], state.x[1]);
    // First entry: release the mutex and mark this thread as waiting.
    let word = t!(r32(state, mutex));
    if word & !WAITERS == my_handle(state) {
        t!(w32(state, mutex, 0));
    }
    keys(state, |k| {
        k.tokens.entry(key).or_insert(0);
    });
    wait_key_resume(state)
}

fn wait_key_resume(state: &mut State) -> HostResult {
    let (mutex, key, tag, timeout) = (state.x[0], state.x[1], state.x[2] as u32, state.x[3] as i64);
    let woken = keys(state, |k| match k.tokens.get_mut(&key) {
        Some(n) if *n > 0 => {
            *n -= 1;
            true
        }
        _ => false,
    });
    if !woken {
        if timeout == 0 {
            return HostResult::Returned(TIMED_OUT);
        }
        return HostResult::Block { resume: wait_key_resume, timed: timeout > 0 };
    }
    let word = t!(r32(state, mutex));
    if word & !WAITERS != 0 && word & !WAITERS != tag {
        return HostResult::Block { resume: retake_mutex, timed: false };
    }
    t!(w32(state, mutex, tag));
    HostResult::Returned(OK)
}

fn retake_mutex(state: &mut State) -> HostResult {
    let (mutex, tag) = (state.x[0], state.x[2] as u32);
    let word = t!(r32(state, mutex));
    if word & !WAITERS != 0 && word & !WAITERS != tag {
        return HostResult::Block { resume: retake_mutex, timed: false };
    }
    t!(w32(state, mutex, tag));
    HostResult::Returned(OK)
}

/// `svc 0x1D SignalProcessWideKey(key, count)`: wake `count` waiters, or all
/// of them for a negative count.
fn signal_key(state: &mut State) -> HostResult {
    let (key, count) = (state.x[0], state.x[1] as i32);
    keys(state, |k| {
        let n = k.tokens.entry(key).or_insert(0);
        *n = if count < 0 { u32::MAX as u64 } else { *n + count as u64 };
    });
    HostResult::Returned(OK)
}

/// `svc 0x1E GetSystemTick()`.
fn get_system_tick(_state: &mut State) -> HostResult {
    HostResult::Returned(crate::threads::now_ticks())
}

/// `svc 0x0B SleepThread(ns)`: yields; the scheduler runs the others.
fn sleep_thread(_state: &mut State) -> HostResult {
    HostResult::Returned(OK)
}

/// `svc 0x27 OutputDebugString(str, len)`: kept in the host log.
fn output_debug_string(state: &mut State) -> HostResult {
    let (p, n) = (state.x[0], state.x[1].min(4096));
    if let Ok(b) = read_bytes(state, p, n) {
        let text = String::from_utf8_lossy(&b).trim_end().to_string();
        state.host.log.push(format!("debug: {text}"));
    }
    HostResult::Returned(OK)
}

/// `svc 0x26 Break(reason, arg, size)`: the guest asked to stop.
fn break_(state: &mut State) -> HostResult {
    HostResult::Unavailable(format!("the guest called svcBreak (reason 0x{:x}, 0x{:x}, 0x{:x})", state.x[0], state.x[1], state.x[2]))
}

/// `svc 0x25 GetThreadId(handle)`: returns the id in x1.
fn get_thread_id(state: &mut State) -> HostResult {
    let handle = state.x[1] as u32;
    let me = my_handle(state);
    state.x[1] = if handle == 0xFFFF_8000 { me as u64 } else { handle as u64 };
    HostResult::Returned(OK)
}

pub fn register(s: &mut HostServices) {
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("svc:0xb", sleep_thread, "yields to the other guest threads"),
        ("svc:0x1a", arbitrate_lock, "waits until the lock word is free, then takes it"),
        ("svc:0x1b", arbitrate_unlock, "frees the lock word"),
        ("svc:0x1c", wait_key, "condition wait on a guest address"),
        ("svc:0x1d", signal_key, "condition signal on a guest address"),
        ("svc:0x1e", get_system_tick, "the host clock at 19.2 MHz"),
        ("svc:0x25", get_thread_id, "the thread's handle serves as its id"),
        ("svc:0x26", break_, "stops the run with the guest's reason"),
        ("svc:0x27", output_debug_string, "kept in the host log"),
    ];
    for (name, handler, claim) in table {
        s.register(name, *handler, claim);
    }
}
