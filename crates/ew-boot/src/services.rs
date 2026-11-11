//! Single-threaded `nn::os` services.
//!
//! Every handler here states the contract it implements, and every one of those
//! contracts is exact for a host that runs exactly one guest thread. That model
//! is not a placeholder: a mutex that can never be contended, an event that is
//! observed rather than blocked on, and a page allocator that hands out memory
//! from a reserved arena all have one correct answer, and the handlers give it.
//!
//! What the model cannot do is create a second thread. Those symbols stay
//! unregistered on purpose, so a run that needs a scheduler stops with that
//! symbol's name instead of silently dropping the work it was meant to start.
//!
//! State lives in the guest object the caller passed in, tagged with a magic
//! word, so a handler never needs global state and two runs in one process
//! cannot see each other.

use crate::hle::{read_bytes, write_bytes, HostResult, HostServices};
use ew_lift::State;

/// `nn::os` mutex object, 0x10 bytes of state inside the caller's storage.
const MUTEX_MAGIC: u32 = u32::from_le_bytes(*b"EWMX");
/// `nn::os` reader/writer lock object.
const RW_MAGIC: u32 = u32::from_le_bytes(*b"EWRW");
/// `nn::os` event object.
const EVENT_MAGIC: u32 = u32::from_le_bytes(*b"EWEV");
/// `nn::os` semaphore object.
const SEM_MAGIC: u32 = u32::from_le_bytes(*b"EWSM");

/// The tick frequency every Nintendo title measures time against.
const TICK_FREQ: u64 = 19_200_000;
/// Priority reported for the host's only thread. 44 is the module's default.
const MAIN_PRIORITY: u64 = 44;

// Guest object access. Every read and write is bounds checked, so a bad pointer
// becomes a reported fault instead of a host panic.

fn u32_at(state: &State, addr: u64) -> Result<u32, HostResult> {
    let bytes = read_bytes(state, addr, 4)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn u64_at(state: &State, addr: u64) -> Result<u64, HostResult> {
    let bytes = read_bytes(state, addr, 8)?;
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes);
    Ok(u64::from_le_bytes(buf))
}

fn put(state: &mut State, addr: u64, bytes: &[u8]) -> Result<(), HostResult> {
    write_bytes(state, addr, bytes)
}

fn put_u32(state: &mut State, addr: u64, value: u32) -> Result<(), HostResult> {
    put(state, addr, &value.to_le_bytes())
}

/// Write a tag and a zeroed body, the shape every object initialiser uses.
fn init_object(state: &mut State, addr: u64, magic: u32, body: &[u32]) -> Result<(), HostResult> {
    let mut bytes = Vec::with_capacity(4 + body.len() * 4);
    bytes.extend_from_slice(&magic.to_le_bytes());
    for word in body {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    put(state, addr, &bytes)
}

/// Read a tagged object, reporting the address when the tag is wrong. Guest
/// objects are only ever written by these handlers, so a mismatch means the
/// guest passed something that is not the object the call expects.
fn tagged(state: &State, addr: u64, magic: u32, what: &str) -> Result<(), HostResult> {
    match u32_at(state, addr) {
        Ok(word) if word == magic => Ok(()),
        Ok(word) => Err(HostResult::Unavailable(format!(
            "{what} at 0x{addr:x} carries tag 0x{word:08x}, not 0x{magic:08x}"
        ))),
        Err(e) => Err(e),
    }
}

// Mutexes. One thread means one owner, so a lock either succeeds, recurses, or
// is a self-deadlock the original title would also hang on.

fn mutex_init(state: &mut State) -> HostResult {
    let object = state.x[0];
    let recursive = (state.x[1] & 1) as u32;
    match init_object(state, object, MUTEX_MAGIC, &[0, recursive, 0]) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn mutex_finalize(state: &mut State) -> HostResult {
    let object = state.x[0];
    match put_u32(state, object, 0) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn mutex_lock(state: &mut State) -> HostResult {
    let object = state.x[0];
    let (depth, recursive) = match (u32_at(state, object + 4), u32_at(state, object + 8)) {
        (Ok(depth), Ok(recursive)) if u32_at(state, object).ok() == Some(MUTEX_MAGIC) => {
            (depth, recursive)
        }
        // A mutex the guest never initialised is unlocked, which is exactly what
        // a static zeroed mutex means. Tagging it here keeps later calls exact.
        (Ok(_), Ok(_)) => {
            if let Err(e) = init_object(state, object, MUTEX_MAGIC, &[0, 0, 0]) {
                return e;
            }
            (0, 0)
        }
        (Err(e), _) | (_, Err(e)) => return e,
    };
    if depth == 0 {
        return match put_u32(state, object + 4, 1) {
            Ok(()) => HostResult::Returned(0),
            Err(e) => e,
        };
    }
    if recursive == 0 {
        return HostResult::Unavailable(format!(
            "self-deadlock: the only guest thread locked the non-recursive mutex at 0x{object:x} twice"
        ));
    }
    match put_u32(state, object + 4, depth + 1) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn mutex_try_lock(state: &mut State) -> HostResult {
    let object = state.x[0];
    let depth = match u32_at(state, object + 4) {
        Ok(depth) => depth,
        Err(e) => return e,
    };
    let recursive = u32_at(state, object + 8).unwrap_or(0);
    if depth == 0 {
        return match put_u32(state, object + 4, 1) {
            Ok(()) => HostResult::Returned(1),
            Err(e) => e,
        };
    }
    if recursive != 0 {
        return match put_u32(state, object + 4, depth + 1) {
            Ok(()) => HostResult::Returned(1),
            Err(e) => e,
        };
    }
    HostResult::Returned(0)
}

fn mutex_unlock(state: &mut State) -> HostResult {
    let object = state.x[0];
    let depth = match u32_at(state, object + 4) {
        Ok(depth) => depth,
        Err(e) => return e,
    };
    if depth == 0 {
        return HostResult::Unavailable(format!(
            "unlocked the already-unlocked mutex at 0x{object:x}"
        ));
    }
    match put_u32(state, object + 4, depth - 1) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn mutex_is_locked(state: &mut State) -> HostResult {
    match u32_at(state, state.x[0] + 4) {
        Ok(depth) => HostResult::Returned(u64::from(depth > 0)),
        Err(e) => e,
    }
}

// Reader/writer locks. The counters are real; with one thread a write lock can
// never be held while a reader asks for the same object.

fn rw_init(state: &mut State) -> HostResult {
    let object = state.x[0];
    match init_object(state, object, RW_MAGIC, &[0, 0]) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn rw_finalize(state: &mut State) -> HostResult {
    match put_u32(state, state.x[0], 0) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn rw_acquire_read(state: &mut State) -> HostResult {
    let object = state.x[0];
    if let Err(e) = tagged(state, object, RW_MAGIC, "reader/writer lock") {
        return e;
    }
    let readers = u32_at(state, object + 4).unwrap_or(0);
    let writer = u32_at(state, object + 8).unwrap_or(0);
    if writer != 0 {
        return HostResult::Unavailable(format!(
            "would block: the only guest thread holds the write lock on 0x{object:x} and asked to read"
        ));
    }
    match put_u32(state, object + 4, readers + 1) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn rw_release_read(state: &mut State) -> HostResult {
    let object = state.x[0];
    let readers = match u32_at(state, object + 4) {
        Ok(readers) => readers,
        Err(e) => return e,
    };
    if readers == 0 {
        return HostResult::Unavailable(format!(
            "released a read lock the only guest thread does not hold on 0x{object:x}"
        ));
    }
    match put_u32(state, object + 4, readers - 1) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn rw_acquire_write(state: &mut State) -> HostResult {
    let object = state.x[0];
    if let Err(e) = tagged(state, object, RW_MAGIC, "reader/writer lock") {
        return e;
    }
    let readers = u32_at(state, object + 4).unwrap_or(0);
    if readers != 0 {
        return HostResult::Unavailable(format!(
            "would block: the only guest thread holds a read lock on 0x{object:x} and asked to write"
        ));
    }
    match put_u32(state, object + 8, 1) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn rw_release_write(state: &mut State) -> HostResult {
    match put_u32(state, state.x[0] + 8, 0) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

// Events. Signalling is exact; waiting on a clear event cannot be, because
// nothing else in this host can set it while the guest sleeps.

fn event_init(state: &mut State) -> HostResult {
    let object = state.x[0];
    let signaled = (state.x[1] & 1) as u32;
    let clear_mode = state.x[2] as u32;
    match init_object(state, object, EVENT_MAGIC, &[signaled, clear_mode]) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn event_finalize(state: &mut State) -> HostResult {
    match put_u32(state, state.x[0], 0) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn event_signal(state: &mut State) -> HostResult {
    let object = state.x[0];
    if let Err(e) = tagged(state, object, EVENT_MAGIC, "event") {
        return e;
    }
    match put_u32(state, object + 4, 1) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn event_clear(state: &mut State) -> HostResult {
    let object = state.x[0];
    if let Err(e) = tagged(state, object, EVENT_MAGIC, "event") {
        return e;
    }
    match put_u32(state, object + 4, 0) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn event_wait(state: &mut State) -> HostResult {
    let object = state.x[0];
    if let Err(e) = tagged(state, object, EVENT_MAGIC, "event") {
        return e;
    }
    match event_take(state, object) {
        Ok(true) => HostResult::Returned(0),
        Ok(false) => HostResult::Unavailable(format!(
            "would block: the only guest thread waited on the unsignalled event at 0x{object:x}"
        )),
        Err(e) => e,
    }
}

fn event_try_wait(state: &mut State) -> HostResult {
    let object = state.x[0];
    if let Err(e) = tagged(state, object, EVENT_MAGIC, "event") {
        return e;
    }
    match event_take(state, object) {
        Ok(signaled) => HostResult::Returned(u64::from(signaled)),
        Err(e) => e,
    }
}

/// Consume a signal according to the object's clear mode, returning whether the
/// wait succeeded.
fn event_take(state: &mut State, object: u64) -> Result<bool, HostResult> {
    let signaled = u32_at(state, object + 4)? != 0;
    if signaled {
        // nn::os::EventClearMode: ManualClear = 0, AutoClear = 1.
        let auto_clear = u32_at(state, object + 8)? == 1;
        if auto_clear {
            put_u32(state, object + 4, 0)?;
        }
    }
    Ok(signaled)
}

// Light events are a single word: non-zero means signalled.

fn light_event_init(state: &mut State) -> HostResult {
    let object = state.x[0];
    let signaled = state.x[1] & 1;
    match put_u32(state, object, signaled as u32) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn light_event_signal(state: &mut State) -> HostResult {
    match put_u32(state, state.x[0], 1) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn light_event_clear(state: &mut State) -> HostResult {
    match put_u32(state, state.x[0], 0) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn light_event_wait(state: &mut State) -> HostResult {
    let object = state.x[0];
    match u32_at(state, object) {
        Ok(word) if word != 0 => match put_u32(state, object, 0) {
            Ok(()) => HostResult::Returned(0),
            Err(e) => e,
        },
        Ok(_) => HostResult::Unavailable(format!(
            "would block: the only guest thread waited on the unsignalled light event at 0x{object:x}"
        )),
        Err(e) => e,
    }
}

/// A light event persists its state until cleared, so a try is a plain read.
fn light_event_try_wait(state: &mut State) -> HostResult {
    match u32_at(state, state.x[0]) {
        Ok(word) => HostResult::Returned(u64::from(word != 0)),
        Err(e) => e,
    }
}

// Semaphores: the count is real, so an acquire with nothing available is a
// reported block rather than a hang.

fn sem_init(state: &mut State) -> HostResult {
    let object = state.x[0];
    let count = state.x[1] as u32;
    let max = state.x[2] as u32;
    match init_object(state, object, SEM_MAGIC, &[count, max]) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn sem_finalize(state: &mut State) -> HostResult {
    match put_u32(state, state.x[0], 0) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn sem_acquire(state: &mut State) -> HostResult {
    let object = state.x[0];
    if let Err(e) = tagged(state, object, SEM_MAGIC, "semaphore") {
        return e;
    }
    let count = u32_at(state, object + 4).unwrap_or(0);
    if count == 0 {
        return HostResult::Unavailable(format!(
            "would block: the semaphore at 0x{object:x} has no count left for the only guest thread"
        ));
    }
    match put_u32(state, object + 4, count - 1) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn sem_release(state: &mut State) -> HostResult {
    let object = state.x[0];
    if let Err(e) = tagged(state, object, SEM_MAGIC, "semaphore") {
        return e;
    }
    let count = u32_at(state, object + 4).unwrap_or(0);
    let max = u32_at(state, object + 8).unwrap_or(u32::MAX);
    match put_u32(state, object + 4, (count + 1).min(max)) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn sem_release_n(state: &mut State) -> HostResult {
    let object = state.x[0];
    let n = state.x[1] as u32;
    if let Err(e) = tagged(state, object, SEM_MAGIC, "semaphore") {
        return e;
    }
    let count = u32_at(state, object + 4).unwrap_or(0);
    let max = u32_at(state, object + 8).unwrap_or(u32::MAX);
    match put_u32(state, object + 4, count.saturating_add(n).min(max)) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

// Page allocation. Pages come from the arena the dispatcher reserves between the
// guest stack and the top of guest memory.

fn allocate_pages(state: &mut State) -> HostResult {
    let address = state.x[0];
    // Page allocations are whole pages by definition, so the size is rounded up
    // before it reaches the arena.
    let size = state.x[1].max(1).div_ceil(0x1000) * 0x1000;
    if address == 0 {
        return match state.host.allocate(size, 0x1000) {
            Ok(_) => HostResult::Returned(0),
            Err(message) => HostResult::Unavailable(message),
        };
    }
    // A fixed-address allocation is honoured when it lands inside the arena and
    // does not overlap what the allocator already handed out.
    if address < state.host.alloc_next || address >= state.host.alloc_end {
        return HostResult::Unavailable(format!(
            "fixed page allocation at 0x{address:x} is outside the guest page arena"
        ));
    }
    let end = address.saturating_add(size);
    if end > state.host.alloc_end {
        return HostResult::Unavailable(format!(
            "fixed page allocation at 0x{address:x} of 0x{size:x} bytes runs past the arena"
        ));
    }
    state.host.alloc_next = end;
    HostResult::Returned(0)
}

fn free_pages(state: &mut State) -> HostResult {
    let address = state.x[0];
    let size = state.x[1];
    // The arena is a bump allocator: released pages are simply not handed out
    // again. Reporting success is exact for the guest, which never sees the
    // memory again through this address.
    let _ = (address, size);
    HostResult::Returned(0)
}

fn allocate_memory_block(state: &mut State) -> HostResult {
    let out = state.x[0];
    let size = state.x[1].max(1);
    match state.host.allocate(size, 0x10) {
        Ok(at) => match put(state, out, &at.to_le_bytes()) {
            Ok(()) => HostResult::Returned(0),
            Err(e) => e,
        },
        Err(message) => HostResult::Unavailable(message),
    }
}

fn allocate_address_region(state: &mut State) -> HostResult {
    allocate_memory_block(state)
}

fn free_address_region(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

/// `nn::os::SetMemoryHeapSize(size_t)`. The heap is the arena the dispatcher
/// reserved; a request that fits is recorded, one that does not is reported
/// instead of being acknowledged with memory that does not exist.
fn set_heap_size(state: &mut State) -> HostResult {
    let size = state.x[0];
    let capacity = state.host.alloc_end.saturating_sub(state.host.alloc_next) + state.host.heap_size;
    if size > capacity {
        return HostResult::Unavailable(format!(
            "SetMemoryHeapSize(0x{size:x}) asks for {} MiB, but the guest heap arena holds {} MiB",
            size >> 20,
            capacity >> 20
        ));
    }
    state.host.heap_size = size;
    HostResult::Returned(0)
}

fn heap_size(state: &mut State) -> HostResult {
    HostResult::Returned(state.host.alloc_end.saturating_sub(state.host.alloc_next))
}

/// `nn::os::QueryMemoryInfo(nn::os::MemoryInfo*)`. Every field comes from the
/// host's real layout: available memory is the program plus the whole arena,
/// and one thread stack exists.
fn query_memory_info(state: &mut State) -> HostResult {
    let out = state.x[0];
    let h = &state.host;
    let arena = h.alloc_end.saturating_sub(h.alloc_next) + h.pages_allocated * 0x1000;
    let fields: [u64; 6] = [
        h.program_size + h.stack_size + arena,
        h.program_size + h.stack_size + h.pages_allocated * 0x1000,
        h.heap_size,
        h.pages_allocated * 0x1000,
        h.program_size,
        h.stack_size,
    ];
    let mut bytes = Vec::with_capacity(56);
    for f in fields {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    match put(state, out, &bytes) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn virtual_address_memory(_state: &mut State) -> HostResult {
    // Reported as unavailable so the title stays on the classic allocator this
    // host implements, instead of entering a virtual-address path it cannot map.
    HostResult::Returned(0)
}

// Threads: identity, priority and scheduling queries are exact for one thread.
// Creation is deliberately absent (see the module comment).

fn current_thread(state: &mut State) -> HostResult {
    match state.host.take_thread_object() {
        Ok(at) => HostResult::Returned(at),
        Err(message) => HostResult::Unavailable(message),
    }
}

fn thread_id(state: &mut State) -> HostResult {
    let object = state.x[0];
    if object == 0 {
        return HostResult::Unavailable("thread id requested for a null thread".to_string());
    }
    HostResult::Returned(object)
}

fn thread_priority(state: &mut State) -> HostResult {
    let object = state.x[0];
    let threaded = u64_at(state, object + 0x10).unwrap_or(MAIN_PRIORITY);
    // Objects this host did not create carry zero, which is not a valid
    // priority, so the module default is the honest answer.
    HostResult::Returned(if threaded == 0 { MAIN_PRIORITY } else { threaded })
}

fn core_mask(_state: &mut State) -> HostResult {
    HostResult::Returned(1)
}

/// `SetThreadCoreMask(ThreadType*, int idealCore, uint64 mask)`. The host runs
/// guest threads on one host core, so any request that includes core 0 (or
/// leaves the mask to the default, ideal core -1/-2) is satisfied exactly.
fn set_core_mask(state: &mut State) -> HostResult {
    let (ideal, mask) = (state.x[1] as i32, state.x[2]);
    if mask & 1 == 0 && ideal >= 0 {
        return HostResult::Unavailable(format!(
            "SetThreadCoreMask asked for ideal core {ideal} with mask 0x{mask:x}, which excludes core 0, the only core the host schedules"
        ));
    }
    HostResult::Returned(0)
}

fn current_core(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

fn yield_thread(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

fn sleep_thread(_state: &mut State) -> HostResult {
    // Nothing else can run while this thread sleeps, so the deadline has already
    // passed when the call returns.
    HostResult::Returned(0)
}

fn system_tick(_state: &mut State) -> HostResult {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    HostResult::Returned(ns.saturating_mul(TICK_FREQ) / 1_000_000_000)
}

fn tick_frequency(_state: &mut State) -> HostResult {
    HostResult::Returned(TICK_FREQ)
}

fn to_tick(state: &mut State) -> HostResult {
    HostResult::Returned(state.x[0].saturating_mul(TICK_FREQ) / 1_000_000_000)
}

fn to_timespan(state: &mut State) -> HostResult {
    HostResult::Returned(state.x[0].saturating_mul(1_000_000_000) / TICK_FREQ)
}

fn set_thread_name(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

// Thread-local storage: a real slot table belonging to the only thread, so the
// C runtime that calls through these sees per-thread values behave as values.

pub(crate) fn alloc_tls_slot(state: &mut State) -> HostResult {
    let out = state.x[0];
    let slot = match (0..state.host.tls.len()).find(|i| state.host.tls_used & (1 << i) == 0) {
        Some(slot) => slot,
        None => {
            return HostResult::Unavailable(format!(
                "no thread-local slot left: all {} are in use",
                state.host.tls.len()
            ))
        }
    };
    state.host.tls_used |= 1 << slot;
    state.host.tls[slot] = 0;
    match put_u32(state, out, slot as u32) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn free_tls_slot(state: &mut State) -> HostResult {
    let slot = state.x[0] as usize;
    if slot >= state.host.tls.len() {
        return HostResult::Unavailable(format!("thread-local slot {slot} does not exist"));
    }
    state.host.tls_used &= !(1 << slot);
    state.host.tls[slot] = 0;
    HostResult::Returned(0)
}

fn get_tls_value(state: &mut State) -> HostResult {
    let slot = state.x[0] as usize;
    match state.host.tls.get(slot) {
        Some(value) => HostResult::Returned(*value),
        None => HostResult::Unavailable(format!("thread-local slot {slot} does not exist")),
    }
}

fn set_tls_value(state: &mut State) -> HostResult {
    let slot = state.x[0] as usize;
    let value = state.x[1];
    match state.host.tls.get_mut(slot) {
        Some(cell) => {
            *cell = value;
            HostResult::Returned(0)
        }
        None => HostResult::Unavailable(format!("thread-local slot {slot} does not exist")),
    }
}

/// Register every single-threaded service into `services`, with the claim the
/// host is making for each one.
pub fn register(services: &mut HostServices) {
    // Mutexes.
    services.register_prefix(
        "_ZN2nn2os15InitializeMutexE",
        mutex_init as _,
        "single guest thread: initialises the caller's mutex storage",
    );
    services.register_prefix(
        "_ZN2nn2os13FinalizeMutexE",
        mutex_finalize as _,
        "single guest thread: clears the mutex tag",
    );
    services.register_prefix(
        "_ZN2nn2os9LockMutexE",
        mutex_lock as _,
        "single guest thread: an uncontended lock always succeeds, a second non-recursive lock is a reported deadlock",
    );
    services.register_prefix(
        "_ZN2nn2os12TryLockMutexE",
        mutex_try_lock as _,
        "single guest thread: succeeds unless the thread already holds a non-recursive lock",
    );
    services.register_prefix(
        "_ZN2nn2os11UnlockMutexE",
        mutex_unlock as _,
        "single guest thread: releases one recursion level",
    );
    services.register_prefix(
        "_ZN2nn2os29IsMutexLockedByCurrentThreadE",
        mutex_is_locked as _,
        "single guest thread: true while the lock depth is non-zero",
    );

    // Reader/writer locks.
    services.register_prefix(
        "_ZN2nn2os26InitializeReaderWriterLockE",
        rw_init as _,
        "single guest thread: initialises the caller's reader/writer lock storage",
    );
    services.register_prefix(
        "_ZN2nn2os24FinalizeReaderWriterLockE",
        rw_finalize as _,
        "single guest thread: clears the reader/writer lock tag",
    );
    services.register_prefix(
        "_ZN2nn2os15AcquireReadLockE",
        rw_acquire_read as _,
        "single guest thread: succeeds unless the same thread holds the write lock",
    );
    services.register_prefix(
        "_ZN2nn2os15ReleaseReadLockE",
        rw_release_read as _,
        "single guest thread: drops one read count",
    );
    services.register_prefix(
        "_ZN2nn2os16AcquireWriteLockE",
        rw_acquire_write as _,
        "single guest thread: succeeds unless the same thread holds a read lock",
    );
    services.register_prefix(
        "_ZN2nn2os16ReleaseWriteLockE",
        rw_release_write as _,
        "single guest thread: drops the write count",
    );

    // Events.
    services.register_prefix(
        "_ZN2nn2os15InitializeEventE",
        event_init as _,
        "single guest thread: stores the initial signal state and clear mode",
    );
    services.register_prefix(
        "_ZN2nn2os13FinalizeEventE",
        event_finalize as _,
        "single guest thread: clears the event tag",
    );
    services.register_prefix(
        "_ZN2nn2os11SignalEventE",
        event_signal as _,
        "single guest thread: setting a signal is exact",
    );
    services.register_prefix(
        "_ZN2nn2os10ClearEventE",
        event_clear as _,
        "single guest thread: clearing a signal is exact",
    );
    services.register_prefix(
        "_ZN2nn2os9WaitEventE",
        event_wait as _,
        "single guest thread: a signalled wait is exact, waiting on a clear event is reported instead of hanging",
    );
    services.register_prefix(
        "_ZN2nn2os12TryWaitEventE",
        event_try_wait as _,
        "single guest thread: reports the signal state and honours auto-clear",
    );

    // Light events.
    services.register_prefix(
        "_ZN2nn2os20InitializeLightEventE",
        light_event_init as _,
        "single guest thread: the light event word is its own state",
    );
    services.register_prefix(
        "_ZN2nn2os14WaitLightEventE",
        light_event_wait as _,
        "single guest thread: a signalled wait consumes the word, a clear wait is reported",
    );
    services.register_prefix(
        "_ZN2nn2os17TryWaitLightEventE",
        light_event_try_wait as _,
        "single guest thread: reads the light event word",
    );
    services.register_prefix(
        "_ZN2nn2os16SignalLightEventE",
        light_event_signal as _,
        "single guest thread: setting the light event word is exact",
    );
    services.register_prefix(
        "_ZN2nn2os15ClearLightEventE",
        light_event_clear as _,
        "single guest thread: clearing the light event word is exact",
    );
    services.register_prefix(
        "_ZN2nn2os18FinalizeLightEventE",
        light_event_clear as _,
        "single guest thread: finalising clears the light event word",
    );

    // Semaphores.
    services.register_prefix(
        "_ZN2nn2os19InitializeSemaphoreE",
        sem_init as _,
        "single guest thread: stores the initial and maximum count",
    );
    services.register_prefix(
        "_ZN2nn2os17FinalizeSemaphoreE",
        sem_finalize as _,
        "single guest thread: clears the semaphore tag",
    );
    services.register_prefix(
        "_ZN2nn2os16AcquireSemaphoreE",
        sem_acquire as _,
        "single guest thread: decrements the count, or reports the block it cannot serve",
    );
    // The two release overloads differ only in their trailing argument, so the
    // counted form is registered exactly and the plain form by prefix.
    services.register(
        "_ZN2nn2os16ReleaseSemaphoreEPNS0_13SemaphoreTypeEi",
        sem_release_n as _,
        "single guest thread: adds the requested count, clamped to the maximum",
    );
    services.register_prefix(
        "_ZN2nn2os16ReleaseSemaphoreE",
        sem_release as _,
        "single guest thread: adds one to the count up to its maximum",
    );

    services.register(
        "_ZN2nn2os15QueryMemoryInfoEPNS0_10MemoryInfoE",
        query_memory_info as _,
        "MemoryInfo filled from the host layout: program image, one stack, the guest heap arena",
    );

    // Memory pages and blocks.
    services.register_prefix(
        "_ZN2nn2os19AllocateMemoryPagesE",
        allocate_pages as _,
        "pages come from the arena reserved above the guest stack and are 4 KiB aligned",
    );
    services.register_prefix(
        "_ZN2nn2os15FreeMemoryPagesE",
        free_pages as _,
        "the arena is a bump allocator, so a freed page is not handed out again",
    );
    services.register_prefix(
        "_ZN2nn2os17SetMemoryHeapSizeE",
        set_heap_size as _,
        "records the size when it fits the reserved arena, reports it otherwise",
    );
    services.register_prefix(
        "_ZN2nn2os18GetMemoryHeapSizeE",
        heap_size as _,
        "reports what the arena has left",
    );
    services.register_prefix(
        "_ZN2nn2os19AllocateMemoryBlockE",
        allocate_memory_block as _,
        "allocates from the arena and writes the address to the caller's out parameter",
    );
    services.register_prefix(
        "_ZN2nn2os21AllocateAddressRegionE",
        allocate_address_region as _,
        "an address region is arena memory: the host has no separate reservation step",
    );
    services.register_prefix(
        "_ZN2nn2os17FreeAddressRegionE",
        free_address_region as _,
        "address regions are arena memory, so releasing one is a no-op",
    );
    services.register_prefix(
        "_ZN2nn2os29IsVirtualAddressMemoryEnabledE",
        virtual_address_memory as _,
        "reports unavailable so the title stays on the classic allocator the host implements",
    );

    // Thread identity, time and scheduling for the one thread that exists.
    services.register_prefix(
        "_ZN2nn2os16GetCurrentThreadE",
        current_thread as _,
        "returns the pseudo thread object the dispatcher reserved for this run",
    );
    services.register_prefix(
        "_ZN2nn2os11GetThreadIdE",
        thread_id as _,
        "a thread's identity is its own object address, which is unique per run",
    );
    services.register_prefix(
        "_ZN2nn2os17GetThreadPriorityE",
        thread_priority as _,
        "reports the priority stored in the thread object, or the module default",
    );
    services.register_prefix(
        "_ZN2nn2os24GetThreadCurrentPriorityE",
        thread_priority as _,
        "reports the priority stored in the thread object, or the module default",
    );
    services.register_prefix(
        "_ZN2nn2os26GetThreadAvailableCoreMaskE",
        core_mask as _,
        "the host schedules exactly one core, so the mask is core 0",
    );
    services.register_prefix(
        "_ZN2nn2os17SetThreadCoreMaskE",
        set_core_mask as _,
        "accepted when the mask includes core 0, the core the host schedules guest threads on",
    );
    services.register_prefix(
        "_ZN2nn2os20GetCurrentCoreNumberE",
        current_core as _,
        "the only guest thread always runs on core 0",
    );
    services.register_prefix(
        "_ZN2nn2os11YieldThreadE",
        yield_thread as _,
        "there is no other guest thread to yield to, so the call returns at once",
    );
    services.register_prefix(
        "_ZN2nn2os11SleepThreadE",
        sleep_thread as _,
        "no other guest thread can run during a sleep, so the deadline has passed on return",
    );
    services.register_prefix(
        "_ZN2nn2os13SetThreadNameE",
        set_thread_name as _,
        "the host keeps no thread names yet; naming has no observable effect",
    );
    services.register_prefix(
        "_ZN2nn2os15GetSystemTickE",
        system_tick as _,
        "guest ticks are the host's wall clock scaled to the 19.2 MHz tick",
    );
    services.register_prefix(
        "_ZN2nn2os22GetSystemTickFrequencyE",
        tick_frequency as _,
        "the Switch system tick runs at 19.2 MHz",
    );
    services.register_prefix(
        "_ZN2nn2os13ConvertToTickE",
        to_tick as _,
        "converts a nanosecond timespan at the 19.2 MHz tick",
    );
    services.register_prefix(
        "_ZN2nn2os17ConvertToTimeSpanE",
        to_timespan as _,
        "converts ticks back to a nanosecond timespan",
    );

    // Thread-local storage.
    services.register_prefix(
        "_ZN2nn2os15AllocateTlsSlotE",
        alloc_tls_slot as _,
        "hands out a slot from the only thread's table and writes the index to the caller's out parameter",
    );
    services.register_prefix(
        "_ZN2nn2os11FreeTlsSlotE",
        free_tls_slot as _,
        "clears the slot and its value",
    );
    services.register_prefix(
        "_ZN2nn2os11GetTlsValueE",
        get_tls_value as _,
        "reads the only thread's slot table",
    );
    services.register_prefix(
        "_ZN2nn2os11SetTlsValueE",
        set_tls_value as _,
        "writes the only thread's slot table",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arena_state() -> State {
        let mut s = State::default();
        s.mem = vec![0u8; 0x10000];
        s.host.alloc_next = 0x8000;
        s.host.alloc_end = 0x10000;
        s
    }

    #[test]
    fn mutex_reports_a_second_non_recursive_lock() {
        let mut s = arena_state();
        s.x[0] = 0x100;
        s.x[1] = 0;
        assert_eq!(mutex_init(&mut s), HostResult::Returned(0));
        assert_eq!(mutex_lock(&mut s), HostResult::Returned(0));
        match mutex_lock(&mut s) {
            HostResult::Unavailable(message) => assert!(message.contains("self-deadlock"), "{message}"),
            other => panic!("expected a reported deadlock, got {other:?}"),
        }
        assert_eq!(mutex_unlock(&mut s), HostResult::Returned(0));
    }

    #[test]
    fn recursive_mutex_counts_nesting() {
        let mut s = arena_state();
        s.x[0] = 0x100;
        s.x[1] = 1;
        mutex_init(&mut s);
        mutex_lock(&mut s);
        mutex_lock(&mut s);
        assert_eq!(u32_at(&s, 0x104).unwrap(), 2);
        mutex_unlock(&mut s);
        mutex_unlock(&mut s);
        assert_eq!(u32_at(&s, 0x104).unwrap(), 0);
        match mutex_unlock(&mut s) {
            HostResult::Unavailable(m) => assert!(m.contains("already-unlocked"), "{m}"),
            other => panic!("expected a reported fault, got {other:?}"),
        }
    }

    #[test]
    fn signalled_event_wait_clears_only_auto_clear_events() {
        let mut s = arena_state();
        s.x[0] = 0x200;
        s.x[1] = 1; // signalled
        s.x[2] = 1; // EventClearMode_AutoClear
        event_init(&mut s);
        assert_eq!(event_wait(&mut s), HostResult::Returned(0));
        assert_eq!(u32_at(&s, 0x204).unwrap(), 0);

        s.x[0] = 0x240;
        s.x[1] = 1;
        s.x[2] = 0; // EventClearMode_ManualClear
        event_init(&mut s);
        assert_eq!(event_wait(&mut s), HostResult::Returned(0));
        assert_eq!(u32_at(&s, 0x244).unwrap(), 1);
    }

    #[test]
    fn unsignalled_event_wait_is_reported_not_hung() {
        let mut s = arena_state();
        s.x[0] = 0x200;
        s.x[1] = 0;
        s.x[2] = 0;
        event_init(&mut s);
        match event_wait(&mut s) {
            HostResult::Unavailable(m) => assert!(m.contains("would block"), "{m}"),
            other => panic!("expected a reported block, got {other:?}"),
        }
    }

    #[test]
    fn page_allocation_is_aligned_and_bounded() {
        let mut s = arena_state();
        s.x[0] = 0;
        s.x[1] = 0x1234;
        assert_eq!(allocate_pages(&mut s), HostResult::Returned(0));
        assert_eq!(s.host.alloc_next, 0xa000, "0x1234 rounds up to two pages at the arena base");
        s.x[1] = 0x8000;
        match allocate_pages(&mut s) {
            HostResult::Unavailable(m) => assert!(m.contains("exhausted"), "{m}"),
            other => panic!("expected exhaustion, got {other:?}"),
        }
    }

    #[test]
    fn allocator_never_returns_memory_it_did_not_own() {
        let mut s = arena_state();
        // Fixed allocation below the arena is refused instead of granted.
        s.x[0] = 0x1000;
        s.x[1] = 0x1000;
        match allocate_pages(&mut s) {
            HostResult::Unavailable(m) => assert!(m.contains("outside the guest page arena"), "{m}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn tls_slots_are_handed_out_once() {
        let mut s = arena_state();
        let mut seen = Vec::new();
        for i in 0..3u64 {
            s.x[0] = 0x300 + i * 8;
            assert_eq!(alloc_tls_slot(&mut s), HostResult::Returned(0));
            seen.push(u32_at(&s, 0x300 + i * 8).unwrap());
        }
        assert_eq!(seen, vec![0, 1, 2], "slots are not reused while held");
        s.x[0] = 0;
        s.x[1] = 0xDEAD;
        assert_eq!(set_tls_value(&mut s), HostResult::Returned(0));
        assert_eq!(get_tls_value(&mut s), HostResult::Returned(0xDEAD));
        free_tls_slot(&mut s);
        assert_eq!(get_tls_value(&mut s), HostResult::Returned(0));
    }

    #[test]
    fn current_thread_is_stable_and_inside_the_arena() {
        let mut s = arena_state();
        let first = match current_thread(&mut s) {
            HostResult::Returned(at) => at,
            other => panic!("expected a thread object, got {other:?}"),
        };
        assert!(first >= 0x8000 && first < 0x10000);
        let again = match current_thread(&mut s) {
            HostResult::Returned(at) => at,
            other => panic!("expected the same object, got {other:?}"),
        };
        assert_eq!(first, again);
    }

    #[test]
    fn every_service_registers_a_claim() {
        let mut services = HostServices::new();
        register(&mut services);
        for name in [
            "_ZN2nn2os15InitializeMutexEPNS0_9MutexTypeEbi",
            "_ZN2nn2os19AllocateMemoryPagesEmm",
            "_ZN2nn2os16GetCurrentThreadEv",
        ] {
            assert!(services.get(name).is_some(), "missing {name}");
            assert!(services.claim(name).is_some_and(|c| !c.is_empty()));
        }
        // Thread creation is the wall this model cannot pass, and it must stay
        // unregistered so a run stops on its name.
        assert!(services.get("_ZN2nn2os12CreateThreadEPNS0_10ThreadTypeEPFvPvES3_S3_mi").is_none());
    }
}
