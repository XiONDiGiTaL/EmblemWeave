#!/usr/bin/env python3
"""Write release notes from the checked-in changelog and the built archives.

The workflow and `tools/release.ps1` both use this shape: install steps, the
SHA-256 of each asset that is actually uploaded, then the changelog body.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--version', required=True)
    ap.add_argument('--changelog', required=True)
    ap.add_argument('--zip', required=True)
    ap.add_argument('--tar', default='')
    ap.add_argument('--out', required=True)
    args = ap.parse_args()
    version = args.version.lstrip('v')
    lines = Path(args.changelog).read_text(encoding='utf-8').splitlines()
    if not lines or not lines[0].startswith('# '):
        raise SystemExit('the changelog must start with "# EmblemWeave <version>"')
    summary, body_start = '', 1
    for i, line in enumerate(lines[1:], 1):
        if not line.strip():
            continue
        if line.startswith('> '):
            summary, body_start = line[2:].strip(), i + 1
        break
    if not summary:
        raise SystemExit("the changelog needs a '> one line' summary")
    body = '\n'.join(lines[body_start:]).strip()

    zip_name = Path(args.zip).name
    zip_sha = sha256(args.zip)
    tar_sha = sha256(args.tar) if args.tar else ''
    text = [
        f'# EmblemWeave {version}',
        '',
        summary,
        '',
        '## Install',
        '',
        f'1. Download **{zip_name}** below and unzip it into a folder of its own.',
        '2. Run `EmblemWeave.exe`.',
        '3. On **Game Setup**, choose your dump. On **Overview**, press **Build the game** (once), then **Play**.',
        '',
        f'`sha256` of the zip: `{zip_sha}`',
    ]
    if args.tar:
        text += [
            '',
            f'`sha256` of `{Path(args.tar).name}` (the shader translator for Linux, not needed on Windows): `{tar_sha}`',
            '',
            'The Linux tarball is built by CI on Linux, so its executable bit is recorded. Extract it in `engine/fefw-recomp/tools/shaders/prebuilt/` and run `./linux-x64/spirv_harness`.',
        ]
    text += ['', body, '']
    Path(args.out).write_text('\n'.join(text), encoding='utf-8')
    print(f'wrote {args.out}')
    print(json.dumps({'version': version, 'zip': zip_name, 'zipSha': zip_sha}, indent=2))


if __name__ == '__main__':
    main()
