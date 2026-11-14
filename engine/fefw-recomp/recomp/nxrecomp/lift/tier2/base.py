# SPDX-License-Identifier: GPL-3.0-or-later
# Ported from SuperMarioWonderRecomp recomp/emit.py (constants, Reg) (MIT License, Copyright (c) 2026 BelmanteGu; see
# recomp/THIRD_PARTY.md for the notice). Changes: the generated code uses nxrecomp's context (Ctx *c, c->x / c->v /
# c->N Z C V) and C++ helpers (nxrecomp/cxx.h); absolute guest addresses instead of module offsets.
"""AArch64 -> C++ instruction emitter.

Each guest function becomes `void sub_XXXXXXXX(nx::CPU& c)`. Registers live in the context; intra-function
branches are gotos, jump tables are switches over the computed target, calls are direct when the target is
known and go through nx::call otherwise. Instructions we do not handle become nx::unimpl traps and are counted.
"""
import collections
import re
import struct

import capstone
from capstone import arm64_const as A

B = "0ull"            # guest addresses are absolute here (capstone is given the real pc)

CC = {
    A.ARM64_CC_EQ: "c->Z", A.ARM64_CC_NE: "!c->Z", A.ARM64_CC_HS: "c->C", A.ARM64_CC_LO: "!c->C",
    A.ARM64_CC_MI: "c->N", A.ARM64_CC_PL: "!c->N", A.ARM64_CC_VS: "c->V", A.ARM64_CC_VC: "!c->V",
    A.ARM64_CC_HI: "(c->C && !c->Z)", A.ARM64_CC_LS: "(!c->C || c->Z)", A.ARM64_CC_GE: "(c->N == c->V)",
    A.ARM64_CC_LT: "(c->N != c->V)", A.ARM64_CC_GT: "(!c->Z && c->N == c->V)",
    A.ARM64_CC_LE: "(c->Z || c->N != c->V)", A.ARM64_CC_AL: "true", A.ARM64_CC_NV: "true",
}
INVERT = {A.ARM64_CC_EQ: A.ARM64_CC_NE, A.ARM64_CC_NE: A.ARM64_CC_EQ, A.ARM64_CC_HS: A.ARM64_CC_LO,
          A.ARM64_CC_LO: A.ARM64_CC_HS, A.ARM64_CC_MI: A.ARM64_CC_PL, A.ARM64_CC_PL: A.ARM64_CC_MI,
          A.ARM64_CC_VS: A.ARM64_CC_VC, A.ARM64_CC_VC: A.ARM64_CC_VS, A.ARM64_CC_HI: A.ARM64_CC_LS,
          A.ARM64_CC_LS: A.ARM64_CC_HI, A.ARM64_CC_GE: A.ARM64_CC_LT, A.ARM64_CC_LT: A.ARM64_CC_GE,
          A.ARM64_CC_GT: A.ARM64_CC_LE, A.ARM64_CC_LE: A.ARM64_CC_GT}

VAS = {  # arrangement -> (element bits, lanes)
    A.ARM64_VAS_16B: (8, 16), A.ARM64_VAS_8B: (8, 8), A.ARM64_VAS_8H: (16, 8), A.ARM64_VAS_4H: (16, 4),
    A.ARM64_VAS_4S: (32, 4), A.ARM64_VAS_2S: (32, 2), A.ARM64_VAS_2D: (64, 2), A.ARM64_VAS_1D: (64, 1),
    A.ARM64_VAS_1B: (8, 1), A.ARM64_VAS_1H: (16, 1), A.ARM64_VAS_1S: (32, 1), A.ARM64_VAS_1Q: (128, 1),
    A.ARM64_VAS_4B: (8, 4), A.ARM64_VAS_2H: (16, 2),
}

SYSREG_TPIDR = 0xDE82
SYSREG_TPIDRRO = 0xDE83
SYSREG_FPCR = 0xDA20
SYSREG_FPSR = 0xDA21
SYSREG_NZCV = 0xDA10


class Unhandled(Exception):
    pass


def subst(expr, **vals):
    """Replace the whole-word placeholders a / b / o in an expression template."""
    return re.sub(r"\b([abo])\b", lambda m: vals.get(m.group(1), m.group(1)), expr)


def u(bits):
    return f"uint{bits}_t"


def s(bits):
    return f"int{bits}_t"


class Reg:
    """A register operand: kind 'x' (64-bit gp), 'w' (32-bit gp), 'zr64'/'zr32', or fp/simd b/h/s/d/q/v."""

    def __init__(self, name):
        self.name = name
        if name is None:  # an operand Capstone decodes but cannot name
            raise Unhandled("register without a name")
        if name in ("sp",):
            self.kind, self.n = "x", 31
        elif name == "wsp":
            self.kind, self.n = "w", 31
        elif name == "xzr":
            self.kind, self.n = "zr64", None
        elif name == "wzr":
            self.kind, self.n = "zr32", None
        elif name == "fp":
            self.kind, self.n = "x", 29
        elif name == "lr":
            self.kind, self.n = "x", 30
        elif name[0] in "xwbhsdqv" and name[1:].isdigit():
            self.kind, self.n = name[0], int(name[1:])
        else:
            raise Unhandled(f"register {name}")

    @property
    def gp(self):
        return self.kind in ("x", "w", "zr64", "zr32")

    @property
    def bits(self):
        return {"x": 64, "zr64": 64, "w": 32, "zr32": 32, "b": 8, "h": 16, "s": 32, "d": 64, "q": 128,
                "v": 128}[self.kind]

    @property
    def zr(self):
        return self.kind.startswith("zr")

    def r(self):
        """Read as unsigned of its width."""
        if self.zr:
            return "0ull" if self.bits == 64 else "0u"
        if self.bits == 64:
            return f"c->x[{self.n}]"
        return f"uint32_t(c->x[{self.n}])"

    def w(self, expr):
        if self.zr:
            return f"(void)({expr});"
        if self.bits == 64:
            return f"c->x[{self.n}] = uint64_t({expr});"
        return f"c->x[{self.n}] = uint32_t({expr});"


