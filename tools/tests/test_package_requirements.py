#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""The packager and release workflow must only require files that are in the public tree.

The 0.1.43 cleanup moved internal audits and release reports into the ignored
`docs/local-only/` folder. Both the packaging script and the release workflow
still listed `docs/RELEASE_VALIDATION.md`, so a correct HEAD archive would have
failed verification. This test reads the two required-file lists and checks the
paths on disk, which catches the same mistake the next time a document moves.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PACKAGE = ROOT / "tools" / "package_release.ps1"
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"

# These are created by the packager or the release workflow, not checked in.
# The shader translator is published into the tree by the release build and
# arrives with its three notices.
GENERATED = {
    "EmblemWeave.exe",
    "portable.txt",
    "engine/fefw-recomp/tools/shaders/prebuilt/win-x64/spirv_harness.exe",
    "engine/fefw-recomp/tools/shaders/prebuilt/win-x64/LICENSE.Ryujinx.MIT.txt",
    "engine/fefw-recomp/tools/shaders/prebuilt/win-x64/DOTNET-LICENSE.txt",
    "engine/fefw-recomp/tools/shaders/prebuilt/win-x64/DOTNET-THIRD-PARTY-NOTICES.txt",
}


def package_required_paths() -> list[str]:
    text = PACKAGE.read_text(encoding="utf-8")
    start = text.index("$required = @(")
    end = text.index("\n    )", start)
    return re.findall(r"'([^']+)'", text[start:end])


def workflow_required_paths() -> list[str]:
    text = WORKFLOW.read_text(encoding="utf-8")
    return re.findall(r'--require "EmblemWeave-v\$\{VERSION\}/([^"]+)"', text)


class RequiredFileTests(unittest.TestCase):
    def test_package_required_files_exist(self) -> None:
        missing = [
            path
            for path in package_required_paths()
            if path not in GENERATED and not (ROOT / path).exists()
        ]
        self.assertEqual(
            missing,
            [],
            "tools/package_release.ps1 requires files that are not in the public tree",
        )

    def test_workflow_required_files_exist(self) -> None:
        missing = [
            path
            for path in workflow_required_paths()
            if path not in GENERATED and not (ROOT / path).exists()
        ]
        self.assertEqual(
            missing,
            [],
            "the release workflow requires files that are not in the public tree",
        )

    def test_required_lists_have_no_duplicates(self) -> None:
        for label, paths in (
            ("package_release.ps1", package_required_paths()),
            ("release.yml", workflow_required_paths()),
        ):
            with self.subTest(list=label):
                self.assertEqual(
                    len(paths),
                    len(set(paths)),
                    f"{label} names the same required file more than once",
                )


if __name__ == "__main__":
    unittest.main()
