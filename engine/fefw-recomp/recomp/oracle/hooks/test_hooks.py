# SPDX-License-Identifier: GPL-3.0-or-later
"""Site-list test: patches, inline callouts, function hooks and dynamic immediates compiled into lifted code.

    python test_hooks.py [--build DIR]

Takes the end-to-end test program 01_int (-O2), picks addresses it really executes (from a Unicorn trace), writes a site list
(entryhook, inline, midhook, noinline, dynimm) and lifts with it. PASS when
  * the lifted program still ends in the same state as Unicorn running the same (poked) program;
  * every callout was called exactly as often as Unicorn executed that address (entry hooks, inline sites, mid-function hooks);
  * a dynimm site follows a rewrite of its instruction in memory (NX_POKE on x64, a poke on the Unicorn side) while the same
    instruction WITHOUT a dynimm site does not (case C must differ from Unicorn);
  * a jtable line on something that is not a `br` is refused.
Hooks here only count (they return 0): taking over a function is the runtime's business, tested with the runtime.
"""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
sys.path.insert(0, os.path.join(ROOT, 'oracle', 'e2e'))
sys.path.insert(0, ROOT)
import build_prog       # noqa: E402
import run_e2e          # noqa: E402
import unicorn_side     # noqa: E402
from nxrecomp.image import Image   # noqa: E402
from nxrecomp import funcs         # noqa: E402


def bits(w, hi, lo):
    return (w >> lo) & ((1 << (hi - lo + 1)) - 1)


def main():
    build = os.path.join(ROOT, 'build', 'hooks')
    for i, a in enumerate(sys.argv):
        if a == '--build':
            build = sys.argv[i + 1]
    os.makedirs(build, exist_ok=True)
    src = os.path.join(ROOT, 'oracle', 'e2e', 'progs', '01_int.c')
    elf = os.path.join(build, '01_int_O2.elf')
    ok, msg = build_prog.build(src, elf, '-O2')
    assert ok, msg
    img = Image.from_static_elf(elf)
    rows, _ = funcs.analyse(img)
    allt = [img.base + r['start'] + 4 * k for r in rows for k in range((r['end'] - r['start']) // 4)]
    base_run = unicorn_side.run(elf, count_addrs=allt)
    counts = {a: n for a, n in base_run['hits'].items() if n}
    entries = [img.base + r['start'] for r in rows]
    hot_entries = [a for a in entries if counts.get(a)]
    assert len(hot_entries) >= 3, 'need executed functions'
    entry_hook, noinline = hot_entries[0], hot_entries[1]
    mid = dyn = inl = None
    for r in rows:
        s, e = img.base + r['start'], img.base + r['end']
        for a in range(s + 8, e - 4, 4):
            w = img.word(a)
            plain = bits(w, 28, 25) not in (10, 11)
            if counts.get(a) and plain:
                if mid is None:
                    mid = a
                elif inl is None and a != mid:
                    inl = a
            if counts.get(a) and dyn is None and (w & 0xff800000) == 0x91000000 and bits(w, 4, 0) != 31 and bits(w, 9, 5) != 31:
                dyn = a                                  # add xd, xn, #imm (64-bit, no shift)
    assert mid and dyn and inl, (mid, dyn, inl)
    sites = os.path.join(build, 'sites.txt')
    with open(sites, 'w') as f:
        f.write('entryhook %x\nnoinline %x\nmidhook %x\ninline %x\ndynimm %x\n' % (entry_hook, noinline, mid, inl, dyn))
    w0 = img.word(dyn)
    new_dyn = (w0 & ~(0xfff << 10)) | ((((w0 >> 10) & 0xfff) + 5) & 0xfff) << 10
    sites_nodyn = os.path.join(build, 'sites_nodyn.txt')
    with open(sites_nodyn, 'w') as f:
        f.write('entryhook %x\n' % entry_hook)
    cases = {'A': (sites, None), 'B': (sites, {dyn: new_dyn}), 'C': (sites_nodyn, {dyn: new_dyn})}
    cm = os.path.join(build, 'cases.cmake')
    with open(cm, 'w') as f:
        f.write('set(E2E_CASE_NAMES %s)\n' % ' '.join('hk_' + k for k in cases))
        for k, (st, _) in cases.items():
            f.write('set(E2E_ELF_hk_%s "%s")\nset(E2E_SITES_hk_%s "%s")\n' % (k, elf.replace('\\', '/'), k, st.replace('\\', '/')))
    bdir = os.path.join(build, 'cmake')
    if os.path.isdir(bdir):
        for old in os.listdir(bdir):
            if old.startswith('run_') and old.endswith('.exe'):
                os.remove(os.path.join(bdir, old))
    r = run_e2e.sh(['cmake', '-S', os.path.join(ROOT, 'oracle', 'e2e'), '-B', bdir, '-G', 'Ninja', '-DCMAKE_BUILD_TYPE=Release',
                    '-DCMAKE_TOOLCHAIN_FILE=' + os.path.join(ROOT, 'cmake', 'clang-cl.cmake').replace('\\', '/'),
                    '-DE2E_CASES=' + cm.replace('\\', '/')])
    assert r.returncode == 0, r.stdout[-2000:] + r.stderr[-2000:]
    r = run_e2e.sh(['cmake', '--build', bdir, '-j', '8'])
    assert r.returncode == 0, (r.stdout + r.stderr)[-4000:]
    results = {}
    for k, (st, pokes) in cases.items():
        env = dict(os.environ)
        if pokes:
            env['NX_POKE'] = ','.join('%x:%x' % (a, w) for a, w in pokes.items())
        prefix = os.path.join(build, 'out_' + k)
        r = subprocess.run([os.path.join(bdir, 'run_hk_%s.exe' % k), elf, prefix], capture_output=True, text=True, env=env)
        assert r.returncode == 0, r.stderr
        regs, mem = run_e2e.read_x64(prefix, img)
        watch = [a for a in (entry_hook, mid, inl) if k != 'C' or a == entry_hook]
        uc = unicorn_side.run(elf, pokes=pokes, count_addrs=watch)
        diffs = run_e2e.compare(uc, regs, mem)
        hits = {}
        for line in open(prefix + '.txt'):
            p = line.split()
            if p[0] == 'hit':
                hits[int(p[1], 16)] = int(p[2])
        for a in watch:
            if hits.get(a, 0) != uc['hits'][a]:
                diffs.append('callout at %x ran %d times, Unicorn executed it %d times' % (a, hits.get(a, 0), uc['hits'][a]))
        results[k] = diffs
        what = {'A': 'sites, no poke', 'B': 'sites + dynimm poke', 'C': 'poke without a dynimm site (must DIFFER)'}[k]
        print('%s case %s (%s)  %s' % ('same' if not diffs else 'differs', k, what, '; '.join(diffs[:3])))
    bad = os.path.join(build, 'sites_bad.txt')
    open(bad, 'w').write('jtable %x\n' % entry_hook)
    rr = subprocess.run([sys.executable, '-m', 'nxrecomp', 'lift', elf, os.path.join(build, 'gen_bad'), '--sites', bad, '--jobs', '1'],
                        capture_output=True, text=True, cwd=ROOT)
    refused = rr.returncode != 0 and 'is not a br' in (rr.stderr + rr.stdout)
    ok_all = not results['A'] and not results['B'] and bool(results['C']) and refused
    print('hooks: A identical %s, B identical %s, C differs %s, jtable on a non-br refused %s'
          % (not results['A'], not results['B'], bool(results['C']), refused))
    print('hook test: %s' % ('PASS' if ok_all else 'FAIL'))
    return 0 if ok_all else 1


if __name__ == '__main__':
    sys.exit(main())
