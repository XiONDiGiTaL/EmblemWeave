// SPDX-License-Identifier: MIT
// FP32/FP64 ALU and MUFU. Opcodes, operand slots, FTZ/DNZ/SAT/rounding/compare fields: [NAK] OpFFma/OpFMul/OpFAdd/OpFMnMx/
// OpFSet/OpFSetP/OpMuFu/OpDAdd/OpDMul/OpDFma/OpDSetP. FMUL scale field 84..87 values, FSEL, FSET.BF spelling: [oracle].
namespace Nx.Shaders.Ampere
{
    public static partial class SassDecoder
    {
        // FMUL 84..87: [oracle] 1 D8, 2 D4, 3 D2, 4 none, 5 M2, 6 M4, 7 M8
        static readonly string?[] PDiv = { "?PDIV0", "D8", "D4", "D2", null, "M2", "M4", "M8" };

        static void FloatMods(Ctx c, bool dnz, bool pdiv)
        {
            if (c.B(80)) c.Mod("FTZ");
            else if (dnz && c.B(76)) c.Mod("FMZ");
            if (pdiv) c.Mod(84, 3, PDiv);
            c.Mod(78, 2, Rnd);
            c.Mod(c.B(77), "SAT");
        }

        static void RegisterFloat()
        {
            // FFMA [NAK] 0x023
            Op9(0x023, c => { c.Op = "FFMA"; FloatMods(c, true, false); c.ImmK = ImmKind.F32; c.Dst = 1; c.Add(c.R(16), c.SrcA()); var (b, d) = c.SrcBC(); c.Add(b, d); });
            // FMUL [NAK] 0x020
            Op9(0x020, c => { c.Op = "FMUL"; FloatMods(c, true, true); c.ImmK = ImmKind.F32; c.Dst = 1; c.Add(c.R(16), c.SrcA(), c.Src2()); });
            // FADD [NAK] 0x021
            Op9(0x021, c => { c.Op = "FADD"; c.Src2Slot = 2; FloatMods(c, false, false); c.ImmK = ImmKind.F32; c.Dst = 1; c.Add(c.R(16), c.SrcA(), c.Src2()); });
            // FMNMX [NAK] 0x009
            Op9(0x009, c => { c.Op = "FMNMX"; c.Mod(c.B(80), "FTZ"); c.ImmK = ImmKind.F32; c.Dst = 1; c.Add(c.R(16), c.SrcA(), c.Src2(), c.P(87, 90)); });
            // FSEL 0x008 [oracle] (same layout as SEL)
            Op9(0x008, c => { c.Op = "FSEL"; c.ImmK = ImmKind.F32; c.Dst = 1; c.Add(c.R(16), c.SrcA(), c.Src2(), c.P(87, 90)); });
            // FSET [NAK] 0x00a; nvdisasm always spells .BF on SM86 [oracle]
            Op9(0x00a, c =>
            {
                c.Op = "FSET"; c.Mod("BF"); c.Mod(76, 4, FCmp); c.Mod(c.B(80), "FTZ"); c.Mod(74, 2, PSet);
                c.ImmK = ImmKind.F32; c.Dst = 1; c.Add(c.R(16), c.SrcA(), c.Src2(), c.P(87, 90));
            });
            // FSETP [NAK] 0x00b
            Op9(0x00b, c =>
            {
                c.Op = "FSETP"; c.Mod(76, 4, FCmp); c.Mod(c.B(80), "FTZ"); c.Mod(74, 2, PSet);
                c.ImmK = ImmKind.F32; c.Dst = 2; c.Add(c.P(81), c.P(84), c.SrcA(), c.Src2(), c.P(87, 90));
            });
            // MUFU [NAK] 0x108: op 74..80, .F16 bit 73
            Op9(0x108, c =>
            {
                c.Op = "MUFU"; c.Mod(74, 6, "COS", "SIN", "EX2", "LG2", "RCP", "RSQ", "RCP64H", "RSQ64H", "SQRT", "TANH");
                bool f16 = c.B(73); c.Mod(f16, "F16");
                if (f16) c.Swz = SwzH1;
                c.ImmK = ImmKind.F32; c.Dst = 1; c.Add(c.R(16), c.Src2());
            });
            // FP64 [NAK] DMUL 0x028, DADD 0x029, DFMA 0x02b, DSETP 0x02a (immediate is the high 32 bits of a double)
            Op9(0x028, c => { c.Op = "DMUL"; c.Mod(78, 2, Rnd); c.ImmK = ImmKind.None; c.Dst = 1; c.Add(c.R(16), c.SrcA(), D64(c, c.Src2())); });
            Op9(0x029, c => { c.Op = "DADD"; c.Mod(78, 2, Rnd); c.ImmK = ImmKind.None; c.Dst = 1; c.Add(c.R(16), c.SrcA(), D64(c, c.Src2())); });
            Op9(0x02b, c => { c.Op = "DFMA"; c.Mod(78, 2, Rnd); c.ImmK = ImmKind.None; c.Dst = 1; c.Add(c.R(16), c.SrcA()); var (b, d) = c.SrcBC(); c.Add(D64(c, b), D64(c, d)); });
            Op9(0x02a, c =>
            {
                c.Op = "DSETP"; c.Mod(76, 4, FCmp); c.Mod(74, 2, PSet); c.ImmK = ImmKind.None;
                c.Dst = 2; c.Add(c.P(81), c.P(84), c.SrcA(), D64(c, c.Src2()), c.P(87, 90));
            });
        }

        /// fp64 immediates hold the top 32 bits of the double.
        static Operand D64(Ctx c, Operand o)
        {
            if (o.Kind != OperandKind.Imm) return o;
            ulong bits = (ulong)(uint)o.ImmBits << 32;
            return new Operand(OperandKind.FImm, ImmBits: bits, Text: SassText.F64(bits));
        }

        /// scalar fp16 source selector (60..62 for the B slot, 74..76 for A): 1 = .H1 [oracle]
        static string? SwzH1(Ctx c, int slot)
        {
            int v = slot switch { 24 => c.I(74, 2), 32 => c.I(60, 2), _ => c.I(81, 2) };
            return v == 1 ? ".H1" : v == 0 ? null : "?swz" + v;
        }
    }
}
