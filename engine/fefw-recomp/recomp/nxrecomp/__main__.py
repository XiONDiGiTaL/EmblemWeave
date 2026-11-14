# SPDX-License-Identifier: GPL-3.0-or-later
"""python -m nxrecomp <command> ...   (see README.md)

  lift   <input> <outdir> [--game NAME] [--kind nso|nx_elf|static_elf] [--base 0x..] [--name N] [--jobs J] [--shards S]
         [--patches FILE]            lift every function of one module
  cover  <input> [--game NAME] ...   per-instruction-word translation coverage of .text
  funcs  -> python -m nxrecomp.funcs
"""
import argparse
import sys


def spec_from(a):
    kind = a.kind
    if not kind:
        magic = open(a.input, 'rb').read(4)
        kind = 'nso' if magic == b'NSO0' else 'static_elf'
    return {'kind': kind, 'path': a.input, 'base': int(a.base, 16), 'name': a.name, 'game': a.game, 'patches': a.patches, 'sites': a.sites, 'index': a.index}


def main(argv=None):
    ap = argparse.ArgumentParser(prog='nxrecomp')
    sub = ap.add_subparsers(dest='cmd', required=True)
    for name in ('lift', 'cover'):
        p = sub.add_parser(name)
        p.add_argument('input')
        if name == 'lift':
            p.add_argument('outdir')
            p.add_argument('--jobs', type=int, default=8)
            p.add_argument('--shards', type=int, default=0)
        p.add_argument('--game', default='generic')
        p.add_argument('--kind', choices=['nso', 'nx_elf', 'static_elf'])
        p.add_argument('--base', default='0x7100000000')
        p.add_argument('--name', default='main')
        p.add_argument('--patches')
        p.add_argument('--index', type=int, help='module index (NxModule.index, import ids); default: position in the game config')
        p.add_argument('--sites', help='site list: patches, inline callouts, hooks (see program.load_sites)')
    a = ap.parse_args(argv)
    spec = spec_from(a)
    from . import program
    if a.cmd == 'lift':
        program.lift_module(spec, a.outdir, a.jobs, a.shards)
        return 0
    from .config import load_game
    img = program.open_image(spec)
    ok, bad, _ = program.cover(img, load_game(a.game))
    return 0 if not bad else 1


if __name__ == '__main__':
    sys.exit(main())
