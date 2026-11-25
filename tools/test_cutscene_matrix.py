#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Focused tests for tools/cutscene_matrix.py.

Run from the repository root:  python -m unittest tools.test_cutscene_matrix -v
The tests use temporary folders and log samples; they never launch the game.
"""
from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent


def load_harness():
    spec = importlib.util.spec_from_file_location("cutscene_matrix", HERE / "cutscene_matrix.py")
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


h = load_harness()

STATS = (
    "movie: stats: packets 248, pictures 183, frames acquired 154, released 150, "
    "queue 64 packets (1.8 MiB, max 64), feed waits 183 (5015.9 ms); "
    "decode 6.58 ms per packet (slowest 32.2) over 180, 0 late pictures skipped; "
    "frame gaps 32.7 ms mean (p95 <= 66.7, p99 <= 66.7, max 90.5), 11 over 50 ms"
)
ACQUIRE = "movie: AcquireVideoFrame: 7ff6a000 at 16666667 (luma 3686400, chroma 1843200 bytes)"
EOS = "movie: AcquireVideoFrame: 7ff6b000 at 110033333 (luma 0, chroma 0 bytes, end of stream)"


def stats(late: int = 0) -> h.MovieStats:
    return h.MovieStats(
        packets=248,
        pictures=183,
        acquired=154,
        released=150,
        queue=64,
        queue_mib=1.8,
        queue_max=64,
        feed_waits=183,
        feed_ms=5015.9,
        decode_ms=6.58,
        slowest_ms=32.2,
        decode_n=180,
        late=late,
        gap_mean_ms=32.7,
        gap_p95_ms=66.7,
        gap_p99_ms=66.7,
        gap_max_ms=90.5,
        over50=11,
    )


class ParserTests(unittest.TestCase):
    def test_stats_and_acquire(self):
        analysis = h.Analysis()
        analysis.feed("[ 57.449638 I t23228 guest-118] " + STATS)
        analysis.feed("[ 57.450000 I t23228 guest-118] " + ACQUIRE)
        analysis.feed("[ 57.451000 I t23228 guest-118] " + EOS)
        self.assertEqual(len(analysis.stats), 1)
        self.assertEqual(analysis.stats[0].pictures, 183)
        self.assertEqual(analysis.acquires, 2)
        self.assertTrue(analysis.eos)
        self.assertEqual(analysis.eos_at, 57.451)

    def test_gpu_and_player_lines(self):
        analysis = h.Analysis()
        analysis.feed("[ 1.0 I t1 -] gfx: using GPU 1: AMD Radeon RX 7900 XT (RADV NAVI31) (override)")
        analysis.feed("[ 1.1 I t1 -] boot: NVN back end running on AMD Radeon RX 7900 XT (headless)")
        self.assertEqual(analysis.gpu_index, 1)
        self.assertEqual(analysis.gpu_actual, "AMD Radeon RX 7900 XT (RADV NAVI31)")
        self.assertTrue(h.gpu_matches("AMD Radeon RX 7900 XT", analysis.gpu_actual, analysis.gpu_index))
        self.assertTrue(h.gpu_matches("1", analysis.gpu_actual, analysis.gpu_index))

    def test_gpu_list(self):
        gpus = h.parse_gpu_list("GPU 0\tAMD Radeon RX 7900 XT\tdiscrete\t21458059264\r\n")
        self.assertEqual(gpus, [(0, "AMD Radeon RX 7900 XT")])


class ResultTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.movie = self.root / "MV00_00.webm"
        self.movie.write_bytes(b"webm")
        self.run_dir = self.root / "runs" / "MV00_00"
        self.run_dir.mkdir(parents=True)
        self.log = self.run_dir / "fefw.log"
        self.log.write_text("log\n", encoding="utf-8")
        self.ini = self.run_dir / "fefw.ini"
        self.ini.write_text("ini\n", encoding="utf-8")

    def tearDown(self):
        self.temp.cleanup()

    def row(self, analysis: h.Analysis, decoder: str = "dav1d", hwaccel: bool = True):
        args = SimpleNamespace(gpu="AMD Radeon RX 7900 XT", decoder=decoder, hwaccel=hwaccel)
        return h.result_row(self.movie, args, analysis, 12.5, 0, False, self.ini, self.log, self.run_dir, "A@12")

    def passing_analysis(self) -> h.Analysis:
        analysis = h.Analysis()
        analysis.movie_player = True
        analysis.started = True
        analysis.started_at = 2.0
        analysis.decoder_banner = "movie: movie::VideoDecoder on the host: AV1 through dav1d 1.5.1"
        analysis.gpu_actual = "AMD Radeon RX 7900 XT"
        analysis.gpu_index = 0
        analysis.stats = [stats()]
        analysis.acquires = 10
        analysis.eos = True
        analysis.eos_at = 110.0
        return analysis

    def test_pass(self):
        row = self.row(self.passing_analysis())
        self.assertEqual(row["status"], "PASS")
        self.assertEqual(row["reason"], "")
        self.assertEqual(row["eos_frame"], "yes")

    def test_late_skip_fails(self):
        analysis = self.passing_analysis()
        analysis.stats = [stats(late=3)]
        row = self.row(analysis)
        self.assertEqual(row["status"], "FAIL")
        self.assertIn("late picture", row["reason"])

    def test_stall_fails_even_with_eos(self):
        analysis = self.passing_analysis()
        analysis.feed("[ 3.0 E t1 movie] movie: movie decoder made no progress for 5.0 s while the game presented 60 frames; stopping the movie")
        row = self.row(analysis)
        self.assertEqual(row["status"], "FAIL")
        self.assertIn("decoder stalled", row["reason"])

    def test_wrong_gpu_fails(self):
        analysis = self.passing_analysis()
        analysis.gpu_actual = "Intel Arc A770"
        row = self.row(analysis)
        self.assertEqual(row["status"], "FAIL")
        self.assertIn("not 'AMD Radeon RX 7900 XT'", row["reason"])

    def test_ffmpeg_software_needs_no_d3d11va(self):
        analysis = self.passing_analysis()
        analysis.decoder_banner = "AV1 through FFmpeg N-123619 (software, 8 threads"
        analysis.feed("[ 3.0 I t1 movie-vdec] movie: FFmpeg AV1 decoder: libdav1d, 8 threads, software")
        row = self.row(analysis, decoder="ffmpeg", hwaccel=False)
        self.assertEqual(row["status"], "PASS")

    def test_ffmpeg_hardware_fallback_fails(self):
        analysis = self.passing_analysis()
        analysis.decoder_banner = "AV1 through FFmpeg N-123619 (D3D11VA requested, 8 threads"
        analysis.feed("[ 3.0 I t1 movie-vdec] movie: FFmpeg AV1 decoder: libdav1d, 8 threads, software")
        row = self.row(analysis, decoder="ffmpeg", hwaccel=True)
        self.assertEqual(row["status"], "FAIL")
        self.assertIn("D3D11VA was not active", row["reason"])

    def test_ffmpeg_hardware_active_passes(self):
        analysis = self.passing_analysis()
        analysis.decoder_banner = "AV1 through FFmpeg N-123619 (D3D11VA requested, 8 threads"
        analysis.feed("[ 3.0 I t1 movie-vdec] movie: FFmpeg AV1 decoder: av1, 1 threads, D3D11VA")
        row = self.row(analysis, decoder="ffmpeg", hwaccel=True)
        self.assertEqual(row["status"], "PASS")


class LayoutTests(unittest.TestCase):
    def test_mod_movie_is_under_romfs(self):
        path = h.mod_movie_path(Path("runs") / "MV40_02")
        self.assertEqual(path, Path("runs/MV40_02/mods/01-cutscene/romfs/Movie/Product/MV00_00.webm"))


class DiscoveryTests(unittest.TestCase):
    def test_product_folder_and_exact_count(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            product = root / "Movie" / "Product"
            product.mkdir(parents=True)
            for index in range(h.EXPECTED_CUTSCENES):
                (product / f"MV{index:02d}_00.webm").write_bytes(b"")
            resolved = h.resolve_movies_dir(root)
            movies = h.discover_movies(resolved, h.EXPECTED_CUTSCENES)
            self.assertEqual(len(movies), h.EXPECTED_CUTSCENES)
            self.assertEqual(movies[0].name, "MV00_00.webm")

    def test_wrong_count_is_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            product = Path(temp) / "Movie" / "Product"
            product.mkdir(parents=True)
            (product / "MV00_00.webm").write_bytes(b"")
            with self.assertRaises(SystemExit):
                h.discover_movies(h.resolve_movies_dir(Path(temp)), h.EXPECTED_CUTSCENES)

    def test_only_accepts_a_file_name(self):
        movies = [Path("MV00_00.webm"), Path("MV00_01.webm")]
        selected = h.select_movies(movies, "MV00_01.webm", None)
        self.assertEqual([m.name for m in selected], ["MV00_01.webm"])


if __name__ == "__main__":
    unittest.main()
