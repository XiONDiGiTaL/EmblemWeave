//! Jump-table recovery.
//!
//! The compilers used for this title emit the same switch shape every time:
//!
//! ```text
//! cmp   w8, #0x10
//! b.hi  .Ldefault
//! adr   x9, .LJTI              (or adrp x9, page / add x9, x9, #lo)
//! ldrb  w10, [x9, w8, uxtw]    (or ldrh, with lsl #1)
//! add   x9, x9, w10, lsl #2    (or sxtw #2 for signed byte offsets)
//! br    x9
//! ```
//!
//! This module recognises that shape in a decoded instruction window and reads
//! the actual entries out of the text image, so an indirect `br` becomes a set
//! of concrete successor blocks. Anything that does not fit is left alone and
//! reported as an unresolved edge: a wrong table would silently corrupt the
//! graph, which is worse than an honest gap.

use crate::aarch64::{decode, Cond, Op, Shift};
use crate::scan::{word_at, Insn};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct JumpTable {
    /// Address of the first table entry in the text image.
    pub table: u64,
    /// Base address that entry offsets are relative to (the `adr` target).
    pub base: u64,
    pub entries: usize,
    pub entry_bytes: u8,
    pub shift: u8,
    pub signed: bool,
    pub default: Option<u64>,
    pub targets: Vec<u64>,
}

/// Constants established by a function prologue, used to seed register
/// resolution. Compilers frequently build one table base in the prologue and
/// reuse it for every switch in the function.
pub type Known = std::collections::BTreeMap<u8, u64>;

/// Collect simple constant register values from a linear scan.
pub fn prologue_constants(text: &[u8], start: u64, max_insns: usize) -> Known {
    let mut known = Known::new();
    let mut addr = start;
    for _ in 0..max_insns {
        let word = match crate::scan::word_at(text, addr) {
            Some(w) => w,
            None => break,
        };
        match decode(word, addr) {
            Op::Adr { rd, target } => {
                known.insert(rd, target);
            }
            Op::AddSubImm { rd, rn, imm, sub: false, .. } if rd == rn => {
                if let Some(v) = known.get(&rn).copied() {
                    known.insert(rd, v.wrapping_add(imm as u64));
                }
            }
            Op::MoveWide { rd, imm, shift, kind: 0, is64 } => {
                let v = if is64 {
                    (imm as u64) << shift
                } else {
                    ((imm as u64) << shift) & 0xFFFF_FFFF
                };
                known.insert(rd, v);
            }
            Op::LoadStore { rt, .. } | Op::LoadStoreReg { rt, .. } | Op::LoadLiteral { rt, .. } => {
                known.remove(&rt);
            }
            Op::LoadStorePair { rt, rt2, .. } => {
                known.remove(&rt);
                known.remove(&rt2);
            }
            Op::LogicalReg { rd, .. }
            | Op::LogicalImm { rd, .. }
            | Op::AddSubReg { rd, .. }
            | Op::AddSubExt { rd, .. }
            | Op::MultiplyAdd { rd, .. }
            | Op::MulLong { rd, .. }
            | Op::CondSelect { rd, .. }
            | Op::DataProc2 { rd, .. } => {
                known.remove(&rd);
            }
            _ => {}
        }
        addr += 4;
    }
    known
}

/// Slice backwards to the nearest definition of `reg` and return the constant
/// it was built from.
///
/// Walking forwards and requiring *every* write in the window to be understood
/// was wrong: a function reuses its scratch registers, so any later unrelated
/// write to the same number erased a perfectly good table base. Slicing stops at
/// the first definition, which is the one that actually reaches the use, and
/// follows it through `adr`/`adrp` plus `add` immediate pairs.
fn resolve_register(insns: &[Insn], reg: u8, known: &Known) -> Option<u64> {
    if reg == 31 {
        return None;
    }
    let mut tracked = reg;
    let mut addend: i64 = 0;
    for insn in insns.iter().rev() {
        match insn.op {
            Op::Adr { rd, target } if rd == tracked => {
                return Some((target as i64).wrapping_add(addend) as u64);
            }
            Op::AddSubImm { rd, rn, imm, sub: false, set_flags: false, .. } if rd == tracked => {
                if rn == 31 {
                    return None;
                }
                addend = addend.wrapping_add(imm as i64);
                tracked = rn;
            }
            Op::MoveWide { rd, imm, shift, kind: 0, .. } if rd == tracked => {
                return Some(((imm as i64) << shift).wrapping_add(addend) as u64);
            }
            Op::AddSubImm { rd, .. }
            | Op::LogicalReg { rd, .. }
            | Op::LogicalImm { rd, .. }
            | Op::AddSubReg { rd, .. }
            | Op::AddSubExt { rd, .. }
            | Op::MultiplyAdd { rd, .. }
            | Op::MulLong { rd, .. }
            | Op::CondSelect { rd, .. }
            | Op::DataProc2 { rd, .. }
            | Op::MoveWide { rd, .. }
            | Op::Adr { rd, .. }
            | Op::LoadLiteral { rt: rd, .. }
            | Op::LoadStore { rt: rd, .. }
            | Op::LoadStoreReg { rt: rd, .. }
                if rd == tracked =>
            {
                return None;
            }
            Op::LoadStorePair { rt, rt2, .. } if rt == tracked || rt2 == tracked => return None,
            _ => {}
        }
    }
    known.get(&reg).copied().or_else(|| {
        // A base recorded by the prologue scan resolves directly.
        if tracked == reg { None } else { known.get(&tracked).copied().map(|v| (v as i64).wrapping_add(addend) as u64) }
    })
}

/// Mapped read-only segments besides text, as (module address, bytes). Jump
/// tables are usually emitted into rodata, so reading one requires the module
/// image rather than the text file alone.
pub type Segments<'a> = &'a [(u64, &'a [u8])];

fn slice_at<'a>(text: &'a [u8], segments: Segments<'a>, at: u64, len: usize) -> Option<&'a [u8]> {
    let off = at as usize;
    if off.saturating_add(len) <= text.len() {
        return Some(&text[off..off + len]);
    }
    for (base, data) in segments {
        if at >= *base {
            let rel = (at - *base) as usize;
            if rel.saturating_add(len) <= data.len() {
                return Some(&data[rel..rel + len]);
            }
        }
    }
    None
}

fn read_entry(text: &[u8], segments: Segments, at: u64, bytes: u8) -> Option<u64> {
    let raw = slice_at(text, segments, at, bytes as usize)?;
    Some(match bytes {
        1 => raw[0] as u64,
        2 => u16::from_le_bytes([raw[0], raw[1]]) as u64,
        4 => u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as u64,
        _ => return None,
    })
}

/// Recognise a jump table whose `br` is the last instruction of `insns`.
pub fn detect(
    text: &[u8],
    segments: Segments,
    known: &Known,
    insns: &[Insn],
) -> Option<JumpTable> {
    detect_with_reason(text, segments, known, insns).ok()
}

/// Why a `br` was not recognised as a jump table. Reported by the `ew-tables`
/// diagnostic so a missing instruction pattern is measured, not guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unresolved {
    NotABranch,
    NoIndexAdd,
    BaseUnknown,
    NoTableLoad,
    TableUnknown,
    NoBoundsCheck,
    NoEntries,
    BadDefault,
    BadTarget,
    Empty,
}

impl Unresolved {
    pub fn as_str(self) -> &'static str {
        match self {
            Unresolved::NotABranch => "last instruction is not br",
            Unresolved::NoIndexAdd => "no add of a loaded offset to a base",
            Unresolved::BaseUnknown => "table base register is not a known constant",
            Unresolved::NoTableLoad => "no load from the table base",
            Unresolved::TableUnknown => "table address register is not a known constant",
            Unresolved::NoBoundsCheck => "no cmp/subs bound before the branch",
            Unresolved::NoEntries => "bound check is not a hi/cs comparison",
            Unresolved::BadDefault => "default target is not code",
            Unresolved::BadTarget => "a table entry does not land on code",
            Unresolved::Empty => "table has no entries",
        }
    }
}

pub fn detect_with_reason(
    text: &[u8],
    segments: Segments,
    known: &Known,
    insns: &[Insn],
) -> Result<JumpTable, Unresolved> {
    let last = insns.last().ok_or(Unresolved::NotABranch)?;
    let br_reg = match last.op {
        Op::Br { rn } => rn,
        _ => return Err(Unresolved::NotABranch),
    };

    // The instruction that adds the table entry to the base. Compilers use
    // either the extended-register form (`add x9, x9, w8, lsl #2`) or the plain
    // shifted-register form (`add x10, x10, w11, lsl #2`); both appear here.
    // Two shapes are accepted: `add x9, x9, w10, lsl #2`, where the base is the
    // branch register itself, and `add x9, x8, w10, lsl #2`, where the base is a
    // separate register and the branch register is only the sum.
    let (add_idx, index_reg, base_reg, shift, signed) =
        insns.iter().enumerate().rev().find_map(|(i, insn)| match insn.op {
            Op::AddSubExt { rd, rn, rm, option, shift, sub: false, set_flags: false, .. }
                if rd == br_reg && rn != 31 =>
            {
                Some((i, rm, rn, shift as u32, option.is_signed()))
            }
            Op::AddSubReg {
                rd,
                rn,
                rm,
                shift: Shift::Lsl,
                amount,
                sub: false,
                set_flags: false,
                ..
            } if rd == br_reg && rn != 31 => Some((i, rm, rn, amount as u32, false)),
            _ => None,
        })
        .ok_or(Unresolved::NoIndexAdd)?;

    let base = resolve_register(&insns[..add_idx], base_reg, known).ok_or(Unresolved::BaseUnknown)?;

    // The table load: `ldrb wI, [xTable, xIndex, extend]`.
    let (load_idx, table_reg, index_src, entry_bytes) =
        insns[..add_idx].iter().enumerate().rev().find_map(|(i, insn)| match insn.op {
            Op::LoadStoreReg { rt, rn, rm, bytes, load: true, .. } if rt == index_reg => {
                Some((i, rn, rm, bytes))
            }
            _ => None,
        })
        .ok_or(Unresolved::NoTableLoad)?;

    let table = resolve_register(&insns[..load_idx], table_reg, known)
        .ok_or(Unresolved::TableUnknown)?;

    // The index is usually copied into a scratch register first
    // (`mov w8, w0`), and the range check is then emitted against the original
    // argument. Follow that copy, or the bound looks absent when it is right
    // there.
    let mut bound_regs = vec![index_src];
    if let Some(source) = insns[..load_idx].iter().rev().find_map(|insn| match insn.op {
        Op::LogicalReg { rd, rn: 31, rm, shift: Shift::Lsl, amount: 0, op: crate::aarch64::LogicalOp::Orr, set_flags: false, is64, invert: false } if rd == index_src => {
            let _ = is64;
            Some(rm)
        }
        Op::LogicalReg { rd, .. } if rd == index_src => None,
        _ => None,
    }) {
        if source != 31 {
            bound_regs.push(source);
        }
    }

    // `cmp wIndex, #bound` plus the branch that excludes larger values.
    let mut entries = None;
    let mut default = None;
    if let Some((cmp_idx, bound)) = insns[..load_idx].iter().enumerate().rev().find_map(|(i, insn)| {
        match insn.op {
            Op::AddSubImm { rd: 31, rn, imm, sub: true, set_flags: true, .. }
                if bound_regs.contains(&rn) =>
            {
                Some((i, imm as u64))
            }
            _ => None,
        }
    }) {
        if let Some(Op::BCond { cond, target }) = insns.get(cmp_idx + 1).map(|i| i.op) {
            match cond {
                Cond::Hi => entries = Some(bound + 1),
                Cond::Cs => entries = Some(bound),
                _ => {}
            }
            default = Some(target);
        }
    }

    let entries = entries.ok_or(Unresolved::NoEntries)?;
    if entries == 0 {
        return Err(Unresolved::Empty);
    }
    if entries > 2048 {
        return Err(Unresolved::NoEntries);
    }
    // The default target is part of the same switch, so it has to be real code.
    if let Some(target) = default {
        if word_at(text, target).is_none() {
            return Err(Unresolved::BadDefault);
        }
    }

    let mut targets = Vec::with_capacity(entries as usize);
    for i in 0..entries {
        let at = table
            .checked_add(i * entry_bytes as u64)
            .ok_or(Unresolved::TableUnknown)?;
        let raw = read_entry(text, segments, at, entry_bytes).ok_or(Unresolved::TableUnknown)?;
        let extended = if signed {
            match entry_bytes {
                1 => (raw as u8 as i8) as i64,
                2 => (raw as u16 as i16) as i64,
                _ => (raw as u32 as i32) as i64,
            }
        } else {
            raw as i64
        };
        let target = (base as i64)
            .wrapping_add(extended.wrapping_shl(shift))
            .max(0) as u64;
        // Every successor must be a real, aligned instruction in the image.
        if target % 4 != 0 || word_at(text, target).is_none() || word_at(text, target) == Some(0) {
            return Err(Unresolved::BadTarget);
        }
        targets.push(target);
    }

    targets.sort_unstable();
    targets.dedup();

    Ok(JumpTable {
        table,
        base,
        entries: entries as usize,
        entry_bytes,
        shift: shift as u8,
        signed,
        default,
        targets,
    })
}


/// How far back a jump-table search may look for the code that built the table
/// base. Compilers put that in the function prologue, so the window has to reach
/// past the block that ends in `br`.
pub const CONTEXT_WORDS: u64 = 96;

/// Decode `window` words of context before `addr` and return them followed by
/// the words from `addr` to `end` inclusive.
pub fn context_window(text: &[u8], addr: u64, end: u64) -> Vec<Insn> {
    let mut insns = Vec::new();
    let start = addr.saturating_sub(CONTEXT_WORDS * 4);
    let mut at = start;
    while at <= end {
        if let Some(word) = word_at(text, at) {
            insns.push(Insn { addr: at, word, op: decode(word, at) });
        }
        at += 4;
    }
    insns
}

/// Convenience for address discovery: try to resolve the table that feeds the
/// `br` at `br_addr` using a window of preceding instructions.
pub fn targets_at(text: &[u8], segments: Segments, br_addr: u64) -> Vec<u64> {
    let insns = context_window(text, br_addr, br_addr);
    detect(text, segments, &Known::new(), &insns)
        .map(|t| t.targets)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-assembled switch in the exact shape clang produces. The table holds
    /// signed byte offsets, scaled by four, relative to `base`.
    fn switch_image() -> Vec<u8> {
        let words: [u32; 10] = [
            0x7100_0D1F, // 0x00: cmp w8, #3
            0x5400_0128, // 0x04: b.hi 0x28 (default)
            0x1000_0109, // 0x08: adr x9, 0x28   (base = table start)
            0x3868_692A, // 0x0c: ldrb w10, [x9, x8, uxtw]
            0x8B2A_C929, // 0x10: add x9, x9, w10, sxtw #2
            0xD61F_0120, // 0x14: br x9
            0xD503_201F, // 0x18: nop
            0xD503_201F, // 0x1c: nop
            0xD503_201F, // 0x20: nop
            0xD503_201F, // 0x24: nop
        ];
        let mut text: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        // Table at 0x28: signed byte offsets relative to 0x28, scaled by four.
        // 6, 8, 10, 12 scale to 0x18, 0x20, 0x28, 0x30.
        text.extend_from_slice(&[6, 8, 10, 12]);
        text.extend_from_slice(&[0xD5, 0x03, 0x20, 0x1F]); // 0x2c
        text.extend_from_slice(&[0xD5, 0x03, 0x20, 0x1F]); // 0x30
        text.extend_from_slice(&[0xD5, 0x03, 0x20, 0x1F]); // 0x34
        text.extend_from_slice(&[0xD5, 0x03, 0x20, 0x1F]); // 0x38
        text.extend_from_slice(&[0xD5, 0x03, 0x20, 0x1F]); // 0x3c
        text.extend_from_slice(&[0xD6, 0x5F, 0x03, 0xC0]); // 0x40: ret
        text.extend_from_slice(&[0xD6, 0x5F, 0x03, 0xC0]); // 0x44
        text.extend_from_slice(&[0xD6, 0x5F, 0x03, 0xC0]); // 0x48: ret
        text.extend_from_slice(&[0xD6, 0x5F, 0x03, 0xC0]); // 0x4c
        text.extend_from_slice(&[0xD6, 0x5F, 0x03, 0xC0]); // 0x50: ret
        text.extend_from_slice(&[0xD6, 0x5F, 0x03, 0xC0]); // 0x54
        text.extend_from_slice(&[0xD6, 0x5F, 0x03, 0xC0]); // 0x58: ret
        text
    }

    #[test]
    fn resolves_a_signed_byte_table() {
        let text = switch_image();
        let insns: Vec<Insn> = (0..6u64)
            .filter_map(|i| {
                let addr = i * 4;
                word_at(&text, addr).map(|word| Insn { addr, word, op: decode(word, addr) })
            })
            .collect();
        let table = detect(&text, &[], &Known::new(), &insns).expect("table must resolve");
        assert_eq!(table.table, 0x28);
        assert_eq!(table.base, 0x28);
        assert_eq!(table.entries, 4);
        assert_eq!(table.entry_bytes, 1);
        assert_eq!(table.shift, 2);
        assert!(table.signed);
        assert_eq!(table.default, Some(0x28));
        assert_eq!(table.targets, vec![0x40, 0x48, 0x50, 0x58]);
    }

    #[test]
    fn rejects_a_table_with_a_bad_target() {
        let mut text = switch_image();
        // point the first entry outside the image
        text[0x28] = 0x7F;
        let insns: Vec<Insn> = (0..6u64)
            .filter_map(|i| {
                let addr = i * 4;
                word_at(&text, addr).map(|word| Insn { addr, word, op: decode(word, addr) })
            })
            .collect();
        assert!(detect(&text, &[], &Known::new(), &insns).is_none());
    }

    #[test]
    fn ignores_a_plain_indirect_branch() {
        let text = switch_image();
        let insns: Vec<Insn> = (5..6u64)
            .filter_map(|i| {
                let addr = i * 4;
                word_at(&text, addr).map(|word| Insn { addr, word, op: decode(word, addr) })
            })
            .collect();
        assert!(detect(&text, &[], &Known::new(), &insns).is_none());
    }
}
