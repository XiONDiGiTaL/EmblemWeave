# SPDX-License-Identifier: GPL-3.0-or-later
"""Tier 2 of the instruction lifter: Capstone-driven emitters, for what tier 1 (lift/a64.py) does not decode.

The emitters are SuperMarioWonderRecomp's (MIT; see recomp/THIRD_PARTY.md), ported to nxrecomp's context and helpers
(include/nxrecomp/cxx.h), and brought to the project's exactness standard by the differential test (oracle/difftest):
what Wonder's own test accepted (NaN payloads "both NaN", estimates "more precise") is not accepted here.

Tier2.lift(fn, w, pc) -> one statement for the instruction word w at address pc, or raises Unimpl. Branches are never seen
here: tier 1 handles every branch.
"""
import re
import struct

import capstone

from ..a64 import Unimpl
from .base import Unhandled
from .ext import ExtOps
from .fp import FpOps
from .int import IntOps
from .lse import LseOps
from .misc import MiscOps
from .neon import NeonOps
from .neon_wide import NeonWideOps
from .vldst import VecLdSt
from .x_fp import XFp
from .x_ldst import XLdst
from .x_lse import XLse
from .x_neon1 import XNeon1
from .x_neon2 import XNeon2
from .x_pac import XPac


class Tier2(XFp, XLdst, XLse, XPac, XNeon1, XNeon2, LseOps, ExtOps, VecLdSt, IntOps, FpOps, NeonOps, NeonWideOps, MiscOps):
    _shared = None
    # The ported FP lane emitters name the V128 view after the element size; V128 (ctx.h) has no half-precision view, so a
    # FEAT_FP16 vector form they do not special-case would not compile. Such a form is reported as not translated (debug note 010).
    NO_VIEW = re.compile(r"\.f16\[")

    @classmethod
    def shared(cls):
        if cls._shared is None:
            cls._shared = cls()
        return cls._shared

    def __init__(self):
        self.cs = capstone.Cs(capstone.CS_ARCH_ARM64, capstone.CS_MODE_ARM)
        self.cs.detail = True

    def lift(self, f, w, pc):
        got = list(self.cs.disasm(struct.pack('<I', w), pc))
        if not got:
            raise Unimpl('undecodable')
        i = got[0]
        ops = i.operands
        mn = i.mnemonic
        h = getattr(self, 'i_' + mn.replace('.', '_'), None)
        try:
            if h:
                out = h(f, i, ops)
                if self.NO_VIEW.search(out):
                    raise Unhandled("half-precision lanes")
                return out
            if self.LSE.match(mn):
                return self.lse(i, ops)
        except Unhandled as e:
            raise Unimpl('t2:%s:%s' % (mn, e))
        except (IndexError, KeyError, AttributeError, ValueError, TypeError) as e:
            raise Unimpl('t2:%s:decode-%s' % (mn, type(e).__name__))
        raise Unimpl('t2:%s' % mn)
