//! NVN memory pools, buffers, syncs and queue control on the host.
//!
//! NVN memory is guest memory: a pool's storage is a range the game allocated,
//! and the GPU reads it in place. The host renderer reads the same guest bytes
//! when it executes work, so a storage-backed pool's GPU address is its guest
//! address and map/flush/invalidate need no copies. A virtual pool (no storage)
//! gets addresses from a range above guest memory that nothing can map.
//!
//! Work submitted to a queue completes before the submitting call returns,
//! which is why a fence is signalled as soon as it is placed.

use crate::hle::{HostResult, HostServices};
use crate::nvn::{called, field, set_field};
use ew_lift::State;
use std::collections::BTreeMap;

const NVN_TRUE: u64 = 1;

/// Base of the GPU address range for pools without storage.
const VIRTUAL_BASE: u64 = 0x80_0000_0000;

#[derive(Default)]
pub struct Pool {
    pub storage: u64,
    pub size: u64,
    pub flags: u64,
    pub gpu: u64,
}

#[derive(Default)]
pub struct Memory {
    pub pools: BTreeMap<u64, Pool>,
    pub next_virtual: u64,
    /// Buffers: object to (pool, offset, size).
    pub buffers: BTreeMap<u64, (u64, u64, u64)>,
    /// Syncs that a queue has signalled.
    pub signalled: BTreeMap<u64, bool>,
}

fn mem<R>(state: &State, f: impl FnOnce(&mut Memory) -> R) -> R {
    state.host.ext.with::<Memory, R>(f)
}

/// Arguments a builder setter recorded (see `nvn::record_state`).
fn recorded_args(state: &State, obj: u64, setter: &str) -> Option<Vec<u64>> {
    state.host.ext.with::<crate::nvn::Nvn, _>(|n| n.state.get(&obj).and_then(|m| m.get(setter)).cloned())
}

/// `nvnMemoryPoolInitialize(NVNmemoryPool*, const NVNmemoryPoolBuilder*)`.
fn pool_initialize(state: &mut State) -> HostResult {
    let (pool, b) = (state.x[0], state.x[1]);
    let Some(storage) = recorded_args(state, b, "nvnMemoryPoolBuilderSetStorage") else {
        return HostResult::Unavailable("nvnMemoryPoolInitialize with no storage or size set".into());
    };
    let (ptr, size) = (storage[0], storage[1]);
    let flags = recorded_args(state, b, "nvnMemoryPoolBuilderSetFlags").map(|a| a[0]).unwrap_or(0);
    if ptr != 0 && ptr.saturating_add(size) > state.mem.len() as u64 {
        return HostResult::Unavailable(format!(
            "nvnMemoryPoolInitialize storage 0x{ptr:x}+0x{size:x} is outside guest memory"
        ));
    }
    let gpu = mem(state, |m| {
        let gpu = if ptr != 0 {
            ptr
        } else {
            let at = VIRTUAL_BASE + m.next_virtual;
            m.next_virtual += size.div_ceil(0x1_0000) * 0x1_0000;
            at
        };
        m.pools.insert(pool, Pool { storage: ptr, size, flags, gpu });
        gpu
    });
    state.host.log.push(format!(
        "gfx: memory pool 0x{pool:x}: 0x{size:x} bytes, flags 0x{flags:x}, GPU address 0x{gpu:x}"
    ));
    HostResult::Returned(NVN_TRUE)
}

fn pool_query(state: &mut State, f: impl FnOnce(&Pool) -> u64) -> HostResult {
    let pool = state.x[0];
    match mem(state, |m| m.pools.get(&pool).map(f)) {
        Some(v) => HostResult::Returned(v),
        None => HostResult::Unavailable(format!("{} on memory pool 0x{pool:x}, which was never initialized", called(state))),
    }
}

fn pool_get_buffer_address(state: &mut State) -> HostResult {
    pool_query(state, |p| p.gpu)
}

fn pool_map(state: &mut State) -> HostResult {
    pool_query(state, |p| p.storage)
}

fn pool_get_size(state: &mut State) -> HostResult {
    pool_query(state, |p| p.size)
}

fn pool_get_flags(state: &mut State) -> HostResult {
    pool_query(state, |p| p.flags)
}

fn pool_finalize(state: &mut State) -> HostResult {
    let pool = state.x[0];
    mem(state, |m| m.pools.remove(&pool));
    HostResult::Returned(0)
}

/// Flushes and invalidates: the host reads guest memory in place, so caches
/// are already coherent.
fn coherent(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

/// `nvnBufferInitialize(NVNbuffer*, const NVNbufferBuilder*)`.
fn buffer_initialize(state: &mut State) -> HostResult {
    let (buf, b) = (state.x[0], state.x[1]);
    let Some(st) = recorded_args(state, b, "nvnBufferBuilderSetStorage") else {
        return HostResult::Unavailable("nvnBufferInitialize with no storage set".into());
    };
    let (pool, offset, size) = (st[0], st[1], st[2]);
    let known = mem(state, |m| {
        let known = m.pools.contains_key(&pool);
        if known {
            m.buffers.insert(buf, (pool, offset, size));
        }
        known
    });
    if !known {
        return HostResult::Unavailable(format!("nvnBufferInitialize on memory pool 0x{pool:x}, which was never initialized"));
    }
    HostResult::Returned(NVN_TRUE)
}

fn buffer_query(state: &mut State, f: impl FnOnce(&Pool, u64, u64, u64) -> u64) -> HostResult {
    let buf = state.x[0];
    let v = mem(state, |m| {
        let (pool, offset, size) = *m.buffers.get(&buf)?;
        m.pools.get(&pool).map(|p| f(p, pool, offset, size))
    });
    match v {
        Some(v) => HostResult::Returned(v),
        None => HostResult::Unavailable(format!("{} on buffer 0x{buf:x}, which was never initialized", called(state))),
    }
}

fn buffer_get_address(state: &mut State) -> HostResult {
    buffer_query(state, |p, _, off, _| p.gpu + off)
}

fn buffer_map(state: &mut State) -> HostResult {
    buffer_query(state, |p, _, off, _| if p.storage == 0 { 0 } else { p.storage + off })
}

fn buffer_get_size(state: &mut State) -> HostResult {
    buffer_query(state, |_, _, _, size| size)
}

fn buffer_get_memory_pool(state: &mut State) -> HostResult {
    buffer_query(state, |_, pool, _, _| pool)
}

fn buffer_get_memory_offset(state: &mut State) -> HostResult {
    buffer_query(state, |_, _, off, _| off)
}

fn buffer_finalize(state: &mut State) -> HostResult {
    let buf = state.x[0];
    mem(state, |m| m.buffers.remove(&buf));
    HostResult::Returned(0)
}

/// `nvnSyncInitialize(NVNsync*, NVNdevice*)`.
fn sync_initialize(state: &mut State) -> HostResult {
    let sync = state.x[0];
    mem(state, |m| m.signalled.insert(sync, false));
    HostResult::Returned(NVN_TRUE)
}

fn sync_finalize(state: &mut State) -> HostResult {
    let sync = state.x[0];
    mem(state, |m| m.signalled.remove(&sync));
    HostResult::Returned(0)
}

/// `nvnQueueFenceSync(NVNqueue*, NVNsync*, condition, flags)`: work before the
/// fence has already completed on the host, so the fence is signalled now.
fn queue_fence_sync(state: &mut State) -> HostResult {
    let sync = state.x[1];
    mem(state, |m| m.signalled.insert(sync, true));
    HostResult::Returned(0)
}

/// NVN wait results.
const WAIT_ALREADY_SIGNALED: u64 = 0;
const WAIT_TIMEOUT_EXPIRED: u64 = 2;

/// `nvnSyncWait(const NVNsync*, uint64 timeoutNs)`. A sync no queue has
/// fenced can never signal, so the wait reports its timeout.
fn sync_wait(state: &mut State) -> HostResult {
    let sync = state.x[0];
    let done = mem(state, |m| m.signalled.get(&sync).copied().unwrap_or(false));
    HostResult::Returned(if done { WAIT_ALREADY_SIGNALED } else { WAIT_TIMEOUT_EXPIRED })
}

/// `nvnQueueWaitSync(NVNqueue*, const NVNsync*)`: the queue is in order and
/// synchronous, so a wait on a fence is already satisfied.
fn queue_wait_sync(_state: &mut State) -> HostResult {
    HostResult::Returned(NVN_TRUE)
}

/// `nvnQueueFlush` / `nvnQueueFinish`: submitted work has executed.
fn queue_flush(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

/// `nvnQueueGetError(NVNqueue*, NVNqueueErrorInfo*)`: no GPU error occurred.
fn queue_get_error(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

fn queue_finalize(state: &mut State) -> HostResult {
    let q = state.x[0];
    set_field(state, q, "live", 0);
    HostResult::Returned(0)
}

/// Memory-usage counters: the host queue keeps its commands on the host and
/// uses none of the guest memory the game gave it.
fn zero(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

pub fn register(s: &mut HostServices) {
    let _ = field;
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("nvnMemoryPoolInitialize", pool_initialize, "a pool over the game's guest storage, GPU address = guest address"),
        ("nvnMemoryPoolGetBufferAddress", pool_get_buffer_address, "the pool's GPU address"),
        ("nvnMemoryPoolMap", pool_map, "the pool's guest storage"),
        ("nvnMemoryPoolGetSize", pool_get_size, "the size the pool was created with"),
        ("nvnMemoryPoolGetFlags", pool_get_flags, "the flags the pool was created with"),
        ("nvnMemoryPoolFlushMappedRange", coherent, "guest memory is read in place, so it is coherent"),
        ("nvnMemoryPoolInvalidateMappedRange", coherent, "guest memory is read in place, so it is coherent"),
        ("nvnMemoryPoolFinalize", pool_finalize, "forgets the pool"),
        ("nvnBufferInitialize", buffer_initialize, "a range of an initialized pool"),
        ("nvnBufferGetAddress", buffer_get_address, "pool GPU address plus offset"),
        ("nvnBufferMap", buffer_map, "pool guest storage plus offset"),
        ("nvnBufferGetSize", buffer_get_size, "the buffer's size"),
        ("nvnBufferGetMemoryPool", buffer_get_memory_pool, "the buffer's pool"),
        ("nvnBufferGetMemoryOffset", buffer_get_memory_offset, "the buffer's offset in its pool"),
        ("nvnBufferFlushMappedRange", coherent, "guest memory is read in place, so it is coherent"),
        ("nvnBufferInvalidateMappedRange", coherent, "guest memory is read in place, so it is coherent"),
        ("nvnBufferFinalize", buffer_finalize, "forgets the buffer"),
        ("nvnSyncInitialize", sync_initialize, "a host fence"),
        ("nvnSyncFinalize", sync_finalize, "forgets the fence"),
        ("nvnQueueFenceSync", queue_fence_sync, "signalled at once: queue work completes before the call returns"),
        ("nvnSyncWait", sync_wait, "already signalled when fenced, timeout otherwise"),
        ("nvnQueueWaitSync", queue_wait_sync, "the host queue is in order and synchronous"),
        ("nvnQueueFlush", queue_flush, "submitted work has executed"),
        ("nvnQueueFinish", queue_flush, "submitted work has executed"),
        ("nvnQueueGetError", queue_get_error, "no GPU error has occurred"),
        ("nvnQueueFinalize", queue_finalize, "forgets the queue"),
        ("nvnQueueGetTotalCommandMemoryUsed", zero, "host queues use none of the game's queue memory"),
        ("nvnQueueGetTotalControlMemoryUsed", zero, "host queues use none of the game's queue memory"),
        ("nvnQueueGetTotalComputeMemoryUsed", zero, "host queues use none of the game's queue memory"),
        ("nvnQueueResetMemoryUsageCounts", zero, "nothing to reset"),
    ];
    for (name, handler, claim) in table {
        s.register(&format!("nvn:{name}"), *handler, claim);
    }
}
