# SPDX-License-Identifier: GPL-3.0-or-later
"""Tier-2 handlers for pointer authentication (register forms) and the MTE tag stores. A method named i_<mnemonic> here
OVERRIDES the same-named handler of the ported mixins (this class comes first in Tier2's base list).

Pointer authentication is not modelled (the console's CPU has none): PAC* sign nothing and AUT* never fail, so both leave
the pointer unchanged, and XPAC* has no code to strip. This is the register-form counterpart of the hint-space PACIASP /
AUTIASP / XPACLRI (no-ops in tier 1) and of RETAA / RETAB (lifted as RET). PACGA yields 0 (the generic code is never
checked, so any value does).

MTE is not modelled: a tag store changes no data; STZG / STZ2G zero the 16 / 32 bytes of the granule(s) at the address
(rounded down to 16) and ST2G / STZ2G cover two granules. The pre- and post-indexed forms write the base register back."""
from capstone import arm64_const as A

from .base import Reg, Unhandled


class XPac:
    def i_pac_identity(self, f, i, ops):
        return ";"

    i_pacia = i_pacib = i_pacda = i_pacdb = i_pac_identity
    i_paciza = i_pacizb = i_pacdza = i_pacdzb = i_pac_identity
    i_autia = i_autib = i_autda = i_autdb = i_pac_identity
    i_autiza = i_autizb = i_autdza = i_autdzb = i_pac_identity
    i_xpaci = i_xpacd = i_pac_identity

    def i_pacga(self, f, i, ops):
        return self.regop(i, ops[0]).w("0ull")

    def tag_store(self, i, ops, nbytes, zero):
        regs, mem, after = self.mem_ops(ops)
        if mem is None or mem.mem.index:
            raise Unhandled("address form")
        a, wb = self.address(i, mem, after)
        parts = [f"uint64_t a = {a};"]
        if zero:
            parts.append(f"std::memset(reinterpret_cast<void*>(a & ~15ull), 0, {nbytes});")
        parts.append(wb)
        return "{ " + " ".join(p for p in parts if p) + " }"

    def i_stg(self, f, i, ops):
        return self.tag_store(i, ops, 16, False)

    def i_st2g(self, f, i, ops):
        return self.tag_store(i, ops, 32, False)

    def i_stzg(self, f, i, ops):
        return self.tag_store(i, ops, 16, True)

    def i_stz2g(self, f, i, ops):
        return self.tag_store(i, ops, 32, True)
