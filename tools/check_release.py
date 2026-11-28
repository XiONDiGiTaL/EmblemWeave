#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check that every checked-in release pointer names the same version.

The release workflow and the local packaging script both read the version from
`Cargo.toml`. The site, the updater metadata and the README are edited by hand,
so a release can otherwise ship a launcher whose own site still links the
previous download. This check fails before anything is built or published.

The development tree may be one version ahead of the published release: when
`Cargo.toml` names a newer version, this check requires that version's draft
changelog and index row, and it still requires the site, `docs/release.json`
and the README's current-release line to name the published version. Passing
`--version` is the packaging path and is strict: every pointer must already
name that version.

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


def version_key(version: str) -> tuple[int, int, int]:
    major, minor, patch = version.split(".")
    return int(major), int(minor), int(patch)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", help="the version the caller is packaging; every pointer must already name it")
    parser.add_argument(
        "--candidate",
        action="store_true",
        help="package an unpublished candidate while the public pointers still name the published release",
    )
    args = parser.parse_args(argv)

    cargo = cargo_version()
    if args.candidate and args.version is None:
        args.version = cargo
    release = json.loads(read("docs/release.json"))
    published = release.get("version")
    if not isinstance(published, str) or not re.fullmatch(r"\d+\.\d+\.\d+", published):
        print(f"docs/release.json version {published!r} is not major.minor.patch")
        return 1

    problems: list[str] = []
    packaging = args.version is not None
    if packaging and not re.fullmatch(r"\d+\.\d+\.\d+", args.version):
        print(f"release version {args.version!r} is not major.minor.patch")
        return 1
    if packaging and args.version != cargo:
        problems.append(f"the caller is packaging {args.version!r}, Cargo.toml says {cargo!r}")
    if packaging and args.version != published and not args.candidate:
        problems.append(
            f"the caller is packaging {args.version!r}, but docs/release.json still names the published "
            f"{published!r}; bump the release pointers before packaging"
        )
    target = args.version or published

    def need(condition: bool, message: str) -> None:
        if not condition:
            problems.append(message)

    tag = f"v{published}"
    download = f"{REPO}/releases/download/{tag}/EmblemWeave-{tag}-windows-x64.zip"
    need(release.get("version") == published, f"docs/release.json version is {release.get('version')!r}")
    need(release.get("tag") == tag, f"docs/release.json tag is {release.get('tag')!r}, expected {tag!r}")
    need(release.get("download") == download, "docs/release.json download does not name the published asset")
    need(release.get("repo") == REPO, "docs/release.json repo is not the canonical repository")

    index = read("docs/index.html")
    need(f'id="version">v{published}<' in index, "docs/index.html version badge is not the published release")
    download_hrefs = [download]
    if args.candidate:
        # An unpublished candidate must leave the site's public download on the
        # always-valid releases page, never point it at a tag that does not exist.
        download_hrefs.append(f"{REPO}/releases/latest")
    need(any(f'href="{href}"' in index for href in download_hrefs), "docs/index.html download button is not the published asset")
    need(f'id="badge-release">release {published}<' in index, "docs/index.html release badge is not the published release")
    need(f"site.css?v={published}" in index, "docs/index.html stylesheet cache key is not the published release")
    need(f"site.js?v={published}" in index, "docs/index.html script cache key is not the published release")

    readme = read("README.md")
    need(f"Current release: {published}." in readme, "README.md current-release line does not name the published release")
    need(f"docs/changelog/{published}.md" in readme, "README.md does not link the published changelog")

    changelog = ROOT / "docs" / "changelog" / f"{published}.md"
    need(changelog.is_file(), f"docs/changelog/{published}.md is missing")
    if changelog.is_file():
        need(changelog.read_text(encoding="utf-8").startswith(f"# EmblemWeave {published}"),
             f"docs/changelog/{published}.md does not start with '# EmblemWeave {published}'")

    changelog_index = read("docs/changelog/README.md")
    need(f"| [{published}]({published}.md) |" in changelog_index,
         "docs/changelog/README.md has no row for the published version")

    if cargo != published:
        need(version_key(cargo) > version_key(published),
             f"Cargo.toml says {cargo!r}, which is not newer than the published {published!r}")
        draft = ROOT / "docs" / "changelog" / f"{cargo}.md"
        need(draft.is_file(), f"docs/changelog/{cargo}.md is missing for the development version")
        if draft.is_file():
            need(draft.read_text(encoding="utf-8").startswith(f"# EmblemWeave {cargo}"),
                 f"docs/changelog/{cargo}.md does not start with '# EmblemWeave {cargo}'")
        need(f"| [{cargo}]({cargo}.md) |" in changelog_index,
             f"docs/changelog/README.md has no row for the development version {cargo}")
        need(f"{cargo} is in development" in readme,
             f"README.md does not say {cargo} is in development")

    if problems:
        print(f"release metadata does not agree on {target}:")
        for problem in problems:
            print(f"  - {problem}")
        return 1
    if cargo == published:
        print(f"release metadata agrees on {published} ({tag})")
    elif args.candidate:
        print(f"candidate {target} is packable; public release metadata still agrees on {published} ({tag})")
    else:
        print(f"published release metadata agrees on {published} ({tag}); {cargo} is unreleased development")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))