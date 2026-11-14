#!/usr/bin/env python3
"""Turn deko3d .dksh shader modules (made by uam) into the harness's corpus layout.

The harness (harness/Program.cs, ported from SuperMarioWonderRecomp, MIT) reads "extracted" folders as written by
Wonder's containers.py: manifest.tsv plus code_<hash>.bin and ctrl_<hash>.bin per distinct code block, where ctrl is an
NVN shader control section (only a handful of its fields matter, see docs in README.md). A DKSH file holds the same two
things in deko3d's own layout, so this script re-wraps them:

  code block  = the DKSH code section (graphics: SPH at the program entrypoint, normally 0x30; compute: no SPH)
  ctrl        = 0x800 zero bytes with +0x6f8 program length, +0x6fc / +0x700 constant data (constbuf 1) size / offset,
                +0x708 program start, +0x714 NVN stage, +0x748..0x750 work group, +0x754 shared bytes

Usage: dksh_to_corpus.py <dir or .dksh files...> --out <dir> [--group NAME]
Output: <out>/manifest.tsv (Wonder's column layout), <out>/code_<h>.bin, <out>/ctrl_<h>.bin.
"""
import argparse
import hashlib
import os
import struct
import sys

DKSH_MAGIC = 0x48534B44
STAGE_NAMES = {0: 'vertex', 1: 'fragment', 2: 'geometry', 3: 'tess_control', 4: 'tess_eval', 5: 'compute'}
BRA_M, BRA_V = 0xfff0000000000000, 0xe240000000000000
EXIT_M, EXIT_V = 0xfff0000000000000, 0xe300000000000000


def program_end(code, start, limit):
    """Bytes from `start` to the end of the 32-byte group holding the program's last instruction: a program ends with a
    branch to itself (BRA -8), small ones with EXIT alone. Returns the remainder up to `limit` if neither is found."""
    q, last_exit = start, None
    while q + 32 <= limit:
        for k in (1, 2, 3):          # word 0 of each group of four is scheduling information
            w, = struct.unpack_from('<Q', code, q + 8 * k)
            if w & BRA_M == BRA_V and (w >> 20) & 0xffffff == 0xfffff8:
                return q + 32 - start
            if w & EXIT_M == EXIT_V:
                last_exit = q + 32 - start
        q += 32
    return last_exit if last_exit is not None else limit - start


def read_dksh(blob):
    magic, header_sz, control_sz, code_sz, programs_off, num = struct.unpack_from('<6I', blob, 0)
    if magic != DKSH_MAGIC:
        raise ValueError('not a DKSH file')
    code = blob[control_sz:control_sz + code_sz]
    for i in range(num):
        p = programs_off + 64 * i
        typ, entry, gprs, cb1_off, cb1_sz, scratch = struct.unpack_from('<6I', blob, p)
        extra = blob[p + 24:p + 64]
        comp = struct.unpack_from('<8I', extra, 0) if typ == 5 else (0,) * 8
        yield dict(type=typ, entry=entry, cb1_off=cb1_off, cb1_sz=cb1_sz, block=comp[0:3], shared=comp[3], code=code)


def make_ctrl(prog, length):
    c = bytearray(0x800)
    struct.pack_into('<I', c, 0x6f8, length)
    struct.pack_into('<II', c, 0x6fc, prog['cb1_sz'], prog['cb1_off'])
    struct.pack_into('<I', c, 0x708, prog['entry'])
    struct.pack_into('<I', c, 0x714, prog['type'])
    struct.pack_into('<3I', c, 0x748, *prog['block'])
    struct.pack_into('<I', c, 0x754, prog['shared'])
    return bytes(c)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('inputs', nargs='+')
    ap.add_argument('--out', required=True)
    ap.add_argument('--group', default='')
    a = ap.parse_args()
    files = []
    for p in a.inputs:
        if os.path.isdir(p):
            files += sorted(os.path.join(p, n) for n in os.listdir(p) if n.endswith('.dksh'))
        else:
            files.append(p)
    os.makedirs(a.out, exist_ok=True)
    seen, rows = set(), []
    for f in files:
        blob = open(f, 'rb').read()
        for i, prog in enumerate(read_dksh(blob)):
            code = prog['code']
            start = prog['entry']
            if prog['type'] == 5:
                length = program_end(code, start, len(code))
            else:
                length = 0x50 + program_end(code, start + 0x50, len(code))
            ctrl = make_ctrl(prog, length)
            h = hashlib.sha1(code + ctrl).hexdigest()[:16]
            cf, tf = f'code_{h}.bin', f'ctrl_{h}.bin'
            if h not in seen:
                seen.add(h)
                open(os.path.join(a.out, cf), 'wb').write(code)
                open(os.path.join(a.out, tf), 'wb').write(ctrl)
            ident = os.path.splitext(os.path.basename(f))[0] + (f'#{i}' if i else '')
            rows.append((ident, STAGE_NAMES.get(prog['type'], '?'), 0, len(code), 0, len(ctrl), start, length,
                         prog['cb1_off'], prog['cb1_sz'], h, cf, tf))
    with open(os.path.join(a.out, 'manifest.tsv'), 'w', newline='\n') as m:
        m.write('id\tstage\tcode_off\tcode_size\tctrl_off\tctrl_size\tprogram_start\tprogram_bytes\tconst_off\t'
                'const_size\tcode_sha1_16\tcode_file\tctrl_file\n')
        for r in rows:
            m.write('\t'.join(f'{x:#x}' if isinstance(x, int) and i in (2, 3, 4, 5, 6, 7, 8, 9) else str(x)
                              for i, x in enumerate(r)) + '\n')
    print(f'{len(rows)} programs, {len(seen)} distinct code blocks -> {a.out}')


if __name__ == '__main__':
    sys.exit(main())
