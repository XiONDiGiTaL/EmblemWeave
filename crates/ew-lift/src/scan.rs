//! Function and block discovery.
//!
//! Discovery has two jobs: find addresses that must be dispatchable, and hold
//! the types that describe a recovered function. The actual graph traversal
//! lives in [`crate::cfg`]; this module owns the data model so that the emitter,
//! the interpreter and the reports all describe functions the same way.
//!
//! A function is a set of basic blocks. A block is a straight-line run of
//! instructions that ends at the first control-flow instruction. Every block
//! records its outgoing edges, so "complete" is a statement about the graph
//! rather than about a single straight-line body.

use crate::aarch64::{decode, MemMode, Op};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize)]
pub struct Insn {
    pub addr: u64,
    pub word: u32,
    pub op: Op,
}

/// Where control can go when a block ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EdgeKind {
    /// Direct branch or resolved jump-table target.
    Direct,
    /// Address of the next instruction after a conditional branch or a call.
    FallThrough,
    /// Return to the caller; the address comes from a register at run time.
    Return,
    /// A call into another function through a known address (`bl`).
    Call,
    /// A call through a register (`blr`). The callee is unknown, but control
    /// returns here, so this does not make the caller's own graph incomplete.
    IndirectCall,
    /// Indirect branch we could not resolve statically. This is the one kind
    /// that leaves a function's own control flow genuinely unknown.
    Indirect,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Edge {
    pub kind: EdgeKind,
    pub target: Option<u64>,
}

impl Edge {
    pub fn direct(kind: EdgeKind, target: u64) -> Edge {
        Edge { kind, target: Some(target) }
    }
    pub fn terminal(kind: EdgeKind) -> Edge {
        Edge { kind, target: None }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Block {
    pub addr: u64,
    pub insns: Vec<Insn>,
    pub edges: Vec<Edge>,
    /// Set when the block ends in a `br` driven by a resolved jump table.
    pub jump_table: Option<crate::jump_table::JumpTable>,
    /// False when the block ran into the instruction cap before a terminator.
    pub terminated: bool,
}

impl Block {
    pub fn end_addr(&self) -> u64 {
        self.insns.last().map(|i| i.addr + 4).unwrap_or(self.addr)
    }

    /// Address the block falls through to, if it can.
    pub fn fallthrough(&self) -> Option<u64> {
        self.edges
            .iter()
            .find(|e| e.kind == EdgeKind::FallThrough)
            .and_then(|e| e.target)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LiftedFunction {
    pub addr: u64,
    pub blocks: Vec<Block>,
    /// True when every block terminated and no edge was left unresolved.
    pub complete: bool,
    pub unsupported: usize,
    pub has_memory: bool,
    pub has_calls: bool,
    pub jump_tables: usize,
    pub jump_table_targets: usize,
    pub unresolved_edges: usize,
    /// Calls through a register. Counted, but not treated as a gap in the
    /// caller's own control flow.
    pub indirect_calls: usize,
    /// Blocks that ran into the instruction budget instead of a terminator.
    pub unterminated_blocks: usize,
    /// The function's instruction budget was exhausted, so its recovery is
    /// partial by construction rather than blocked by an unknown instruction.
    pub budget_exhausted: bool,
}

impl LiftedFunction {
    pub fn c_name(&self) -> String {
        format!("ew_fn_{:08x}", self.addr)
    }

    pub fn insn_count(&self) -> usize {
        self.blocks.iter().map(|b| b.insns.len()).sum()
    }

    pub fn insns(&self) -> impl Iterator<Item = &Insn> {
        self.blocks.iter().flat_map(|b| b.insns.iter())
    }

    /// A function that only touches registers, calls nothing and has fully
    /// recovered control flow: suitable for bit-exact differential testing
    /// against the reference interpreter.
    pub fn is_pure_leaf(&self) -> bool {
        self.complete && !self.has_memory && !self.has_calls && self.unsupported == 0
    }

    /// True when the block set contains `addr`.
    pub fn contains(&self, addr: u64) -> bool {
        self.blocks.binary_search_by_key(&addr, |b| b.addr).is_ok()
    }
}

/// Iterate the text segment as 32-bit guest words at addresses `0..len`.
pub fn words(text: &[u8]) -> impl Iterator<Item = (u64, u32)> + '_ {
    text.chunks_exact(4)
        .enumerate()
        .map(|(i, c)| ((i as u64) * 4, u32::from_le_bytes([c[0], c[1], c[2], c[3]])))
}

pub fn word_at(text: &[u8], addr: u64) -> Option<u32> {
    // The address comes from guest registers, so it can be anything the guest
    // computed. A wrapped addition here would panic in debug builds and read the
    // wrong word in release builds.
    if addr % 4 != 0 {
        return None;
    }
    let off = usize::try_from(addr).ok()?;
    let end = off.checked_add(4)?;
    if end > text.len() {
        return None;
    }
    Some(u32::from_le_bytes([text[off], text[off + 1], text[off + 2], text[off + 3]]))
}

/// Entry points of other functions, used to keep a traversal inside one
/// function: a branch to a known call target is a tail call, not a block.
pub fn call_targets(text: &[u8]) -> BTreeSet<u64> {
    let mut set = BTreeSet::new();
    let len = text.len() as u64;
    for (addr, word) in words(text) {
        if let Op::Bl { target } = decode(word, addr) {
            if target % 4 == 0 && target < len {
                set.insert(target);
            }
        }
    }
    set
}

/// Collect every address that must be dispatchable: the module entry point,
/// branch targets, call targets, call return sites, and resolved jump-table
/// targets.
pub fn discover_starts(text: &[u8], entry: u64, limit: usize) -> Vec<u64> {
    discover_starts_in(text, &[], entry, limit)
}

/// True when `addr` is preceded by a terminator or padding, which is what a
/// real function boundary looks like in a linked image.
fn is_boundary(text: &[u8], addr: u64) -> bool {
    if addr == 0 {
        return true;
    }
    match word_at(text, addr - 4) {
        None => true,
        Some(0) => true,
        Some(word) => matches!(
            decode(word, addr - 4),
            Op::Ret { .. } | Op::Br { .. } | Op::B { .. }
        ),
    }
}

/// The prologue shapes this toolchain emits: a frame push of `x29`/`x30`, or a
/// plain stack reservation.
fn looks_like_prologue(word: u32, addr: u64) -> bool {
    match decode(word, addr) {
        Op::LoadStorePair { rt: 29, rt2: 30, mode: MemMode::PreIndex, .. } => true,
        Op::AddSubImm { rd: 31, rn: 31, sub: true, .. } => true,
        _ => false,
    }
}

/// Function entries that no branch reaches.
///
/// C++ virtual functions and callbacks are only ever called through a register,
/// so nothing in the branch graph points at them, yet a large share of the
/// switch-heavy code sits behind exactly those entry points. A prologue at a
/// real boundary is the standard way to recover them, and both conditions are
/// required so a mid-function instruction is never mistaken for an entry.
pub fn prologue_starts(text: &[u8], limit: usize) -> Vec<u64> {
    let mut set = BTreeSet::new();
    for (addr, word) in words(text) {
        if addr == 0 || word == 0 {
            continue;
        }
        if looks_like_prologue(word, addr) && is_boundary(text, addr) {
            set.insert(addr);
            if set.len() >= limit {
                break;
            }
        }
    }
    set.into_iter().collect()
}

pub fn discover_starts_in(
    text: &[u8],
    segments: crate::jump_table::Segments,
    entry: u64,
    limit: usize,
) -> Vec<u64> {
    let mut set = BTreeSet::new();
    let len = text.len() as u64;
    fn push(set: &mut BTreeSet<u64>, a: u64, len: u64) {
        if a % 4 == 0 && a < len {
            set.insert(a);
        }
    }
    push(&mut set, entry, len);
    for (addr, word) in words(text) {
        match decode(word, addr) {
            Op::B { target } => push(&mut set, target, len),
            Op::Bl { target } => {
                push(&mut set, target, len);
                push(&mut set, addr + 4, len);
            }
            Op::BCond { target, .. }
            | Op::CompareBranchZero { target, .. }
            | Op::TestBitBranch { target, .. } => push(&mut set, target, len),
            Op::Br { .. } => {
                for target in crate::jump_table::targets_at(text, segments, addr) {
                    push(&mut set, target, len);
                }
            }
            _ => {}
        }
        if set.len() >= limit {
            break;
        }
    }
    // Prologue entries get their own budget. Sharing the edge budget meant they
    // were never added at all: the branch scan always filled it first.
    let mut added = 0usize;
    for start in prologue_starts(text, limit) {
        if added >= limit {
            break;
        }
        if set.insert(start) {
            added += 1;
        }
    }
    set.into_iter().collect()
}

/// Recover one function, starting at `start`.
pub fn lift_function(text: &[u8], start: u64, max_insns: usize) -> Option<LiftedFunction> {
    lift_function_in(text, &[], start, max_insns)
}

/// Recover one function using the module image, so jump tables held in rodata
/// can be read.
pub fn lift_function_in(
    text: &[u8],
    segments: crate::jump_table::Segments,
    start: u64,
    max_insns: usize,
) -> Option<LiftedFunction> {
    let boundaries = call_targets(text);
    crate::cfg::build(text, segments, start, max_insns, &boundaries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words_to_bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn discovers_calls_and_returns() {
        // bl 0x20 ; b 0x10 ; <pad> ... ; function bodies
        let text = words_to_bytes(&[
            0x9400_0008, // 0x00: bl 0x20
            0x1400_0004, // 0x04: b 0x14
            0xD503201F,  // 0x08: nop
            0xD503201F,  // 0x0c: nop
            0xD503201F,  // 0x10: nop
            0xD65F03C0,  // 0x14: ret
            0xD503201F,  // 0x18: nop
            0xD503201F,  // 0x1c: nop
            0xD65F03C0,  // 0x20: ret
        ]);
        let starts = discover_starts(&text, 0, 100);
        for expected in [0u64, 0x04, 0x14, 0x20] {
            assert!(starts.contains(&expected), "missing start 0x{expected:x}");
        }
        assert!(starts.contains(&0x04), "call return address must be a start");
    }

    #[test]
    fn straight_line_body_is_complete() {
        let text = words_to_bytes(&[
            0x9100_4020, // add x0, x1, #0x10
            0xD65F03C0, // ret
        ]);
        let f = lift_function(&text, 0, 256).unwrap();
        assert!(f.complete);
        assert_eq!(f.insn_count(), 2);
        assert!(f.is_pure_leaf());
    }

    #[test]
    fn conditional_branch_recovers_both_paths() {
        // cbz x0, +8 ; ret ; ret
        let text = words_to_bytes(&[
            0xB400_0040, // 0x00: cbz x0, 0x08
            0xD65F03C0, // 0x04: ret
            0xD65F03C0, // 0x08: ret
        ]);
        let f = lift_function(&text, 0, 256).unwrap();
        assert!(f.complete, "both successors of a conditional branch must be recovered");
        assert_eq!(f.blocks.len(), 3);
        assert_eq!(f.blocks[0].edges.len(), 2);
        assert_eq!(f.unresolved_edges, 0);
    }

    #[test]
    fn backward_branch_does_not_loop_forever() {
        // loop: b loop
        let text = words_to_bytes(&[0x1400_0000]);
        let f = lift_function(&text, 0, 256).unwrap();
        assert_eq!(f.blocks.len(), 1);
        assert!(f.complete);
    }

    #[test]
    fn unsupported_instruction_is_counted() {
        let text = words_to_bytes(&[0x0000_0000, 0xD65F03C0]);
        let f = lift_function(&text, 0, 256).unwrap();
        assert_eq!(f.unsupported, 1);
        assert!(!f.is_pure_leaf());
    }

    #[test]
    fn ignores_unaligned_and_out_of_range() {
        let text = words_to_bytes(&[0xD65F03C0]);
        assert!(lift_function(&text, 2, 8).is_none());
        assert!(lift_function(&text, 0x100, 8).is_none());
    }
}
