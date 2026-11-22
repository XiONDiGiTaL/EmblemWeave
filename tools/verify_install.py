#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check an unpacked EmblemWeave install against a release manifest.

The manifest answers what a release contains and what each file hashes to. This
tool answers whether the folder on disk is that release: every manifest file is
present with the right bytes, and unfamiliar files are reported. User data and
generated build output are expected to be extra and are ignored by default:

    data/                 settings, saves, mods, logs, decoded modules
    backups/              launcher and save backups
    upscalers/            user-supplied DLSS/FSR/XeSS files
    engine/.../game/      the user's dump or a hand-picked engine folder
    engine/.../work/      lifted and decoded output
    engine/.../*/build/   compiled engine and fetched dependencies
    engine/.../tools/download, textures/third_party, shaders/third_party

Unknown extras are warnings; --strict turns them into failures. Missing or
changed manifest files always fail. Nothing is written or deleted.

    python tools/verify_install.py --root D:\\EmblemWeave \
        --manifest D:\\EmblemWeave\\release-manifest.json
"""

from __future__ import annotations

import argparse
import hashlib
import os
import sys
from pathlib import Path, PurePosixPath

HERE = Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

import release_manifest  # noqa: E402

CHUNK = 1 << 20

# Extras in these trees are the player's data, not a release mismatch. Keep this
# list narrow: a stale file under docs/ or engine/fefw-recomp/recomp/ is exactly
# what a mixed install leaves behind and should be reported.
USER_TREES = (
    "data/",
    "backups/",
    "logs/",
    "upscalers/",
    "engine/fefw-recomp/game/",
    "engine/fefw-recomp/work/",
    "engine/fefw-recomp/build/",
    "engine/fefw-recomp/runtime/build/",
    "engine/fefw-recomp/game-build/fefw/build/",
    "engine/fefw-recomp/tools/download/",
    "engine/fefw-recomp/tools/textures/third_party/",
    "engine/fefw-recomp/tools/shaders/third_party/",
    "engine/fefw-recomp/tools/shaders/prebuilt/linux-x64/",
)
USER_FILES = {
    "engine/fefw-recomp/engine.txt",
    "engine.txt",
    # A player may keep the published verification sidecars beside the launcher; they are not release files.
    "release-manifest.json",
    "SHA256SUMS",
}


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(CHUNK):
            h.update(chunk)
    return h.hexdigest()


def is_user_extra(rel: str) -> bool:
    posix = PurePosixPath(rel)
    text = posix.as_posix()
    if text in USER_FILES:
        return True
    if any(text.startswith(prefix) for prefix in USER_TREES):
        return True
    name = posix.name.lower()
    return name.endswith((".log", ".log.prev", ".old", ".tmp", ".bak")) or name.startswith("emblemweave.exe.pre-install-")


def verify_install(root: Path, manifest: dict[str, object], *, strict: bool = False) -> tuple[list[str], list[str]]:
    """Return (failures, warnings) for a release manifest against a folder."""
    failures: list[str] = []
    warnings: list[str] = []
    if not root.is_dir():
        return [f"{root}: no such folder"], warnings
    expected_items = manifest.get("files")
    archive = manifest.get("archive")
    if not isinstance(expected_items, list) or not isinstance(archive, dict):
        return [f"{root}: the manifest has no files or archive object"], warnings
    version = manifest.get("version")
    prefix = archive.get("prefix") if isinstance(archive, dict) else None
    if not isinstance(prefix, str) or not prefix:
        prefix = f"EmblemWeave-v{version}/" if isinstance(version, str) else ""

    expected: dict[str, dict[str, object]] = {}
    for item in expected_items:
        if not isinstance(item, dict) or not isinstance(item.get("path"), str):
            failures.append(f"{root}: malformed manifest file entry {item!r}")
            continue
        path = item["path"]
        if not path.startswith(prefix):
            failures.append(f"{root}: {path!r} is outside the release prefix {prefix!r}")
            continue
        rel = path[len(prefix):]
        if not rel or rel.startswith("/") or ".." in PurePosixPath(rel).parts:
            failures.append(f"{root}: unsafe manifest path {path!r}")
            continue
        expected[rel] = item

    if "EmblemWeave.exe" not in expected:
        failures.append(f"{root}: the manifest does not list EmblemWeave.exe")
    root_resolved = root.resolve()
    for rel, item in sorted(expected.items()):
        target = (root / PurePosixPath(rel)).resolve()
        try:
            target.relative_to(root_resolved)
        except ValueError:
            failures.append(f"{root}: {rel!r} resolves outside the install")
            continue
        if not target.is_file():
            failures.append(f"{root}: missing {rel}")
            continue
        size = target.stat().st_size
        if item.get("size") != size:
            failures.append(f"{root}: {rel} is {size} bytes, the manifest says {item.get('size')!r}")
        want = item.get("sha256")
        got = sha256_file(target)
        if want != got:
            failures.append(f"{root}: {rel} has sha256 {got}, the manifest says {want!r}")

    extras: list[str] = []
    for dirpath, _dirnames, filenames in os.walk(root):
        for name in filenames:
            full = Path(dirpath) / name
            rel = full.relative_to(root).as_posix()
            if rel in expected or is_user_extra(rel):
                continue
            extras.append(rel)
    extras.sort()
    for rel in extras:
        message = f"{root}: extra file not in the release manifest: {rel}"
        (failures if strict else warnings).append(message)
    return failures, warnings


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", required=True, help="the unpacked install folder")
    ap.add_argument("--manifest", required=True, help="release-manifest.json from the release")
    ap.add_argument("--strict", action="store_true", help="treat unknown extra files as failures")
    args = ap.parse_args(argv)
    try:
        manifest = release_manifest.load_manifest(args.manifest)
    except release_manifest.ManifestError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    failures, warnings = verify_install(Path(args.root), manifest, strict=args.strict)
    for warning in warnings:
        print("warning: " + warning, file=sys.stderr)
    if failures:
        print(f"install verification FAILED ({len(failures)} problem(s)):", file=sys.stderr)
        for failure in failures:
            print("  " + failure, file=sys.stderr)
        return 1
    files = len(manifest.get("files", [])) if isinstance(manifest.get("files"), list) else 0
    print(f"install ok: {files} release files match {manifest.get('version')} ({len(warnings)} extra warning(s))")
    return 0


if __name__ == "__main__":
    sys.exit(main())