#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Prepare disposable Issue #79 save fixtures without touching a live save.

The harness reads a **copy** of a save tree, copies it into a scratch run
directory, writes a dedicated `fefw.ini` per direct-engine row, records
SHA-256 manifests of the source before and after the copy, and prints the
matrix commands. It never launches the game.

Safety rules:

* The source is opened read-only. Its before/after manifests must match.
* `E:\\EmblemWeave`, `%APPDATA%\\EmblemWeave`, the repository's `data`
  folder, drive roots, the home folder and the E: drive are refused.
* Every direct row gets its own save copy and its own ini.
* The scratch directory must be new/empty and outside the source.

Examples:

    python tools/issue79_fixture.py --source D:\\fixtures\\issue79-save-copy ^
        --scratch target\\validation\\issue79\\run1 ^
        --engine E:\\EmblemWeave\\data\\play\\fefw.exe ^
        --romfs D:\\fixtures\\fefw-romfs

    python tools/issue79_fixture.py --verify target\\validation\\issue79\\run1

`--verify` re-hashes the source and reports what the game changed in each
fixture. Source changes are a failure; fixture changes are expected.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

CHUNK = 1 << 20
DEFAULT_USER = "0100000000000000706d635257464546"
FOREIGN_USER = "11110000000000000000000000000001"
DIRECT_ROWS = frozenset({16, 17, 19, 20, 22})
ROW_TITLES = {
    16: "stale absolute save_dir",
    17: "moved portable copy with a stale absolute save_dir",
    19: "foreign account id",
    20: "save nested one level too deep",
    22: "missing account .meta",
    24: "empty profile loaded (launcher-side)",
}
ROW_EXPECT = {
    16: "the ini points at stale-target; when it exists the old save loads and the new root stays unchanged. Delete stale-target to exercise the empty-create branch.",
    17: "same stale-ini check as row 16, with a portable marker beside the fixture; the marker does not change the engine's ini behaviour.",
    19: "the game starts a new game; the log says no such save data and then creates account/DEFAULT_USER. The source must stay unchanged.",
    20: "the game starts a new game; the log does not say no such save data. The source must stay unchanged.",
    22: "the runtime writes the missing .meta and opens the existing tree; the game offers Continue. The source must stay unchanged.",
}
LAUNCHER_NOTE = (
    "Launcher-side row: this harness does not prepare it. Build the row in a scratch portable install from "
    "<scratch>/base-save and use the Saves/Setup pages. A direct engine run does not exercise profile loading."
)


class FixtureError(RuntimeError):
    """A refused or invalid fixture operation."""


def resolve_path(value: str) -> Path:
    return Path(os.path.abspath(os.path.expanduser(value))).resolve(strict=False)


def path_key(path: Path) -> str:
    return os.path.normcase(str(path))


def inside(child: Path, parent: Path) -> bool:
    child_key = path_key(child)
    parent_key = path_key(parent).rstrip("\\/")
    return child_key == parent_key or child_key.startswith(parent_key + os.sep)


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def live_roots() -> list[Path]:
    """Known live installs and save roots. None of these may be a source."""
    roots: list[Path] = []
    if os.name == "nt":
        roots.append(Path(r"E:\EmblemWeave"))
    for name in ("APPDATA", "LOCALAPPDATA"):
        base = os.environ.get(name)
        if base:
            roots.append(Path(base) / "EmblemWeave")
    roots.append(repo_root() / "data")
    return [root.resolve(strict=False) for root in roots]


def is_forbidden_source(path: Path) -> bool:
    """True for a live install/save root or a path the tool must not read."""
    source = resolve_path(str(path))
    if os.name == "nt" and os.path.splitdrive(str(source))[0].upper() == "E:":
        return True
    if source.parent == source:  # drive or filesystem root
        return True
    if source == Path.home().resolve(strict=False):
        return True
    if source == repo_root().resolve(strict=False):
        return True
    return any(inside(source, root) for root in live_roots())


def is_forbidden_scratch(path: Path) -> bool:
    scratch = resolve_path(str(path))
    if os.name == "nt" and os.path.splitdrive(str(scratch))[0].upper() == "E:":
        return True
    if scratch.parent == scratch:
        return True
    if scratch == repo_root().resolve(strict=False):
        return True
    return any(inside(scratch, root) for root in live_roots())


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(CHUNK):
            digest.update(chunk)
    return digest.hexdigest()


def tree_manifest(root: Path) -> dict[str, object]:
    """Hash every regular file and record every directory below root."""
    root = resolve_path(str(root))
    if not root.is_dir():
        raise FixtureError(f"{root}: not a directory")
    dirs: list[str] = []
    files: dict[str, dict[str, object]] = {}
    for dirpath, dirnames, filenames in os.walk(root, topdown=True, followlinks=False):
        base = Path(dirpath)
        dirnames.sort()
        filenames.sort()
        for name in dirnames:
            path = base / name
            if path.is_symlink():
                raise FixtureError(f"{path}: symlinks are not accepted in a fixture source")
            dirs.append(path.relative_to(root).as_posix())
        for name in filenames:
            path = base / name
            if path.is_symlink():
                raise FixtureError(f"{path}: symlinks are not accepted in a fixture source")
            if not path.is_file():
                raise FixtureError(f"{path}: not a regular file")
            rel = path.relative_to(root).as_posix()
            files[rel] = {"size": path.stat().st_size, "sha256": sha256_file(path)}
    return {"root": str(root), "dirs": sorted(dirs), "files": dict(sorted(files.items()))}


def manifest_equal(left: dict[str, object], right: dict[str, object]) -> bool:
    return left.get("dirs") == right.get("dirs") and left.get("files") == right.get("files")


def manifest_diff(before: dict[str, object], after: dict[str, object]) -> dict[str, list[str]]:
    before_files = before.get("files", {})
    after_files = after.get("files", {})
    before_dirs = before.get("dirs", [])
    after_dirs = after.get("dirs", [])
    if not isinstance(before_files, dict) or not isinstance(after_files, dict):
        return {"dirs_added": [], "dirs_removed": [], "added": [], "removed": [], "changed": []}
    if not isinstance(before_dirs, list) or not isinstance(after_dirs, list):
        return {"dirs_added": [], "dirs_removed": [], "added": [], "removed": [], "changed": []}
    before_names = set(before_files)
    after_names = set(after_files)
    return {
        "dirs_added": sorted(set(after_dirs) - set(before_dirs)),
        "dirs_removed": sorted(set(before_dirs) - set(after_dirs)),
        "added": sorted(after_names - before_names),
        "removed": sorted(before_names - after_names),
        "changed": sorted(name for name in before_names & after_names if before_files[name] != after_files[name]),
    }


def write_manifest(manifest: dict[str, object], json_path: Path, sha_path: Path) -> None:
    json_path.parent.mkdir(parents=True, exist_ok=True)
    json_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    files = manifest.get("files", {})
    if not isinstance(files, dict):
        raise FixtureError(f"{json_path}: malformed manifest")
    lines = [f"{item['sha256']}  {name}\n" for name, item in sorted(files.items())]
    sha_path.write_text("".join(lines), encoding="utf-8")


def copy_tree(source: Path, destination: Path) -> None:
    if destination.exists():
        raise FixtureError(f"{destination}: already exists")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source, destination, copy_function=shutil.copy2, symlinks=False)


def write_ini(path: Path, save_dir: Path, log_path: Path, row: int) -> None:
    text = (
        f"; Generated by tools/issue79_fixture.py for Issue #79 matrix row {row}.\n"
        "; The save below is a disposable copy. Never point this file at E:\\EmblemWeave.\n"
        "[fs]\n"
        f"save_dir = {save_dir.as_posix()}\n"
        "\n[log]\n"
        f"path = {log_path.as_posix()}\n"
        "level = info\n"
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="\n") as handle:
        handle.write(text)


def format_command(parts: list[str]) -> str:
    if os.name == "nt":
        return subprocess.list2cmdline(parts)
    return shlex.join(parts)


def build_command(args: argparse.Namespace, ini: Path, log: Path) -> str:
    engine = str(resolve_path(args.engine)) if args.engine else "<fefw.exe>"
    parts = [engine]
    if args.exefs:
        parts += ["--exefs", str(resolve_path(args.exefs))]
    if args.main:
        parts += ["--main", str(resolve_path(args.main))]
    parts += ["--romfs", str(resolve_path(args.romfs)) if args.romfs else "<ROMFS>"]
    parts += ["--ini", str(ini), "--log", str(log)]
    parts += ["--headless"] if args.headless else ["--window"]
    return format_command(parts)


def parse_rows(text: str) -> list[int]:
    if text.strip().lower() in {"all", "*"}:
        return list(range(1, 26))
    rows: set[int] = set()
    for part in re.split(r"[\s,]+", text.strip()):
        if not part:
            continue
        try:
            row = int(part)
        except ValueError as error:
            raise FixtureError(f"--rows: {part!r} is not a matrix row number") from error
        if not 1 <= row <= 25:
            raise FixtureError(f"--rows: {row} is outside the 1-25 matrix")
        rows.add(row)
    if not rows:
        raise FixtureError("--rows selected no rows")
    return sorted(rows)


def prepare_row(row: int, base_save: Path, row_root: Path, args: argparse.Namespace) -> tuple[dict[str, object], list[dict[str, str]]]:
    row_root.mkdir(parents=True, exist_ok=True)
    save = row_root / "save"
    copy_tree(base_save, save)
    active = save
    note = ""

    if row == 19:
        account = save / "account"
        default = account / DEFAULT_USER
        if default.is_dir():
            default.rename(account / FOREIGN_USER)
            note = f"moved account/{DEFAULT_USER} to account/{FOREIGN_USER}; account/{DEFAULT_USER}.meta is left in place."
        elif account.is_dir() and any(account.iterdir()):
            note = f"account/{DEFAULT_USER} was already absent; the foreign-account layout was kept."
        else:
            raise FixtureError(f"row 19 needs {account / DEFAULT_USER}; the source has no account save")
    elif row == 20:
        slot = save / "account" / DEFAULT_USER / "000"
        if not slot.is_dir():
            raise FixtureError(f"row 20 needs {slot}; the source save has no 000 folder")
        holder = save / "account" / DEFAULT_USER / ".000-hold"
        if holder.exists():
            raise FixtureError(f"row 20: {holder} already exists")
        slot.rename(holder)
        slot.mkdir()
        holder.rename(slot / "000")
        note = f"moved account/{DEFAULT_USER}/000 to account/{DEFAULT_USER}/000/000."
    elif row == 22:
        meta = save / "account" / f"{DEFAULT_USER}.meta"
        if meta.exists():
            meta.unlink()
            note = f"removed account/{DEFAULT_USER}.meta."
        else:
            note = f"account/{DEFAULT_USER}.meta was already missing."
    elif row in (16, 17):
        stale = row_root / "stale-target"
        copy_tree(base_save, stale)
        active = stale
        note = "the ini points at stale-target; save is the new root and must remain unchanged."
        if row == 17:
            (row_root / "portable.txt").write_text("", encoding="utf-8")
            note = "portable marker beside the fixture; " + note

    ini = row_root / "fefw.ini"
    log = row_root / "fefw.log"
    write_ini(ini, active, log, row)

    fixtures = [
        {
            "row": row,
            "role": "active-save",
            "path": str(active),
            "manifest": str(row_root / "fixture-before.json"),
        }
    ]
    write_manifest(tree_manifest(active), Path(fixtures[0]["manifest"]), row_root / "fixture-before.sha256")
    if active != save:
        extra = row_root / "new-root-before.json"
        write_manifest(tree_manifest(save), extra, row_root / "new-root-before.sha256")
        fixtures.append({"row": row, "role": "new-root", "path": str(save), "manifest": str(extra)})

    info = {
        "row": row,
        "save": str(save),
        "active": str(active),
        "ini": str(ini),
        "log": str(log),
        "note": note,
        "command": build_command(args, ini, log),
        "fixtures": fixtures,
    }
    return info, fixtures


def commands_text(run: dict[str, object], rows: list[int], args: argparse.Namespace) -> str:
    lines = [
        "Issue #79 fixture run commands",
        "==============================",
        "",
        f"Prepared: {run['created']}",
        f"Source (read-only): {run['source']}",
        f"Scratch: {run['scratch']}",
        f"Source before: {run['source_before']}",
        f"Source after: {run['source_after']}",
        "",
        "Rules:",
        "  - Run one row at a time. Each direct row has its own save copy and ini.",
        "  - Never point --source or --scratch at E:\\EmblemWeave or a live save root.",
        "  - Always pass --ini. Do not let baked engine defaults point at an install.",
        "  - A launcher badge is not evidence; the game must load the expected save.",
        "  - The harness only prints these commands; it never launches the game.",
        "",
    ]
    if not args.engine:
        lines.append("WARNING: --engine was not given; <fefw.exe> is a placeholder.")
    if not args.romfs:
        lines.append("WARNING: --romfs was not given; <ROMFS> is a placeholder.")
    if not args.exefs or not args.main:
        lines.append("WARNING: --exefs/--main were not given; the engine may use baked defaults.")
    lines.append("")

    for row in rows:
        title = ROW_TITLES.get(row, f"matrix row {row}")
        lines.append(f"[{row}] {title}")
        info = run["rows"].get(str(row))
        if isinstance(info, dict):
            lines.append(f"  fixture: {info['active']}")
            lines.append(f"  ini: {info['ini']}")
            if info.get("note"):
                lines.append(f"  note: {info['note']}")
            lines.append(f"  run: {info['command']}")
            lines.append(f"  expect: {ROW_EXPECT[row]}")
        elif row == 18:
            lines.append("  Deliberately not prepared: row 18 runs without --ini and would use the engine fallback root.")
            lines.append("  Test it only inside a scratch install, never against E: or a live save.")
        elif row == 24:
            lines.append(f"  {LAUNCHER_NOTE}")
            lines.append("  expect: the current save is backed up, the empty profile starts a new game, and Restore returns the old tree.")
        elif row in (21, 23, 25):
            lines.append(f"  Not prepared: matrix row {row} is a direct scenario outside the controlled plan; see the matrix before adding a fixture.")
        else:
            lines.append(f"  {LAUNCHER_NOTE}")
        lines.append("")
    lines.append("After the runs: python tools/issue79_fixture.py --verify <scratch>")
    lines.append("Source changes are a failure. Fixture changes are the game's writes and are listed for evidence.")
    return "\n".join(lines) + "\n"


def prepare(args: argparse.Namespace) -> int:
    source = resolve_path(args.source)
    if is_forbidden_source(source):
        raise FixtureError(
            f"{source}: refused. Pass a disposable COPY of a save tree, never E:\\EmblemWeave, "
            "%APPDATA%\\EmblemWeave, this repository's data folder, a drive root or the home folder."
        )
    if not source.is_dir():
        raise FixtureError(f"{source}: not a directory")
    if (source / "save").is_dir() and ((source / "config.json").is_file() or (source / "play").is_dir()):
        raise FixtureError(f"{source}: looks like a data root; pass its save subfolder instead")
    try:
        if not any(source.iterdir()):
            raise FixtureError(f"{source}: the save tree is empty")
    except OSError as error:
        raise FixtureError(f"{source}: cannot read: {error}") from error

    scratch = resolve_path(args.scratch) if args.scratch else (
        repo_root() / "target" / "validation" / "issue79" / datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    )
    if is_forbidden_scratch(scratch):
        raise FixtureError(f"{scratch}: refused as a scratch root; use a new folder outside E: and the live install")
    if inside(scratch, source) or inside(source, scratch):
        raise FixtureError(f"{scratch}: scratch and source must not contain one another")
    if scratch.exists() and any(scratch.iterdir()):
        raise FixtureError(f"{scratch}: scratch is not empty; choose a new run folder")
    scratch.mkdir(parents=True, exist_ok=True)

    rows = parse_rows(args.rows)
    manifests = scratch / "manifests"
    manifests.mkdir(parents=True, exist_ok=True)

    before = tree_manifest(source)
    before_json = manifests / "source-before.json"
    before_sha = manifests / "source-before.sha256"
    write_manifest(before, before_json, before_sha)

    base_save = scratch / "base-save"
    copy_tree(source, base_save)
    after = tree_manifest(source)
    after_json = manifests / "source-after.json"
    after_sha = manifests / "source-after.sha256"
    write_manifest(after, after_json, after_sha)
    if not manifest_equal(before, after):
        diff = manifest_diff(before, after)
        raise FixtureError(
            "the source changed while it was being copied; refusing to continue "
            f"(dirs added={diff['dirs_added']}, dirs removed={diff['dirs_removed']}, "
            f"added={diff['added']}, removed={diff['removed']}, changed={diff['changed']})"
        )

    base_manifest = tree_manifest(base_save)
    base_json = manifests / "base-save-before.json"
    write_manifest(base_manifest, base_json, manifests / "base-save-before.sha256")
    if not manifest_equal(before, base_manifest):
        raise FixtureError("the scratch copy does not match the source manifest; refusing to continue")

    created = datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")
    run: dict[str, object] = {
        "tool": "issue79-fixture",
        "version": 1,
        "created": created,
        "source": str(source),
        "scratch": str(scratch),
        "source_before": str(before_json),
        "source_after": str(after_json),
        "source_sha256": str(before_sha),
        "rows": {},
        "fixtures": [],
    }

    for row in rows:
        if row not in DIRECT_ROWS:
            continue
        info, fixtures = prepare_row(row, base_save, scratch / "runs" / f"row-{row}", args)
        run["rows"][str(row)] = info
        run["fixtures"].extend(fixtures)

    run_path = scratch / "run.json"
    run_path.write_text(json.dumps(run, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    text = commands_text(run, rows, args)
    commands_path = scratch / "matrix-commands.txt"
    commands_path.write_text(text, encoding="utf-8")

    files = before.get("files", {})
    print(f"prepared: {scratch}")
    print(f"source unchanged: yes ({len(files) if isinstance(files, dict) else 0} files)")
    print(f"source before: {before_sha}")
    print(f"source after:  {after_sha}")
    print(f"direct rows: {', '.join(str(row) for row in rows if row in DIRECT_ROWS) or 'none'}")
    print(f"commands: {commands_path}")
    print("")
    print(text)
    return 0


def verify(run_dir: Path) -> int:
    run_path = resolve_path(str(run_dir)) / "run.json"
    try:
        run = json.loads(run_path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        print(f"error: cannot read {run_path}: {error}", file=sys.stderr)
        return 2
    if run.get("tool") != "issue79-fixture":
        print(f"error: {run_path} is not an issue79-fixture run", file=sys.stderr)
        return 2
    source = Path(run["source"])
    try:
        before = json.loads(Path(run["source_before"]).read_text(encoding="utf-8"))
        after = tree_manifest(source)
    except (OSError, ValueError, FixtureError) as error:
        print(f"error: cannot verify the source: {error}", file=sys.stderr)
        return 2
    source_diff = manifest_diff(before, after)
    source_ok = not any(source_diff.values())
    print(f"source: {source}")
    print("source unchanged: yes" if source_ok else "source unchanged: NO")
    if not source_ok:
        for name in source_diff["dirs_added"]:
            print(f"  source dir added: {name}")
        for name in source_diff["dirs_removed"]:
            print(f"  source dir removed: {name}")
        for name in source_diff["added"]:
            print(f"  source added: {name}")
        for name in source_diff["removed"]:
            print(f"  source removed: {name}")
        for name in source_diff["changed"]:
            print(f"  source changed: {name}")

    for fixture in run.get("fixtures", []):
        path = Path(fixture["path"])
        try:
            baseline = json.loads(Path(fixture["manifest"]).read_text(encoding="utf-8"))
            current = tree_manifest(path)
        except (OSError, ValueError, FixtureError) as error:
            print(f"error: cannot verify {path}: {error}", file=sys.stderr)
            return 2
        diff = manifest_diff(baseline, current)
        label = f"row {fixture.get('row')} {fixture.get('role')}"
        if not any(diff.values()):
            print(f"{label}: no fixture changes")
            continue
        print(f"{label}: fixture changes (expected game writes):")
        for name in diff["dirs_added"]:
            print(f"  dir added: {name}")
        for name in diff["dirs_removed"]:
            print(f"  dir removed: {name}")
        for name in diff["added"]:
            print(f"  added: {name}")
        for name in diff["removed"]:
            print(f"  removed: {name}")
        for name in diff["changed"]:
            print(f"  changed: {name}")

    return 0 if source_ok else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--source", help="a COPY of the save tree to read (never the live save)")
    parser.add_argument("--scratch", help="new/empty scratch run folder (default: target/validation/issue79/<timestamp>)")
    parser.add_argument("--engine", help="fefw.exe path for the printed commands")
    parser.add_argument("--romfs", help="RomFS folder or .nup for --romfs")
    parser.add_argument("--exefs", help="ExeFS folder for --exefs")
    parser.add_argument("--main", help="uncompressed main NSO for --main")
    parser.add_argument("--rows", default="all", help="matrix rows to prepare, e.g. 19,20,16 or all (default: all)")
    parser.add_argument("--headless", action="store_true", help="print --headless instead of --window")
    parser.add_argument("--verify", metavar="RUN_DIR", help="re-hash a prepared run; no preparation")
    args = parser.parse_args(argv)

    if args.verify:
        if args.source or args.scratch or args.engine or args.romfs or args.exefs or args.main:
            parser.error("--verify cannot be combined with preparation options")
        return verify(Path(args.verify))
    if not args.source:
        parser.error("--source is required unless --verify is used")
    try:
        return prepare(args)
    except (FixtureError, OSError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
