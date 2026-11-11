//! Multi-threaded `nn::os`: threads, blocking synchronisation, sleeps and timers.
//!
//! Guest threads are scheduled by the dispatcher (see `sched`). A service that
//! has to wait returns [`HostResult::Block`] with a continuation; the scheduler
//! parks the thread and retries the continuation, with that thread's registers
//! loaded, whenever it next considers the thread. Waits are therefore real
//! waits: a mutex held by another thread, an unsignalled event, a condition
//! variable or `WaitThread` all suspend the caller until another thread changes
//! the state, and a run in which every thread waits on something only another
//! thread could provide stops as a deadlock with every waiter named.
//!
//! Continuations keep their private data in caller-saved registers (`x2`..`x7`
//! beyond the call's own arguments), which the AArch64 procedure call standard
//! lets a callee clobber, so the guest cannot observe the bookkeeping.
//!
//! State lives in the guest objects, tagged with magic words, as in `services`.
//! Mutexes record their owner, so ownership errors are reported, not ignored.

use crate::hle::{read_bytes, write_bytes, HostResult, HostServices};
use ew_lift::{State, ThreadInfo};

const CV_NEXT: u64 = 0;
const CV_RELEASED: u64 = 4;
const TIMER_MAGIC: u32 = u32::from_le_bytes(*b"EWTM");
const EVENT_MAGIC: u32 = u32::from_le_bytes(*b"EWEV");
const SEM_MAGIC: u32 = u32::from_le_bytes(*b"EWSM");
const RW_MAGIC: u32 = u32::from_le_bytes(*b"EWRW");

/// The tick frequency every title measures time against.
pub const TICK_FREQ: u64 = 19_200_000;

/// `nn::os::ConditionVariableStatus`.
const CV_NO_TIMEOUT: u64 = 0;
const CV_TIMEOUT: u64 = 1;

// ---- guest object access ----------------------------------------------------

fn r32(state: &State, a: u64) -> Result<u32, HostResult> {
    let b = read_bytes(state, a, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn w32(state: &mut State, a: u64, v: u32) -> Result<(), HostResult> {
    write_bytes(state, a, &v.to_le_bytes())
}
fn r64(state: &State, a: u64) -> Result<u64, HostResult> {
    let b = read_bytes(state, a, 8)?;
    Ok(u64::from_le_bytes(b.try_into().unwrap()))
}
fn w64(state: &mut State, a: u64, v: u64) -> Result<(), HostResult> {
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

fn me(state: &State) -> u64 {
    state.host.thread_object
}

/// Host clock in guest ticks. Monotonic, so a deadline cannot move backwards.
pub fn now_ticks() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    let start = *START.get_or_init(std::time::Instant::now);
    let ns = start.elapsed().as_nanos() as u64;
    // Offset so the first tick is not zero, which some titles treat as "unset".
    ns.saturating_mul(TICK_FREQ) / 1_000_000_000 + TICK_FREQ
}

fn ns_to_ticks(ns: u64) -> u64 {
    ((ns as u128 * TICK_FREQ as u128) / 1_000_000_000) as u64
}

fn block(resume: fn(&mut State) -> HostResult) -> HostResult {
    HostResult::Block { resume, timed: false }
}
fn block_timed(resume: fn(&mut State) -> HostResult) -> HostResult {
    HostResult::Block { resume, timed: true }
}

// ---- mutexes -------------------------------------------------------------------
//
// Mutex state lives on the host, keyed by the guest object's address: the
// guest objects differ in size (nn::os::MutexType is 32 bytes, this SDK's
// pthread_mutex_t is 8, and a condition variable often follows directly), so
// writing state into them would corrupt their neighbours. An address the host
// has not seen is a zeroed static mutex: unlocked and not recursive.

/// (depth, recursive, owner) by mutex address.
#[derive(Default)]
struct Mutexes(std::collections::HashMap<u64, (u32, u32, u64)>);

fn mutex_state(state: &mut State, m: u64) -> Result<(u32, u32, u64), HostResult> {
    Ok(state.host.ext.with::<Mutexes, _>(|x| x.0.get(&m).copied().unwrap_or_default()))
}

/// Record a new depth and owner, keeping the mutex's recursive flag.
fn mutex_hold(state: &mut State, m: u64, depth: u32, owner: u64) {
    state.host.ext.with::<Mutexes, _>(|x| {
        let e = x.0.entry(m).or_default();
        e.0 = depth;
        e.2 = owner;
    });
}

fn mutex_init(state: &mut State) -> HostResult {
    let (m, recursive) = (state.x[0], (state.x[1] & 1) as u32);
    state.host.ext.with::<Mutexes, _>(|x| x.0.insert(m, (0, recursive, 0)));
    HostResult::Returned(0)
}

fn mutex_finalize(state: &mut State) -> HostResult {
    let m = state.x[0];
    state.host.ext.with::<Mutexes, _>(|x| x.0.remove(&m));
    HostResult::Returned(0)
}

/// Take the mutex in `x0` for the current thread, or say who holds it.
fn try_take(state: &mut State, m: u64) -> Result<bool, HostResult> {
    let (depth, recursive, owner) = mutex_state(state, m)?;
    let cur = me(state);
    if depth == 0 {
        mutex_hold(state, m, 1, cur);
        return Ok(true);
    }
    if owner == cur {
        if recursive == 0 {
            return Err(HostResult::Unavailable(format!(
                "self-deadlock: thread 0x{cur:x} locked the non-recursive mutex at 0x{m:x} twice"
            )));
        }
        mutex_hold(state, m, depth + 1, cur);
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn mutex_lock(state: &mut State) -> HostResult {
    match try_take(state, state.x[0]) {
        Ok(true) => HostResult::Returned(0),
        Ok(false) => block(mutex_lock),
        Err(e) => e,
    }
}

fn mutex_try_lock(state: &mut State) -> HostResult {
    match try_take(state, state.x[0]) {
        Ok(taken) => HostResult::Returned(taken as u64),
        Err(e) => e,
    }
}

fn release(state: &mut State, m: u64) -> Result<(), HostResult> {
    let (depth, _, owner) = mutex_state(state, m)?;
    let cur = me(state);
    if depth == 0 || owner != cur {
        return Err(HostResult::Unavailable(format!(
            "thread 0x{cur:x} unlocked the mutex at 0x{m:x}, which it does not hold (depth {depth}, owner 0x{owner:x})"
        )));
    }
    mutex_hold(state, m, depth - 1, if depth == 1 { 0 } else { owner });
    Ok(())
}

pub(crate) fn mutex_unlock(state: &mut State) -> HostResult {
    match release(state, state.x[0]) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn mutex_locked_by_me(state: &mut State) -> HostResult {
    let m = state.x[0];
    let (depth, _, owner) = t!(mutex_state(state, m));
    HostResult::Returned((depth > 0 && owner == me(state)) as u64)
}

// ---- condition variables: [next_ticket, released] ---------------------

fn cv_init(state: &mut State) -> HostResult {
    let cv = state.x[0];
    t!(w32(state, cv + CV_NEXT, 0));
    t!(w32(state, cv + CV_RELEASED, 0));
    HostResult::Returned(0)
}

/// The next ticket to hand out and the release mark. `ConditionVariableType`
/// is 8 bytes and is often followed directly by its mutex, so both counters
/// live in those 8 bytes; zeroed memory is a valid, empty condition variable.
pub(crate) fn cv_fields(state: &mut State, cv: u64) -> Result<(u32, u32), HostResult> {
    Ok((r32(state, cv + CV_NEXT)?, r32(state, cv + CV_RELEASED)?))
}

pub(crate) fn cv_signal(state: &mut State) -> HostResult {
    let cv = state.x[0];
    let (next, released) = t!(cv_fields(state, cv));
    if released < next {
        t!(w32(state, cv + CV_RELEASED, released + 1));
    }
    HostResult::Returned(0)
}

pub(crate) fn cv_broadcast(state: &mut State) -> HostResult {
    let cv = state.x[0];
    let (next, _) = t!(cv_fields(state, cv));
    t!(w32(state, cv + CV_RELEASED, next));
    HostResult::Returned(0)
}

/// Common wait: take a ticket, release the mutex completely, park.
/// x0 = cv, x1 = mutex; the ticket goes to x2 and the saved depth to x5.
pub(crate) fn cv_enter(state: &mut State) -> Result<(), HostResult> {
    let (cv, m) = (state.x[0], state.x[1]);
    let (next, _) = cv_fields(state, cv)?;
    w32(state, cv + CV_NEXT, next.wrapping_add(1))?;
    let (depth, _, owner) = mutex_state(state, m)?;
    if depth == 0 || owner != me(state) {
        return Err(HostResult::Unavailable(format!(
            "waited on the condition variable at 0x{cv:x} without holding the mutex at 0x{m:x}"
        )));
    }
    mutex_hold(state, m, 0, 0);
    state.x[2] = next as u64;
    state.x[5] = depth as u64;
    Ok(())
}

/// Reacquire the mutex with its saved depth, then return `x6`.
pub(crate) fn cv_relock(state: &mut State) -> HostResult {
    let m = state.x[1];
    let (depth, _, _) = t!(mutex_state(state, m));
    if depth != 0 {
        return block(cv_relock);
    }
    let cur = me(state);
    mutex_hold(state, m, state.x[5] as u32, cur);
    HostResult::Returned(state.x[6])
}

fn cv_resume(state: &mut State) -> HostResult {
    let (cv, ticket) = (state.x[0], state.x[2] as u32);
    let (_, released) = t!(cv_fields(state, cv));
    // Tickets are handed out in order and released in order, so a ticket below
    // the release mark has been signalled.
    if ticket.wrapping_sub(released) as i32 >= 0 {
        return block(cv_resume);
    }
    state.x[6] = CV_NO_TIMEOUT;
    cv_relock(state)
}

fn cv_wait(state: &mut State) -> HostResult {
    t!(cv_enter(state));
    cv_resume(state)
}

/// `TimedWaitConditionVariable(cv, mutex, TimeSpan)`: deadline kept in x3.
fn cv_timed_resume(state: &mut State) -> HostResult {
    let (cv, ticket, deadline) = (state.x[0], state.x[2] as u32, state.x[3]);
    let (next, released) = t!(cv_fields(state, cv));
    if (ticket.wrapping_sub(released) as i32) < 0 {
        state.x[6] = CV_NO_TIMEOUT;
        return cv_relock(state);
    }
    if now_ticks() >= deadline {
        // Withdraw this waiter: the slot it held is skipped by a later signal
        // only if it is the newest; otherwise a signal may land on it, which is
        // the spurious-wakeup latitude the API already grants.
        if ticket.wrapping_add(1) == next {
            t!(w32(state, cv + CV_NEXT, ticket));
        }
        state.x[6] = CV_TIMEOUT;
        return cv_relock(state);
    }
    block_timed(cv_timed_resume)
}

fn cv_timed_wait(state: &mut State) -> HostResult {
    let deadline = now_ticks().saturating_add(ns_to_ticks(state.x[2]));
    t!(cv_enter(state));
    state.x[3] = deadline;
    cv_timed_resume(state)
}

// ---- events, light events, semaphores, reader/writer locks -------------------
// Layouts match `services`: event [magic, signalled, manual-clear],
// light event [signalled], semaphore [magic, count, max], rw [magic, readers, writer].

fn event_wait(state: &mut State) -> HostResult {
    let e = state.x[0];
    if t!(r32(state, e)) != EVENT_MAGIC {
        return HostResult::Unavailable(format!("0x{e:x} is not an initialised event"));
    }
    if t!(r32(state, e + 4)) == 0 {
        return block(event_wait);
    }
    // nn::os::EventClearMode: ManualClear = 0, AutoClear = 1.
    if t!(r32(state, e + 8)) == 1 {
        t!(w32(state, e + 4, 0));
    }
    HostResult::Returned(0)
}

/// Light events auto-clear on a successful wait in this model, as `services`
/// implemented them for the single-threaded case.
fn light_event_wait(state: &mut State) -> HostResult {
    let e = state.x[0];
    if t!(r32(state, e)) == 0 {
        return block(light_event_wait);
    }
    t!(w32(state, e, 0));
    HostResult::Returned(0)
}

fn sem_acquire(state: &mut State) -> HostResult {
    let s = state.x[0];
    if t!(r32(state, s)) != SEM_MAGIC {
        return HostResult::Unavailable(format!("0x{s:x} is not an initialised semaphore"));
    }
    let count = t!(r32(state, s + 4));
    if count == 0 {
        return block(sem_acquire);
    }
    t!(w32(state, s + 4, count - 1));
    HostResult::Returned(0)
}

fn rw_read(state: &mut State) -> HostResult {
    let l = state.x[0];
    if t!(r32(state, l)) != RW_MAGIC {
        return HostResult::Unavailable(format!("0x{l:x} is not an initialised reader/writer lock"));
    }
    if t!(r32(state, l + 8)) != 0 {
        return block(rw_read);
    }
    let readers = t!(r32(state, l + 4));
    t!(w32(state, l + 4, readers + 1));
    HostResult::Returned(0)
}

fn rw_write(state: &mut State) -> HostResult {
    let l = state.x[0];
    if t!(r32(state, l)) != RW_MAGIC {
        return HostResult::Unavailable(format!("0x{l:x} is not an initialised reader/writer lock"));
    }
    if t!(r32(state, l + 4)) != 0 || t!(r32(state, l + 8)) != 0 {
        return block(rw_write);
    }
    t!(w32(state, l + 8, 1));
    HostResult::Returned(0)
}

// ---- threads -------------------------------------------------------------------

/// `CreateThread(ThreadType*, ThreadFunction, void* arg, void* stack,
/// size_t stackSize, int priority[, int idealCore])`. The game supplies the
/// stack; the thread starts at `StartThread`.
fn create_thread(state: &mut State) -> HostResult {
    let (t, entry, arg, stack, size, prio) =
        (state.x[0], state.x[1], state.x[2], state.x[3], state.x[4], state.x[5] as i32);
    if t == 0 || entry == 0 || stack == 0 || size < 0x1000 {
        return HostResult::Unavailable(format!(
            "CreateThread(0x{t:x}, entry 0x{entry:x}, stack 0x{stack:x}+0x{size:x}) has a null or tiny argument"
        ));
    }
    if stack.checked_add(size).is_none_or(|end| end as usize > state.mem.len()) {
        return HostResult::Unavailable(format!("thread stack 0x{stack:x}+0x{size:x} is outside guest memory"));
    }
    state.host.threads.insert(
        t,
        ThreadInfo { entry, arg, stack_top: (stack + size) & !0xF, priority: prio, ..Default::default() },
    );
    HostResult::Returned(0)
}

fn start_thread(state: &mut State) -> HostResult {
    let t = state.x[0];
    match state.host.threads.get_mut(&t) {
        Some(info) => {
            info.started = true;
            state.host.thread_starts += 1;
            HostResult::Returned(0)
        }
        None => HostResult::Unavailable(format!("StartThread on 0x{t:x}, which was never created")),
    }
}

fn wait_thread(state: &mut State) -> HostResult {
    let t = state.x[0];
    match state.host.threads.get(&t) {
        Some(info) if info.exited => HostResult::Returned(0),
        Some(info) if !info.started => HostResult::Unavailable(format!(
            "WaitThread on 0x{t:x}, which was created but never started, would wait forever"
        )),
        Some(_) => block(wait_thread),
        None => HostResult::Unavailable(format!("WaitThread on 0x{t:x}, which was never created")),
    }
}

fn destroy_thread(state: &mut State) -> HostResult {
    let t = state.x[0];
    match state.host.threads.get(&t) {
        Some(info) if info.exited || !info.started => {
            state.host.threads.remove(&t);
            HostResult::Returned(0)
        }
        Some(_) => block(destroy_thread),
        None => HostResult::Unavailable(format!("DestroyThread on 0x{t:x}, which was never created")),
    }
}

fn set_thread_name(state: &mut State) -> HostResult {
    let (t, name) = (state.x[0], state.x[1]);
    if let Some(info) = state.host.threads.get_mut(&t) {
        info.name = name;
    }
    HostResult::Returned(0)
}

fn current_thread(state: &mut State) -> HostResult {
    HostResult::Returned(me(state))
}

fn thread_priority(state: &mut State) -> HostResult {
    let t = state.x[0];
    HostResult::Returned(state.host.threads.get(&t).map(|i| i.priority as i64 as u64).unwrap_or(44))
}

fn change_priority(state: &mut State) -> HostResult {
    let (t, p) = (state.x[0], state.x[1] as i32);
    let old = state.host.threads.get(&t).map(|i| i.priority).unwrap_or(44);
    if let Some(info) = state.host.threads.get_mut(&t) {
        info.priority = p;
    }
    HostResult::Returned(old as i64 as u64)
}

/// `YieldThread()`: park for one scheduling round.
fn yield_thread(_state: &mut State) -> HostResult {
    block(yielded)
}
fn yielded(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

/// `SleepThread(TimeSpan ns)`: deadline in x1.
fn sleep_thread(state: &mut State) -> HostResult {
    let ns = state.x[0] as i64;
    if ns <= 0 {
        return block(yielded);
    }
    state.x[1] = now_ticks().saturating_add(ns_to_ticks(ns as u64));
    block_timed(sleep_resume)
}
fn sleep_resume(state: &mut State) -> HostResult {
    if now_ticks() >= state.x[1] {
        HostResult::Returned(0)
    } else {
        block_timed(sleep_resume)
    }
}

fn system_tick(_state: &mut State) -> HostResult {
    HostResult::Returned(now_ticks())
}

// ---- timer events: [magic, signalled, manual, active, next u64, period u64] ----

fn timer_init(state: &mut State) -> HostResult {
    let e = state.x[0];
    // nn::os::EventClearMode: ManualClear = 0, AutoClear = 1.
    let mode = (state.x[1] & 1) as u32;
    t!(w32(state, e, TIMER_MAGIC));
    t!(w32(state, e + 4, 0));
    t!(w32(state, e + 8, mode));
    t!(w32(state, e + 12, 0));
    t!(w64(state, e + 16, 0));
    t!(w64(state, e + 24, 0));
    HostResult::Returned(0)
}

fn timer_finalize(state: &mut State) -> HostResult {
    t!(w32(state, state.x[0], 0));
    HostResult::Returned(0)
}

/// Fire the timer if its deadline has passed, then report whether it is set.
fn timer_poll(state: &mut State, e: u64) -> Result<bool, HostResult> {
    if r32(state, e)? != TIMER_MAGIC {
        return Err(HostResult::Unavailable(format!("0x{e:x} is not an initialised timer event")));
    }
    if r32(state, e + 12)? != 0 {
        let next = r64(state, e + 16)?;
        let now = now_ticks();
        if now >= next {
            w32(state, e + 4, 1)?;
            let period = r64(state, e + 24)?;
            if period == 0 {
                w32(state, e + 12, 0)?;
            } else {
                let missed = (now - next) / period + 1;
                w64(state, e + 16, next + missed * period)?;
            }
        }
    }
    Ok(r32(state, e + 4)? != 0)
}

fn timer_consume(state: &mut State, e: u64) -> Result<(), HostResult> {
    if r32(state, e + 8)? == 1 {
        w32(state, e + 4, 0)?;
    }
    Ok(())
}

fn timer_start(state: &mut State, e: u64, first_ns: u64, period_ns: u64) -> HostResult {
    if t!(r32(state, e)) != TIMER_MAGIC {
        return HostResult::Unavailable(format!("0x{e:x} is not an initialised timer event"));
    }
    state.host.log.push(format!("os: timer 0x{e:x} armed, first {first_ns} ns, period {period_ns} ns"));
    t!(w32(state, e + 12, 1));
    t!(w64(state, e + 16, now_ticks().saturating_add(ns_to_ticks(first_ns))));
    t!(w64(state, e + 24, ns_to_ticks(period_ns)));
    HostResult::Returned(0)
}

fn timer_start_periodic(state: &mut State) -> HostResult {
    let (e, first, period) = (state.x[0], state.x[1], state.x[2]);
    timer_start(state, e, first, period.max(1))
}
fn timer_start_oneshot(state: &mut State) -> HostResult {
    let (e, first) = (state.x[0], state.x[1]);
    timer_start(state, e, first, 0)
}
fn timer_stop(state: &mut State) -> HostResult {
    t!(w32(state, state.x[0] + 12, 0));
    HostResult::Returned(0)
}
fn timer_signal(state: &mut State) -> HostResult {
    t!(w32(state, state.x[0] + 4, 1));
    HostResult::Returned(0)
}
fn timer_clear(state: &mut State) -> HostResult {
    t!(w32(state, state.x[0] + 4, 0));
    HostResult::Returned(0)
}
fn timer_wait(state: &mut State) -> HostResult {
    let e = state.x[0];
    if !t!(timer_poll(state, e)) {
        let active = t!(r32(state, e + 12)) != 0;
        return if active { block_timed(timer_wait) } else { block(timer_wait) };
    }
    t!(timer_consume(state, e));
    HostResult::Returned(0)
}
fn timer_try_wait(state: &mut State) -> HostResult {
    let e = state.x[0];
    if !t!(timer_poll(state, e)) {
        return HostResult::Returned(0);
    }
    t!(timer_consume(state, e));
    HostResult::Returned(1)
}

// ---- multi-wait --------------------------------------------------------------
//
// Holders and their links are kept on the host, keyed by the guest objects'
// addresses, so the guest structures' own sizes never matter. A holder waits
// on an event or a system event (an ordinary event or a host timer event).
// WaitAny reports a signalled holder without consuming the signal, as on the
// console: the caller then waits on or clears the object itself.

#[derive(Default)]
struct MultiWaits {
    /// Each multi-wait's linked holders, in link order.
    lists: std::collections::HashMap<u64, Vec<u64>>,
    /// Each holder: (waited object, user data, linked multi-wait).
    holders: std::collections::HashMap<u64, (u64, u64, Option<u64>)>,
}

fn multi<R>(state: &State, f: impl FnOnce(&mut MultiWaits) -> R) -> R {
    state.host.ext.with::<MultiWaits, R>(f)
}

/// Whether an event or system event is signalled, without consuming it.
fn object_signalled(state: &mut State, e: u64) -> Result<bool, HostResult> {
    match r32(state, e)? {
        TIMER_MAGIC => timer_poll(state, e),
        EVENT_MAGIC => Ok(r32(state, e + 4)? != 0),
        // Never initialised by the host: nothing can have signalled it.
        _ => Ok(false),
    }
}

fn mw_init(state: &mut State) -> HostResult {
    let mw = state.x[0];
    multi(state, |m| m.lists.insert(mw, Vec::new()));
    HostResult::Returned(0)
}

fn mw_finalize(state: &mut State) -> HostResult {
    let mw = state.x[0];
    multi(state, |m| m.lists.remove(&mw));
    HostResult::Returned(0)
}

fn mw_holder_init(state: &mut State) -> HostResult {
    let (holder, object) = (state.x[0], state.x[1]);
    multi(state, |m| m.holders.insert(holder, (object, 0, None)));
    HostResult::Returned(0)
}

fn mw_holder_finalize(state: &mut State) -> HostResult {
    let holder = state.x[0];
    multi(state, |m| m.holders.remove(&holder));
    HostResult::Returned(0)
}

fn mw_link(state: &mut State) -> HostResult {
    let (mw, holder) = (state.x[0], state.x[1]);
    let linked = multi(state, |m| {
        let h = m.holders.get_mut(&holder)?;
        h.2 = Some(mw);
        m.lists.entry(mw).or_default().push(holder);
        Some(())
    });
    match linked {
        Some(()) => HostResult::Returned(0),
        None => HostResult::Unavailable(format!("LinkMultiWaitHolder: holder 0x{holder:x} was never initialised")),
    }
}

fn mw_unlink(state: &mut State) -> HostResult {
    let holder = state.x[0];
    multi(state, |m| {
        if let Some(mw) = m.holders.get_mut(&holder).and_then(|h| h.2.take()) {
            if let Some(list) = m.lists.get_mut(&mw) {
                list.retain(|h| *h != holder);
            }
        }
    });
    HostResult::Returned(0)
}

fn mw_set_user_data(state: &mut State) -> HostResult {
    let (holder, data) = (state.x[0], state.x[1]);
    multi(state, |m| m.holders.entry(holder).or_insert((0, 0, None)).1 = data);
    HostResult::Returned(0)
}

fn mw_get_user_data(state: &mut State) -> HostResult {
    let holder = state.x[0];
    HostResult::Returned(multi(state, |m| m.holders.get(&holder).map(|h| h.1).unwrap_or(0)))
}

/// `WaitAny(MultiWaitType*)`: the first linked holder whose object is
/// signalled, else wait.
fn mw_wait_any(state: &mut State) -> HostResult {
    let mw = state.x[0];
    let linked: Vec<(u64, u64)> = multi(state, |m| {
        m.lists.get(&mw).map(|l| l.iter().filter_map(|h| m.holders.get(h).map(|x| (*h, x.0))).collect()).unwrap_or_default()
    });
    let mut timed = false;
    for (holder, object) in linked {
        if t!(object_signalled(state, object)) {
            return HostResult::Returned(holder);
        }
        timed |= t!(r32(state, object)) == TIMER_MAGIC;
    }
    if timed { block_timed(mw_wait_any) } else { block(mw_wait_any) }
}

// ---- system events ----------------------------------------------------------
//
// A system event the host creates (an audio renderer's frame event) is a timer
// event in the guest object, so waits block until the host clock fires it.
// Waits on events the game set up as ordinary events use the event layout.

/// Set up a timer event in a guest object, unarmed.
pub fn init_timer_event(state: &mut State, e: u64, auto_clear: bool) -> Result<(), HostResult> {
    w32(state, e, TIMER_MAGIC)?;
    w32(state, e + 4, 0)?;
    w32(state, e + 8, auto_clear as u32)?;
    w32(state, e + 12, 0)?;
    w64(state, e + 16, 0)?;
    w64(state, e + 24, 0)
}

pub fn start_timer_event(state: &mut State, e: u64, first_ns: u64, period_ns: u64) -> Result<(), HostResult> {
    match timer_start(state, e, first_ns, period_ns) {
        HostResult::Returned(_) => Ok(()),
        other => Err(other),
    }
}

pub fn stop_timer_event(state: &mut State, e: u64) -> Result<(), HostResult> {
    w32(state, e + 12, 0)
}

fn is_timer(state: &State, e: u64) -> Result<bool, HostResult> {
    Ok(r32(state, e)? == TIMER_MAGIC)
}

fn system_event_wait(state: &mut State) -> HostResult {
    if t!(is_timer(state, state.x[0])) { timer_wait(state) } else { event_wait(state) }
}

fn system_event_try_wait(state: &mut State) -> HostResult {
    if t!(is_timer(state, state.x[0])) {
        return timer_try_wait(state);
    }
    let e = state.x[0];
    if t!(r32(state, e)) != EVENT_MAGIC {
        return HostResult::Unavailable(format!("0x{e:x} is not an initialised system event"));
    }
    if t!(r32(state, e + 4)) == 0 {
        return HostResult::Returned(0);
    }
    if t!(r32(state, e + 8)) == 1 {
        t!(w32(state, e + 4, 0));
    }
    HostResult::Returned(1)
}

/// `TimedWaitSystemEvent(event, TimeSpan)`: deadline kept in x3.
fn system_event_timed_resume(state: &mut State) -> HostResult {
    let deadline = state.x[3];
    match system_event_try_wait(state) {
        HostResult::Returned(1) => HostResult::Returned(1),
        HostResult::Returned(_) if now_ticks() >= deadline => HostResult::Returned(0),
        HostResult::Returned(_) => block_timed(system_event_timed_resume),
        other => other,
    }
}

fn system_event_timed_wait(state: &mut State) -> HostResult {
    state.x[3] = now_ticks().saturating_add(ns_to_ticks(state.x[1]));
    system_event_timed_resume(state)
}

fn system_event_clear(state: &mut State) -> HostResult {
    t!(w32(state, state.x[0] + 4, 0));
    HostResult::Returned(0)
}

fn system_event_destroy(state: &mut State) -> HostResult {
    t!(w32(state, state.x[0], 0));
    HostResult::Returned(0)
}

// ---- message queues: [magic, capacity, used, head, buffer u64] -------------
//
// Messages live in the buffer the game supplies, as a ring, exactly as the
// console keeps them. A full send or an empty receive blocks the caller.

const MQ_MAGIC: u32 = u32::from_le_bytes(*b"EWMQ");

fn mq(state: &State, q: u64) -> Result<(u32, u32, u32, u64), HostResult> {
    if r32(state, q)? != MQ_MAGIC {
        return Err(HostResult::Unavailable(format!("0x{q:x} is not an initialised message queue")));
    }
    Ok((r32(state, q + 4)?, r32(state, q + 8)?, r32(state, q + 12)?, r64(state, q + 16)?))
}

fn mq_init(state: &mut State) -> HostResult {
    let (q, buf, count) = (state.x[0], state.x[1], state.x[2]);
    if count == 0 || buf == 0 {
        return HostResult::Unavailable(format!("message queue 0x{q:x} given buffer 0x{buf:x} of {count} entries"));
    }
    t!(w32(state, q, MQ_MAGIC));
    t!(w32(state, q + 4, count as u32));
    t!(w32(state, q + 8, 0));
    t!(w32(state, q + 12, 0));
    t!(w64(state, q + 16, buf));
    HostResult::Returned(0)
}

fn mq_finalize(state: &mut State) -> HostResult {
    t!(w32(state, state.x[0], 0));
    HostResult::Returned(0)
}

/// Push at the back (or the front for `jam`). False when full.
fn mq_push(state: &mut State, q: u64, msg: u64, jam: bool) -> Result<bool, HostResult> {
    let (cap, used, head, buf) = mq(state, q)?;
    if used == cap {
        return Ok(false);
    }
    let (slot, head) = if jam { let h = (head + cap - 1) % cap; (h, h) } else { ((head + used) % cap, head) };
    w64(state, buf + slot as u64 * 8, msg)?;
    w32(state, q + 8, used + 1)?;
    w32(state, q + 12, head)?;
    Ok(true)
}

/// Pop from the front into `out`. False when empty.
fn mq_pop(state: &mut State, out: u64, q: u64) -> Result<bool, HostResult> {
    let (cap, used, head, buf) = mq(state, q)?;
    if used == 0 {
        return Ok(false);
    }
    let msg = r64(state, buf + head as u64 * 8)?;
    w64(state, out, msg)?;
    w32(state, q + 8, used - 1)?;
    w32(state, q + 12, (head + 1) % cap)?;
    Ok(true)
}

fn mq_send(state: &mut State) -> HostResult {
    let (q, msg) = (state.x[0], state.x[1]);
    if t!(mq_push(state, q, msg, false)) { HostResult::Returned(0) } else { block(mq_send) }
}

fn mq_jam(state: &mut State) -> HostResult {
    let (q, msg) = (state.x[0], state.x[1]);
    if t!(mq_push(state, q, msg, true)) { HostResult::Returned(0) } else { block(mq_jam) }
}

fn mq_try_send(state: &mut State) -> HostResult {
    let (q, msg) = (state.x[0], state.x[1]);
    HostResult::Returned(t!(mq_push(state, q, msg, false)) as u64)
}

fn mq_receive(state: &mut State) -> HostResult {
    let (out, q) = (state.x[0], state.x[1]);
    if t!(mq_pop(state, out, q)) { HostResult::Returned(0) } else { block(mq_receive) }
}

fn mq_try_receive(state: &mut State) -> HostResult {
    let (out, q) = (state.x[0], state.x[1]);
    HostResult::Returned(t!(mq_pop(state, out, q)) as u64)
}

pub fn register(s: &mut HostServices) {
    let system_events: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("_ZN2nn2os19InitializeMultiWaitEPNS0_13MultiWaitTypeE", mw_init, "holders are kept on the host"),
        ("_ZN2nn2os17FinalizeMultiWaitEPNS0_13MultiWaitTypeE", mw_finalize, "forgets its links"),
        ("_ZN2nn2os25InitializeMultiWaitHolderEPNS0_19MultiWaitHolderTypeEPNS0_9EventTypeE", mw_holder_init, "a holder for an event"),
        ("_ZN2nn2os25InitializeMultiWaitHolderEPNS0_19MultiWaitHolderTypeEPNS0_15SystemEventTypeE", mw_holder_init, "a holder for a system event"),
        ("_ZN2nn2os23FinalizeMultiWaitHolderEPNS0_19MultiWaitHolderTypeE", mw_holder_finalize, "forgets the holder"),
        ("_ZN2nn2os19LinkMultiWaitHolderEPNS0_13MultiWaitTypeEPNS0_19MultiWaitHolderTypeE", mw_link, "links in order"),
        ("_ZN2nn2os21UnlinkMultiWaitHolderEPNS0_19MultiWaitHolderTypeE", mw_unlink, "unlinks"),
        ("_ZN2nn2os26SetMultiWaitHolderUserDataEPNS0_19MultiWaitHolderTypeEm", mw_set_user_data, "kept on the host"),
        ("_ZN2nn2os26GetMultiWaitHolderUserDataEPKNS0_19MultiWaitHolderTypeE", mw_get_user_data, "what was set"),
        ("_ZN2nn2os7WaitAnyEPNS0_13MultiWaitTypeE", mw_wait_any, "the first signalled holder in link order; signals are not consumed"),
        ("_ZN2nn2os15WaitSystemEventEPNS0_15SystemEventTypeE", system_event_wait, "blocks until the event is signalled or its timer fires"),
        ("_ZN2nn2os18TryWaitSystemEventEPNS0_15SystemEventTypeE", system_event_try_wait, "reports and consumes a pending signal"),
        ("_ZN2nn2os20TimedWaitSystemEventEPNS0_15SystemEventTypeENS_8TimeSpanE", system_event_timed_wait, "waits up to the timeout"),
        ("_ZN2nn2os16ClearSystemEventEPNS0_15SystemEventTypeE", system_event_clear, "clears the signal"),
        ("_ZN2nn2os18DestroySystemEventEPNS0_15SystemEventTypeE", system_event_destroy, "clears the tag"),
    ];
    for (name, handler, claim) in system_events {
        s.register(name, *handler, claim);
    }
    let queues: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("_ZN2nn2os22InitializeMessageQueueEPNS0_16MessageQueueTypeEPmm", mq_init, "a ring in the game's buffer"),
        ("_ZN2nn2os20FinalizeMessageQueueEPNS0_16MessageQueueTypeE", mq_finalize, "clears the tag"),
        ("_ZN2nn2os16SendMessageQueueEPNS0_16MessageQueueTypeEm", mq_send, "blocks while the queue is full"),
        ("_ZN2nn2os19ReceiveMessageQueueEPmPNS0_16MessageQueueTypeE", mq_receive, "blocks while the queue is empty"),
        ("_ZN2nn2os27InitializeLightMessageQueueEPNS0_21LightMessageQueueTypeEPmm", mq_init, "a ring in the game's buffer"),
        ("_ZN2nn2os25FinalizeLightMessageQueueEPNS0_21LightMessageQueueTypeE", mq_finalize, "clears the tag"),
        ("_ZN2nn2os21SendLightMessageQueueEPNS0_21LightMessageQueueTypeEm", mq_send, "blocks while the queue is full"),
        ("_ZN2nn2os24ReceiveLightMessageQueueEPmPNS0_21LightMessageQueueTypeE", mq_receive, "blocks while the queue is empty"),
    ];
    for (name, handler, claim) in queues {
        s.register(name, *handler, claim);
    }
    s.register("_ZN2nn2os22TryReceiveMessageQueueEPmPNS0_16MessageQueueTypeE", mq_try_receive, "false when empty");
    s.register("_ZN2nn2os19TrySendMessageQueueEPNS0_16MessageQueueTypeEm", mq_try_send, "false when full");
    s.register("_ZN2nn2os15JamMessageQueueEPNS0_16MessageQueueTypeEm", mq_jam, "pushes at the front, blocking while full");
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("_ZN2nn2os15InitializeMutexE", mutex_init, "owner-tracking mutex in the caller's object"),
        ("_ZN2nn2os13FinalizeMutexE", mutex_finalize, "clears the tag"),
        ("_ZN2nn2os9LockMutexE", mutex_lock, "blocks the calling guest thread while another thread holds it"),
        ("_ZN2nn2os12TryLockMutexE", mutex_try_lock, "takes the mutex if free or already owned (recursive)"),
        ("_ZN2nn2os11UnlockMutexE", mutex_unlock, "unlocking a mutex the thread does not hold is reported"),
        ("_ZN2nn2os29IsMutexLockedByCurrentThreadE", mutex_locked_by_me, "compares the recorded owner"),
        ("_ZN2nn2os27InitializeConditionVariableE", cv_init, "ticketed condition variable"),
        ("_ZN2nn2os25FinalizeConditionVariableE", mutex_finalize, "clears the tag"),
        ("_ZN2nn2os23SignalConditionVariableE", cv_signal, "releases the oldest waiter"),
        ("_ZN2nn2os26BroadcastConditionVariableE", cv_broadcast, "releases every waiter"),
        ("_ZN2nn2os21WaitConditionVariableE", cv_wait, "releases the mutex, waits for a signal, reacquires it"),
        ("_ZN2nn2os26TimedWaitConditionVariableE", cv_timed_wait, "as Wait, returning Timeout once the deadline passes"),
        ("_ZN2nn2os9WaitEventE", event_wait, "blocks until another thread signals the event"),
        ("_ZN2nn2os14WaitLightEventE", light_event_wait, "blocks until another thread signals the light event"),
        ("_ZN2nn2os16AcquireSemaphoreE", sem_acquire, "blocks while the count is zero"),
        ("_ZN2nn2os15AcquireReadLockE", rw_read, "blocks while a writer holds the lock"),
        ("_ZN2nn2os16AcquireWriteLockE", rw_write, "blocks while any reader or writer holds the lock"),
        ("_ZN2nn2os12CreateThreadE", create_thread, "registers a guest thread on the game's own stack"),
        ("_ZN2nn2os11StartThreadE", start_thread, "makes the thread runnable; the scheduler starts it at its entry"),
        ("_ZN2nn2os10WaitThreadE", wait_thread, "blocks until the thread's function returns"),
        ("_ZN2nn2os13DestroyThreadE", destroy_thread, "waits for the thread to exit, then forgets it"),
        ("_ZN2nn2os13SetThreadNameE", set_thread_name, "records the name pointer"),
        ("_ZN2nn2os20SetThreadNamePointerE", set_thread_name, "records the name pointer"),
        ("_ZN2nn2os16GetCurrentThreadE", current_thread, "the ThreadType of the thread the scheduler is running"),
        ("_ZN2nn2os17GetThreadPriorityE", thread_priority, "the priority given at creation"),
        ("_ZN2nn2os24GetThreadCurrentPriorityE", thread_priority, "the priority given at creation"),
        ("_ZN2nn2os20ChangeThreadPriorityE", change_priority, "records the new priority, returns the old one"),
        ("_ZN2nn2os11YieldThreadE", yield_thread, "lets every other runnable thread run once"),
        ("_ZN2nn2os11SleepThreadE", sleep_thread, "parks the thread until the host clock passes the deadline"),
        ("_ZN2nn2os13GetSystemTickE", system_tick, "monotonic host clock at 19.2 MHz"),
        ("_ZN2nn2os20InitializeTimerEventE", timer_init, "timer state in the caller's object"),
        ("_ZN2nn2os18FinalizeTimerEventE", timer_finalize, "clears the tag"),
        ("_ZN2nn2os23StartPeriodicTimerEventE", timer_start_periodic, "fires at first, then every period, on the host clock"),
        ("_ZN2nn2os22StartOneShotTimerEventE", timer_start_oneshot, "fires once on the host clock"),
        ("_ZN2nn2os14StopTimerEventE", timer_stop, "disarms the timer"),
        ("_ZN2nn2os16SignalTimerEventE", timer_signal, "sets the event"),
        ("_ZN2nn2os15ClearTimerEventE", timer_clear, "clears the event"),
        ("_ZN2nn2os14WaitTimerEventE", timer_wait, "blocks until the timer fires or is signalled"),
        ("_ZN2nn2os17TryWaitTimerEventE", timer_try_wait, "polls the timer"),
    ];
    for (name, handler, claim) in table {
        s.register_prefix(name, *handler, claim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st() -> State {
        let mut s = State::default();
        s.host.thread_object = 0xA000;
        s
    }

    #[test]
    fn a_held_mutex_blocks_another_thread_until_released() {
        let mut s = st();
        s.x[0] = 0x100;
        assert_eq!(mutex_init(&mut s), HostResult::Returned(0));
        s.x[0] = 0x100;
        assert_eq!(mutex_lock(&mut s), HostResult::Returned(0));
        s.host.thread_object = 0xB000;
        s.x[0] = 0x100;
        assert!(matches!(mutex_lock(&mut s), HostResult::Block { timed: false, .. }));
        assert!(matches!(mutex_unlock(&mut s), HostResult::Unavailable(_)), "B does not own it");
        s.host.thread_object = 0xA000;
        s.x[0] = 0x100;
        assert_eq!(mutex_unlock(&mut s), HostResult::Returned(0));
        s.host.thread_object = 0xB000;
        s.x[0] = 0x100;
        assert_eq!(mutex_lock(&mut s), HostResult::Returned(0));
    }

    #[test]
    fn condition_variable_releases_and_reacquires() {
        let mut s = st();
        s.x[0] = 0x200;
        cv_init(&mut s);
        s.x[0] = 0x100;
        mutex_init(&mut s);
        s.x[0] = 0x100;
        mutex_lock(&mut s);
        // Thread A waits: the mutex is released.
        s.x[0] = 0x200;
        s.x[1] = 0x100;
        let HostResult::Block { resume, .. } = cv_wait(&mut s) else { panic!("should wait") };
        let saved = s.x;
        assert_eq!(u32::from_le_bytes(s.mem[0x104..0x108].try_into().unwrap()), 0);
        // Thread B takes the mutex and signals.
        s.host.thread_object = 0xB000;
        s.x[0] = 0x100;
        assert_eq!(mutex_lock(&mut s), HostResult::Returned(0));
        s.x[0] = 0x200;
        cv_signal(&mut s);
        // A is released but must wait for the mutex B still holds.
        s.host.thread_object = 0xA000;
        s.x = saved;
        let HostResult::Block { resume, .. } = resume(&mut s) else { panic!("mutex still held") };
        let saved = s.x;
        s.host.thread_object = 0xB000;
        s.x[0] = 0x100;
        assert_eq!(mutex_unlock(&mut s), HostResult::Returned(0));
        s.host.thread_object = 0xA000;
        s.x = saved;
        assert_eq!(resume(&mut s), HostResult::Returned(CV_NO_TIMEOUT));
        s.x[0] = 0x100;
        assert_eq!(mutex_locked_by_me(&mut s), HostResult::Returned(1));
    }

    #[test]
    fn threads_are_created_started_and_joined() {
        let mut s = st();
        s.mem.resize(0x40000, 0);
        s.x[0] = 0x300;
        s.x[1] = 0x1234;
        s.x[2] = 7;
        s.x[3] = 0x10000;
        s.x[4] = 0x8000;
        s.x[5] = 16;
        assert_eq!(create_thread(&mut s), HostResult::Returned(0));
        s.x[0] = 0x300;
        assert!(matches!(wait_thread(&mut s), HostResult::Unavailable(_)), "never started");
        assert_eq!(start_thread(&mut s), HostResult::Returned(0));
        assert!(matches!(wait_thread(&mut s), HostResult::Block { .. }));
        s.host.threads.get_mut(&0x300).unwrap().exited = true;
        assert_eq!(wait_thread(&mut s), HostResult::Returned(0));
    }

    #[test]
    fn a_one_shot_timer_fires_once() {
        let mut s = st();
        s.x[0] = 0x400;
        s.x[1] = 1; // EventClearMode_AutoClear
        timer_init(&mut s);
        s.x[0] = 0x400;
        s.x[1] = 0;
        timer_start_oneshot(&mut s);
        s.x[0] = 0x400;
        assert_eq!(timer_try_wait(&mut s), HostResult::Returned(1));
        assert_eq!(timer_try_wait(&mut s), HostResult::Returned(0));
    }
}
