#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Assert that mpr_launcher_tests skips exactly the two MPR-data cases.

The repository has no games/mpr, so the launcher suite builds against the FEFW
game data: the factory controls.ini and its bindings do not exist, and exactly
two cases skip for that reason. A suite that loses a case, or that quietly
turns another case into a skip, must not pass because "two skips" still looks
expected. This check pins both the skip names and the size of the whole suite.

    python tools/check_launcher_skips.py \
        --binary engine/fefw-recomp/runtime/build/release/mpr_launcher_tests.exe

--log parses a saved run instead of starting the binary.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The two cases that need Metroid Prime Remastered's factory controls.ini.
EXPECTED_SKIPS = {
    "launcher_bindings_edit.factory_fire_is_left_mouse_only",
    "launcher_tweaks.tweaks_factory_file_has_the_section_off",
}

# The whole launcher suite. Update this together with EXPECTED_SKIPS when a case
# is added or removed, so a lost case cannot pass as an expected skip.
EXPECTED_CASES = 69

STATUS_RE = re.compile(r"^\[\s*(RUN|OK|SKIP|FAIL)\s*\]\s*(\S.*?)\s*$")
SUMMARY_RE = re.compile(r"^SUMMARY .*?: (\d+) passed, (\d+) failed, (\d+) skipped \(of (\d+)\)\s*$")
REASON_RE = re.compile(r"^\s+SKIP:\s*(.*\S)\s*$")


def run_suite(binary: Path) -> tuple[int, str]:
    completed = subprocess.run(
        [str(binary), "--all"],
        cwd=str(binary.parent),
        capture_output=True,
        encoding="utf-8",
        errors="replace",
        timeout=300,
    )
    return completed.returncode, (completed.stdout or "") + (completed.stderr or "")


def parse(output: str) -> tuple[list[str], dict[str, str], dict[str, str], tuple[int, int, int, int] | None]:
    started: list[str] = []
    finished: dict[str, str] = {}
    reasons: dict[str, str] = {}
    summary: tuple[int, int, int, int] | None = None
    current: str | None = None
    for line in output.splitlines():
        match = STATUS_RE.match(line)
        if match:
            status, name = match.group(1), match.group(2)
            if status == "RUN":
                started.append(name)
                current = name
            else:
                finished[name] = status
                current = None
            continue
        if current and current not in finished:
            reason = REASON_RE.match(line)
            if reason:
                reasons[current] = reason.group(1)
        found = SUMMARY_RE.match(line)
        if found:
            summary = tuple(int(value) for value in found.groups())  # type: ignore[assignment]
    return started, finished, reasons, summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "--binary",
        type=Path,
        default=ROOT / "engine/fefw-recomp/runtime/build/release/mpr_launcher_tests.exe",
        help="the mpr_launcher_tests executable to run",
    )
    parser.add_argument("--log", type=Path, help="parse this saved run instead of starting the binary")
    args = parser.parse_args(argv)

    if args.log:
        if not args.log.is_file():
            print(f"error: {args.log} does not exist", file=sys.stderr)
            return 2
        returncode, output = 0, args.log.read_text(encoding="utf-8", errors="replace")
    else:
        binary = args.binary if args.binary.is_absolute() else ROOT / args.binary
        if not binary.is_file():
            print(f"error: {binary} does not exist; build the mpr_launcher_tests target first", file=sys.stderr)
            return 2
        try:
            returncode, output = run_suite(binary)
        except (OSError, subprocess.TimeoutExpired) as error:
            print(f"error: could not run {binary}: {error}", file=sys.stderr)
            return 2

    started, finished, reasons, summary = parse(output)
    problems: list[str] = []

    if args.log is None and returncode != 0:
        problems.append(f"the suite exited {returncode}")

    duplicates = [name for name, count in Counter(started).items() if count > 1]
    if duplicates:
        problems.append("case(s) started more than once: " + ", ".join(sorted(duplicates)))
    unfinished = [name for name in started if name not in finished]
    if unfinished:
        problems.append("case(s) started but never reported a result: " + ", ".join(unfinished))

    counts = Counter(finished.values())
    if summary is None:
        problems.append("no SUMMARY line: the run did not finish")
    else:
        passed, failed, skipped, ran = summary
        if failed:
            problems.append(f"{failed} case(s) failed")
        if counts["FAIL"] != failed:
            problems.append(f"SUMMARY says {failed} failed, the output reports {counts['FAIL']}")
        if counts["OK"] != passed:
            problems.append(f"SUMMARY says {passed} passed, the output reports {counts['OK']}")
        if counts["SKIP"] != skipped:
            problems.append(f"SUMMARY says {skipped} skipped, the output reports {counts['SKIP']}")
        if ran != EXPECTED_CASES:
            problems.append(f"{ran} case(s) ran, expected {EXPECTED_CASES} (update EXPECTED_CASES deliberately when the suite changes)")
        if len(started) != ran:
            problems.append(f"{len(started)} case(s) started, SUMMARY says {ran}")

    actual_skips = {name for name, status in finished.items() if status == "SKIP"}
    missing = sorted(EXPECTED_SKIPS - actual_skips)
    unexpected = sorted(actual_skips - EXPECTED_SKIPS)
    for name in missing:
        problems.append(f"expected MPR-data skip did not happen: {name}")
    for name in unexpected:
        problems.append(f"unexpected skip: {name} (a lost case cannot hide as an expected skip)")

    if problems:
        print(f"launcher skip baseline FAILED ({len(problems)} problem(s)):", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        printed = sorted(actual_skips & reasons.keys())
        if printed:
            print("skip reasons:", file=sys.stderr)
            for name in printed:
                print(f"  {name}: {reasons[name]}", file=sys.stderr)
        failed_names = sorted(name for name, status in finished.items() if status == "FAIL")
        if failed_names:
            print("failed cases:", file=sys.stderr)
            for name in failed_names:
                print(f"  {name}", file=sys.stderr)
        return 1

    print(f"launcher skip baseline OK: {summary[0]} passed, {summary[1]} failed, {summary[2]} skipped (of {summary[3]})")
    for name in sorted(EXPECTED_SKIPS):
        print(f"  skipped as expected: {name}: {reasons.get(name, '<no reason printed>')}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
