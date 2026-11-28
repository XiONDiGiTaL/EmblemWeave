#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Finalize and verify the combined SHA256SUMS beside a release archive.

The zip, its per-file release manifest and the package manifest are produced by
different steps. The checksum file must describe the files that actually exist
after all of them have finished, not the previous build's files. This tool is
the single writer and checker for that file:

    python tools/release_sums.py finalize \
        --sums target/pkg/SHA256SUMS \
        --artifact target/pkg/EmblemWeave-v0.1.43-windows-x64.zip \
        --artifact target/pkg/release-manifest.json \
        --package-manifest target/pkg/manifest.json

`finalize` replaces any existing file atomically. It refuses duplicate or
unsafe names, a missing artifact, an output path that names one of the
artifacts, and any disagreement between package-manifest.json and the actual
zip and release manifest. It then re-reads the result and verifies it again, so
a write failure cannot leave an unverified file. The checksum file never lists
itself.

    python tools/release_sums.py verify --sums target/pkg/SHA256SUMS \
        --artifact target/pkg/EmblemWeave-v0.1.43-windows-x64.zip \
        --artifact target/pkg/release-manifest.json

`verify` requires exactly the named artifacts: a missing line, an extra line,
a duplicate, a malformed line, a self-entry, or a changed byte all fail.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
from pathlib import Path
from typing import Iterable

HEX64_RE = re.compile(r"^[0-9a-fA-F]{64}$")
LINE_RE = re.compile(r"^([0-9a-fA-F]{64}) [ *]([^/\\]+)$")
CHUNK = 1 << 20


class SumsError(Exception):
    """The release checksum file cannot be written or trusted."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(CHUNK):
            digest.update(chunk)
    return digest.hexdigest()


def _artifact_entry(path: Path) -> tuple[str, str]:
    if not path.is_file():
        raise SumsError(f"release artifact is missing: {path}")
    name = path.name
    if not name or name in {".", ".."} or "/" in name or "\\" in name:
        raise SumsError(f"release artifact has no usable file name: {path}")
    return name, sha256_file(path)


def _package_manifest(path: Path) -> dict[str, object]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        raise SumsError(f"cannot read package manifest {path}: {exc}") from exc
    if not isinstance(data, dict):
        raise SumsError(f"package manifest {path} is not a JSON object")
    required = (
        "version",
        "zip",
        "zipSha",
        "zipSize",
        "releaseManifest",
        "releaseManifestSha256",
    )
    missing = [key for key in required if key not in data]
    if missing:
        raise SumsError(f"package manifest {path} has no {', '.join(missing)}")
    return data


def _check_package_manifest(
    manifest_path: Path,
    artifacts: dict[str, Path],
) -> None:
    data = _package_manifest(manifest_path)
    expected_zip = data["zip"]
    expected_manifest = data["releaseManifest"]
    if not isinstance(expected_zip, str) or not isinstance(expected_manifest, str):
        raise SumsError(f"package manifest {manifest_path} names a non-text artifact")
    by_name = {path.name: path for path in artifacts.values()}
    if expected_zip not in by_name:
        raise SumsError(
            f"package manifest names zip {expected_zip!r}, which is not one of the artifacts"
        )
    if expected_manifest not in by_name:
        raise SumsError(
            "package manifest names release manifest "
            f"{expected_manifest!r}, which is not one of the artifacts"
        )
    zip_path = by_name[expected_zip]
    release_path = by_name[expected_manifest]
    for path in (zip_path, release_path):
        if not path.is_file():
            raise SumsError(f"release artifact is missing: {path}")
    zip_hash = sha256_file(zip_path)
    release_hash = sha256_file(release_path)
    zip_size = zip_path.stat().st_size
    if data["zipSha"] != zip_hash:
        raise SumsError(
            f"package manifest says zip sha256 {data['zipSha']!r}, actual {zip_hash}"
        )
    if data["zipSize"] != zip_size:
        raise SumsError(
            f"package manifest says zip size {data['zipSize']!r}, actual {zip_size}"
        )
    if data["releaseManifestSha256"] != release_hash:
        raise SumsError(
            "package manifest says release manifest sha256 "
            f"{data['releaseManifestSha256']!r}, actual {release_hash}"
        )


def _lines(artifacts: list[tuple[str, str]]) -> str:
    # Lowercase, one space, an asterisk: the portable sha256sum binary format.
    # LF endings keep the file identical when packaged on another host.
    return "".join(f"{digest} *{name}\n" for name, digest in artifacts)


def _read_sums(path: Path) -> list[tuple[str, str]]:
    try:
        raw = path.read_bytes()
    except OSError as exc:
        raise SumsError(f"cannot read checksum file {path}: {exc}") from exc
    try:
        text = raw.decode("ascii")
    except UnicodeDecodeError as exc:
        raise SumsError(f"checksum file {path} is not ASCII") from exc
    if text.startswith("\ufeff"):
        raise SumsError(f"checksum file {path} starts with a byte-order mark")
    if not text:
        raise SumsError(f"checksum file {path} is empty")
    if not text.endswith("\n"):
        raise SumsError(f"checksum file {path} has no final newline")
    entries: list[tuple[str, str]] = []
    for number, line in enumerate(text.splitlines(), 1):
        if not line:
            raise SumsError(f"checksum file {path} line {number} is blank")
        match = LINE_RE.fullmatch(line)
        if not match:
            raise SumsError(
                f"checksum file {path} line {number} is not '<sha256> *<file>': {line!r}"
            )
        digest, name = match.groups()
        entries.append((name, digest.lower()))
    return entries


def verify_sums(sums_path: Path | str, artifact_paths: Iterable[Path | str]) -> list[str]:
    """Return every way the checksum file disagrees with the named artifacts."""
    sums_path = Path(sums_path)
    paths = [Path(path) for path in artifact_paths]
    problems: list[str] = []
    if not paths:
        return ["no artifacts were named; refusing to verify an empty set"]
    if sums_path.name.lower() == "sha256sums" and any(
        path.resolve() == sums_path.resolve() for path in paths
    ):
        problems.append(f"{sums_path}: the checksum file cannot list itself")
    by_name: dict[str, Path] = {}
    for path in paths:
        if path.name in by_name:
            problems.append(f"two artifacts have the same file name: {path.name!r}")
            continue
        by_name[path.name] = path
    try:
        entries = _read_sums(sums_path)
    except SumsError as exc:
        return problems + [str(exc)]
    seen: dict[str, int] = {}
    for name, digest in entries:
        key = name.casefold()
        seen[key] = seen.get(key, 0) + 1
        if key == sums_path.name.casefold():
            problems.append(f"{sums_path}: the checksum file must not list itself")
        if name not in by_name:
            problems.append(f"{sums_path}: unexpected artifact entry {name!r}")
            continue
        path = by_name[name]
        if not path.is_file():
            problems.append(f"{path}: named in the checksum file but missing")
            continue
        actual = sha256_file(path)
        if actual != digest:
            problems.append(
                f"{path}: sha256 is {actual}, the checksum file says {digest}"
            )
    for key, count in seen.items():
        if count > 1:
            problems.append(f"{sums_path}: duplicate artifact entry {key!r}")
    listed = {name for name, _ in entries}
    for name in by_name:
        if name not in listed:
            problems.append(f"{sums_path}: artifact {name!r} has no checksum entry")
    return problems


def finalize(
    sums_path: Path | str,
    artifact_paths: Iterable[Path | str],
    *,
    package_manifest: Path | str | None = None,
) -> None:
    """Write an exact checksum file atomically, then verify it.

    Every artifact must exist before the old checksum file is touched. The
    package manifest, when given, is checked against the actual zip and release
    manifest first. This is deliberately not a merge: entries from a previous
    build cannot survive, because no existing content is read.
    """
    sums_path = Path(sums_path)
    paths = [Path(path) for path in artifact_paths]
    if not paths:
        raise SumsError("no artifacts were named")
    if len(paths) != len({path.name.casefold() for path in paths}):
        raise SumsError("two artifacts have the same file name")
    resolved_output = sums_path.resolve()
    if any(path.resolve() == resolved_output for path in paths):
        raise SumsError(f"{sums_path} is one of the artifacts; it cannot list itself")
    if package_manifest is not None:
        _check_package_manifest(Path(package_manifest), {path.name: path for path in paths})
    entries = [_artifact_entry(path) for path in paths]
    text = _lines(entries)
    sums_path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(
        prefix=sums_path.name + ".", suffix=".tmp", dir=sums_path.parent
    )
    try:
        with os.fdopen(descriptor, "w", encoding="ascii", newline="\n") as stream:
            stream.write(text)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, sums_path)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise
    problems = verify_sums(sums_path, paths)
    if problems:
        raise SumsError(
            f"the new checksum file failed verification:\n  " + "\n  ".join(problems)
        )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("finalize", "verify"):
        command = sub.add_parser(name)
        command.add_argument("--sums", required=True)
        command.add_argument(
            "--artifact",
            action="append",
            required=True,
            help="a published file; repeat for every artifact",
        )
        if name == "finalize":
            command.add_argument(
                "--package-manifest",
                default=None,
                help="manifest.json whose zip and release-manifest hashes must agree",
            )
    args = parser.parse_args(argv)
    sums = Path(args.sums)
    artifacts = [Path(path) for path in args.artifact]
    try:
        if args.command == "finalize":
            finalize(sums, artifacts, package_manifest=args.package_manifest)
            print(f"{sums}: wrote and verified {len(artifacts)} checksum entries")
            return 0
        problems = verify_sums(sums, artifacts)
    except SumsError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    if problems:
        print(f"{sums}: {len(problems)} checksum problem(s)", file=sys.stderr)
        for problem in problems:
            print("  " + problem, file=sys.stderr)
        return 1
    print(f"{sums}: checksums match {len(artifacts)} artifacts")
    return 0


if __name__ == "__main__":
    sys.exit(main())
