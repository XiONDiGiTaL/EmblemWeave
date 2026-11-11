//! AArch64 (ARMv8-A) instruction decoder for the subset needed by lifted
//! Nintendo Switch 2 modules.
//!
//! This is a *decoder*, not an interpreter: it produces a structured
//! description of each instruction which `lift` turns into C. Anything not
//! recognised is reported as `Op::Unsupported` so the emitted code traps at
//! run time instead of executing a wrong guess.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Cond {
    Eq,
    Ne,
    Cs,
    Cc,
    Mi,
    Pl,
    Vs,
    Vc,
    Hi,
    Ls,
    Ge,
    Lt,
    Gt,
    Le,
    Al,
    Nv,
}

impl Cond {
    pub fn from_bits(b: u32) -> Option<Cond> {
        use Cond::*;
        Some(match b & 0xF {
            0x0 => Eq,
            0x1 => Ne,
            0x2 => Cs,
            0x3 => Cc,
            0x4 => Mi,
            0x5 => Pl,
            0x6 => Vs,
            0x7 => Vc,
            0x8 => Hi,
            0x9 => Ls,
            0xA => Ge,
            0xB => Lt,
            0xC => Gt,
            0xD => Le,
            0xE => Al,
            _ => Nv,
        })
    }

    /// C expression suffix for this condition, used by the emitter.
    pub fn c_suffix(self) -> &'static str {
        match self {
            Cond::Eq => "eq",
            Cond::Ne => "ne",
            Cond::Cs => "cs",
            Cond::Cc => "cc",
            Cond::Mi => "mi",
            Cond::Pl => "pl",
            Cond::Vs => "vs",
            Cond::Vc => "vc",
            Cond::Hi => "hi",
            Cond::Ls => "ls",
            Cond::Ge => "ge",
            Cond::Lt => "lt",
            Cond::Gt => "gt",
            Cond::Le => "le",
            Cond::Al => "al",
            Cond::Nv => "nv",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LogicalOp {
    And,
    Orr,
    Eor,
    Ands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Shift {
    Lsl,
    Lsr,
    Asr,
    Ror,
}

impl Shift {
    fn from_bits(b: u32) -> Shift {
        match b & 3 {
            0 => Shift::Lsl,
            1 => Shift::Lsr,
            2 => Shift::Asr,
            _ => Shift::Ror,
        }
    }
    pub fn c_op(self) -> &'static str {
        match self {
            Shift::Lsl => "<<",
            Shift::Lsr => ">>",
            Shift::Asr => ">>s",
            Shift::Ror => "rors",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MemMode {
    Offset,
    PreIndex,
    PostIndex,
}

/// Operand extension used by register-offset addressing and by extended-register
/// add/subtract. The numeric values match the `option` field encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Extend {
    Uxtb = 0,
    Uxth = 1,
    Uxtw = 2,
    Uxtx = 3,
    Sxtb = 4,
    Sxth = 5,
    Sxtw = 6,
    Sxtx = 7,
}

impl Extend {
    pub fn from_option(option: u32) -> Extend {
        match option & 7 {
            0 => Extend::Uxtb,
            1 => Extend::Uxth,
            2 => Extend::Uxtw,
            3 => Extend::Uxtx,
            4 => Extend::Sxtb,
            5 => Extend::Sxth,
            6 => Extend::Sxtw,
            _ => Extend::Sxtx,
        }
    }

    /// True when the extension leaves the operand as a full 64-bit value.
    pub fn is_64(self) -> bool {
        matches!(self, Extend::Uxtx | Extend::Sxtx)
    }

    pub fn is_signed(self) -> bool {
        matches!(self, Extend::Sxtb | Extend::Sxth | Extend::Sxtw | Extend::Sxtx)
    }
}

/// The two-source integer operations that appear in real code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DataOp2 {
    Udiv,
    Sdiv,
    Lslv,
    Lsrv,
    Asrv,
    Rorv,
}

impl DataOp2 {
    fn from_opcode(op: u32) -> Option<DataOp2> {
        Some(match op {
            2 => DataOp2::Udiv,
            3 => DataOp2::Sdiv,
            8 => DataOp2::Lslv,
            9 => DataOp2::Lsrv,
            10 => DataOp2::Asrv,
            11 => DataOp2::Rorv,
            _ => return None,
        })
    }

    pub fn c_name(self) -> &'static str {
        match self {
            DataOp2::Udiv => "udiv",
            DataOp2::Sdiv => "sdiv",
            DataOp2::Lslv => "lslv",
            DataOp2::Lsrv => "lsrv",
            DataOp2::Asrv => "asrv",
            DataOp2::Rorv => "rorv",
        }
    }
}

/// Single-source bit manipulation, encoded in one family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BitScan {
    Rbit,
    Rev16,
    Rev32,
    Rev,
    Clz,
    Cls,
}

impl BitScan {
    fn from_opcode(op: u32) -> Option<BitScan> {
        Some(match op {
            0 => BitScan::Rbit,
            1 => BitScan::Rev16,
            2 => BitScan::Rev32,
            3 => BitScan::Rev,
            4 => BitScan::Clz,
            5 => BitScan::Cls,
            _ => return None,
        })
    }

    pub fn c_name(self) -> &'static str {
        match self {
            BitScan::Rbit => "rbit",
            BitScan::Rev16 => "rev16",
            BitScan::Rev32 => "rev32",
            BitScan::Rev => "rev",
            BitScan::Clz => "clz",
            BitScan::Cls => "cls",
        }
    }
}

/// Scalar floating point unary operations, one family of six-bit opcodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FpUnop {
    Neg,
    Abs,
    Sqrt,
    /// Round to nearest, ties to even.
    RoundN,
    /// Round toward +infinity.
    RoundP,
    /// Round toward -infinity.
    RoundM,
    /// Round toward zero.
    RoundZ,
    /// Round to nearest, ties away from zero.
    RoundA,
    /// Round to nearest even, reporting inexact in the cumulative flag.
    RoundX,
    /// Round to integral using the current rounding mode, which is nearest even.
    RoundI,
    /// Convert between single and double precision. The destination width is
    /// the opposite of `FpUnary::is64`.
    Cvt,
}

impl FpUnop {
    fn from_opcode(op: u32) -> Option<FpUnop> {
        Some(match op {
            0b000010 => FpUnop::Neg,
            0b000001 => FpUnop::Abs,
            0b000011 => FpUnop::Sqrt,
            0b001000 => FpUnop::RoundN,
            0b001001 => FpUnop::RoundP,
            0b001010 => FpUnop::RoundM,
            0b001011 => FpUnop::RoundZ,
            0b001100 => FpUnop::RoundA,
            0b001110 => FpUnop::RoundX,
            0b001111 => FpUnop::RoundI,
            0b000100 | 0b000101 => FpUnop::Cvt,
            _ => return None,
        })
    }
}

/// Scalar floating point binary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FpBinop {
    Mul,
    Div,
    Add,
    Sub,
    Max,
    Min,
    /// Maximum, quiet on NaN.
    MaxNm,
    /// Minimum, quiet on NaN.
    MinNm,
    /// Negated multiply, used by `fnmul`.
    NegMul,
}

impl FpBinop {
    fn from_opcode(op: u32) -> Option<FpBinop> {
        Some(match op {
            0 => FpBinop::Mul,
            1 => FpBinop::Div,
            2 => FpBinop::Add,
            3 => FpBinop::Sub,
            4 => FpBinop::Max,
            5 => FpBinop::Min,
            6 => FpBinop::MaxNm,
            7 => FpBinop::MinNm,
            8 => FpBinop::NegMul,
            _ => return None,
        })
    }

    pub fn is_commutative(self) -> bool {
        !matches!(self, FpBinop::Sub | FpBinop::Div | FpBinop::NegMul)
    }
}

/// Rounding mode used by the float to integer conversions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FpRound {
    NearestEven,
    NearestAway,
    PlusInf,
    MinusInf,
    Zero,
}

impl FpRound {
    /// Map the five-bit opcode field to a rounding mode and signedness. The low
    /// bit of the field selects unsigned, which is why the pairs are adjacent.
    fn from_opcode(op: u32) -> Option<(FpRound, bool)> {
        let unsigned = op & 1 != 0;
        let round = match op & !1 {
            0x00 => FpRound::NearestEven,
            0x04 => FpRound::NearestAway,
            0x08 => FpRound::PlusInf,
            0x10 => FpRound::MinusInf,
            0x18 => FpRound::Zero,
            _ => return None,
        };
        Some((round, unsigned))
    }

    pub fn c_code(self) -> u32 {
        match self {
            FpRound::NearestEven => 0,
            FpRound::NearestAway => 1,
            FpRound::PlusInf => 2,
            FpRound::MinusInf => 3,
            FpRound::Zero => 4,
        }
    }
}

/// Read-modify-write operation from the LSE atomic family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AtomicOp {
    Add,
    Clr,
    Eor,
    Set,
    SMax,
    SMin,
    UMax,
    UMin,
    Swap,
}

impl AtomicOp {
    fn from_opc(opc: u32) -> Option<AtomicOp> {
        Some(match opc {
            0 => AtomicOp::Add,
            1 => AtomicOp::Clr,
            2 => AtomicOp::Eor,
            3 => AtomicOp::Set,
            4 => AtomicOp::SMax,
            5 => AtomicOp::SMin,
            6 => AtomicOp::UMax,
            7 => AtomicOp::UMin,
            8 => AtomicOp::Swap,
            _ => return None,
        })
    }

    pub fn c_name(self) -> &'static str {
        match self {
            AtomicOp::Add => "add",
            AtomicOp::Clr => "clr",
            AtomicOp::Eor => "eor",
            AtomicOp::Set => "set",
            AtomicOp::SMax => "smax",
            AtomicOp::SMin => "smin",
            AtomicOp::UMax => "umax",
            AtomicOp::UMin => "umin",
            AtomicOp::Swap => "swp",
        }
    }
}

/// Ordering requested by an atomic access. The lifted runtime is single
/// threaded, so the access is performed sequentially; the ordering is recorded
/// so the emitted comment and the interpreter agree about what was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AtomicOrder {
    pub acquire: bool,
    pub release: bool,
}

/// System registers the guest reads and writes, as the 15-bit field of
/// `mrs`/`msr` (op0:op1:CRn:CRm:op2).
pub mod sysreg {
    const fn id(word: u32) -> u16 {
        ((word >> 5) & 0x7FFF) as u16
    }
    pub const TPIDRRO_EL0: u16 = id(0xD53B_D060);
    pub const TPIDR_EL0: u16 = id(0xD53B_D040);
    pub const CNTPCT_EL0: u16 = id(0xD53B_E020);
    pub const CNTVCT_EL0: u16 = id(0xD53B_E040);
    pub const CNTFRQ_EL0: u16 = id(0xD53B_E000);
    pub const FPCR: u16 = id(0xD53B_4400);
    pub const FPSR: u16 = id(0xD53B_4420);
    pub const CTR_EL0: u16 = id(0xD53B_0020);
    pub const DCZID_EL0: u16 = id(0xD53B_00E0);
    pub const NZCV: u16 = id(0xD53B_4200);
}

/// A decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Op {
    Nop,
    /// `b` — unconditional branch to an absolute guest address.
    B { target: u64 },
    /// `bl` — call, setting x30.
    Bl { target: u64 },
    /// `b.cond`
    BCond { cond: Cond, target: u64 },
    /// `cbz` / `cbnz`
    CompareBranchZero { rt: u8, target: u64, nonzero: bool, is64: bool },
    /// `tbz` / `tbnz`
    TestBitBranch { rt: u8, bit: u8, target: u64, nonzero: bool },
    Br { rn: u8 },
    Blr { rn: u8 },
    Ret { rn: u8 },
    /// `adr` / `adrp`
    Adr { rd: u8, target: u64 },
    /// `movz` / `movn` / `movk` — `kind` is 0/1/2 respectively.
    MoveWide { rd: u8, imm: u16, shift: u8, kind: u8, is64: bool },
    /// `add`/`sub` with an immediate (covers `cmp`/`cmn` when `set_flags`).
    AddSubImm { rd: u8, rn: u8, imm: u32, sub: bool, set_flags: bool, is64: bool },
    /// `add`/`sub` with a shifted register.
    AddSubReg {
        rd: u8,
        rn: u8,
        rm: u8,
        shift: Shift,
        amount: u8,
        sub: bool,
        set_flags: bool,
        is64: bool,
    },
    /// `and`/`orr`/`eor`/`ands` with a bitmask immediate.
    LogicalImm { rd: u8, rn: u8, imm: u64, op: LogicalOp, set_flags: bool, is64: bool },
    /// `and`/`orr`/`eor`/`ands` with a shifted register (covers `mov` reg),
    /// and with `invert` (bit 21) their inverting forms `bic`/`orn`/`eon`/
    /// `bics` (covers `mvn`): the shifted operand is complemented first.
    LogicalReg {
        rd: u8,
        rn: u8,
        rm: u8,
        shift: Shift,
        amount: u8,
        op: LogicalOp,
        set_flags: bool,
        is64: bool,
        invert: bool,
    },
    /// Scaled `ldr`/`str` (also `ldur`/`stur` with `mode == Offset`).
    ///
    /// `is64` is the width of the destination register, and `signed` says the
    /// loaded value is sign extended from `bytes` into that width. Together they
    /// cover `ldrsb`, `ldrsh`, `ldrsw` and their unscaled/indexed forms.
    LoadStore {
        rt: u8,
        rn: u8,
        offset: i64,
        bytes: u8,
        load: bool,
        signed: bool,
        mode: MemMode,
        is64: bool,
    },
    /// `ldp`/`stp`.
    LoadStorePair {
        rt: u8,
        rt2: u8,
        rn: u8,
        offset: i64,
        load: bool,
        mode: MemMode,
        is64: bool,
    },
    /// `ldr` literal from the text section.
    LoadLiteral { rt: u8, target: u64, is64: bool },
    /// `madd`/`msub`.
    MultiplyAdd { rd: u8, rn: u8, rm: u8, ra: u8, sub: bool, is64: bool },
    /// `csel`/`csinc`/`csinv`/`csneg` — `kind` is 0..3.
    CondSelect { rd: u8, rn: u8, rm: u8, cond: Cond, kind: u8, is64: bool },
    /// `add`/`sub` with an extended register operand, e.g. `add x9, x9, w8, sxtw #2`.
    AddSubExt {
        rd: u8,
        rn: u8,
        rm: u8,
        option: Extend,
        /// Left shift applied after extension, encoded in `imm3` (0..4).
        shift: u8,
        sub: bool,
        set_flags: bool,
        is64: bool,
    },
    /// `sbfm`/`bfm`/`ubfm`, which also cover `sbfx`, `ubfx`, `sbfiz`, `ubfiz`,
    /// `bfi`, `bfxil`, `lsl`, `lsr` and `asr` with an immediate.
    Bitfield { rd: u8, rn: u8, immr: u8, imms: u8, opc: u8, is64: bool },
    /// `ldr`/`str` with a register offset and an extend, e.g. `ldrb w10, [x9, w8, uxtw]`.
    LoadStoreReg {
        rt: u8,
        rn: u8,
        rm: u8,
        option: Extend,
        shift: u8,
        bytes: u8,
        load: bool,
        signed: bool,
        is64: bool,
    },
    /// `smaddl`/`smull`/`smsubl` and their unsigned forms.
    MulLong { rd: u8, rn: u8, rm: u8, ra: u8, sub: bool, unsigned: bool },
    /// `udiv`, `sdiv`, `lslv`, `lsrv`, `asrv`, `rorv`.
    DataProc2 { rd: u8, rn: u8, rm: u8, op: DataOp2, is64: bool },
    /// `rbit`, `rev16`, `rev32`, `rev`, `clz`, `cls`.
    BitScan { rd: u8, rn: u8, op: BitScan, is64: bool },
    /// `crc32b`, `crc32h`, `crc32w`, `crc32x` and the `crc32c` Castagnoli forms.
    /// `bytes` is the operand width the produced CRC absorbs: 1, 2, 4 or 8.
    Crc32 { rd: u8, rn: u8, rm: u8, bytes: u8, castagnoli: bool },
    /// `ccmp` / `ccmn`, with either a register or a five-bit immediate.
    CondCompare {
        rn: u8,
        rm: u8,
        /// Immediate operand, valid only when `immediate` is set.
        imm5: u8,
        immediate: bool,
        cond: Cond,
        /// NZCV written directly when the condition fails.
        nzcv: u8,
        /// True for `ccmp` (subtract), false for `ccmn` (add).
        sub: bool,
        is64: bool,
    },
    /// `ldadd`, `ldclr`, `ldeor`, `ldset`, `ldsmax`, `ldsmin`, `ldumax`,
    /// `ldumin` and `swp`, including their ordered and byte/half/word forms.
    AtomicRmw {
        rs: u8,
        rt: u8,
        rn: u8,
        op: AtomicOp,
        order: AtomicOrder,
        /// Access width in bytes: 1, 2, 4 or 8.
        bytes: u8,
    },
    /// `ldr`/`str` of a SIMD/FP register, byte for byte.
    VectorLoadStore { rt: u8, rn: u8, offset: i64, bytes: u8, load: bool, mode: MemMode },
    /// `ldr`/`str` of a SIMD/FP register with a register offset.
    VectorLoadStoreReg { rt: u8, rn: u8, rm: u8, option: Extend, shift: u8, bytes: u8, load: bool },
    /// `ldp`/`stp` of SIMD/FP register pairs.
    VectorLoadStorePair {
        rt: u8,
        rt2: u8,
        rn: u8,
        offset: i64,
        bytes: u8,
        load: bool,
        mode: MemMode,
    },
    /// `movi` whose immediate is zero, which is how the compiler clears a
    /// vector register. `full` is true for the 128-bit (`v`) form.
    VectorZero { rd: u8, full: bool },
    /// `fmov` between a general purpose register and a SIMD/FP register.
    FpMoveGp { fp: u8, gp: u8, to_fp: bool, is64: bool },
    /// `fmov` between two SIMD/FP registers of the same width.
    FpCopy { rd: u8, rn: u8, is64: bool },
    /// Scalar floating point unary operation. `is64` is the operand width, and
    /// for `FpUnop::Cvt` it is the *source* width, because `fcvt` writes the
    /// other precision.
    FpUnary { rd: u8, rn: u8, op: FpUnop, is64: bool },
    /// Scalar floating point binary arithmetic.
    FpBinary { rd: u8, rn: u8, rm: u8, op: FpBinop, is64: bool },
    /// `fcmp`/`fcmpe` of a register against a register. `signal` is the `e`
    /// form, which raises on a quiet NaN; the host does not model FP traps, so
    /// both forms write the same flags.
    FpCompare { rn: u8, rm: u8, is64: bool, signal: bool },
    /// `fcmp`/`fcmpe` against the only immediate the encoding allows, `#0.0`.
    FpCompareZero { rn: u8, is64: bool, signal: bool },
    /// `fccmp`/`fccmpe`: compare when `cond` holds, otherwise write `nzcv`.
    FpCondCompare { rn: u8, rm: u8, cond: Cond, nzcv: u8, is64: bool },
    /// `fcsel`, the floating point conditional select.
    FpCondSelect { rd: u8, rn: u8, rm: u8, cond: Cond, is64: bool },
    /// `fcvtzs` and the rest of the float to integer conversions, with the
    /// rounding mode the encoding selects.
    FpToInt { rd: u8, rn: u8, round: FpRound, unsigned: bool, src_is64: bool, dest_is64: bool },
    /// `scvtf`/`ucvtf`: integer to floating point.
    IntToFp { rd: u8, rn: u8, unsigned: bool, src_is64: bool, dest_is64: bool },
    /// Fixed-point `scvtf`/`ucvtf`: the integer scaled by 2^-fbits.
    IntToFpFixed { rd: u8, rn: u8, unsigned: bool, src_is64: bool, dest_is64: bool, fbits: u8 },
    /// Fixed-point `fcvtzs`/`fcvtzu`: the value scaled by 2^fbits, rounded
    /// toward zero and saturated.
    FpToIntFixed { rd: u8, rn: u8, unsigned: bool, src_is64: bool, dest_is64: bool, fbits: u8 },
    /// Round to an integral value and leave the result in a floating point
    /// register (`fcvtzs s0, s1`). This is not the integer conversion: the
    /// result stays floating point, which is why clang uses it as the first
    /// step of a float to unsigned conversion.
    FpRoundFp { rd: u8, rn: u8, round: FpRound, is64: bool },
    /// `fmov` with an eight bit encoded immediate.
    FpMovImm { rd: u8, imm8: u8, is64: bool },
    /// `fmadd`, `fmsub`, `fnmadd`, `fnmsub`.
    FpMulAdd {
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
        negate_product: bool,
        negate_addend: bool,
        is64: bool,
    },
    /// Bitfield move instruction we could not express as a simple shift.
    UnsupportedBitfield { word: u32 },
    /// `extr` (and `ror` by immediate): bits `lsb..` of the concatenation `rn:rm`.
    Extr { rd: u8, rn: u8, rm: u8, lsb: u8, is64: bool },
    /// `ldpsw`: two 32-bit loads, each sign-extended to 64 bits.
    LoadPairSigned { rt: u8, rt2: u8, rn: u8, offset: i64, mode: MemMode },
    /// `mrs xt, <sysreg>`: `reg` is the 15-bit op0:op1:CRn:CRm:op2 field.
    Mrs { rt: u8, reg: u16 },
    /// `msr <sysreg>, xt`.
    Msr { rt: u8, reg: u16 },
    /// `svc #imm`: a supervisor call, handled by the host kernel layer.
    Svc { imm: u16 },
    /// `umulh`/`smulh`: the high 64 bits of the 128-bit product.
    MulHigh { rd: u8, rn: u8, rm: u8, unsigned: bool },
    /// `ldxr`/`ldaxr`/`stxr`/`stlxr` and the pair forms `ldxp`/`stxp`.
    /// `rs` receives the store status (0 success, 1 failure).
    Exclusive { rs: u8, rt: u8, rt2: u8, rn: u8, bytes: u8, load: bool, pair: bool },
    /// `cas`/`casa`/`casl`/`casal` and `casp`: compare `rs` (and `rs+1`) with
    /// memory, store `rt` (and `rt+1`) when equal, return the old value in `rs`.
    Cas { rs: u8, rt: u8, rn: u8, bytes: u8, pair: bool },
    /// A register-only Advanced SIMD instruction (see `simd`).
    Simd(crate::simd::SimdOp),
    /// An Advanced SIMD structure load or store (see `simd`).
    SimdMem(crate::simd::SimdMem),
    /// Anything the decoder does not model.
    Unsupported { word: u32 },
}

impl Op {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Op::B { .. }
                | Op::BCond { .. }
                | Op::Br { .. }
                | Op::Blr { .. }
                | Op::Ret { .. }
                | Op::CompareBranchZero { .. }
                | Op::TestBitBranch { .. }
        )
    }

    pub fn is_call(&self) -> bool {
        matches!(self, Op::Bl { .. } | Op::Blr { .. })
    }

    /// Branches that may fall through to the next instruction.
    pub fn is_conditional_branch(&self) -> bool {
        matches!(
            self,
            Op::BCond { .. }
                | Op::CompareBranchZero { .. }
                | Op::TestBitBranch { .. }
        )
    }

    /// Instructions that can read or write guest memory.
    pub fn touches_memory(&self) -> bool {
        matches!(
            self,
            Op::LoadStore { .. }
                | Op::LoadStorePair { .. }
                | Op::LoadPairSigned { .. }
                | Op::LoadLiteral { .. }
                | Op::LoadStoreReg { .. }
                | Op::AtomicRmw { .. }
                | Op::VectorLoadStore { .. }
                | Op::VectorLoadStoreReg { .. }
                | Op::VectorLoadStorePair { .. }
                | Op::SimdMem(..)
                | Op::Exclusive { .. }
                | Op::Cas { .. }
        )
    }

    /// Instructions that exchange data between the integer and the floating
    /// point register files, which matters when a function is summarised.
    pub fn is_fp(&self) -> bool {
        matches!(
            self,
            Op::FpUnary { .. }
                | Op::FpBinary { .. }
                | Op::FpCompare { .. }
                | Op::FpCompareZero { .. }
                | Op::FpCondCompare { .. }
                | Op::FpCondSelect { .. }
                | Op::FpToInt { .. }
                | Op::IntToFp { .. }
                | Op::IntToFpFixed { .. }
                | Op::FpToIntFixed { .. }
                | Op::FpMovImm { .. }
                | Op::FpRoundFp { .. }
                | Op::FpMulAdd { .. }
                | Op::FpMoveGp { .. }
                | Op::FpCopy { .. }
                | Op::Simd(..)
        )
    }

    pub fn supports_64(&self) -> bool {
        !matches!(self, Op::Unsupported { .. } | Op::UnsupportedBitfield { .. })
    }
}

fn sext(value: u64, bits: u32) -> i64 {
    let shift = 64 - bits;
    ((value << shift) as i64) >> shift
}

/// Decode a 32-bit instruction located at guest address `addr`.
///
/// The scalar and integer decoder runs first; a word it does not model is
/// offered to the Advanced SIMD decoder before it is reported as unsupported.
pub fn decode(word: u32, addr: u64) -> Op {
    match decode_base(word, addr) {
        Op::Unsupported { word } => crate::simd::decode(word).unwrap_or(Op::Unsupported { word }),
        op => op,
    }
}

fn decode_base(word: u32, addr: u64) -> Op {
    // extr: sf 00 100111 N 0 Rm imms Rn Rd, with N == sf.
    if word & 0x7FA0_0000 == 0x1380_0000 {
        let is64 = word >> 31 != 0;
        let n = (word >> 22) & 1 != 0;
        let lsb = ((word >> 10) & 0x3F) as u8;
        if n == is64 && (is64 || lsb < 32) {
            return Op::Extr {
                rd: (word & 31) as u8,
                rn: ((word >> 5) & 31) as u8,
                rm: ((word >> 16) & 31) as u8,
                lsb,
                is64,
            };
        }
    }
    // umulh / smulh: Ra must be 11111.
    if word & 0xFF60_FC00 == 0x9B40_7C00 {
        return Op::MulHigh {
            rd: (word & 31) as u8,
            rn: ((word >> 5) & 31) as u8,
            rm: ((word >> 16) & 31) as u8,
            unsigned: (word >> 23) & 1 != 0,
        };
    }
    // Load/store exclusive, ordered, and compare-and-swap: bits 29:24 = 001000.
    if word & 0x3F00_0000 == 0x0800_0000 {
        if let Some(op) = decode_exclusive(word) {
            return op;
        }
    }
    decode_base_rest(word, addr)
}

fn decode_exclusive(word: u32) -> Option<Op> {
    let size = (word >> 30) & 3;
    let o2 = (word >> 23) & 1;
    let load = (word >> 22) & 1 != 0;
    let o1 = (word >> 21) & 1;
    let rs = ((word >> 16) & 31) as u8;
    let rt2 = ((word >> 10) & 31) as u8;
    let rn = ((word >> 5) & 31) as u8;
    let rt = (word & 31) as u8;
    let bytes = 1u8 << size;
    match (o2, o1) {
        (0, 0) => Some(Op::Exclusive { rs, rt, rt2: 31, rn, bytes, load, pair: false }),
        (0, 1) if size >= 2 => Some(Op::Exclusive { rs, rt, rt2, rn, bytes, load, pair: true }),
        (0, 1) if rt2 == 31 && rs % 2 == 0 && rt % 2 == 0 => {
            // casp: size 00 is 32-bit pairs, 01 is 64-bit pairs.
            Some(Op::Cas { rs, rt, rn, bytes: if size == 0 { 4 } else { 8 }, pair: true })
        }
        // ldar/stlr and the limited-order ldlar/stllr: with one guest CPU
        // executing at a time they are plain loads and stores.
        (1, 0) if rs == 31 && rt2 == 31 => Some(Op::LoadStore {
            rt,
            rn,
            offset: 0,
            bytes,
            load,
            signed: false,
            mode: MemMode::Offset,
            is64: size == 3,
        }),
        (1, 1) if rt2 == 31 => Some(Op::Cas { rs, rt, rn, bytes, pair: false }),
        _ => None,
    }
}

fn decode_base_rest(word: u32, addr: u64) -> Op {
    // nop and the whole hint space. Switch 2 code is built with pointer
    // authentication, so pacibsp/paciasp/autibsp/autiasp appear in most
    // prologues and epilogues. They are modelled as nops: the host has no
    // authentication keys and the guest never inspects a signed pointer.
    if word & 0xFFFF_F01F == 0xD503_201F {
        return Op::Nop;
    }

    // Barriers: clrex, dsb, dmb, isb, sb (d503 3 CRm op2 11111). One guest
    // CPU runs at a time on host memory, so ordering is already total, and
    // exclusive pairs never fail, so there is no monitor for clrex to clear.
    if word & 0xFFFF_F01F == 0xD503_301F && matches!((word >> 5) & 7, 2 | 4 | 5 | 6 | 7) {
        return Op::Nop;
    }

    // Cache maintenance by address: dc cvac/cvau/civac/cvap/cvadp and
    // ic ivau (sys #3, c7, Cm, #op2). Guest memory is host memory and the
    // host keeps it coherent, so these are nops. dc zva (c7, c4, #1) writes
    // memory, so it is left to fall through as unsupported.
    if word & 0xFFF8_F000 == 0xD508_7000 && (word >> 16) & 7 == 3 {
        let crm = (word >> 8) & 0xF;
        let op2 = (word >> 5) & 7;
        if matches!((crm, op2), (10, 1) | (11, 1) | (12, 1) | (13, 1) | (14, 1) | (5, 1)) {
            return Op::Nop;
        }
    }

    // svc #imm16
    if word & 0xFFE0_001F == 0xD400_0001 {
        return Op::Svc { imm: ((word >> 5) & 0xFFFF) as u16 };
    }
    // mrs / msr (register form)
    if word & 0xFFF0_0000 == 0xD530_0000 {
        return Op::Mrs { rt: (word & 31) as u8, reg: ((word >> 5) & 0x7FFF) as u16 };
    }
    if word & 0xFFF0_0000 == 0xD510_0000 {
        return Op::Msr { rt: (word & 31) as u8, reg: ((word >> 5) & 0x7FFF) as u16 };
    }

    // Unconditional branch immediate: b / bl
    if word & 0x7C00_0000 == 0x1400_0000 {
        let imm = sext((word & 0x03FF_FFFF) as u64, 26) << 2;
        let target = (addr as i64).wrapping_add(imm) as u64;
        return if word & 0x8000_0000 != 0 {
            Op::Bl { target }
        } else {
            Op::B { target }
        };
    }

    // Conditional branch
    if word & 0xFF00_0010 == 0x5400_0000 {
        let imm = sext(((word >> 5) & 0x7_FFFF) as u64, 19) << 2;
        let target = (addr as i64).wrapping_add(imm) as u64;
        if let Some(cond) = Cond::from_bits(word) {
            return Op::BCond { cond, target };
        }
    }

    // Compare and branch (cbz / cbnz)
    if word & 0x7E00_0000 == 0x3400_0000 {
        let imm = sext(((word >> 5) & 0x7_FFFF) as u64, 19) << 2;
        let target = (addr as i64).wrapping_add(imm) as u64;
        return Op::CompareBranchZero {
            rt: (word & 0x1F) as u8,
            target,
            nonzero: (word >> 24) & 1 != 0,
            is64: (word >> 31) & 1 != 0,
        };
    }

    // Test and branch (tbz / tbnz)
    if word & 0x7E00_0000 == 0x3600_0000 {
        let imm = sext(((word >> 5) & 0x3FFF) as u64, 14) << 2;
        let target = (addr as i64).wrapping_add(imm) as u64;
        let bit = (((word >> 31) & 1) << 5 | ((word >> 19) & 0x1F)) as u8;
        return Op::TestBitBranch {
            rt: (word & 0x1F) as u8,
            bit,
            target,
            nonzero: (word >> 24) & 1 != 0,
        };
    }

    // Unconditional branch register
    if word & 0xFFFF_FC1F == 0xD61F_0000 {
        return Op::Br { rn: ((word >> 5) & 0x1F) as u8 };
    }
    if word & 0xFFFF_FC1F == 0xD63F_0000 {
        return Op::Blr { rn: ((word >> 5) & 0x1F) as u8 };
    }
    if word & 0xFFFF_FC1F == 0xD65F_0000 {
        return Op::Ret { rn: ((word >> 5) & 0x1F) as u8 };
    }

    // retaa / retab: authenticated returns. Pointer authentication is a no-op
    // on the host, so these are ordinary returns for lifting purposes.
    if word & 0xFFFF_FBFF == 0xD65F_0BFF {
        return Op::Ret { rn: 30 };
    }

    // PRFM / PRFUM: size = 11 and opc = 10 in the load/store register classes.
    // Prefetch has no architectural effect, so it becomes a nop. The match is
    // exact: opc = 11 with a smaller size is LDRSB/LDRSH into a W register, a
    // real load, and an earlier, looser mask turned every one of those into a
    // nop (found by tools/oracle_lockstep.py at 0x6c51c, `ldrsb w8, [x5]`).
    if word & 0xFFC0_0000 == 0xF980_0000 // unsigned offset
        || word & 0xFFE0_0C00 == 0xF880_0000 // unscaled (prfum)
        || word & 0xFFE0_0C00 == 0xF8A0_0800
    // register offset
    {
        return Op::Nop;
    }

    // ADR / ADRP
    if word & 0x1F00_0000 == 0x1000_0000 && (word & 0x8000_0000) == 0 {
        let immlo = ((word >> 29) & 3) as u64;
        let immhi = ((word >> 5) & 0x7_FFFF) as u64;
        let imm = sext(immhi << 2 | immlo, 21);
        return Op::Adr { rd: (word & 0x1F) as u8, target: (addr as i64).wrapping_add(imm) as u64 };
    }
    if word & 0x1F00_0000 == 0x1000_0000 && (word & 0x8000_0000) != 0 {
        let immlo = ((word >> 29) & 3) as u64;
        let immhi = ((word >> 5) & 0x7_FFFF) as u64;
        let imm = sext(immhi << 2 | immlo, 21) << 12;
        let page = addr & !0xFFF;
        return Op::Adr { rd: (word & 0x1F) as u8, target: (page as i64).wrapping_add(imm) as u64 };
    }

    // Move wide immediate
    let mw = word & 0x7F80_0000;
    if mw == 0x5280_0000 || mw == 0x1280_0000 || mw == 0x7280_0000 {
        let kind = match mw {
            0x5280_0000 => 0, // movz
            0x1280_0000 => 1, // movn
            _ => 2,           // movk
        };
        return Op::MoveWide {
            rd: (word & 0x1F) as u8,
            imm: ((word >> 5) & 0xFFFF) as u16,
            shift: (((word >> 21) & 3) * 16) as u8,
            kind,
            is64: (word >> 31) & 1 != 0,
        };
    }

    // Add/subtract (immediate)
    if word & 0x1F00_0000 == 0x1100_0000 {
        let imm = (word >> 10) & 0xFFF;
        let imm = if (word >> 22) & 1 != 0 { imm << 12 } else { imm };
        return Op::AddSubImm {
            rd: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            imm,
            sub: (word >> 30) & 1 != 0,
            set_flags: (word >> 29) & 1 != 0,
            is64: (word >> 31) & 1 != 0,
        };
    }

    // Add/subtract (extended register)
    if word & 0x1F20_0000 == 0x0B20_0000 {
        return Op::AddSubExt {
            rd: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            rm: ((word >> 16) & 0x1F) as u8,
            option: Extend::from_option((word >> 13) & 7),
            shift: ((word >> 10) & 7) as u8,
            sub: (word >> 30) & 1 != 0,
            set_flags: (word >> 29) & 1 != 0,
            is64: (word >> 31) & 1 != 0,
        };
    }

    // Add/subtract (shifted register)
    if word & 0x1F20_0000 == 0x0B00_0000 {
        return Op::AddSubReg {
            rd: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            rm: ((word >> 16) & 0x1F) as u8,
            shift: Shift::from_bits((word >> 22) & 3),
            amount: ((word >> 10) & 0x3F) as u8,
            sub: (word >> 30) & 1 != 0,
            set_flags: (word >> 29) & 1 != 0,
            is64: (word >> 31) & 1 != 0,
        };
    }

    // Logical (immediate)
    if word & 0x1F80_0000 == 0x1200_0000 {
        let opc = (word >> 29) & 3;
        let n = (word >> 22) & 1;
        let immr = (word >> 16) & 0x3F;
        let imms = (word >> 10) & 0x3F;
        let op = match opc {
            0 => LogicalOp::And,
            1 => LogicalOp::Orr,
            2 => LogicalOp::Eor,
            _ => LogicalOp::Ands,
        };
        return match decode_bitmask(n, immr, imms, (word >> 31) & 1 != 0) {
            Some(imm) => Op::LogicalImm {
                rd: (word & 0x1F) as u8,
                rn: ((word >> 5) & 0x1F) as u8,
                imm,
                op,
                set_flags: opc == 3,
                is64: (word >> 31) & 1 != 0,
            },
            None => Op::Unsupported { word },
        };
    }

    // Logical (shifted register)
    if word & 0x1F00_0000 == 0x0A00_0000 {
        let opc = (word >> 29) & 3;
        let op = match opc {
            0 => LogicalOp::And,
            1 => LogicalOp::Orr,
            2 => LogicalOp::Eor,
            _ => LogicalOp::Ands,
        };
        return Op::LogicalReg {
            rd: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            rm: ((word >> 16) & 0x1F) as u8,
            shift: Shift::from_bits((word >> 22) & 3),
            amount: ((word >> 10) & 0x3F) as u8,
            op,
            set_flags: opc == 3,
            is64: (word >> 31) & 1 != 0,
            invert: (word >> 21) & 1 != 0,
        };
    }

    // Bitfield move (sbfm / bfm / ubfm and all of their aliases)
    if word & 0x1F80_0000 == 0x1300_0000 {
        let sf = (word >> 31) & 1 != 0;
        let n = (word >> 22) & 1 != 0;
        if sf == n {
            return Op::Bitfield {
                rd: (word & 0x1F) as u8,
                rn: ((word >> 5) & 0x1F) as u8,
                immr: ((word >> 16) & 0x3F) as u8,
                imms: ((word >> 10) & 0x3F) as u8,
                opc: ((word >> 29) & 3) as u8,
                is64: sf,
            };
        }
        return Op::UnsupportedBitfield { word };
    }

    // Multiply-add long (smaddl / smsubl / umaddl / umsubl, with smull/umull as
    // the Ra = XZR aliases).
    if word & 0x7F20_0000 == 0x1B20_0000 {
        return Op::MulLong {
            rd: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            ra: ((word >> 10) & 0x1F) as u8,
            rm: ((word >> 16) & 0x1F) as u8,
            sub: (word >> 15) & 1 != 0,
            unsigned: (word >> 23) & 1 != 0,
        };
    }

    // Multiply-add / multiply-subtract. The op31 field at bits 23:21 must be
    // zero and the op54 field at bits 30:29 must be zero; testing bits 28:21
    // together would reject every real MADD the compiler emits.
    if word & 0x1F00_0000 == 0x1B00_0000 && (word >> 21) & 7 == 0 && (word >> 29) & 3 == 0 {
        return Op::MultiplyAdd {
            rd: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            ra: ((word >> 10) & 0x1F) as u8,
            rm: ((word >> 16) & 0x1F) as u8,
            sub: (word >> 15) & 1 != 0,
            is64: (word >> 31) & 1 != 0,
        };
    }

    // Single-source bit manipulation: rbit, rev16, rev32, rev, clz, cls. This
    // family shares its top bits with the two-source data-processing group, so
    // it has to be matched first.
    if word & 0x7FFF_0000 == 0x5AC0_0000 {
        if let Some(op) = BitScan::from_opcode((word >> 10) & 0x3F) {
            return Op::BitScan {
                rd: (word & 0x1F) as u8,
                rn: ((word >> 5) & 0x1F) as u8,
                op,
                is64: (word >> 31) & 1 != 0,
            };
        }
    }

    // CRC32 checksum: CRC32B/H/W/X. `sf` is set only for the 64-bit operand
    // form, which still returns a 32-bit checksum.
    if word & 0x7FE0_E000 == 0x1AC0_4000 {
        return Op::Crc32 {
            rd: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            rm: ((word >> 16) & 0x1F) as u8,
            bytes: 1 << ((word >> 10) & 3),
            castagnoli: (word >> 12) & 1 != 0,
        };
    }

    // Data-processing (2 source): udiv/sdiv and the variable shifts
    if word & 0x1FE0_0000 == 0x1AC0_0000 {
        if let Some(op) = DataOp2::from_opcode((word >> 10) & 0x3F) {
            return Op::DataProc2 {
                rd: (word & 0x1F) as u8,
                rn: ((word >> 5) & 0x1F) as u8,
                rm: ((word >> 16) & 0x1F) as u8,
                op,
                is64: (word >> 31) & 1 != 0,
            };
        }
        return Op::Unsupported { word };
    }

    // Conditional select
    if word & 0x1FE0_0000 == 0x1A80_0000 {
        // op (bit 30) and o2 (bit 10): 0 CSEL, 1 CSINC, 2 CSINV, 3 CSNEG.
        let kind = ((word >> 30) & 1) as u8;
        let o2 = (word >> 10) & 1;
        return Op::CondSelect {
            rd: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            rm: ((word >> 16) & 0x1F) as u8,
            cond: Cond::from_bits((word >> 12) & 0xF).unwrap_or(Cond::Al),
            kind: kind * 2 + o2 as u8,
            is64: (word >> 31) & 1 != 0,
        };
    }

    // LDAPR family: load-acquire with RCpc ordering (Armv8.3). The ordering is
    // vacuous on a single-threaded runtime, so the exact semantics here is a
    // plain zero-extending load of the encoded size. `ldaprb w8, [x8]` appears
    // in the game's static initialisation path.
    if word & 0x3FFF_FC00 == 0x38BF_C000 {
        let size = (word >> 30) & 3;
        return Op::LoadStore {
            rt: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            offset: 0,
            bytes: 1u8 << size,
            load: true,
            signed: false,
            mode: MemMode::Offset,
            is64: size == 3,
        };
    }

    // LSE atomics: ldadd family and swp, with the A and R ordering bits. Bit 21
    // is fixed to one in this family, which separates it from PRFM; bits 23:22
    // carry the ordering and therefore must stay outside the match mask.
    if word & 0x3F20_0C00 == 0x3820_0000 && word & 0x0400_0000 == 0 {
        if let Some(op) = AtomicOp::from_opc((word >> 12) & 0xF) {
            return Op::AtomicRmw {
                rs: ((word >> 16) & 0x1F) as u8,
                rt: (word & 0x1F) as u8,
                rn: ((word >> 5) & 0x1F) as u8,
                op,
                order: AtomicOrder {
                    acquire: (word >> 23) & 1 != 0,
                    release: (word >> 22) & 1 != 0,
                },
                bytes: 1u8 << ((word >> 30) & 3),
            };
        }
    }

    // Load/store register, register offset. This is the addressing mode that
    // jump tables use to read their index (`ldrb w8, [x9, w8, uxtw]`).
    if word & 0x3F20_0C00 == 0x3820_0800 {
        let size = (word >> 30) & 3;
        let opc = (word >> 22) & 3;
        // opc 0 stores, opc 1 loads zero-extended, opc 2 and 3 are the
        // sign-extending loads (to 64 and to 32 bits respectively).
        let scale = (word >> 12) & 1 != 0;
        return Op::LoadStoreReg {
            rt: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            rm: ((word >> 16) & 0x1F) as u8,
            option: Extend::from_option((word >> 13) & 7),
            shift: if scale { size as u8 } else { 0 },
            bytes: (1u8) << size,
            load: opc != 0,
            signed: opc >= 2,
            is64: if opc >= 2 { opc == 2 } else { size == 3 },
        };
    }

    // Load/store register, unsigned offset (general purpose registers only; the
    // V bit selects the SIMD/FP forms, decoded further down).
    if word & 0x3B00_0000 == 0x3900_0000 && word & 0x0400_0000 == 0 {
        let size = (word >> 30) & 3;
        let opc = (word >> 22) & 3;
        let imm = ((word >> 10) & 0xFFF) as i64;
        let bytes = 1u8 << size;
        return Op::LoadStore {
            rt: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            offset: imm * bytes as i64,
            bytes,
            load: opc != 0,
            signed: opc >= 2,
            mode: MemMode::Offset,
            is64: if opc >= 2 { opc == 2 } else { size == 3 },
        };
    }

    // Load/store register, unscaled (ldur/stur)
    if word & 0x3B20_0C00 == 0x3800_0000 && word & 0x0400_0000 == 0 {
        let size = (word >> 30) & 3;
        let opc = (word >> 22) & 3;
        let bytes = 1u8 << size;
        return Op::LoadStore {
            rt: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            offset: sext(((word >> 12) & 0x1FF) as u64, 9),
            bytes,
            load: opc != 0,
            signed: opc >= 2,
            mode: MemMode::Offset,
            is64: if opc >= 2 { opc == 2 } else { size == 3 },
        };
    }

    // Load/store register, pre/post indexed. Bit 10 is 1 in this group and bit
    // 11 picks pre (1) from post (0), so the mask must leave bit 11 free.
    if word & 0x3B20_0400 == 0x3800_0400 && word & 0x0400_0000 == 0 {
        let size = (word >> 30) & 3;
        let opc = (word >> 22) & 3;
        let bytes = 1u8 << size;
        return Op::LoadStore {
            rt: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            offset: sext(((word >> 12) & 0x1FF) as u64, 9),
            bytes,
            load: opc != 0,
            signed: opc >= 2,
            mode: if (word >> 11) & 1 != 0 { MemMode::PreIndex } else { MemMode::PostIndex },
            is64: if opc >= 2 { opc == 2 } else { size == 3 },
        };
    }

    // Load/store pair, general purpose registers only. The SIMD pair forms set
    // the V bit and are decoded separately: treating them as general purpose
    // pairs would turn a 16-byte vector store into a 4-byte integer store.
    if word & 0x3A00_0000 == 0x2800_0000 && word & 0x0400_0000 == 0 {
        let opc = (word >> 30) & 3;
        let is64 = opc == 2;
        let bytes = if is64 { 8 } else { 4 };
        let mode = match (word >> 23) & 3 {
            1 => MemMode::PostIndex,
            3 => MemMode::PreIndex,
            _ => MemMode::Offset,
        };
        if opc == 1 && (word >> 22) & 1 != 0 {
            return Op::LoadPairSigned {
                rt: (word & 0x1F) as u8,
                rt2: ((word >> 10) & 0x1F) as u8,
                rn: ((word >> 5) & 0x1F) as u8,
                offset: sext(((word >> 15) & 0x7F) as u64, 7) * 4,
                mode,
            };
        }
        if opc == 0 || opc == 2 {
            return Op::LoadStorePair {
                rt: (word & 0x1F) as u8,
                rt2: ((word >> 10) & 0x1F) as u8,
                rn: ((word >> 5) & 0x1F) as u8,
                offset: sext(((word >> 15) & 0x7F) as u64, 7) * bytes,
                load: (word >> 22) & 1 != 0,
                mode,
                is64,
            };
        }
    }

    // Load register (literal)
    if word & 0x3B00_0000 == 0x1800_0000 {
        let opc = (word >> 30) & 3;
        let imm = sext(((word >> 5) & 0x7_FFFF) as u64, 19) << 2;
        return Op::LoadLiteral {
            rt: (word & 0x1F) as u8,
            target: (addr as i64).wrapping_add(imm) as u64,
            is64: opc == 1,
        };
    }

    // Conditional compare: ccmp / ccmn, register and immediate forms.
    if word & 0x3FE0_0410 == 0x3A40_0000 {
        let immediate = (word >> 11) & 1 != 0;
        let field = ((word >> 16) & 0x1F) as u8;
        return Op::CondCompare {
            rn: ((word >> 5) & 0x1F) as u8,
            rm: if immediate { 0 } else { field },
            imm5: if immediate { field } else { 0 },
            immediate,
            cond: Cond::from_bits((word >> 12) & 0xF).unwrap_or(Cond::Al),
            nzcv: (word & 0xF) as u8,
            sub: (word >> 30) & 1 != 0,
            is64: (word >> 31) & 1 != 0,
        };
    }

    // SIMD/FP load and store, register form. The width is 16 bytes when opc bit
    // 1 is set (the `q` form), otherwise it is the access size.
    let vec_width = |size: u32, opc: u32| -> u8 {
        if opc & 2 != 0 {
            16
        } else {
            1u8 << size
        }
    };

    // unsigned offset. imm12 lives in bits 21:10, so none of those bits can be
    // part of the match mask.
    if word & 0x3F00_0000 == 0x3D00_0000 {
        let size = (word >> 30) & 3;
        let opc = (word >> 22) & 3;
        let bytes = vec_width(size, opc);
        return Op::VectorLoadStore {
            rt: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            offset: (((word >> 10) & 0xFFF) as i64) * bytes as i64,
            bytes,
            load: opc & 1 != 0,
            mode: MemMode::Offset,
        };
    }

    // unscaled (ldur/stur)
    if word & 0x3F20_0C00 == 0x3C00_0000 {
        let size = (word >> 30) & 3;
        let opc = (word >> 22) & 3;
        return Op::VectorLoadStore {
            rt: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            offset: sext(((word >> 12) & 0x1FF) as u64, 9),
            bytes: vec_width(size, opc),
            load: opc & 1 != 0,
            mode: MemMode::Offset,
        };
    }

    // pre/post indexed. As with the integer form, bit 10 is 1 and bit 11 picks
    // pre (1) from post (0).
    if word & 0x3F20_0400 == 0x3C00_0400 {
        let size = (word >> 30) & 3;
        let opc = (word >> 22) & 3;
        return Op::VectorLoadStore {
            rt: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            offset: sext(((word >> 12) & 0x1FF) as u64, 9),
            bytes: vec_width(size, opc),
            load: opc & 1 != 0,
            mode: if (word >> 11) & 1 != 0 { MemMode::PreIndex } else { MemMode::PostIndex },
        };
    }

    // register offset
    if word & 0x3F20_0C00 == 0x3C20_0800 {
        let size = (word >> 30) & 3;
        let opc = (word >> 22) & 3;
        let scale = (word >> 12) & 1 != 0;
        return Op::VectorLoadStoreReg {
            rt: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            rm: ((word >> 16) & 0x1F) as u8,
            option: Extend::from_option((word >> 13) & 7),
            shift: if scale { size as u8 } else { 0 },
            bytes: vec_width(size, opc),
            load: opc & 1 != 0,
        };
    }

    // SIMD/FP pair load and store. Width 4 for the `s` form, 8 for `d`, 16 for
    // `q`, selected by the size field.
    if word & 0x3E00_0000 == 0x2C00_0000 {
        let opc = (word >> 30) & 3;
        let bytes = 4u8 << opc;
        let mode = match (word >> 23) & 3 {
            1 => MemMode::PostIndex,
            3 => MemMode::PreIndex,
            _ => MemMode::Offset,
        };
        return Op::VectorLoadStorePair {
            rt: (word & 0x1F) as u8,
            rt2: ((word >> 10) & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            offset: sext(((word >> 15) & 0x7F) as u64, 7) * bytes as i64,
            bytes,
            load: (word >> 22) & 1 != 0,
            mode,
        };
    }

    // movi with a zero immediate: the idiom compilers use to clear a vector.
    if word & 0x9FF8_0C00 == 0x0F00_0400 {
        let imm8 = (((word >> 16) & 7) << 5) | ((word >> 5) & 0x1F);
        if imm8 == 0 && (word >> 12) & 0xF == 0xE {
            return Op::VectorZero { rd: (word & 0x1F) as u8, full: (word >> 30) & 1 != 0 };
        }
        return Op::Unsupported { word };
    }

    // fmov between a general purpose register and a SIMD/FP register. Bits
    // 15:10 must be zero: with them set the same top bits are fmov with an
    // immediate whose imm8 is 0x30..0x3f (fmov s0, #14.0 is 0x1e26d000).
    if word & 0x7FBF_FC00 == 0x1E27_0000 {
        return Op::FpMoveGp {
            fp: (word & 0x1F) as u8,
            gp: ((word >> 5) & 0x1F) as u8,
            to_fp: true,
            is64: (word >> 31) & 1 != 0,
        };
    }
    if word & 0x7FBF_FC00 == 0x1E26_0000 {
        return Op::FpMoveGp {
            fp: ((word >> 5) & 0x1F) as u8,
            gp: (word & 0x1F) as u8,
            to_fp: false,
            is64: (word >> 31) & 1 != 0,
        };
    }

    // fmov between two SIMD/FP registers (a pure bit copy of the low half).
    if word & 0x7FBF_FC00 == 0x1E20_4000 {
        return Op::FpCopy {
            rd: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            is64: (word >> 22) & 1 != 0,
        };
    }

    // Scalar floating point. Every family below was checked against clang
    // output and an independent disassembler rather than written from the
    // manual, because one wrong mask here is a silent numerical bug.

    // Three source: fmadd, fmsub, fnmadd, fnmsub. Bit 21 negates the product
    // and bit 15 negates the addend, which is the whole difference between the
    // four mnemonics.
    if word & 0xFF80_0000 == 0x1F00_0000 {
        return Op::FpMulAdd {
            rd: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            ra: ((word >> 10) & 0x1F) as u8,
            rm: ((word >> 16) & 0x1F) as u8,
            negate_product: (word >> 21) & 1 != 0,
            negate_addend: (word >> 15) & 1 != 0,
            is64: (word >> 22) & 1 != 0,
        };
    }

    // Two source arithmetic. Bits 11:10 are 10, which separates this family
    // from the one source encodings that share bits 31:21.
    if word & 0xFFA0_0C00 == 0x1E20_0800 {
        if let Some(op) = FpBinop::from_opcode((word >> 12) & 0xF) {
            return Op::FpBinary {
                rd: (word & 0x1F) as u8,
                rn: ((word >> 5) & 0x1F) as u8,
                rm: ((word >> 16) & 0x1F) as u8,
                op,
                is64: (word >> 22) & 1 != 0,
            };
        }
        return Op::Unsupported { word };
    }

    // fcmp and fcmpe. Bits 15:10 are 001000. Bit 4 is the signalling form. The
    // immediate form is only defined for #0.0, and it is the one place where
    // bits 3:0 are not zero, so the two forms are distinguishable.
    if word & 0xFFA0_FC00 == 0x1E20_2000 {
        let signal = (word >> 4) & 1 != 0;
        let is64 = (word >> 22) & 1 != 0;
        let rn = ((word >> 5) & 0x1F) as u8;
        let rm = (word >> 16) & 0x1F;
        if rm == 0 && (word & 0xF) == 8 {
            return Op::FpCompareZero { rn, is64, signal };
        }
        return Op::FpCompare { rn, rm: rm as u8, is64, signal };
    }

    // fccmp and fccmpe: the condition sits in bits 15:12 and bits 11:10 are 01.
    if word & 0xFFA0_0C00 == 0x1E20_0400 {
        return Op::FpCondCompare {
            rn: ((word >> 5) & 0x1F) as u8,
            rm: ((word >> 16) & 0x1F) as u8,
            cond: Cond::from_bits((word >> 12) & 0xF).unwrap_or(Cond::Al),
            nzcv: (word & 0xF) as u8,
            is64: (word >> 22) & 1 != 0,
        };
    }

    // fcsel: the condition sits in bits 15:12 and bits 11:10 are 11.
    if word & 0xFFA0_0C00 == 0x1E20_0C00 {
        return Op::FpCondSelect {
            rd: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            rm: ((word >> 16) & 0x1F) as u8,
            cond: Cond::from_bits((word >> 12) & 0xF).unwrap_or(Cond::Al),
            is64: (word >> 22) & 1 != 0,
        };
    }

    // Fixed-point conversions: bit 21 clear, the scale in bits 15:10.
    if word & 0x7F20_0000 == 0x1E00_0000 && (word >> 22) & 3 < 2 {
        let fbits = (64 - ((word >> 10) & 0x3F)) as u8;
        let (rd, rn) = ((word & 0x1F) as u8, ((word >> 5) & 0x1F) as u8);
        let (double, wide_int) = ((word >> 22) & 1 != 0, (word >> 31) & 1 != 0);
        let (rmode, opcode) = ((word >> 19) & 3, (word >> 16) & 7);
        if wide_int || fbits <= 32 {
            match (rmode, opcode) {
                (0, 2) | (0, 3) => {
                    return Op::IntToFpFixed { rd, rn, unsigned: opcode == 3, src_is64: wide_int, dest_is64: double, fbits }
                }
                (3, 0) | (3, 1) => {
                    return Op::FpToIntFixed { rd, rn, unsigned: opcode == 1, src_is64: double, dest_is64: wide_int, fbits }
                }
                _ => {}
            }
        }
    }

    // Conversions between floating point and integer. Bits 15:10 are zero, bit
    // 31 selects a 64-bit integer operand, bit 22 selects double precision and
    // bits 20:16 carry the rounding mode plus the unsigned bit.
    if word & 0x7FA0_FC00 == 0x1E20_0000 {
        let double = (word >> 22) & 1 != 0;
        let wide_int = (word >> 31) & 1 != 0;
        let op = (word >> 16) & 0x1F;
        let rn = ((word >> 5) & 0x1F) as u8;
        let rd = (word & 0x1F) as u8;
        if op == 0x02 || op == 0x03 {
            return Op::IntToFp {
                rd,
                rn,
                unsigned: op == 0x03,
                src_is64: wide_int,
                dest_is64: double,
            };
        }
        if let Some((round, unsigned)) = FpRound::from_opcode(op) {
            return Op::FpToInt { rd, rn, round, unsigned, src_is64: double, dest_is64: wide_int };
        }
        return Op::Unsupported { word };
    }

    // One source: fneg, fabs, fsqrt, the frint family and fcvt between
    // precisions. Bits 14:10 are 10000.
    if word & 0xFFA0_7C00 == 0x1E20_4000 {
        let opcode = (word >> 15) & 0x3F;
        let rd = (word & 0x1F) as u8;
        let rn = ((word >> 5) & 0x1F) as u8;
        let is64 = (word >> 22) & 1 != 0;
        return match FpUnop::from_opcode(opcode) {
            // fcvt fixes its destination width in the opcode, so an encoding
            // where the two disagree is unallocated: refuse it rather than
            // pick one and produce a silently wrong number.
            Some(FpUnop::Cvt) if (opcode == 4) != is64 => Op::Unsupported { word },
            Some(op) => Op::FpUnary { rd, rn, op, is64 },
            None => Op::Unsupported { word },
        };
    }

    // Scalar floating point rounding to an integral value, result in a
    // floating point register. Bits 20:16 are 1 and bits 11:10 are 10; the
    // rounding mode lives in bit 23 and bits 15:12, and bit 29 is the unsigned
    // form. Ten mnemonics, none of them a conversion to integer.
    if word & 0xDF3F_0C00 == 0x5E21_0800 {
        let double = (word >> 22) & 1 != 0;
        let key = (((word >> 23) & 1) << 4) | ((word >> 12) & 0xF);
        let round = match key {
            0b1_1011 => FpRound::Zero,
            0b0_1010 => FpRound::NearestEven,
            0b1_1010 => FpRound::PlusInf,
            0b0_1011 => FpRound::MinusInf,
            0b0_1100 => FpRound::NearestAway,
            _ => return Op::Unsupported { word },
        };
        return Op::FpRoundFp {
            rd: (word & 0x1F) as u8,
            rn: ((word >> 5) & 0x1F) as u8,
            round,
            is64: double,
        };
    }

    // fmov with an encoded immediate: bits 12:5 are 100 00000 and bits 20:13
    // hold the eight bit immediate. (Bits 15:13 are the low immediate bits,
    // so they must not be part of the match.)
    if word & 0xFFA0_1FE0 == 0x1E20_1000 {
        return Op::FpMovImm {
            rd: (word & 0x1F) as u8,
            imm8: ((word >> 13) & 0xFF) as u8,
            is64: (word >> 22) & 1 != 0,
        };
    }

    Op::Unsupported { word }
}

/// Decode an AArch64 bitmask immediate (`DecodeBitMasks`).
fn decode_bitmask(n: u32, immr: u32, imms: u32, is64: bool) -> Option<u64> {
    // len = index of the highest set bit of N:NOT(imms) within six bits.
    let combined = (n << 6) | ((!imms) & 0x3F);
    let len = 31i32 - combined.leading_zeros() as i32;
    if len < 1 || (!is64 && len > 5) {
        return None;
    }
    let levels = (1u32 << len) - 1;
    if (imms & levels) == levels {
        return None;
    }
    let s = imms & levels;
    let r = immr & levels;
    let size = 1u32 << len;

    let ones = |k: u32| -> u64 {
        if k >= 64 {
            u64::MAX
        } else {
            (1u64 << k) - 1
        }
    };
    // Element of `size` bits with the low s+1 bits set, rotated right by r.
    let welem = ones(s + 1) & ones(size);
    let rot = if r == 0 {
        welem
    } else {
        ((welem >> r) | (welem << (size - r))) & ones(size)
    };

    let mut result = 0u64;
    let mut shift = 0u32;
    while shift < 64 {
        result |= rot << shift;
        shift += size;
    }
    Some(if is64 { result } else { result & 0xFFFF_FFFF })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aarch64::{FpBinop, FpRound, FpUnop};

    #[test]
    fn decodes_conditional_select_kinds() {
        // csel w0, w9, w8, eq / csinc / csinv / csneg, and the 64-bit csinv.
        for (word, kind, is64) in [(0x1a880120u32, 0u8, false), (0x1a880520, 1, false), (0x5a880120, 2, false), (0x5a880520, 3, false), (0xda880120, 2, true)] {
            match decode(word, 0) {
                Op::CondSelect { rd: 0, rn: 9, rm: 8, cond: Cond::Eq, kind: k, is64: w } => {
                    assert_eq!((k, w), (kind, is64), "word {word:08x}");
                }
                other => panic!("word {word:08x} decoded as {other:?}"),
            }
        }
    }

    #[test]
    fn decodes_nop_and_ret() {
        assert_eq!(decode(0xD503201F, 0), Op::Nop);
        // fmov s0, #14.0 and fmov d1, #12.0 are immediates, not register moves.
        assert_eq!(decode(0x1E26_D000, 0), Op::FpMovImm { rd: 0, imm8: 0x36, is64: false });
        assert_eq!(decode(0x1E66_5001, 0), Op::FpMovImm { rd: 1, imm8: 0x32, is64: true });
        assert_eq!(decode(0x1E26_0000, 0), Op::FpMoveGp { fp: 0, gp: 0, to_fp: false, is64: false });
        // dsb sy, dmb ish, isb, clrex
        for word in [0xD503_3F9F, 0xD503_3BBF, 0xD503_3FDF, 0xD503_3F5F] {
            assert_eq!(decode(word, 0), Op::Nop, "{word:08x}");
        }
        // dc civac x11, dc cvac x0, ic ivau x0
        assert_eq!(decode(0xD50B_7E2B, 0), Op::Nop);
        assert_eq!(decode(0xD50B_7A20, 0), Op::Nop);
        assert_eq!(decode(0xD50B_7520, 0), Op::Nop);
        assert_eq!(decode(0xD53B_D060, 0), Op::Mrs { rt: 0, reg: sysreg::TPIDRRO_EL0 });
        assert_eq!(decode(0xD53B_E020, 0), Op::Mrs { rt: 0, reg: sysreg::CNTPCT_EL0 });
        assert_eq!(decode(0xD51B_4400, 0), Op::Msr { rt: 0, reg: sysreg::FPCR });
        assert_eq!(decode(0xD400_0B01, 0), Op::Svc { imm: 0x58 });
        // scvtf s0, w0, #31
        assert_eq!(
            decode(0x1E02_8400, 0),
            Op::IntToFpFixed { rd: 0, rn: 0, unsigned: false, src_is64: false, dest_is64: false, fbits: 31 }
        );
        // ldpsw x20, x11, [x19, #20]
        assert_eq!(
            decode(0x6942_AE74, 0),
            Op::LoadPairSigned { rt: 20, rt2: 11, rn: 19, offset: 20, mode: MemMode::Offset }
        );
        assert_eq!(decode(0xD65F03C0, 0), Op::Ret { rn: 30 });
    }

    #[test]
    fn decodes_branches() {
        // bl +0x40 at 0x1000
        assert_eq!(decode(0x9400_0010, 0x1000), Op::Bl { target: 0x1040 });
        // b -0x8 at 0x1000
        assert_eq!(decode(0x17FF_FFFE, 0x1000), Op::B { target: 0xFF8 });
        assert_eq!(decode(0xD61F0200, 0), Op::Br { rn: 16 });
        assert_eq!(decode(0xD63F0100, 0), Op::Blr { rn: 8 });
    }

    #[test]
    fn decodes_prologue_pair() {
        // stp x29, x30, [sp, #-16]!
        let op = decode(0xA9BF_7BFD, 0);
        assert_eq!(
            op,
            Op::LoadStorePair {
                rt: 29,
                rt2: 30,
                rn: 31,
                offset: -16,
                load: false,
                mode: MemMode::PreIndex,
                is64: true,
            }
        );
        // ldp x29, x30, [sp], #16
        assert_eq!(
            decode(0xA8C1_7BFD, 0),
            Op::LoadStorePair {
                rt: 29,
                rt2: 30,
                rn: 31,
                offset: 16,
                load: true,
                mode: MemMode::PostIndex,
                is64: true,
            }
        );
    }

    #[test]
    fn decodes_move_wide() {
        assert_eq!(
            decode(0xD280_0020, 0),
            Op::MoveWide { rd: 0, imm: 1, shift: 0, kind: 0, is64: true }
        );
    }

    #[test]
    fn decodes_add_sub() {
        // add x29, sp, #0  (mov x29, sp)
        assert_eq!(
            decode(0x9100_03FD, 0),
            Op::AddSubImm { rd: 29, rn: 31, imm: 0, sub: false, set_flags: false, is64: true }
        );
        // sub sp, sp, #0x10  (canonical prologue stack adjustment)
        assert_eq!(
            decode(0xD100_43FF, 0),
            Op::AddSubImm { rd: 31, rn: 31, imm: 0x10, sub: true, set_flags: false, is64: true }
        );
        // add w0, w1, w2
        assert_eq!(
            decode(0x0B02_0020, 0),
            Op::AddSubReg {
                rd: 0,
                rn: 1,
                rm: 2,
                shift: Shift::Lsl,
                amount: 0,
                sub: false,
                set_flags: false,
                is64: false
            }
        );
        // cmp w0, #1  (subs wzr, w0, #1)
        assert_eq!(
            decode(0x7100_041F, 0),
            Op::AddSubImm { rd: 31, rn: 0, imm: 1, sub: true, set_flags: true, is64: false }
        );
    }

    #[test]
    fn decodes_load_store() {
        // ldr x0, [x1]
        assert_eq!(
            decode(0xF940_0020, 0),
            Op::LoadStore {
                rt: 0,
                rn: 1,
                offset: 0,
                bytes: 8,
                load: true,
                signed: false,
                mode: MemMode::Offset,
                is64: true
            }
        );
        // str w0, [x1, #4]
        assert_eq!(
            decode(0xB900_0420, 0),
            Op::LoadStore {
                rt: 0,
                rn: 1,
                offset: 4,
                bytes: 4,
                load: false,
                signed: false,
                mode: MemMode::Offset,
                is64: false
            }
        );
    }

    #[test]
    fn decodes_conditional_and_compare_branch() {
        assert_eq!(decode(0x5400_0040, 0x100), Op::BCond { cond: Cond::Eq, target: 0x108 });
        assert_eq!(
            decode(0xB400_0040, 0x100),
            Op::CompareBranchZero { rt: 0, target: 0x108, nonzero: false, is64: true }
        );
    }

    #[test]
    fn decodes_adrp_and_mov_reg() {
        // adrp x0, #0x1000
        assert_eq!(decode(0x9000_0000, 0), Op::Adr { rd: 0, target: 0 });
        // mov x0, x1  (orr x0, xzr, x1)
        assert_eq!(
            decode(0xAA01_03E0, 0),
            Op::LogicalReg {
                rd: 0,
                rn: 31,
                rm: 1,
                shift: Shift::Lsl,
                amount: 0,
                op: LogicalOp::Orr,
                set_flags: false,
                is64: true,
                invert: false,
            }
        );
        // mvn w8, w8 (orn w8, wzr, w8): bit 21 complements the operand. The
        // oracle found this at 0xc8e34, where the bit had been ignored.
        assert!(matches!(
            decode(0x2A28_03E8, 0),
            Op::LogicalReg { rd: 8, rn: 31, rm: 8, op: LogicalOp::Orr, invert: true, is64: false, .. }
        ));
    }

    #[test]
    fn inverted_logical_ops_and_their_flags_execute_architecturally() {
        let mut s = crate::interpreter::State::default();
        s.x[8] = 9;
        s.step(&decode(0x2A28_03E8, 0)).unwrap(); // mvn w8, w8
        assert_eq!(s.x[8], 0xFFFF_FFF6);
        // bics x0, x1, x2 with x1 = x2: result 0, Z set, C and V cleared.
        s.nzcv = 0b0011;
        s.x[1] = 0xF0;
        s.x[2] = 0xF0;
        s.step(&decode(0xEA22_0020, 0)).unwrap();
        assert_eq!(s.x[0], 0);
        assert_eq!(s.nzcv, 0b0100);
    }

    /// The encodings below were produced by clang for AArch64 and captured in
    /// `build/asm_check.s`, so they are external ground truth rather than a
    /// restatement of this decoder.
    #[test]
    fn decodes_register_offset_load_store() {
        assert_eq!(
            decode(0x3868_492A, 0),
            Op::LoadStoreReg {
                rt: 10,
                rn: 9,
                rm: 8,
                option: Extend::Uxtw,
                shift: 0,
                bytes: 1,
                load: true,
                signed: false,
                is64: false
            }
        );
        // ldr w10, [x9, w8, uxtw #2]
        assert_eq!(
            decode(0xB868_592A, 0),
            Op::LoadStoreReg {
                rt: 10,
                rn: 9,
                rm: 8,
                option: Extend::Uxtw,
                shift: 2,
                bytes: 4,
                load: true,
                signed: false,
                is64: false
            }
        );
        // ldr x8, [x9, w8, sxtw #3]
        assert_eq!(
            decode(0xF868_D928, 0),
            Op::LoadStoreReg {
                rt: 8,
                rn: 9,
                rm: 8,
                option: Extend::Sxtw,
                shift: 3,
                bytes: 8,
                load: true,
                signed: false,
                is64: true
            }
        );
        // strb w0, [x1, w2, uxtw]
        assert_eq!(
            decode(0x3822_4820, 0),
            Op::LoadStoreReg {
                rt: 0,
                rn: 1,
                rm: 2,
                option: Extend::Uxtw,
                shift: 0,
                bytes: 1,
                load: false,
                signed: false,
                is64: false
            }
        );
    }

    /// Every encoding in this test was produced by the AArch64 assembler and
    /// cross-checked with an independent disassembler, so a wrong bit layout
    /// here fails the test rather than silently mis-lifting the guest.
    #[test]
    fn decodes_conditional_compare() {
        assert_eq!(
            decode(0xFA40_1104, 0),
            Op::CondCompare {
                rn: 8,
                rm: 0,
                imm5: 0,
                immediate: false,
                cond: Cond::Ne,
                nzcv: 4,
                sub: true,
                is64: true,
            }
        );
        assert_eq!(
            decode(0xFA43_1904, 0),
            Op::CondCompare {
                rn: 8,
                rm: 0,
                imm5: 3,
                immediate: true,
                cond: Cond::Ne,
                nzcv: 4,
                sub: true,
                is64: true,
            }
        );
        assert_eq!(
            decode(0x3A43_C841, 0),
            Op::CondCompare {
                rn: 2,
                rm: 0,
                imm5: 3,
                immediate: true,
                cond: Cond::Gt,
                nzcv: 1,
                sub: false,
                is64: false,
            }
        );
    }

    #[test]
    fn decodes_bit_scan_family() {
        assert_eq!(
            decode(0xDAC0_1129, 0),
            Op::BitScan { rd: 9, rn: 9, op: BitScan::Clz, is64: true }
        );
        assert_eq!(
            decode(0x5AC0_0021, 0),
            Op::BitScan { rd: 1, rn: 1, op: BitScan::Rbit, is64: false }
        );
        assert_eq!(
            decode(0xDAC0_0529, 0),
            Op::BitScan { rd: 9, rn: 9, op: BitScan::Rev16, is64: true }
        );
        assert_eq!(
            decode(0xDAC0_0D29, 0),
            Op::BitScan { rd: 9, rn: 9, op: BitScan::Rev, is64: true }
        );
    }

    #[test]
    fn decodes_extr_mulh_and_the_exclusive_family() {
        assert_eq!(decode(0x1389_0529, 0), Op::Extr { rd: 9, rn: 9, rm: 9, lsb: 1, is64: false });
        assert_eq!(decode(0x9BCA_7D2A, 0), Op::MulHigh { rd: 10, rn: 9, rm: 10, unsigned: true });
        assert_eq!(decode(0x9B48_7C08, 0), Op::MulHigh { rd: 8, rn: 0, rm: 8, unsigned: false });
        // stlrb w22, [x8] (size 00 is the byte form)
        assert!(matches!(decode(0x089F_FD16, 0), Op::LoadStore { rt: 22, rn: 8, offset: 0, bytes: 1, load: false, .. }));
        // ldaxr x8, [x0] and stlxr w9, x10, [x0]
        assert!(matches!(decode(0xC85F_FC08, 0), Op::Exclusive { rt: 8, rn: 0, bytes: 8, load: true, pair: false, .. }));
        assert!(matches!(decode(0xC809_FC0A, 0), Op::Exclusive { rs: 9, rt: 10, rn: 0, bytes: 8, load: false, .. }));
        // casal w1, w2, [x0]
        assert!(matches!(decode(0x88E1_FC02, 0), Op::Cas { rs: 1, rt: 2, rn: 0, bytes: 4, pair: false }));
    }

    #[test]
    fn decodes_lse_atomics() {
        assert_eq!(
            decode(0xF829_0108, 0),
            Op::AtomicRmw {
                rs: 9,
                rt: 8,
                rn: 8,
                op: AtomicOp::Add,
                order: AtomicOrder { acquire: false, release: false },
                bytes: 8,
            }
        );
        // ldaddal w9, w8, [x8]: acquire and release, four byte access
        assert_eq!(
            decode(0xB8E9_0108, 0),
            Op::AtomicRmw {
                rs: 9,
                rt: 8,
                rn: 8,
                op: AtomicOp::Add,
                order: AtomicOrder { acquire: true, release: true },
                bytes: 4,
            }
        );
        // swp x9, x8, [x8]
        assert_eq!(
            decode(0xF829_8108, 0),
            Op::AtomicRmw {
                rs: 9,
                rt: 8,
                rn: 8,
                op: AtomicOp::Swap,
                order: AtomicOrder { acquire: false, release: false },
                bytes: 8,
            }
        );
    }

    #[test]
    fn decodes_vector_memory_and_moves() {
        assert_eq!(
            decode(0x3DC0_0100, 0),
            Op::VectorLoadStore {
                rt: 0,
                rn: 8,
                offset: 0,
                bytes: 16,
                load: true,
                mode: MemMode::Offset,
            }
        );
        assert_eq!(
            decode(0xFD40_0500, 0),
            Op::VectorLoadStore {
                rt: 0,
                rn: 8,
                offset: 8,
                bytes: 8,
                load: true,
                mode: MemMode::Offset,
            }
        );
        assert_eq!(
            decode(0x3CE9_6900, 0),
            Op::VectorLoadStoreReg {
                rt: 0,
                rn: 8,
                rm: 9,
                option: Extend::Uxtx,
                shift: 0,
                bytes: 16,
                load: true,
            }
        );
        assert_eq!(
            decode(0xFC24_7862, 0),
            Op::VectorLoadStoreReg {
                rt: 2,
                rn: 3,
                rm: 4,
                option: Extend::Uxtx,
                shift: 3,
                bytes: 8,
                load: false,
            }
        );
        assert_eq!(
            decode(0x6D41_0901, 0),
            Op::VectorLoadStorePair {
                rt: 1,
                rt2: 2,
                rn: 8,
                offset: 16,
                bytes: 8,
                load: true,
                mode: MemMode::Offset,
            }
        );
        assert_eq!(decode(0x2F00_E400, 0), Op::VectorZero { rd: 0, full: false });
        assert_eq!(decode(0x6F00_E400, 0), Op::VectorZero { rd: 0, full: true });
        assert_eq!(
            decode(0x1E27_0101, 0),
            Op::FpMoveGp { fp: 1, gp: 8, to_fp: true, is64: false }
        );
        assert_eq!(
            decode(0x9E66_00A2, 0),
            Op::FpMoveGp { fp: 5, gp: 2, to_fp: false, is64: true }
        );
        assert_eq!(decode(0x1E60_4083, 0), Op::FpCopy { rd: 3, rn: 4, is64: true });
    }

    #[test]
    fn decodes_sign_extending_loads() {
        // ldrsw x11, [x11, x10, lsl #2]
        assert_eq!(
            decode(0xB8AA_796B, 0),
            Op::LoadStoreReg {
                rt: 11,
                rn: 11,
                rm: 10,
                option: Extend::Uxtx,
                shift: 2,
                bytes: 4,
                load: true,
                signed: true,
                is64: true,
            }
        );
        // ldrsh x9, [x1, x18, lsl #1]
        assert_eq!(
            decode(0x78B2_7829, 0),
            Op::LoadStoreReg {
                rt: 9,
                rn: 1,
                rm: 18,
                option: Extend::Uxtx,
                shift: 1,
                bytes: 2,
                load: true,
                signed: true,
                is64: true,
            }
        );
    }

    #[test]
    fn decodes_multiply_accumulate() {
        // msub x11, x12, x9, x11
        assert_eq!(
            decode(0x9B09_AD8B, 0),
            Op::MultiplyAdd { rd: 11, rn: 12, rm: 9, ra: 11, sub: true, is64: true }
        );
    }

    #[test]
    fn decodes_extended_register_add() {
        // add x11, x11, w10, sxtw #2
        assert_eq!(
            decode(0x8B2A_C96B, 0),
            Op::AddSubExt {
                rd: 11,
                rn: 11,
                rm: 10,
                option: Extend::Sxtw,
                shift: 2,
                sub: false,
                set_flags: false,
                is64: true
            }
        );
        // sub x11, x11, w10, uxtw
        assert_eq!(
            decode(0xCB2A_416B, 0),
            Op::AddSubExt {
                rd: 11,
                rn: 11,
                rm: 10,
                option: Extend::Uxtw,
                shift: 0,
                sub: true,
                set_flags: false,
                is64: true
            }
        );
        // add w0, w1, w2, uxtw
        assert_eq!(
            decode(0x0B22_4020, 0),
            Op::AddSubExt {
                rd: 0,
                rn: 1,
                rm: 2,
                option: Extend::Uxtw,
                shift: 0,
                sub: false,
                set_flags: false,
                is64: false
            }
        );
    }

    #[test]
    fn decodes_bitfield_family() {
        // sbfx w0, w1, #3, #5
        assert_eq!(
            decode(0x1303_1C20, 0),
            Op::Bitfield { rd: 0, rn: 1, immr: 3, imms: 7, opc: 0, is64: false }
        );
        // ubfx x0, x1, #3, #5
        assert_eq!(
            decode(0xD343_1C20, 0),
            Op::Bitfield { rd: 0, rn: 1, immr: 3, imms: 7, opc: 2, is64: true }
        );
        // ubfiz w0, w1, #3, #5
        assert_eq!(
            decode(0x531D_1020, 0),
            Op::Bitfield { rd: 0, rn: 1, immr: 29, imms: 4, opc: 2, is64: false }
        );
        // sbfiz x0, x1, #3, #5
        assert_eq!(
            decode(0x937D_1020, 0),
            Op::Bitfield { rd: 0, rn: 1, immr: 61, imms: 4, opc: 0, is64: true }
        );
        // bfi w0, w1, #3, #5
        assert_eq!(
            decode(0x331D_1020, 0),
            Op::Bitfield { rd: 0, rn: 1, immr: 29, imms: 4, opc: 1, is64: false }
        );
        // lsl w0, w1, #4 is a ubfm
        assert_eq!(
            decode(0x531C_6C20, 0),
            Op::Bitfield { rd: 0, rn: 1, immr: 28, imms: 27, opc: 2, is64: false }
        );
    }

    #[test]
    fn sign_extending_word_loads_are_loads_not_prefetches() {
        // ldrsb w8, [x5]
        assert!(matches!(
            decode(0x39C0_00A8, 0),
            Op::LoadStore { rt: 8, rn: 5, bytes: 1, load: true, signed: true, is64: false, .. }
        ));
        // ldrsh w0, [x1, #2]
        assert!(matches!(
            decode(0x79C0_0420, 0),
            Op::LoadStore { rt: 0, rn: 1, bytes: 2, load: true, signed: true, is64: false, .. }
        ));
        // prfm pldl1keep, [x0] and prfum pldl1keep, [x0, #-8] are still nops.
        assert_eq!(decode(0xF980_0000, 0), Op::Nop);
        assert_eq!(decode(0xF89F_8000, 0), Op::Nop);
    }

    #[test]
    fn decodes_authenticated_return_and_prefetch() {
        // retab, the epilogue form present 36,342 times in the retail game module
        assert_eq!(decode(0xD65F_0FFF, 0), Op::Ret { rn: 30 });
        assert_eq!(decode(0xD65F_0BFF, 0), Op::Ret { rn: 30 });
        // ldaddal x9, x8, [x8]: this word was previously treated as a prefetch
        // and turned into a nop, which would have dropped a real atomic update.
        assert_eq!(
            decode(0xF8E9_0108, 0),
            Op::AtomicRmw {
                rs: 9,
                rt: 8,
                rn: 8,
                op: AtomicOp::Add,
                order: AtomicOrder { acquire: true, release: true },
                bytes: 8,
            }
        );
    }

    #[test]
    fn decodes_crc32_widths() {
        // crc32b w8, w8, w11: the word the boot run stopped on at 0x560.
        assert_eq!(
            decode(0x1ACB_4108, 0),
            Op::Crc32 { rd: 8, rn: 8, rm: 11, bytes: 1, castagnoli: false }
        );
        // Size is bits 11:10: b, h, w, and x (which sets sf and reads x11).
        assert_eq!(
            decode(0x1ACB_4508, 0),
            Op::Crc32 { rd: 8, rn: 8, rm: 11, bytes: 2, castagnoli: false }
        );
        assert_eq!(
            decode(0x1ACB_4908, 0),
            Op::Crc32 { rd: 8, rn: 8, rm: 11, bytes: 4, castagnoli: false }
        );
        assert_eq!(
            decode(0x9ACB_4D08, 0),
            Op::Crc32 { rd: 8, rn: 8, rm: 11, bytes: 8, castagnoli: false }
        );
        // Bit 12 selects the Castagnoli polynomial.
        assert_eq!(
            decode(0x1ACB_5108, 0),
            Op::Crc32 { rd: 8, rn: 8, rm: 11, bytes: 1, castagnoli: true }
        );
    }

    #[test]
    fn decodes_pointer_authentication_hints_as_nop() {
        for word in [0xD503_237F, 0xD503_23FF, 0xD503_233F, 0xD503_23BF, 0xD503_245F] {
            assert_eq!(decode(word, 0), Op::Nop, "0x{word:08x} is a hint");
        }
    }

    #[test]
    fn decodes_two_source_and_long_multiply() {
        assert_eq!(
            decode(0x9AC2_0820, 0),
            Op::DataProc2 { rd: 0, rn: 1, rm: 2, op: DataOp2::Udiv, is64: true }
        );
        assert_eq!(
            decode(0x1AC2_2820, 0),
            Op::DataProc2 { rd: 0, rn: 1, rm: 2, op: DataOp2::Asrv, is64: false }
        );
        assert_eq!(
            decode(0x9B22_0C20, 0),
            Op::MulLong { rd: 0, rn: 1, rm: 2, ra: 3, sub: false, unsigned: false }
        );
        assert_eq!(
            decode(0x9BA2_0C20, 0),
            Op::MulLong { rd: 0, rn: 1, rm: 2, ra: 3, sub: false, unsigned: true }
        );
        assert_eq!(
            decode(0x9B22_8C20, 0),
            Op::MulLong { rd: 0, rn: 1, rm: 2, ra: 3, sub: true, unsigned: false }
        );
        // smull x0, w1, w2 is smaddl with Ra = XZR
        assert_eq!(
            decode(0x9B22_7C20, 0),
            Op::MulLong { rd: 0, rn: 1, rm: 2, ra: 31, sub: false, unsigned: false }
        );
    }

    #[test]
    fn unknown_words_are_marked_unsupported() {
        match decode(0x0000_0000, 0) {
            Op::Unsupported { .. } => {}
            other => panic!("expected unsupported, got {other:?}"),
        }
    }

    /// Every word in this test was produced by clang for AArch64 and then
    /// confirmed with an independent disassembler, so a failure here means the
    /// decoder disagrees with real toolchains, not with itself.
    #[test]
    fn decodes_scalar_floating_point_arithmetic() {
        assert_eq!(
            decode(0x1E22_2820, 0),
            Op::FpBinary { rd: 0, rn: 1, rm: 2, op: FpBinop::Add, is64: false }
        );
        assert_eq!(
            decode(0x1E62_0820, 0),
            Op::FpBinary { rd: 0, rn: 1, rm: 2, op: FpBinop::Mul, is64: true }
        );
        assert_eq!(
            decode(0x1E62_8820, 0),
            Op::FpBinary { rd: 0, rn: 1, rm: 2, op: FpBinop::NegMul, is64: true }
        );
        assert_eq!(
            decode(0x1E20_C020, 0),
            Op::FpUnary { rd: 0, rn: 1, op: FpUnop::Abs, is64: false }
        );
        assert_eq!(
            decode(0x1E61_4020, 0),
            Op::FpUnary { rd: 0, rn: 1, op: FpUnop::Neg, is64: true }
        );
        // fcvt s0, d1 narrows, so the source width is double.
        assert_eq!(
            decode(0x1E62_4020, 0),
            Op::FpUnary { rd: 0, rn: 1, op: FpUnop::Cvt, is64: true }
        );
        assert_eq!(
            decode(0x1F22_8C20, 0),
            Op::FpMulAdd {
                rd: 0,
                rn: 1,
                rm: 2,
                ra: 3,
                negate_product: true,
                negate_addend: true,
                is64: false
            }
        );
    }

    #[test]
    fn decodes_scalar_floating_point_conversions() {
        assert_eq!(
            decode(0x9E62_0020, 0),
            Op::IntToFp { rd: 0, rn: 1, unsigned: false, src_is64: true, dest_is64: true }
        );
        assert_eq!(
            decode(0x1E23_0020, 0),
            Op::IntToFp { rd: 0, rn: 1, unsigned: true, src_is64: false, dest_is64: false }
        );
        assert_eq!(
            decode(0x9E78_0020, 0),
            Op::FpToInt {
                rd: 0,
                rn: 1,
                round: FpRound::Zero,
                unsigned: false,
                src_is64: true,
                dest_is64: true
            }
        );
        assert_eq!(
            decode(0x1E25_0020, 0),
            Op::FpToInt {
                rd: 0,
                rn: 1,
                round: FpRound::NearestAway,
                unsigned: true,
                src_is64: false,
                dest_is64: false
            }
        );
    }

    #[test]
    fn decodes_scalar_floating_point_compares_and_selects() {
        assert_eq!(
            decode(0x1E21_2000, 0),
            Op::FpCompare { rn: 0, rm: 1, is64: false, signal: false }
        );
        assert_eq!(
            decode(0x1E21_2010, 0),
            Op::FpCompare { rn: 0, rm: 1, is64: false, signal: true }
        );
        assert_eq!(decode(0x1E20_2008, 0), Op::FpCompareZero { rn: 0, is64: false, signal: false });
        assert_eq!(
            decode(0x1E21_1400, 0),
            Op::FpCondCompare { rn: 0, rm: 1, cond: Cond::Ne, nzcv: 0, is64: false }
        );
        assert_eq!(
            decode(0x1E22_0C20, 0),
            Op::FpCondSelect { rd: 0, rn: 1, rm: 2, cond: Cond::Eq, is64: false }
        );
        assert_eq!(decode(0x1E2E_1000, 0), Op::FpMovImm { rd: 0, imm8: 0x70, is64: false });
    }

    #[test]
    fn bitmask_immediate_matches_known_values() {
        // and w0, w0, #0xff  -> immr=0, imms=7, n=0
        match decode(0x1200_1C00, 0) {
            Op::LogicalImm { imm, is64, .. } => {
                assert!(!is64);
                assert_eq!(imm & 0xFFFF_FFFF, 0xFF);
            }
            other => panic!("expected logical immediate, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod ldapr_tests {
    use super::*;

    /// `ldaprb w8, [x8]`, the instruction that stopped the boot attempt.
    #[test]
    fn ldaprb_decodes_as_a_plain_byte_load() {
        match decode(0x38BF_C108, 0) {
            Op::LoadStore { rt, rn, offset, bytes, load, signed, mode, is64 } => {
                assert_eq!((rt, rn, offset), (8, 8, 0));
                assert_eq!((bytes, load, signed), (1, true, false));
                assert_eq!(mode, MemMode::Offset);
                assert!(!is64);
            }
            other => panic!("expected a byte load, got {other:?}"),
        }
    }

    /// `ldaprb` is one byte; the word and doubleword forms follow the size
    /// field, not a fixed width.
    #[test]
    fn ldapr_sizes_follow_the_size_field() {
        match decode(0xF8BF_C020, 0) {
            Op::LoadStore { bytes, is64, .. } => {
                assert_eq!(bytes, 8);
                assert!(is64);
            }
            other => panic!("expected a doubleword load, got {other:?}"),
        }
    }
}
