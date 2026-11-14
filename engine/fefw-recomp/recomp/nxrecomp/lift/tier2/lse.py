# SPDX-License-Identifier: GPL-3.0-or-later
# Ported from SuperMarioWonderRecomp recomp/emit_ext.py (MIT License, Copyright (c) 2026 BelmanteGu; see
# recomp/THIRD_PARTY.md): the ARMv8.1 LSE atomics (ldadd/ldclr/ldeor/ldset/ldsmax/ldsmin/ldumax/ldumin, st* forms, swp, cas,
# casp). Split into its own file so one owner can work on it.
import re

from capstone import arm64_const as A

from .base import VAS, Reg, Unhandled, subst, u


class LseOps:
    # LSE atomics (ARMv8.1): ld<op>/st<op>, swp, cas, casp with a/l/al ordering and b/h sizes. Every ordering is
    # emitted as seq_cst, which is at least as strong.
    LSE = re.compile(r"^(ldadd|ldclr|ldeor|ldset|ldsmax|ldsmin|ldumax|ldumin|stadd|stclr|steor|stset|stsmax|stsmin|"
                     r"stumax|stumin|swp|casp|cas)(al|a|l)?(b|h)?$")
    LSE_OPS = {"add": "__atomic_fetch_add(p_, v_, __ATOMIC_SEQ_CST)", "clr": "__atomic_fetch_and(p_, T_(~v_), __ATOMIC_SEQ_CST)",
               "eor": "__atomic_fetch_xor(p_, v_, __ATOMIC_SEQ_CST)", "set": "__atomic_fetch_or(p_, v_, __ATOMIC_SEQ_CST)",
               "smax": "nx::amo_minmax<T_, true, true>(p_, v_)", "smin": "nx::amo_minmax<T_, true, false>(p_, v_)",
               "umax": "nx::amo_minmax<T_, false, true>(p_, v_)", "umin": "nx::amo_minmax<T_, false, false>(p_, v_)"}

    def lse(self, i, ops):
        m = self.LSE.match(i.mnemonic)
        if not m:
            raise Unhandled("lse form")
        base, size = m.group(1), m.group(3)
        regs, mem, _ = self.mem_ops(ops)
        rs = [self.regop(i, o) for o in regs]
        addr = f"c->x[{Reg(i.reg_name(mem.mem.base)).n}]"
        bits = {"b": 8, "h": 16}.get(size, rs[0].bits)
        t = u(bits)
        if base == "casp":  # casp x0, x1, x2, x3, [x4]: compare (x0, x1), store (x2, x3), old value to (x0, x1)
            s0, s1, t0, t1 = rs
            if bits == 32:
                return (f"{{ uint64_t e_ = uint64_t(uint32_t({s0.r()})) | uint64_t(uint32_t({s1.r()})) << 32; "
                        f"uint64_t n_ = uint64_t(uint32_t({t0.r()})) | uint64_t(uint32_t({t1.r()})) << 32; "
                        f"__atomic_compare_exchange_n(reinterpret_cast<uint64_t*>({addr}), &e_, n_, false, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST); "
                        f"{s0.w('uint32_t(e_)')} {s1.w('uint32_t(e_ >> 32)')} }}")
            return (f"{{ unsigned __int128 e_ = (unsigned __int128){s1.r()} << 64 | {s0.r()}; "
                    f"unsigned __int128 n_ = (unsigned __int128){t1.r()} << 64 | {t0.r()}; "
                    f"__atomic_compare_exchange_n(reinterpret_cast<unsigned __int128*>({addr}), &e_, n_, false, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST); "
                    f"{s0.w('uint64_t(e_)')} {s1.w('uint64_t(e_ >> 64)')} }}")
        if base == "cas":  # cas ws, wt, [xn]: compare ws, store wt, old value to ws
            rs_, rt = rs
            return (f"{{ {t} e_ = {t}({rs_.r()}); __atomic_compare_exchange_n(reinterpret_cast<{t}*>({addr}), &e_, {t}({rt.r()}), false, "
                    f"__ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST); {rs_.w('e_')} }}")
        if base == "swp":
            rs_, rt = rs
            return f"{{ {t} o_ = __atomic_exchange_n(reinterpret_cast<{t}*>({addr}), {t}({rs_.r()}), __ATOMIC_SEQ_CST); {rt.w('o_')} }}"
        op = base[2:]
        rs_ = rs[0]
        rt = rs[1] if len(rs) > 1 and base.startswith("ld") else None
        stmt = f"{{ using T_ = {t}; T_* p_ = reinterpret_cast<T_*>({addr}); T_ v_ = T_({rs_.r()}); T_ o_ = {self.LSE_OPS[op]}; "
        return stmt + (rt.w("o_") if rt else "(void)o_;") + " }"

