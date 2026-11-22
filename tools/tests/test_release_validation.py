#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests for the release archive, manifest and install verifiers.

Run with the standard library only:

    python -m unittest discover -s tools/tests -v
"""

from __future__ import annotations

import stat
import sys
import tempfile
import unittest
import warnings
import zipfile
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1]
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

import release_manifest  # noqa: E402
import verify_archive  # noqa: E402
import verify_install  # noqa: E402

VERSION = "0.1.40"
PREFIX = f"EmblemWeave-v{VERSION}/"


def make_zip(path: Path, entries: list[tuple[str, bytes]]) -> Path:
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", UserWarning)
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
            for name, data in entries:
                z.writestr(name, data)
    return path


def release_zip(path: Path, extra: list[tuple[str, bytes]] | None = None) -> Path:
    entries = [
        (PREFIX, b""),
        (PREFIX + "EmblemWeave.exe", b"launcher"),
        (PREFIX + "portable.txt", b""),
        (PREFIX + "docs/PACKAGE.md", b"# package\n"),
        (PREFIX + "engine/fefw-recomp/runtime/CMakeLists.txt", b"# cmake\n"),
    ]
    entries.extend((PREFIX + name, data) for name, data in (extra or []))
    return make_zip(path, entries)


class ArchiveNameTests(unittest.TestCase):
    def test_safe_release_path(self):
        self.assertEqual(verify_archive.problems_for_name(PREFIX + "docs/PACKAGE.md", "z"), [])

    def test_unsafe_paths(self):
        cases = [
            "../escape.txt",
            "/absolute.txt",
            "C:/drive.txt",
            "a\\backslash.txt",
            "a//empty.txt",
            "a/./dot.txt",
            "CON",
            "con.txt",
            "COM1.log",
            "trailing.",
            "trailing ",
            "colon:stream",
            "question?.txt",
            "less<than.txt",
            "pipe|name.txt",
            "nul.txt",
        ]
        for name in cases:
            with self.subTest(name=name):
                self.assertTrue(verify_archive.problems_for_name(name, "z"), name)

    def test_unicode_name_is_safe(self):
        self.assertEqual(verify_archive.problems_for_name(PREFIX + "docs/caf\u00e9.md", "z"), [])


class ArchiveZipTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def test_duplicate_exact_entry_is_rejected(self):
        path = make_zip(self.root / "dup.zip", [("a.txt", b"one"), ("a.txt", b"two")])
        problems, _, _ = verify_archive.check_zip(str(path), "", [])
        self.assertTrue(any("duplicate archive entry" in p for p in problems), problems)

    def test_case_collision_is_rejected(self):
        path = make_zip(self.root / "case.zip", [("A.txt", b"one"), ("a.txt", b"two")])
        problems, _, _ = verify_archive.check_zip(str(path), "", [])
        self.assertTrue(any("case folding" in p for p in problems), problems)

    def test_unicode_normalization_collision_is_rejected(self):
        path = make_zip(self.root / "unicode.zip", [("caf\u00e9.txt", b"one"), ("cafe\u0301.txt", b"two")])
        problems, _, _ = verify_archive.check_zip(str(path), "", [])
        self.assertTrue(any("Unicode normalization" in p for p in problems), problems)

    def test_required_file_is_required(self):
        path = release_zip(self.root / "ok.zip")
        problems, _, _ = verify_archive.check_zip(str(path), "", [PREFIX + "missing.txt"])
        self.assertTrue(any("required file" in p for p in problems), problems)

    def test_traversal_is_not_extracted(self):
        path = make_zip(self.root / "traversal.zip", [("../escape.txt", b"bad")])
        out = self.root / "out"
        problems, _, _ = verify_archive.check_zip(str(path), str(out), [])
        self.assertTrue(any("walks up" in p for p in problems), problems)
        self.assertFalse((self.root / "escape.txt").exists())
        self.assertFalse((out / ".." / "escape.txt").exists())

    def test_symlink_entry_is_rejected(self):
        path = self.root / "link.zip"
        with zipfile.ZipFile(path, "w") as z:
            info = zipfile.ZipInfo("link")
            info.external_attr = (stat.S_IFLNK | 0o777) << 16
            z.writestr(info, "target")
        problems, _, _ = verify_archive.check_zip(str(path), "", [])
        self.assertTrue(any("symbolic link" in p for p in problems), problems)

    def test_empty_archive_is_rejected(self):
        path = self.root / "empty.zip"
        zipfile.ZipFile(path, "w").close()
        problems, _, _ = verify_archive.check_zip(str(path), "", [])
        self.assertTrue(any("empty" in p for p in problems), problems)


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def manifest(self, zip_path: Path) -> dict:
        return release_manifest.create_manifest(
            zip_path,
            VERSION,
            source_commit="abc123",
            source_dirty=False,
            created_utc="2026-11-22T00:00:00Z",
        )

    def test_round_trip_lists_every_file(self):
        path = release_zip(self.root / "ok.zip")
        manifest = self.manifest(path)
        self.assertEqual(manifest["kind"], release_manifest.KIND)
        self.assertEqual(manifest["source"]["commit"], "abc123")
        self.assertEqual(manifest["archive"]["files"], 4)
        self.assertEqual(manifest["archive"]["prefix"], PREFIX)
        self.assertEqual(release_manifest.verify_manifest(path, manifest), [])
        paths = {item["path"] for item in manifest["files"]}
        self.assertIn(PREFIX + "EmblemWeave.exe", paths)
        self.assertNotIn(PREFIX, paths)

    def test_wrong_archive_hash_is_reported(self):
        path = release_zip(self.root / "ok.zip")
        manifest = self.manifest(path)
        manifest["archive"]["sha256"] = "0" * 64
        problems = release_manifest.verify_manifest(path, manifest)
        self.assertTrue(any("sha256 is" in p for p in problems), problems)

    def test_changed_file_is_reported(self):
        path = release_zip(self.root / "ok.zip")
        manifest = self.manifest(path)
        release_zip(path, [("EmblemWeave.exe", b"different")])
        problems = release_manifest.verify_manifest(path, manifest)
        self.assertTrue(any("EmblemWeave.exe" in p and "sha256" in p for p in problems), problems)

    def test_extra_file_is_reported(self):
        path = release_zip(self.root / "ok.zip")
        manifest = self.manifest(path)
        release_zip(path, [("docs/EXTRA.md", b"extra")])
        problems = release_manifest.verify_manifest(path, manifest)
        self.assertTrue(any("not in the manifest" in p for p in problems), problems)

    def test_missing_file_is_reported(self):
        path = release_zip(self.root / "ok.zip")
        manifest = self.manifest(path)
        make_zip(
            path,
            [
                (PREFIX, b""),
                (PREFIX + "EmblemWeave.exe", b"launcher"),
                (PREFIX + "portable.txt", b""),
                (PREFIX + "docs/PACKAGE.md", b"# package\n"),
            ],
        )
        problems = release_manifest.verify_manifest(path, manifest)
        self.assertTrue(any("missing from the archive" in p for p in problems), problems)

    def test_wrong_prefix_is_refused(self):
        path = make_zip(self.root / "wrong.zip", [("EmblemWeave-v0.1.39/EmblemWeave.exe", b"launcher")])
        with self.assertRaises(release_manifest.ManifestError):
            release_manifest.create_manifest(path, VERSION, source_dirty=False)

    def test_unsafe_zip_is_refused(self):
        path = make_zip(self.root / "bad.zip", [("../escape.txt", b"bad")])
        with self.assertRaises(release_manifest.ManifestError):
            release_manifest.create_manifest(path, VERSION, source_dirty=False)

    def test_manifest_file_round_trip(self):
        path = release_zip(self.root / "ok.zip")
        manifest = self.manifest(path)
        out = self.root / "release-manifest.json"
        release_manifest._write_json(str(out), manifest)
        loaded = release_manifest.load_manifest(out)
        self.assertEqual(release_manifest.verify_manifest(path, loaded), [])


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.zip = release_zip(self.root / "ok.zip")
        self.manifest = release_manifest.create_manifest(
            self.zip, VERSION, source_commit="abc123", source_dirty=False
        )
        self.extract = self.root / "extract"
        problems, _, _ = verify_archive.check_zip(str(self.zip), str(self.extract), [])
        self.assertEqual(problems, [])
        self.install = self.extract / f"EmblemWeave-v{VERSION}"

    def tearDown(self):
        self.tmp.cleanup()

    def test_matching_install_passes(self):
        failures, warnings = verify_install.verify_install(self.install, self.manifest)
        self.assertEqual(failures, [])
        self.assertEqual(warnings, [])

    def test_user_data_is_not_a_mismatch(self):
        (self.install / "data").mkdir()
        (self.install / "data" / "save.sav").write_bytes(b"save")
        (self.install / "upscalers").mkdir()
        (self.install / "upscalers" / "dlss.dll").write_bytes(b"dll")
        (self.install / "engine" / "fefw-recomp" / "work").mkdir(parents=True)
        (self.install / "engine" / "fefw-recomp" / "work" / "decoded.bin").write_bytes(b"game")
        (self.install / "release-manifest.json").write_bytes(b"kept beside the install")
        (self.install / "SHA256SUMS").write_bytes(b"checksums")
        failures, warnings = verify_install.verify_install(self.install, self.manifest, strict=True)
        self.assertEqual(failures, [])
        self.assertEqual(warnings, [])

    def test_tampered_release_file_fails(self):
        (self.install / "EmblemWeave.exe").write_bytes(b"other")
        failures, _ = verify_install.verify_install(self.install, self.manifest)
        self.assertTrue(any("EmblemWeave.exe" in f and "sha256" in f for f in failures), failures)

    def test_unknown_extra_is_a_warning_unless_strict(self):
        (self.install / "docs" / "STALE.md").write_bytes(b"old")
        failures, warnings = verify_install.verify_install(self.install, self.manifest)
        self.assertEqual(failures, [])
        self.assertTrue(any("STALE.md" in w for w in warnings), warnings)
        failures, _ = verify_install.verify_install(self.install, self.manifest, strict=True)
        self.assertTrue(any("STALE.md" in f for f in failures), failures)

    def test_missing_release_file_fails(self):
        (self.install / "docs" / "PACKAGE.md").unlink()
        failures, _ = verify_install.verify_install(self.install, self.manifest)
        self.assertTrue(any("missing docs/PACKAGE.md" in f for f in failures), failures)


if __name__ == "__main__":
    unittest.main()