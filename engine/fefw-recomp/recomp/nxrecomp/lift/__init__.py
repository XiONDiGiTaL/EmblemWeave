# SPDX-License-Identifier: GPL-3.0-or-later
"""The instruction lifter: one 32-bit word -> one statement.

    tier 1  lift/a64.py     bit-field decoder (smo-recomp's), the reference for everything it covers
    tier 2  lift/tier2.py   Capstone-driven emitters (SuperMarioWonderRecomp's, MIT), used when tier 1 raises Unimpl

Lifter.lift(w, pc) tries tier 1, then tier 2, and raises Unimpl when neither knows the instruction.
"""
from .a64 import Fn, Unimpl, bits, sext

try:
    from .tier2 import Tier2
except ImportError:                      # tier 2 not available (no Capstone)
    Tier2 = None


class Lifter(Fn):
    """Fn with tier-2 fallback. `env` supplies call(), jump tables, etc. (see program.Env)."""

    def __init__(self, entry, end, env):
        Fn.__init__(self, entry, end, env)
        self._t2 = None

    def lift(self, w, pc):
        try:
            return Fn.lift(self, w, pc)
        except Unimpl as e1:
            if Tier2 is None:
                raise
            if self._t2 is None:
                self._t2 = Tier2.shared()
            try:
                return self._t2.lift(self, w, pc)
            except Unimpl as e2:
                raise Unimpl(str(e2) if str(e2) else str(e1))
