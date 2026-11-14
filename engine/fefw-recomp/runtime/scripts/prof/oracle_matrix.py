"""Frame-time oracle of S10: title and menu at several frame caps, before and after the display changes.

  python oracle_matrix.py [--caps 60,120,144,240,0] [--seconds 70] [--repeat 2] [--hogs 0] [--out work\\prof\\matrix.json]

"before" switches the display changes off with their own config keys (process priority class normal, host page queries, no prewarm, no
background pool import); "after" is the default. Runs alternate before / after per cap so a change in the machine's load hits both.
The analysed window skips the first 12 s (boot, logos) and ends 2 s before the run does. Prints a table; the JSON has every run.
"""
import argparse, json, os, statistics, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
from project_root import project_root
ROOT = project_root()
VARIANTS = {
    'before': ['--env', 'NXRT_SCHED_PRIORITY_CLASS=normal', '--env', 'NXRT_MEMORY_FAST_QUERY=0', '--env', 'NXRT_NVN_PREWARM=0', '--env', 'NXRT_NVN_POOL_PREFETCH=0'],
    'after': [],
}


def run_one(tag, cap, seconds, extra, hogs):
    cmd = [sys.executable, os.path.join(HERE, 'prof_run.py'), '--cap', str(cap), '--seconds', str(seconds), '--tag', tag, '--skip', '12', '--press', 'A@38']
    if hogs: cmd += ['--hogs', str(hogs)]
    cmd += extra
    out = subprocess.run(cmd, capture_output=True, text=True).stdout
    path = os.path.join(ROOT, 'work', 'prof', tag + '.json')
    r = json.load(open(path)) if os.path.exists(path) else {}
    busy = None
    for line in out.splitlines():
        if 'system CPU busy' in line:
            busy = int(line.rsplit('busy', 1)[1].strip().rstrip('%'))
    r['cpu_busy'] = busy
    return r


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--caps', default='60,120,144,240,0')
    ap.add_argument('--seconds', type=int, default=70)
    ap.add_argument('--repeat', type=int, default=2)
    ap.add_argument('--hogs', type=int, default=0)
    ap.add_argument('--variants', default='before,after')
    ap.add_argument('--out', default=os.path.join(ROOT, 'work', 'prof', 'matrix.json'))
    a = ap.parse_args()
    res = {}
    for cap in [int(c) for c in a.caps.split(',')]:
        for rep in range(a.repeat):
            for v in a.variants.split(','):
                tag = 'm_%s_%d_%d' % (v, cap, rep)
                r = run_one(tag, cap, a.seconds, VARIANTS[v], a.hogs)
                res.setdefault('%s@%d' % (v, cap), []).append(r)
                print('%-10s cap %-3d rep %d: %s' % (v, cap, rep, {k: r.get(k) for k in ('fps', 'p50', 'p99', 'max', 'stalls50', 'stalls100', 'cpu_busy')}), flush=True)
    json.dump(res, open(a.out, 'w'), indent=1)
    print('\n| cap | variant | fps | p50 ms | p99 ms | max ms | stalls >50 ms | stalls >100 ms | CPU busy % |')
    print('|---|---|---|---|---|---|---|---|---|')
    for key, runs in res.items():
        v, cap = key.split('@')
        med = lambda k: statistics.median([r.get(k, 0) or 0 for r in runs])
        print('| %s | %s | %.1f | %.2f | %.2f | %.0f | %.0f | %.0f | %.0f |' % ('uncapped' if cap == '0' else cap, v, med('fps'), med('p50'), med('p99'), med('max'), med('stalls50'), med('stalls100'), med('cpu_busy')))


if __name__ == '__main__':
    main()
