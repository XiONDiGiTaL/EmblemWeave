// SPDX-License-Identifier: MIT
// Shared model of one decoded Ampere (SM86, 128-bit) instruction: what the decoder (stream D3) produces and the translator front end
// (stream D4) consumes. It mirrors nvdisasm's text form, so ToSassText() can be compared with the hash-locked oracle line by line.
// Both streams code against this file; changes go through the lead.
using System.Collections.Generic;

namespace Nx.Shaders.Ampere
{
    public enum OperandKind
    {
        Reg,        // Rn, RZ = 255
        UReg,       // URn, URZ = 63
        Pred,       // Pn, PT = 7
        UPred,      // UPn, UPT = 7
        Imm,        // integer immediate (ImmBits)
        FImm,       // floating-point immediate (ImmBits holds the raw bits; Text the nvdisasm spelling)
        CBuf,       // c[Bank][Offset], optionally + IndexReg (Reg or UReg); bank may itself be a UReg (cx[URn])
        Mem,        // [Rn(+URm)(+Offset)] for loads/stores; space in the opcode modifiers
        Attr,       // a[Offset] (IPA, ALD, AST)
        Label,      // branch target address (Offset)
        Special,    // SR_* and other named operands (Text)
        Other,      // anything else nvdisasm prints verbatim (Text), e.g. texture shape "2D", masks
    }

    public sealed record Operand(
        OperandKind Kind,
        int Reg = 0,
        bool Neg = false,
        bool Abs = false,
        bool Not = false,
        int Bank = 0,
        bool BankIsUReg = false,
        long Offset = 0,
        int IndexReg = -1,
        bool IndexIsUReg = false,
        ulong ImmBits = 0,
        string Text = "",
        IReadOnlyList<string>? Suffixes = null); // ".reuse", ".H1", ".B2" ... as printed after the operand

    public sealed record SassInst(
        int Address,
        ulong Lo,
        ulong Hi,
        int Pred,                       // guard predicate 0..7 (7 = PT, i.e. none); UPn guards use UPred = true
        bool PredNeg,
        bool PredIsUniform,
        string Opcode,                  // base mnemonic, e.g. "IADD3", "TEX"
        IReadOnlyList<string> Mods,     // modifiers in printed order, e.g. ["SCR", "NODEP"]
        IReadOnlyList<Operand> Operands,// in nvdisasm order
        int DstCount,                   // how many leading Operands are written
        uint Sched)                     // the scheduling/control bits from the high word (stall, yield, barriers, reuse)
    {
        /// nvdisasm-style text of the instruction without address or hex, e.g. "@!P0 IADD3 R1, R2, 0x4, RZ ;".
        public string ToSassText() => SassText.Format(this);
    }
}
