// CUDA kernel descriptor: what tools/shaders/ampere/extract_corpus.py keeps of a cubin kernel (.nv.info, .nv.shared,
// .nv.constant4) in place of an NVN control section. Layout (u32 words): [0] 'CUDA', [1] version 1, [2] SM, [3] text size,
// [4] parameter offset in bank 0, [5] parameter bytes, [6] static shared memory bytes, [7..9] EIATTR_MAX_THREADS,
// [10..12] EIATTR_REQNTID, [13] registers, [14] frame bytes, [15] bank 4 bytes, [16] indirect branch count; then per
// BRX {address, count, targets}; then the bank 4 bytes.
using System;
using System.Collections.Generic;

namespace SpirvHarness
{
    sealed class CudaKernel
    {
        public const uint Magic = 0x41445543;

        public int Sm, TextSize, ParamOffset, ParamSize, Shared, Registers, FrameSize;
        public int[] MaxThreads = new int[3], ReqNtid = new int[3];
        public readonly Dictionary<int, int[]> IndirectTargets = new();
        public byte[] Bank4 = Array.Empty<byte>();

        public static bool Is(byte[] ctrl) => ctrl != null && ctrl.Length >= 0x44 && BitConverter.ToUInt32(ctrl, 0) == Magic;

        public static CudaKernel Read(byte[] d)
        {
            int W(int k) => BitConverter.ToInt32(d, 4 * k);
            CudaKernel k = new()
            {
                Sm = W(2), TextSize = W(3), ParamOffset = W(4), ParamSize = W(5), Shared = W(6),
                MaxThreads = [W(7), W(8), W(9)], ReqNtid = [W(10), W(11), W(12)], Registers = W(13), FrameSize = W(14),
            };
            int bank4 = W(15), brx = W(16), p = 17 * 4;
            for (int i = 0; i < brx; i++)
            {
                int addr = BitConverter.ToInt32(d, p), n = BitConverter.ToInt32(d, p + 4);
                int[] t = new int[n];
                for (int j = 0; j < n; j++) t[j] = BitConverter.ToInt32(d, p + 8 + 4 * j);
                k.IndirectTargets[addr] = t;
                p += 8 + 4 * n;
            }
            k.Bank4 = d.AsSpan(p, Math.Min(bank4, d.Length - p)).ToArray();
            return k;
        }

        /// The work group the SPIR-V is built for: REQNTID, else MAX_THREADS, else 128 (the launch size of the game's DLSS
        /// kernels without either, seen in Eden-NS2's dispatch log). The runtime overrides it per launch through the
        /// specialization constants 0..2 (see Program.AddWorkgroupSizeSpecConstants).
        public (int X, int Y, int Z) LocalSize =>
            ReqNtid[0] > 0 ? (ReqNtid[0], Math.Max(1, ReqNtid[1]), Math.Max(1, ReqNtid[2])) :
            MaxThreads[0] > 0 ? (MaxThreads[0], Math.Max(1, MaxThreads[1]), Math.Max(1, MaxThreads[2])) : (128, 1, 1);
    }
}
