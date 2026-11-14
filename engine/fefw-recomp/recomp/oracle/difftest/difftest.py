# SPDX-License-Identifier: GPL-3.0-or-later
"""Differential test of the lifter against the Unicorn AArch64 emulator (the reference).

Adapted from smo-recomp tools/lifter/difftest.py (GPL-3.0-or-later; itself from ssbu-recomp). For every instruction shape of
corpus.json (gen_corpus.py: every A64 shape of the Cortex-A57 ISA + LSE atomics that Capstone decodes and Unicorn executes)
a few real instruction words are executed twice from the same pseudo-random machine state: once by Unicorn, once by the C++
the lifter generates. Final registers, flags, TPIDR/FPCR and a memory window must be identical.

    python difftest.py gen      lift every corpus word, write build/difftest/gen/*.cpp + tests.json + coverage.json
    python difftest.py build    CMake + Ninja + clang-cl: build/difftest/difftest.exe
    python difftest.py check    run it and Unicorn, compare; exit code 1 on any difference
    python difftest.py show <i> registers that differ for one test
    python difftest.py all      gen + build + check
    options: --filter <regex>   only shapes whose key matches (gen), --corpus FILE, --work DIR, --states N

Test kinds (column `kind`):
  straight  data processing, loads/stores, FP/SIMD, atomics, barriers: registers + NZCV + TPIDR/FPCR + a memory window must
            match, from a random state and from an integer-edge-value state
  special   every floating-point instruction again from states made of NaN / signed zero / subnormal / tie operands (smo)
  branch    conditional branches: taken / not taken must match Unicorn's PC, registers unchanged
  trap      words that are not instructions of the console's CPU: Unicorn must fault, the lifted code must call guest_trap
  seq       short instruction sequences: exclusive load / store pairs (success, failure, CLREX), system-register access
A sampled word the lifter cannot translate is a FAILURE of coverage (listed, not hidden); it does not hide differences of
the translated ones. What is NOT tested here, by design: unconditional control transfers (the end-to-end oracle runs
them), SVC and system registers other than the whitelist below (the runtime answers them).
"""
import argparse
import collections
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import *          # noqa: F401,F403,E402
from nxrecomp.lift import Lifter, Unimpl, bits   # noqa: E402

DEFAULT_WORK = os.path.join(ROOT, 'build', 'difftest')
NX_ENV = os.path.join(ROOT, 'scripts', 'nx_env.cmd')
SHARDS = 24
COND_BRANCH = ('b.', 'cbz', 'cbnz', 'tbz', 'tbnz')
SPECIAL_STEP = 11             # of the n*n operand pairs, every 11th is tested (n*n/11 pairs per word)

# whitelisted system-register accesses: mrs xt / msr <reg>, xt of these, for a few registers
SYSREG_WORDS = []
for rt in (0, 7, 30):
    SYSREG_WORDS += [0xd53bd040 | rt, 0xd53bd060 | rt, 0xd51bd040 | rt, 0xd53b4400 | rt, 0xd53b4420 | rt,
                     0xd51b4400 | rt, 0xd51b4420 | rt, 0xd53b4200 | rt, 0xd51b4200 | rt]
# words that are not instructions of the CPU: permanently undefined / reserved encodings and BRK
TRAP_WORDS = [0xe7ffdefe, 0x00000000, 0x0000dead, 0x00010000, 0x02000000, 0x04000000, 0x06aabbcc, 0xd4200000, 0xd43eec60]


def dt_env():
    class E:
        wanted = {}
        jt = {}

        def call(self, target):
            return 'f_%x(c);' % target
    return E()


def fp_mode(w):
    if bits(w, 28, 24) == 0x1e:
        return {0: 32, 1: 64, 3: 16}.get(bits(w, 23, 22), 0)
    return 64 if bits(w, 22, 22) else 32


def reg_index(w):
    """Rm of single-register register-offset loads/stores and of structure post-index forms (the lifter reads it as an
    index: keep it small), else -1."""
    if is_mem(w):
        if bits(w, 29, 27) == 7 and not bits(w, 26, 26) and bits(w, 24, 24) == 0 and bits(w, 21, 21) and bits(w, 11, 10) == 2:
            return bits(w, 20, 16)
        if bits(w, 31, 31) == 0 and bits(w, 29, 24) in (0x0c, 0x0d) and bits(w, 23, 23):
            return bits(w, 20, 16)
        if bits(w, 29, 27) == 7 and bits(w, 26, 26) and bits(w, 24, 24) == 0 and bits(w, 21, 21) and bits(w, 11, 10) == 2:
            return bits(w, 20, 16)
    return -1


def exclusive_sequences():
    """[(name, [words])]: exclusive load / store pairs. x1 is the base, x2 / x3 data, w4 the status."""
    seqs = []
    for size in range(4):
        for acq in (0, 1):
            ld = (size << 30) | 0x08400000 | (acq << 15) | 0x7c00 | (1 << 5) | 2                  # ldxr / ldaxr x2, [x1]
            st = (size << 30) | 0x08000000 | (4 << 16) | (acq << 15) | 0x7c00 | (1 << 5) | 3       # stxr / stlxr w4, x3, [x1]
            st_other = (size << 30) | 0x08000000 | (4 << 16) | (acq << 15) | 0x7c00 | (5 << 5) | 3  # stxr to a different address
            clrex = 0xd503305f
            seqs += [('excl %d/%d ok' % (size, acq), [ld, st]), ('excl %d/%d wrong address' % (size, acq), [ld, st_other]),
                     ('excl %d/%d without load' % (size, acq), [st]), ('excl %d/%d clrex' % (size, acq), [ld, clrex, st])]
    for size in (2, 3):
        for acq in (0, 1):
            ld = (size << 30) | 0x08600000 | (acq << 15) | (3 << 10) | (1 << 5) | 2               # ldxp / ldaxp x2, x3, [x1]
            st = (size << 30) | 0x08200000 | (4 << 16) | (acq << 15) | (6 << 10) | (1 << 5) | 5   # stxp / stlxp w4, x5, x6, [x1]
            st_other = (size << 30) | 0x08200000 | (4 << 16) | (acq << 15) | (6 << 10) | (7 << 5) | 5
            seqs += [('excl pair %d/%d ok' % (size, acq), [ld, st]), ('excl pair %d/%d wrong address' % (size, acq), [ld, st_other]),
                     ('excl pair %d/%d without load' % (size, acq), [st])]
    return seqs


class Tests:
    def __init__(self, fn, md):
        self.fn, self.md = fn, md
        self.tests, self.untranslated, self.excluded, self.shape_kinds = [], [], collections.defaultdict(list), {}

    def lift(self, w):
        try:
            return self.fn.lift(w, CODE), None
        except Unimpl as e:
            return None, str(e)

    def classify(self, w, key, cls):
        mnem = key.split()[0]
        stmt, err = self.lift(w)
        if stmt is None:
            return 'untranslated', err
        if mnem.startswith(COND_BRANCH):
            return 'branch', stmt
        if re.search(r'\b(goto|return|call_indirect|guest_svc|f_7)', stmt) or 'guest_trap' in stmt:
            return None, 'leaves straight-line code'
        if bits(w, 29, 24) == 8 and is_mem(w) and not bits(w, 26, 26) and not bits(w, 23, 23) and not (bits(w, 21, 21) and not bits(w, 31, 31)):
            return None, 'exclusive access (tested as sequences)'      # LDXR / STXR / LDXP / STXP families (CASP is not one)
        return 'straight', stmt

    def add(self, kind, key, word, stmt, **kw):
        t = {'kind': kind, 'key': key, 'word': word, 'stmt': stmt, 'mem': 0, 'rn': bits(word, 9, 5), 'rm': -1, 'fpmode': 0,
             'k': 0, 'frm': -1, 'target': 0, 'style': 0, 'words': [word], 'model': 'a57'}
        t.update(kw)
        self.tests.append(t)


def build_tests(corpus, filt, states):
    from common import md as mkmd
    cap = mkmd()
    env = dt_env()
    # a wide function around CODE, so every conditional-branch target becomes a local label (goto L_<target>)
    fn = Lifter(CODE - 0x200000, CODE + 0x200000, env)
    T = Tests(fn, cap)
    for key, info in sorted(corpus['shapes'].items()):
        if filt and not re.search(filt, key):
            continue
        kinds = set()
        for w in info['words']:
            kind, stmt = T.classify(w, key, info['class'])
            model = 'max' if info['class'] == 'lse' else 'a57'
            if kind == 'untranslated':
                T.untranslated.append((key, w, stmt))
                kinds.add('UNTRANSLATED')
                continue
            if kind is None:
                T.excluded[stmt].append(key)
                kinds.add('excluded: ' + stmt)
                continue
            mem = int(is_mem(w))
            rm = reg_index(w)
            if 0 <= rm < 31 and rm == bits(w, 9, 5):
                T.excluded['index register is the base register'].append(key)      # no defined test state
                kinds.add('excluded: index register is the base register')
                continue
            if kind == 'branch':
                m = re.search(r'goto L_([0-9a-f]+);', stmt)
                T.add('branch', key, w, stmt, target=int(m.group(1), 16), model=model)
                kinds.add('branch')
                continue
            styles = list(range(states))
            if op0(w) in (5, 8, 9, 13) and not key.split()[0].startswith('f'):
                styles += [2] * 8                                 # integer data processing: colliding operands (8 draws)
            if key.split()[0] in ('aese', 'aesd', 'aesmc', 'aesimc'):
                styles += list(range(3, 35))                      # every byte value through the S-box and the mix
            for style in styles:
                T.add('straight', key, w, stmt, mem=mem, rm=rm, style=style, model=model)
            kinds.add('straight')
            if key.split()[0].startswith('f'):
                mode = fp_mode(w)
                if mode in SPECIAL:
                    unary = bool(re.match(r'^(fabs|fneg|fsqrt|fcvt|frint|frecpe|frsqrte|fmov|fcmp[a-z]*0|fcm\w+ .*#)', key)) or key.count(',') < 2
                    n = len(SPECIAL[mode])
                    for k in range(0, n if unary else n * n, 1 if unary else SPECIAL_STEP):
                        T.add('special', key + ' [special]', w, stmt, mem=0, fpmode=mode, k=k, model=model,
                              frm=-1 if unary else bits(w, 20, 16))
        T.shape_kinds[key] = sorted(kinds)
    want = (lambda k: not filt or re.search(filt, k))
    if True:
        for w in TRAP_WORDS:
            if not want('trap %08x' % w):
                continue
            stmt, err = T.lift(w)
            if stmt is None:
                T.untranslated.append(('trap word %08x' % w, w, err))
                continue
            T.add('trap', 'trap %08x' % w, w, stmt)             # a word that does not call guest_trap fails the test
        for w in SYSREG_WORDS:
            if not want('system register %08x' % w):
                continue
            stmt, err = T.lift(w)
            if stmt is None:
                T.untranslated.append(('sysreg %08x' % w, w, err))
                continue
            for style in range(states):
                T.add('straight', 'system register %08x' % w, w, stmt, style=style)
        for name, words in exclusive_sequences():
            if not want(name):
                continue
            stmts, bad = [], None
            for w in words:
                stmt, err = T.lift(w)
                if stmt is None:
                    bad = err
                    break
                stmts.append(stmt)
            if bad:
                T.untranslated.append((name, words[0], bad))
                continue
            for style in range(2):
                T.add('seq', name, words[0], ' '.join(stmts), mem=1, rn=1, rm=-1, style=style, words=words)
    return T


def write_tables(path):
    def arr(name, vals, ctype, suffix):
        return 'static const %s %s[] = {%s};\n#define %s_N %d\n' % (ctype, name, ', '.join('0x%x%s' % (v, suffix) for v in vals), name, len(vals))
    with open(path, 'w') as f:
        f.write('/* generated by difftest.py */\n')
        f.write(arr('EDGES', EDGES, 'uint64_t', 'ULL'))
        f.write(arr('TINY', TINY, 'uint64_t', 'ULL'))
        f.write(arr('SPECIAL32', SPECIAL[32], 'uint32_t', 'u'))
        f.write(arr('SPECIAL64', SPECIAL[64], 'uint64_t', 'ULL'))
        f.write(arr('SPECIAL16', SPECIAL[16], 'uint16_t', 'u'))


def cmd_gen(a):
    corpus = json.load(open(a.corpus))
    gen = os.path.join(a.work, 'gen')
    if os.path.isdir(gen):
        shutil.rmtree(gen)
    os.makedirs(gen)
    T = build_tests(corpus, a.filter, a.states)
    tests = T.tests
    write_tables(os.path.join(gen, 'tables.h'))
    per = (len(tests) + SHARDS - 1) // SHARDS
    for s in range(SHARDS):
        with open(os.path.join(gen, 'gen_%02d.cpp' % s), 'w') as f:
            f.write('/* generated by difftest.py -- do not edit */\n#include <nxrecomp/cxx.h>\nextern "C" volatile int dt_event;\n\n')
            for i in range(s * per, min(len(tests), (s + 1) * per)):
                t = tests[i]
                if t['kind'] == 'branch':
                    f.write('static void t_%d(Ctx *c) { %s dt_event = 0; return; L_%x: dt_event = 1; }\n' % (i, t['stmt'], t['target']))
                else:
                    f.write('static void t_%d(Ctx *c) { %s }\n' % (i, t['stmt']))
            f.write('\ntypedef struct { void (*fn)(Ctx *); uint32_t word; int mem, rn, rm, fpmode, k, frm, style; } Test;\n')
            f.write('extern "C" const Test tests_%02d[] = {\n' % s)
            for i in range(s * per, min(len(tests), (s + 1) * per)):
                t = tests[i]
                f.write('  { t_%d, 0x%08xU, %d, %d, %d, %d, %d, %d, %d },\n' % (i, t['word'], t['mem'], t['rn'], t['rm'], t['fpmode'], t['k'], t['frm'], t['style']))
            f.write('};\nextern "C" const int tests_%02d_count = %d;\n' % (s, max(0, min(len(tests), (s + 1) * per) - s * per)))
    with open(os.path.join(gen, 'tests_table.cpp'), 'w') as f:       # one flat table over the shards, in order
        f.write('/* generated by difftest.py */\n#include <string.h>\n#include <nxrecomp/ctx.h>\n')
        f.write('typedef struct { void (*fn)(Ctx *); uint32_t word; int mem, rn, rm, fpmode, k, frm, style; } Test;\n')
        for s in range(SHARDS):
            f.write('extern "C" const Test tests_%02d[]; extern "C" const int tests_%02d_count;\n' % (s, s))
        f.write('extern "C" { Test tests[%d]; extern const int test_count; const int test_count = %d; }\n' % (max(1, len(tests)), len(tests)))
        f.write('static struct Init { Init() { int n = 0;\n')
        for s in range(SHARDS):
            f.write('  memcpy(tests + n, tests_%02d, sizeof(Test) * tests_%02d_count); n += tests_%02d_count;\n' % (s, s, s))
        f.write('} } init_;\n')
    json.dump(tests, open(os.path.join(a.work, 'tests.json'), 'w'))
    json.dump({'untranslated': T.untranslated, 'excluded': T.excluded, 'shape_kind': T.shape_kinds},
              open(os.path.join(a.work, 'coverage.json'), 'w'), indent=0)
    kinds = collections.Counter(t['kind'] for t in tests)
    print('tests generated: %d  (%s)' % (len(tests), ', '.join('%s %d' % kv for kv in sorted(kinds.items()))))
    tested = sum(1 for k in T.shape_kinds.values() if any(x in ('straight', 'branch', 'special') for x in k))
    print('shapes in corpus: %d   tested: %d   excluded by design: %d   untranslated: %d'
          % (len(T.shape_kinds), tested, sum(1 for k in T.shape_kinds.values() if all(x.startswith('excluded') for x in k)),
             sum(1 for k in T.shape_kinds.values() if 'UNTRANSLATED' in k)))
    for reason, keys in sorted(T.excluded.items()):
        print('  excluded (%s): %d shapes, e.g. %s' % (reason, len(set(keys)), ', '.join(sorted(set(keys))[:4])))
    un = collections.Counter(k.split()[0] for k, w, e in T.untranslated)
    if un:
        print('  UNTRANSLATED mnemonics (%d, %d words): %s' % (len(un), len(T.untranslated), ', '.join('%s:%d' % kv for kv in sorted(un.items())[:60])))


def sh(cmd, **kw):
    return subprocess.run(['cmd', '/c', NX_ENV] + cmd, capture_output=True, text=True, **kw)


def exe_path(a):
    return os.environ.get('DIFFTEST_EXE') or os.path.join(a.work, 'cmake', 'difftest.exe')


def cmd_build(a, gen=None, bdir=None):
    """Configure + build. A generated test that does not compile is replaced by a stub that reports event 8 (and counted as a
    failure of that shape by `check`), so one bad statement never hides the rest of the corpus."""
    gen = gen or os.path.join(a.work, 'gen')
    bdir = bdir or os.path.join(a.work, 'cmake')
    r = sh(['cmake', '-S', HERE, '-B', bdir, '-G', 'Ninja', '-DCMAKE_BUILD_TYPE=Release',
            '-DCMAKE_TOOLCHAIN_FILE=' + os.path.join(ROOT, 'cmake', 'clang-cl.cmake').replace('\\', '/'),
            '-DDT_GEN_DIR=' + gen.replace('\\', '/')])
    if r.returncode:
        print(r.stdout[-3000:], r.stderr[-3000:])
        sys.exit(2)
    broken = set()
    bpath = os.path.join(a.work, 'broken.json')
    if os.path.exists(bpath):
        os.remove(bpath)
    for attempt in range(6):
        r = sh(['cmake', '--build', bdir, '-j', str(os.cpu_count() or 8), '--', '-k', '0'])
        if r.returncode == 0:
            break
        errs = re.findall(r'gen[\\/](gen_\d+\.cpp)\((\d+),\d+\): error', r.stdout + r.stderr)
        if not errs:
            print((r.stdout + r.stderr)[-5000:])
            sys.exit(2)
        byfile = collections.defaultdict(set)
        for f, ln in errs:
            byfile[f].add(int(ln))
        for f, lines in byfile.items():
            path = os.path.join(gen, f)
            text = open(path).read().split(chr(10))
            for ln in lines:
                m = re.match(r'static void t_(\d+)\(Ctx \*c\) \{', text[ln - 1])
                if not m:
                    print('error outside a test function: %s:%d: %s' % (f, ln, text[ln - 1][:200]))
                    print((r.stdout + r.stderr)[-3000:])
                    sys.exit(2)
                broken.add(int(m.group(1)))
                text[ln - 1] = 'static void t_%s(Ctx *c) { dt_event = 8; }' % m.group(1)
            open(path, 'w').write(chr(10).join(text))
    else:
        print('build still failing after %d repair rounds' % attempt)
        sys.exit(2)
    json.dump(sorted(broken), open(bpath, 'w'))
    print('built %s (%d generated tests did not compile)' % (os.path.join(bdir, 'difftest.exe'), len(broken)))



# Unicorn 2.1.4 (QEMU) executes LDSMAX / LDSMIN / STSMAX / STSMIN of the byte, halfword and word sizes with an UNSIGNED
# comparison (reproduced: LDSMAX w, memory 1, operand 0xffffffff stores 0xffffffff; the 64-bit forms are right). The Arm ARM
# defines them as signed. For exactly these instructions the reference result is therefore taken from the architectural
# definition, applied by `spec_smaxmin` to Unicorn's final state (memory word and Rt); everything else is Unicorn's.
def smaxmin_fields(w):
    """(is_max, size in bytes, rt, rs, rn) when w is a sub-64-bit LDSMAX/LDSMIN/STSMAX/STSMIN (any ordering), else None."""
    if bits(w, 29, 27) != 7 or bits(w, 26, 26) or bits(w, 24, 24) or not bits(w, 21, 21) or bits(w, 11, 10) != 0 or bits(w, 15, 15):
        return None
    opc = bits(w, 14, 12)
    size = bits(w, 31, 30)
    if opc not in (4, 5) or size == 3:
        return None
    return opc == 4, 1 << size, bits(w, 4, 0), bits(w, 20, 16), bits(w, 9, 5)


def spec_smaxmin(uc, A, xr, window, x, fields):
    is_max, nbytes, rt, rs, rn = fields
    base = x[rn] if rn != 31 else x[31]
    mask = (1 << (8 * nbytes)) - 1
    old = int.from_bytes(bytes(window[base - WIN_LO:base - WIN_LO + nbytes]), 'little')
    opnd = (x[rs] if rs != 31 else 0) & mask
    sx = lambda v: v - (1 << (8 * nbytes)) if v >> (8 * nbytes - 1) else v     # noqa: E731
    new = max(sx(old), sx(opnd)) if is_max else min(sx(old), sx(opnd))
    uc.mem_write(base, (new & mask).to_bytes(nbytes, 'little'))
    if rt != 31:
        uc.reg_write(xr[rt], old)


def reference(models, t, i, window):
    """-> ((regs crc, memory crc), event, error). models: {'a57': make_uc('a57'), 'max': make_uc('max')}; every test runs on
    the Cortex-A57 model (the console's CPU) except the LSE atomics, which only the max model has."""
    uc, A, xr, vr = models['max' if t.get('model') == 'max' else 'a57']
    from unicorn import UcError
    x, v, flags, tp0, tp1 = initial_state(i, t['mem'], t['rn'], t['rm'], t['style'], t['word'] & 31)
    if t.get('fpmode'):
        v = fp_special_v(t)
    for r, val in zip(xr, x[:31]):
        uc.reg_write(r, val)
    uc.reg_write(A.UC_ARM64_REG_SP, x[31])
    for r, val in zip(vr, v):
        uc.reg_write(r, int.from_bytes(val, 'little'))
    uc.reg_write(A.UC_ARM64_REG_NZCV, flags << 28)
    uc.reg_write(A.UC_ARM64_REG_TPIDR_EL0, tp0)
    uc.reg_write(A.UC_ARM64_REG_TPIDRRO_EL0, tp1)
    uc.reg_write(A.UC_ARM64_REG_FPCR, 0)
    uc.reg_write(A.UC_ARM64_REG_FPSR, 0)
    uc.mem_write(WIN_LO, window)
    words = t.get('words') or [t['word']]
    uc.mem_write(CODE, b''.join(struct.pack('<I', w) for w in words))
    uc.reg_write(A.UC_ARM64_REG_PC, CODE)
    try:
        uc.emu_start(CODE, CODE + 4 * len(words), count=len(words))
    except UcError as e:
        return None, 2, str(e)
    regs = b''.join(struct.pack('<Q', uc.reg_read(r)) for r in xr) + struct.pack('<Q', uc.reg_read(A.UC_ARM64_REG_SP))
    regs += b''.join(uc.reg_read(r).to_bytes(16, 'little') for r in vr)
    regs += struct.pack('<I', (uc.reg_read(A.UC_ARM64_REG_NZCV) >> 28) & 0xf)
    regs += struct.pack('<QQQ', uc.reg_read(A.UC_ARM64_REG_TPIDR_EL0), uc.reg_read(A.UC_ARM64_REG_TPIDRRO_EL0), uc.reg_read(A.UC_ARM64_REG_FPCR))
    fields = smaxmin_fields(t['word']) if t['kind'] == 'straight' else None
    if fields:
        spec_smaxmin(uc, A, xr, window, x, fields)
        regs = b''.join(struct.pack('<Q', uc.reg_read(r)) for r in xr) + struct.pack('<Q', uc.reg_read(A.UC_ARM64_REG_SP))
        regs += b''.join(uc.reg_read(r).to_bytes(16, 'little') for r in vr)
        regs += struct.pack('<I', (uc.reg_read(A.UC_ARM64_REG_NZCV) >> 28) & 0xf)
        regs += struct.pack('<QQQ', uc.reg_read(A.UC_ARM64_REG_TPIDR_EL0), uc.reg_read(A.UC_ARM64_REG_TPIDRRO_EL0), uc.reg_read(A.UC_ARM64_REG_FPCR))
    pc = uc.reg_read(A.UC_ARM64_REG_PC)
    event = 0
    if t['kind'] == 'branch':
        event = 1 if pc == t['target'] else 0 if pc == CODE + 4 else -1
    return (zlib.crc32(regs), zlib.crc32(bytes(uc.mem_read(WIN_LO, WIN_HI - WIN_LO)))), event, None


def run_host(a):
    out = subprocess.run([exe_path(a)], capture_output=True, text=True).stdout.split('\n')
    host = {}
    for line in out:
        p = line.split()
        if len(p) == 4:
            host[int(p[0])] = ((int(p[1], 16), int(p[2], 16)), int(p[3]))
    return host


def cmd_check(a):
    tests = json.load(open(os.path.join(a.work, 'tests.json')))
    cov = json.load(open(os.path.join(a.work, 'coverage.json')))
    host = run_host(a)
    bpath = os.path.join(a.work, 'broken.json')
    broken = set(json.load(open(bpath))) if os.path.exists(bpath) else set()
    models = {'a57': make_uc('a57'), 'max': make_uc('max')}
    window = pattern()[WIN_LO - REGION:WIN_HI - REGION]
    bad = collections.defaultdict(list)
    ok, total = collections.Counter(), collections.Counter()
    for i, t in enumerate(tests):
        total[t['kind']] += 1
        if i in broken:
            bad['(generated code does not compile) ' + t['key']].append(i)
            continue
        ref, ev, err = reference(models, t, i, window)
        h = host.get(i)
        if t['kind'] == 'trap':
            good = err is not None and h is not None and h[1] == 2
        elif err is not None:
            bad['(reference fault: %s) %s' % (err[:60], t['key'])].append(i)
            continue
        elif t['kind'] == 'branch':
            good = h is not None and h == (ref, ev) and ev in (0, 1)
        else:
            good = h is not None and h[0] == ref and h[1] == 0
        if good:
            ok[t['kind']] += 1
        else:
            bad[t['key']].append(i)
    n_untr = len(cov['untranslated'])
    failing = sum(total.values()) - sum(ok.values())
    shapes_total = len(cov['shape_kind'])
    shapes_untr = sum(1 for k in cov['shape_kind'].values() if 'UNTRANSLATED' in k)
    print('tests: %d   identical: %d   different: %d   untranslated corpus words: %d (%d of %d shapes)'
          % (sum(total.values()), sum(ok.values()), failing, n_untr, shapes_untr, shapes_total))
    for k in sorted(total):
        print('  %-9s %6d identical of %6d' % (k, ok[k], total[k]))
    for key, idxs in sorted(bad.items(), key=lambda kv: -len(kv[1]))[:80]:
        t = tests[idxs[0]]
        print('  DIFF %-52s %d   e.g. #%d %08x' % (key[:52], len(idxs), idxs[0], t['word']))
    json.dump({k: v for k, v in bad.items()}, open(os.path.join(a.work, 'bad.json'), 'w'))
    sys.exit(1 if failing or (a.strict and n_untr) else 0)


def cmd_show(a):
    tests = json.load(open(os.path.join(a.work, 'tests.json')))
    t = tests[a.index]
    print('#%d %s %s %s\n  %s' % (a.index, t['kind'], ' '.join('%08x' % w for w in t['words']), t['key'], t['stmt']))
    models = {'a57': make_uc('a57'), 'max': make_uc('max')}
    uc, A, xr, vr = models['max' if t.get('model') == 'max' else 'a57']
    x, v, flags, tp0, tp1 = initial_state(a.index, t['mem'], t['rn'], t['rm'], t['style'], t['word'] & 31)
    if t.get('fpmode'):
        v = fp_special_v(t)
    reference(models, t, a.index, pattern()[WIN_LO - REGION:WIN_HI - REGION])
    ref = {('x%d' % i): uc.reg_read(r) for i, r in enumerate(xr)}
    ref['sp'] = uc.reg_read(A.UC_ARM64_REG_SP)
    ref.update({('v%d' % i): uc.reg_read(r) for i, r in enumerate(vr)})
    ref['nzcv'] = (uc.reg_read(A.UC_ARM64_REG_NZCV) >> 28) & 0xf
    ref['tpidr'] = uc.reg_read(A.UC_ARM64_REG_TPIDR_EL0)
    ref['tpidrro'] = uc.reg_read(A.UC_ARM64_REG_TPIDRRO_EL0)
    ref['fpcr'] = uc.reg_read(A.UC_ARM64_REG_FPCR)
    host = {}
    for line in subprocess.run([exe_path(a), str(a.index)], capture_output=True, text=True).stdout.split('\n'):
        p = line.split()
        if len(p) == 2:
            host[p[0]] = int(p[1], 16)
    init = {('x%d' % i): x[i] for i in range(31)}
    init['sp'] = x[31]
    init.update({('v%d' % i): int.from_bytes(v[i], 'little') for i in range(32)})
    init['nzcv'] = flags
    print('  host event %s' % host.get('event'))
    for k in ref:
        if ref[k] != host.get(k):
            print('  %-5s init %x\n        ref  %x\n        host %x' % (k, init.get(k, -1), ref[k], host.get(k, -1)))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('cmd', choices=['gen', 'build', 'check', 'show', 'all'])
    ap.add_argument('index', nargs='?', type=int)
    ap.add_argument('--corpus', default=os.path.join(HERE, 'corpus.json'))
    ap.add_argument('--work', default=DEFAULT_WORK)
    ap.add_argument('--filter', default='')
    ap.add_argument('--states', type=int, default=2)
    ap.add_argument('--uc-model', default='max')
    ap.add_argument('--strict', action='store_true', help='untranslated corpus words also fail the check')
    a = ap.parse_args()
    os.makedirs(a.work, exist_ok=True)
    if a.cmd in ('gen', 'all'):
        cmd_gen(a)
    if a.cmd in ('build', 'all'):
        cmd_build(a)
    if a.cmd in ('check', 'all'):
        cmd_check(a)
    if a.cmd == 'show':
        cmd_show(a)


if __name__ == '__main__':
    main()
