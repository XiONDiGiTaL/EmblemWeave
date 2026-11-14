// SPDX-License-Identifier: MIT
// Ampere (SM86) 128-bit SASS decoder. Dispatch is a table lookup on the opcode: the full 12 bits first, then the 9-bit ALU
// opcode (bits 0..8, form in 9..11). Handlers live in SassDecoder.*.cs grouped by unit; each one names its provenance:
// [NAK] = inverted from Mesa NAK sm70_encode.rs (MIT, Collabora Ltd.), [probe] = field found by flipping bits of corpus words
// and reading nvdisasm's text (tools/shaders/ampere/probe), [oracle] = spelling read from the nvdisasm oracle text.
using System;
using System.Buffers.Binary;
using System.Collections.Generic;

namespace Nx.Shaders.Ampere
{
    public static partial class SassDecoder
    {
        static readonly Dictionary<int, Action<Ctx>> ByOp12 = new();
        static readonly Dictionary<int, Action<Ctx>> ByOp9 = new();

        static SassDecoder()
        {
            RegisterFloat();
            RegisterInt();
            RegisterHalf();
            RegisterConv();
            RegisterMem();
            RegisterTex();
            RegisterCtrl();
            RegisterUniform();
        }

        static void Op12(int op, Action<Ctx> h) => ByOp12[op] = h;
        static void Op9(int op, Action<Ctx> h) => ByOp9[op] = h;

        /// Number of handled opcodes (12-bit and 9-bit tables).
        public static int RuleCount => ByOp12.Count + ByOp9.Count;

        public static SassInst Decode(ulong lo, ulong hi, int address)
        {
            var c = new Ctx(lo, hi, address);
            if (ByOp12.TryGetValue((int)(lo & 0xfff), out var h) || ByOp9.TryGetValue((int)(lo & 0x1ff), out h))
                h(c);
            else
            {
                c.Op = ".UNKNOWN_" + (lo & 0xfff).ToString("x3");
                c.Unknown = true;
            }
            return c.Build();
        }

        /// Decodes [start, end) of a code block (16 bytes per instruction). Addresses are relative to start.
        public static IReadOnlyList<SassInst> DecodeBlock(ReadOnlySpan<byte> code, int start, int end)
        {
            int n = (end - start) / 16;
            var list = new List<SassInst>(n);
            for (int k = 0; k < n; k++)
            {
                int o = start + k * 16;
                ulong lo = BinaryPrimitives.ReadUInt64LittleEndian(code.Slice(o, 8));
                ulong hi = BinaryPrimitives.ReadUInt64LittleEndian(code.Slice(o + 8, 8));
                list.Add(Decode(lo, hi, k * 16));
            }
            return list;
        }

        // ---- shared tables ---------------------------------------------------------------------------------------------
        // [NAK] set_float_cmp_op / set_int_cmp_op / set_pred_set_op / set_rnd_mode
        internal static readonly string[] FCmp = { "F", "LT", "EQ", "LE", "GT", "NE", "GE", "NUM", "NAN", "LTU", "EQU", "LEU", "GTU", "NEU", "GEU", "T" };
        internal static readonly string[] ICmp = { "F", "LT", "EQ", "LE", "GT", "NE", "GE", "T" };
        internal static readonly string[] PSet = { "AND", "OR", "XOR", "?3" };
        internal static readonly string?[] Rnd = { null, "RM", "RP", "RZ" };

        /// predicate operand printed only when it is not PT (used for optional predicate destinations).
        static bool IsPT(Ctx c, int start) => c.I(start, 3) == 7;
    }
}
