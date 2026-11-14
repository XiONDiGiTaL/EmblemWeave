# SPDX-License-Identifier: GPL-3.0-or-later
"""Count subgroup-dependent modules in a translated shader cache.

Usage: python tools/shaders/inspect_subgroups.py CACHE [--manifest CORPUS/manifest.tsv]
Capabilities are deliberately ignored: the translator declares them unconditionally.
"""
import argparse
import collections
import csv
import struct
from pathlib import Path


def inspect(path):
    data = path.read_bytes()
    words = struct.unpack('<%dI' % (len(data) // 4), data)
    stage, used, instructions = None, False, collections.Counter()
    i = 5
    while i < len(words):
        n, op = words[i] >> 16, words[i] & 0xffff
        if not n or i + n > len(words):
            raise ValueError(f'{path}: invalid SPIR-V instruction at {i}')
        if op == 15:
            stage = words[i + 1]
        if 333 <= op <= 366:
            used = True
            instructions[op] += 1
        if op == 71 and n >= 4 and words[i + 2] == 11:
            builtin = words[i + 3]
            used |= 36 <= builtin <= 41 or 4416 <= builtin <= 4420
        i += n
    return stage, used, instructions


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('cache', type=Path)
    parser.add_argument('--manifest', type=Path)
    args = parser.parse_args()
    total, dependent, ops = (collections.Counter() for _ in range(3))
    hashes = set()
    for path in sorted(args.cache.glob('*.spv')):
        stage, used, instructions = inspect(path)
        total[stage] += 1
        ops.update(instructions)
        if used:
            dependent[stage] += 1
            hashes.add(path.stem)
    names = {0: 'vertex', 1: 'tess-control', 2: 'tess-eval', 3: 'geometry', 4: 'fragment', 5: 'compute'}
    for stage in sorted(total):
        print(f'{names.get(stage, stage)}: {dependent[stage]} subgroup-dependent / {total[stage]} total')
    print('Subgroup instruction occurrences:', dict(sorted(ops.items())))
    if args.manifest:
        sources = collections.defaultdict(set)
        with args.manifest.open(encoding='utf-8') as f:
            for row in csv.DictReader(f, delimiter='\t'):
                if row['code_sha1_16'] in hashes:
                    sources[row['source']].add(row['code_sha1_16'])
        for source, shaders in sorted(sources.items()):
            print(f'{source}: {len(shaders)} distinct subgroup-dependent modules')


if __name__ == '__main__':
    main()
