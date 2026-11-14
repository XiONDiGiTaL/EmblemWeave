// SPDX-License-Identifier: MIT
// Control flow, barriers, system registers, warp ops. Opcodes and fields: [NAK] OpBra/OpBSSy/OpBSync/OpBreak/OpBMov/OpExit/
// OpKill/OpNop/OpWarpSync/OpBar/OpCS2R/OpS2R/OpVote/OpShfl/OpMatch/OpRedux/OpR2UR/OpFSwzAdd/OpOut/OpMemBar.
// Branch target = address + 16 + 4 * signed field at 34 [NAK set_rel_offset]; CALL/RET/BRX/BRXU/DEPBAR/ERRBAR/WARPSYNC
// spellings, FSWZADD lane pattern, SR_* names: [oracle] and [probe] (SrNames, from tools/shaders/ampere/probe).
using System;

namespace Nx.Shaders.Ampere
{
    public static partial class SassDecoder
    {
        static long Target(Ctx c, int len) => c.Addr + 16 + c.S(34, len) * 4;

        /// branch condition predicate 87/90, printed first when it is not PT
        static void BranchPred(Ctx c)
        {
            if (!(IsPT(c, 87) && !c.B(90))) c.Add(c.P(87, 90));
        }

        static void RegisterCtrl()
        {
            // BRA 0x947 [NAK]; 0x547 = uniform-predicate form (UP 24..27, .U)
            Op12(0x947, c => { c.Op = "BRA"; BraMods(c); BranchPred(c); c.Add(c.Label(Target(c, 48))); });
            Op12(0x547, c =>
            {
                c.Op = "BRA"; BraMods(c);
                if (!(c.I(24, 3) == 7 && !c.B(27))) c.Add(c.UP(24, 27));
                BranchPred(c); c.Add(c.Label(Target(c, 48)));
            });
            // BRX 0x949 / BRXU 0x958 [oracle]: "BRX Ra offset" (offset relative, no comma)
            Op12(0x949, c => { c.Op = "BRX"; BraMods(c); BranchPred(c); c.Add(c.Text(SassText.RegName(c.I(24, 8)) + " " + SassText.Hex(c.S(32, 50)), OperandKind.Other)); });
            Op12(0x958, c => { c.Op = "BRXU"; BraMods(c); BranchPred(c); c.Add(c.Text(SassText.URegName(c.I(24, 6)) + " " + SassText.Hex(c.S(32, 50)), OperandKind.Other)); });
            // JMP 0x94a / JMX 0x94c (absolute) [probe]
            // CALL.REL 0x944, RET.REL 0x950 [oracle]: NOINC / NODEC bit 86
            Op12(0x944, c => { c.Op = "CALL"; c.Mod("REL"); c.Mod(c.B(86), "NOINC"); BranchPred(c); c.Add(c.Label(Target(c, 48))); });
            Op12(0x950, c =>
            {
                c.Op = "RET"; c.Mod("REL"); c.Mod(c.B(86), "NODEC"); BranchPred(c);
                c.Add(c.Text(SassText.RegName(c.I(24, 8)) + " " + SassText.Hex(Target(c, 48)), OperandKind.Other));
            });
            // EXIT 0x94d, KILL 0x95b, NOP 0x918
            Op12(0x94d, c => { c.Op = "EXIT"; c.Mod(c.B(84), "KEEPREFCOUNT"); c.Mod(c.B(85), "NO_ATEXIT"); BranchPred(c); });
            Op12(0x95b, c => { c.Op = "KILL"; BranchPred(c); });
            Op12(0x918, c => { c.Op = "NOP"; });
            // BSSY 0x945 B16..20, target 34..64; BSYNC 0x941; BREAK 0x942 [NAK]
            Op12(0x945, c => { c.Op = "BSSY"; BranchPred(c); c.Add(Bar(c, 16), c.Label(Target(c, 30))); c.Dst = c.O[0].Kind == OperandKind.Special ? 1 : 0; });
            Op12(0x941, c => { c.Op = "BSYNC"; BranchPred(c); c.Add(Bar(c, 16)); });
            Op12(0x942, c => { c.Op = "BREAK"; BranchPred(c); c.Add(Bar(c, 16)); c.Dst = c.O[0].Kind == OperandKind.Special ? 1 : 0; });
            // BMOV 0x355 (R <- B) / 0x356 (B <- R) [NAK]
            Op12(0x355, c => { c.Op = "BMOV"; c.Mod("32"); c.Mod(c.B(84), "CLEAR"); c.Dst = 1; c.Add(c.R(16), Bar(c, 24)); });
            Op12(0x356, c => { c.Op = "BMOV"; c.Mod("32"); c.Mod(c.B(84), "CLEAR"); c.Dst = 1; c.Add(Bar(c, 24), c.R(32)); });
            // WARPSYNC 0x148 ALU form (imm/reg/cbuf) [NAK]
            Op9(0x148, c => { c.Op = "WARPSYNC"; BranchPred(c); c.ANeg = c.AAbs = c.BAbs = c.BNeg = -1; c.Add(c.Src2()); });
            // DEPBAR 0x91a [oracle]: DEPBAR.LE SBn, count (n 44..47, count 38..44)
            Op12(0x91a, c =>
            {
                c.Op = "DEPBAR"; c.Mod("LE");
                c.Add(c.Text("SB" + c.I(44, 3), OperandKind.Special), c.Hex(38, 6));
                int list = c.I(32, 6);
                if (list != 0) c.O.Add(c.Text(DepList(list)));
            });
            // BAR 0xb1d [NAK]: BAR.SYNC id
            Op12(0xb1d, c => { c.Op = "BAR"; c.Mod("SYNC"); c.Add(c.Hex(54, 4)); });
            Op12(0x91d, c => { c.Op = "BAR"; c.Mod("SYNC"); c.Add(c.R(24)); });
            // MEMBAR [probe] findings/membar_76.tsv: scope 76..79 (bit 79 = ALL, else SC; bit 80 drops the SC/ALL word)
            Op12(0x992, c => { c.Op = "MEMBAR"; if (!c.B(80)) c.Mod(c.B(79) ? "ALL" : "SC"); c.Mod(76, 3, "CTA", "SM", "GPU", "SYS", "INVALID4", "VC", "INVALID6", "INVALID7"); });
            Op12(0x9ab, c => { c.Op = "ERRBAR"; });
            // S2R 0x919 / S2UR 0x9c3 [NAK]: SR index 72..80; CS2R 0x805: 64-bit unless bit 80 clear? [oracle]
            Op12(0x919, c => { c.Op = "S2R"; c.Dst = 1; c.Add(c.R(16), Sr(c)); });
            Op12(0x9c3, c => { c.Op = "S2UR"; c.UGuard = false; c.Dst = 1; c.Add(c.UR(16), Sr(c)); });
            Op12(0x805, c => { c.Op = "CS2R"; c.Mod(!c.B(80), "32"); c.Dst = 1; c.Add(c.R(16), Sr(c)); });
            // R2UR 0x3c2 [NAK]
            Op12(0x3c2, c =>
            {
                c.Op = "R2UR"; c.Dst = 1; c.Add(c.UR(16));
                if (!IsPT(c, 81)) { c.Add(c.P(81)); c.Dst++; }
                c.Add(c.R(24));
            });
            // REDUX 0x3c4 [NAK]
            Op12(0x3c4, c => { c.Op = "REDUX"; c.Mod(78, 3, "AND", "OR", "XOR", "SUM", "MIN", "MAX", "?6", "?7"); c.Mod(c.B(73), "S32"); c.Dst = 1; c.Add(c.UR(16), c.R(24)); }); // [probe] findings/redux_72.tsv
            // VOTE 0x806 / VOTEU 0x886 [NAK]: op 72..74; Rd omitted when RZ/URZ [oracle]
            Op12(0x806, c =>
            {
                c.Op = "VOTE"; c.Mod(72, 2, "ALL", "ANY", "EQ", "?3");
                if (c.I(16, 8) != 255) { c.Add(c.R(16)); c.Dst++; }
                c.Add(c.P(81), c.P(87, 90)); c.Dst++;
            });
            Op12(0x886, c =>
            {
                c.Op = "VOTEU"; c.Mod(72, 2, "ALL", "ANY", "EQ", "?3");
                if (c.I(16, 6) != 63) { c.Add(c.UR(16)); c.Dst++; }
                c.Add(c.UP(81), c.P(87, 90)); c.Dst++;
            });
            // SHFL 0x389 (R,R) 0x589 (R,imm c) 0x989 (imm lane, R) 0xf89 (imm, imm) [NAK]: op 58..60, P dst 81
            Action<Ctx> shfl = c =>
            {
                c.Op = "SHFL"; c.Mod(58, 2, "IDX", "UP", "DOWN", "BFLY");
                int op = (int)(c.Lo & 0xfff);
                c.Dst = 2; c.Add(c.P(81), c.R(16), c.R(24));
                c.Add(op == 0x989 || op == 0xf89 ? c.Hex(53, 5) : c.R(32));
                c.Add(op == 0x589 || op == 0xf89 ? c.Hex(40, 13) : c.R(64));
            };
            Op12(0x389, shfl); Op12(0x589, shfl); Op12(0x989, shfl); Op12(0xf89, shfl);
            // MATCH 0x3a1 [NAK]
            Op12(0x3a1, c =>
            {
                c.Op = "MATCH"; c.Mod(c.B(79) ? "ANY" : "ALL"); c.Mod(c.B(73), "U64");
                c.Dst = 1;
                if (!IsPT(c, 81)) { c.Add(c.P(81)); c.Dst++; }
                c.Add(c.R(16), c.R(24));
            });
            // FSWZADD 0x822 [NAK]: lanes 32..40 printed as P/N per bit, MSB first [oracle]
            Op12(0x822, c =>
            {
                c.Op = "FSWZADD"; c.Mod(c.B(80), "FTZ"); c.Mod(78, 2, Rnd); c.Mod(c.B(77), "NDV");
                int v = c.I(32, 8); var s = new char[8];
                for (int k = 0; k < 4; k++) { int p = (v >> (6 - 2 * k)) & 3; s[2 * k] = p == 2 ? 'N' : p == 3 ? 'Z' : 'P'; s[2 * k + 1] = p == 1 ? 'N' : 'P'; } // 2 bits per lane [NAK FSwzAddOp] spelled PP/PN/NP/ZP [oracle]
                c.Dst = 1; c.Add(c.R(16), c.R(24), c.R(64), c.Text(new string(s)));
            });
            // OUT 0x124 [NAK]: EMIT/CUT 78..80
            Op9(0x124, c =>
            {
                c.Op = "OUT"; int t = c.I(78, 2); c.Mod(t switch { 0 => "FINAL", 1 => "EMIT", 2 => "CUT", _ => "EMIT_THEN_CUT" });
                c.ANeg = c.AAbs = c.BAbs = c.BNeg = -1;
                if (t == 0) { c.Add(c.SrcA()); return; } // OUT.FINAL Ra [oracle]
                c.Dst = 1; c.Add(c.R(16), c.SrcA(), c.Src2());
            });
            // LEPC 0x34e [oracle]
            Op12(0x34e, c => { c.Op = "LEPC"; c.Dst = 1; c.Add(c.R(16)); });
            // BMOV.64 ATEXIT_PC 0x357 (R32) / 0x957 (imm 32..64) [oracle + probe findings/bmov64_reg.tsv]
            Op12(0x357, c => { c.Op = "BMOV"; c.Mod("64"); c.Add(c.Text("ATEXIT_PC", OperandKind.Special), c.R(32)); });
            Op12(0x957, c => { c.Op = "BMOV"; c.Mod("64"); c.Add(c.Text("ATEXIT_PC", OperandKind.Special), c.Hex(32, 32)); });
            // BMOV.32 Bn <- special (0xf55, .CLEAR 84) and special <- Bn (0xf56, .PQUAD 84) [oracle + probe findings/bmov_special_f55.tsv]
            Op12(0xf55, c => { c.Op = "BMOV"; c.Mod("32"); c.Mod(c.B(84), "CLEAR"); c.Dst = 1; c.Add(Bar(c, 16), BarSpecial(c)); });
            Op12(0xf56, c => { c.Op = "BMOV"; c.Mod("32"); c.Mod(c.B(84), "PQUAD"); c.Dst = 1; c.Add(BarSpecial(c), Bar(c, 16)); });
        }

        static void BraMods(Ctx c)
        {
            // [probe] bits 32..34: .U (32), .DIV/.CONV etc. are filled in from probe findings
            c.Mod(c.B(32), "U");
        }

        static Operand Bar(Ctx c, int start) => c.Text("B" + c.I(start, 4), OperandKind.Special);

        static Operand BarSpecial(Ctx c) { int i = c.I(24, 6); return new Operand(OperandKind.Special, Reg: i, Text: BarSpecialNames[i] ?? "???" + i); }

        static string DepList(int mask)
        {
            var parts = new System.Collections.Generic.List<string>();
            for (int k = 5; k >= 0; k--) if ((mask & (1 << k)) != 0) parts.Add(k.ToString()); // printed high to low [oracle]
            return "{" + string.Join(",", parts) + "}";
        }

        static Operand Sr(Ctx c)
        {
            int i = c.I(72, 8);
            string n = i < SrNames.Length && SrNames[i] != null ? SrNames[i]! : "SR" + i;
            return new Operand(OperandKind.Special, Reg: i, Text: n);
        }
    }
}
