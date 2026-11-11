//! `nn::mem::StandardAllocator`, the legacy nnSdk heap interface.
//!
//! The game hands each allocator object a heap region and expects pointers
//! back. The properties callers can observe are kept: returns are aligned, live
//! blocks never overlap, `GetSizeOf` reports what was asked for, freed memory is
//! reused, and a heap never hands out memory outside the region it was given.
//!
//! Every allocator object is its own heap, keyed by its address. A title
//! creates several (a main heap, then sub-heaps carved out of it), so one
//! shared heap would let a later `Initialize` overwrite an earlier one.
//!
//! **Assumption, stated plainly:** this is a first-fit free list, not nnSdk's
//! dlmalloc. Block headers live out of band on the host, so a program that
//! reads the allocator's own metadata out of the heap would see nothing where
//! nnSdk puts its headers. Everything the game can do through the API behaves
//! as specified.
//!
//! A zero size returns a null pointer, as the interface specifies.
//!
//! One boot-order fact shapes the rest of this module. The title allocates
//! through a **static** `StandardAllocator` that nnSdk initialises inside its
//! own code, which this host does not run. Rather than fail a call whose
//! contract the host can honour, an allocator that was never initialised serves
//! from the dispatcher's reserved guest heap.

use crate::hle::{HostResult, HostServices};
use ew_lift::State;
use std::collections::BTreeMap;

/// Default alignment when the caller passes zero.
const DEFAULT_ALIGN: u64 = 16;

/// One allocator object's heap.
#[derive(Default, Clone)]
pub struct Heap {
    pub base: u64,
    pub end: u64,
    /// Live blocks: address to (requested size, span start, span end).
    live: BTreeMap<u64, (u64, u64, u64)>,
    /// Free spans: start to length, coalesced.
    free: BTreeMap<u64, u64>,
    /// Spans are rounded up to this many units so free spans stay aligned.
    granule: u64,
}

impl Heap {
    pub fn new(base: u64, size: u64) -> Heap {
        let mut free = BTreeMap::new();
        free.insert(base, size);
        Heap { base, end: base + size, live: BTreeMap::new(), free, granule: DEFAULT_ALIGN }
    }

    /// An empty number line with unit granularity; ranges join with `add_range`.
    pub fn number_line() -> Heap {
        Heap { granule: 1, ..Heap::default() }
    }

    /// Make `[start, start + len)` available. Overlap with a live block or an
    /// existing free span is refused.
    pub fn add_range(&mut self, start: u64, len: u64) -> bool {
        let end = start + len;
        let overlaps = self.free.iter().any(|(&s, &l)| s < end && start < s + l)
            || self.live.values().any(|&(_, s, e)| s < end && start < e);
        if len == 0 || overlaps {
            return false;
        }
        self.free.insert(start, len);
        self.base = if self.end == 0 { start } else { self.base.min(start) };
        self.end = self.end.max(end);
        true
    }

    /// First fit. Padding in front of an aligned block stays free; the span a
    /// block holds runs from its address to its end rounded to the granule.
    pub fn allocate(&mut self, size: u64, align: u64) -> Option<u64> {
        let (start, len, at) = self.free.iter().find_map(|(&start, &len)| {
            let at = start.div_ceil(align) * align;
            (at + size <= start + len).then_some((start, len, at))
        })?;
        self.free.remove(&start);
        // Round the span up so the next free span stays 16-byte aligned.
        let g = self.granule.max(1);
        let span_end = (at + size).div_ceil(g).saturating_mul(g).min(start + len);
        if span_end < start + len {
            self.free.insert(span_end, start + len - span_end);
        }
        // Alignment padding in front of the block stays free.
        if at > start {
            self.free.insert(start, at - start);
        }
        self.live.insert(at, (size, at, span_end));
        Some(at)
    }

    pub fn release(&mut self, ptr: u64) -> bool {
        let Some((_, mut start, mut end)) = self.live.remove(&ptr) else { return false };
        if let Some((&prev, &len)) = self.free.range(..start).next_back() {
            if prev + len == start {
                self.free.remove(&prev);
                start = prev;
            }
        }
        if let Some(len) = self.free.remove(&end) {
            end += len;
        }
        self.free.insert(start, end - start);
        true
    }

    pub fn size_of(&self, ptr: u64) -> Option<u64> {
        self.live.get(&ptr).map(|b| b.0)
    }

    pub fn largest_free(&self) -> u64 {
        self.free.values().copied().max().unwrap_or(0)
    }

    pub fn total_free(&self) -> u64 {
        self.free.values().sum()
    }
}

/// Every allocator object's heap, plus blocks served from the reserved guest
/// heap for allocators nnSdk would have initialised.
#[derive(Default)]
pub struct Heaps {
    pub by_object: BTreeMap<u64, Heap>,
    pub static_blocks: BTreeMap<u64, u64>,
}

fn heaps<R>(state: &State, f: impl FnOnce(&mut Heaps) -> R) -> R {
    state.host.ext.with::<Heaps, R>(f)
}

fn initialize(state: &mut State) -> HostResult {
    let (this, base, size) = (state.x[0], state.x[1], state.x[2]);
    if base == 0 || size == 0 {
        return HostResult::Unavailable(format!(
            "StandardAllocator::Initialize got base 0x{base:x} and size 0x{size:x}"
        ));
    }
    state.host.log.push(format!("heap: StandardAllocator 0x{this:x} given 0x{size:x} bytes at 0x{base:x}"));
    heaps(state, |h| h.by_object.insert(this, Heap::new(base, size)));
    HostResult::Returned(0)
}

fn allocate(state: &mut State) -> HostResult {
    let (this, size) = (state.x[0], state.x[1]);
    let align = if state.x[2] == 0 { DEFAULT_ALIGN } else { state.x[2] };
    if size == 0 {
        return HostResult::Returned(0);
    }
    let align = align.max(DEFAULT_ALIGN).next_power_of_two();
    let served = heaps(state, |h| {
        h.by_object.get_mut(&this).map(|heap| (heap.allocate(size, align), heap.largest_free()))
    });
    match served {
        Some((Some(ptr), _)) => HostResult::Returned(ptr),
        Some((None, largest)) => HostResult::Unavailable(format!(
            "StandardAllocator 0x{this:x} is exhausted: asked for 0x{size:x} bytes aligned {align}, largest free span 0x{largest:x}"
        )),
        None => {
            // Never initialised on this host: nnSdk set it up in code that is
            // not executed here. The reserved guest heap keeps the pointer
            // valid, aligned and inside guest memory.
            match state.host.allocate(size, align) {
                Ok(ptr) => {
                    heaps(state, |h| h.static_blocks.insert(ptr, size));
                    HostResult::Returned(ptr)
                }
                Err(message) => HostResult::Unavailable(message),
            }
        }
    }
}

fn free(state: &mut State) -> HostResult {
    let (this, ptr) = (state.x[0], state.x[1]);
    heaps(state, |h| {
        let released = h.by_object.get_mut(&this).is_some_and(|heap| heap.release(ptr));
        if !released {
            h.static_blocks.remove(&ptr);
        }
    });
    HostResult::Returned(0)
}

fn size_of(state: &mut State) -> HostResult {
    let (this, ptr) = (state.x[0], state.x[1]);
    let size = heaps(state, |h| {
        h.by_object
            .get(&this)
            .and_then(|heap| heap.size_of(ptr))
            .or_else(|| h.static_blocks.get(&ptr).copied())
            .unwrap_or(0)
    });
    HostResult::Returned(size)
}

fn allocatable_size(state: &mut State) -> HostResult {
    let this = state.x[0];
    let reserved = state.host.alloc_end.saturating_sub(state.host.alloc_next);
    HostResult::Returned(heaps(state, |h| h.by_object.get(&this).map(Heap::largest_free)).unwrap_or(reserved))
}

fn total_free_size(state: &mut State) -> HostResult {
    let this = state.x[0];
    let reserved = state.host.alloc_end.saturating_sub(state.host.alloc_next);
    HostResult::Returned(heaps(state, |h| h.by_object.get(&this).map(Heap::total_free)).unwrap_or(reserved))
}

fn finalize(state: &mut State) -> HostResult {
    let this = state.x[0];
    heaps(state, |h| h.by_object.remove(&this));
    HostResult::Returned(0)
}

// nn::mem::NumberLineAllocator: sub-allocation of integer ranges (the game uses
// it for offsets inside memory pools). Each object is its own number line.

#[derive(Default)]
pub struct NumberLines(pub BTreeMap<u64, Heap>);

fn lines<R>(state: &State, f: impl FnOnce(&mut BTreeMap<u64, Heap>) -> R) -> R {
    state.host.ext.with::<NumberLines, R>(|n| f(&mut n.0))
}

fn nl_initialize(state: &mut State) -> HostResult {
    let this = state.x[0];
    lines(state, |l| l.insert(this, Heap::number_line()));
    HostResult::Returned(0)
}

fn nl_finalize(state: &mut State) -> HostResult {
    let this = state.x[0];
    lines(state, |l| l.remove(&this));
    HostResult::Returned(0)
}

fn nl_set_mode(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

/// `AddRange(int begin, int size)`.
fn nl_add_range(state: &mut State) -> HostResult {
    let (this, begin, size) = (state.x[0], state.x[1] as u32 as u64, state.x[2] as u32 as u64);
    match lines(state, |l| l.get_mut(&this).map(|h| h.add_range(begin, size))) {
        Some(ok) => HostResult::Returned(ok as u64),
        None => HostResult::Unavailable(format!("NumberLineAllocator 0x{this:x} used before Initialize")),
    }
}

fn nl_allocate_impl(state: &mut State, align: u64) -> HostResult {
    let (this, out, size) = (state.x[0], state.x[1], state.x[2] as u32 as u64);
    if size == 0 {
        return HostResult::Returned(0);
    }
    let got = lines(state, |l| l.get_mut(&this).map(|h| h.allocate(size, align.max(1))));
    match got {
        None => HostResult::Unavailable(format!("NumberLineAllocator 0x{this:x} used before Initialize")),
        Some(None) => HostResult::Returned(0),
        Some(Some(at)) => match crate::hle::write_bytes(state, out, &(at as u32).to_le_bytes()) {
            Ok(()) => HostResult::Returned(1),
            Err(e) => e,
        },
    }
}

/// `Allocate(int* out, int size)`.
fn nl_allocate(state: &mut State) -> HostResult {
    nl_allocate_impl(state, 1)
}

/// `Allocate(int* out, int size, int alignment)`.
fn nl_allocate_aligned(state: &mut State) -> HostResult {
    let align = state.x[3] as u32 as u64;
    nl_allocate_impl(state, align)
}

/// `Free(int begin)`.
fn nl_free(state: &mut State) -> HostResult {
    let (this, begin) = (state.x[0], state.x[1] as u32 as u64);
    lines(state, |l| l.get_mut(&this).map(|h| h.release(begin)));
    HostResult::Returned(0)
}

/// The allocator object itself lives in the guest's `.bss`. Every field it would
/// carry is held by the host arena instead, so construction and teardown have no
/// guest-visible effect beyond what `Initialize` and `Finalize` already do.
fn constructor(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

fn clear_thread_cache(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

pub fn register(services: &mut HostServices) {
    let nl: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("_ZN2nn3mem19NumberLineAllocator10InitializeEPFPvmS2_ES2_PFvS2_S2_ES2_b", nl_initialize, "an empty number line held on the host"),
        ("_ZN2nn3mem19NumberLineAllocator8FinalizeEv", nl_finalize, "forgets the number line"),
        ("_ZN2nn3mem19NumberLineAllocator17SetAllocationModeENS1_14AllocationModeE", nl_set_mode, "first fit serves every mode's contract"),
        ("_ZN2nn3mem19NumberLineAllocator8AddRangeEii", nl_add_range, "adds a free range, refusing overlap"),
        ("_ZN2nn3mem19NumberLineAllocator8AllocateEPii", nl_allocate, "first-fit range allocation"),
        ("_ZN2nn3mem19NumberLineAllocator8AllocateEPiii", nl_allocate_aligned, "aligned first-fit range allocation"),
        ("_ZN2nn3mem19NumberLineAllocator4FreeEi", nl_free, "returns the range, coalescing neighbours"),
    ];
    for (name, handler, claim) in nl {
        services.register(name, *handler, claim);
    }
    services.register(
        "_ZN2nn3mem17StandardAllocatorC1Ev",
        constructor as _,
        "the allocator's state is held by the host arena, so construction writes nothing",
    );
    services.register(
        "_ZN2nn3mem17StandardAllocator10InitializeEPvm",
        initialize as _,
        "gives this allocator object its own heap over the region the game provides",
    );
    services.register(
        "_ZN2nn3mem17StandardAllocator10InitializeEPvmb",
        initialize as _,
        "gives this allocator object its own heap over the region the game provides",
    );
    services.register(
        "_ZN2nn3mem17StandardAllocator8AllocateEmm",
        allocate as _,
        "aligned first-fit allocation from this allocator's own heap once Initialize has run, and from the reserved guest heap for the static allocator nnSdk would have initialised",
    );
    services.register(
        "_ZN2nn3mem17StandardAllocator4FreeEPv",
        free as _,
        "returns the block to its heap's free list, coalescing neighbours",
    );
    services.register(
        "_ZNK2nn3mem17StandardAllocator9GetSizeOfEPKv",
        size_of as _,
        "returns the size the block was allocated with, or 0 for an unknown pointer",
    );
    services.register(
        "_ZNK2nn3mem17StandardAllocator18GetAllocatableSizeEv",
        allocatable_size as _,
        "largest free span in this allocator's heap, or the reserved guest heap when it was never initialised",
    );
    services.register(
        "_ZN2nn3mem17StandardAllocator8FinalizeEv",
        finalize as _,
        "forgets this allocator's heap",
    );
    services.register(
        "_ZNK2nn3mem17StandardAllocator16GetTotalFreeSizeEv",
        total_free_size as _,
        "sum of the free spans in this allocator's heap",
    );
    services.register(
        "_ZNK2nn3mem17StandardAllocator16ClearThreadCacheEv",
        clear_thread_cache as _,
        "the host heap keeps no per-thread cache",
    );
    services.register(
        "_ZNK2nn3mem17StandardAllocator21CleanUpManagementAreaEv",
        clear_thread_cache as _,
        "no management area is reserved, so there is nothing to clean",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init(s: &mut State, base: u64, size: u64) {
        s.x[0] = 0x10;
        s.x[1] = base;
        s.x[2] = size;
        assert_eq!(initialize(s), HostResult::Returned(0));
    }

    #[test]
    fn allocations_are_aligned_and_do_not_overlap() {
        let mut s = State::default();
        init(&mut s, 0x1000, 0x100);
        let mut seen = Vec::new();
        for (size, align) in [(1u64, 0u64), (7, 32), (3, 0)] {
            s.x[0] = 0x10;
            s.x[1] = size;
            s.x[2] = align;
            let ptr = match allocate(&mut s) {
                HostResult::Returned(p) => p,
                other => panic!("{other:?}"),
            };
            assert_eq!(ptr % align.max(DEFAULT_ALIGN), 0);
            assert!(ptr >= 0x1000 && ptr + size <= 0x1100);
            for (other, other_size) in &seen {
                assert!(ptr + size <= *other || *other + other_size <= ptr, "blocks overlap");
            }
            seen.push((ptr, size));
        }
    }

    #[test]
    fn size_of_and_free_track_the_block() {
        let mut s = State::default();
        init(&mut s, 0x2000, 0x80);
        s.x[1] = 24;
        s.x[2] = 8;
        let ptr = match allocate(&mut s) {
            HostResult::Returned(p) => p,
            other => panic!("{other:?}"),
        };
        s.x[1] = ptr;
        assert_eq!(size_of(&mut s), HostResult::Returned(24));
        assert_eq!(free(&mut s), HostResult::Returned(0));
        assert_eq!(size_of(&mut s), HostResult::Returned(0));
    }

    #[test]
    fn a_zero_size_returns_null_and_exhaustion_is_reported() {
        let mut s = State::default();
        init(&mut s, 0x3000, 0x20);
        s.x[1] = 0;
        s.x[2] = 0;
        assert_eq!(allocate(&mut s), HostResult::Returned(0));

        s.x[1] = 0x1000;
        s.x[2] = 16;
        assert!(matches!(allocate(&mut s), HostResult::Unavailable(_)));
    }

    #[test]
    fn a_static_allocator_serves_from_the_reserved_heap() {
        let mut s = State::default();
        s.mem = vec![0u8; 0x20000];
        s.host.alloc_next = 0x10000;
        s.host.alloc_end = 0x20000;
        // No Initialize: this is the object nnSdk would have set up.
        s.x[1] = 48;
        s.x[2] = 16;
        let ptr = match allocate(&mut s) {
            HostResult::Returned(p) => p,
            other => panic!("expected the reserved heap to serve the call, got {other:?}"),
        };
        assert!(ptr >= 0x10000 && ptr + 48 <= 0x20000, "0x{ptr:x} is outside the guest heap");
        assert_eq!(ptr % 16, 0);
        s.x[1] = ptr;
        assert_eq!(size_of(&mut s), HostResult::Returned(48));
        assert_eq!(allocatable_size(&mut s), HostResult::Returned(0x10000 - 48));
    }

    #[test]
    fn a_number_line_allocates_aligned_ranges_and_reuses_them() {
        let mut h = Heap::number_line();
        assert!(h.add_range(0, 100));
        assert!(!h.add_range(50, 10), "overlap must be refused");
        assert_eq!(h.allocate(10, 1), Some(0));
        assert_eq!(h.allocate(10, 32), Some(32));
        assert_eq!(h.allocate(5, 1), Some(10));
        assert!(h.release(0));
        assert!(h.release(10));
        assert_eq!(h.allocate(15, 1), Some(0));
        assert_eq!(h.allocate(200, 1), None);
    }

    #[test]
    fn each_allocator_object_has_its_own_heap_and_reuses_freed_memory() {
        let mut s = State::default();
        init(&mut s, 0x1000, 0x1000);
        s.x[0] = 0x20;
        s.x[1] = 0x8000;
        s.x[2] = 0x100;
        assert_eq!(initialize(&mut s), HostResult::Returned(0));
        let alloc = |s: &mut State, this: u64, size: u64| {
            s.x[0] = this;
            s.x[1] = size;
            s.x[2] = 16;
            match allocate(s) {
                HostResult::Returned(p) => p,
                other => panic!("{other:?}"),
            }
        };
        let a = alloc(&mut s, 0x10, 0x400);
        let b = alloc(&mut s, 0x20, 0x40);
        assert!((0x1000..0x2000).contains(&a));
        assert!((0x8000..0x8100).contains(&b), "second heap served from the first: 0x{b:x}");
        let c = alloc(&mut s, 0x10, 0x400);
        s.x[0] = 0x10;
        s.x[1] = a;
        assert_eq!(free(&mut s), HostResult::Returned(0));
        s.x[1] = c;
        assert_eq!(free(&mut s), HostResult::Returned(0));
        // Everything was returned and coalesced, so the whole heap fits again.
        assert_eq!(alloc(&mut s, 0x10, 0x1000), 0x1000);
    }

    #[test]
    fn allocating_before_initialize_is_reported() {
        let mut s = State::default();
        s.x[1] = 16;
        s.x[2] = 16;
        match allocate(&mut s) {
            HostResult::Unavailable(message) => {
                assert!(message.contains("no guest page arena"), "{message}")
            }
            other => panic!("expected a reported fault, got {other:?}"),
        }
    }
}
