// SPDX-License-Identifier: MIT
// Texture and surface ops. Opcodes (bound 0xb6x, bindless 0x36x), handle c-slot 40..59 (offset/4 in 40..54, index 54..59),
// scalar 60, dim 61..64, channel mask 72..76, AOFFI 76, NDV 77, DC 78, LOD mode 87..90, NODEP 90, TLD4 component 87..89,
// TXQ query 62..64, surface dim/mask: [NAK] OpTex/OpTld/OpTld4/OpTmml/OpTxd/OpTxq/OpSuLd/OpSuSt.
// Printed operand order (second destination R64 first), mod spellings (SCR, LZ/LL, dims, TEX_HEADER_*), bound SULD/SUST
// opcodes 0xb97/0xb9b: [oracle].
using System;

namespace Nx.Shaders.Ampere
{
    public static partial class SassDecoder
    {
        static readonly string[] TexDim = { "1D", "2D", "3D", "CUBE", "ARRAY_1D", "ARRAY_2D", "?dim6", "ARRAY_CUBE" };
        static readonly string?[] TexLod = { null, "LZ", "LB", "LL", "LC", "LB.LC", "?lod6", "?lod7" };
        static readonly string[] ImgDim = { "1D", "1D_BUFFER", "1D_ARRAY", "2D", "2D_ARRAY", "3D", "?6", "?7" };

        static void TexHandle(Ctx c, bool bindless)
        {
            if (bindless) return;
            if (IsUrHandle(c)) { c.Add(c.UR(40), c.Hex(46, 8)); return; }
            c.Add(c.Hex(54, 5), c.Imm((long)c.F(40, 14), SassText.Hex((long)c.F(40, 14))));
        }

        /// TEX/TLD/TLD4 0xf60/0xf66/0xf63: the handle is a uniform register (40..45) plus an 8-bit immediate (46..53) [probe].
        static bool IsUrHandle(Ctx c) => (c.Lo & 0xfff) is 0xf60 or 0xf63 or 0xf66;

        static void TexMask(Ctx c)
        {
            if (c.I(72, 4) != 0xf) c.Add(c.Hex(72, 4));
        }

        /// TEX / TLD / TLD4 / TMML / TXD common shape: R64, R16, R24, [R32], handle, dim, [mask]
        static void TexCommon(Ctx c, string op, bool bindless, Action<Ctx> mods)
        {
            c.Op = op;
            c.Mod(c.B(60), "SCR");
            c.Mod(bindless, "B");
            // bit 79: half-precision results, printed "F16.RN" [probe]
            if (c.B(79)) { c.Mod("F16"); c.Mod("RN"); }
            mods(c);
            c.Mod(84, 3, Evict);
            c.Mod(c.B(90), "NODEP");
            c.Dst = 2;
            c.Add(c.R(64, reuse: false), c.R(16, reuse: false));
            if (!IsPT(c, 81)) { c.Add(c.P(81)); c.Dst++; }
            c.Add(c.R(24, reuse: false));
            if (c.I(32, 8) != 255) c.Add(c.R(32, reuse: false));
            TexHandle(c, bindless);
            c.Add(c.Text(TexDim[c.I(61, 3)]));
            TexMask(c);
        }

        static void RegisterTex()
        {
            Action<Ctx> texMods = c => { c.Mod(87, 3, TexLod); c.Mod(c.B(76), "AOFFI"); c.Mod(c.B(78), "DC"); c.Mod(c.B(77), "NDV"); };
            Action<Ctx> tldMods = c => { c.Mod(87, 3, TexLod); c.Mod(c.B(76), "AOFFI"); c.Mod(c.B(78), "MS"); c.Mod(c.B(77), "CL"); };
            Action<Ctx> tld4Mods = c => { c.Mod(87, 2, "R", "G", "B", "A"); c.Mod(76, 2, null, "AOFFI", "PTP", "?3"); c.Mod(c.B(78), "DC"); };
            Action<Ctx> tmmlMods = c => { c.Mod(c.B(77), "NDV"); };
            Action<Ctx> txdMods = c => { c.Mod(c.B(76), "AOFFI"); };
            Op12(0xb60, c => TexCommon(c, "TEX", false, texMods));
            Op12(0x361, c => TexCommon(c, "TEX", true, texMods));
            Op12(0xf60, c => TexCommon(c, "TEX", false, texMods));
            Op12(0xb66, c => TexCommon(c, "TLD", false, tldMods));
            Op12(0x367, c => TexCommon(c, "TLD", true, tldMods));
            Op12(0xb63, c => TexCommon(c, "TLD4", false, tld4Mods));
            Op12(0x364, c => TexCommon(c, "TLD4", true, tld4Mods));
            Op12(0xb69, c => TexCommon(c, "TMML", false, tmmlMods));
            Op12(0x36a, c => TexCommon(c, "TMML", true, tmmlMods));
            Op12(0xb6c, c => TexCommon(c, "TXD", false, txdMods));
            Op12(0x36d, c => TexCommon(c, "TXD", true, txdMods));
            // TXQ: R64, R16, R24, query, handle, mask
            Action<Ctx> txq = c =>
            {
                bool bl = (c.Lo & 0xfff) == 0x370;
                c.Op = "TXQ"; c.Mod(bl, "B"); c.Mod(c.B(90), "NODEP");
                c.Dst = 2; c.Add(c.R(64, reuse: false), c.R(16, reuse: false), c.R(24, reuse: false));
                c.Add(c.Text(c.I(62, 2) switch { 0 => "TEX_HEADER_DIMENSION", 1 => "TEX_HEADER_TEXTURE_TYPE", 2 => "TEX_HEADER_SAMPLER_POS", _ => "?q3" }));
                TexHandle(c, bl);
                TexMask(c);
            };
            Op12(0xb6f, txq); Op12(0x370, txq);

            // surfaces: bound SULD 0xb97 / SUST 0xb9b [oracle], bindless 0x998/0x99a, 0x99c/0x99e [NAK]
            Op12(0xb97, c => Surface(c, true, false));
            Op12(0x998, c => Surface(c, true, true));
            Op12(0xb9b, c => Surface(c, false, false));
            Op12(0x99c, c => Surface(c, false, true));
        }

        static string Channels(int m)
        {
            string s = "";
            if ((m & 1) != 0) s += "R";
            if ((m & 2) != 0) s += "G";
            if ((m & 4) != 0) s += "B";
            if ((m & 8) != 0) s += "A";
            return s;
        }

        static void Surface(Ctx c, bool load, bool bindless)
        {
            c.Op = load ? "SULD" : "SUST";
            c.Mod("P"); c.Mod(bindless, "B");
            c.Mod(ImgDim[c.I(61, 3)]);
            int m = c.I(72, 4); if (m != 0xf) c.Mod(Channels(m));
            MemOrder(c);
            c.Mod(84, 3, Evict);
            c.Mod("IGN");
            if (load)
            {
                c.Dst = 1; c.Add(c.R(16));
                if (!IsPT(c, 81)) { c.Add(c.P(81)); c.Dst++; }
                c.Add(MemOp(c, c.I(24, 8), null, -1, 0));
            }
            else { c.Dst = 0; c.Add(MemOp(c, c.I(24, 8), null, -1, 0), c.R(32)); }
            if (bindless) c.Add(c.R(64)); else TexHandle(c, false);
        }
    }
}
