"""Report on a frame trace written with [prof] trace = <file> (include/nxrt/ftrace.h).

  python ftrace_report.py trace.txt [--skip 8] [--seconds 60] [--stall-ms 50] [--top 8] [--chrome out.json] [--json out.json]

Frames are the "frame" marks (one per present, after the limiter wait). The window starts --skip seconds after the first frame and
lasts --seconds. Prints the interval percentiles, the stall count, the time each zone takes per thread, and for the worst stalls
the zones (>= 3 ms) that overlap them on any thread.
"""
import argparse, collections, json, sys


def svc_names():
    import os, re
    f = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'src', 'kernel', 'svc_names.inc')
    d = {}
    try:
        for m in re.finditer(r'NXRT_SVC\((0x[0-9a-fA-F]+),\s*"([^"]+)"', open(f).read()): d[int(m.group(1), 16)] = m.group(2)
    except OSError:
        pass
    return d


SVC = svc_names()


def zname(zones, z, a):
    n = zones.get(z, '?%d' % z)
    return 'svc:' + SVC.get(a, '%d' % a) if n == 'svc' else n


def load(path):
    zones, threads, ev = {}, {}, []
    with open(path, 'r', errors='replace') as f:
        for line in f:
            if line.startswith('# zone '):
                p = line.split(None, 3); zones[int(p[2])] = p[3].strip()
            elif line.startswith('# thread '):
                p = line.split(None, 4); threads[int(p[2])] = (int(p[3]), p[4].strip() if len(p) > 4 else '-')
            elif line and line[0] != '#':
                p = line.split()
                if len(p) == 5:
                    ev.append((int(p[0]), int(p[1]), int(p[2]), int(p[3]), int(p[4])))
    return zones, threads, ev


def pct(v, q):
    if not v: return 0.0
    v = sorted(v); i = min(len(v) - 1, int(q * len(v)))
    return v[i]


def analyse(path, skip=8.0, seconds=60.0, stall_ms=50.0, top=8):
    zones, threads, ev = load(path)
    name = lambda z: zones.get(z, '?%d' % z)
    zn = lambda z, a: zname(zones, z, a)
    frames = sorted(e[0] for e in ev if name(e[3]) == 'frame')
    out = {'frames': 0}
    if len(frames) < 3:
        return out, zones, threads, ev
    t0 = frames[0] + int(skip * 1e9)
    t1 = t0 + int(seconds * 1e9)
    fr = [t for t in frames if t0 <= t <= t1]
    if len(fr) < 3:
        fr = frames; t0, t1 = frames[0], frames[-1]
    iv = [(fr[i] - fr[i - 1]) / 1e6 for i in range(1, len(fr))]
    span = (fr[-1] - fr[0]) / 1e9
    out.update(frames=len(iv), seconds=round(span, 2), fps=round(len(iv) / span, 2) if span else 0,
               p50=round(pct(iv, .5), 3), p90=round(pct(iv, .9), 3), p99=round(pct(iv, .99), 3), p999=round(pct(iv, .999), 3), max=round(max(iv), 2),
               stalls50=sum(1 for d in iv if d > 50), stalls100=sum(1 for d in iv if d > 100), stalls_user=sum(1 for d in iv if d > stall_ms))
    # zone totals per thread inside the window
    tot = collections.defaultdict(lambda: [0, 0, 0])
    for t, d, th, z, a in ev:
        if t < t0 or t > t1 or name(z) == 'frame' or name(z) == 'main.update': continue
        k = (threads.get(th, (0, '?'))[1], zn(z, a))
        r = tot[k]; r[0] += 1; r[1] += d; r[2] = max(r[2], d)
    out['zones'] = sorted(([k[0], k[1], v[0], round(v[1] / 1e6, 1), round(v[2] / 1e6, 2)] for k, v in tot.items()), key=lambda r: -r[3])
    # worst stalls
    idx = sorted(range(len(iv)), key=lambda i: -iv[i])[:top]
    st = []
    for i in sorted(idx):
        a, b = fr[i], fr[i + 1]
        ov = collections.defaultdict(float)
        for t, d, th, z, ar in ev:
            if d >= 1e6 and t < b and t + d > a and name(z) not in ('frame', 'main.update'):
                ov[(threads.get(th, (0, '?'))[1], zn(z, ar))] += (min(b, t + d) - max(a, t)) / 1e6
        ovl = sorted(([k[0], k[1], round(v, 1)] for k, v in ov.items()), key=lambda r: -r[2])
        st.append({'at_s': round((a - frames[0]) / 1e9, 2), 'ms': round(iv[i], 1), 'zones': ovl[:14]})
    out['worst'] = st
    return out, zones, threads, ev


def chrome(path, outp):
    zones, threads, ev = load(path)
    te = []
    for th, (tid, nm) in threads.items():
        te.append({'ph': 'M', 'name': 'thread_name', 'pid': 1, 'tid': th, 'args': {'name': '%s (%d)' % (nm, tid)}})
    for t, d, th, z, a in ev:
        n = zones.get(z, '?')
        if d == 0: te.append({'ph': 'i', 'name': n, 'pid': 1, 'tid': th, 'ts': t / 1000, 's': 't'})
        else: te.append({'ph': 'X', 'name': n if n != 'svc' else 'svc %d' % a, 'pid': 1, 'tid': th, 'ts': t / 1000, 'dur': d / 1000, 'args': {'arg': a}})
    json.dump({'traceEvents': te}, open(outp, 'w'))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('trace'); ap.add_argument('--skip', type=float, default=8); ap.add_argument('--seconds', type=float, default=60)
    ap.add_argument('--stall-ms', type=float, default=50); ap.add_argument('--top', type=int, default=8)
    ap.add_argument('--chrome'); ap.add_argument('--json')
    a = ap.parse_args()
    r, zones, threads, ev = analyse(a.trace, a.skip, a.seconds, a.stall_ms, a.top)
    if a.chrome: chrome(a.trace, a.chrome)
    if a.json: json.dump(r, open(a.json, 'w'), indent=1)
    if not r.get('frames'):
        print('no frames in the trace'); return 1
    print('frames %d over %.1fs = %.1f fps | interval ms p50 %.2f p90 %.2f p99 %.2f p99.9 %.2f max %.1f | stalls >50ms %d, >100ms %d' % (
        r['frames'], r['seconds'], r['fps'], r['p50'], r['p90'], r['p99'], r['p999'], r['max'], r['stalls50'], r['stalls100']))
    print('\nzone time in the window (thread, zone, count, total ms, max ms):')
    for z in r['zones'][:30]: print('  %-14s %-16s %7d %10.1f %9.2f' % tuple(z))
    print('\nworst intervals:')
    for s in r['worst']:
        print('  at %.2fs  %.1f ms' % (s['at_s'], s['ms']))
        for z in s['zones']: print('      %-14s %-22s %7.1f ms' % tuple(z))
    return 0


if __name__ == '__main__':
    sys.exit(main())
