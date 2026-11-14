# SPDX-License-Identifier: GPL-3.0-or-later
"""Mutation test of the differential oracle: does it notice a lifter that is wrong?

    python mutation.py [--work DIR] [--only N,N,...]

Copies the lifter (nxrecomp/, include/) and the oracle to a scratch tree, plants the bugs listed in MUTATIONS (each one a
small, plausible mistake: an operator, a condition, a size, an ordering, a table entry) in the COPY, and runs the whole
differential test on the shapes each bug should break, twice: on the unmutated copy (baseline: it must pass) and with ALL
mutations planted at once. Passes only if

  * the baseline run has no difference in any of the mutation filters (the oracle is green on the correct lifter), and
  * for every mutation at least one test of its expected shapes is reported different (the planted bug is caught).

The real files are never modified. Adapted from smo-recomp tools/lifter/difftest_mutation.py (GPL-3.0-or-later), which
mutated the generated test code of four tests; here the bugs are planted in the lifter's own sources.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))

# (name, file, old text, new text, regex of the shape keys that must show a difference)
MUTATIONS = [
    ('add immediate: + becomes -', 'nxrecomp/lift/a64.py', "'%s %s %dULL' % (a, '-' if sub else '+', imm)", "'%s %s %dULL' % (a, '+' if sub else '-', imm)", r'^(add|sub) '),
    ('b.cond: condition inverted', 'nxrecomp/lift/a64.py', "return 'if (cond(c, %d)) %s' % (bits(w, 3, 0), self.jump(target))", "return 'if (!cond(c, %d)) %s' % (bits(w, 3, 0), self.jump(target))", r'^b\.'),
    ('cbz / cbnz swapped', 'nxrecomp/lift/a64.py', "return 'if (%s %s 0) %s' % (R(bits(w, 4, 0), bits(w, 31, 31)), '!=' if bits(w, 24, 24) else '==', self.jump(target))", "return 'if (%s %s 0) %s' % (R(bits(w, 4, 0), bits(w, 31, 31)), '==' if bits(w, 24, 24) else '!=', self.jump(target))", r'^cb'),
    ('tbz / tbnz swapped', 'nxrecomp/lift/a64.py', "return 'if (((%s >> %d) & 1) %s 0) %s' % (X(bits(w, 4, 0)), bit, '!=' if bits(w, 24, 24) else '==', self.jump(target))", "return 'if (((%s >> %d) & 1) %s 0) %s' % (X(bits(w, 4, 0)), bit, '==' if bits(w, 24, 24) else '!=', self.jump(target))", r'^tb'),
    ('undefined instruction does not trap', 'nxrecomp/lift/a64.py', "return 'guest_trap(\"undefined instruction\", 0x%xULL);' % pc", "return ';'", r'^trap '),
    ('csel / csinc operands swapped', 'nxrecomp/lift/a64.py', "return SET(rd, sf, 'cond(c, %d) ? %s : %s' % (cc, R(rn, sf), alt))", "return SET(rd, sf, 'cond(c, %d) ? %s : %s' % (cc, alt, R(rn, sf)))", r'^(csel|csinc|csinv|csneg|cset|cinc|cinv|cneg) '),
    ('adds: carry flag wrong', 'include/nxrecomp/ctx.h', "c->C = (r < a) || (cin && r == a);", "c->C = (r <= a) && (r != 0);", r'^(adds|subs|cmp|cmn|adcs|sbcs|negs)'),
    ('stxr succeeds without the monitor', 'include/nxrecomp/ctx.h', "if (c->ex_addr != a) { stx_diag(c, a, 0, 0, v, -bits); return 1; } \\", "if (0) { stx_diag(c, a, 0, 0, v, -bits); return 1; } \\", r'^excl .*(wrong address|without load)'),
    ('clrex does not close the monitor', 'nxrecomp/lift/a64.py', "return 'c->ex_addr = ~0ULL;'", "return ';'", r'^excl .* clrex'),
    ('mrs tpidrro reads tpidr', 'nxrecomp/lift/a64.py', "return SET(rt, 1, 'c->tpidrro')", "return SET(rt, 1, 'c->tpidr')", r'^system register d53bd0'),
    ('fcvtzs saturation bound', 'include/nxrecomp/ctx.h', "if (v >= 2147483647.0) return 0x7fffffffu;", "if (v >= 2147483646.0) return 0x7fffffffu;", r'^(fcvtzs|fcvtas|fcvtms|fcvtps|fcvtns) w'),
    ('fmax / fmin selection inverted', 'include/nxrecomp/fpexact.h', "    return (is_min ? fa < fb : fa > fb) ? a : b;\n}\nstatic inline uint64_t fp_maxmin64_bits", "    return (is_min ? fa > fb : fa < fb) ? a : b;\n}\nstatic inline uint64_t fp_maxmin64_bits", r'^(fmax|fmin|fmaxnm|fminnm) s'),
    ('NEON saturating add bound', 'include/nxrecomp/neon.h', "__int128 hi = sz == 3 ? (__int128)INT64_MAX : (((__int128)1 << ((8 << sz) - 1)) - 1), lo = -hi - 1;", "__int128 hi = sz == 3 ? (__int128)INT64_MAX : (((__int128)1 << ((8 << sz) - 1)) - 2), lo = -hi - 1;", r'^(sqadd|sqsub) v'),
    ('LSE ldadd becomes subtract', 'nxrecomp/lift/tier2/lse.py', '"add": "__atomic_fetch_add(p_, v_, __ATOMIC_SEQ_CST)"', '"add": "__atomic_fetch_sub(p_, v_, __ATOMIC_SEQ_CST)"', r'^(ldadd|stadd)'),
    ('SHA-1 round rotate', 'include/nxrecomp/cxx.h', "y = y + rol(x.u32[0], 5) + t + w.u32[e];", "y = y + rol(x.u32[0], 6) + t + w.u32[e];", r'^sha1'),
    ('pair load stores swapped registers', 'nxrecomp/lift/a64.py', "body += 'V128 t1 = rdv(a, %d), t2 = rdv(a + %d, %d); c->v[%d] = t1; c->v[%d] = t2;' % (size, size, size, rt, rt2)", "body += 'V128 t1 = rdv(a, %d), t2 = rdv(a + %d, %d); c->v[%d] = t2; c->v[%d] = t1;' % (size, size, size, rt, rt2)", r'^ldp [sdq],'),
    ('post-index writeback dropped', 'nxrecomp/lift/a64.py', "                return '', base, '%s += (uint64_t)%dLL;' % (base, imm)", "                return '', base, ''", r'^(ldr|str)[bh]? [wx], \[x\], #'),
    ('rbit32 off by one position', 'include/nxrecomp/ctx.h', "r |= ((v >> i) & 1u) << (31 - i);", "r |= ((v >> i) & 1u) << (30 - i % 31);", r'^rbit w'),
    ('clz of zero', 'include/nxrecomp/ctx.h', "static inline uint32_t clz32(uint32_t v) { return v ? (uint32_t)__builtin_clz(v) : 32; }", "static inline uint32_t clz32(uint32_t v) { return v ? (uint32_t)__builtin_clz(v) : 31; }", r'^(clz|cls) w'),
    ('sdiv INT_MIN / -1', 'include/nxrecomp/ctx.h', "    if (a == 0x80000000u && b == 0xffffffffu) return a;\n    return (uint32_t)((int32_t)a / (int32_t)b);", "    if (a == 0x80000000u && b == 0xffffffffu) return 0;\n    return (uint32_t)((int32_t)a / (int32_t)b);", r'^sdiv w'),
    ('umulh high part', 'include/nxrecomp/ctx.h', "return (uint64_t)(((unsigned __int128)a * b) >> 64); }", "return (uint64_t)(((unsigned __int128)a * b) >> 63); }", r'^umulh '),
    ('AES SubBytes table byte', 'include/nxrecomp/cxx.h', "0x63,0x7c,0x77,0x7b,0xf2,0x6b,0x6f,0xc5,0x30,0x01,0x67,0x2b,0xfe,0xd7,0xab,0x76,", "0x63,0x7c,0x77,0x7b,0xf2,0x6b,0x6f,0xc5,0x30,0x01,0x67,0x2b,0xfe,0xd7,0xab,0x77,", r'^(aese|aesd)'),
]


def copy_tree(dst):
    if os.path.isdir(dst):
        shutil.rmtree(dst)
    for part in ('nxrecomp', 'include', 'cmake', 'scripts', os.path.join('oracle', 'difftest')):
        shutil.copytree(os.path.join(ROOT, part), os.path.join(dst, part), ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))


def run_diff(tree, work, filt):
    py = sys.executable
    r = subprocess.run([py, os.path.join(tree, 'oracle', 'difftest', 'difftest.py'), 'all', '--work', work, '--filter', filt],
                       capture_output=True, text=True, cwd=tree)
    out = r.stdout + r.stderr
    bad = {}
    p = os.path.join(work, 'bad.json')
    if os.path.exists(p):
        bad = json.load(open(p))
    return out, bad


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--work', default=os.path.join(ROOT, 'build', 'mutation'))
    ap.add_argument('--only', default='')
    a = ap.parse_args()
    sel = [int(x) for x in a.only.split(',')] if a.only else list(range(len(MUTATIONS)))
    muts = [MUTATIONS[i] for i in sel]
    tree = os.path.join(a.work, 'tree')
    copy_tree(tree)
    filt = '|'.join('(%s)' % m[4] for m in muts)
    # the filter also selects among the hand-written tests (trap / system register / exclusive sequences) by their names
    use = filt
    print('baseline (unmutated copy), filter: %s' % (use or '(everything)'))
    out, bad = run_diff(tree, os.path.join(a.work, 'base'), use)
    base_hits = {m[0]: [k for k in bad if re.search(m[4], k)] for m in muts}
    base_fail = [n for n, k in base_hits.items() if k]
    print(out.strip().split('\n')[-3:] if out else '')
    for name, f, old, new, expect in muts:
        path = os.path.join(tree, f)
        text = open(path, encoding='utf-8', newline='').read()
        if text.count(old) != 1:
            print('MUTATION NOT APPLICABLE (%d matches): %s in %s' % (text.count(old), name, f))
            sys.exit(2)
        open(path, 'w', encoding='utf-8', newline='').write(text.replace(old, new))
    print('mutated run: %d mutations planted' % len(muts))
    out, bad = run_diff(tree, os.path.join(a.work, 'mut'), use)
    ok = True
    for name, f, old, new, expect in muts:
        hits = [k for k in bad if re.search(expect, k)]
        caught = bool(hits)
        print('%-8s %s   (%d failing shapes, e.g. %s)' % ('CAUGHT' if caught else 'MISSED', name, len(hits), hits[0] if hits else '-'))
        ok &= caught
    if base_fail:
        print('BASELINE NOT GREEN for: %s' % ', '.join(base_fail))
        ok = False
    print('mutation test: %s' % ('PASS, all %d planted bugs caught and the baseline is green' % len(muts) if ok else 'FAIL'))
    sys.exit(0 if ok else 1)


if __name__ == '__main__':
    main()
