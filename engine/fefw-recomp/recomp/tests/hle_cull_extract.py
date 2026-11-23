# SPDX-License-Identifier: GPL-3.0-or-later
"""Writes cull_fns.inc for hle_cull_test.cpp: the game's own two culling tests (the inline_replace targets of games/fefw/config.py),
renamed g_<addr>. With the replacement in place the lift holds each original as `static void o_<addr>`; a lift made without it
(NXRECOMP_OPT=none, or before the replacement existed) holds it as `void f_<addr>`. Both are found. The functions are game code:
generate the file locally from your own dump's lift, never commit it.

    python recomp/tests/hle_cull_extract.py <lift>/main recomp/tests/cull_fns.inc
    (then build hle_cull_test.cpp with -I recomp/include -I recomp/tests, together with game-build/fefw/src/fefw_hle.cpp)
"""
import glob
import os
import re
import sys

WANT = ['71019786c0', '7101980580']
head = re.compile(r'^(?:__attribute__\(\(section\("[^"]*"\)\)\) )?(?:static )?void [fo]_([0-9a-f]+)\(Ctx \*__restrict (?:NX_NOESC )?c\) \{$')
bodies = {}
for f in sorted(glob.glob(os.path.join(sys.argv[1], 'c_*.cpp'))):
    lines = open(f, encoding='utf-8').read().split('\n')
    i = 0
    while i < len(lines):
        m = head.match(lines[i])
        if m and m.group(1) in WANT and (m.group(1) not in bodies or ' o_' in lines[i]):
            j = i
            while lines[j] != '}':
                j += 1
            body = lines[i + 1:j]
            if not any('nx_hle_cull_' in b for b in body):      # the replacement's own wrapper is not the original
                bodies[m.group(1)] = 'void g_%s(Ctx *__restrict c) {\n%s\n}' % (m.group(1), '\n'.join(body))
            i = j + 1
        else:
            i += 1
missing = [w for w in WANT if w not in bodies]
if missing:
    sys.exit('missing: ' + ' '.join(missing))
open(sys.argv[2], 'w', newline='\n').write('\n\n'.join(bodies[w] for w in WANT) + '\n')
print('wrote', sys.argv[2])
