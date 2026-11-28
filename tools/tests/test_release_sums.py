#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests for the combined release SHA256SUMS finalizer."""

from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1]
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

import release_sums  # noqa: E402

VERSION = "0.1.43"
ZIP_NAME = f"EmblemWeave-v{VERSION}-windows-x64.zip"
RELEASE_NAME = "release-manifest.json"


class ReleaseSumsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.zip = self.root / ZIP_NAME
        self.release = self.root / RELEASE_NAME
        self.package = self.root / "manifest.json"
        self.sums = self.root / "SHA256SUMS"
        self.zip.write_bytes(b"the current release archive\n")
        self.release.write_bytes(b'{"files": []}\n')
        self.write_package_manifest()

    def tearDown(self):
        self.tmp.cleanup()

    def write_package_manifest(self, **overrides):
        data = {
            "version": VERSION,
            "zip": self.zip.name,
            "zipSha": release_sums.sha256_file(self.zip),
            "zipSize": self.zip.stat().st_size,
            "releaseManifest": self.release.name,
            "releaseManifestSha256": release_sums.sha256_file(self.release),
        }
        data.update(overrides)
        self.package.write_text(json.dumps(data) + "\n", encoding="utf-8")

    def finalize(self):
        release_sums.finalize(
            self.sums,
            [self.zip, self.release],
            package_manifest=self.package,
        )

    def test_finalize_writes_exact_entries_and_never_itself(self):
        self.finalize()
        text = self.sums.read_text(encoding="ascii")
        self.assertEqual(
            text,
            f"{release_sums.sha256_file(self.zip)} *{ZIP_NAME}\n"
            f"{release_sums.sha256_file(self.release)} *{RELEASE_NAME}\n",
        )
        self.assertNotIn("SHA256SUMS", text)
        self.assertEqual(release_sums.verify_sums(self.sums, [self.zip, self.release]), [])

    def test_previous_candidate_sums_are_replaced_not_merged(self):
        old_zip = self.root / "old.zip"
        old_manifest = self.root / "old-release-manifest.json"
        old_zip.write_bytes(b"previous candidate")
        old_manifest.write_bytes(b"previous manifest")
        old_hash = hashlib.sha256(b"previous candidate").hexdigest()
        self.sums.write_text(
            f"{old_hash} *{old_zip.name}\n"
            f"{'0' * 64} *{old_manifest.name}\n"
            f"{'1' * 64} *translator-linux-x64.tar.gz\n",
            encoding="ascii",
        )

        self.finalize()

        text = self.sums.read_text(encoding="ascii")
        self.assertNotIn("old.zip", text)
        self.assertNotIn("old-release-manifest.json", text)
        self.assertNotIn("translator-linux-x64.tar.gz", text)
        self.assertNotIn(old_hash, text)
        self.assertEqual(len(text.splitlines()), 2)
        self.assertEqual(release_sums.verify_sums(self.sums, [self.zip, self.release]), [])

    def test_repeated_finalize_is_byte_identical(self):
        self.finalize()
        first = self.sums.read_bytes()
        self.finalize()
        self.assertEqual(self.sums.read_bytes(), first)
        self.assertEqual(release_sums.verify_sums(self.sums, [self.zip, self.release]), [])

    def test_missing_artifact_fails_before_old_sums_are_replaced(self):
        self.sums.write_text("old contents\n", encoding="ascii")
        self.zip.unlink()
        with self.assertRaises(release_sums.SumsError):
            self.finalize()
        self.assertEqual(self.sums.read_text(encoding="ascii"), "old contents\n")

    def test_zip_hash_disagreement_fails(self):
        self.write_package_manifest(zipSha="0" * 64)
        with self.assertRaises(release_sums.SumsError):
            self.finalize()
        self.assertFalse(self.sums.exists())

    def test_zip_size_disagreement_fails(self):
        self.write_package_manifest(zipSize=self.zip.stat().st_size + 1)
        with self.assertRaises(release_sums.SumsError):
            self.finalize()

    def test_release_manifest_disagreement_fails(self):
        self.write_package_manifest(releaseManifestSha256="0" * 64)
        with self.assertRaises(release_sums.SumsError):
            self.finalize()

    def test_package_manifest_must_name_the_actual_artifacts(self):
        self.write_package_manifest(zip="another.zip")
        with self.assertRaises(release_sums.SumsError):
            self.finalize()

    def test_package_manifest_must_have_every_checksum_field(self):
        data = json.loads(self.package.read_text(encoding="utf-8"))
        del data["releaseManifestSha256"]
        self.package.write_text(json.dumps(data) + "\n", encoding="utf-8")
        with self.assertRaises(release_sums.SumsError):
            self.finalize()

    def test_output_cannot_be_an_artifact(self):
        with self.assertRaises(release_sums.SumsError):
            release_sums.finalize(self.zip, [self.zip, self.release])

    def test_duplicate_artifact_name_is_rejected(self):
        duplicate = self.root / "other" / ZIP_NAME
        duplicate.parent.mkdir()
        duplicate.write_bytes(b"other")
        with self.assertRaises(release_sums.SumsError):
            release_sums.finalize(self.sums, [self.zip, duplicate, self.release])

    def test_verify_reports_changed_artifact(self):
        self.finalize()
        self.zip.write_bytes(b"tampered\n")
        problems = release_sums.verify_sums(self.sums, [self.zip, self.release])
        self.assertTrue(any("sha256 is" in problem for problem in problems), problems)

    def test_verify_reports_unexpected_entry(self):
        self.finalize()
        with self.sums.open("a", encoding="ascii", newline="\n") as stream:
            stream.write(f"{'2' * 64} *old.zip\n")
        problems = release_sums.verify_sums(self.sums, [self.zip, self.release])
        self.assertTrue(any("unexpected artifact" in problem for problem in problems), problems)

    def test_verify_reports_missing_entry(self):
        self.finalize()
        lines = self.sums.read_text(encoding="ascii").splitlines()
        self.sums.write_text(lines[0] + "\n", encoding="ascii")
        problems = release_sums.verify_sums(self.sums, [self.zip, self.release])
        self.assertTrue(any("has no checksum entry" in problem for problem in problems), problems)

    def test_verify_reports_duplicate_entry(self):
        self.finalize()
        first = self.sums.read_text(encoding="ascii").splitlines()[0]
        with self.sums.open("a", encoding="ascii", newline="\n") as stream:
            stream.write(first + "\n")
        problems = release_sums.verify_sums(self.sums, [self.zip, self.release])
        self.assertTrue(any("duplicate artifact" in problem for problem in problems), problems)

    def test_verify_rejects_a_self_entry(self):
        self.finalize()
        with self.sums.open("a", encoding="ascii", newline="\n") as stream:
            stream.write(f"{'3' * 64} *SHA256SUMS\n")
        problems = release_sums.verify_sums(self.sums, [self.zip, self.release])
        self.assertTrue(any("must not list itself" in problem for problem in problems), problems)

    def test_verify_rejects_malformed_or_empty_files(self):
        for text in ("", "not a checksum\n", f"{'0' * 64}   spaced.txt\n"):
            with self.subTest(text=text):
                self.sums.write_text(text, encoding="ascii")
                problems = release_sums.verify_sums(self.sums, [self.zip, self.release])
                self.assertTrue(problems)


if __name__ == "__main__":
    unittest.main()
