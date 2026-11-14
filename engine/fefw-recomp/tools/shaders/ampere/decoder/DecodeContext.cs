// SPDX-License-Identifier: MIT
// Bit access and operand builders shared by the opcode handlers. Field positions follow Mesa NAK sm70_encode.rs
// (MIT, Collabora): predicate guard 12..16, dst 16..24, src A 24..32, src B 32..40 / imm 32..64 / cbuf 32..59, src C 64..72,
// ALU form 9..12, scheduling 105..126 (stall 105..109, yield 109, write barrier 110..113, read barrier 113..116,
// wait mask 116..122, reuse 122..126).
using System;
using System.Collections.Generic;

namespace Nx.Shaders.Ampere
{
    /// Imm spelling used by an ALU handler for its 32-bit immediate slot.
    internal enum ImmKind { Hex, SHex, F32, F16x2, None }

    internal sealed class Ctx
    {
        public readonly ulong Lo, Hi;
        public readonly int Addr;
        public string Op = "";
        public readonly List<string> M = new(4);
        public readonly List<Operand> O = new(6);
        public int Dst;
        public bool UGuard;
        public bool Unknown;
        /// uniform datapath op: register sources/destinations are URs, predicates UPs, the guard is a UP.
        public bool U;

        public Ctx(ulong lo, ulong hi, int addr) { Lo = lo; Hi = hi; Addr = addr; }

        public ulong F(int start, int len)
        {
            ulong v;
            if (start >= 64) v = Hi >> (start - 64);
            else if (start + len <= 64) v = Lo >> start;
            else v = (Lo >> start) | (Hi << (64 - start));
            return len >= 64 ? v : v & ((1UL << len) - 1);
        }
        public int I(int start, int len) => (int)F(start, len);
        public bool B(int bit) => F(bit, 1) != 0;
        public long S(int start, int len) { long v = (long)F(start, len); long sign = 1L << (len - 1); return (v ^ sign) - sign; }
        public int Form => I(9, 3);
        public int Op9 => I(0, 9);

        public void Mod(string m) { M.Add(m); }
        public void Mod(bool cond, string m) { if (cond) M.Add(m); }
        /// modifier from a table indexed by a field; null/empty entries print nothing.
        public void Mod(int start, int len, params string?[] table)
        {
            int v = I(start, len);
            if (v < table.Length) { if (!string.IsNullOrEmpty(table[v])) M.Add(table[v]!); }
            else { M.Add("?" + v); Unknown = true; }
        }

        // ---- operands ------------------------------------------------------------------------------------------------

        static readonly string[] Reuse = { ".reuse" };

        static int ReuseSlot(int start) => start switch { 24 => 0, 32 => 1, 64 => 2, _ => -1 };
        /// .reuse follows the operand's printed slot (A 122, B 123, C 124), not its bit position [oracle]; never printed on RZ.
        bool Reused(int start, int slot) { int s = slot == -2 ? ReuseSlot(start) : slot; return s >= 0 && B(122 + s) && I(start, 8) != 255; }
        /// reuse slot of the second source of a two-source op (FADD and HADD2 use C) [oracle]
        public int Src2Slot = 1;

        static IReadOnlyList<string>? Sfx(bool reuse, string? extra)
        {
            if (!reuse && extra == null) return null;
            if (extra == null) return Reuse;
            return reuse ? new[] { ".reuse", extra } : new[] { extra };
        }

        public Operand R(int start, bool neg = false, bool abs = false, bool not = false, string? sfx = null, bool reuse = true, int slot = -2)
            => new(OperandKind.Reg, I(start, 8), Neg: neg, Abs: abs, Not: not, Suffixes: Sfx(reuse && Reused(start, slot), sfx));
        public Operand UR(int start, bool neg = false, bool abs = false, bool not = false, string? sfx = null)
            => new(OperandKind.UReg, I(start, 6), Neg: neg, Abs: abs, Not: not, Suffixes: Sfx(false, sfx));
        public Operand RegN(int r) => new(OperandKind.Reg, r);
        public Operand P(int start, int notBit = -1) => new(OperandKind.Pred, I(start, 3), Not: notBit >= 0 && B(notBit));
        public Operand UP(int start, int notBit = -1) => new(OperandKind.UPred, I(start, 3), Not: notBit >= 0 && B(notBit));
        public Operand Imm(long v, string text) => new(OperandKind.Imm, ImmBits: (ulong)v, Offset: v, Text: text);
        public Operand Hex(int start, int len) { long v = (long)F(start, len); return Imm(v, SassText.Hex(v)); }
        public Operand SHex(int start, int len) { long v = S(start, len); return Imm(v, SassText.Hex(v)); }
        public Operand Text(string t, OperandKind k = OperandKind.Other) => new(k, Text: t);
        public Operand Label(long target) => new(OperandKind.Label, Offset: target);

        /// c[bank][offset] in the 32..59 slot; bit 91 set = bank is a UReg at 32..38 (cx[URn]).
        public Operand CBuf(int start = 32, bool neg = false, bool abs = false, int cxBit = 91, string? sfx = null, bool not = false)
        {
            long off = (long)F(start + 6, 16);
            if (cxBit >= 0 && B(cxBit))
                return new Operand(OperandKind.CBuf, Neg: neg, Abs: abs, Not: not, Bank: I(start, 6), BankIsUReg: true, Offset: off, Suffixes: Sfx(false, sfx));
            return new Operand(OperandKind.CBuf, Neg: neg, Abs: abs, Not: not, Bank: I(start + 22, 5), Offset: off, Suffixes: Sfx(false, sfx));
        }

        public Operand Imm32(ImmKind k, int start = 32)
        {
            uint v = (uint)F(start, 32);
            switch (k)
            {
                case ImmKind.F32: return new Operand(OperandKind.FImm, ImmBits: v, Text: SassText.F32(v));
                case ImmKind.F16x2: return new Operand(OperandKind.FImm, ImmBits: v, Text: SassText.F16(v >> 16) + ", " + SassText.F16(v)); // high half first [oracle]
                case ImmKind.SHex: return Imm((int)v, SassText.Hex((int)v));
                default: return Imm(v, SassText.Hex(v));
            }
        }

        // ---- generic ALU sources ------------------------------------------------------------------------------------------
        // Volta+ ALU forms (bits 9..11): 1 R,R,R  2 R,R(64),imm  3 R,R(64),c  4 R,imm,R  5 R,c,R  6 R,UR,R  7 R,R(64),UR.
        // In forms 2/3/7 the 32..64 slot is the third source and the register at 64 the second.

        /// abs/neg bit positions: src A 73/72, B slot 62/63, C slot 74/75 (fp16 ops: 83/84). -1 = not used by this op.
        public int AAbs = 73, ANeg = 72, BAbs = 62, BNeg = 63, CAbs = 74, CNeg = 75;
        /// the negate bits are a bitwise NOT (printed "~"): the carry-in (.X) forms of IADD3, IMAD and LEA [NAK OpIAdd3X/OpIMad .X, oracle]
        public bool NegIsNot;
        public ImmKind ImmK = ImmKind.Hex;
        public Func<Ctx, int, string?>? Swz; // fp16 swizzle suffix for a source slot (24, 32, 64)

        bool Bit(int b) => b >= 0 && B(b);
        bool Neg(int b) => !NegIsNot && Bit(b);
        bool Not(int b) => NegIsNot && Bit(b);

        public Operand SrcA() => U ? UR(24, Neg(ANeg), Bit(AAbs), Not(ANeg)) : R(24, Neg(ANeg), Bit(AAbs), Not(ANeg), sfx: Swz?.Invoke(this, 24));
        public Operand SrcRegC() => U ? UR(64, Neg(CNeg), Bit(CAbs), Not(CNeg)) : R(64, Neg(CNeg), Bit(CAbs), Not(CNeg), sfx: Swz?.Invoke(this, 64));
        /// destination register at 16 (UR for uniform ops)
        public Operand D() => U ? UR(16) : R(16);
        /// predicate (UP for uniform ops)
        public Operand Pr(int start, int notBit = -1) => U ? UP(start, notBit) : P(start, notBit);

        /// the 32..64 slot by form: R / imm / cbuf / UR.
        public Operand SrcBSlot()
        {
            switch (Form)
            {
                case 1: return U ? UR(32, Neg(BNeg), Bit(BAbs), Not(BNeg)) : R(32, Neg(BNeg), Bit(BAbs), Not(BNeg), sfx: Swz?.Invoke(this, 32), slot: Src2Slot);
                case 2: case 4: return Imm32(ImmK);
                case 3: case 5: return CBuf(32, Neg(BNeg), Bit(BAbs), sfx: Swz?.Invoke(this, 32), not: Not(BNeg));
                case 6: case 7: return UR(32, Neg(BNeg), Bit(BAbs), Not(BNeg), sfx: Swz?.Invoke(this, 32));
                default: Unknown = true; return Text("?form" + Form);
            }
        }

        /// sources 2 and 3 of a three-source ALU op in printed order.
        public (Operand b, Operand c) SrcBC()
        {
            if (Form == 2 || Form == 3 || Form == 7) { var c = SrcBSlot(); return (SrcRegCAsB(), c); }
            return (SrcBSlot(), SrcRegC());
        }

        // when the register at 64 is printed second its modifiers are the C ones still, its reuse bit is B's [oracle]
        Operand SrcRegCAsB() => U ? UR(64, Neg(CNeg), Bit(CAbs), Not(CNeg)) : R(64, Neg(CNeg), Bit(CAbs), Not(CNeg), sfx: Swz?.Invoke(this, 64), slot: 1);

        /// second source of a two-source ALU op (FMUL/FADD/...): the 32..64 slot for every form.
        public Operand Src2() => SrcBSlot();

        public void Add(Operand o) => O.Add(o);
        public void Add(params Operand[] os) => O.AddRange(os);
        public void Pred3(int start, int notBit) => O.Add(P(start, notBit));

        public SassInst Build()
        {
            int g = I(12, 3);
            bool gn = B(15);
            return new SassInst(Addr, Lo, Hi, g, gn, UGuard || U, Op, M.ToArray(), O.ToArray(), Dst, (uint)(Hi >> 41));
        }
    }
}
