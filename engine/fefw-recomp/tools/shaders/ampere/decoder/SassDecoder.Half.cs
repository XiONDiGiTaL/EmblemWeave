// SPDX-License-Identifier: MIT
// Packed fp16 ALU. Opcodes, abs/neg/swizzle positions (A 73/72/74..76, B 62/63/60..62, C 83/84/81..83), SAT/F32/FTZ/RELU,
// HSETP2 fields: [NAK] OpHAdd2/OpHFma2/OpHMul2/OpHSet2/OpHSetP2/OpHMnMx2. Swizzle spellings, immediate pairs: [oracle].
namespace Nx.Shaders.Ampere
{
    public static partial class SassDecoder
    {
        // F16v2 source selector: 0 = H1_H0 (not printed), 1 = F32, 2 = H0_H0, 3 = H1_H1 [NAK set_swizzle + oracle]
        static string? SwzV2(Ctx c, int slot)
        {
            // [probe] the operand printed second (B slot, or the register at 64 in forms 2/3/7) has a third selector bit, 86
            bool second = (c.Op9 == 0x030 || c.Op9 == 0x031 || c.Op9 == 0x032) && (slot == 32 || (slot == 64 && (c.Form == 2 || c.Form == 3 || c.Form == 7)));
            int v = (slot switch { 24 => c.I(74, 2), 32 => c.I(60, 2), _ => c.I(81, 2) }) | (second && c.B(86) ? 4 : 0);
            return v switch { 0 => null, 1 => ".F32", 2 => ".H0_H0", 3 => ".H1_H1", 4 => ".H0_NH1", _ => ".INVALID" + v };
        }

        static void HalfSetup(Ctx c)
        {
            c.AAbs = 73; c.ANeg = 72; c.BAbs = 62; c.BNeg = 63; c.CAbs = 83; c.CNeg = 84;
            c.ImmK = ImmKind.F16x2; c.Swz = SwzV2;
        }

        static void RegisterHalf()
        {
            // HADD2 0x030: SAT 77, F32 78, FTZ 80
            Op9(0x030, c =>
            {
                c.Op = "HADD2"; c.Src2Slot = 2; c.Mod(c.B(78), "F32"); c.Mod(c.B(80), "FTZ"); c.Mod(c.B(77), "SAT"); HalfSetup(c);
                c.Dst = 1; c.Add(c.R(16), c.SrcA(), c.Src2());
            });
            // HFMA2 0x031: DNZ 76, SAT 77, F32 78, RELU 79, FTZ 80
            Op9(0x031, c =>
            {
                c.Op = "HFMA2"; c.Mod(c.B(78), "F32"); c.Mod(c.B(80), "FTZ"); c.Mod(!c.B(80) && c.B(76), "FMZ"); c.Mod(c.B(77), "SAT"); c.Mod(c.B(79), "RELU"); HalfSetup(c);
                c.Dst = 1; c.Add(c.R(16), c.SrcA()); var (b, d) = c.SrcBC(); c.Add(b, d); if (c.B(79) && !(IsPT(c, 87) && !c.B(90))) c.Add(c.P(87, 90)); // RELU predicate [probe]
            });
            // HMUL2 0x032
            Op9(0x032, c =>
            {
                c.Op = "HMUL2"; c.Mod(c.B(78), "F32"); c.Mod(c.B(80), "FTZ"); c.Mod(!c.B(80) && c.B(76), "FMZ"); c.Mod(c.B(77), "SAT"); c.Mod(c.B(79), "RELU"); HalfSetup(c);
                c.Dst = 1; c.Add(c.R(16), c.SrcA(), c.Src2());
            });
            // HSET2 0x033: set op 69..71, cmp 76..80, BF 71, FTZ 80
            Op9(0x033, c =>
            {
                c.Op = "HSET2"; c.Mod(c.B(71), "BF"); c.Mod(76, 4, FCmp); c.Mod(c.B(80), "FTZ"); c.Mod(69, 2, PSet); HalfSetup(c);
                c.Dst = 1; c.Add(c.R(16), c.SrcA(), c.Src2(), c.P(87, 90));
            });
            // HSETP2 0x034: set op 69..71, H_AND 71, cmp 76..80, FTZ 80
            Op9(0x034, c =>
            {
                c.Op = "HSETP2"; c.Mod(76, 4, FCmp); c.Mod(c.B(80), "FTZ"); c.Mod(c.B(71), "H_AND"); c.Mod(69, 2, PSet); HalfSetup(c);
                c.Dst = 2; c.Add(c.P(81), c.P(84), c.SrcA(), c.Src2(), c.P(87, 90));
            });
            // HMNMX2 0x040: F32 78, FTZ 80, NAN 81, XORSIGN 82, min/max predicate 87
            Op9(0x040, c =>
            {
                c.Op = "HMNMX2"; c.Mod(c.B(78), "F32"); c.Mod(c.B(80), "FTZ"); c.Mod(c.B(81), "NAN"); c.Mod(c.B(82), "XORSIGN"); HalfSetup(c);
                c.Dst = 1; c.Add(c.R(16), c.SrcA(), c.Src2(), c.P(87, 90));
            });
        }
    }
}
