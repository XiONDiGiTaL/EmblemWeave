"""Run mpr.exe in a window for a fixed time with the frame trace on, then print the report.

  python prof_run.py --exe game-build\\build\\display\\mpr.exe --cap 120 --seconds 40 --tag base120 [--vsync] [--env K=V ...] [--ini-extra "[sec]\\nkey=v"]

Runs in work\\run_prof_<tag> (its own controls.ini with the requested cap), shares work\\run\\bc7_cache. Output: work\\prof\\<tag>.trace,
<tag>.json (the numbers) and the report on stdout. Needs the game folder at <project root>\\game (romfs) as boot.py does.
"""
import argparse, ctypes, os, re, shutil, subprocess, sys, threading, time

from project_root import project_root
ROOT = project_root()
sys.path.insert(0, os.path.dirname(__file__))
import ftrace_report


class CpuLoad:
    """System-wide CPU busy percentage sampled once a second (GetSystemTimes): how noisy the machine was during a run."""
    def __init__(self):
        self.s = []; self.stop = False
        self.t = threading.Thread(target=self.run, daemon=True); self.t.start()
    @staticmethod
    def times():
        i, k, u = (ctypes.c_ulonglong(), ctypes.c_ulonglong(), ctypes.c_ulonglong())
        ctypes.windll.kernel32.GetSystemTimes(ctypes.byref(i), ctypes.byref(k), ctypes.byref(u))
        return i.value, k.value + u.value
    def run(self):
        a = self.times()
        while not self.stop:
            time.sleep(1); b = self.times()
            di, dt = b[0] - a[0], b[1] - a[1]
            if dt: self.s.append(100.0 * (1 - di / dt))
            a = b
    def mean(self):
        self.stop = True
        return sum(self.s) / len(self.s) if self.s else 0.0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--exe', default=os.path.join(ROOT, 'game-build', 'build', 'display', 'mpr.exe'))
    ap.add_argument('--cap', type=int, default=60)
    ap.add_argument('--seconds', type=int, default=40)
    ap.add_argument('--skip', type=float, default=10)
    ap.add_argument('--tag', default='run')
    ap.add_argument('--vsync', action='store_true')
    ap.add_argument('--size', default='1280x720')
    ap.add_argument('--env', action='append', default=[])
    ap.add_argument('--press', default='', help='NXRT_BOOT_PRESS, e.g. A@12')
    ap.add_argument('--shots', default='', help='NXRT_BOOT_SCREENSHOT_AT seconds')
    ap.add_argument('--shots-every', default='', help='NXRT_BOOT_SCREENSHOT_EVERY seconds')
    ap.add_argument('--set', action='append', default=[], help='[display] key=value in controls.ini, e.g. render_scale=200')
    ap.add_argument('--fresh-cache', action='store_true', help='start without pipeline_cache.bin')
    ap.add_argument('--no-report', action='store_true')
    ap.add_argument('--ini-append', default='', help='text appended to the run folder controls.ini (for example "[fps]\nin_screenshots = 1"; \n is a new line)')
    ap.add_argument('--hogs', type=int, default=0, help='run N busy-loop processes (normal priority) during the run: a repeatable background load')
    ap.add_argument('--prio', default='', help='process priority class of mpr.exe: high | above | idle (default normal)')
    a = ap.parse_args()
    run = os.path.join(ROOT, 'work', 'run_prof_' + a.tag)
    os.makedirs(run, exist_ok=True)
    os.makedirs(os.path.join(ROOT, 'work', 'prof'), exist_ok=True)
    ini = open(os.path.join(ROOT, 'work', 'run', 'controls.ini')).read()
    w, h = a.size.split('x')
    for k, v in (('fps_cap', a.cap), ('vsync', int(a.vsync)), ('width', w), ('height', h)):
        ini = re.sub(r'(?m)^%s\s*=.*$' % k, '%s = %s' % (k, v), ini)
    for kv in a.set:
        k, v = kv.split('=', 1)
        if re.search(r'(?m)^%s\s*=' % k, ini): ini = re.sub(r'(?m)^%s\s*=.*$' % k, '%s = %s' % (k, v), ini)
        else: ini = ini.replace('[display]', '[display]' + chr(10) + '%s = %s' % (k, v), 1)
    if a.ini_append: ini += chr(10) + a.ini_append.replace(chr(92) + 'n', chr(10)) + chr(10)
    open(os.path.join(run, 'controls.ini'), 'w').write(ini)
    trace = os.path.join(ROOT, 'work', 'prof', a.tag + '.trace')
    if a.fresh_cache and os.path.exists(os.path.join(run, 'pipeline_cache.bin')):
        os.remove(os.path.join(run, 'pipeline_cache.bin'))
    env = dict(os.environ)
    env.update(NXRT_FS_ROMFS=os.path.join(ROOT, 'game', 'romfs'), NXRT_FS_SAVE_DIR=os.path.join(run, 'save'), NXRT_BOOT_GFX='window',
               NXRT_NVN_BC7_CACHE=os.path.join(ROOT, 'work', 'run', 'bc7_cache'), NXRT_PROF_TRACE=trace, NXRT_LOG_LEVEL='info')
    if a.press: env['NXRT_BOOT_PRESS'] = a.press
    if a.shots: env['NXRT_BOOT_SCREENSHOT_AT'] = a.shots
    if a.shots_every: env['NXRT_BOOT_SCREENSHOT_EVERY'] = a.shots_every
    for kv in a.env:
        k, v = kv.split('=', 1); env[k] = v
    cmd = [a.exe, '--log', 'mpr.log', '--max-seconds', str(a.seconds), '--link', 'rtld']
    t = time.time()
    flags = {'high': 0x80, 'above': 0x8000, 'idle': 0x40, 'realtime': 0x100}.get(a.prio, 0)
    hogs = [subprocess.Popen([sys.executable, '-c', 'while True: pass']) for _ in range(a.hogs)]
    time.sleep(1 if hogs else 0)
    load = CpuLoad()
    p = subprocess.Popen(cmd, cwd=run, env=env, creationflags=flags, stdout=open(os.path.join(run, 'run.out'), 'w'), stderr=subprocess.STDOUT)
    try:
        rc = p.wait(timeout=a.seconds + 25)
    except subprocess.TimeoutExpired:
        p.kill(); rc = 124
    for h in hogs: h.kill()
    print('exit %d after %.1fs (run dir %s), system CPU busy %.0f%%' % (rc, time.time() - t, run, load.mean()))
    if a.no_report: return rc
    r, *_ = ftrace_report.analyse(trace, a.skip, a.seconds - a.skip - 2, 50, 6)
    import json
    json.dump(r, open(os.path.join(ROOT, 'work', 'prof', a.tag + '.json'), 'w'), indent=1)
    if r.get('frames'):
        print('%s: frames %d over %.1fs = %.1f fps | interval ms p50 %.2f p90 %.2f p99 %.2f p99.9 %.2f max %.1f | stalls >50ms %d, >100ms %d' % (
            a.tag, r['frames'], r['seconds'], r['fps'], r['p50'], r['p90'], r['p99'], r['p999'], r['max'], r['stalls50'], r['stalls100']))
        for z in r['zones'][:14]: print('  %-14s %-16s %7d %10.1f %9.2f' % tuple(z))
        for s in r['worst'][:4]:
            print('  worst at %.1fs %.1f ms: %s' % (s['at_s'], s['ms'], '; '.join('%s/%s %.0fms' % (z[0], z[1], z[2]) for z in s['zones'][:4])))
    else:
        print('no frames traced')
    return rc


if __name__ == '__main__':
    sys.exit(main())
