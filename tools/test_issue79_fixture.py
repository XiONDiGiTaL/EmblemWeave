#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests for tools/issue79_fixture.py: the safety refusals, the per-row layout, and --verify's drift checks.

Run from the repository root:  python -m unittest tools.test_issue79_fixture -v
Every test works in a temporary directory; none reads a live save.
"""
from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load_harness():
    spec = importlib.util.spec_from_file_location("issue79_fixture", HERE / "issue79_fixture.py")
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


h = load_harness()
DEFAULT_USER = h.DEFAULT_USER


def make_source(root: Path) -> Path:
    source = root / "source"
    slot = source / "account" / DEFAULT_USER / "000"
    slot.mkdir(parents=True)
    (slot / "000.bin").write_bytes(b"SAVE-PROGRESS-v1\n")
    (slot / "header.bin").write_bytes(b"HDR-2\n")
    (source / "account" / f"{DEFAULT_USER}.meta").write_text('{"size":4194304,"journal":1048576}\n', encoding="utf-8")
    return source


class FixtureHarness(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.source = make_source(self.root)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def run_main(self, *args: str) -> tuple[int, str]:
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf), contextlib.redirect_stderr(buf):
            code = h.main(list(args))
        return code, buf.getvalue()

    def test_a_live_root_is_refused_as_a_source(self) -> None:
        for forbidden in (Path(r"E:\EmblemWeave"), Path.home(), Path(self.root.anchor)):
            if os.name != "nt" and str(forbidden).startswith("E:"):
                continue
            self.assertTrue(h.is_forbidden_source(forbidden), str(forbidden))

    def test_the_repository_data_folder_is_refused(self) -> None:
        self.assertTrue(h.is_forbidden_source(h.repo_root() / "data"))

    def test_a_source_below_the_scratch_folder_is_refused(self) -> None:
        code, out = self.run_main("--source", str(self.source), "--scratch", str(self.source / "run"))
        self.assertNotEqual(code, 0)
        self.assertIn("scratch", out.lower())
        self.assertFalse((self.source / "run").exists())

    def test_a_symlink_in_the_source_is_refused(self) -> None:
        if not hasattr(os, "symlink"):
            self.skipTest("no symlinks on this host")
        link = self.source / "account" / "link"
        try:
            link.symlink_to(self.source / "account" / DEFAULT_USER, target_is_directory=True)
        except OSError:
            self.skipTest("symlinks need a privilege this account lacks")
        self.assertRaises(h.FixtureError, h.tree_manifest, self.source)

    def test_prepare_copies_every_row_and_leaves_the_source_alone(self) -> None:
        before = h.tree_manifest(self.source)
        scratch = self.root / "run"
        code, out = self.run_main("--source", str(self.source), "--scratch", str(scratch), "--rows", "16,17,19,20,22")
        self.assertEqual(code, 0, out)
        self.assertEqual(h.tree_manifest(self.source), before)   # source hashes unchanged
        for row in (16, 17, 19, 20, 22):
            info = json.loads((scratch / "run.json").read_text(encoding="utf-8"))["rows"][str(row)]
            self.assertTrue(Path(info["ini"]).is_file())
            self.assertIn("save_dir", Path(info["ini"]).read_text(encoding="utf-8"))

    def test_a_second_prepare_into_the_same_scratch_is_refused(self) -> None:
        scratch = self.root / "run"
        self.assertEqual(self.run_main("--source", str(self.source), "--scratch", str(scratch), "--rows", "22")[0], 0)
        code, out = self.run_main("--source", str(self.source), "--scratch", str(scratch), "--rows", "22")
        self.assertNotEqual(code, 0)
        self.assertIn("scratch is not empty", out.lower())

    def test_verify_reports_a_source_change_as_a_failure(self) -> None:
        scratch = self.root / "run"
        self.assertEqual(self.run_main("--source", str(self.source), "--scratch", str(scratch), "--rows", "22")[0], 0)
        # The game is supposed to write the fixture, never the source. Simulate a source write.
        (self.source / "account" / f"{DEFAULT_USER}.meta").write_text('{"size":1}\n', encoding="utf-8")
        code, out = self.run_main("--verify", str(scratch))
        self.assertNotEqual(code, 0)
        self.assertIn("source unchanged: no", out.lower())

    def test_verify_reports_fixture_writes_separately_from_source_writes(self) -> None:
        scratch = self.root / "run"
        self.assertEqual(self.run_main("--source", str(self.source), "--scratch", str(scratch), "--rows", "22")[0], 0)
        info = json.loads((scratch / "run.json").read_text(encoding="utf-8"))["rows"]["22"]
        (Path(info["active"]) / "account" / DEFAULT_USER / "000" / "000.bin").write_bytes(b"written by the game\n")
        code, out = self.run_main("--verify", str(scratch))
        self.assertEqual(code, 0, out)   # fixture writes are expected evidence, not a failure
        self.assertIn("source unchanged: yes", out)
        self.assertIn("changed: account/0100000000000000706d635257464546/000/000.bin", out)


if __name__ == "__main__":
    unittest.main()
