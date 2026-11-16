#!/usr/bin/env python3
"""Stamp `docs/index.html` and `docs/release.json` for a release.

The badge and download read the latest release at runtime, so no manual badge
edit is needed. The stamped version is the fallback for a page whose fetch is
blocked, and the stylesheet query is what stops a new release from being served
with a cached old palette.

  python tools/stamp_site.py --version 0.1.34
  python tools/stamp_site.py --version 0.1.34 --tag v0.1.34 --download https://...
"""

import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INDEX = ROOT / 'docs' / 'index.html'
RELEASE = ROOT / 'docs' / 'release.json'
REPO = 'https://github.com/XiONDiGiTaL/EmblemWeave'


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--version', required=True, help='version without the leading v')
    ap.add_argument('--tag', default='', help='tag; defaults to v<version>')
    ap.add_argument('--download', default='', help='direct asset URL; defaults to the release page')
    args = ap.parse_args()
    version = args.version.lstrip('v')
    tag = args.tag or 'v' + version
    download = args.download or f'{REPO}/releases/tag/{tag}'

    html = INDEX.read_text(encoding='utf-8')
    html, n_css = re.subn(
        r'(<link rel="stylesheet" href="site\.css)(?:\?v=[^"]*)?(" id="site-css">)',
        rf'\g<1>?v={version}\g<2>',
        html,
    )
    html, n_chip = re.subn(r'(<span class="build" id="version">)v[^<]*(</span>)', rf'\g<1>v{version}\g<2>', html)
    html, n_badge = re.subn(r'(<li id="badge-release">)release [^<]*(</li>)', rf'\g<1>release {version}\g<2>', html)
    html, n_download = re.subn(r'(<a class="btn primary" id="download" href=")[^"]*(")', rf'\g<1>{download}\g<2>', html)
    html, n_js = re.subn(r'(<script src="site\.js)(?:\?v=[^"]*)?("></script>)', rf'\g<1>?v={version}\g<2>', html)
    missing = [name for name, n in [('stylesheet', n_css), ('version chip', n_chip), ('release badge', n_badge), ('download', n_download), ('site script', n_js)] if n != 1]
    if missing:
        raise SystemExit('could not stamp ' + ', '.join(missing) + ': docs/index.html no longer has the expected markers')
    INDEX.write_text(html, encoding='utf-8')

    RELEASE.write_text(
        json.dumps({'version': version, 'tag': tag, 'download': download, 'repo': REPO}, indent=2) + '\n',
        encoding='utf-8',
    )
    print(f'stamped docs/index.html and docs/release.json for {tag}')


if __name__ == '__main__':
    main()
