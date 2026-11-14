// SPDX-License-Identifier: MIT
// Memory, attributes and constants. Opcodes, address/data slots, offset 40..64, mem type 73..76, order/scope 77..81,
// eviction 84..87, shared stride 78..80, LDC mode 78..80, ALD/AST/IPA/ISBERD fields: [NAK] OpLd/OpSt/OpAtom/OpLdc/OpALd/OpASt/
// OpIpa/OpIsberd. ULDC with a UR index (0xabb), the address spelling ([Rn.U32+URm+off], .X16 stride), order names
// (STRONG.SM/GPU/SYS), ATOMG predicate operand: [oracle].
using System.Collections.Generic;
using System.Text;

namespace Nx.Shaders.Ampere
{
    public static partial class SassDecoder
    {
        static readonly string?[] MemType = { "U8", "S8", "U16", "S16", null, "64", "128", "U.128" };
        static readonly string?[] Evict = { "EF", null, "EL", "LU", "EU", "NA", "?ev6", "?ev7" };

        /// [NAK] set_mem_order (SM80+): 77..81 combined
        static void MemOrder(Ctx c)
        {
            int v = c.I(77, 4);
            switch (v)
            {
                case 0: break;
                case 4: c.Mod("CONSTANT"); break;
                case 5: c.Mod("STRONG"); c.Mod("SM"); break;
                case 7: c.Mod("STRONG"); c.Mod("GPU"); break;
                case 0xa: c.Mod("STRONG"); c.Mod("SYS"); break;
                default: c.Mod("?ord" + v); c.Unknown = true; break;
            }
        }

        /// [base(.sfx)+URn+offset] memory operand.
        static Operand MemOp(Ctx c, int baseReg, string? baseSfx, int ureg, long offset, bool showUZ = false)
        {
            var parts = new List<string>(3);
            if (baseReg != 255) parts.Add(SassText.RegName(baseReg) + (baseSfx ?? ""));
            if (ureg >= 0 && (ureg != 63 || showUZ)) parts.Add(SassText.URegName(ureg));
            var sb = new StringBuilder("[");
            sb.Append(string.Join("+", parts));
            if (offset != 0 || parts.Count == 0)
            {
                if (parts.Count == 0 && offset == 0) sb.Append("RZ");
                else { if (parts.Count > 0) sb.Append('+'); sb.Append(SassText.Hex(offset)); } // "[UR5+-0x30]" [oracle]
            }
            sb.Append(']');
            return new Operand(OperandKind.Mem, Reg: baseReg, IndexReg: ureg, IndexIsUReg: ureg >= 0, Offset: offset, Text: sb.ToString(),
                Suffixes: null);
        }

        static Operand MemReuse(Ctx c, Operand m) => m; // memory operands never carry .reuse

        static void RegisterMem()
        {
            // LDC [NAK] 0xb82: dst, c[bank][Ra+off]; type 73..76, mode 78..80
            Op12(0xb82, c =>
            {
                c.Op = "LDC"; c.Mod(73, 3, MemType); c.Mod(78, 2, null, "IL", "IS", "ISL");
                int r = c.I(24, 8);
                long off = r != 255 ? c.S(38, 16) : (long)c.F(38, 16);
                c.Dst = 1;
                c.Add(c.R(16), new Operand(OperandKind.CBuf, Bank: c.I(54, 5), Offset: off, IndexReg: r != 255 ? r : -1));
            });
            // ULDC [NAK] 0xab9; 0xabb = with UR index at 24 [oracle]
            Op12(0xab9, c => { c.Op = "ULDC"; c.UGuard = true; c.Mod(73, 3, MemType); c.Dst = 1; c.Add(c.UR(16), new Operand(OperandKind.CBuf, Bank: c.I(54, 5), Offset: (long)c.F(38, 16))); });
            Op12(0xabb, c =>
            {
                c.Op = "ULDC"; c.UGuard = true; c.Mod(73, 3, MemType); c.Dst = 1;
                int ur = c.I(24, 6);
                c.Add(c.UR(16), new Operand(OperandKind.CBuf, Bank: c.I(54, 5), Offset: ur != 63 ? c.S(38, 16) : (long)c.F(38, 16), IndexReg: ur != 63 ? ur : -1, IndexIsUReg: true));
            });

            // LDG 0x381 / 0x981 (UR form): .E 72, 32-bit base when UR form and 90 clear (.U32)
            Op9(0x181, c =>
            {
                c.Op = "LDG"; c.Mod(c.B(72), "E"); c.Mod(73, 3, MemType); MemOrder(c); c.Mod(84, 3, Evict);
                bool u = c.B(91);
                c.Dst = 1; c.Add(c.R(16), MemOp(c, c.I(24, 8), u && !c.B(90) ? ".U32" : (!u && !c.B(72) ? null : null), u ? c.I(32, 6) : -1, c.S(40, 24)));
                if (!IsPT(c, 81)) { c.O.Insert(1, c.P(81)); c.Dst++; }
            });
            // STG 0x386 / 0x986: UR at 64
            Op9(0x186, c =>
            {
                c.Op = "STG"; c.Mod(c.B(72), "E"); c.Mod(73, 3, MemType); MemOrder(c); c.Mod(84, 3, Evict);
                bool u = c.B(91);
                c.Dst = 0; c.Add(MemOp(c, c.I(24, 8), u && !c.B(90) ? ".U32" : null, u ? c.I(64, 6) : -1, c.S(40, 24)), c.R(32));
            });
            // LDL 0x983, STL 0x387/0x987: eviction 84..87 (LU)
            Op9(0x183, c => { c.Op = "LDL"; c.Mod(84, 3, Evict); c.Mod(73, 3, MemType); c.Dst = 1; c.Add(c.R(16), MemOp(c, c.I(24, 8), null, c.B(91) ? c.I(32, 6) : -1, c.S(40, 24))); });
            Op9(0x187, c => { c.Op = "STL"; c.Mod(84, 3, Evict); c.Mod(73, 3, MemType); c.Dst = 0; c.Add(MemOp(c, c.I(24, 8), null, c.B(91) ? c.I(64, 6) : -1, c.S(40, 24)), c.R(32)); });
            // LDS 0x984, STS 0x388/0x988: stride 78..80 as a base-register suffix
            Op9(0x184, c =>
            {
                c.Op = "LDS"; c.Mod(c.B(87), "U"); c.Mod(73, 3, MemType); c.Dst = 1;
                c.Add(c.R(16), MemOp(c, c.I(24, 8), Stride(c), c.B(91) ? c.I(32, 6) : -1, c.S(40, 24)));
            });
            Op9(0x188, c =>
            {
                c.Op = "STS"; c.Mod(73, 3, MemType); c.Dst = 0;
                c.Add(MemOp(c, c.I(24, 8), Stride(c), c.B(91) ? c.I(64, 6) : -1, c.S(40, 24)), c.R(32));
            });
            // ATOMS 0x38c [NAK]: op 87..91, type 73..76
            Op12(0x38c, c =>
            {
                c.Op = "ATOMS"; c.Mod(87, 4, AtomOp); c.Mod(73, 3, AtomTypeS);
                c.Dst = 1; c.Add(c.R(16), MemOp(c, c.I(24, 8), Stride(c), -1, c.S(40, 24)), c.R(32));
            });
            // ATOMS.POPC.INC.32 0xf8c [oracle 62619a57fc0feb43@003a0]: increment by the popcount of the lanes hitting the address;
            // no data source, URZ printed: "ATOMS.POPC.INC.32 RZ, [R3.X4+URZ]"
            Op12(0xf8c, c =>
            {
                c.Op = "ATOMS"; c.Mod("POPC"); c.Mod("INC"); c.Mod("32");
                c.Dst = 1; c.Add(c.R(16), MemOp(c, c.I(24, 8), Stride(c), c.I(64, 6), c.S(40, 24), showUZ: true));
            });
            // 0x38d compare op 87..88: 0/2 CAS, 1 CAST, 3 CAST.SPIN (compare and store) [probe]
            Op12(0x38d, c =>
            {
                c.Op = "ATOMS";
                int cmp = c.I(87, 2);
                if ((cmp & 1) == 0) c.Mod("CAS"); else { c.Mod("CAST"); c.Mod(cmp == 3, "SPIN"); }
                c.Mod(73, 3, AtomTypeS);
                c.Dst = 1; c.Add(c.R(16), MemOp(c, c.I(24, 8), Stride(c), -1, c.S(40, 24)), c.R(32), c.R(64));
            });
            // ATOMG 0x3a8 / 0x9a8 [NAK]: P dst 81 printed always [oracle], op 87..91, type 73..76
            Op9(0x1a8, c =>
            {
                c.Op = "ATOMG"; c.Mod(c.B(72), "E"); c.Mod(87, 4, AtomOp); c.Mod(73, 3, AtomTypeG); MemOrder(c); c.Mod(84, 3, Evict);
                bool u = c.B(91);
                c.Dst = 2; c.Add(c.P(81), c.R(16), MemOp(c, c.I(24, 8), u && !c.B(90) ? ".U32" : null, u ? c.I(64, 6) : -1, c.S(40, 24)), c.R(32));
            });
            Op9(0x1a9, c =>
            {
                c.Op = "ATOMG"; c.Mod(c.B(72), "E"); c.Mod("CAS"); c.Mod(73, 3, AtomTypeG); MemOrder(c); c.Mod(84, 3, Evict);
                c.Dst = 2; c.Add(c.P(81), c.R(16), MemOp(c, c.I(24, 8), null, -1, c.S(40, 24)), c.R(32), c.R(64));
            });
            // RED 0x98e [NAK]: op 87..90
            Op9(0x18e, c =>
            {
                c.Op = "RED"; c.Mod(c.B(72), "E"); c.Mod(87, 3, AtomOp); c.Mod(73, 3, AtomTypeG); MemOrder(c); c.Mod(84, 3, Evict);
                bool u = c.B(91);
                c.Dst = 0; c.Add(MemOp(c, c.I(24, 8), u && !c.B(90) ? ".U32" : null, u ? c.I(64, 6) : -1, c.S(40, 24)), c.R(32));
            });

            // ALD [NAK] 0x321: dst, a[Ra+addr 40..50], vertex R32 (printed when not RZ); comps 74..76, patch 76, phys 77, output 79
            Op12(0x321, c =>
            {
                c.Op = "ALD"; c.Mod(c.B(77), "PHYS"); c.Mod(c.B(76), "P"); c.Mod(c.B(79), "O"); c.Mod(74, 2, null, "64", "96", "128");
                c.Dst = 1; c.Add(c.R(16), Attr(c, 24, 40, 10));
                if (c.I(32, 8) != 255) c.Add(c.R(32));
            });
            // AST [NAK] 0x322: a[Ra+addr], data R32, vertex R64
            Op12(0x322, c =>
            {
                c.Op = "AST"; c.Mod(c.B(77), "PHYS"); c.Mod(c.B(76), "P"); c.Mod(74, 2, null, "64", "96", "128");
                c.Dst = 0; c.Add(Attr(c, 24, 40, 10), c.R(32));
                if (c.I(64, 8) != 255) c.Add(c.R(64));
            });
            // IPA [NAK] 0x326: freq 78..80, loc 76..78, attribute 64..72 (x4), offset R32, multiplier R24 [oracle]
            Op12(0x326, c =>
            {
                c.Op = "IPA"; int freq = c.I(78, 2), loc = c.I(76, 2);
                c.Mod(freq switch { 0 => "PASS", 1 => "CONSTANT", 2 => "SC", _ => "?f3" });
                c.Mod(76, 2, null, "CENTROID", "OFFSET", "?l3");
                c.Mod(c.B(72), "SAT");
                c.Dst = 1; c.Add(c.R(16));
                if (!IsPT(c, 81)) { c.Add(c.P(81)); c.Dst++; }
                c.Add(new Operand(OperandKind.Attr, Offset: (long)c.F(64, 8) << 2));
                if (loc == 2 || c.I(32, 8) != 255) c.Add(c.R(32));
            });
            // ISBERD [NAK] 0x923
            Op12(0x923, c =>
            {
                c.Op = "ISBERD"; c.Mod(c.B(79), "O"); c.Mod(c.B(78), "SKEW"); c.Mod(76, 2, null, "PATCH", "PRIM", "ATTR"); c.Mod(74, 2, null, "U16", "32", "?3");
                c.Dst = 1; c.Add(c.R(16), MemOp(c, c.I(24, 8), null, -1, c.S(40, 16)));
            });
        }

        static string? Stride(Ctx c) => c.I(78, 2) switch { 1 => ".X4", 2 => ".X8", 3 => ".X16", _ => null };

        static Operand Attr(Ctx c, int regStart, int addrStart, int addrLen)
        {
            int r = c.I(regStart, 8);
            return new Operand(OperandKind.Attr, Offset: (long)c.F(addrStart, addrLen), IndexReg: r != 255 ? r : -1);
        }

        // [NAK] set_atom_op / set_atom_type (pre-SM90)
        static readonly string?[] AtomOp = { "ADD", "MIN", "MAX", "INC", "DEC", "AND", "OR", "XOR", "EXCH", "?9", "?10", "?11", "?12", "?13", "?14", "?15" };
        static readonly string?[] AtomTypeG = { null, "S32", "64", "F32.FTZ.RN", "F16x2.RN", "S64", "F64.RN", "?7" };
        static readonly string?[] AtomTypeS = { null, "S32", "64", "F32.FTZ.RN", "F16x2.RN", "S64", "F64.RN", "?7" };
    }
}
