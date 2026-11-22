#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Copy/check the engine tree from Git's tracked file list.

A release must not acquire a local object file, a fetched third-party tree or
an experimental source file that happens to be in the working directory. The
packaging script uses ``list`` to enumerate exactly the tracked engine files
(apart from fetched/generated trees and the GitHub templates) and ``verify`` to
prove the copied tree has no extras other than the generated Windows shader
translator and its redistribution notices.

    python tools/verify_engine_tree.py list --source .
    python tools/verify_engine_tree.py verify --source . --root target/pkg/.../engine/fefw-recomp

``list`` writes a JSON array of paths relative to ``engine/fefw-recomp/`` so a
PowerShell caller can copy them without relying on native-command NUL handling.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path, PurePosixPath

ENGINE_PREFIX = "engine/fefw-recomp/"

# These are not player files: fetched toolchains, the fetched texture/shader
# trees, the dump, generated work, the Linux translator and GitHub templates.
# Everything else that is tracked in the engine is part of the source release.
EXCLUDED_PREFIXES = (
    ENGINE_PREFIX + ".github/",
    ENGINE_PREFIX + "game/",
    ENGINE_PREFIX + "work/",
    ENGINE_PREFIX + "tools/download/",
    ENGINE_PREFIX + "tools/textures/third_party/",
    ENGINE_PREFIX + "tools/shaders/third_party/",
    ENGINE_PREFIX + "tools/shaders/prebuilt/linux-x64/",
)

DEFAULT_ALLOWED = (
    "tools/shaders/prebuilt/win-x64/spirv_harness.exe",
    "tools/shaders/prebuilt/win-x64/LICENSE.Ryujinx.MIT.txt",
    "tools/shaders/prebuilt/win-x64/DOTNET-LICENSE.txt",
    "tools/shaders/prebuilt/win-x64/DOTNET-THIRD-PARTY-NOTICES.txt",
)


class EngineTreeError(Exception):
    """The tracked list or the packaged tree cannot be checked."""


def normalize(path: str) -> str:
    path = str(path).replace("\\", "/")
    if path.startswith("./"):
        path = path[2:]
    return path


def _excluded(path: str) -> bool:
    return any(path == prefix.rstrip("/") or path.startswith(prefix) for prefix in EXCLUDED_PREFIXES)


def tracked_paths(source: Path | str) -> list[str]:
    """Tracked engine files, sorted and relative to the repository root."""
    source = Path(source)
    proc = subprocess.run(
        ["git", "-C", str(source), "ls-files", "-z", "--", ENGINE_PREFIX.rstrip("/")],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if proc.returncode != 0:
        detail = proc.stderr.decode("utf-8", "replace").strip()
        raise EngineTreeError(f"git ls-files failed in {source}: {detail or 'unknown git error'}")
    paths: list[str] = []
    for raw in proc.stdout.split(b"\0"):
        if not raw:
            continue
        try:
            path = normalize(raw.decode("utf-8"))
        except UnicodeDecodeError as e:
            raise EngineTreeError(f"git returned a path that is not UTF-8: {e}") from e
        if not path.startswith(ENGINE_PREFIX):
            raise EngineTreeError(f"git returned a path outside {ENGINE_PREFIX!r}: {path!r}")
        if _excluded(path):
            continue
        paths.append(path)
    if not paths:
        raise EngineTreeError(f"git tracks no engine files under {ENGINE_PREFIX!r} in {source}")
    return sorted(set(paths))


def relative_paths(paths: list[str] | tuple[str, ...]) -> list[str]:
    """Strip the engine prefix and validate the resulting relative paths."""
    result: set[str] = set()
    for value in paths:
        path = normalize(value)
        if not path.startswith(ENGINE_PREFIX):
            raise EngineTreeError(f"path is outside {ENGINE_PREFIX!r}: {path!r}")
        rel = path[len(ENGINE_PREFIX) :]
        pure = PurePosixPath(rel)
        if not rel or rel.startswith("/") or ".." in pure.parts:
            raise EngineTreeError(f"unsafe engine-relative path: {value!r}")
        result.add(rel)
    return sorted(result)


def package_paths(paths: list[str] | tuple[str, ...]) -> list[str]:
    """Repository-relative engine paths with fetched/generated trees removed."""
    normalized = [normalize(path) for path in paths]
    return relative_paths([path for path in normalized if not _excluded(path)])


def relative_path(value: str) -> str:
    """Accept a path relative to the engine root, or a repository-relative one."""
    path = normalize(value)
    if path.startswith(ENGINE_PREFIX):
        path = path[len(ENGINE_PREFIX) :]
    pure = PurePosixPath(path)
    if not path or path.startswith("/") or ".." in pure.parts:
        raise EngineTreeError(f"unsafe engine-relative path: {value!r}")
    return path


def verify_tree(
    root: Path | str,
    expected: list[str] | tuple[str, ...] | set[str],
    allowed: list[str] | tuple[str, ...] | set[str] = (),
) -> list[str]:
    """Return problems for a copied engine tree, or an empty list when it matches."""
    root = Path(root)
    if not root.is_dir():
        return [f"no packaged engine tree at {root}"]
    expected_set = {normalize(p) for p in expected}
    allowed_set = {normalize(p) for p in allowed}
    actual = {
        p.relative_to(root).as_posix()
        for p in root.rglob("*")
        if p.is_file()
    }
    problems: list[str] = []
    for path in sorted(expected_set - actual):
        problems.append(f"missing tracked engine file: {path!r}")
    for path in sorted(actual - expected_set - allowed_set):
        problems.append(f"untracked engine file in package: {path!r}")
    return problems


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="command", required=True)

    list_cmd = sub.add_parser("list", help="print tracked engine-relative paths as a JSON array")
    list_cmd.add_argument("--source", default=".", help="repository root (default: .)")

    verify_cmd = sub.add_parser("verify", help="compare a copied engine tree with the tracked list")
    verify_cmd.add_argument("--source", default=".", help="repository root (default: .)")
    verify_cmd.add_argument("--root", required=True, help="copied engine/fefw-recomp directory")
    verify_cmd.add_argument(
        "--allow",
        action="append",
        default=list(DEFAULT_ALLOWED),
        help="generated engine-relative file allowed in the package (repeatable)",
    )

    args = ap.parse_args(argv)
    try:
        if args.command == "list":
            sys.stdout.write(json.dumps(package_paths(tracked_paths(args.source)), ensure_ascii=True) + "\n")
            return 0
        expected = package_paths(tracked_paths(args.source))
        allowed = sorted({relative_path(value) for value in args.allow})
        problems = verify_tree(args.root, expected, allowed)
        if problems:
            print(f"{args.root}: {len(problems)} engine-tree problem(s)")
            for problem in problems[:40]:
                print("  " + problem)
            if len(problems) > 40:
                print(f"  ... and {len(problems) - 40} more")
            return 1
        print(f"engine tree ok: {len(expected)} tracked files, {len(allowed)} generated allowance(s)")
        return 0
    except EngineTreeError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
