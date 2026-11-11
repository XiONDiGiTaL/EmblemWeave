//! Control-flow graph recovery.
//!
//! The traversal starts at one address and follows every statically known edge
//! until the function's own blocks are exhausted. Three rules keep it honest:
//!
//! * an unconditional branch is followed, so straight-line lifting no longer
//!   stops after the first `b` that a compiler emits for an if/else;
//! * a conditional branch contributes both successors, which is what turns
//!   chains of `cbz`/`b.ne` into one complete function instead of N fragments;
//! * a call edge is not followed into the callee, and a `br` whose table cannot
//!   be resolved stays an explicit unresolved edge rather than a guess.

use crate::aarch64::Op;
use crate::jump_table::{self, JumpTable};
use crate::scan::{word_at, Block, Edge, EdgeKind, Insn, LiftedFunction};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Hard ceiling on one function, so a bad edge cannot walk the module.
const MAX_BLOCKS: usize = 2048;
const MAX_BLOCK_INSNS: usize = 1024;

/// Every call site in a block, including calls that do not end it.
///
/// A `bl` in the middle of a block still calls another function; the block keeps
/// going after the call returns, but the callee exists, and reachability has to
/// know about it. Only recording a call when it happens to be the last
/// instruction is what makes a whole-program walk stop at the first function.
fn call_edges(insns: &[Insn]) -> Vec<Edge> {
    let mut edges = Vec::new();
    for insn in insns {
        match insn.op {
            Op::Bl { target } => edges.push(Edge::direct(EdgeKind::Call, target)),
            Op::Blr { .. } => edges.push(Edge::terminal(EdgeKind::IndirectCall)),
            _ => {}
        }
    }
    edges
}

fn edges_for(
    text: &[u8],
    segments: jump_table::Segments,
    known: &jump_table::Known,
    insns: &[Insn],
    terminated: bool,
) -> (Vec<Edge>, Option<JumpTable>) {
    let mut edges = Vec::new();
    let mut table = None;
    if !terminated {
        return (edges, table);
    }
    let last = match insns.last() {
        Some(i) => i,
        None => return (edges, table),
    };
    let next = last.addr + 4;
    let fallthrough = |edges: &mut Vec<Edge>| {
        if word_at(text, next).is_some() {
            edges.push(Edge::direct(EdgeKind::FallThrough, next));
        }
    };

    match last.op {
        Op::B { target } => edges.push(Edge::direct(EdgeKind::Direct, target)),
        Op::BCond { target, .. }
        | Op::CompareBranchZero { target, .. }
        | Op::TestBitBranch { target, .. } => {
            edges.push(Edge::direct(EdgeKind::Direct, target));
            fallthrough(&mut edges);
        }
        Op::Bl { target } => {
            edges.push(Edge::direct(EdgeKind::Call, target));
            fallthrough(&mut edges);
        }
        Op::Blr { .. } => {
            edges.push(Edge::terminal(EdgeKind::Call));
            fallthrough(&mut edges);
        }
        Op::Ret { .. } => edges.push(Edge::terminal(EdgeKind::Return)),
        Op::Br { .. } => {
            // The table base is usually built in the function prologue, so look
            // back beyond this block rather than only inside it.
            let window = jump_table::context_window(text, insns[0].addr, last.addr);
            match jump_table::detect(text, segments, known, &window) {
                Some(t) => {
                    for target in &t.targets {
                        edges.push(Edge::direct(EdgeKind::Direct, *target));
                    }
                    table = Some(t);
                }
                None => edges.push(Edge::terminal(EdgeKind::Indirect)),
            }
        }
        _ => {}
    }
    (edges, table)
}

/// Recover the function that starts at `start`.
///
/// `boundaries` holds known function entry points (call targets). A branch to
/// one of them is a tail call, so traversal stops there instead of absorbing
/// another function's blocks.
pub fn build(
    text: &[u8],
    segments: jump_table::Segments,
    start: u64,
    max_insns: usize,
    boundaries: &BTreeSet<u64>,
) -> Option<LiftedFunction> {
    if start % 4 != 0 || word_at(text, start).is_none() {
        return None;
    }

    // One linear pass over the prologue: table bases are often built once and
    // reused by every switch in the function.
    let known = jump_table::prologue_constants(text, start, 64);

    let mut blocks: BTreeMap<u64, Block> = BTreeMap::new();
    let mut queue: VecDeque<u64> = VecDeque::new();
    queue.push_back(start);
    let mut total = 0usize;
    let mut truncated = false;

    while let Some(addr) = queue.pop_front() {
        if blocks.contains_key(&addr) {
            continue;
        }
        if word_at(text, addr).is_none() {
            continue;
        }

        let mut insns = Vec::new();
        let mut cur = addr;
        let mut terminated = false;
        while insns.len() < MAX_BLOCK_INSNS {
            let word = match word_at(text, cur) {
                Some(w) => w,
                None => break,
            };
            let op = crate::aarch64::decode(word, cur);
            insns.push(Insn { addr: cur, word, op });
            if op.is_terminal() {
                terminated = true;
                break;
            }
            cur += 4;
        }
        if insns.is_empty() {
            continue;
        }

        total += insns.len();
        let over_budget = total > max_insns;

        let (mut edges, jump_table) = edges_for(text, segments, &known, &insns, terminated);
        // Terminator calls are already present above; add the ones inside the
        // block without duplicating them.
        for edge in call_edges(&insns) {
            if !edges.iter().any(|e| e.kind == edge.kind && e.target == edge.target) {
                edges.push(edge);
            }
        }
        for edge in &edges {
            let target = match (edge.kind, edge.target) {
                (EdgeKind::Direct, Some(t)) | (EdgeKind::FallThrough, Some(t)) => t,
                _ => continue,
            };
            if target == start {
                continue; // loop back to the entry: already discovered
            }
            if boundaries.contains(&target) {
                continue; // tail call into another function
            }
            if word_at(text, target).is_some() {
                queue.push_back(target);
            }
        }

        blocks.insert(
            addr,
            Block { addr, insns, edges, jump_table, terminated },
        );
        // The block that crossed the budget is kept: dropping it would look
        // like the function has no code at that address. The function is
        // flagged partial instead, which is the truth.
        if over_budget || blocks.len() >= MAX_BLOCKS {
            truncated = true;
            break;
        }
    }

    if blocks.is_empty() {
        return None;
    }

    let mut unsupported = 0usize;
    let mut has_memory = false;
    let mut has_calls = false;
    let mut indirect_calls = 0usize;
    let mut jump_tables = 0usize;
    let mut jump_table_targets = 0usize;
    let mut unresolved_edges = 0usize;
    let mut unterminated_blocks = 0usize;

    for block in blocks.values() {
        for insn in &block.insns {
            if matches!(insn.op, Op::Unsupported { .. } | Op::UnsupportedBitfield { .. }) {
                unsupported += 1;
            }
            if insn.op.touches_memory() {
                has_memory = true;
            }
            if insn.op.is_call() {
                has_calls = true;
            }
        }
        if !block.terminated {
            unterminated_blocks += 1;
        }
        if let Some(table) = &block.jump_table {
            jump_tables += 1;
            jump_table_targets += table.targets.len();
        }
        unresolved_edges += block
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Indirect)
            .count();
        indirect_calls += block
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::IndirectCall)
            .count();
    }

    Some(LiftedFunction {
        addr: start,
        blocks: blocks.into_values().collect(),
        complete: !truncated && unterminated_blocks == 0 && unresolved_edges == 0,
        unsupported,
        has_memory,
        has_calls,
        jump_tables,
        jump_table_targets,
        unresolved_edges,
        indirect_calls,
        unterminated_blocks,
        budget_exhausted: truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(text: &mut [u8], off: usize, word: u32) {
        text[off..off + 4].copy_from_slice(&word.to_le_bytes());
    }

    /// One switch, recovered as a single function: the conditional branch keeps
    /// both successors and the `br` at the end of the default arm contributes
    /// every table target, so the result is complete rather than four fragments.
    #[test]
    fn a_switch_becomes_one_complete_function() {
        let mut text = vec![0u8; 0x2c];
        put(&mut text, 0x00, 0x7100_051F); // cmp w8, #1
        put(&mut text, 0x04, 0x5400_00A8); // b.hi 0x18
        put(&mut text, 0x08, 0x1000_00C9); // adr x9, 0x20
        put(&mut text, 0x0c, 0x3868_692A); // ldrb w10, [x9, x8]
        put(&mut text, 0x10, 0x8B2A_C929); // add x9, x9, w10, sxtw #2
        put(&mut text, 0x14, 0xD61F_0120); // br x9
        put(&mut text, 0x18, 0xD65F_03C0); // default arm: ret
        put(&mut text, 0x1c, 0xD503_201F); // nop
        // Table at 0x20: two signed byte offsets, relative to the base and
        // scaled by four, so the targets land on the two returns below.
        text[0x20] = 1;
        text[0x21] = 2;
        put(&mut text, 0x24, 0xD65F_03C0); // ret
        put(&mut text, 0x28, 0xD65F_03C0); // ret

        let boundaries = BTreeSet::new();
        let f = build(&text, &[], 0, 4096, &boundaries).expect("function must lift");
        assert!(f.complete, "a fully recovered switch is complete");
        assert_eq!(f.jump_tables, 1);
        assert_eq!(f.jump_table_targets, 2);
        assert_eq!(f.unresolved_edges, 0);
        assert_eq!(f.unterminated_blocks, 0);
        // Entry block (cmp + b.hi), the switch block at the fallthrough 0x08,
        // the default arm, and the two table targets.
        let addrs: Vec<u64> = f.blocks.iter().map(|b| b.addr).collect();
        assert_eq!(addrs, vec![0x00, 0x08, 0x18, 0x24, 0x28]);
    }

    /// A conditional branch that skips an instruction contributes both arms, so
    /// the skipped block is recovered too.
    #[test]
    fn a_conditional_branch_keeps_both_successors() {
        let mut text = vec![0u8; 0x18];
        put(&mut text, 0x00, 0xB500_0040); // cbnz x0, 0x0c
        put(&mut text, 0x04, 0xAA03_03E0); // mov x0, x3
        put(&mut text, 0x08, 0xD65F_03C0); // ret
        put(&mut text, 0x0c, 0xD65F_03C0); // ret

        let boundaries = BTreeSet::new();
        let f = build(&text, &[], 0, 4096, &boundaries).expect("function must lift");
        assert_eq!(f.blocks.len(), 3);
        assert!(f.complete);
    }

    /// A budget too small to reach the terminator is reported as truncated, not
    /// as a complete function.
    #[test]
    fn a_tiny_budget_is_reported_as_partial() {
        let mut text = vec![0u8; 0x40];
        for i in (0..0x3c).step_by(4) {
            put(&mut text, i, 0x9100_4020); // add x0, x1, #0x10
        }
        put(&mut text, 0x3c, 0xD65F_03C0);

        let boundaries = BTreeSet::new();
        let f = build(&text, &[], 0, 8, &boundaries).expect("function must lift");
        assert!(!f.complete);
        assert!(f.budget_exhausted);
    }
}
