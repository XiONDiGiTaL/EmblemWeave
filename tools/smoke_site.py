#!/usr/bin/env python3
"""Post-deploy check for the site a release just changed.

A release is not done when the archive uploads: this checks the live page for
the version it stamps, the stylesheet URL that carries that version, and the
palette inside the CSS. It also compares `release.json` with GitHub's latest
release, so a stale site is a failed check instead of a surprise for a player.

  python tools/smoke_site.py --url https://xiondigital.net/EmblemWeave/
  python tools/smoke_site.py --url ... --expect-version 0.1.34
  python tools/smoke_site.py --url ... --retries 30 --delay 20  # wait out a Pages deploy
"""

import argparse
import json
import re
import sys
import time
import urllib.error
import urllib.request

PALETTE = {
    '--bg': '#050607',
    '--panel': '#0c0e11',
    '--ink': '#ecedef',
    '--body': '#bdc3cc',
    '--blue': '#3f80ff',
    '--blue-hi': '#79a8ff',
}
API = 'https://api.github.com/repos/XiONDiGiTaL/EmblemWeave/releases/latest'


def get(url, timeout=20):
    req = urllib.request.Request(url, headers={'User-Agent': 'EmblemWeave-site-smoke', 'Cache-Control': 'no-cache'})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.status, r.read().decode('utf-8', 'replace')


def latest_version():
    try:
        _, text = get(API)
        rel = json.loads(text)
        return str(rel.get('tag_name', '')).lstrip('v')
    except Exception:
        return ''


def _same_version(url, version):
    """A cache revision (0.1.34-2) is the same release as 0.1.34."""
    found = url.split('?v=', 1)[-1] if '?v=' in url else ''
    return found == version or found.startswith(version + '-')


def css_vars(text):
    out = {}
    for name in PALETTE:
        m = re.search(re.escape(name) + r'\s*:\s*([^;]+);', text)
        if m:
            out[name] = m.group(1).strip().lower()
    return out


def check(base, expect):
    problems = []
    base = base.rstrip('/') + '/'
    _, html = get(base)
    _, release_text = get(base + 'release.json')
    release = json.loads(release_text)
    version = str(release.get('version', '')).lstrip('v')
    expected = expect or latest_version()
    if expected and version != expected:
        problems.append(f'release.json says {version or "nothing"}, the latest release is {expected}')

    m = re.search(r'<link[^>]+href="([^"]*site\.css[^"]*)"', html)
    if not m:
        problems.append('the page has no site.css stylesheet link')
        css_url = ''
    else:
        href = m.group(1)
        css_url = href if href.startswith('http') else base + href.lstrip('/')
        if '?v=' not in href:
            problems.append(f'the stylesheet URL has no version query: {href}')
        elif version and not _same_version(href, version):
            problems.append(f'the stylesheet URL says {href}, release.json says {version}')

    if f'id="version">v{version}<' not in html:
        problems.append(f'the version chip is not stamped with v{version}')
    if f'id="badge-release">release {version}<' not in html:
        problems.append(f'the release badge is not stamped with {version}')
    download = re.search(r'id="download"[^>]*href="([^"]*)"', html)
    if not download:
        problems.append('the download button is missing')
    elif '/releases/download/' not in download.group(1) or not download.group(1).endswith('.zip'):
        problems.append(f'the download button does not point at a release asset: {download.group(1)}')
    elif release.get('download') and download.group(1) != release['download']:
        problems.append('the download button and release.json name different assets')
    script = re.search(r'<script[^>]+src="([^"]*site\.js[^"]*)"', html)
    if not script:
        problems.append('site.js is not loaded: the auto-latest badge and download cannot run')
    elif '?v=' not in script.group(1):
        problems.append(f'the site.js URL has no version query: {script.group(1)}')
    elif version and not _same_version(script.group(1), version):
        problems.append(f'the site.js URL says {script.group(1)}, release.json says {version}')

    if css_url:
        _, css = get(css_url)
        found = css_vars(css)
        for name, want in PALETTE.items():
            if found.get(name) != want:
                problems.append(f'{name} is {found.get(name) or "missing"}, expected {want}')
    return version, problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--url', default='https://xiondigital.net/EmblemWeave/')
    ap.add_argument('--expect-version', default='')
    ap.add_argument('--retries', type=int, default=1, help='attempts; the workflow uses this to wait out a deploy')
    ap.add_argument('--delay', type=float, default=10.0)
    args = ap.parse_args()
    last = []
    for attempt in range(max(1, args.retries)):
        if attempt:
            time.sleep(args.delay)
        try:
            version, problems = check(args.url, args.expect_version)
        except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, ValueError, json.JSONDecodeError) as e:
            problems = [f'could not read the site: {e}']
            version = ''
        if not problems:
            print(f'site smoke ok: {args.url} serves release {version}, versioned CSS and the XiON palette')
            return 0
        last = problems
        print(f'attempt {attempt + 1}/{args.retries}: ' + '; '.join(problems), file=sys.stderr)
    print('site smoke failed:', file=sys.stderr)
    for p in last:
        print('  ' + p, file=sys.stderr)
    return 1


if __name__ == '__main__':
    sys.exit(main())
