"""Screenshot oracle of S10: title and main menu at several window sizes (internal resolution = window size).

  python oracle_shots.py [--sizes 1920x1080,2560x1440,3840x2160,2560x1080,3840x1080] [--out work\\shots_s10] [--set key=value ...]

For each size: boots the game in a window of that size, takes a screenshot at 28 s (title) and one at 50 s (after A was pressed at 34 s: main
menu), copies them to <out>\\<size>_title.png and <size>_menu.png and checks the picture size equals the window size, that the title shows
the lit "press A" text and the menu the lit entries (bright pixel counts), and that the text is not stretched (the bounding box of the bright
pixels of the title text has a height to width ratio within 8 % of the same text in the 1280x720 reference run, which is run first).
Prints one PASS/FAIL line per picture and a table.
"""
import argparse, os, shutil, subprocess, sys

import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
from project_root import project_root
ROOT = project_root()


def run(size, tag, sets):
    cmd = [sys.executable, os.path.join(HERE, 'prof_run.py'), '--cap', '60', '--seconds', '58', '--tag', tag, '--size', size, '--shots', '28', '--shots-every', '22',
           '--press', 'A@34', '--no-report']
    for s in sets: cmd += ['--set', s]
    subprocess.run(cmd, capture_output=True, text=True)
    d = os.path.join(ROOT, 'work', 'run_prof_' + tag)
    return [os.path.join(d, 'shot_%d.png' % i) for i in (0, 1)]


def bright_bbox(path, thr=120, centre=False):
    a = np.asarray(Image.open(path).convert('L'))
    if centre:      # the centred "PRESS A TO START" text only: not the copyright line at the right edge
        h, w = a.shape
        m = np.zeros_like(a, dtype=bool)
        m[int(0.7 * h):int(0.95 * h), int(0.3 * w):int(0.7 * w)] = True
        a = np.where(m, a, 0)
    ys, xs = np.where(a > thr)
    if len(xs) < 50: return None
    return a.shape[1], a.shape[0], xs.min(), xs.max(), ys.min(), ys.max(), len(xs)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--sizes', default='1920x1080,2560x1440,3840x2160,2560x1080,3840x1080')
    ap.add_argument('--out', default=os.path.join(ROOT, 'work', 'shots_s10'))
    ap.add_argument('--set', action='append', default=[])
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    rows = []
    ref = None
    for size in ['1280x720'] + a.sizes.split(','):
        tag = 'shots_' + size
        title, menu = run(size, tag, a.set)
        w, h = [int(v) for v in size.split('x')]
        for name, src in (('title', title), ('menu', menu)):
            dst = os.path.join(a.out, '%s_%s.png' % (size, name))
            ok_file = os.path.exists(src)
            if ok_file: shutil.copy(src, dst)
            bb = bright_bbox(dst, centre=(name == 'title')) if ok_file else None
            size_ok = ok_file and Image.open(dst).size == (w, h)
            rows.append((size, name, ok_file, size_ok, bb))
            if size == '1280x720' and name == 'title' and bb: ref = (bb[3] - bb[2] + 1, bb[5] - bb[4] + 1)
            print('%-10s %-6s file %s size %s bright %s' % (size, name, ok_file, size_ok, bb[6] if bb else None), flush=True)
    print('\nstretch check (title text bbox height/width against the 1280x720 reference %s):' % (ref,))
    fails = 0
    for size, name, ok_file, size_ok, bb in rows:
        if not ok_file or not size_ok or bb is None:
            print('FAIL', size, name, 'missing or wrong size or nothing lit'); fails += 1; continue
        if name == 'title' and ref:
            bw, bh = bb[3] - bb[2] + 1, bb[5] - bb[4] + 1
            r = (bh / bw) / (ref[1] / ref[0])
            ok = abs(r - 1) < 0.08
            print('%s %-10s title text aspect ratio vs reference %.3f' % ('PASS' if ok else 'FAIL', size, r)); fails += 0 if ok else 1
        else:
            print('PASS', size, name)
    print('\nFAILED' if fails else '\nALL PASS')
    sys.exit(1 if fails else 0)


if __name__ == '__main__':
    main()
