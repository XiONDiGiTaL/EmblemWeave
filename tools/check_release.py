#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check that every checked-in release pointer names the same version.

The release workflow and the local packaging script both read the version from
`Cargo.toml`. The site, the updater metadata and the README are edited by hand,
so a release can otherwise ship a launcher whose own site still links the
previous download. This check fails before anything is built or published.

It reads only checked-in text; it does not reach the network.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPO = "https://github.com/XiONDiGiTaL/EmblemWeave"


def read(rel: str) -> str:
    return (ROOT / rel).read_text(encoding="utf-8")


def cargo_version() -> str:
    text = read("Cargo.toml")
    section = re.search(r"(?ms)^\[workspace\.package\]\s*$.*?(?=^\[|\Z)", text)
    if not section:
        raise SystemExit("Cargo.toml has no [workspace.package] section")
    version = re.search(r"(?m)^version\s*=\s*\"([^\"]+)\"", section.group(0))
    if not version:
        raise SystemExit("Cargo.toml's [workspace.package] has no version")
    return version.group(1)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", help="the version the caller is packaging; defaults to Cargo.toml's")
    args = parser.parse_args(argv)
    cargo = cargo_version()
    version = args.version or cargo
    problems: list[str] = []
    if version != cargo:
        problems.append(f"the caller is packaging {version!r}, Cargo.toml says {cargo!r}")
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        print(f"release version {version!r} is not major.minor.patch")
        return 1

    tag = f"v{version}"
    download = f"{REPO}/releases/download/{tag}/EmblemWeave-{tag}-windows-x64.zip"

    def need(condition: bool, message: str) -> None:
        if not condition:
            problems.append(message)

    release = json.loads(read("docs/release.json"))
    need(release.get("version") == version, f"docs/release.json version is {release.get('version')!r}, expected {version!r}")
    need(release.get("tag") == tag, f"docs/release.json tag is {release.get('tag')!r}, expected {tag!r}")
    need(release.get("download") == download, "docs/release.json download does not name the current asset")
    need(release.get("repo") == REPO, "docs/release.json repo is not the canonical repository")

    index = read("docs/index.html")
    need(f'id="version">v{version}<' in index, "docs/index.html version badge is not current")
    need(f'href="{download}"' in index, "docs/index.html download button is not the current asset")
    need(f'id="badge-release">release {version}<' in index, "docs/index.html release badge is not current")
    need(f'site.css?v={version}' in index, "docs/index.html stylesheet cache key is not current")
    need(f'site.js?v={version}' in index, "docs/index.html script cache key is not current")

    readme = read("README.md")
    need(f"Current release: {version}." in readme, "README.md current-release line is not current")
    need(f"docs/changelog/{version}.md" in readme, "README.md does not link the current changelog")

    changelog = ROOT / "docs" / "changelog" / f"{version}.md"
    need(changelog.is_file(), f"docs/changelog/{version}.md is missing")
    if changelog.is_file():
        need(changelog.read_text(encoding="utf-8").startswith(f"# EmblemWeave {version}"),
             f"docs/changelog/{version}.md does not start with '# EmblemWeave {version}'")

    changelog_index = read("docs/changelog/README.md")
    need(f"| [{version}]({version}.md) |" in changelog_index, "docs/changelog/README.md has no row for the current version")

    if problems:
        print(f"release metadata does not agree on {version}:")
        for problem in problems:
            print(f"  - {problem}")
        return 1
    print(f"release metadata agrees on {version} ({tag})")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))