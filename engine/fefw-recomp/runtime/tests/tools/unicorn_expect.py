# SPDX-License-Identifier: GPL-3.0-or-later
"""Expected output of an S1 end-to-end program: runs it under Unicorn (recomp/oracle/e2e/unicorn_side.py, the reference of the
lifter's oracle) and writes its results[] array (512 little-endian u64) to a file.

    python unicorn_expect.py <prog.elf> <out.expected>
Exit code 0 = the program ran to its return address."""
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
E2E = os.path.abspath(os.path.join(HERE, '..', '..', '..', 'recomp', 'oracle', 'e2e'))
sys.path.insert(0, E2E)
import unicorn_side   # noqa: E402


def main():
    elf, out = sys.argv[1], sys.argv[2]
    res = unicorn_side.run(elf, 'max')
    if not res['finished']:
        print('unicorn did not reach the return address (pc=%x)' % res['pc'])
        return 1
    words = res.get('results')
    if not words:
        print('the program has no results[] array')
        return 1
    words = (list(words) + [0] * 512)[:512]
    with open(out, 'wb') as f:
        f.write(struct.pack('<512Q', *words))
    print('%s: %d non-zero words' % (out, sum(1 for w in words if w)))
    return 0


if __name__ == '__main__':
    sys.exit(main())
