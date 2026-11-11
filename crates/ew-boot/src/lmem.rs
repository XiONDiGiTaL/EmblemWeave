//! `nn::lmem` frame heaps.
//!
//! A frame heap is a region the game owns, carved from both ends: positive
//! alignments allocate upward from the head, negative alignments downward from
//! the tail, and `FreeToFrameHeap` releases everything taken from one end (or
//! both) at once. That contract is small enough to implement exactly.
//!
//! State lives in the `HeapHead` the game supplies (or, for the overload without
//! one, at the start of the region, which is where nnSdk places it), tagged with
//! a magic word so a stray handle is reported instead of trusted.

use crate::hle::{read_bytes, write_bytes, HostResult, HostServices};
use ew_lift::State;

const MAGIC: u64 = 0x4845_4150_4D52_4646; // "FFRMPAEH"
/// Bytes of head state this host keeps.
const HEAD_BYTES: u64 = 0x40;
const OPTION_ZERO_CLEAR: u64 = 1;
const FREE_HEAD: u64 = 1;
const FREE_TAIL: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Head {
    start: u64,
    end: u64,
    head: u64,
    tail: u64,
    option: u64,
}

fn load(state: &State, at: u64) -> Result<Head, HostResult> {
    let b = read_bytes(state, at, 48)?;
    let w = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
    if w(0) != MAGIC {
        return Err(HostResult::Unavailable(format!("0x{at:x} is not a frame heap this host created")));
    }
    Ok(Head { start: w(1), end: w(2), head: w(3), tail: w(4), option: w(5) })
}

fn store(state: &mut State, at: u64, h: Head) -> Result<(), HostResult> {
    let mut b = Vec::with_capacity(48);
    for v in [MAGIC, h.start, h.end, h.head, h.tail, h.option] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    write_bytes(state, at, &b)
}

fn create(state: &mut State, head_at: u64, start: u64, size: u64, option: u64) -> HostResult {
    if start == 0 || size == 0 {
        return HostResult::Returned(0);
    }
    let h = Head { start, end: start + size, head: start, tail: start + size, option };
    match store(state, head_at, h) {
        Ok(()) => HostResult::Returned(head_at),
        Err(e) => e,
    }
}

/// `CreateFrameHeap(void*, size_t, int option, HeapHead*)`.
fn create_with_head(state: &mut State) -> HostResult {
    let (addr, size, option, head) = (state.x[0], state.x[1], state.x[2], state.x[3]);
    create(state, head, addr, size, option)
}

/// `CreateFrameHeap(void*, size_t, int option)`: the head sits at the start of
/// the region, so the usable part begins after it.
fn create_in_place(state: &mut State) -> HostResult {
    let (addr, size, option) = (state.x[0], state.x[1], state.x[2]);
    let head = (addr + 15) & !15;
    let start = head + HEAD_BYTES;
    if start >= addr + size {
        return HostResult::Returned(0);
    }
    create(state, head, start, addr + size - start, option)
}

fn destroy(state: &mut State) -> HostResult {
    let at = state.x[0];
    if let Err(e) = load(state, at) {
        return e;
    }
    match write_bytes(state, at, &[0u8; 8]) {
        Ok(()) => HostResult::Returned(at),
        Err(e) => e,
    }
}

/// `AllocateFromFrameHeap(HeapHead*, size_t, int alignment)`. A negative
/// alignment allocates from the tail. Returns null when the heap is full.
fn allocate(state: &mut State) -> HostResult {
    let (at, size) = (state.x[0], state.x[1].max(1));
    let alignment = state.x[2] as i32;
    let mut h = match load(state, at) {
        Ok(h) => h,
        Err(e) => return e,
    };
    let align = (alignment.unsigned_abs() as u64).max(4).next_power_of_two();
    let ptr = if alignment >= 0 {
        let p = (h.head + align - 1) & !(align - 1);
        if p + size > h.tail {
            return HostResult::Returned(0);
        }
        h.head = p + size;
        p
    } else {
        let Some(p) = h.tail.checked_sub(size).map(|v| v & !(align - 1)) else {
            return HostResult::Returned(0);
        };
        if p < h.head {
            return HostResult::Returned(0);
        }
        h.tail = p;
        p
    };
    if h.option & OPTION_ZERO_CLEAR != 0 {
        if let Err(e) = write_bytes(state, ptr, &vec![0u8; size as usize]) {
            return e;
        }
    }
    match store(state, at, h) {
        Ok(()) => HostResult::Returned(ptr),
        Err(e) => e,
    }
}

/// `FreeToFrameHeap(HeapHead*, FreeMode)`: 1 head, 2 tail, 3 both.
fn free(state: &mut State) -> HostResult {
    let (at, mode) = (state.x[0], state.x[1]);
    let mut h = match load(state, at) {
        Ok(h) => h,
        Err(e) => return e,
    };
    if mode & FREE_HEAD != 0 {
        h.head = h.start;
    }
    if mode & FREE_TAIL != 0 {
        h.tail = h.end;
    }
    match store(state, at, h) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

pub fn register(s: &mut HostServices) {
    s.register("_ZN2nn4lmem15CreateFrameHeapEPvmiPNS0_6detail8HeapHeadE", create_with_head, "frame heap state kept in the caller's HeapHead");
    s.register("_ZN2nn4lmem15CreateFrameHeapEPvmi", create_in_place, "head placed at the start of the region, as nnSdk does");
    s.register("_ZN2nn4lmem16DestroyFrameHeapEPNS0_6detail8HeapHeadE", destroy, "clears the tag; the memory stays the caller's");
    s.register("_ZN2nn4lmem21AllocateFromFrameHeapEPNS0_6detail8HeapHeadEmi", allocate, "head-up for positive alignment, tail-down for negative; null when full; ZeroClear honoured");
    s.register("_ZN2nn4lmem15FreeToFrameHeapEPNS0_6detail8HeapHeadENS0_8FreeModeE", free, "releases the head side, the tail side, or both");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_from_both_ends_and_frees() {
        let mut s = State::default();
        s.x[0] = 0x1000;
        s.x[1] = 0x1000;
        s.x[2] = OPTION_ZERO_CLEAR;
        s.x[3] = 0x100;
        assert_eq!(create_with_head(&mut s), HostResult::Returned(0x100));
        s.mem[0x1000..0x2000].fill(0xAA);

        s.x[0] = 0x100;
        s.x[1] = 0x10;
        s.x[2] = 16;
        assert_eq!(allocate(&mut s), HostResult::Returned(0x1000));
        assert_eq!(&s.mem[0x1000..0x1010], &[0u8; 16], "ZeroClear");
        s.x[0] = 0x100;
        s.x[1] = 0x20;
        s.x[2] = (-32i64) as u64;
        assert_eq!(allocate(&mut s), HostResult::Returned(0x1FE0));
        s.x[0] = 0x100;
        s.x[1] = 0x2000;
        s.x[2] = 4;
        assert_eq!(allocate(&mut s), HostResult::Returned(0), "full heap returns null");

        s.x[0] = 0x100;
        s.x[1] = 3;
        assert_eq!(free(&mut s), HostResult::Returned(0));
        s.x[0] = 0x100;
        s.x[1] = 0x10;
        s.x[2] = 16;
        assert_eq!(allocate(&mut s), HostResult::Returned(0x1000));
    }

    #[test]
    fn a_foreign_handle_is_reported() {
        let mut s = State::default();
        s.x[0] = 0x500;
        s.x[1] = 1;
        assert!(matches!(free(&mut s), HostResult::Unavailable(_)));
    }
}
