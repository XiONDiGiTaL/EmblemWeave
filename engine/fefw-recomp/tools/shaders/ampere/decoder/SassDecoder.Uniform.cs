// SPDX-License-Identifier: MIT
// Uniform datapath. Opcodes [NAK] (UIADD3 0x090, ULEA 0x091, ULOP3 0x092, USHF 0x099, USEL 0x087, UISETP 0x08c, UIMAD 0x0a4,
// UIMAD.WIDE 0x0a5, UFLO 0x0bd, UBREV 0x0be, UPOPC 0x0bf, USGXT 0x09a, UBMSK 0x09b, UMOV 0xc82, UPLOP3 0x89c); UPRMT 0x096 and
// the operand layouts are the vector ones with UR/UP register files [oracle]. Each one reuses the vector handler.
using System;

namespace Nx.Shaders.Ampere
{
    public static partial class SassDecoder
    {
        static void Uni(int op, int vectorOp, bool op12 = false)
        {
            var h = op12 ? ByOp12[vectorOp] : ByOp9[vectorOp];
            Action<Ctx> u = c => { c.U = true; h(c); c.Op = "U" + c.Op; };
            if (op12) Op12(op, u); else Op9(op, u);
        }

        static void RegisterUniform()
        {
            Uni(0x090, 0x010); // UIADD3
            Uni(0x091, 0x011); // ULEA
            Uni(0x092, 0x012); // ULOP3
            Uni(0x096, 0x016); // UPRMT
            Uni(0x099, 0x019); // USHF
            Uni(0x087, 0x007); // USEL
            Uni(0x08c, 0x00c); // UISETP
            Uni(0x0a4, 0x024); // UIMAD
            Uni(0x0a5, 0x025); // UIMAD.WIDE
            Uni(0x0bd, 0x100); // UFLO
            Uni(0x0be, 0x101); // UBREV
            Uni(0x0bf, 0x109); // UPOPC
            Uni(0x09a, 0x01a); // USGXT
            Uni(0x09b, 0x01b); // UBMSK
            Uni(0x082, 0x002); // UMOV
            Uni(0x89c, 0x81c, true); // UPLOP3
            Op12(0x883, c => { c.U = true; ByOp9[0x003](c); c.Op = "UP2UR"; }); // UP2UR URd, UPR, URb, mask: the P2R layout [oracle: "UP2UR UR6, UPR, URZ, 0x1"]
        }
    }
}
