//! `ew-lift` — the static recompilation pipeline.
//!
//! Pipeline: decode AArch64 words -> discover dispatchable addresses -> recover
//! basic blocks and their control-flow edges (including jump tables) -> emit C
//! that dispatches between blocks internally and returns to the runtime
//! dispatcher at every edge that leaves the function.
//!
//! Two independent implementations of the same instruction semantics exist on
//! purpose: [`interpreter`] (used as a reference for differential tests) and
//! [`lift`] (the shipped C). Agreement between them is evidence; the interpreter
//! is not part of the product runtime.

pub mod aarch64;
pub mod cfg;
pub mod interpreter;
pub mod jump_table;
pub mod lift;
pub mod scan;
pub mod simd;

pub use aarch64::{decode, Cond, DataOp2, Extend, LogicalOp, MemMode, Op, Shift};
pub use interpreter::{HostArena, Program, State, Transfer};
pub use jump_table::JumpTable;
pub use interpreter::{StepRecord, ThreadInfo};
pub use lift::{
    dispatch_header, emit_campaign, emit_harness, emit_module, emit_simd_campaign, simd_seed_vector,
    RUNTIME_HEADER,
};
pub use scan::{
    discover_starts, discover_starts_in, lift_function, lift_function_in, Block, Edge, EdgeKind,
    Insn, LiftedFunction,
};

use serde::Serialize;

#[derive(Debug, Clone)]
pub struct LiftOptions {
    pub module: String,
    pub entry: u64,
    pub max_functions: usize,
    pub max_insns: usize,
    pub pure_leaf_only: bool,
    /// Keep a block only in the function whose start is the closest one at
    /// or below it. Control flow is followed from every start, so without
    /// this the same code is lifted into every function that reaches it
    /// (about fifty times over for the whole game).
    pub own_blocks: bool,
}

impl Default for LiftOptions {
    fn default() -> Self {
        LiftOptions {
            module: "module".to_string(),
            entry: 0,
            max_functions: 512,
            // Per-function budget, not a per-block cap. A budget of 256 truncated
            // most real functions before their switch blocks were reached, which
            // made control-flow recovery look far worse than it is. The block
            // ceiling in `cfg` is what stops a bad edge walking the module.
            max_insns: 4096,
            pure_leaf_only: false,
            own_blocks: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LiftReport {
    pub module: String,
    pub text_bytes: usize,
    pub text_words: usize,
    /// All-zero words. AArch64 leaves alignment gaps between functions, so these
    /// are padding, not code. They are excluded from the coverage ratio instead
    /// of being counted as undecoded instructions.
    pub padding_words: usize,
    /// Words that decode to a supported instruction across the whole segment.
    pub decodable_words: usize,
    pub unsupported_words: usize,
    /// Fraction of the real (non-padding) text segment that decodes to
    /// supported instructions.
    pub decode_coverage: f64,
    pub candidate_starts: usize,
    pub lifted_functions: usize,
    pub complete_functions: usize,
    pub pure_leaf_functions: usize,
    pub blocks_lifted: usize,
    pub instructions_lifted: usize,
    pub unsupported_in_lifted: usize,
    pub jump_tables: usize,
    pub jump_table_targets: usize,
    pub unresolved_edges: usize,
    /// Functions left partial because their instruction budget ran out.
    pub incomplete_budget: usize,
    /// Functions left partial because a block had no terminator.
    pub incomplete_unterminated: usize,
    /// Functions left partial because an indirect edge could not be resolved.
    pub incomplete_indirect: usize,
    pub limitations: Vec<String>,
}

/// Lift a text segment.
pub fn lift_text(text: &[u8], opts: &LiftOptions) -> (LiftReport, Vec<LiftedFunction>) {
    lift_text_in(text, &[], opts)
}

/// Lift a text segment with the rest of the module image available, so jump
/// tables stored in rodata can be resolved.
pub fn lift_text_in(
    text: &[u8],
    segments: jump_table::Segments,
    opts: &LiftOptions,
) -> (LiftReport, Vec<LiftedFunction>) {
    let text_words = text.len() / 4;
    let mut decodable = 0usize;
    let mut padding = 0usize;
    for (addr, word) in scan::words(text) {
        if word == 0 {
            padding += 1;
            continue;
        }
        if !matches!(
            decode(word, addr),
            Op::Unsupported { .. } | Op::UnsupportedBitfield { .. }
        ) {
            decodable += 1;
        }
    }
    let real_words = text_words.saturating_sub(padding);

    // Computed once: rebuilding the call-target set per function would be
    // quadratic in the size of the module. Prologue entries are boundaries too,
    // and without them the traversal walks straight into the next function and
    // exhausts its budget on code it does not own.
    let mut boundaries = scan::call_targets(text);
    boundaries.extend(scan::prologue_starts(text, 262_144));

    let candidates =
        discover_starts_in(text, segments, opts.entry, opts.max_functions.saturating_mul(8).max(64));
    let candidate_count = candidates.len();

    let mut sorted: Vec<u64> = candidates.clone();
    sorted.sort_unstable();
    sorted.dedup();
    let region_owner = |a: u64| match sorted.binary_search(&a) {
        Ok(i) => sorted[i],
        Err(0) => u64::MAX,
        Err(i) => sorted[i - 1],
    };
    let mut funcs = Vec::new();
    for start in candidates {
        if funcs.len() >= opts.max_functions {
            break;
        }
        if let Some(mut f) = cfg::build(text, segments, start, opts.max_insns, &boundaries) {
            if opts.pure_leaf_only && !f.is_pure_leaf() {
                continue;
            }
            if opts.own_blocks {
                f.blocks.retain(|b| region_owner(b.addr) == start);
                if f.blocks.is_empty() {
                    continue;
                }
            }
            funcs.push(f);
        }
    }

    let report = LiftReport {
        module: opts.module.clone(),
        text_bytes: text.len(),
        text_words,
        padding_words: padding,
        decodable_words: decodable,
        unsupported_words: real_words.saturating_sub(decodable),
        decode_coverage: if real_words == 0 {
            0.0
        } else {
            decodable as f64 / real_words as f64
        },
        candidate_starts: candidate_count,
        lifted_functions: funcs.len(),
        complete_functions: funcs.iter().filter(|f| f.complete).count(),
        pure_leaf_functions: funcs.iter().filter(|f| f.is_pure_leaf()).count(),
        blocks_lifted: funcs.iter().map(|f| f.blocks.len()).sum(),
        instructions_lifted: funcs.iter().map(|f| f.insn_count()).sum(),
        unsupported_in_lifted: funcs.iter().map(|f| f.unsupported).sum(),
        jump_tables: funcs.iter().map(|f| f.jump_tables).sum(),
        jump_table_targets: funcs.iter().map(|f| f.jump_table_targets).sum(),
        unresolved_edges: funcs.iter().map(|f| f.unresolved_edges).sum(),
        incomplete_budget: funcs.iter().filter(|f| !f.complete && f.budget_exhausted).count(),
        incomplete_unterminated: funcs
            .iter()
            .filter(|f| !f.complete && !f.budget_exhausted && f.unterminated_blocks > 0)
            .count(),
        incomplete_indirect: funcs
            .iter()
            .filter(|f| !f.complete && !f.budget_exhausted && f.unresolved_edges > 0)
            .count(),
        limitations: vec![
            "indirect branches without a recognisable table remain unresolved and return to the dispatcher"
                .to_string(),
            "function entry points are inferred from call and branch targets, not from debug metadata"
                .to_string(),
            "SIMD/FP instructions are still decoded as unsupported and trap when reached".to_string(),
            "zero words are counted as padding and excluded from the coverage ratio".to_string(),
        ],
    };
    (report, funcs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words_to_bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn lifts_synthetic_segment_and_reports_coverage() {
        // The zero word is alignment padding, so the coverage ratio is measured
        // over the four real words and is therefore total.
        let text = words_to_bytes(&[
            0x9100_4020, // 0x00: add x0, x1, #0x10
            0xAA01_03E0, // 0x04: mov x0, x1
            0xD65F03C0,  // 0x08: ret
            0x0000_0000, // 0x0c: unsupported
            0xD65F03C0,  // 0x10: ret
        ]);
        let opts = LiftOptions { module: "synthetic".into(), ..Default::default() };
        let (report, funcs) = lift_text(&text, &opts);
        assert_eq!(report.text_words, 5);
        assert_eq!(report.padding_words, 1);
        assert_eq!(report.unsupported_words, 0);
        assert_eq!(report.decodable_words, 4);
        assert!((report.decode_coverage - 1.0).abs() < 1e-9);
        assert_eq!(report.lifted_functions, funcs.len());
        assert!(funcs.iter().any(|f| f.addr == 0 && f.complete));
        assert_eq!(report.blocks_lifted, 1, "one block, ending at the ret");
    }

    /// `ldadd x9, x8, [x8]` stores old + x9 and returns the old value in x8.
    /// The emitter once stored `x9 & (old + x9)` and returned x9.
    #[test]
    fn atomic_add_returns_the_old_value() {
        let text = words_to_bytes(&[0xF829_0108, 0xD65F03C0]);
        let opts = LiftOptions { module: "t".into(), ..Default::default() };
        let (_, funcs) = lift_text(&text, &opts);
        let c = emit_module("t", &funcs);
        assert!(c.contains("ew_u64 new_ = old_ + v_;"), "{c}");
        assert!(c.contains("ew_u64 v_ = (EW_R(cpu, 9, 0)) & ~0ULL;"), "{c}");
        assert!(c.contains("old_);"), "the destination gets the old value: {c}");
    }

    /// Differential test: the interpreter and the emitter share only the decoder.
    #[test]
    fn interpreter_follows_branches() {
        let text = words_to_bytes(&[
            0xAA02_03E0, // 0x00: mov x0, x2
            0xB500_0040, // 0x04: cbnz x0, 0x0c
            0xAA03_03E0, // 0x08: mov x0, x3
            0xD65F03C0, // 0x0c: ret
            0xD65F03C0, // 0x10: ret (unreached)
        ]);
        let f = lift_function(&text, 0, 256).unwrap();
        assert!(f.complete);
        assert_eq!(f.blocks.len(), 3);
        let mut st = State::patterned();
        interpreter::run_function(&mut st, &f, 64).unwrap();
        assert_eq!(st.x[0], st.x[2], "cbz taken skips the mov x0, x3");
        assert_eq!(st.pc, st.x[30], "left the function through ret");
    }
}
