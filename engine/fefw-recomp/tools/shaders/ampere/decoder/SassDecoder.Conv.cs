// SPDX-License-Identifier: MIT
// Conversions. Opcodes and fields (dst size 75..77, src size 84..86, signed 72/74, rounding 78..80, FTZ 80, NTZ 77): [NAK]
// OpF2F/OpF2I/OpI2F/OpFRnd/OpF2FP. I2FP (0x045), the F2FP PACK_AB/MERGE_C spellings, FRND/F2I rounding names
// (FLOOR/CEIL/TRUNC), which types are printed and in what order: [oracle].
using System;

namespace Nx.Shaders.Ampere
{
    public static partial class SassDecoder
    {
        static readonly string?[] RndInt = { null, "FLOOR", "CEIL", "TRUNC" };

        static string FType(int size) => size switch { 0 => "F8", 1 => "F16", 2 => "F32", _ => "F64" };
        static string IType(int size, bool signed) => (signed ? "S" : "U") + (8 << size);

        static void ConvSrc(Ctx c, int srcSize)
        {
            c.ANeg = -1; c.AAbs = -1; c.BAbs = 62; c.BNeg = 63;
            if (srcSize == 1) c.Swz = SwzH1;
        }

        static void RegisterConv()
        {
            // F2F 0x104 / 0x110 (64-bit)
            Action<Ctx> f2f = c =>
            {
                c.Op = "F2F"; int ds = c.I(75, 2), ss = c.I(84, 2);
                c.Mod(c.B(80), "FTZ"); c.Mod(FType(ds)); c.Mod(FType(ss));
                if (ds == ss || ds > ss) c.Mod(78, 2, RndInt); else c.Mod(78, 2, Rnd);
                ConvSrc(c, ss); c.ImmK = ImmKind.F32; c.Dst = 1; c.Add(c.R(16), c.Src2());
            };
            Op9(0x104, f2f); Op9(0x110, f2f);
            // F2I 0x105 / 0x111 (64-bit)
            Action<Ctx> f2i = c =>
            {
                c.Op = "F2I"; int ds = c.I(75, 2), ss = c.I(84, 2); bool sg = c.B(72);
                c.Mod(c.B(80), "FTZ");
                if (!(ds == 2 && sg)) c.Mod(IType(ds, sg));
                if (ss != 2) c.Mod(FType(ss));
                c.Mod(78, 2, RndInt); c.Mod(c.B(77), "NTZ");
                ConvSrc(c, ss); c.ImmK = ImmKind.F32; c.Dst = 1; c.Add(c.R(16), c.Src2());
            };
            Op9(0x105, f2i); Op9(0x111, f2i);
            // I2F 0x106 / 0x112 (64-bit): signed 74, sub-word select 60..62 (.B1..B3 / .H1)
            Action<Ctx> i2f = c =>
            {
                c.Op = "I2F"; int ds = c.I(75, 2), ss = c.I(84, 2); bool sg = c.B(74);
                if (ds != 2) c.Mod(FType(ds));
                if (!(ss == 2 && sg)) c.Mod(IType(ss, sg));
                c.Mod(78, 2, Rnd);
                c.ANeg = -1; c.AAbs = -1; c.BAbs = -1; c.BNeg = -1;
                c.Swz = (cc, slot) => { int v = cc.I(60, 2); return v == 0 ? null : (ss == 0 ? ".B" + v : ss == 1 ? ".H" + v : "?sel" + v); };
                c.ImmK = ImmKind.SHex; c.Dst = 1; c.Add(c.R(16), c.Src2());
            };
            Op9(0x106, i2f); Op9(0x112, i2f);
            // FRND 0x107 / 0x113 (64-bit)
            Action<Ctx> frnd = c =>
            {
                c.Op = "FRND"; int ds = c.I(75, 2), ss = c.I(84, 2);
                c.Mod(c.B(80), "FTZ");
                if (ds != 2 || ss != 2) { c.Mod(FType(ds)); if (ss != ds) c.Mod(FType(ss)); }
                c.Mod(78, 2, RndInt);
                ConvSrc(c, ss); c.ImmK = ImmKind.F32; c.Dst = 1; c.Add(c.R(16), c.Src2());
            };
            Op9(0x107, frnd); Op9(0x113, frnd);
            // I2FP 0x045 [oracle]: always F32 then S32/U32 (signed 74), rounding 78..80
            Op9(0x045, c =>
            {
                c.Op = "I2FP"; c.Mod("F32"); c.Mod(c.B(74) ? "S32" : "U32"); c.Mod(78, 2, Rnd);
                c.ANeg = -1; c.AAbs = -1; c.BAbs = -1; c.BNeg = -1; c.ImmK = ImmKind.SHex; c.Dst = 1; c.Add(c.R(16), c.Src2());
            });
            // F2FP 0x03e [NAK]: MERGE_C 78 (dst, B slot, R64; .H1 72), else PACK_AB (dst, A, B slot); rounding 79..81
            Op9(0x03e, c =>
            {
                c.Op = "F2FP"; bool merge = c.B(78);
                c.Mod(c.B(77), "RELU"); // [probe] see probe findings
                c.Mod(merge ? "MERGE_C" : "PACK_AB");
                c.Mod(c.B(72) && merge, "H1");
                c.Mod(79, 2, Rnd);
                c.ImmK = ImmKind.F32; c.Dst = 1; c.Add(c.R(16));
                if (merge) c.Add(c.Src2(), c.R(64)); else c.Add(c.SrcA(), c.Src2());
            });
        }
    }
}
