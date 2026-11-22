#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check that a release archive extracts the same way everywhere.

An archive entry has to be a relative path with `/` separators. Windows' own
zippers have written `engine\\recomp\\foo.cpp` for years, and an archive full of
those unpacks on Linux and macOS as one flat pile of files whose names contain
backslashes. An entry that starts with `/`, names a drive (`C:/...`) or contains
a `..` segment is worse: it writes outside the folder the player extracted into.
This script rejects all three, plus names that collide case-insensitively (two
entries that one filesystem merges and another keeps), and it can extract the
archive itself and check that every file arrived with its size.

  python tools/verify_archive.py target/pkg/EmblemWeave-v0.1.17-windows-x64.zip \
      --require EmblemWeave.exe --require engine/fefw-recomp/runtime/CMakeLists.txt
  python tools/verify_archive.py <archive> --extract target/verify/unpacked

Zip and tar (.tar, .tar.gz, .tgz) are read; a zip is checked for its entries and
sizes, a tar also for the executable bit of anything it ships as a program. The
exit code is 1 for any problem, so a release step can rely on it.
"""

import argparse
import gzip
import os
import re
import stat
import sys
import tarfile
import unicodedata
import zipfile

DRIVE = re.compile(r'^[A-Za-z]:')
# A name a Win32 call would cut at its first NUL, or that no filesystem accepts.
CONTROL = re.compile(r'[\x00-\x1f]')
# Characters Win32 does not allow in a file name. A release zip is opened on Windows by definition; an entry that
# cannot be created there is a packaging bug, not a platform preference.
WINDOWS_BAD = set('<>:"|?*')
WINDOWS_RESERVED = {'CON', 'PRN', 'AUX', 'NUL', *(f'COM{i}' for i in range(1, 10)), *(f'LPT{i}' for i in range(1, 10))}


def problems_for_name(name, where):
    """Why `name` is not a safe relative archive path, as a list of sentences."""
    out = []
    if not name:
        return [f'{where}: an entry with an empty name']
    if '\\' in name:
        out.append(f'{where}: {name!r} uses a backslash; archive paths use "/"')
    if CONTROL.search(name):
        out.append(f'{where}: {name!r} holds a control character (a NUL truncates it on Windows)')
    if name.startswith('/') or name.startswith('//'):
        out.append(f'{where}: {name!r} is an absolute path')
    if DRIVE.match(name):
        out.append(f'{where}: {name!r} names a drive letter')
    parts = name.rstrip('/').split('/')
    if any(p in ('..',) for p in parts):
        out.append(f'{where}: {name!r} walks up with ".."')
    if any(p == '.' for p in parts):
        out.append(f'{where}: {name!r} has a "." path segment')
    if any(p == '' for p in parts[:-1]):
        out.append(f'{where}: {name!r} has an empty path segment')
    if parts and parts[0] == '.':
        out.append(f'{where}: {name!r} starts with "./"')
    if name.startswith('~'):
        out.append(f'{where}: {name!r} starts with a tilde; it is not a relative path')
    for part in parts:
        if not part:
            continue
        if part[-1] in ' .':
            out.append(f'{where}: {name!r} has a segment ending in a space or dot; Windows strips it')
        bad = sorted(WINDOWS_BAD.intersection(part))
        if bad:
            out.append(f'{where}: {name!r} holds {bad!r}, which Win32 does not allow in a name')
        stem = part.split('.')[0].upper()
        if stem in WINDOWS_RESERVED:
            out.append(f'{where}: {name!r} uses the reserved Windows name {stem!r}')
    return out


def check_zip(path, extract, required):
    problems, files, dirs, names, normalized, seen = [], {}, set(), {}, {}, set()
    with zipfile.ZipFile(path) as z:
        entries = z.infolist()
        if not entries:
            problems.append(f'{path}: the archive is empty')
        for e in entries:
            name = e.filename
            problems.extend(problems_for_name(name, path))
            if name in seen:
                problems.append(f'{path}: duplicate archive entry {name!r}')
            seen.add(name)
            if e.flag_bits & 0x1:
                problems.append(f'{path}: {name!r} is encrypted; a release entry must be readable')
            mode = (e.external_attr >> 16) & 0xFFFF
            if stat.S_ISLNK(mode):
                problems.append(f'{path}: {name!r} is a symbolic link; the release ships plain files')
            key = name.rstrip('/').lower()
            if key in names and names[key] != name:
                problems.append(f'{path}: {names[key]!r} and {name!r} collide after case folding; one filesystem merges them and another does not')
            folded = unicodedata.normalize('NFC', key)
            previous = normalized.get(folded)
            if previous is not None and previous.casefold() != name.casefold():
                problems.append(f'{path}: {name!r} collides with another name after Unicode normalization')
            normalized[folded] = name
            names[key] = name
            if name.endswith('/'):
                dirs.add(name.rstrip('/').lower())
            else:
                if e.file_size == 0 and e.compress_size == 0 and e.is_dir():
                    continue
                files[name] = e.file_size
        # A path cannot be both a file and a folder: the extractor picks one and the other is lost.
        for d in dirs:
            if d in {k.lower() for k in files}:
                problems.append(f'{path}: {d!r} is both a file and a folder')
        for req in required:
            if req not in files:
                problems.append(f'{path}: required file {req!r} is missing')
        if extract:
            os.makedirs(extract, exist_ok=True)
            root = os.path.realpath(extract)
            for name, size in files.items():
                target = os.path.realpath(os.path.join(root, *name.split('/')))
                if target != root and not target.startswith(root + os.sep):
                    problems.append(f'{path}: {name!r} would extract outside {extract}')
                    continue
                os.makedirs(os.path.dirname(target), exist_ok=True)
                try:
                    with z.open(name) as src, open(target, 'wb') as dst:
                        while True:
                            chunk = src.read(1 << 20)
                            if not chunk:
                                break
                            dst.write(chunk)
                except (OSError, RuntimeError) as e:
                    problems.append(f'{path}: {name!r} could not be extracted: {e}')
                    continue
                if os.path.getsize(target) != size:
                    problems.append(f'{path}: {name!r} extracted as {os.path.getsize(target)} bytes, the entry says {size}')
            missing = [n for n in files if not os.path.isfile(os.path.join(root, *n.split('/')))]
            problems.extend(f'{path}: {n!r} did not extract' for n in missing)
    return problems, len(files), len(dirs)


def check_tar(path, extract, required):
    problems, files, dirs, names, normalized, modes, seen = [], {}, set(), {}, {}, {}, set()
    opener = gzip.open if path.endswith(('.gz', '.tgz')) else open
    with opener(path, 'rb') as raw, tarfile.open(fileobj=raw, mode='r:') as t:
        members = t.getmembers()
        if not members:
            problems.append(f'{path}: the archive is empty')
        for m in members:
            name = m.name
            problems.extend(problems_for_name(name, path))
            if name in seen:
                problems.append(f'{path}: duplicate archive entry {name!r}')
            seen.add(name)
            key = name.rstrip('/').lower()
            if key in names and names[key] != name:
                problems.append(f'{path}: {names[key]!r} and {name!r} differ only in case')
            folded = unicodedata.normalize('NFC', key)
            previous = normalized.get(folded)
            if previous is not None and previous.casefold() != name.casefold():
                problems.append(f'{path}: {name!r} collides with another name after Unicode normalization')
            normalized[folded] = name
            names[key] = name
            if m.isdir():
                dirs.add(name.rstrip('/').lower())
            elif m.isfile():
                files[name] = m.size
                modes[name] = m.mode
            else:
                problems.append(f'{path}: {name!r} is a link or device; the release ships plain files')
        for req in required:
            if req not in files:
                problems.append(f'{path}: required file {req!r} is missing')
        # A program a player runs has to arrive executable, or `./spirv_harness` is "permission denied".
        for name, mode in modes.items():
            if not name.endswith(('.dll', '.so')) and mode & 0o111 == 0 and '/.' not in name:
                if os.path.basename(name) in ('spirv_harness', 'EmblemWeave'):
                    problems.append(f'{path}: {name!r} is not executable (mode {oct(mode)})')
        if extract:
            os.makedirs(extract, exist_ok=True)
            root = os.path.realpath(extract)
            for name in files:
                target = os.path.realpath(os.path.join(root, *name.split('/')))
                if target != root and not target.startswith(root + os.sep):
                    problems.append(f'{path}: {name!r} would extract outside {extract}')
            try:
                t.extractall(root, filter='data') if hasattr(tarfile, 'data_filter') else t.extractall(root)
            except (OSError, tarfile.TarError) as e:
                problems.append(f'{path}: extraction failed: {e}')
            for name, size in files.items():
                got = os.path.join(root, *name.split('/'))
                if not os.path.isfile(got) or os.path.getsize(got) != size:
                    problems.append(f'{path}: {name!r} did not extract with its size')
    return problems, len(files), len(dirs)


def main():
    ap = argparse.ArgumentParser(description='Check a release archive: paths, duplicates, required files, extraction.')
    ap.add_argument('archives', nargs='+')
    ap.add_argument('--require', action='append', default=[], help='a path the archive must ship (repeatable)')
    ap.add_argument('--extract', default='', help='unpack here and check every file arrived')
    args = ap.parse_args()
    bad = 0
    for a in args.archives:
        if not os.path.isfile(a):
            print(f'{a}: no such file')
            bad += 1
            continue
        if a.endswith('.zip'):
            problems, n, d = check_zip(a, args.extract, args.require)
        elif a.endswith(('.tar', '.tar.gz', '.tgz')):
            problems, n, d = check_tar(a, args.extract, args.require)
        else:
            print(f'{a}: unknown archive type (zip, tar, tar.gz or tgz)')
            bad += 1
            continue
        if problems:
            print(f'{a}: {len(problems)} problem(s)')
            for p in problems[:40]:
                print('  ' + p)
            if len(problems) > 40:
                print(f'  ... and {len(problems) - 40} more')
            bad += 1
        else:
            print(f'{a}: ok, {n} files in {d} folders' + (f', extracted into {args.extract}' if args.extract else ''))
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
