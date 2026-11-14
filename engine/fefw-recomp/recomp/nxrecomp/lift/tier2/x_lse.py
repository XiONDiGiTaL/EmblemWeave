# SPDX-License-Identifier: GPL-3.0-or-later
"""Tier-2 handlers for one instruction family (lse). A method named i_<mnemonic> here OVERRIDES the same-named handler of
Wonder's ported mixins (this class comes first in Tier2's base list), so a faulty ported handler is fixed by redefining it
here, and a missing one is added here. Same-named helpers for the generated C++ go in include/nxrecomp/cxx_lse.h.
Handler signature: i_<mnemonic>(self, f, i, ops) -> one C++ statement (a string); raise Unhandled('why') when a form is
not supported. i is the Capstone instruction (i.address, i.mnemonic, i.op_str, i.operands, i.writeback), ops = i.operands."""
from capstone import arm64_const as A

from .base import B, VAS, Reg, Unhandled, subst, u, s  # noqa: F401


class XLse:
    pass
