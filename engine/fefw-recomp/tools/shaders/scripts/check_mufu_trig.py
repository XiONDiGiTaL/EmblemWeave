#!/usr/bin/env python3
"""Regression check: Ampere MUFU.SIN/COS take the angle in turns (result = sin/cos(2*pi*x)).

Volta and later have no RRO; the compiler multiplies by 1/(2*pi) (FMUL.RZ x, 0.15915494) before MUFU.SIN/COS, so the
translated SPIR-V must feed GLSL.std.450 Sin/Cos with (operand * 2*pi). Before the fix the translator emitted Sin/Cos
of the raw operand, i.e. frequencies 2*pi too low (fog height waves in 7c8c05da24a1f5a2, and ~32k other blocks).

Translates one corpus block with the harness, disassembles it with spirv-dis and checks that every Sin/Cos argument is
an OpFMul by the float constant 2*pi (0x40c90fdb). Exit 0 = PASS (and at least one Sin/Cos seen), 1 = FAIL.

Usage: python -I check_mufu_trig.py --corpus <dir with unique.tsv code/ ctrl/> --hash <code hash>
       [--harness <spirv_harness.dll>] [--spirv-dis <exe>]
Example block: 7c8c05da24a1f5a2 (the first-area fog pass, 24 MUFU.COS).
"""
import argparse, os, re, struct, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
TWO_PI_BITS = 0x40c90fdb


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--corpus', required=True)
    ap.add_argument('--hash', required=True)
    ap.add_argument('--harness', default=os.path.join(HERE, '..', 'build', 'bin', 'Release', 'net10.0', 'spirv_harness.dll'))
    ap.add_argument('--spirv-dis', default='spirv-dis')
    a = ap.parse_args()

    ctrl = None
    with open(os.path.join(a.corpus, 'unique.tsv')) as f:
        hdr = f.readline().rstrip('\n').split('\t')
        for line in f:
            row = line.rstrip('\n').split('\t')
            if row[0] == a.hash:
                ctrl = row[hdr.index('first_ctrl')]
                break
    if ctrl is None:
        print(f'FAIL: {a.hash} not in {a.corpus}/unique.tsv'); return 1

    with tempfile.TemporaryDirectory() as td:
        spv = os.path.join(td, 'x.spv')
        r = subprocess.run(['dotnet', a.harness, os.path.join(a.corpus, 'code', a.hash + '.bin'),
                            os.path.join(a.corpus, 'ctrl', ctrl + '.bin'), '-o', spv], capture_output=True, text=True)
        if r.returncode != 0 or not os.path.exists(spv):
            print('FAIL: translation failed\n' + r.stdout + r.stderr); return 1
        d = subprocess.run([a.spirv_dis, '--raw-id', spv], capture_output=True, text=True)
        if d.returncode != 0:
            print('FAIL: spirv-dis\n' + d.stderr); return 1
        text = d.stdout

    consts = {}   # id -> float bits
    defs = {}     # id -> (opcode, operands)
    for line in text.splitlines():
        m = re.match(r'\s*(%\w+) = (Op\w+) (.*)$', line)
        if not m:
            continue
        rid, op, rest = m.groups()
        args = rest.split()
        defs[rid] = (op, args)
        if op == 'OpConstant' and len(args) == 2:
            v = args[1]
            try:
                bits = int(v, 16) if v.startswith('0x') else struct.unpack('<I', struct.pack('<f', float(v)))[0]
                consts[rid] = bits
            except ValueError:
                pass

    seen = bad = 0
    for rid, (op, args) in defs.items():
        if op != 'OpExtInst' or len(args) < 4 or args[2] not in ('Sin', 'Cos'):
            continue
        seen += 1
        src = defs.get(args[3])
        ok = src is not None and src[0] == 'OpFMul' and any(
            abs(consts.get(x, 0) - TWO_PI_BITS) <= 1 for x in src[1][1:3])
        if not ok:
            bad += 1
            print(f'  {rid} = {args[2]}({args[3]}): argument is not x * 2pi')
    if seen == 0:
        print('FAIL: no Sin/Cos in the module'); return 1
    if bad:
        print(f'FAIL: {bad} of {seen} Sin/Cos without the 2pi turn scale'); return 1
    print(f'PASS: {seen} Sin/Cos, all scaled by 2pi'); return 0


if __name__ == '__main__':
    sys.exit(main())
