# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate the instruction corpus of the differential test: the set of A64 instruction SHAPES (mnemonic x operand form x
arrangement x addressing mode x ...) with a few real encodings of each. smo-recomp sampled its corpus from the game's
binary; there is no binary yet, so this enumerates the encoding space instead:

  1. sample random 32-bit words and decode them with Capstone;
  2. close the set over single-bit flips (every shape reachable by changing one bit is found);
  3. keep a shape only if it is an instruction the reference accepts: Unicorn's Cortex-A57 executes it, or Unicorn's
     `max` CPU executes it and it is one of the ARMv8.1 LSE atomics, the FEAT_LRCPC / LRCPC2 loads and stores (LDAPR,
     LDAPUR, STLUR families: acquire / release, same model as LDAR / STLR); both run on the `max` model (class 'lse'). The
     rest of the project's scope is Cortex-A57 = ARMv8.0 incl. crypto + CRC32. SVE / SME / FP16 arithmetic / dot product
     ... are not instructions of the console's CPU: counted per reason in `out_of_scope`, listed in docs/GAPS.md. Two
     families the lifter handles are out of scope here because Unicorn is no reference for them: pointer authentication
     (PACIA ... AUTDZB, XPACI, XPACD, PACGA; identity in the lifter, PAC is not modelled, whereas Unicorn's `max` models
     real PAC and changes the pointer bits) and the MTE tag stores (STG, STZG, ST2G, STZ2G; Unicorn's `max` has MTE as
     instructions-only without tag memory and raises an exception for them);
  4. drop encodings whose result is architecturally UNPREDICTABLE (base register = transfer register with writeback, ...).

    python gen_corpus.py [--samples N] [--out corpus.json]     deterministic: same arguments, same file
"""
import argparse
import collections
import json
import os
import random
import re
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import *          # noqa: F401,F403,E402

PER_SHAPE = 6                 # encodings kept per shape

LSE = re.compile(r'^(ldadd|ldclr|ldeor|ldset|ldsmax|ldsmin|ldumax|ldumin|stadd|stclr|steor|stset|stsmax|stsmin|stumax|'
                 r'stumin|swp|casp|cas)(al|a|l)?(b|h)?$')
LRCPC = re.compile(r'^(ldapr(b|h)?|ldapur(b|h|sb|sh|sw)?|stlur(b|h)?)$')            # FEAT_LRCPC / FEAT_LRCPC2
MTE_STORE = re.compile(r'^(stg|stzg|st2g|stz2g)$')                                  # MTE tag stores (no-op / zeroing in the lifter)
PAC = re.compile(r'^(pac|aut)(i|d)[ab]z?$|^xpac[id]$|^pacga$')                      # pointer authentication, register forms
CONTROL = {'b', 'bl', 'br', 'blr', 'ret'}                 # control transfers: exercised by the end-to-end oracle
COND_BRANCH = ('b.', 'cbz', 'cbnz', 'tbz', 'tbnz')
SYSTEM = {'mrs', 'msr', 'sys', 'sysl', 'dc', 'ic', 'tlbi', 'at', 'svc', 'hvc', 'smc', 'eret', 'drps', 'dcps1', 'dcps2',
          'dcps3', 'hlt', 'bti', 'msr_imm'}              # system / privileged: whitelisted forms are tested by hand (difftest.py)
SVE_RE = re.compile(r'\b[zp]\d+|\bza\b|\bzt0\b|\bvg\b|\bza[hv]?\d')


def op0(w):
    return (w >> 25) & 0xf


def is_mem(w):
    return op0(w) in (4, 6, 12, 14)


def unpredictable(i0, w):
    """True for encodings whose result the architecture leaves UNPREDICTABLE (so no emulator is a reference for them)."""
    from capstone import arm64_const as A
    m = i0.mnemonic
    if not is_mem(w):
        return False
    ops = i0.operands
    regs = [o for o in ops if o.type == A.ARM64_OP_REG]
    mem = next((o for o in ops if o.type == A.ARM64_OP_MEM), None)
    if mem is None:
        return False
    base = mem.mem.base
    wb = i0.writeback or any(o.type == A.ARM64_OP_IMM for o in ops[ops.index(mem) + 1:]) or \
        any(o.type == A.ARM64_OP_REG for o in ops[ops.index(mem) + 1:])
    rt = [o.reg for o in regs[:ops.index(mem)] if o.reg]
    gp = lambda r: i0.reg_name(r) and i0.reg_name(r)[0] in 'xw' and i0.reg_name(r) not in ('xzr', 'wzr')   # noqa: E731
    bname = i0.reg_name(base) if base else None
    bnum = bname and re.sub(r'\D', '', bname) if bname not in ('sp', 'wsp') else 'sp'
    def num(r):
        n = i0.reg_name(r)
        return re.sub(r'\D', '', n) if n and n[0] in 'xw' and n not in ('xzr', 'wzr') else None
    if m.startswith('ld') or m.startswith('st'):
        loads = m.startswith('ld')
        transfer = [r for r in rt]
        gtr = [num(r) for r in transfer if num(r) is not None]
        if wb and bnum and bnum in gtr:
            return True                                  # writeback with Rt == Rn
        if m.startswith('ldp') or m.startswith('ldnp') or m in ('ldxp', 'ldaxp'):
            if len(set(gtr)) < len(gtr) and gtr:
                return True                              # Rt == Rt2
            nv = [i0.reg_name(r) for r in transfer]
            if len(nv) == 2 and nv[0] == nv[1]:
                return True
        if re.match(r'^st[la]?xr|^st[la]?xp|^stlxr', m):
            if gtr and (gtr[0] in gtr[1:] or (bnum and gtr[0] == bnum)):
                return True                              # Rs == Rt / Rt2 / Rn
    return False


def build(samples, seed):
    import collections as C
    cap = md()
    rng = random.Random(seed)
    seen = C.OrderedDict()                              # shape -> list of words
    sve, system = set(), set()
    cache = {}

    def decode(w):
        r = cache.get(w)
        if r is None:
            ins = list(cap.disasm(struct.pack('<I', w), CODE))
            r = None
            if ins:
                i0 = ins[0]
                r = (shape_of(i0), i0.mnemonic, i0.op_str, bool(SVE_RE.search(i0.op_str)))
            if len(cache) < 3000000:
                cache[w] = r
        return r

    def add(w, queue):
        d = decode(w)
        if d is None:
            return
        shape = d[0]
        if d[3]:                                         # SVE / SME: counted, not expanded
            sve.add(shape)
            return
        if d[1] in SYSTEM:                               # system registers / privileged: hand-written tests
            system.add(d[1])
            return
        lst = seen.get(shape)
        if lst is None:
            seen[shape] = [w]
            queue.append(w)
        elif len(lst) < PER_SHAPE * 4 and w not in lst:
            lst.append(w)

    queue = []
    for _ in range(samples):
        add(rng.getrandbits(32), queue)
    nq = 0
    while nq < len(queue):                               # closure over single-bit flips
        w = queue[nq]
        nq += 1
        for b in range(32):
            add(w ^ (1 << b), queue)
    return seen, cap, sve, system


def classify(seen, cap):
    """-> (shapes {key: {'words': [...], 'class': ...}}, out_of_scope {reason: [[key, word], ...]})"""
    from unicorn import UcError
    ucs = {m: make_uc(m) for m in ('a57', 'max')}
    window = pattern()[WIN_LO - REGION:WIN_HI - REGION]

    def runs(model, w, st):
        uc, A, xr, vr = ucs[model]
        x, v, flags = st[:3]
        for r, val in zip(xr, x[:31]):
            uc.reg_write(r, val)
        uc.reg_write(A.UC_ARM64_REG_SP, x[31])
        for r, val in zip(vr, v):
            uc.reg_write(r, int.from_bytes(val, 'little'))
        uc.reg_write(A.UC_ARM64_REG_NZCV, flags << 28)
        uc.mem_write(WIN_LO, window)
        uc.mem_write(CODE, struct.pack('<I', w))
        uc.reg_write(A.UC_ARM64_REG_PC, CODE)
        try:
            uc.emu_start(CODE, CODE + 4, count=1)
            return True
        except UcError as e:
            return 'EXC' not in str(e) and 'exception' not in str(e).lower()

    shapes, oos = {}, collections.defaultdict(list)
    for key, words in sorted(seen.items()):
        mnem = key.split()[0]
        first = words[0]
        i0 = list(cap.disasm(struct.pack('<I', first), CODE))[0]
        if SVE_RE.search(i0.op_str):
            oos['SVE / SME (not on Cortex-A57)'].append((key, first))
            continue
        if mnem in CONTROL:
            oos['control transfer (end-to-end oracle)'].append((key, first))
            continue
        if mnem in SYSTEM or mnem.startswith('b.') is False and mnem in ('msr', 'mrs'):
            oos['system / privileged (whitelisted forms are hand-written tests)'].append((key, first))
            continue
        if PAC.match(mnem):
            oos['pointer authentication (identity in the lifter; Unicorn max models real PAC, so no reference)'].append((key, first))
            continue
        if MTE_STORE.match(mnem):
            oos['MTE tag stores (no-op / zeroing in the lifter; Unicorn does not execute them)'].append((key, first))
            continue
        good, cls = [], None
        for w in words:
            ii = list(cap.disasm(struct.pack('<I', w), CODE))[0]
            if unpredictable(ii, w):
                continue
            x, v, flags = initial_state(w, is_mem(w), (w >> 5) & 31, (w >> 16) & 31, 0)[:3]
            if runs('a57', w, (x, v, flags)):
                c = 'a57'
            elif (LSE.match(mnem) or LRCPC.match(mnem)) and runs('max', w, (x, v, flags)):
                c = 'lse'
            else:
                continue
            cls = cls or c
            good.append(w)
            if len(good) >= PER_SHAPE:
                break
        if good:
            shapes[key] = {'class': cls, 'words': good}
        else:
            executes_max = any(runs('max', w, initial_state(w, is_mem(w), (w >> 5) & 31, (w >> 16) & 31, 0)) for w in words)
            if executes_max:
                oos['not an instruction of the Cortex-A57 (ARMv8.1+ extension)'].append((key, first))
            else:
                oos['unallocated / UNPREDICTABLE only'].append((key, first))
    return shapes, oos


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--samples', type=int, default=3000000)
    ap.add_argument('--seed', type=int, default=20260101)
    ap.add_argument('--out', default=os.path.join(HERE, 'corpus.json'))
    a = ap.parse_args()
    seen, cap, sve, system = build(a.samples, a.seed)
    print('decodable shapes found: %d (+ %d SVE / SME shapes seen, not expanded)' % (len(seen), len(sve)))
    shapes, oos = classify(seen, cap)
    oos['SVE / SME (not on Cortex-A57)'] = [(k, 0) for k in sorted(sve)]
    oos['system / privileged (whitelisted forms are hand-written tests)'] = [(k, 0) for k in sorted(system)]
    mn = collections.Counter(k.split()[0] for k in shapes)
    print('in scope: %d shapes, %d mnemonics   (a57: %d, lse: %d)' % (len(shapes), len(mn), sum(1 for s in shapes.values() if s['class'] == 'a57'),
                                                                   sum(1 for s in shapes.values() if s['class'] == 'lse')))
    for r, lst in sorted(oos.items()):
        print('  out of scope: %-70s %5d shapes' % (r, len(lst)))
    out = {'samples': a.samples, 'seed': a.seed, 'shapes': shapes,
           'out_of_scope': {r: {'count': len(l), 'examples': l[:12]} for r, l in oos.items()}}
    with open(a.out, 'w') as f:
        json.dump(out, f, sort_keys=True, indent=0)
    print('wrote', a.out)


if __name__ == '__main__':
    main()
