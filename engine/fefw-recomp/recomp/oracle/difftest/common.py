# SPDX-License-Identifier: GPL-3.0-or-later
"""Shared pieces of the instruction-level oracle (gen_corpus.py, difftest.py, mutation.py).

Adapted from smo-recomp tools/lifter/difftest.py (GPL-3.0-or-later): the test region layout, the pseudo-random generator
shared with the host side, the Unicorn reference run, shape names. New: the corpus is generated (not sampled from a game
binary), initial states include integer edge values, and Unicorn can run as a Cortex-A57 or as the `max` model.
"""
import os
import re
import struct
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
sys.path.insert(0, ROOT)

REGION, REGION_SIZE = 0x7200000000, 0x400000
CODE, DATA = REGION + 0x200000, REGION + 0x300000
WIN_LO, WIN_HI = DATA - 0x1000, DATA + 0x12000
M64 = (1 << 64) - 1

EDGES = [0, 1, M64, 1 << 63, (1 << 63) - 1, 0x80000000, 0x7FFFFFFF, 0xFFFFFFFF, 0x5555555555555555, 0xAAAAAAAAAAAAAAAA,
         8, 16, 32, 63, 64, 127, 128, 0xFF, 0x80, 0xFFFF, 0x8000, 0x100000000, 0xFFFFFFFF00000000, 2, M64 - 1]

# special floating-point operands (bit patterns): zeros, ones, halves, ties, infinities, quiet / signalling NaNs with payloads,
# denormals, the largest / smallest normals, integer-conversion boundaries
SPECIAL = {
    32: [0x00000000, 0x80000000, 0x3f800000, 0xbf800000, 0x40200000, 0xc0200000, 0x7f800000, 0xff800000, 0x7fc00000,
         0xffc00000, 0x7fc12345, 0x7f800001, 0xffa00000, 0x00000001, 0x007fffff, 0x80000400, 0x00800000, 0x7f7fffff,
         0x477fe000, 0x477ff000, 0x477fefff, 0x33800000, 0x33000000, 0x33000001, 0x38800000, 0x387fffff, 0x3f801000,
         0x3f803000, 0x3f801001, 0x4f000000, 0x4effffff, 0xcf000000, 0x5f000000, 0x3f000000, 0x3fc00000, 0x4b800000, 0x4b7fffff, 0x5effffff],
    64: [0x0000000000000000, 0x8000000000000000, 0x3ff0000000000000, 0xbff0000000000000, 0x4004000000000000,
         0xc004000000000000, 0x7ff0000000000000, 0xfff0000000000000, 0x7ff8000000000000, 0xfff8000000000000,
         0x7ff8000000012345, 0x7ff0000000000001, 0xfff4000000000000, 0x0000000000000001, 0x000fffffffffffff,
         0x8000000000000400, 0x0010000000000000, 0x7fefffffffffffff, 0x40effc0000000000, 0x40effe0000000000,
         0x40effdffffffffff, 0x3e70000000000000, 0x3e60000000000000, 0x3e60000000000001, 0x3f10000000000000,
         0x3f0fffffffffffff, 0x3ff0020000000000, 0x3ff0060000000000, 0x3ff0020000001000, 0x41e0000000000000,
         0x41dfffffffc00000, 0xc1e0000000000000, 0x43e0000000000000, 0x3fe0000000000000, 0x3ff8000000000000,
         0x41dfffffffa00000, 0x41efffffffd00000, 0x43dfffffffffffff, 0x43efffffffffffff, 0x41dffffffff00000],
    16: [0x0000, 0x8000, 0x3c00, 0xbc00, 0x7c00, 0xfc00, 0x7e00, 0xfe00, 0x7e01, 0x7c01, 0xfd55, 0x0001, 0x03ff, 0x8200,
         0x0400, 0x7bff, 0x3555, 0xc123],
}


TINY = [0, 1, 0x80000000, 0xFFFFFFFF, 0x8000000000000000, M64]


class Rng:
    def __init__(self, seed):
        self.s = (seed * 0x9E3779B97F4A7C15 + 0x1234567) & M64

    def next(self):
        self.s = (self.s + 0x9E3779B97F4A7C15) & M64
        z = self.s
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M64
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M64
        return z ^ (z >> 31)


def pattern():
    r = Rng(0xABCDEF)
    return b''.join(struct.pack('<Q', r.next()) for _ in range(REGION_SIZE // 8))


def initial_state(idx, mem, rn, rm, style, rd=-1):
    """(x[0..31] (x[31] = SP), v[0..31] as 16 bytes, NZCV, TPIDR_EL0, TPIDRRO_EL0). The host (host.cpp) makes the same draws in
    the same order. style 0: random registers, 1: integer edge values (also for the vector registers' integer lanes), 2: every
    general register one of TINY (so INT_MIN / -1, 0 / 0 ... collide), 3..18: byte sweep k = style - 3 (v[rd] = 16k + lane, v[rn] = 0),
    19..34: byte sweep with the source in v[rn] (v[rd] = 0): together every byte value 0..255 reaches every lane."""
    r = Rng(idx)
    x = [r.next() for _ in range(31)] + [DATA + 0x800]
    v = []
    for _ in range(32):
        lanes = [(r.next() % 200001 - 100000) / 64.0 for _ in range(4)]
        v.append(struct.pack('<4f', *lanes))
    flags = r.next() & 0xf
    if style == 1:
        for i in range(31):
            if r.next() % 4:
                x[i] = EDGES[r.next() % len(EDGES)]
        for i in range(32):
            if r.next() % 2:
                v[i] = struct.pack('<QQ', EDGES[r.next() % len(EDGES)], EDGES[r.next() % len(EDGES)])
    if style == 2:
        for i in range(31):
            x[i] = TINY[r.next() % len(TINY)]
    if style >= 3 and rd >= 0:
        k = (style - 3) % 16
        sweep = bytes((16 * k + l) & 0xff for l in range(16))
        if style < 19:
            v[rd] = sweep
            if rn != rd:
                v[rn] = bytes(16)
        else:
            v[rn] = sweep
            if rn != rd:
                v[rd] = bytes(16)
    if mem:
        base = DATA + 0x100 * (r.next() % 8)
        index = r.next() % 0x40
        if rn != 31:
            x[rn] = base
        if 0 <= rm < 31 and rm != rn:
            x[rm] = index
    tp0, tp1 = r.next(), r.next()
    return x, v, flags, tp0, tp1


def fp_special_v(test):
    """The 32 vector registers filled with special operands for a floating-point test (same rule on the host side)."""
    mode, k = test['fpmode'], test['k']
    table = SPECIAL[mode]
    n = len(table)
    ia, ib = k % n, (k // n) % n
    v = []
    for r in range(32):
        b = ib if r == test['frm'] else ia if r == test['rn'] else (k + r) % n
        lanes = [table[(b + 7 * l) % n] for l in range(128 // mode)]
        v.append(b''.join(x.to_bytes(mode // 8, 'little') for x in lanes))
    return v


def make_uc(model='max'):
    from unicorn import Uc, UC_ARCH_ARM64, UC_MODE_ARM
    from unicorn import arm64_const as A
    cpu = {'max': A.UC_CPU_ARM64_MAX, 'a57': A.UC_CPU_ARM64_A57}[model]
    uc = Uc(UC_ARCH_ARM64, UC_MODE_ARM, cpu=cpu)
    uc.mem_map(REGION, REGION_SIZE)
    uc.mem_write(REGION, pattern())
    uc.reg_write(A.UC_ARM64_REG_CPACR_EL1, 0x300000)
    xr = [getattr(A, 'UC_ARM64_REG_X%d' % i) for i in range(29)] + [A.UC_ARM64_REG_X29, A.UC_ARM64_REG_X30]
    vr = [getattr(A, 'UC_ARM64_REG_V%d' % i) for i in range(32)]
    return uc, A, xr, vr


def md():
    from capstone import Cs, CS_ARCH_ARM64, CS_MODE_ARM
    m = Cs(CS_ARCH_ARM64, CS_MODE_ARM)
    m.detail = True
    return m


def shape_of(i0):
    """mnemonic + operands with registers reduced to their class letter and numbers (immediates, lane indices) to '#'."""
    s = re.sub(r'#-?(0x)?[0-9a-f.e+-]+', '#', i0.op_str)
    s = re.sub(r'\b([vqdshbwxzp])\d+\b', r'\1', s)
    s = re.sub(r'\[\d+\]', '[#]', s)
    return i0.mnemonic + ' ' + s


def crc(b):
    return zlib.crc32(b)


def op0(w):
    """Bits 28:25, the top-level A64 encoding group."""
    return (w >> 25) & 0xf


def is_mem(w):
    """Loads and stores."""
    return op0(w) in (4, 6, 12, 14)
