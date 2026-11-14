# SPDX-License-Identifier: GPL-3.0-or-later
"""Prints the non-zero words of a results[] dump (NRES little-endian u64) as `index: hex`; the last word is the program's hash."""
import struct
import sys

data = open(sys.argv[1], 'rb').read()
words = struct.unpack('<%dQ' % (len(data) // 8), data)
last = max((i for i, w in enumerate(words) if w), default=-1)
for i, w in enumerate(words):
    if w and (i < 40 or i == len(words) - 1 or i > last - 3):
        print('%4d: %x' % (i, w))
print('nonzero words: %d of %d' % (sum(1 for w in words if w), len(words)))
