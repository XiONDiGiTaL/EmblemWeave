#!/usr/bin/env python3
"""Tiling oracle: PASS/FAIL over (1) a cross-check against Ryujinx's own tiled output on the generated case matrix,
(2) a tile -> untile identity plus independent address-formula check on a large synthetic matrix.

  python oracle/run_oracle.py [--cases DIR] [--regen] [--seeds 3]

--regen rebuilds the Ryujinx reference tool and regenerates the cases (needs .NET 10 and scripts/setup_ryujinx.ps1 run once).
Exit 0 = ORACLE PASS.
"""
import argparse
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXPECT_CASES = 332          # RyuTilingRef gen --seed 1 (locked)
EXPECT_BLOCKS = 20455125


def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    return r.returncode, (r.stdout + r.stderr).strip()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--cases', default=os.path.join(ROOT, 'cases'))
    ap.add_argument('--regen', action='store_true')
    ap.add_argument('--seeds', type=int, default=3)
    a = ap.parse_args()
    exe = os.path.join(ROOT, 'build', 'release', 'tiling_test.exe')
    if not os.path.exists(exe):
        print('ORACLE FAIL: build first (scripts/build.ps1)'); return 2
    fails = []
    if a.regen or not os.path.exists(os.path.join(a.cases, 'cases.tsv')):
        rc, out = run(['dotnet', 'build', os.path.join(ROOT, 'ref', 'RyuTilingRef', 'RyuTilingRef.csproj'), '-c', 'Release', '-v', 'q'])
        if rc: print(out[-2000:]); print('ORACLE FAIL: reference build'); return 2
        rc, out = run(['dotnet', os.path.join(ROOT, 'build', 'bin', 'Release', 'net10.0', 'RyuTilingRef.dll'), 'gen', '--out', a.cases, '--seed', '1'])
        if rc: print(out[-2000:]); print('ORACLE FAIL: case generation'); return 2
    rc, out = run([sys.executable, '-I', os.path.join(ROOT, 'scripts', 'verify_layout.py'), a.cases])
    print('reference self-check (verify_layout.py):', out.splitlines()[-1] if out else rc)
    if rc: fails.append('reference cases fail the independent python check')
    rc, out = run([exe, 'crosscheck', a.cases])
    last = out.splitlines()[-1]
    print(last)
    if rc: fails.append('crosscheck against Ryujinx')
    if f'crosscheck: {EXPECT_CASES} cases, 0 failed, {EXPECT_BLOCKS} blocks compared' != last:
        fails.append(f'crosscheck summary differs from the locked one ({EXPECT_CASES} cases, {EXPECT_BLOCKS} blocks)')
    for s in range(a.seeds):
        rc, out = run([exe, 'roundtrip', '--seed', str(s)])
        print(f'seed {s}:', out.splitlines()[-1])
        if rc: fails.append(f'roundtrip seed {s}')
    print('ORACLE PASS' if not fails else 'ORACLE FAIL: ' + '; '.join(fails))
    return 1 if fails else 0


if __name__ == '__main__':
    sys.exit(main())
