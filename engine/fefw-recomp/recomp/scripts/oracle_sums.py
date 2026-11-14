# SPDX-License-Identifier: GPL-3.0-or-later
"""Write or check oracle/SHA256SUMS (the hash lock of the oracle scripts).

    python scripts/oracle_sums.py write      list every file under oracle/ (LF line endings) with its SHA-256
    python scripts/oracle_sums.py check      exit 1 if a file changed, appeared or disappeared
The format is that of `sha256sum`: "<hash>  <path relative to oracle/>".
"""
import hashlib
import os
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
ORACLE = os.path.join(ROOT, 'oracle')


def files():
    out = []
    for d, dirs, names in os.walk(ORACLE):
        dirs[:] = sorted(x for x in dirs if x != '__pycache__')
        for n in sorted(names):
            if n == 'SHA256SUMS' or n.endswith('.pyc'):
                continue
            out.append(os.path.relpath(os.path.join(d, n), ORACLE).replace(chr(92), '/'))
    return sorted(out)


def digest(rel):
    data = open(os.path.join(ORACLE, rel), 'rb').read().replace(b'\r\n', b'\n')
    return hashlib.sha256(data).hexdigest()


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else 'check'
    path = os.path.join(ORACLE, 'SHA256SUMS')
    if cmd == 'write':
        with open(path, 'w', newline='\n') as f:
            for rel in files():
                f.write('%s  %s\n' % (digest(rel), rel))
        print('wrote %d entries' % len(files()))
        return 0
    want = {}
    for line in open(path):
        h, rel = line.rstrip('\n').split('  ', 1)
        want[rel] = h
    bad = [rel for rel in files() if want.get(rel) != digest(rel)] + [rel for rel in want if rel not in files()]
    for rel in bad:
        print('CHANGED/MISSING/NEW:', rel)
    print('oracle sums: %s (%d files)' % ('OK' if not bad else 'MISMATCH', len(want)))
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
