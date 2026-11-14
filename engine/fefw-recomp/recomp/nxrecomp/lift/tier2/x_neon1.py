# SPDX-License-Identifier: GPL-3.0-or-later
"""Tier-2 handlers for one instruction family (neon1). A method named i_<mnemonic> here OVERRIDES the same-named handler of
Wonder's ported mixins (this class comes first in Tier2's base list), so a faulty ported handler is fixed by redefining it
here, and a missing one is added here. Same-named helpers for the generated C++ go in include/nxrecomp/cxx_neon1.h.
Handler signature: i_<mnemonic>(self, f, i, ops) -> one C++ statement (a string); raise Unhandled('why') when a form is
not supported. i is the Capstone instruction (i.address, i.mnemonic, i.op_str, i.operands, i.writeback), ops = i.operands.

Family neon1: AdvSIMD integer narrowing (high half), saturating doubling multiply long, saturating / rounding register and
immediate shifts, shift-and-insert, rounding shift right and accumulate. Tier 1 (a64.py) owns shl, sshr, ushr, shrn,
sqshrn, uqshrn, sqshrun, xtn, sshll, ushll and sqxtn-style shapes it decodes; those never reach this class."""
from capstone import arm64_const as A

from .base import B, VAS, Reg, Unhandled, subst, u, s  # noqa: F401


class XNeon1:
    # ---- helpers --------------------------------------------------------------------------------------------------

    def n1_vec(self, i, ops, make, nsrc):
        """Lanewise op over the destination arrangement (or one scalar lane). make(bits, s, o) -> C++ lane expression,
        where s[k] is source k's lane and o the destination's current lane (the accumulator)."""
        bits, n = self.vinfo(ops[0])
        d = self.regop(i, ops[0])
        srcs = [self.regop(i, o) for o in ops[1:1 + nsrc]]
        pre = "".join(f"V128 S{k} = c->v[{r.n}]; " for k, r in enumerate(srcs))
        e = make(bits, [f"S{k}.u{bits}[k]" for k in range(nsrc)], f"O.u{bits}[k]")
        return (f"{{ {pre}V128 O = c->v[{d.n}]; V128 r = nx::vzero(); for (int k = 0; k < {n}; k++) "
                f"r.u{bits}[k] = uint{bits}_t({e}); c->v[{d.n}] = r; }}")

    def n1_narrow2(self, i, ops, call):
        """Two wide sources, narrow destination (ADDHN and friends). `2` forms write the upper half and keep the lower.
        call(a, b, dbits) -> C++ expression of the destination lane (uint64)."""
        dbits, n = self.vinfo(ops[0])
        hi = i.mnemonic.endswith("2")
        sbits = 2 * dbits
        d, a, b = self.regop(i, ops[0]), self.regop(i, ops[1]), self.regop(i, ops[2])
        cnt = n // 2 if hi else n
        base = n // 2 if hi else 0
        init = f"c->v[{d.n}]" if hi else "nx::vzero()"
        e = call(f"A_.u{sbits}[k]", f"B_.u{sbits}[k]", dbits)
        return (f"{{ V128 A_ = c->v[{a.n}], B_ = c->v[{b.n}]; V128 r = {init}; for (int k = 0; k < {cnt}; k++) "
                f"r.u{dbits}[{base} + k] = uint{dbits}_t({e}); c->v[{d.n}] = r; }}")

    def n1_dmull(self, i, ops, mode):
        """SQDMULL (mode 0), SQDMLAL (1), SQDMLSL (2): vector, `2` (upper half of both sources), by-element and scalar."""
        dbits, n = self.vinfo(ops[0])
        sbits = dbits // 2
        off = n if i.mnemonic.endswith("2") else 0
        d, a = self.regop(i, ops[0]), self.regop(i, ops[1])
        if ops[2].vector_index != -1:
            b_pre = ""
            bexpr = f"c->v[{self.regop(i, ops[2]).n}].u{sbits}[{ops[2].vector_index}]"
        else:
            b_pre = f"V128 B_ = c->v[{self.regop(i, ops[2]).n}]; "
            bexpr = f"B_.u{sbits}[k + {off}]"
        e = f"nx::n1_dmul<{sbits}>(A_.u{sbits}[k + {off}], {bexpr}, O.u{dbits}[k], {mode})"
        return (f"{{ V128 A_ = c->v[{a.n}]; {b_pre}V128 O = c->v[{d.n}]; V128 r = nx::vzero(); "
                f"for (int k = 0; k < {n}; k++) r.u{dbits}[k] = uint{dbits}_t({e}); c->v[{d.n}] = r; }}")

    # ---- high-half narrowing: ADDHN / SUBHN / RADDHN / RSUBHN ----------------------------------------------------

    def i_addhn(self, f, i, ops):
        return self.n1_narrow2(i, ops, lambda a, b, db: f"nx::n1_hn<{db}>({a}, {b}, false, false)")

    i_addhn2 = i_addhn

    def i_subhn(self, f, i, ops):
        return self.n1_narrow2(i, ops, lambda a, b, db: f"nx::n1_hn<{db}>({a}, {b}, true, false)")

    i_subhn2 = i_subhn

    def i_raddhn(self, f, i, ops):
        return self.n1_narrow2(i, ops, lambda a, b, db: f"nx::n1_hn<{db}>({a}, {b}, false, true)")

    i_raddhn2 = i_raddhn

    def i_rsubhn(self, f, i, ops):
        return self.n1_narrow2(i, ops, lambda a, b, db: f"nx::n1_hn<{db}>({a}, {b}, true, true)")

    i_rsubhn2 = i_rsubhn

    # ---- saturating doubling multiply long -----------------------------------------------------------------------

    def i_sqdmull(self, f, i, ops):
        return self.n1_dmull(i, ops, 0)

    i_sqdmull2 = i_sqdmull

    def i_sqdmlal(self, f, i, ops):
        return self.n1_dmull(i, ops, 1)

    i_sqdmlal2 = i_sqdmlal

    def i_sqdmlsl(self, f, i, ops):
        return self.n1_dmull(i, ops, 2)

    i_sqdmlsl2 = i_sqdmlsl

    # ---- saturating / plain shifts by register or immediate ------------------------------------------------------

    def i_sqshl(self, f, i, ops):
        if ops[2].type == A.ARM64_OP_IMM:
            n = ops[2].imm
            return self.n1_vec(i, ops, lambda b, s_, o: f"nx::n1_qshl_imm<{b}, true>({s_[0]}, {n})", 1)
        return self.n1_vec(i, ops, lambda b, s_, o: f"nx::n1_shl_reg<{b}, true, true, false>({s_[0]}, {s_[1]})", 2)

    def i_uqshl(self, f, i, ops):
        if ops[2].type == A.ARM64_OP_IMM:
            n = ops[2].imm
            return self.n1_vec(i, ops, lambda b, s_, o: f"nx::n1_qshl_imm<{b}, false>({s_[0]}, {n})", 1)
        return self.n1_vec(i, ops, lambda b, s_, o: f"nx::n1_shl_reg<{b}, false, true, false>({s_[0]}, {s_[1]})", 2)

    def i_sqshlu(self, f, i, ops):
        if ops[2].type != A.ARM64_OP_IMM:
            raise Unhandled("sqshlu has only an immediate form")
        n = ops[2].imm
        return self.n1_vec(i, ops, lambda b, s_, o: f"nx::n1_qshlu_imm<{b}>({s_[0]}, {n})", 1)

    def i_sqrshl(self, f, i, ops):
        if ops[2].type == A.ARM64_OP_IMM:
            raise Unhandled("sqrshl has no immediate form")
        return self.n1_vec(i, ops, lambda b, s_, o: f"nx::n1_shl_reg<{b}, true, true, true>({s_[0]}, {s_[1]})", 2)

    def i_uqrshl(self, f, i, ops):
        if ops[2].type == A.ARM64_OP_IMM:
            raise Unhandled("uqrshl has no immediate form")
        return self.n1_vec(i, ops, lambda b, s_, o: f"nx::n1_shl_reg<{b}, false, true, true>({s_[0]}, {s_[1]})", 2)

    def i_urshl(self, f, i, ops):
        if ops[2].type == A.ARM64_OP_IMM:
            raise Unhandled("urshl has no immediate form")
        return self.n1_vec(i, ops, lambda b, s_, o: f"nx::n1_shl_reg<{b}, false, false, true>({s_[0]}, {s_[1]})", 2)

    def i_srshl(self, f, i, ops):
        if ops[2].type == A.ARM64_OP_IMM:
            raise Unhandled("srshl has no immediate form")
        return self.n1_vec(i, ops, lambda b, s_, o: f"nx::n1_shl_reg<{b}, true, false, true>({s_[0]}, {s_[1]})", 2)

    # ---- shift and insert ----------------------------------------------------------------------------------------

    def i_sli(self, f, i, ops):
        n = ops[2].imm
        return self.n1_vec(i, ops, lambda b, s_, o: f"nx::n1_sli<{b}>({s_[0]}, {o}, {n})", 1)

    def i_sri(self, f, i, ops):
        n = ops[2].imm
        return self.n1_vec(i, ops, lambda b, s_, o: f"nx::n1_sri<{b}>({s_[0]}, {o}, {n})", 1)

    # ---- rounding and plain shifts right, with accumulate --------------------------------------------------------

    def i_srshr(self, f, i, ops):
        n = ops[2].imm
        return self.n1_vec(i, ops, lambda b, s_, o: f"nx::n1_rshr<{b}, true>({s_[0]}, {n})", 1)

    def i_urshr(self, f, i, ops):
        n = ops[2].imm
        return self.n1_vec(i, ops, lambda b, s_, o: f"nx::n1_rshr<{b}, false>({s_[0]}, {n})", 1)

    def i_srsra(self, f, i, ops):
        n = ops[2].imm
        return self.n1_vec(i, ops, lambda b, s_, o: f"{o} + nx::n1_rshr<{b}, true>({s_[0]}, {n})", 1)

    def i_ursra(self, f, i, ops):
        n = ops[2].imm
        return self.n1_vec(i, ops, lambda b, s_, o: f"{o} + nx::n1_rshr<{b}, false>({s_[0]}, {n})", 1)

    def i_ssra(self, f, i, ops):
        n = ops[2].imm
        return self.n1_vec(i, ops, lambda b, s_, o: f"{o} + nx::n1_shr<{b}, true>({s_[0]}, {n})", 1)

    def i_usra(self, f, i, ops):
        n = ops[2].imm
        return self.n1_vec(i, ops, lambda b, s_, o: f"{o} + nx::n1_shr<{b}, false>({s_[0]}, {n})", 1)

    # ---- rounding and saturating narrowing shifts (the plain ones are tier 1) ------------------------------------

    def i_sqrshrn(self, f, i, ops):
        db = self.vinfo(ops[0])[0]
        return self.narrow(i, ops, f"nx::n1_sat_s<{db}>(nx::n1_wide_shr<{2 * db}, true, true>({{a}}, {{n}}))",
                           src_kind="s", dst_kind="s")

    i_sqrshrn2 = i_sqrshrn

    def i_uqrshrn(self, f, i, ops):
        db = self.vinfo(ops[0])[0]
        return self.narrow(i, ops, f"nx::n1_sat_u<{db}>(nx::n1_wide_shr<{2 * db}, false, true>({{a}}, {{n}}))",
                           src_kind="u", dst_kind="u")

    i_uqrshrn2 = i_uqrshrn

    def i_sqrshrun(self, f, i, ops):
        db = self.vinfo(ops[0])[0]
        return self.narrow(i, ops, f"nx::n1_sat_u<{db}>(nx::n1_wide_shr<{2 * db}, true, true>({{a}}, {{n}}))",
                           src_kind="s", dst_kind="u")

    i_sqrshrun2 = i_sqrshrun
