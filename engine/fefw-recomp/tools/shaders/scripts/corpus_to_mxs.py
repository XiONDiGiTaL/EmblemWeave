#!/usr/bin/env python3
"""Turn a harness-layout shader corpus (manifest.tsv + code_<h>.bin + ctrl_<h>.bin) into .mxs stage files for mx_batch.exe.

Usage: corpus_to_mxs.py <corpus folder> --out <dir>

Input: the corpus folder as written by dksh_to_corpus.py (or Wonder's containers.py): manifest.tsv with columns
id, stage, code_off, code_size, ctrl_off, ctrl_size, program_start, program_bytes, const_off, const_size, code_sha1_16,
code_file, ctrl_file (numbers in hex), plus the code and ctrl blobs they name.

Output: one <code_sha1_16>_<stage>.mxs per distinct code block (stage = vertex, fragment, geometry, tess_control,
tess_eval or compute). Layout (little endian), the one mx_all.c and mx_batch.c read:
  "MXS1", u32 stage (0 vertex, 1 fragment, 2 geometry, 3 tess control, 4 tess eval, 5 compute), u32 code bytes,
  u32 constant data bytes, u32 work group x, y, z (ctrl +0x748), u32 shared bytes (ctrl +0x754), u32 local bytes (0),
  the code, the constant data.
Code: graphics stages from program_start (the program header, 0x50 bytes, then the instructions) for program_bytes;
compute stages from program_start (instructions only) for program_bytes. Constant data: code[const_off : +const_size].
Stage: ctrl u32 at +0x714 == 5 means compute; otherwise the SPH type in code[program_start] bits 10..13 is mapped
(1->vertex, 5->fragment, 4->geometry, 2->tess control, 3->tess eval, 0->vertex). The manifest's own stage column is
checked against the result and every disagreement is printed.
"""
import argparse
import hashlib
import os
import struct
import sys

SPH_TO_MX = {1: 0, 5: 1, 4: 2, 2: 3, 3: 4, 0: 0}
MX_NAMES = {0: 'vertex', 1: 'fragment', 2: 'geometry', 3: 'tess_control', 4: 'tess_eval', 5: 'compute'}
MANIFEST_NAMES = {'vertex': 0, 'fragment': 1, 'geometry': 2, 'tess_control': 3, 'tess_eval': 4, 'compute': 5}


def num(s):
    return int(s, 16) if s.lower().startswith('0x') else int(s)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('corpus')
    ap.add_argument('--out', required=True)
    a = ap.parse_args()
    man = os.path.join(a.corpus, 'manifest.tsv')
    os.makedirs(a.out, exist_ok=True)
    with open(man, encoding='utf-8') as f:
        rows = [line.rstrip('\n').split('\t') for line in f if line.strip()]
    head, rows = rows[0], rows[1:]
    col = {name: i for i, name in enumerate(head)}
    written, done, per_stage, mismatch, bad = {}, set(), {}, [], []
    for r in rows:
        ident = r[col['id']]
        key = r[col['code_sha1_16']]
        if key in done:
            continue
        done.add(key)
        code = open(os.path.join(a.corpus, r[col['code_file']]), 'rb').read()
        ctrl = open(os.path.join(a.corpus, r[col['ctrl_file']]), 'rb').read()
        if len(ctrl) < 0x760:
            bad.append((ident, 'ctrl section too short (%d bytes)' % len(ctrl)))
            continue
        start = num(r[col['program_start']])
        pbytes = num(r[col['program_bytes']])
        coff = num(r[col['const_off']])
        csize = num(r[col['const_size']])
        nvn = struct.unpack_from('<I', ctrl, 0x714)[0]
        ctrl_pbytes = struct.unpack_from('<I', ctrl, 0x6f8)[0]
        if nvn == 5:
            stage = 5
        else:
            if start + 4 > len(code):
                bad.append((ident, 'program_start beyond code'))
                continue
            sph = struct.unpack_from('<I', code, start)[0]
            t = (sph >> 10) & 15
            if t not in SPH_TO_MX:
                bad.append((ident, 'unknown SPH type %d' % t))
                continue
            stage = SPH_TO_MX[t]
        if ctrl_pbytes != pbytes:
            mismatch.append('%s: program_bytes manifest %#x, ctrl +0x6f8 %#x' % (ident, pbytes, ctrl_pbytes))
        if MANIFEST_NAMES.get(r[col['stage']]) != stage:
            mismatch.append('%s: manifest stage %s, derived %s' % (ident, r[col['stage']], MX_NAMES[stage]))
        if start + pbytes > len(code) or coff + csize > len(code):
            bad.append((ident, 'code or constant data beyond the code block'))
            continue
        body = code[start:start + pbytes]
        cdata = code[coff:coff + csize]
        wg = struct.unpack_from('<3I', ctrl, 0x748)
        shared = struct.unpack_from('<I', ctrl, 0x754)[0]
        name = '%s_%s' % (key, MX_NAMES[stage])
        path = os.path.join(a.out, name + '.mxs')
        with open(path, 'wb') as f:
            f.write(struct.pack('<4s8I', b'MXS1', stage, len(body), len(cdata), wg[0], wg[1], wg[2], shared, 0))
            f.write(body)
            f.write(cdata)
        written[name] = path
        per_stage[MX_NAMES[stage]] = per_stage.get(MX_NAMES[stage], 0) + 1
    print('rows: %d, distinct code blocks: %d, written: %d -> %s' % (len(rows), len(done), len(written), a.out))
    for k in sorted(per_stage):
        print('  %-13s %d' % (k, per_stage[k]))
    for m in mismatch[:40]:
        print('  note: ' + m)
    if len(mismatch) > 40:
        print('  note: ... %d more mismatches' % (len(mismatch) - 40))
    print('  note: %d mismatch(es) in total' % len(mismatch))
    for ident, why in bad:
        print('  SKIPPED %s: %s' % (ident, why))
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
