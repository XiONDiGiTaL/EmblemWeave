#!/usr/bin/env python3
"""Collapse a shader corpus (tools/survey/extract_shaders.py layout) to one block per distinct Maxwell CODE block.

The survey keys a block by sha1(code + control section) (67,506 distinct blocks for Metroid Prime Remastered). The runtime
only has the code block and its control pointer (no control length), and the translated SPIR-V depends on the code block
alone: 9,750 of the 13,016 distinct code blocks occur with several control sections, and every group checked so far gave
byte-identical SPIR-V. This script writes a corpus with one entry per distinct code block, keyed by the first 16 hex digits
of sha1(code block bytes), which is the key the runtime computes (nvn_shaders.cpp: sha1 of the `code_size` bytes at the
NVNshaderData address; the size is the u32 at control +0x704, and it equals the length of code_*.bin).

    python make_code_corpus.py <corpus dir> --out <new corpus dir> [--verify <results.tsv dir of a run on the old corpus>]

--verify: for every group with several control variants, compare the SPIR-V the old run produced for each variant
(<dir>/<oldhash>.spv, plus status and resources in results.tsv) and report differences (exit code 1 when any differ).
"""
import argparse, csv, hashlib, os, shutil, sys


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('corpus')
    ap.add_argument('--out', required=True)
    ap.add_argument('--verify', help='results dir of a harness run on the old corpus (results.tsv + <hash>.spv)')
    a = ap.parse_args()
    man = os.path.join(a.corpus, 'manifest.tsv')
    rows = list(csv.DictReader(open(man, newline=''), delimiter='\t'))
    groups = {}          # code hash -> list of old hashes (distinct blocks) in order of appearance
    rep = {}             # code hash -> first manifest row
    old_to_code = {}
    seen_old = set()
    for r in rows:
        old = r['code_sha1_16']
        if old in old_to_code:
            continue
        with open(os.path.join(a.corpus, r['code_file']), 'rb') as f:
            code = f.read()
        ch = hashlib.sha1(code).hexdigest()[:16]
        old_to_code[old] = ch
        groups.setdefault(ch, []).append(old)
        rep.setdefault(ch, r)
    os.makedirs(a.out, exist_ok=True)
    out_rows = []
    for ch in sorted(rep):
        r = dict(rep[ch])
        cf, kf = 'code_%s.bin' % ch, 'ctrl_%s.bin' % ch
        shutil.copyfile(os.path.join(a.corpus, rep[ch]['code_file']), os.path.join(a.out, cf))
        shutil.copyfile(os.path.join(a.corpus, rep[ch]['ctrl_file']), os.path.join(a.out, kf))
        r['code_sha1_16'] = ch
        r['code_file'] = cf
        r['ctrl_file'] = kf
        out_rows.append(r)
    with open(os.path.join(a.out, 'manifest.tsv'), 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()), delimiter='\t', lineterminator='\n')
        w.writeheader()
        w.writerows(out_rows)
    multi = {k: v for k, v in groups.items() if len(v) > 1}
    print(f'{len(old_to_code)} distinct (code+control) blocks -> {len(groups)} distinct code blocks; {len(multi)} code blocks have several control sections')
    bad = 0
    if a.verify:
        res = {r['hash']: r for r in csv.DictReader(open(os.path.join(a.verify, 'results.tsv'), newline=''), delimiter='\t')}
        checked = 0
        for ch, olds in multi.items():
            digests = set()
            meta = set()
            for o in olds:
                p = os.path.join(a.verify, o + '.spv')
                if not os.path.exists(p):
                    continue
                digests.add(hashlib.sha1(open(p, 'rb').read()).hexdigest())
                r = res.get(o)
                if r:
                    meta.add((r['status'].replace('ok-retry', 'ok'), r['resources']))
            checked += 1
            if len(digests) > 1 or len(meta) > 1:
                bad += 1
                if bad <= 10:
                    print('DIFFERENT:', ch, olds[:4], len(digests), 'spv variants,', len(meta), 'resource/status variants')
        print(f'verified {checked} multi-control groups: {bad} differ')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
