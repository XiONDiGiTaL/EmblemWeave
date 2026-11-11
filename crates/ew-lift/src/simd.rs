//! Advanced SIMD (NEON): decoding and reference semantics.
//!
//! The retail module spends most of its undecoded words in the vector unit:
//! element copies, lane arithmetic, permutes, and structure loads. This module
//! decodes those families into [`SimdOp`] and [`SimdMem`], and executes them
//! for the reference interpreter.
//!
//! The C emitter does not reuse anything here. It calls the helpers in
//! `ew_simd.h`, a second implementation written against the same Arm pseudocode,
//! so a differential run compares two independent readings of the architecture.
//!
//! Floating point follows the architecture with the default FPCR (round to
//! nearest even, no flush to zero, no default-NaN mode): NaN operands propagate
//! by the Arm priority rule (signalling first, then quiet, first operand before
//! second), and an operation that creates a NaN returns the Arm default NaN,
//! not the host's.

use crate::aarch64::Op;
use crate::interpreter::State;

/// Three-register operations (`three same`, plus the pairwise forms).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[repr(u8)]
pub enum V3 {
    Add = 0,
    Sub,
    Mul,
    Mla,
    Mls,
    And,
    Bic,
    Orr,
    Orn,
    Eor,
    Bsl,
    Bit,
    Bif,
    CmEq,
    CmGt,
    CmGe,
    CmHi,
    CmHs,
    CmTst,
    SMax,
    SMin,
    UMax,
    UMin,
    SAbd,
    UAbd,
    SShl,
    UShl,
    AddP,
    SMaxP,
    SMinP,
    UMaxP,
    UMinP,
    FAdd = 64,
    FSub,
    FMul,
    FDiv,
    FMax,
    FMin,
    FMaxNm,
    FMinNm,
    FMla,
    FMls,
    FAbd,
    FMulX,
    FCmEq,
    FCmGe,
    FCmGt,
    FAcGe,
    FAcGt,
    FRecps,
    FRsqrts,
    FAddP,
    FMaxP,
    FMinP,
    FMaxNmP,
    FMinNmP,
}

impl V3 {
    pub fn is_pairwise(self) -> bool {
        matches!(
            self,
            V3::AddP
                | V3::SMaxP
                | V3::SMinP
                | V3::UMaxP
                | V3::UMinP
                | V3::FAddP
                | V3::FMaxP
                | V3::FMinP
                | V3::FMaxNmP
                | V3::FMinNmP
        )
    }
    pub fn is_fp(self) -> bool {
        self as u8 >= 64
    }
}

/// Two-register operations (`two-register miscellaneous`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[repr(u8)]
pub enum V2 {
    Rev16 = 0,
    Rev32,
    Rev64,
    Cnt,
    Not,
    Rbit,
    Clz,
    Cls,
    Abs,
    Neg,
    CmGt0,
    CmGe0,
    CmEq0,
    CmLe0,
    CmLt0,
    /// `xtn`/`xtn2`: `esize` is the narrow destination element.
    Xtn,
    /// `saddlp`/`uaddlp`: `esize` is the wide destination element.
    SAddLp,
    UAddLp,
    FAbs = 64,
    FNeg,
    FSqrt,
    FRintN,
    FRintP,
    FRintM,
    FRintZ,
    FRintA,
    FRintX,
    FRintI,
    FCvtNs,
    FCvtPs,
    FCvtMs,
    FCvtZs,
    FCvtAs,
    FCvtNu,
    FCvtPu,
    FCvtMu,
    FCvtZu,
    FCvtAu,
    SCvtF,
    UCvtF,
    FCmGt0,
    FCmGe0,
    FCmEq0,
    FCmLe0,
    FCmLt0,
    /// `fcvtn`/`fcvtn2`, double to single. `esize` is 4.
    FCvtN,
    /// `fcvtl`/`fcvtl2`, single to double. `esize` is 8.
    FCvtL,
    /// `frecpe`: reciprocal estimate, Arm's 8-bit table.
    FRecpE,
    /// `frsqrte`: reciprocal square root estimate, Arm's 8-bit table.
    FRsqrtE,
}

/// Shift by immediate. `esize` is always the destination element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[repr(u8)]
pub enum VSh {
    SShr = 0,
    UShr,
    SSra,
    USra,
    Shl,
    Sri,
    Sli,
    /// `shrn`: narrow destination, wide source.
    Shrn,
    /// `sshll`/`ushll` (and `sxtl`/`uxtl`): wide destination.
    SShll,
    UShll,
    /// Fixed point conversions; `shift` is the fraction bit count.
    SCvtF,
    UCvtF,
    FCvtZs,
    FCvtZu,
}

/// Three registers of different widths. `esize` is the wide element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[repr(u8)]
pub enum VLong {
    AddL = 0,
    AddW,
    SubL,
    SubW,
    AbdL,
    MlaL,
    MlsL,
    MulL,
}

/// By-element operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[repr(u8)]
pub enum VElem {
    Mul = 0,
    Mla,
    Mls,
    /// `smull`/`umull` by element; `esize` is the narrow source element.
    SMull,
    UMull,
    SMlal,
    UMlal,
    FMul = 64,
    FMla,
    FMls,
    FMulX,
}

/// Reductions across a vector. `esize` is the source element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[repr(u8)]
pub enum VAcross {
    AddV = 0,
    SMaxV,
    SMinV,
    UMaxV,
    UMinV,
    SAddLv,
    UAddLv,
    FMaxV = 64,
    FMinV,
    FMaxNmV,
    FMinNmV,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[repr(u8)]
pub enum VPerm {
    Uzp1 = 0,
    Uzp2,
    Trn1,
    Trn2,
    Zip1,
    Zip2,
}

/// What a modified-immediate instruction does with its expanded immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[repr(u8)]
pub enum ImmKind {
    Mov = 0,
    Orr,
    Bic,
}

/// A register-only Advanced SIMD instruction.
///
/// `esize` is an element size in bytes (1, 2, 4 or 8). `q` selects the 128-bit
/// form; a 64-bit form zeroes the upper half of the destination. For the long
/// and narrow forms `q` instead selects the upper half (the `2` suffix).
/// `scalar` forms operate on lane 0 and zero the rest of the register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum SimdOp {
    DupElem { rd: u8, rn: u8, esize: u8, index: u8, q: bool },
    DupGen { rd: u8, rn: u8, esize: u8, q: bool },
    InsGen { rd: u8, rn: u8, esize: u8, index: u8 },
    InsElem { rd: u8, rn: u8, esize: u8, dst: u8, src: u8 },
    /// `umov`/`smov` (and `mov w0, v1.s[1]`).
    ToGp { rd: u8, rn: u8, esize: u8, index: u8, signed: bool, is64: bool },
    /// `mov s0, v1.s[3]`, the scalar `dup`.
    DupScalar { rd: u8, rn: u8, esize: u8, index: u8 },
    Ext { rd: u8, rn: u8, rm: u8, pos: u8, q: bool },
    /// `movi`, `mvni`, `orr`, `bic` and vector `fmov` immediates. `imm` is the
    /// expanded 64-bit pattern, already inverted for `mvni`.
    Imm { rd: u8, imm: u64, kind: ImmKind, q: bool },
    Three { rd: u8, rn: u8, rm: u8, op: V3, esize: u8, q: bool, scalar: bool },
    Two { rd: u8, rn: u8, op: V2, esize: u8, q: bool, scalar: bool },
    Shift { rd: u8, rn: u8, op: VSh, esize: u8, shift: u8, q: bool, scalar: bool },
    Long { rd: u8, rn: u8, rm: u8, op: VLong, signed: bool, esize: u8, q: bool },
    Perm { rd: u8, rn: u8, rm: u8, op: VPerm, esize: u8, q: bool },
    Elem { rd: u8, rn: u8, rm: u8, index: u8, op: VElem, esize: u8, q: bool, scalar: bool },
    Across { rd: u8, rn: u8, op: VAcross, esize: u8, q: bool },
    /// Scalar pairwise: `addp d0, v1.2d`, `faddp s0, v1.2s`.
    Pair { rd: u8, rn: u8, op: V3, esize: u8 },
    Tbl { rd: u8, rn: u8, rm: u8, len: u8, tbx: bool, q: bool },
}

/// Post-index writeback of a structure load or store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Post {
    None,
    /// The immediate form: the transfer size.
    Imm(u8),
    /// The register form.
    Reg(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Shape {
    /// `ld1 {v0.4s, v1.4s}`, `ld2`, `ld3`, `ld4`.
    Multi,
    /// `ld1 {v0.s}[2]` and its interleaved forms.
    Lane(u8),
    /// `ld1r`..`ld4r`.
    Replicate,
}

/// An Advanced SIMD structure load or store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SimdMem {
    pub rt: u8,
    pub rn: u8,
    /// Registers in the list.
    pub regs: u8,
    /// Elements per structure (1 for `ld1`, 4 for `ld4`).
    pub selem: u8,
    pub esize: u8,
    pub q: bool,
    pub load: bool,
    pub shape: Shape,
    pub post: Post,
}

impl SimdMem {
    /// Bytes moved, which is the post-index immediate.
    pub fn transfer_bytes(&self) -> u64 {
        match self.shape {
            Shape::Multi => self.regs as u64 * if self.q { 16 } else { 8 },
            Shape::Lane(_) | Shape::Replicate => self.selem as u64 * self.esize as u64,
        }
    }
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Decode an Advanced SIMD word, or return `None` when the word is not one of
/// the modelled forms (including reserved encodings).
pub fn decode(w: u32) -> Option<Op> {
    let rd = (w & 31) as u8;
    let rn = ((w >> 5) & 31) as u8;
    let rm = ((w >> 16) & 31) as u8;
    let q = (w >> 30) & 1 != 0;
    let u = (w >> 29) & 1 != 0;
    let size = ((w >> 22) & 3) as u8;

    // Structure loads and stores.
    if w & 0xBF80_0000 == 0x0C00_0000 || w & 0xBF80_0000 == 0x0C80_0000 {
        return decode_multi(w).map(Op::SimdMem);
    }
    if w & 0xBF80_0000 == 0x0D00_0000 || w & 0xBF80_0000 == 0x0D80_0000 {
        return decode_single(w).map(Op::SimdMem);
    }

    let s = |op| Some(Op::Simd(op));

    // ---- vector forms: 0 Q U 0 1 1 1 0 ----
    if w & 0x9FE0_8400 == 0x0E00_0400 {
        return decode_copy(w, rd, rn, q, u).map(Op::Simd);
    }
    if w & 0xBFE0_8400 == 0x2E00_0000 {
        let pos = ((w >> 11) & 15) as u8;
        if !q && pos >= 8 {
            return None;
        }
        return s(SimdOp::Ext { rd, rn, rm, pos, q });
    }
    if w & 0xBFE0_8C00 == 0x0E00_0000 {
        let len = ((w >> 13) & 3) as u8 + 1;
        return s(SimdOp::Tbl { rd, rn, rm, len, tbx: (w >> 12) & 1 != 0, q });
    }
    if w & 0xBF20_8C00 == 0x0E00_0800 {
        let op = match (w >> 12) & 7 {
            1 => VPerm::Uzp1,
            2 => VPerm::Trn1,
            3 => VPerm::Zip1,
            5 => VPerm::Uzp2,
            6 => VPerm::Trn2,
            7 => VPerm::Zip2,
            _ => return None,
        };
        if size == 3 && !q {
            return None;
        }
        return s(SimdOp::Perm { rd, rn, rm, op, esize: 1 << size, q });
    }
    if w & 0x9F20_0400 == 0x0E20_0400 {
        let opcode = ((w >> 11) & 31) as u8;
        let op = three_same(u, size, opcode)?;
        let esize = three_esize(op, size)?;
        if esize == 8 && !q && !is_logical(op) {
            return None;
        }
        if op.is_fp() && size & 1 == 1 && !q {
            return None;
        }
        if matches!(op, V3::Mul | V3::Mla | V3::Mls | V3::SMaxP | V3::SMinP | V3::UMaxP | V3::UMinP)
            && size == 3
        {
            return None;
        }
        return s(SimdOp::Three { rd, rn, rm, op, esize, q, scalar: false });
    }
    if w & 0x9F20_0C00 == 0x0E20_0000 {
        if size == 3 {
            return None;
        }
        let op = match (w >> 12) & 15 {
            0 => VLong::AddL,
            1 => VLong::AddW,
            2 => VLong::SubL,
            3 => VLong::SubW,
            7 => VLong::AbdL,
            8 => VLong::MlaL,
            0xa => VLong::MlsL,
            0xc => VLong::MulL,
            _ => return None,
        };
        return s(SimdOp::Long { rd, rn, rm, op, signed: !u, esize: 2 << size, q });
    }
    if w & 0x9F3E_0C00 == 0x0E20_0800 {
        let opcode = ((w >> 12) & 31) as u8;
        let (op, esize) = two_misc(u, size, opcode)?;
        // A 64-bit vector of 64-bit lanes is reserved, except for the long and
        // narrow forms whose `esize` names the other operand's width.
        if esize == 8 && !q && !matches!(op, V2::FCvtL | V2::SAddLp | V2::UAddLp | V2::Xtn | V2::FCvtN) {
            return None;
        }
        return s(SimdOp::Two { rd, rn, op, esize, q, scalar: false });
    }
    if w & 0x9F3E_0C00 == 0x0E30_0800 {
        let opcode = ((w >> 12) & 31) as u8;
        let a = size >> 1;
        let (op, esize) = match (u, opcode) {
            (false, 0x03) => (VAcross::SAddLv, 1 << size),
            (true, 0x03) => (VAcross::UAddLv, 1 << size),
            (false, 0x0a) => (VAcross::SMaxV, 1 << size),
            (true, 0x0a) => (VAcross::UMaxV, 1 << size),
            (false, 0x1a) => (VAcross::SMinV, 1 << size),
            (true, 0x1a) => (VAcross::UMinV, 1 << size),
            (false, 0x1b) => (VAcross::AddV, 1 << size),
            (true, 0x0c) if size & 1 == 0 && q => {
                (if a == 0 { VAcross::FMaxNmV } else { VAcross::FMinNmV }, 4)
            }
            (true, 0x0f) if size & 1 == 0 && q => {
                (if a == 0 { VAcross::FMaxV } else { VAcross::FMinV }, 4)
            }
            _ => return None,
        };
        if !(op as u8 >= 64) && (size == 3 || (size == 2 && !q)) {
            return None;
        }
        return s(SimdOp::Across { rd, rn, op, esize, q });
    }

    // ---- 0 Q U 0 1 1 1 1: modified immediate, shift, by element ----
    if w & 0x9FF8_0400 == 0x0F00_0400 {
        return decode_modified_imm(w, rd, q, u).map(Op::Simd);
    }
    if w & 0x9F80_0400 == 0x0F00_0400 {
        return decode_shift(w, rd, rn, q, u, false).map(Op::Simd);
    }
    if w & 0x9F00_0400 == 0x0F00_0000 {
        return decode_elem(w, rd, rn, q, u, false).map(Op::Simd);
    }

    // ---- scalar forms: 0 1 U 1 1 1 1 x ----
    if w & 0xFFE0_FC00 == 0x5E00_0400 {
        let imm5 = (w >> 16) & 31;
        let tz = imm5.trailing_zeros();
        if tz > 3 {
            return None;
        }
        let index = (imm5 >> (tz + 1)) as u8;
        return s(SimdOp::DupScalar { rd, rn, esize: 1 << tz, index });
    }
    if w & 0xDF3E_0C00 == 0x5E30_0800 {
        let opcode = ((w >> 12) & 31) as u8;
        let a = size >> 1;
        let fes = if size & 1 == 1 { 8 } else { 4 };
        let (op, esize) = match (u, opcode) {
            (false, 0x1b) if size == 3 => (V3::AddP, 8),
            (true, 0x0c) => (if a == 0 { V3::FMaxNmP } else { V3::FMinNmP }, fes),
            (true, 0x0d) if a == 0 => (V3::FAddP, fes),
            (true, 0x0f) => (if a == 0 { V3::FMaxP } else { V3::FMinP }, fes),
            _ => return None,
        };
        return s(SimdOp::Pair { rd, rn, op, esize });
    }
    if w & 0xDF20_0400 == 0x5E20_0400 {
        let opcode = ((w >> 11) & 31) as u8;
        let op = three_same(u, size, opcode)?;
        let ok = match op {
            V3::Add | V3::Sub | V3::CmEq | V3::CmGt | V3::CmGe | V3::CmHi | V3::CmHs | V3::CmTst
            | V3::SShl | V3::UShl => size == 3,
            V3::FMulX | V3::FCmEq | V3::FRecps | V3::FRsqrts | V3::FAbd | V3::FCmGe | V3::FCmGt
            | V3::FAcGe | V3::FAcGt => true,
            _ => false,
        };
        if !ok {
            return None;
        }
        let esize = three_esize(op, size)?;
        return s(SimdOp::Three { rd, rn, rm, op, esize, q: false, scalar: true });
    }
    if w & 0xDF3E_0C00 == 0x5E20_0800 {
        let opcode = ((w >> 12) & 31) as u8;
        let (op, esize) = two_misc(u, size, opcode)?;
        let ok = match op {
            V2::CmGt0 | V2::CmGe0 | V2::CmEq0 | V2::CmLe0 | V2::CmLt0 | V2::Abs | V2::Neg => size == 3,
            V2::FCmGt0 | V2::FCmGe0 | V2::FCmEq0 | V2::FCmLe0 | V2::FCmLt0 | V2::FCvtNs
            | V2::FCvtPs | V2::FCvtMs | V2::FCvtZs | V2::FCvtAs | V2::FCvtNu | V2::FCvtPu
            | V2::FCvtMu | V2::FCvtZu | V2::FCvtAu | V2::SCvtF | V2::UCvtF | V2::FRecpE | V2::FRsqrtE => true,
            _ => false,
        };
        if !ok {
            return None;
        }
        return s(SimdOp::Two { rd, rn, op, esize, q: false, scalar: true });
    }
    if w & 0xDF80_0400 == 0x5F00_0400 && (w >> 19) & 15 != 0 {
        return decode_shift(w, rd, rn, false, u, true).map(Op::Simd);
    }
    if w & 0xDF00_0400 == 0x5F00_0000 {
        return decode_elem(w, rd, rn, false, u, true).map(Op::Simd);
    }
    None
}

fn is_logical(op: V3) -> bool {
    matches!(
        op,
        V3::And | V3::Bic | V3::Orr | V3::Orn | V3::Eor | V3::Bsl | V3::Bit | V3::Bif
    )
}

/// Element size of a three-same operation. Logical forms use `size` as part
/// of the opcode, so they work on whole 64-bit lanes.
fn three_esize(op: V3, size: u8) -> Option<u8> {
    if is_logical(op) {
        Some(8)
    } else if op.is_fp() {
        Some(if size & 1 == 1 { 8 } else { 4 })
    } else {
        Some(1 << size)
    }
}

fn three_same(u: bool, size: u8, opcode: u8) -> Option<V3> {
    if opcode >= 0x18 {
        let a = size >> 1;
        return Some(match (u, opcode, a) {
            (false, 0x18, 0) => V3::FMaxNm,
            (false, 0x18, 1) => V3::FMinNm,
            (false, 0x19, 0) => V3::FMla,
            (false, 0x19, 1) => V3::FMls,
            (false, 0x1a, 0) => V3::FAdd,
            (false, 0x1a, 1) => V3::FSub,
            (false, 0x1b, 0) => V3::FMulX,
            (false, 0x1c, 0) => V3::FCmEq,
            (false, 0x1e, 0) => V3::FMax,
            (false, 0x1e, 1) => V3::FMin,
            (false, 0x1f, 0) => V3::FRecps,
            (false, 0x1f, 1) => V3::FRsqrts,
            (true, 0x18, 0) => V3::FMaxNmP,
            (true, 0x18, 1) => V3::FMinNmP,
            (true, 0x1a, 0) => V3::FAddP,
            (true, 0x1a, 1) => V3::FAbd,
            (true, 0x1b, 0) => V3::FMul,
            (true, 0x1c, 0) => V3::FCmGe,
            (true, 0x1c, 1) => V3::FCmGt,
            (true, 0x1d, 0) => V3::FAcGe,
            (true, 0x1d, 1) => V3::FAcGt,
            (true, 0x1e, 0) => V3::FMaxP,
            (true, 0x1e, 1) => V3::FMinP,
            (true, 0x1f, 0) => V3::FDiv,
            _ => return None,
        });
    }
    Some(match (u, opcode) {
        (false, 0x03) => [V3::And, V3::Bic, V3::Orr, V3::Orn][size as usize],
        (true, 0x03) => [V3::Eor, V3::Bsl, V3::Bit, V3::Bif][size as usize],
        (false, 0x06) => V3::CmGt,
        (false, 0x07) => V3::CmGe,
        (true, 0x06) => V3::CmHi,
        (true, 0x07) => V3::CmHs,
        (false, 0x08) => V3::SShl,
        (true, 0x08) => V3::UShl,
        (false, 0x0c) => V3::SMax,
        (true, 0x0c) => V3::UMax,
        (false, 0x0d) => V3::SMin,
        (true, 0x0d) => V3::UMin,
        (false, 0x0e) => V3::SAbd,
        (true, 0x0e) => V3::UAbd,
        (false, 0x10) => V3::Add,
        (true, 0x10) => V3::Sub,
        (false, 0x11) => V3::CmTst,
        (true, 0x11) => V3::CmEq,
        (false, 0x12) => V3::Mla,
        (true, 0x12) => V3::Mls,
        (false, 0x13) => V3::Mul,
        (false, 0x14) => V3::SMaxP,
        (true, 0x14) => V3::UMaxP,
        (false, 0x15) => V3::SMinP,
        (true, 0x15) => V3::UMinP,
        (false, 0x17) => V3::AddP,
        _ => return None,
    })
}

/// Returns the operation and its element size in bytes.
fn two_misc(u: bool, size: u8, opcode: u8) -> Option<(V2, u8)> {
    let a = size >> 1;
    let fes = if size & 1 == 1 { 8 } else { 4 };
    let ies = 1u8 << size;
    let fp = (0x0c..=0x0f).contains(&opcode) || opcode >= 0x16;
    if fp {
        let op = match (u, opcode, a) {
            (false, 0x0c, 1) => V2::FCmGt0,
            (false, 0x0d, 1) => V2::FCmEq0,
            (false, 0x0e, 1) => V2::FCmLt0,
            (false, 0x0f, 1) => V2::FAbs,
            (true, 0x0c, 1) => V2::FCmGe0,
            (true, 0x0d, 1) => V2::FCmLe0,
            (true, 0x0f, 1) => V2::FNeg,
            (true, 0x1f, 1) => V2::FSqrt,
            // Only the double to single narrowing and its widening inverse.
            (false, 0x16, 0) if size & 1 == 1 => return Some((V2::FCvtN, 4)),
            (false, 0x17, 0) if size & 1 == 1 => return Some((V2::FCvtL, 8)),
            (false, 0x18, 0) => V2::FRintN,
            (false, 0x18, 1) => V2::FRintP,
            (false, 0x19, 0) => V2::FRintM,
            (false, 0x19, 1) => V2::FRintZ,
            (false, 0x1a, 0) => V2::FCvtNs,
            (false, 0x1a, 1) => V2::FCvtPs,
            (false, 0x1b, 0) => V2::FCvtMs,
            (false, 0x1b, 1) => V2::FCvtZs,
            (false, 0x1c, 0) => V2::FCvtAs,
            (false, 0x1d, 0) => V2::SCvtF,
            (false, 0x1d, 1) => V2::FRecpE,
            (true, 0x1d, 1) => V2::FRsqrtE,
            (true, 0x18, 0) => V2::FRintA,
            (true, 0x19, 0) => V2::FRintX,
            (true, 0x19, 1) => V2::FRintI,
            (true, 0x1a, 0) => V2::FCvtNu,
            (true, 0x1a, 1) => V2::FCvtPu,
            (true, 0x1b, 0) => V2::FCvtMu,
            (true, 0x1b, 1) => V2::FCvtZu,
            (true, 0x1c, 0) => V2::FCvtAu,
            (true, 0x1d, 0) => V2::UCvtF,
            _ => return None,
        };
        return Some((op, fes));
    }
    let op = match (u, opcode) {
        (false, 0x00) if size < 3 => V2::Rev64,
        (false, 0x01) if size == 0 => V2::Rev16,
        (true, 0x00) if size < 2 => V2::Rev32,
        (false, 0x02) if size < 3 => return Some((V2::SAddLp, ies * 2)),
        (true, 0x02) if size < 3 => return Some((V2::UAddLp, ies * 2)),
        (false, 0x04) if size < 3 => V2::Cls,
        (true, 0x04) if size < 3 => V2::Clz,
        (false, 0x05) if size == 0 => V2::Cnt,
        (true, 0x05) if size == 0 => V2::Not,
        (true, 0x05) if size == 1 => return Some((V2::Rbit, 1)),
        (false, 0x08) => V2::CmGt0,
        (true, 0x08) => V2::CmGe0,
        (false, 0x09) => V2::CmEq0,
        (true, 0x09) => V2::CmLe0,
        (false, 0x0a) => V2::CmLt0,
        (false, 0x0b) => V2::Abs,
        (true, 0x0b) => V2::Neg,
        (false, 0x12) if size < 3 => V2::Xtn,
        _ => return None,
    };
    Some((op, ies))
}

fn decode_copy(w: u32, rd: u8, rn: u8, q: bool, op: bool) -> Option<SimdOp> {
    let imm5 = (w >> 16) & 31;
    let imm4 = (w >> 11) & 15;
    let tz = imm5.trailing_zeros();
    if tz > 3 {
        return None;
    }
    let esize = 1u8 << tz;
    let index = (imm5 >> (tz + 1)) as u8;
    match (op, imm4) {
        (false, 0) if !(esize == 8 && !q) => Some(SimdOp::DupElem { rd, rn, esize, index, q }),
        (false, 1) if !(esize == 8 && !q) => Some(SimdOp::DupGen { rd, rn, esize, q }),
        (false, 5) if esize < 8 && (q || esize < 4) => {
            Some(SimdOp::ToGp { rd, rn, esize, index, signed: true, is64: q })
        }
        (false, 7) if (q && esize == 8) || (!q && esize < 8) => {
            Some(SimdOp::ToGp { rd, rn, esize, index, signed: false, is64: q })
        }
        (false, 3) if q => Some(SimdOp::InsGen { rd, rn, esize, index }),
        (true, _) if q => Some(SimdOp::InsElem {
            rd,
            rn,
            esize,
            dst: index,
            src: (imm4 >> tz) as u8,
        }),
        _ => None,
    }
}

fn decode_modified_imm(w: u32, rd: u8, q: bool, op: bool) -> Option<SimdOp> {
    let cmode = (w >> 12) & 15;
    if (w >> 11) & 1 != 0 {
        return None; // half precision fmov
    }
    let imm8 = ((((w >> 16) & 7) << 5) | ((w >> 5) & 31)) as u64;
    let rep32 = |v: u64| v | (v << 32);
    let rep16 = |v: u64| {
        let v = v & 0xFFFF;
        v | v << 16 | v << 32 | v << 48
    };
    let imm = match cmode >> 1 {
        0..=3 => rep32(imm8 << (8 * (cmode >> 1))),
        4 | 5 => rep16(imm8 << (8 * ((cmode >> 1) & 1))),
        6 => {
            if cmode & 1 == 0 {
                rep32((imm8 << 8) | 0xFF)
            } else {
                rep32((imm8 << 16) | 0xFFFF)
            }
        }
        _ => {
            if cmode & 1 == 0 {
                if !op {
                    imm8 * 0x0101_0101_0101_0101
                } else {
                    (0..8).fold(0u64, |acc, i| {
                        if imm8 >> i & 1 != 0 {
                            acc | 0xFF << (8 * i)
                        } else {
                            acc
                        }
                    })
                }
            } else if !op {
                rep32(fp32_imm(imm8 as u8) as u64)
            } else {
                if !q {
                    return None;
                }
                fp64_imm(imm8 as u8)
            }
        }
    };
    let logical = cmode & 1 == 1 && cmode < 12;
    let (kind, imm) = if logical {
        (if op { ImmKind::Bic } else { ImmKind::Orr }, imm)
    } else if op && cmode < 14 {
        (ImmKind::Mov, !imm)
    } else {
        (ImmKind::Mov, imm)
    };
    Some(SimdOp::Imm { rd, imm, kind, q })
}

fn fp32_imm(i: u8) -> u32 {
    let a = (i >> 7) as u32 & 1;
    let b = (i >> 6) as u32 & 1;
    let cdefgh = (i & 0x3F) as u32;
    (a << 31) | ((b ^ 1) << 30) | (if b == 1 { 0x1F << 25 } else { 0 }) | (cdefgh << 19)
}

fn fp64_imm(i: u8) -> u64 {
    let a = (i >> 7) as u64 & 1;
    let b = (i >> 6) as u64 & 1;
    let cdefgh = (i & 0x3F) as u64;
    (a << 63) | ((b ^ 1) << 62) | (if b == 1 { 0xFF << 54 } else { 0 }) | (cdefgh << 48)
}

fn decode_shift(w: u32, rd: u8, rn: u8, q: bool, u: bool, scalar: bool) -> Option<SimdOp> {
    let immh = (w >> 19) & 15;
    if immh == 0 {
        return None;
    }
    let immhb = ((w >> 16) & 0x7F) as u32;
    let hb = 31 - immh.leading_zeros();
    let es = 1u8 << hb;
    let ebits = 8 * es as u32;
    let right = (2 * ebits - immhb) as u8;
    let left = (immhb - ebits) as u8;
    let opcode = (w >> 11) & 31;
    let (op, esize, shift) = match (u, opcode) {
        (false, 0x00) => (VSh::SShr, es, right),
        (true, 0x00) => (VSh::UShr, es, right),
        (false, 0x02) => (VSh::SSra, es, right),
        (true, 0x02) => (VSh::USra, es, right),
        (true, 0x08) => (VSh::Sri, es, right),
        (false, 0x0a) => (VSh::Shl, es, left),
        (true, 0x0a) => (VSh::Sli, es, left),
        (false, 0x10) if !scalar && es < 8 => (VSh::Shrn, es, right),
        (false, 0x14) if !scalar && es < 8 => (VSh::SShll, es * 2, left),
        (true, 0x14) if !scalar && es < 8 => (VSh::UShll, es * 2, left),
        (false, 0x1c) if es >= 4 => (VSh::SCvtF, es, right),
        (true, 0x1c) if es >= 4 => (VSh::UCvtF, es, right),
        (false, 0x1f) if es >= 4 => (VSh::FCvtZs, es, right),
        (true, 0x1f) if es >= 4 => (VSh::FCvtZu, es, right),
        _ => return None,
    };
    let integer_shift = !matches!(op, VSh::SCvtF | VSh::UCvtF | VSh::FCvtZs | VSh::FCvtZu);
    if scalar && integer_shift && es != 8 {
        return None;
    }
    if !scalar && es == 8 && !q && !matches!(op, VSh::Shrn | VSh::SShll | VSh::UShll) {
        return None;
    }
    Some(SimdOp::Shift { rd, rn, op, esize, shift, q, scalar })
}

fn decode_elem(w: u32, rd: u8, rn: u8, q: bool, u: bool, scalar: bool) -> Option<SimdOp> {
    let size = (w >> 22) & 3;
    let l = (w >> 21) & 1;
    let m = (w >> 20) & 1;
    let h = (w >> 11) & 1;
    let rm4 = (w >> 16) & 15;
    let opcode = (w >> 12) & 15;
    let fp_op = match (u, opcode) {
        (false, 1) => Some(VElem::FMla),
        (false, 5) => Some(VElem::FMls),
        (false, 9) => Some(VElem::FMul),
        (true, 9) => Some(VElem::FMulX),
        _ => None,
    };
    if let Some(op) = fp_op {
        let (esize, index) = match size {
            2 => (4u8, (h << 1 | l) as u8),
            3 if l == 0 => (8u8, h as u8),
            _ => return None,
        };
        if esize == 8 && !q && !scalar {
            return None;
        }
        let rm = ((m << 4) | rm4) as u8;
        return Some(SimdOp::Elem { rd, rn, rm, index, op, esize, q, scalar });
    }
    if scalar {
        return None;
    }
    let op = match (u, opcode) {
        (false, 8) => VElem::Mul,
        (true, 0) => VElem::Mla,
        (true, 4) => VElem::Mls,
        (false, 0xa) => VElem::SMull,
        (true, 0xa) => VElem::UMull,
        (false, 2) => VElem::SMlal,
        (true, 2) => VElem::UMlal,
        _ => return None,
    };
    let (esize, index, rm) = match size {
        1 => (2u8, (h << 2 | l << 1 | m) as u8, rm4 as u8),
        2 => (4u8, (h << 1 | l) as u8, ((m << 4) | rm4) as u8),
        _ => return None,
    };
    Some(SimdOp::Elem { rd, rn, rm, index, op, esize, q, scalar: false })
}

fn decode_multi(w: u32) -> Option<SimdMem> {
    let post_form = (w >> 23) & 1 != 0;
    let rm = ((w >> 16) & 31) as u8;
    if !post_form && (w >> 16) & 0x3F != 0 {
        return None;
    }
    let q = (w >> 30) & 1 != 0;
    let size = ((w >> 10) & 3) as u8;
    let (regs, selem) = match (w >> 12) & 15 {
        0b0000 => (4, 4),
        0b0010 => (4, 1),
        0b0100 => (3, 3),
        0b0110 => (3, 1),
        0b0111 => (1, 1),
        0b1000 => (2, 2),
        0b1010 => (2, 1),
        _ => return None,
    };
    if size == 3 && !q && selem != 1 {
        return None;
    }
    let mut m = SimdMem {
        rt: (w & 31) as u8,
        rn: ((w >> 5) & 31) as u8,
        regs,
        selem,
        esize: 1 << size,
        q,
        load: (w >> 22) & 1 != 0,
        shape: Shape::Multi,
        post: Post::None,
    };
    if post_form {
        m.post = if rm == 31 { Post::Imm(m.transfer_bytes() as u8) } else { Post::Reg(rm) };
    }
    Some(m)
}

fn decode_single(w: u32) -> Option<SimdMem> {
    let post_form = (w >> 23) & 1 != 0;
    let rm = ((w >> 16) & 31) as u8;
    if !post_form && rm != 0 {
        return None;
    }
    let q = (w >> 30) & 1 != 0;
    let load = (w >> 22) & 1 != 0;
    let r = (w >> 21) & 1;
    let opcode = (w >> 13) & 7;
    let s = (w >> 12) & 1;
    let size = (w >> 10) & 3;
    let selem = ((((opcode & 1) << 1) | r) + 1) as u8;
    let qi = q as u32;
    let (esize, shape) = match opcode >> 1 {
        0 => (1u8, Shape::Lane(((qi << 3) | (s << 2) | size) as u8)),
        1 if size & 1 == 0 => (2u8, Shape::Lane(((qi << 2) | (s << 1) | (size >> 1)) as u8)),
        2 if size == 0 => (4u8, Shape::Lane(((qi << 1) | s) as u8)),
        2 if size == 1 && s == 0 => (8u8, Shape::Lane(qi as u8)),
        3 if load && s == 0 => (1u8 << size, Shape::Replicate),
        _ => return None,
    };
    let mut m = SimdMem {
        rt: (w & 31) as u8,
        rn: ((w >> 5) & 31) as u8,
        regs: selem,
        selem,
        esize,
        q,
        load,
        shape,
        post: Post::None,
    };
    if post_form {
        m.post = if rm == 31 { Post::Imm(m.transfer_bytes() as u8) } else { Post::Reg(rm) };
    }
    Some(m)
}

// ---------------------------------------------------------------------------
// Disassembly, for comments in the emitted C
// ---------------------------------------------------------------------------

pub fn describe(op: &SimdOp) -> String {
    format!("{op:?}")
}

pub fn describe_mem(m: &SimdMem) -> String {
    format!(
        "{}{} {{v{}..+{}}} [x{}] {:?}",
        if m.load { "ld" } else { "st" },
        m.selem,
        m.rt,
        m.regs,
        m.rn,
        m.shape
    )
}

// ---------------------------------------------------------------------------
// Reference semantics
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Vr([u8; 16]);

impl Vr {
    fn get(&self, i: usize, es: usize) -> u64 {
        let mut x = 0u64;
        for k in 0..es {
            x |= (self.0[i * es + k] as u64) << (8 * k);
        }
        x
    }
    fn set(&mut self, i: usize, es: usize, v: u64) {
        for k in 0..es {
            self.0[i * es + k] = (v >> (8 * k)) as u8;
        }
    }
}

fn vget(st: &State, r: u8) -> Vr {
    let v = st.v[(r & 31) as usize];
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&v[0].to_le_bytes());
    b[8..].copy_from_slice(&v[1].to_le_bytes());
    Vr(b)
}

/// Write `bytes` bytes of `v` and zero the rest of the register.
fn vput(st: &mut State, r: u8, mut v: Vr, bytes: usize) {
    for b in v.0.iter_mut().skip(bytes) {
        *b = 0;
    }
    let lo = u64::from_le_bytes(v.0[..8].try_into().unwrap());
    let hi = u64::from_le_bytes(v.0[8..].try_into().unwrap());
    st.v[(r & 31) as usize] = [lo, hi];
}

fn mask(es: usize) -> u64 {
    if es >= 8 {
        u64::MAX
    } else {
        (1u64 << (8 * es)) - 1
    }
}

fn sx(v: u64, es: usize) -> i64 {
    let s = 64 - 8 * es as u32;
    ((v << s) as i64) >> s
}

fn ones(es: usize) -> u64 {
    mask(es)
}

fn gp(st: &State, r: u8) -> u64 {
    if r == 31 {
        0
    } else {
        st.x[r as usize]
    }
}

/// IEEE operations at one width with the Arm NaN rules.
macro_rules! fp_width {
    ($name:ident, $f:ty, $u:ty, $qbit:expr, $dnan:expr, $expmask:expr) => {
        mod $name {
            pub type F = $f;
            pub const QBIT: $u = $qbit;
            pub const DNAN: $u = $dnan;
            pub fn f(b: u64) -> F {
                F::from_bits(b as $u)
            }
            pub fn b(x: F) -> u64 {
                x.to_bits() as u64
            }
            pub fn is_snan(x: F) -> bool {
                x.is_nan() && x.to_bits() & QBIT == 0
            }
            pub fn quiet(x: F) -> F {
                F::from_bits(x.to_bits() | QBIT)
            }
            pub fn dnan() -> F {
                F::from_bits(DNAN)
            }
            /// FPProcessNaNs: signalling NaNs first, then quiet, in operand order.
            pub fn nan2(a: F, b: F) -> Option<F> {
                if is_snan(a) {
                    Some(quiet(a))
                } else if is_snan(b) {
                    Some(quiet(b))
                } else if a.is_nan() {
                    Some(a)
                } else if b.is_nan() {
                    Some(b)
                } else {
                    None
                }
            }
            pub fn nan3(a: F, b: F, c: F) -> Option<F> {
                if is_snan(a) {
                    Some(quiet(a))
                } else if is_snan(b) {
                    Some(quiet(b))
                } else if is_snan(c) {
                    Some(quiet(c))
                } else if a.is_nan() {
                    Some(a)
                } else if b.is_nan() {
                    Some(b)
                } else if c.is_nan() {
                    Some(c)
                } else {
                    None
                }
            }
            pub fn fix(r: F) -> F {
                if r.is_nan() {
                    dnan()
                } else {
                    r
                }
            }
            pub fn add(a: F, b: F) -> F {
                nan2(a, b).unwrap_or_else(|| fix(a + b))
            }
            pub fn sub(a: F, b: F) -> F {
                nan2(a, b).unwrap_or_else(|| fix(a - b))
            }
            pub fn mul(a: F, b: F) -> F {
                nan2(a, b).unwrap_or_else(|| fix(a * b))
            }
            pub fn div(a: F, b: F) -> F {
                nan2(a, b).unwrap_or_else(|| fix(a / b))
            }
            fn inf_zero(a: F, b: F) -> bool {
                (a.is_infinite() && b == 0.0) || (a == 0.0 && b.is_infinite())
            }
            /// `addend + a * b`, fused.
            pub fn muladd(addend: F, a: F, b: F) -> F {
                if addend.is_nan() && !is_snan(addend) && inf_zero(a, b) {
                    return dnan();
                }
                if let Some(n) = nan3(addend, a, b) {
                    return n;
                }
                fix(a.mul_add(b, addend))
            }
            pub fn mulx(a: F, b: F) -> F {
                if let Some(n) = nan2(a, b) {
                    return n;
                }
                if inf_zero(a, b) {
                    let neg = a.is_sign_negative() != b.is_sign_negative();
                    return if neg { -2.0 } else { 2.0 };
                }
                fix(a * b)
            }
            pub fn recps(a: F, b: F) -> F {
                let a = -a;
                if let Some(n) = nan2(a, b) {
                    return n;
                }
                if inf_zero(a, b) {
                    return 2.0;
                }
                if a.is_infinite() || b.is_infinite() {
                    let neg = a.is_sign_negative() != b.is_sign_negative();
                    return if neg { F::NEG_INFINITY } else { F::INFINITY };
                }
                fix(a.mul_add(b, 2.0))
            }
            pub fn rsqrts(a: F, b: F) -> F {
                let a = -a;
                if let Some(n) = nan2(a, b) {
                    return n;
                }
                if inf_zero(a, b) {
                    return 1.5;
                }
                if a.is_infinite() || b.is_infinite() {
                    let neg = a.is_sign_negative() != b.is_sign_negative();
                    return if neg { F::NEG_INFINITY } else { F::INFINITY };
                }
                fix(a.mul_add(b, 3.0) * 0.5)
            }
            pub fn max(a: F, b: F) -> F {
                if let Some(n) = nan2(a, b) {
                    return n;
                }
                if a == b {
                    return if a.is_sign_negative() && b.is_sign_negative() { a } else if a == 0.0 { 0.0 } else { a };
                }
                if a > b {
                    a
                } else {
                    b
                }
            }
            pub fn min(a: F, b: F) -> F {
                if let Some(n) = nan2(a, b) {
                    return n;
                }
                if a == b {
                    return if a == 0.0 && (a.is_sign_negative() || b.is_sign_negative()) { -0.0 } else { a };
                }
                if a < b {
                    a
                } else {
                    b
                }
            }
            fn qnan_only(x: F) -> bool {
                x.is_nan() && !is_snan(x)
            }
            pub fn maxnm(a: F, b: F) -> F {
                let (mut a, mut b) = (a, b);
                if qnan_only(a) && !qnan_only(b) {
                    a = F::NEG_INFINITY;
                } else if qnan_only(b) && !qnan_only(a) {
                    b = F::NEG_INFINITY;
                }
                max(a, b)
            }
            pub fn minnm(a: F, b: F) -> F {
                let (mut a, mut b) = (a, b);
                if qnan_only(a) && !qnan_only(b) {
                    a = F::INFINITY;
                } else if qnan_only(b) && !qnan_only(a) {
                    b = F::INFINITY;
                }
                min(a, b)
            }
            pub fn abs(x: F) -> F {
                F::from_bits(x.to_bits() & !(1 as $u).rotate_right(1))
            }
            pub fn neg(x: F) -> F {
                F::from_bits(x.to_bits() ^ (1 as $u).rotate_right(1))
            }
            pub fn sqrt(x: F) -> F {
                if x.is_nan() {
                    return if is_snan(x) { quiet(x) } else { x };
                }
                if x < 0.0 {
                    return dnan();
                }
                x.sqrt()
            }
            /// mode: 0 nearest even, 1 +inf, 2 -inf, 3 zero, 4 nearest away.
            pub fn round(x: F, mode: u8) -> F {
                if x.is_nan() {
                    return quiet(x);
                }
                match mode {
                    0 => x.round_ties_even(),
                    1 => x.ceil(),
                    2 => x.floor(),
                    3 => x.trunc(),
                    _ => x.round(),
                }
            }
            #[allow(dead_code)]
            pub const EXPMASK: $u = $expmask;
        }
    };
}

fp_width!(f32w, f32, u32, 0x0040_0000, 0x7FC0_0000, 0x7F80_0000);
fp_width!(f64w, f64, u64, 0x0008_0000_0000_0000, 0x7FF8_0000_0000_0000, 0x7FF0_0000_0000_0000);

/// Dispatch a width-generic FP expression over 4- or 8-byte lanes.
/// Arm `RecipEstimate`: `a` is 256..511, the result 256..511.
fn recip_estimate_table(a: u64) -> u64 {
    let a = a * 2 + 1;
    let b = (1u64 << 19) / a;
    (b + 1) / 2
}

/// Arm `RecipSqrtEstimate`: `a` is 128..511, the result 256..511.
fn rsqrt_estimate_table(a: u64) -> u64 {
    let a = if a < 256 { a * 2 + 1 } else { ((a >> 1) << 1) * 2 + 2 };
    let mut b = 512u64;
    while a * (b + 1) * (b + 1) < (1 << 28) {
        b += 1;
    }
    (b + 1) / 2
}

/// The operand as Arm's pseudocode sees it: sign, biased exponent, and the
/// fraction left-aligned in 52 bits.
fn fp_fields(es: usize, a: u64) -> (u64, i64, u64) {
    if es == 4 {
        let a = a & 0xFFFF_FFFF;
        (a >> 31, ((a >> 23) & 0xFF) as i64, (a & 0x7F_FFFF) << 29)
    } else {
        (a >> 63, ((a >> 52) & 0x7FF) as i64, a & 0xF_FFFF_FFFF_FFFF)
    }
}

/// Special operands shared by both estimates. `Some` is the finished result.
fn estimate_special(es: usize, a: u64, rsqrt: bool) -> Option<u64> {
    let (sign, exp, frac) = fp_fields(es, a);
    let max_exp = if es == 4 { 0xFF } else { 0x7FF };
    let (inf, qbit, dnan) = if es == 4 { (0x7F80_0000u64, 0x40_0000u64, 0x7FC0_0000u64) } else { (0x7FF0_0000_0000_0000, 1 << 51, 0x7FF8_0000_0000_0000) };
    let sign_bit = sign << (es * 8 - 1);
    let bits = if es == 4 { a & 0xFFFF_FFFF } else { a };
    if exp == max_exp && frac != 0 {
        return Some(bits | qbit);
    }
    if exp == 0 && frac == 0 {
        return Some(sign_bit | inf);
    }
    if exp == max_exp {
        return Some(if rsqrt { if sign == 1 { dnan } else { 0 } } else { sign_bit });
    }
    if rsqrt && sign == 1 {
        return Some(dnan);
    }
    // A reciprocal too large to represent rounds to infinity.
    if !rsqrt && exp == 0 && (frac >> 50) == 0 {
        return Some(sign_bit | inf);
    }
    None
}

/// Arm `FPRecipEstimate` with round to nearest and no flush to zero.
pub(crate) fn recip_estimate(es: usize, a: u64) -> u64 {
    if let Some(r) = estimate_special(es, a, false) {
        return r;
    }
    let (sign, mut exp, mut frac) = fp_fields(es, a);
    if exp == 0 {
        if (frac >> 51) & 1 == 0 {
            exp = -1;
            frac = (frac << 2) & 0xF_FFFF_FFFF_FFFF;
        } else {
            frac = (frac << 1) & 0xF_FFFF_FFFF_FFFF;
        }
    }
    let scaled = (1 << 8) | ((frac >> 44) & 0xFF);
    let mut result_exp = if es == 4 { 253 - exp } else { 2045 - exp };
    let estimate = recip_estimate_table(scaled);
    let mut fraction = (estimate & 0xFF) << 44;
    if result_exp == 0 {
        fraction = (1 << 51) | (fraction >> 1);
    } else if result_exp == -1 {
        fraction = (1 << 50) | (fraction >> 2);
        result_exp = 0;
    }
    if es == 4 {
        (sign << 31) | (((result_exp as u64) & 0xFF) << 23) | (fraction >> 29)
    } else {
        (sign << 63) | (((result_exp as u64) & 0x7FF) << 52) | fraction
    }
}

/// Arm `FPRSqrtEstimate` with round to nearest and no flush to zero.
pub(crate) fn rsqrt_estimate(es: usize, a: u64) -> u64 {
    if let Some(r) = estimate_special(es, a, true) {
        return r;
    }
    let (_, mut exp, mut frac) = fp_fields(es, a);
    if exp == 0 {
        while (frac >> 51) & 1 == 0 {
            frac = (frac << 1) & 0xF_FFFF_FFFF_FFFF;
            exp -= 1;
        }
        frac = (frac << 1) & 0xF_FFFF_FFFF_FFFF;
    }
    let scaled = if exp & 1 == 0 { (1 << 8) | ((frac >> 44) & 0xFF) } else { (1 << 7) | ((frac >> 45) & 0x7F) };
    let estimate = rsqrt_estimate_table(scaled) & 0xFF;
    if es == 4 {
        let result_exp = ((380 - exp).div_euclid(2)) as u64;
        ((result_exp & 0xFF) << 23) | (estimate << 15)
    } else {
        let result_exp = ((3068 - exp).div_euclid(2)) as u64;
        ((result_exp & 0x7FF) << 52) | (estimate << 44)
    }
}

macro_rules! fpw {
    ($es:expr, |$m:ident| $body:expr) => {
        if $es == 4 {
            use f32w as $m;
            $body
        } else {
            use f64w as $m;
            $body
        }
    };
}

fn fp3(op: V3, es: usize, a: u64, b: u64, d: u64) -> u64 {
    fpw!(es, |m| {
        let (x, y, z) = (m::f(a), m::f(b), m::f(d));
        let t = ones(es);
        match op {
            V3::FAdd | V3::FAddP => m::b(m::add(x, y)),
            V3::FSub => m::b(m::sub(x, y)),
            V3::FMul => m::b(m::mul(x, y)),
            V3::FDiv => m::b(m::div(x, y)),
            V3::FMax | V3::FMaxP => m::b(m::max(x, y)),
            V3::FMin | V3::FMinP => m::b(m::min(x, y)),
            V3::FMaxNm | V3::FMaxNmP => m::b(m::maxnm(x, y)),
            V3::FMinNm | V3::FMinNmP => m::b(m::minnm(x, y)),
            V3::FMla => m::b(m::muladd(z, x, y)),
            V3::FMls => m::b(m::muladd(z, m::neg(x), y)),
            V3::FAbd => m::b(m::abs(m::sub(x, y))),
            V3::FMulX => m::b(m::mulx(x, y)),
            V3::FCmEq => if x == y { t } else { 0 },
            V3::FCmGe => if x >= y { t } else { 0 },
            V3::FCmGt => if x > y { t } else { 0 },
            V3::FAcGe => if m::abs(x) >= m::abs(y) { t } else { 0 },
            V3::FAcGt => if m::abs(x) > m::abs(y) { t } else { 0 },
            V3::FRecps => m::b(m::recps(x, y)),
            V3::FRsqrts => m::b(m::rsqrts(x, y)),
            _ => unreachable!("integer op in fp3"),
        }
    })
}

fn int3(op: V3, es: usize, a: u64, b: u64, d: u64) -> u64 {
    let mk = mask(es);
    let bits = 8 * es as u32;
    let (sa, sb) = (sx(a, es), sx(b, es));
    let t = ones(es);
    let r = match op {
        V3::Add | V3::AddP => a.wrapping_add(b),
        V3::Sub => a.wrapping_sub(b),
        V3::Mul => a.wrapping_mul(b),
        V3::Mla => d.wrapping_add(a.wrapping_mul(b)),
        V3::Mls => d.wrapping_sub(a.wrapping_mul(b)),
        V3::And => a & b,
        V3::Bic => a & !b,
        V3::Orr => a | b,
        V3::Orn => a | !b,
        V3::Eor => a ^ b,
        V3::Bsl => (d & a) | (!d & b),
        V3::Bit => (d & !b) | (a & b),
        V3::Bif => (d & b) | (a & !b),
        V3::CmEq => if a == b { t } else { 0 },
        V3::CmGt => if sa > sb { t } else { 0 },
        V3::CmGe => if sa >= sb { t } else { 0 },
        V3::CmHi => if a > b { t } else { 0 },
        V3::CmHs => if a >= b { t } else { 0 },
        V3::CmTst => if a & b != 0 { t } else { 0 },
        V3::SMax | V3::SMaxP => if sa >= sb { a } else { b },
        V3::SMin | V3::SMinP => if sa <= sb { a } else { b },
        V3::UMax | V3::UMaxP => a.max(b),
        V3::UMin | V3::UMinP => a.min(b),
        V3::SAbd => (sa as i128 - sb as i128).unsigned_abs() as u64,
        V3::UAbd => a.abs_diff(b),
        V3::SShl | V3::UShl => {
            let sh = (b as u8) as i8 as i32;
            if sh >= 0 {
                if sh as u32 >= bits {
                    0
                } else {
                    a << sh
                }
            } else {
                let r = (-sh) as u32;
                if op == V3::SShl {
                    (sa >> r.min(63)) as u64
                } else if r >= bits {
                    0
                } else {
                    a >> r
                }
            }
        }
        _ => unreachable!("fp op in int3"),
    };
    r & mk
}

fn three(op: V3, es: usize, a: u64, b: u64, d: u64) -> u64 {
    if op.is_fp() {
        fp3(op, es, a, b, d)
    } else {
        int3(op, es, a & mask(es), b & mask(es), d & mask(es))
    }
}

/// Float to integer with Arm saturation. `mode` as in `round`.
fn fp_to_int(es: usize, v: u64, mode: u8, unsigned: bool, fbits: u32) -> u64 {
    // Scaling by a power of two is exact, so doing it in f64 matches the
    // architecture's single rounding.
    let x = if es == 4 { f32::from_bits(v as u32) as f64 } else { f64::from_bits(v) };
    if x.is_nan() {
        return 0;
    }
    let x = x * 2f64.powi(fbits as i32);
    let r = match mode {
        0 => x.round_ties_even(),
        1 => x.ceil(),
        2 => x.floor(),
        3 => x.trunc(),
        _ => x.round(),
    };
    match (es, unsigned) {
        (4, false) => (r as i32) as u32 as u64,
        (4, true) => (r as u32) as u64,
        (_, false) => (r as i64) as u64,
        (_, true) => r as u64,
    }
}

fn int_to_fp(es: usize, v: u64, unsigned: bool, fbits: u32) -> u64 {
    let scale = 2f64.powi(-(fbits as i32));
    if es == 4 {
        let x = if unsigned { v as u32 as f64 } else { v as u32 as i32 as f64 };
        ((x * scale) as f32).to_bits() as u64
    } else if fbits == 0 {
        (if unsigned { v as f64 } else { v as i64 as f64 }).to_bits()
    } else {
        let x = if unsigned { v as f64 } else { v as i64 as f64 };
        (x * scale).to_bits()
    }
}

fn two(op: V2, es: usize, a: u64) -> u64 {
    let t = ones(es);
    let bits = 8 * es as u32;
    match op {
        V2::Cnt => (a as u8).count_ones() as u64,
        V2::Not => !a & mask(es),
        V2::Rbit => (a as u8).reverse_bits() as u64,
        V2::Clz => ((a << (64 - bits)) | if bits < 64 { 1u64 << (63 - bits) } else { 0 }).leading_zeros().min(bits) as u64,
        V2::Cls => {
            let s = sx(a, es);
            let v = if s < 0 { !s as u64 } else { s as u64 };
            let lz = if v == 0 { 64 } else { v.leading_zeros() };
            (lz - (64 - bits) - 1) as u64
        }
        V2::Abs => (sx(a, es).wrapping_abs() as u64) & mask(es),
        V2::Neg => (sx(a, es).wrapping_neg() as u64) & mask(es),
        V2::CmGt0 => if sx(a, es) > 0 { t } else { 0 },
        V2::CmGe0 => if sx(a, es) >= 0 { t } else { 0 },
        V2::CmEq0 => if a & mask(es) == 0 { t } else { 0 },
        V2::CmLe0 => if sx(a, es) <= 0 { t } else { 0 },
        V2::CmLt0 => if sx(a, es) < 0 { t } else { 0 },
        V2::FAbs => fpw!(es, |m| m::b(m::abs(m::f(a)))),
        V2::FNeg => fpw!(es, |m| m::b(m::neg(m::f(a)))),
        V2::FSqrt => fpw!(es, |m| m::b(m::sqrt(m::f(a)))),
        V2::FRecpE => recip_estimate(es, a),
        V2::FRsqrtE => rsqrt_estimate(es, a),
        V2::FRintN | V2::FRintX | V2::FRintI => fpw!(es, |m| m::b(m::round(m::f(a), 0))),
        V2::FRintP => fpw!(es, |m| m::b(m::round(m::f(a), 1))),
        V2::FRintM => fpw!(es, |m| m::b(m::round(m::f(a), 2))),
        V2::FRintZ => fpw!(es, |m| m::b(m::round(m::f(a), 3))),
        V2::FRintA => fpw!(es, |m| m::b(m::round(m::f(a), 4))),
        V2::FCvtNs => fp_to_int(es, a, 0, false, 0),
        V2::FCvtPs => fp_to_int(es, a, 1, false, 0),
        V2::FCvtMs => fp_to_int(es, a, 2, false, 0),
        V2::FCvtZs => fp_to_int(es, a, 3, false, 0),
        V2::FCvtAs => fp_to_int(es, a, 4, false, 0),
        V2::FCvtNu => fp_to_int(es, a, 0, true, 0),
        V2::FCvtPu => fp_to_int(es, a, 1, true, 0),
        V2::FCvtMu => fp_to_int(es, a, 2, true, 0),
        V2::FCvtZu => fp_to_int(es, a, 3, true, 0),
        V2::FCvtAu => fp_to_int(es, a, 4, true, 0),
        V2::SCvtF => int_to_fp(es, a, false, 0),
        V2::UCvtF => int_to_fp(es, a, true, 0),
        V2::FCmGt0 => fpw!(es, |m| if m::f(a) > 0.0 { t } else { 0 }),
        V2::FCmGe0 => fpw!(es, |m| if m::f(a) >= 0.0 { t } else { 0 }),
        V2::FCmEq0 => fpw!(es, |m| if m::f(a) == 0.0 { t } else { 0 }),
        V2::FCmLe0 => fpw!(es, |m| if m::f(a) <= 0.0 { t } else { 0 }),
        V2::FCmLt0 => fpw!(es, |m| if m::f(a) < 0.0 { t } else { 0 }),
        _ => unreachable!("structural op in two"),
    }
}

/// Double to single with the Arm NaN conversion (quieted, payload truncated).
fn f64_to_f32_bits(a: u64) -> u64 {
    let x = f64::from_bits(a);
    if x.is_nan() {
        let sign = (a >> 63) as u32;
        return ((sign << 31) | 0x7FC0_0000 | ((a >> 29) as u32 & 0x003F_FFFF)) as u64;
    }
    (x as f32).to_bits() as u64
}

fn f32_to_f64_bits(a: u64) -> u64 {
    let x = f32::from_bits(a as u32);
    if x.is_nan() {
        let sign = (a >> 31) & 1;
        return (sign << 63) | 0x7FF8_0000_0000_0000 | ((a & 0x003F_FFFF) << 29);
    }
    (x as f64).to_bits()
}

/// `addend + a*b` or `addend - a*b` in floating point for the by-element forms.
fn elem_fp(op: VElem, es: usize, a: u64, b: u64, d: u64) -> u64 {
    match op {
        VElem::FMul => fp3(V3::FMul, es, a, b, d),
        VElem::FMla => fp3(V3::FMla, es, a, b, d),
        VElem::FMls => fp3(V3::FMls, es, a, b, d),
        VElem::FMulX => fp3(V3::FMulX, es, a, b, d),
        _ => unreachable!(),
    }
}

/// Recursive halving reduction, the order the architecture specifies.
fn reduce(vals: &[u64], f: &dyn Fn(u64, u64) -> u64) -> u64 {
    if vals.len() == 1 {
        return vals[0];
    }
    let half = vals.len() / 2;
    let lo = reduce(&vals[..half], f);
    let hi = reduce(&vals[half..], f);
    f(lo, hi)
}

/// Execute a register-only SIMD instruction.
pub fn exec(st: &mut State, op: &SimdOp) -> Result<(), String> {
    match *op {
        SimdOp::DupElem { rd, rn, esize, index, q } => {
            let es = esize as usize;
            let e = vget(st, rn).get(index as usize, es);
            let width = if q { 16 } else { 8 };
            let mut r = Vr([0; 16]);
            for i in 0..width / es {
                r.set(i, es, e);
            }
            vput(st, rd, r, width);
        }
        SimdOp::DupGen { rd, rn, esize, q } => {
            let es = esize as usize;
            let e = gp(st, rn) & mask(es);
            let width = if q { 16 } else { 8 };
            let mut r = Vr([0; 16]);
            for i in 0..width / es {
                r.set(i, es, e);
            }
            vput(st, rd, r, width);
        }
        SimdOp::InsGen { rd, rn, esize, index } => {
            let mut r = vget(st, rd);
            r.set(index as usize, esize as usize, gp(st, rn) & mask(esize as usize));
            vput(st, rd, r, 16);
        }
        SimdOp::InsElem { rd, rn, esize, dst, src } => {
            let e = vget(st, rn).get(src as usize, esize as usize);
            let mut r = vget(st, rd);
            r.set(dst as usize, esize as usize, e);
            vput(st, rd, r, 16);
        }
        SimdOp::ToGp { rd, rn, esize, index, signed, is64 } => {
            let es = esize as usize;
            let e = vget(st, rn).get(index as usize, es);
            let v = if signed { sx(e, es) as u64 } else { e };
            let v = if is64 { v } else { v & 0xFFFF_FFFF };
            if rd != 31 {
                st.x[rd as usize] = v;
            }
        }
        SimdOp::DupScalar { rd, rn, esize, index } => {
            let es = esize as usize;
            let e = vget(st, rn).get(index as usize, es);
            let mut r = Vr([0; 16]);
            r.set(0, es, e);
            vput(st, rd, r, es);
        }
        SimdOp::Ext { rd, rn, rm, pos, q } => {
            let width = if q { 16 } else { 8 };
            let (n, m) = (vget(st, rn), vget(st, rm));
            let mut cat = [0u8; 32];
            cat[..width].copy_from_slice(&n.0[..width]);
            cat[width..2 * width].copy_from_slice(&m.0[..width]);
            let mut r = Vr([0; 16]);
            r.0[..width].copy_from_slice(&cat[pos as usize..pos as usize + width]);
            vput(st, rd, r, width);
        }
        SimdOp::Imm { rd, imm, kind, q } => {
            let width = if q { 16 } else { 8 };
            let mut r = vget(st, rd);
            for i in 0..width / 8 {
                let d = r.get(i, 8);
                let v = match kind {
                    ImmKind::Mov => imm,
                    ImmKind::Orr => d | imm,
                    ImmKind::Bic => d & !imm,
                };
                r.set(i, 8, v);
            }
            vput(st, rd, r, width);
        }
        SimdOp::Three { rd, rn, rm, op, esize, q, scalar } => {
            let es = esize as usize;
            let width = if scalar { es } else if q { 16 } else { 8 };
            let lanes = width / es;
            let (n, m, d) = (vget(st, rn), vget(st, rm), vget(st, rd));
            let mut r = Vr([0; 16]);
            if op.is_pairwise() {
                let half = lanes / 2;
                for i in 0..half {
                    r.set(i, es, three(op, es, n.get(2 * i, es), n.get(2 * i + 1, es), 0));
                    r.set(half + i, es, three(op, es, m.get(2 * i, es), m.get(2 * i + 1, es), 0));
                }
            } else {
                for i in 0..lanes {
                    r.set(i, es, three(op, es, n.get(i, es), m.get(i, es), d.get(i, es)));
                }
            }
            vput(st, rd, r, width);
        }
        SimdOp::Pair { rd, rn, op, esize } => {
            let es = esize as usize;
            let n = vget(st, rn);
            let mut r = Vr([0; 16]);
            r.set(0, es, three(op, es, n.get(0, es), n.get(1, es), 0));
            vput(st, rd, r, es);
        }
        SimdOp::Two { rd, rn, op, esize, q, scalar } => {
            let es = esize as usize;
            let n = vget(st, rn);
            match op {
                V2::Rev16 | V2::Rev32 | V2::Rev64 => {
                    let container = match op {
                        V2::Rev16 => 2,
                        V2::Rev32 => 4,
                        _ => 8,
                    };
                    let width = if q { 16 } else { 8 };
                    let per = container / es;
                    let mut r = Vr([0; 16]);
                    for i in 0..width / es {
                        let src = (i / per) * per + (per - 1 - i % per);
                        r.set(i, es, n.get(src, es));
                    }
                    vput(st, rd, r, width);
                }
                V2::Xtn | V2::FCvtN => {
                    // `es` is the narrow element, the source is twice as wide.
                    let lanes = 8 / es;
                    let mut r = if q { vget(st, rd) } else { Vr([0; 16]) };
                    let base = if q { lanes } else { 0 };
                    for i in 0..lanes {
                        let wide = n.get(i, es * 2);
                        let v = if op == V2::FCvtN { f64_to_f32_bits(wide) } else { wide & mask(es) };
                        r.set(base + i, es, v);
                    }
                    vput(st, rd, r, if q { 16 } else { 8 });
                }
                V2::FCvtL | V2::SAddLp | V2::UAddLp => {
                    let half = es / 2;
                    let mut r = Vr([0; 16]);
                    if op == V2::FCvtL {
                        let base = if q { 2 } else { 0 };
                        for i in 0..2 {
                            r.set(i, 8, f32_to_f64_bits(n.get(base + i, 4)));
                        }
                        vput(st, rd, r, 16);
                    } else {
                        let width = if q { 16 } else { 8 };
                        for i in 0..width / es {
                            let (x, y) = (n.get(2 * i, half), n.get(2 * i + 1, half));
                            let v = if op == V2::SAddLp {
                                (sx(x, half) + sx(y, half)) as u64
                            } else {
                                x + y
                            };
                            r.set(i, es, v & mask(es));
                        }
                        vput(st, rd, r, width);
                    }
                }
                _ => {
                    let width = if scalar { es } else if q { 16 } else { 8 };
                    let mut r = Vr([0; 16]);
                    for i in 0..width / es {
                        r.set(i, es, two(op, es, n.get(i, es)));
                    }
                    vput(st, rd, r, width);
                }
            }
        }
        SimdOp::Shift { rd, rn, op, esize, shift, q, scalar } => {
            let es = esize as usize;
            let sh = shift as u32;
            let n = vget(st, rn);
            let d = vget(st, rd);
            match op {
                VSh::Shrn => {
                    let lanes = 8 / es;
                    let mut r = if q { d } else { Vr([0; 16]) };
                    let base = if q { lanes } else { 0 };
                    for i in 0..lanes {
                        let wide = n.get(i, es * 2);
                        r.set(base + i, es, (wide >> sh) & mask(es));
                    }
                    vput(st, rd, r, if q { 16 } else { 8 });
                }
                VSh::SShll | VSh::UShll => {
                    let half = es / 2;
                    let lanes = 8 / half;
                    let base = if q { lanes } else { 0 };
                    let mut r = Vr([0; 16]);
                    for i in 0..lanes {
                        let x = n.get(base + i, half);
                        let v = if op == VSh::SShll { sx(x, half) as u64 } else { x };
                        r.set(i, es, (v << sh) & mask(es));
                    }
                    vput(st, rd, r, 16);
                }
                _ => {
                    let width = if scalar { es } else if q { 16 } else { 8 };
                    let bits = 8 * es as u32;
                    let mut r = Vr([0; 16]);
                    for i in 0..width / es {
                        let a = n.get(i, es);
                        let old = d.get(i, es);
                        let sshr = (sx(a, es) >> sh.min(63)) as u64 & mask(es);
                        let ushr = if sh >= bits { 0 } else { a >> sh };
                        let v = match op {
                            VSh::SShr => sshr,
                            VSh::UShr => ushr,
                            VSh::SSra => old.wrapping_add(sshr) & mask(es),
                            VSh::USra => old.wrapping_add(ushr) & mask(es),
                            VSh::Shl => (a << sh) & mask(es),
                            VSh::Sri => {
                                if sh >= bits {
                                    old
                                } else {
                                    let m = mask(es) >> sh;
                                    (old & !m) | ushr
                                }
                            }
                            VSh::Sli => {
                                let m = (mask(es) << sh) & mask(es);
                                (old & !m) | ((a << sh) & m)
                            }
                            VSh::SCvtF => int_to_fp(es, a, false, sh),
                            VSh::UCvtF => int_to_fp(es, a, true, sh),
                            VSh::FCvtZs => fp_to_int(es, a, 3, false, sh),
                            VSh::FCvtZu => fp_to_int(es, a, 3, true, sh),
                            _ => unreachable!(),
                        };
                        r.set(i, es, v);
                    }
                    vput(st, rd, r, width);
                }
            }
        }
        SimdOp::Long { rd, rn, rm, op, signed, esize, q } => {
            let es = esize as usize;
            let half = es / 2;
            let lanes = 16 / es;
            let base = if q { lanes } else { 0 };
            let (n, m, d) = (vget(st, rn), vget(st, rm), vget(st, rd));
            let ext = |v: u64, w: usize| if signed { sx(v, w) as u64 } else { v };
            let mut r = Vr([0; 16]);
            for i in 0..lanes {
                let a = if matches!(op, VLong::AddW | VLong::SubW) {
                    n.get(i, es)
                } else {
                    ext(n.get(base + i, half), half)
                };
                let b = ext(m.get(base + i, half), half);
                let old = d.get(i, es);
                let v = match op {
                    VLong::AddL | VLong::AddW => a.wrapping_add(b),
                    VLong::SubL | VLong::SubW => a.wrapping_sub(b),
                    VLong::AbdL => {
                        if signed {
                            (a as i64 - b as i64).unsigned_abs()
                        } else {
                            a.abs_diff(b)
                        }
                    }
                    VLong::MlaL => old.wrapping_add(a.wrapping_mul(b)),
                    VLong::MlsL => old.wrapping_sub(a.wrapping_mul(b)),
                    VLong::MulL => a.wrapping_mul(b),
                };
                r.set(i, es, v & mask(es));
            }
            vput(st, rd, r, 16);
        }
        SimdOp::Perm { rd, rn, rm, op, esize, q } => {
            let es = esize as usize;
            let width = if q { 16 } else { 8 };
            let lanes = width / es;
            let half = lanes / 2;
            let (n, m) = (vget(st, rn), vget(st, rm));
            let cat = |i: usize| if i < lanes { n.get(i, es) } else { m.get(i - lanes, es) };
            let mut r = Vr([0; 16]);
            for i in 0..lanes {
                let v = match op {
                    VPerm::Uzp1 => cat(2 * i),
                    VPerm::Uzp2 => cat(2 * i + 1),
                    VPerm::Trn1 => if i % 2 == 0 { n.get(i, es) } else { m.get(i - 1, es) },
                    VPerm::Trn2 => if i % 2 == 0 { n.get(i + 1, es) } else { m.get(i, es) },
                    VPerm::Zip1 => if i % 2 == 0 { n.get(i / 2, es) } else { m.get(i / 2, es) },
                    VPerm::Zip2 => if i % 2 == 0 { n.get(half + i / 2, es) } else { m.get(half + i / 2, es) },
                };
                r.set(i, es, v);
            }
            vput(st, rd, r, width);
        }
        SimdOp::Elem { rd, rn, rm, index, op, esize, q, scalar } => {
            let es = esize as usize;
            let (n, mreg, d) = (vget(st, rn), vget(st, rm), vget(st, rd));
            let e = mreg.get(index as usize, es);
            let mut r = Vr([0; 16]);
            match op {
                VElem::SMull | VElem::UMull | VElem::SMlal | VElem::UMlal => {
                    let wide = es * 2;
                    let lanes = 8 / es;
                    let base = if q { lanes } else { 0 };
                    let signed = matches!(op, VElem::SMull | VElem::SMlal);
                    let ext = |v: u64| if signed { sx(v, es) as u64 } else { v };
                    for i in 0..lanes {
                        let p = ext(n.get(base + i, es)).wrapping_mul(ext(e));
                        let v = match op {
                            VElem::SMull | VElem::UMull => p,
                            _ => d.get(i, wide).wrapping_add(p),
                        };
                        r.set(i, wide, v & mask(wide));
                    }
                    vput(st, rd, r, 16);
                }
                _ => {
                    let width = if scalar { es } else if q { 16 } else { 8 };
                    for i in 0..width / es {
                        let (a, old) = (n.get(i, es), d.get(i, es));
                        let v = match op {
                            VElem::Mul => a.wrapping_mul(e) & mask(es),
                            VElem::Mla => old.wrapping_add(a.wrapping_mul(e)) & mask(es),
                            VElem::Mls => old.wrapping_sub(a.wrapping_mul(e)) & mask(es),
                            _ => elem_fp(op, es, a, e, old),
                        };
                        r.set(i, es, v);
                    }
                    vput(st, rd, r, width);
                }
            }
        }
        SimdOp::Across { rd, rn, op, esize, q } => {
            let es = esize as usize;
            let width = if q { 16 } else { 8 };
            let n = vget(st, rn);
            let vals: Vec<u64> = (0..width / es).map(|i| n.get(i, es)).collect();
            let (v, out) = match op {
                VAcross::AddV => (vals.iter().fold(0u64, |a, &b| a.wrapping_add(b)) & mask(es), es),
                VAcross::SAddLv => {
                    (vals.iter().fold(0i64, |a, &b| a + sx(b, es)) as u64 & mask(es * 2), es * 2)
                }
                VAcross::UAddLv => (vals.iter().sum::<u64>() & mask(es * 2), es * 2),
                VAcross::SMaxV => (*vals.iter().max_by_key(|&&x| sx(x, es)).unwrap(), es),
                VAcross::SMinV => (*vals.iter().min_by_key(|&&x| sx(x, es)).unwrap(), es),
                VAcross::UMaxV => (*vals.iter().max().unwrap(), es),
                VAcross::UMinV => (*vals.iter().min().unwrap(), es),
                VAcross::FMaxV => (reduce(&vals, &|a, b| fp3(V3::FMax, es, a, b, 0)), es),
                VAcross::FMinV => (reduce(&vals, &|a, b| fp3(V3::FMin, es, a, b, 0)), es),
                VAcross::FMaxNmV => (reduce(&vals, &|a, b| fp3(V3::FMaxNm, es, a, b, 0)), es),
                VAcross::FMinNmV => (reduce(&vals, &|a, b| fp3(V3::FMinNm, es, a, b, 0)), es),
            };
            let mut r = Vr([0; 16]);
            r.set(0, out, v);
            vput(st, rd, r, out);
        }
        SimdOp::Tbl { rd, rn, rm, len, tbx, q } => {
            let width = if q { 16 } else { 8 };
            let mut table = Vec::with_capacity(16 * len as usize);
            for k in 0..len {
                table.extend_from_slice(&vget(st, (rn + k) & 31).0);
            }
            let (m, d) = (vget(st, rm), vget(st, rd));
            let mut r = Vr([0; 16]);
            for i in 0..width {
                let idx = m.0[i] as usize;
                r.0[i] = if idx < table.len() {
                    table[idx]
                } else if tbx {
                    d.0[i]
                } else {
                    0
                };
            }
            vput(st, rd, r, width);
        }
    }
    Ok(())
}

/// Execute a structure load or store.
pub fn exec_mem(st: &mut State, m: &SimdMem) -> Result<(), String> {
    let base = if m.rn == 31 { st.sp } else { st.x[m.rn as usize] };
    let es = m.esize as usize;
    let mut addr = base;
    match m.shape {
        Shape::Multi => {
            let width = if m.q { 16 } else { 8 };
            let elements = width / es;
            let rpt = (m.regs / m.selem) as usize;
            let mut regs: Vec<Vr> = (0..m.regs).map(|k| vget(st, (m.rt + k) & 31)).collect();
            for r in 0..rpt {
                for e in 0..elements {
                    for s in 0..m.selem as usize {
                        let slot = r + s;
                        if m.load {
                            let v = st.load(addr, m.esize)?;
                            regs[slot].set(e, es, v);
                        } else {
                            st.store(addr, regs[slot].get(e, es), m.esize)?;
                        }
                        addr = addr.wrapping_add(es as u64);
                    }
                }
            }
            if m.load {
                for (k, v) in regs.into_iter().enumerate() {
                    vput(st, (m.rt + k as u8) & 31, v, width);
                }
            }
        }
        Shape::Lane(index) => {
            for s in 0..m.selem {
                let reg = (m.rt + s) & 31;
                if m.load {
                    let v = st.load(addr, m.esize)?;
                    let mut r = vget(st, reg);
                    r.set(index as usize, es, v);
                    vput(st, reg, r, 16);
                } else {
                    st.store(addr, vget(st, reg).get(index as usize, es), m.esize)?;
                }
                addr = addr.wrapping_add(es as u64);
            }
        }
        Shape::Replicate => {
            let width = if m.q { 16 } else { 8 };
            for s in 0..m.selem {
                let reg = (m.rt + s) & 31;
                let v = st.load(addr, m.esize)?;
                let mut r = Vr([0; 16]);
                for i in 0..width / es {
                    r.set(i, es, v);
                }
                vput(st, reg, r, width);
                addr = addr.wrapping_add(es as u64);
            }
        }
    }
    let step = match m.post {
        Post::None => None,
        Post::Imm(n) => Some(n as u64),
        Post::Reg(r) => Some(gp(st, r)),
    };
    if let Some(step) = step {
        let v = base.wrapping_add(step);
        if m.rn == 31 {
            st.sp = v;
        } else {
            st.x[m.rn as usize] = v;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aarch64::decode as dec;

    fn simd(w: u32) -> SimdOp {
        match dec(w, 0) {
            Op::Simd(s) => s,
            other => panic!("0x{w:08x} decoded to {other:?}"),
        }
    }

    #[test]
    fn decodes_the_boot_frontier_and_the_common_words() {
        // mov v1.s[2], v0.s[2]: the word that stopped the boot.
        assert_eq!(
            simd(0x6E14_4401),
            SimdOp::InsElem { rd: 1, rn: 0, esize: 4, dst: 2, src: 2 }
        );
        // mov v0.s[3], wzr
        assert_eq!(simd(0x4E1C_1FE0), SimdOp::InsGen { rd: 0, rn: 31, esize: 4, index: 3 });
        // faddp v1.2s, v1.2s, v1.2s
        assert_eq!(
            simd(0x2E21_D421),
            SimdOp::Three { rd: 1, rn: 1, rm: 1, op: V3::FAddP, esize: 4, q: false, scalar: false }
        );
        // fmul v2.2s, v2.2s, v3.2s
        assert!(matches!(simd(0x2E23_DC42), SimdOp::Three { op: V3::FMul, .. }));
        // ext v1.16b, v0.16b, v0.16b, #8
        assert_eq!(simd(0x6E00_4001), SimdOp::Ext { rd: 1, rn: 0, rm: 0, pos: 8, q: true });
        // fmul v1.4s, v1.4s, v0.s[0]
        assert!(matches!(simd(0x4F80_9021), SimdOp::Elem { op: VElem::FMul, index: 0, esize: 4, .. }));
        // mov s0, v0.s[3]
        assert_eq!(simd(0x5E1C_0400), SimdOp::DupScalar { rd: 0, rn: 0, esize: 4, index: 3 });
        // ucvtf s0, s0
        assert!(matches!(simd(0x7E21_D800), SimdOp::Two { op: V2::UCvtF, scalar: true, .. }));
        match dec(0x4D40_8100, 0) {
            Op::SimdMem(m) => {
                assert_eq!(m.shape, Shape::Lane(2));
                assert_eq!(m.esize, 4);
                assert!(m.load);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn element_copies_move_the_right_lane() {
        let mut st = State::default();
        st.v[0] = [0x1111_1111_2222_2222, 0x3333_3333_4444_4444];
        st.v[1] = [0xAAAA_AAAA_BBBB_BBBB, 0xCCCC_CCCC_DDDD_DDDD];
        exec(&mut st, &simd(0x6E14_4401)).unwrap(); // mov v1.s[2], v0.s[2]
        assert_eq!(st.v[1], [0xAAAA_AAAA_BBBB_BBBB, 0xCCCC_CCCC_4444_4444]);
        // dup v2.4s, v0.s[1] (0x4e0c0402)
        exec(&mut st, &simd(0x4E0C_0402)).unwrap();
        assert_eq!(st.v[2], [0x1111_1111_1111_1111, 0x1111_1111_1111_1111]);
    }

    #[test]
    fn vector_float_arithmetic_and_pairwise() {
        let mut st = State::default();
        let pack = |a: f32, b: f32| (a.to_bits() as u64) | ((b.to_bits() as u64) << 32);
        st.v[1] = [pack(1.5, 2.25), 0];
        exec(&mut st, &simd(0x2E21_D421)).unwrap(); // faddp v1.2s, v1.2s, v1.2s
        assert_eq!(st.v[1], [pack(3.75, 3.75), 0]);
    }

    #[test]
    fn invalid_operations_produce_the_arm_default_nan() {
        assert_eq!(f32w::b(f32w::sub(f32::INFINITY, f32::INFINITY)), 0x7FC0_0000);
        assert_eq!(f64w::b(f64w::sqrt(-1.0)), 0x7FF8_0000_0000_0000);
        // A signalling NaN outranks a quiet one in the second operand.
        let snan = f32::from_bits(0x7F80_0001);
        let qnan = f32::from_bits(0x7FC0_0002);
        assert_eq!(f32w::b(f32w::add(qnan, snan)), 0x7FC0_0001);
        // fmaxnm ignores a lone quiet NaN; fmax does not.
        assert_eq!(f32w::maxnm(qnan, 1.0), 1.0);
        assert!(f32w::max(qnan, 1.0).is_nan());
        assert_eq!(f32w::max(0.0, -0.0).to_bits(), 0);
        assert_eq!(f32w::min(0.0, -0.0).to_bits(), 0x8000_0000);
    }

    #[test]
    fn modified_immediates_expand_like_the_assembler() {
        // movi v0.4s, #0x1, lsl #8 is 0x4f002420; orr v0.4s, #0xff is 0x4f0717e0
        assert_eq!(
            simd(0x4F00_2420),
            SimdOp::Imm { rd: 0, imm: 0x0000_0100_0000_0100, kind: ImmKind::Mov, q: true }
        );
        // fmov v0.4s, #1.0 is 0x4f03f600
        assert_eq!(
            simd(0x4F03_F600),
            SimdOp::Imm { rd: 0, imm: 0x3F80_0000_3F80_0000, kind: ImmKind::Mov, q: true }
        );
        // mvni v0.4s, #0 is 0x6f000400 -> all ones
        assert_eq!(
            simd(0x6F00_0400),
            SimdOp::Imm { rd: 0, imm: u64::MAX, kind: ImmKind::Mov, q: true }
        );
    }

    #[test]
    fn structure_loads_interleave() {
        let mut st = State::default();
        for i in 0..32u8 {
            st.mem[0x100 + i as usize] = i;
        }
        st.x[8] = 0x100;
        // ld2 {v0.4s, v1.4s}, [x8], #32 is 0x4cdf8900
        match dec(0x4CDF_8900, 0) {
            Op::SimdMem(m) => {
                assert_eq!((m.regs, m.selem, m.post), (2, 2, Post::Imm(32)));
                exec_mem(&mut st, &m).unwrap();
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(st.x[8], 0x120);
        // v0 gets words 0, 2, 4, 6; v1 gets words 1, 3, 5, 7.
        assert_eq!(st.v[0][0] as u32, 0x0302_0100);
        assert_eq!(st.v[1][0] as u32, 0x0706_0504);
        assert_eq!((st.v[0][0] >> 32) as u32, 0x0B0A_0908);
    }
}

#[cfg(test)]
mod estimate_tests {
    use super::{recip_estimate, rsqrt_estimate};

    #[test]
    fn estimates_match_known_values() {
        assert_eq!(recip_estimate(4, 0x3F80_0000), 0x3F7F_8000);
        assert_eq!(rsqrt_estimate(4, 0x3F80_0000), 0x3F7F_8000);
        assert_eq!(recip_estimate(4, 0x4000_0000), 0x3EFF_8000);
        assert_eq!(rsqrt_estimate(4, 0x4080_0000), 0x3EFF_8000);
        assert_eq!(recip_estimate(4, 0), 0x7F80_0000);
        assert_eq!(rsqrt_estimate(4, 0xBF80_0000), 0x7FC0_0000);
    }

    /// Against vectors from an independent CPU model:
    /// `EW_EST_VECTORS=build/est_vectors.txt cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn estimates_match_reference_vectors() {
        let path = std::env::var("EW_EST_VECTORS").expect("EW_EST_VECTORS");
        let text = std::fs::read_to_string(path).unwrap();
        let mut bad = 0;
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            let (a, want) = (u64::from_str_radix(f[1], 16).unwrap(), u64::from_str_radix(f[2], 16).unwrap());
            let got = match f[0] {
                "recpe_s" => recip_estimate(4, a),
                "rsqrte_s" => rsqrt_estimate(4, a),
                "recpe_d" => recip_estimate(8, a),
                _ => rsqrt_estimate(8, a),
            };
            if got != want {
                bad += 1;
                if bad < 10 {
                    eprintln!("{} {a:x}: got {got:x}, want {want:x}", f[0]);
                }
            }
        }
        assert_eq!(bad, 0);
    }
}
