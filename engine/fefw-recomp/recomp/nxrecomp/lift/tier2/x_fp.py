# SPDX-License-Identifier: GPL-3.0-or-later
"""Tier-2 handlers for one instruction family (fp). A method named i_<mnemonic> here OVERRIDES the same-named handler of
Wonder's ported mixins (this class comes first in Tier2's base list), so a faulty ported handler is fixed by redefining it
here, and a missing one is added here. Same-named helpers for the generated C++ go in include/nxrecomp/cxx_fp.h.
Handler signature: i_<mnemonic>(self, f, i, ops) -> one C++ statement (a string); raise Unhandled('why') when a form is
not supported. i is the Capstone instruction (i.address, i.mnemonic, i.op_str, i.operands, i.writeback), ops = i.operands."""
from capstone import arm64_const as A  # noqa: F401

from .base import B, VAS, Reg, Unhandled, subst, u, s  # noqa: F401


class XFp:
    def i_fmulx(self, f, i, ops):
        return self.fbin(i, ops, "nx::fpm_mulx(a, b)")

    def sign_op(self, i, ops, neg):
        """FNEG / FABS change only the sign bit: no rounding, and a NaN is not quietened. The half-precision forms (FEAT_FP16:
        v.4h, v.8h, h) are written on the integer lanes, because V128 has no half-precision view and the ported scalar path goes
        through f16_to_f32, which quietens a signalling NaN (debug note 010). Single and double precision keep the ported path."""
        d, n = self.regop(i, ops[0]), self.regop(i, ops[1])
        if self.is_vec(i, ops[0]):
            bits, lanes = self.vinfo(ops[0])
        else:
            bits, lanes = (16, 1) if d.kind == "h" else (0, 0)
        if bits != 16:
            return self.fun(i, ops, "-a" if neg else "std::fabs(a)")
        e = "S.u16[k] ^ 0x8000u" if neg else "S.u16[k] & 0x7fffu"
        return (f"{{ V128 S = c->v[{n.n}]; V128 r = nx::vzero(); for (int k = 0; k < {lanes}; k++) "
                f"r.u16[k] = uint16_t({e}); c->v[{d.n}] = r; }}")

    def i_fneg(self, f, i, ops):
        return self.sign_op(i, ops, True)

    def i_fabs(self, f, i, ops):
        return self.sign_op(i, ops, False)

    def i_frecpx(self, f, i, ops):
        d, n = self.regop(i, ops[0]), self.regop(i, ops[1])
        if d.kind not in ("s", "d"):
            raise Unhandled(f"frecpx {d.name}")
        ty = "float" if d.kind == "s" else "double"
        return self.fw(d, f"nx::fpm_recpx<{ty}>({self.fr(n)})")

    def fp_reduce(self, i, ops, fn):
        """FMAXV / FMINV (only the S, V.4S form exists without FP16): fold the four lanes in order."""
        d, a = self.regop(i, ops[0]), self.regop(i, ops[1])
        bits, n = self.vinfo(ops[1])
        if d.kind != "s" or bits != 32 or n != 4:
            raise Unhandled(f"{i.mnemonic} form")
        return (f"{{ float s_ = c->v[{a.n}].f32[0]; for (int k = 1; k < 4; k++) "
                f"s_ = nx::{fn}(s_, c->v[{a.n}].f32[k]); V128 r = nx::vzero(); r.f32[0] = s_; c->v[{d.n}] = r; }}")

    def i_fmaxv(self, f, i, ops):
        return self.fp_reduce(i, ops, "fpm_max")

    def i_fminv(self, f, i, ops):
        return self.fp_reduce(i, ops, "fpm_min")

    def fcvtxn_any(self, i, ops):
        """FCVTXN (scalar s, d or vector v.2s, v.2d) and FCVTXN2 (upper half of v.4s, v.2d; the lower half is kept)."""
        d, a = self.regop(i, ops[0]), self.regop(i, ops[1])
        if not ops[0].vas:
            if d.kind != "s" or self.regop(i, ops[1]).kind != "d":
                raise Unhandled("fcvtxn scalar form")
            return self.fw(d, f"nx::fpm_fcvtxn({self.fr(a)})")
        bits, n = self.vinfo(ops[0])
        hi = i.mnemonic.endswith("2")
        base = n // 2 if hi else 0
        if bits != 32 or n != (4 if hi else 2):
            raise Unhandled("fcvtxn vector form")
        init = f"c->v[{d.n}]" if hi else "nx::vzero()"
        return (f"{{ V128 S = c->v[{a.n}]; V128 r = {init}; for (int k = 0; k < 2; k++) "
                f"r.f32[{base} + k] = nx::fpm_fcvtxn(S.f64[k]); c->v[{d.n}] = r; }}")

    def i_fcvtxn(self, f, i, ops):
        return self.fcvtxn_any(i, ops)

    def i_fcvtxn2(self, f, i, ops):
        return self.fcvtxn_any(i, ops)
