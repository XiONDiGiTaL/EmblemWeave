#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Create and verify a per-file SHA-256 manifest for a release archive.

The release zip already carries a zip-level SHA-256 in the release notes. That
answers "is this the same byte-for-byte download?" but not "which files were in
it, and did they arrive intact?". This tool writes a sidecar JSON manifest from
the archive itself, including the source commit, build target and a SHA-256 for
every file. The manifest is small enough to publish beside the zip and can be
checked later without the build tree.

    python tools/release_manifest.py create \
        --archive target/pkg/EmblemWeave-v0.1.40-windows-x64.zip \
        --version 0.1.40 --output target/pkg/release-manifest.json
    python tools/release_manifest.py verify \
        --archive target/pkg/EmblemWeave-v0.1.40-windows-x64.zip \
        --manifest target/pkg/release-manifest.json

The archive is first passed through tools/verify_archive.py, so a manifest is
never created for a zip with an unsafe, duplicate or missing path.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import zipfile
from pathlib import Path
from typing import Any, Iterable

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

import verify_archive  # noqa: E402

SCHEMA = 1
KIND = "emblemweave-release-manifest"
PRODUCT = "EmblemWeave"
VERSION_RE = re.compile(r"^\d+\.\d+\.\d+$")
CHUNK = 1 << 20


class ManifestError(Exception):
    """The archive or manifest is not usable as a release artifact."""


def sha256_file(path:Path | str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(CHUNK):
            h.update(chunk)
    return h.hexdigest()


def _git(*args: str) -> str | None:
    try:
        p = subprocess.run(
            ["git", *args], cwd=ROOT, capture_output=True, text=True, timeout=15
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if p.returncode != 0:
        return None
    return p.stdout.strip()


def source_info(commit: str | None = None, dirty: bool | None = None) -> dict[str, Any]:
    """Provenance with explicit values, or the current checkout when omitted."""
    if commit is None:
        commit = _git("rev-parse", "HEAD") or "unknown"
    if dirty is None:
        status = _git("status", "--porcelain", "--untracked-files=no")
        dirty = None if status is None else bool(status)
    branch = _git("rev-parse", "--abbrev-ref", "HEAD")
    return {"commit": commit, "dirty": dirty, "branch": branch or "unknown"}


def zip_file_hashes(archive: Path) -> tuple[list[dict[str, Any]], int]:
    """Every regular file in the zip, sorted by path, with its decompressed hash."""
    files: list[dict[str, Any]] = []
    directories = 0
    with zipfile.ZipFile(archive) as z:
        for info in z.infolist():
            name = info.filename
            if name.endswith("/"):
                directories += 1
                continue
            h = hashlib.sha256()
            size = 0
            with z.open(info) as src:
                while chunk := src.read(CHUNK):
                    h.update(chunk)
                    size += len(chunk)
            files.append({"path": name, "size": size, "sha256": h.hexdigest()})
    files.sort(key=lambda item: item["path"])
    return files, directories


def _prefix_problems(files: Iterable[dict[str, Any]], prefix: str) -> list[str]:
    if not prefix:
        return []
    return [
        f"{item['path']!r} is outside the release prefix {prefix!r}"
        for item in files
        if not item["path"].startswith(prefix)
    ]


def create_manifest(
    archive: Path | str,
    version: str,
    *,
    prefix: str | None = None,
    source_commit: str | None = None,
    source_dirty: bool | None = None,
    build_configuration: str = "Release",
    build_target: str = "windows-x64",
    launcher_commit: str | None = None,
    launcher_dirty: bool | None = None,
    require: Iterable[str] = (),
    created_utc: str | None = None,
) -> dict[str, Any]:
    """Build a manifest for a validated release zip."""
    archive = Path(archive)
    if not archive.is_file():
        raise ManifestError(f"no archive at {archive}")
    if not VERSION_RE.fullmatch(version):
        raise ManifestError(f"version {version!r} is not major.minor.patch")
    if prefix is None:
        prefix = f"{PRODUCT}-v{version}/"
    if prefix and not prefix.endswith("/"):
        prefix += "/"
    required = list(require)
    try:
        problems, n_files, n_dirs = verify_archive.check_zip(str(archive), "", required)
    except (OSError, zipfile.BadZipFile) as e:
        raise ManifestError(f"{archive}: cannot read the zip: {e}") from e
    if problems:
        raise ManifestError(
            f"{archive} failed archive validation; refusing to describe it:\n  "
            + "\n  ".join(problems[:20])
            + (f"\n  ... and {len(problems) - 20} more" if len(problems) > 20 else "")
        )
    files, directories = zip_file_hashes(archive)
    prefixed = _prefix_problems(files, prefix)
    if prefixed:
        raise ManifestError(
            f"{archive} does not use one release prefix; refusing to describe it:\n  "
            + "\n  ".join(prefixed[:20])
        )
    if len(files) != n_files:
        raise ManifestError(f"{archive}: read {len(files)} files, verify_archive counted {n_files}")
    manifest: dict[str, Any] = {
        "schema": SCHEMA,
        "kind": KIND,
        "product": PRODUCT,
        "version": version,
        "archive": {
            "name": archive.name,
            "sha256": sha256_file(archive),
            "size": archive.stat().st_size,
            "files": len(files),
            "directories": max(directories, n_dirs),
            "prefix": prefix,
        },
        "source": source_info(source_commit, source_dirty),
        "build": {
            "configuration": build_configuration,
            "target": build_target,
            "packager": "tools/package_release.ps1",
            "launcher": {"commit": launcher_commit, "dirty": launcher_dirty},
        },
        "files": files,
    }
    if created_utc:
        manifest["created_utc"] = created_utc
    return manifest


def load_manifest(path: Path | str) -> dict[str, Any]:
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise ManifestError(f"cannot read {path}: {e}") from e
    if not isinstance(data, dict):
        raise ManifestError(f"{path}: the manifest is not a JSON object")
    return data


def verify_manifest(
    archive: Path | str,
    manifest: dict[str, Any],
    *,
    require: Iterable[str] = (),
) -> list[str]:
    """Recompute the archive and compare every file with the manifest."""
    archive = Path(archive)
    problems: list[str] = []
    if manifest.get("kind") != KIND or manifest.get("schema") != SCHEMA:
        problems.append(f"{archive}: not a schema-{SCHEMA} {KIND} manifest")
    if manifest.get("product") != PRODUCT:
        problems.append(f"{archive}: product is {manifest.get('product')!r}, expected {PRODUCT!r}")
    version = manifest.get("version")
    if not isinstance(version, str) or not VERSION_RE.fullmatch(version):
        problems.append(f"{archive}: manifest version is {version!r}")
        version = ""
    if not archive.is_file():
        return problems + [f"{archive}: no such file"]
    archive_info = manifest.get("archive")
    if not isinstance(archive_info, dict):
        problems.append(f"{archive}: manifest has no archive object")
        archive_info = {}
    if archive_info.get("name") not in (None, archive.name):
        problems.append(f"{archive}: manifest names {archive_info.get('name')!r}, found {archive.name!r}")
    try:
        digest = sha256_file(archive)
        size = archive.stat().st_size
    except OSError as e:
        return problems + [f"{archive}: cannot read: {e}"]
    if archive_info.get("sha256") != digest:
        problems.append(f"{archive}: sha256 is {digest}, the manifest says {archive_info.get('sha256')!r}")
    if archive_info.get("size") != size:
        problems.append(f"{archive}: size is {size}, the manifest says {archive_info.get('size')!r}")
    prefix = archive_info.get("prefix") or (f"{PRODUCT}-v{version}/" if version else "")
    try:
        archive_problems, n_files, _ = verify_archive.check_zip(str(archive), "", list(require))
    except (OSError, zipfile.BadZipFile) as e:
        return problems + [f"{archive}: cannot read the zip: {e}"]
    problems.extend(archive_problems)
    try:
        actual_files, _ = zip_file_hashes(archive)
    except (OSError, zipfile.BadZipFile, RuntimeError) as e:
        return problems + [f"{archive}: cannot hash its files: {e}"]
    problems.extend(_prefix_problems(actual_files, prefix))
    actual = {item["path"]: item for item in actual_files}
    expected_items = manifest.get("files")
    if not isinstance(expected_items, list):
        return problems + [f"{archive}: manifest has no files list"]
    expected: dict[str, dict[str, Any]] = {}
    for item in expected_items:
        if not isinstance(item, dict) or not isinstance(item.get("path"), str):
            problems.append(f"{archive}: malformed file entry {item!r}")
            continue
        expected[item["path"]] = item
    if archive_info.get("files") not in (None, len(expected)):
        problems.append(f"{archive}: manifest archive.files is {archive_info.get('files')!r}, it lists {len(expected)}")
    for path in sorted(expected.keys() - actual.keys()):
        problems.append(f"{archive}: missing from the archive: {path!r}")
    for path in sorted(actual.keys() - expected.keys()):
        problems.append(f"{archive}: not in the manifest: {path!r}")
    for path in sorted(expected.keys() & actual.keys()):
        want, got = expected[path], actual[path]
        if want.get("size") != got["size"]:
            problems.append(f"{archive}: {path!r} is {got['size']} bytes, the manifest says {want.get('size')!r}")
        if want.get("sha256") != got["sha256"]:
            problems.append(f"{archive}: {path!r} has sha256 {got['sha256']}, the manifest says {want.get('sha256')!r}")
    if n_files != len(actual):
        problems.append(f"{archive}: verify_archive counted {n_files} files, hashing found {len(actual)}")
    return problems


def _write_json(path: str, data: dict[str, Any]) -> None:
    text = json.dumps(data, indent=2, ensure_ascii=False) + "\n"
    if path == "-":
        sys.stdout.write(text)
        return
    out = Path(path)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8", newline="\n") as f:
        f.write(text)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="command", required=True)

    create = sub.add_parser("create", help="describe a release archive")
    create.add_argument("--archive", required=True)
    create.add_argument("--version", required=True)
    create.add_argument("--output", required=True, help="manifest path, or - for stdout")
    create.add_argument("--prefix", default=None, help="top-level archive folder (default: EmblemWeave-v<version>/)")
    create.add_argument("--source-commit", default=None)
    source = create.add_mutually_exclusive_group()
    source.add_argument("--source-dirty", dest="source_dirty", action="store_true", default=None)
    source.add_argument("--source-clean", dest="source_dirty", action="store_false")
    create.add_argument("--build-configuration", default="Release")
    create.add_argument("--build-target", default="windows-x64")
    create.add_argument("--launcher-commit", default=None)
    launcher = create.add_mutually_exclusive_group()
    launcher.add_argument("--launcher-dirty", dest="launcher_dirty", action="store_true", default=None)
    launcher.add_argument("--launcher-clean", dest="launcher_dirty", action="store_false")
    create.add_argument("--require", action="append", default=[])
    create.add_argument("--created-utc", default=None)

    verify = sub.add_parser("verify", help="check an archive against a manifest")
    verify.add_argument("--archive", required=True)
    verify.add_argument("--manifest", required=True)
    verify.add_argument("--require", action="append", default=[])

    args = ap.parse_args(argv)
    try:
        if args.command == "create":
            manifest = create_manifest(
                args.archive,
                args.version,
                prefix=args.prefix,
                source_commit=args.source_commit,
                source_dirty=args.source_dirty,
                build_configuration=args.build_configuration,
                build_target=args.build_target,
                launcher_commit=args.launcher_commit,
                launcher_dirty=args.launcher_dirty,
                require=args.require,
                created_utc=args.created_utc,
            )
            _write_json(args.output, manifest)
            print(
                f"manifest: {manifest['archive']['files']} files, "
                f"{manifest['archive']['size']} bytes, sha256 {manifest['archive']['sha256']}",
                file=sys.stderr,
            )
            return 0
        manifest = load_manifest(args.manifest)
        problems = verify_manifest(args.archive, manifest, require=args.require)
        if problems:
            print(f"{args.archive}: {len(problems)} manifest problem(s)")
            for problem in problems[:40]:
                print("  " + problem)
            if len(problems) > 40:
                print(f"  ... and {len(problems) - 40} more")
            return 1
        print(f"{args.archive}: manifest ok ({manifest['archive']['files']} files)")
        return 0
    except ManifestError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())