#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Run every Movie/Product/*.webm cutscene through the game's movie player.

The game plays ``Movie/Product/MV00_00.webm`` automatically at start-up.  For
each cutscene this harness writes a disposable RomFS mod that puts the target
at that path, launches one ``fefw.exe`` with a fresh save and a dedicated ini,
and stops it once the player acquires the end-of-stream frame.  Nothing in the
runtime or the game data is modified.

Captured evidence per row:

* ``movie: stats:`` lines (packets, pictures, acquired/released frames, decode
  time, late skips, frame gaps);
* ``movie: AcquireVideoFrame:`` lines, including the end-of-stream frame;
* the selected Vulkan adapter and the decoder banner.

The result is one CSV row per cutscene.  A row passes when the movie opened,
reached end of stream, produced stats and acquired frames, kept the requested
GPU, had no decoder/stall errors, and skipped no late pictures.

Examples::

    python tools/cutscene_matrix.py --list \
        --movies-dir "X:/FEFW/FEFW ROM/131072/romfs/Movie/Product"

    python tools/cutscene_matrix.py \
        --engine engine/fefw-recomp/game-build/fefw/build/release/fefw.exe \
        --base-ini _recovery/boot-check/fefw.ini \
        --movies-dir "X:/FEFW/FEFW ROM/131072/romfs/Movie/Product" \
        --gpu "AMD Radeon RX 7900 XT" --headless

The base ini supplies the game paths and shader caches.  The harness overrides
``[fs] save_dir``, ``[fs] mods``, ``[log] path``, ``[gfx] gpu`` and the movie
decoder keys in the per-run copy.  A headless run needs a scripted title-screen
press (``--press A@12`` by default); use ``--window`` to see the movie.

The stats values in the CSV are from the last ``movie: stats:`` line the
runtime emitted.  The runtime emits one on the first acquired frame and then
every 5 s, so a cutscene shorter than that (``stats_samples`` is 1) has only
the initial partial sample; ``acquire_lines`` is the actual frame count.
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import re
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path

EXPECTED_CUTSCENES = 82
OPENING_MOVIE = "MV00_00.webm"
PRODUCT_DIR = Path("Movie") / "Product"

# Lines that mean the decoder or player failed, even if an EOS frame follows.
ERROR_LINES = {
    "movie decoder made no progress": "decoder stalled",
    "movie stall state": "decoder stall state",
    "AV1 decode error": "AV1 decode error",
    "out of memory for": "decoder out of memory",
    "frames left empty": "picture did not fit its frame",
    "does not fit its frame": "picture did not fit its frame",
    "dav1d failed to start": "dav1d failed to start",
    "FFmpeg is selected but its AV1 decoder could not start": "FFmpeg decoder did not start",
    "D3D11VA failed": "D3D11VA failed",
    "switched to its libdav1d": "FFmpeg fell back to software",
    "a picture dropped": "picture dropped",
    "is not decoded on the host": "unsupported video codec",
    "neither configOBUs nor an av1C record": "invalid AV1 configuration",
    "guest nn::os / nn::movie::MediaProperty functions missing": "movie imports missing",
}

# A stats line is emitted on the first AcquireVideoFrame and then every 5 s.
STATS_RE = re.compile(
    r"movie: stats: packets (?P<packets>\d+), pictures (?P<pictures>\d+), "
    r"frames acquired (?P<acquired>\d+), released (?P<released>\d+), "
    r"queue (?P<queue>\d+) packets \((?P<queue_mib>[0-9.]+) MiB, max (?P<queue_max>\d+)\), "
    r"feed waits (?P<feed_waits>\d+) \((?P<feed_ms>[0-9.]+) ms\); "
    r"decode (?P<decode>[0-9.]+) ms per packet \(slowest (?P<slowest>[0-9.]+)\) over (?P<decode_n>\d+), "
    r"(?P<late>\d+) late pictures skipped; "
    r"frame gaps (?P<gap_mean>[0-9.]+) ms mean \(p95 <= (?P<p95>[0-9.]+), p99 <= (?P<p99>[0-9.]+), max (?P<gap_max>[0-9.]+)\), "
    r"(?P<over50>\d+) over 50 ms"
)
ACQUIRE_RE = re.compile(
    r"movie: AcquireVideoFrame: (?P<addr>[0-9a-fA-F]+) at (?P<time>-?\d+) "
    r"\(luma (?P<luma>\d+), chroma (?P<chroma>\d+) bytes(?P<eos>, end of stream)?\)"
)
GPU_USE_RE = re.compile(r"using GPU (?P<index>\d+): (?P<name>.*)")
GPU_READY_RE = re.compile(r"NVN back end running on (?P<name>.*) \((?:headless|window)\)$")
LOG_TIME_RE = re.compile(r"^\[\s*(?P<t>[0-9.]+) ")


@dataclass
class MovieStats:
    packets: int
    pictures: int
    acquired: int
    released: int
    queue: int
    queue_mib: float
    queue_max: int
    feed_waits: int
    feed_ms: float
    decode_ms: float
    slowest_ms: float
    decode_n: int
    late: int
    gap_mean_ms: float
    gap_p95_ms: float
    gap_p99_ms: float
    gap_max_ms: float
    over50: int


@dataclass
class Analysis:
    movie_player: bool = False
    started: bool = False
    started_at: float | None = None
    decoder_line: str = ""
    decoder_banner: str = ""
    gpu_index: int | None = None
    gpu_actual: str = ""
    gpu_line: str = ""
    stats: list[MovieStats] = field(default_factory=list)
    acquires: int = 0
    empty_frames: int = 0
    eos: bool = False
    eos_at: float | None = None
    late_skip_messages: int = 0
    errors: list[str] = field(default_factory=list)
    d3d11va: bool = False
    ffmpeg_decoder_line: str = ""
    ffmpeg_seen: bool = False

    def feed(self, line: str) -> None:
        t = log_time(line)
        if "movie: stats:" in line:
            match = STATS_RE.search(line)
            if match:
                self.stats.append(
                    MovieStats(
                        packets=int(match["packets"]),
                        pictures=int(match["pictures"]),
                        acquired=int(match["acquired"]),
                        released=int(match["released"]),
                        queue=int(match["queue"]),
                        queue_mib=float(match["queue_mib"]),
                        queue_max=int(match["queue_max"]),
                        feed_waits=int(match["feed_waits"]),
                        feed_ms=float(match["feed_ms"]),
                        decode_ms=float(match["decode"]),
                        slowest_ms=float(match["slowest"]),
                        decode_n=int(match["decode_n"]),
                        late=int(match["late"]),
                        gap_mean_ms=float(match["gap_mean"]),
                        gap_p95_ms=float(match["p95"]),
                        gap_p99_ms=float(match["p99"]),
                        gap_max_ms=float(match["gap_max"]),
                        over50=int(match["over50"]),
                    )
                )
        if "movie: AcquireVideoFrame:" in line:
            match = ACQUIRE_RE.search(line)
            if match:
                self.acquires += 1
                if match["eos"]:
                    self.eos = True
                    self.eos_at = t
                elif int(match["luma"]) == 0:
                    self.empty_frames += 1
        if "fefw: movie player:" in line:
            self.movie_player = True
        if "movie: VideoDecoder:" in line:
            self.started = True
            self.started_at = t
            self.decoder_line = line
        if "movie::VideoDecoder on the host:" in line:
            self.decoder_banner = line
        if "movie clock: late pictures skipped" in line:
            self.late_skip_messages += 1
        if "movie: FFmpeg AV1 decoder:" in line:
            self.ffmpeg_decoder_line = line
            if "D3D11VA" in line.upper() and "software" not in line.casefold():
                self.d3d11va = True
        if "FFmpeg" in line:
            self.ffmpeg_seen = True
        match = GPU_USE_RE.search(line)
        if match:
            self.gpu_index = int(match["index"])
            name = match["name"].strip()
            for suffix in (" (override)", " (default rule)"):
                if name.endswith(suffix):
                    name = name[: -len(suffix)].rstrip()
            self.gpu_actual = name
            self.gpu_line = line
        match = GPU_READY_RE.search(line)
        if match and not self.gpu_actual:
            self.gpu_actual = match["name"].strip()
            self.gpu_line = line

        for needle, reason in ERROR_LINES.items():
            if needle in line and reason not in self.errors:
                self.errors.append(reason)


def log_time(line: str) -> float | None:
    match = LOG_TIME_RE.match(line)
    return float(match["t"]) if match else None


def normalize_name(value: str) -> str:
    return " ".join(value.split()).casefold()


def gpu_matches(requested: str, actual: str, index: int | None) -> bool:
    requested = requested.strip()
    if requested.isdigit():
        return index is not None and int(requested) == index
    wanted = normalize_name(requested)
    have = normalize_name(actual)
    return bool(wanted and have and (wanted == have or wanted in have or have in wanted))


def parse_gpu_list(text: str) -> list[tuple[int, str]]:
    """Parse ``fefw.exe --list-gpus`` lines: ``GPU 0<TAB>name<TAB>kind<TAB>bytes``."""
    gpus: list[tuple[int, str]] = []
    for line in text.splitlines():
        parts = line.strip().split("\t")
        if len(parts) < 2 or not parts[0].startswith("GPU "):
            continue
        try:
            index = int(parts[0].split()[1])
        except (IndexError, ValueError):
            continue
        gpus.append((index, parts[1].strip()))
    return gpus


def check_gpu(engine: Path, requested: str) -> list[tuple[int, str]]:
    """Fail before the matrix if --gpu names no adapter the engine can see."""
    try:
        result = subprocess.run(
            [str(engine), "--list-gpus"],
            capture_output=True,
            text=True,
            timeout=60,
        )
    except (OSError, subprocess.SubprocessError) as error:
        print(f"warning: could not list GPUs: {error}", file=sys.stderr)
        return []
    gpus = parse_gpu_list(result.stdout or "")
    if not gpus:
        print("warning: the engine listed no GPUs; the run will still assert the selected one", file=sys.stderr)
        return []
    if not any(gpu_matches(requested, name, index) for index, name in gpus):
        shown = ", ".join(f"{index}: {name}" for index, name in gpus)
        raise SystemExit(f"error: --gpu {requested!r} matches none of: {shown}")
    return gpus


def resolve_movies_dir(path: Path) -> Path:
    path = path.resolve()
    if not path.is_dir():
        raise SystemExit(
            f"error: {path} is not a directory. Point --movies-dir at the extracted "
            "RomFS Movie/Product folder (a .nup cannot be read by this harness)."
        )
    if (path / PRODUCT_DIR).is_dir():
        return path / PRODUCT_DIR
    if path.name.casefold() == "product" or any(path.glob("*.webm")):
        return path
    raise SystemExit(f"error: {path} holds no Movie/Product/*.webm files")


def discover_movies(movies_dir: Path, expected: int) -> list[Path]:
    movies = sorted(
        (p for p in movies_dir.iterdir() if p.is_file() and p.suffix.casefold() == ".webm"),
        key=lambda p: p.name.casefold(),
    )
    if expected and len(movies) != expected:
        raise SystemExit(
            f"error: {movies_dir} has {len(movies)} .webm files, expected {expected}. "
            "Use --expect-count 0 only for a partial/exploratory folder."
        )
    if not movies:
        raise SystemExit(f"error: no .webm files in {movies_dir}")
    return movies


def select_movies(movies: list[Path], only: str | None, limit: int | None) -> list[Path]:
    selected = movies
    if only:
        wanted = {part.strip().casefold() for part in only.split(",") if part.strip()}
        known = {m.stem.casefold(): m for m in movies}
        known.update({m.name.casefold(): m for m in movies})
        missing = sorted(wanted - set(known))
        if missing:
            raise SystemExit(f"error: --only names no such cutscene: {', '.join(missing)}")
        selected = [
            m for m in movies if m.stem.casefold() in wanted or m.name.casefold() in wanted
        ]
    if limit is not None:
        selected = selected[:limit]
    if not selected:
        raise SystemExit("error: the selection is empty")
    return selected


def ini_value(value: str) -> str:
    """Quote a value the runtime's ini parser would otherwise truncate."""
    if not value or value != value.strip() or value.startswith('"') or re.search(r"[;#]", value):
        return '"' + value.replace('"', "") + '"'
    return value


def append_overrides(base: str, overrides: list[tuple[str, str, str]]) -> str:
    """Append ini keys; the runtime's parser keeps the last value for a key."""
    lines = [base.rstrip(), "", "; --- tools/cutscene_matrix.py per-run overrides ---"]
    for section, key, value in overrides:
        lines += [f"[{section}]", f"{key} = {ini_value(str(value))}"]
    return "\n".join(lines) + "\n"


def remove_env_overrides(env: dict[str, str]) -> dict[str, str]:
    """Let the run's ini decide these keys instead of a shell's environment."""
    drop = {
        "NXRT_INI",
        "NXRT_LOG_PATH",
        "NXRT_FS_EXEFS",
        "NXRT_FS_MAIN",
        "NXRT_FS_ROMFS",
        "NXRT_FS_SAVE_DIR",
        "NXRT_FS_MODS",
        "NXRT_GFX_GPU",
        "NXRT_GFX_HEADLESS",
        "NXRT_BOOT_GFX",
        "NXRT_BOOT_MAX_SECONDS",
        "NXRT_MEDIA_MOVIES",
        "NXRT_MEDIA_MOVIE_DECODER",
        "NXRT_MEDIA_MOVIE_HWACCEL",
        "NXRT_MEDIA_VIDEO_DECODER_LOG",
    }
    return {k: v for k, v in env.items() if k.upper() not in drop}


def mod_movie_path(run_dir: Path) -> Path:
    """Where the target movie goes in the disposable RomFS overlay."""
    return run_dir / "mods" / "01-cutscene" / "romfs" / PRODUCT_DIR / OPENING_MOVIE


def make_mod_movie(source: Path, destination: Path) -> str:
    """Hard-link when possible, otherwise copy one movie into the disposable mod."""
    destination.parent.mkdir(parents=True, exist_ok=True)
    try:
        os.link(source, destination)
        return "hardlink"
    except OSError:
        shutil.copy2(source, destination)
        return "copy"


def stop_process(proc: subprocess.Popen[bytes]) -> bool:
    if proc.poll() is not None:
        return False
    proc.terminate()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=10)
    return True


def read_lines(handle, pending: str) -> tuple[list[str], str]:
    chunk = handle.read()
    if not chunk:
        return [], pending
    pending += chunk
    parts = pending.splitlines()
    if pending and not pending.endswith(("\n", "\r")):
        pending = parts.pop() if parts else pending
    else:
        pending = ""
    return parts, pending


def run_one(
    movie: Path,
    index: int,
    total: int,
    args: argparse.Namespace,
    run_root: Path,
    press: str,
    start_timeout: float,
) -> dict[str, object]:
    run_dir = run_root / "runs" / movie.stem
    if run_dir.exists():
        shutil.rmtree(run_dir)
    mod_movie = mod_movie_path(run_dir)
    mod_movie.parent.mkdir(parents=True)
    (run_dir / "save").mkdir()
    log_path = run_dir / "fefw.log"
    out_path = run_dir / "run.out"
    ini_path = run_dir / "fefw.ini"
    log_path.touch()

    link_kind = make_mod_movie(movie, mod_movie)
    base_ini = Path(args.base_ini).resolve()
    try:
        base_text = base_ini.read_text(encoding="utf-8", errors="replace")
    except OSError as error:
        raise SystemExit(f"error: cannot read --base-ini {base_ini}: {error}") from error

    overrides = [
        ("fs", "save_dir", run_dir / "save"),
        ("fs", "mods", run_dir / "mods"),
        ("log", "path", log_path),
        ("log", "level", "info"),
        ("media", "movies", "host"),
        ("media", "movie_decoder", args.decoder),
        ("media", "movie_hwaccel", "1" if args.hwaccel else "0"),
        ("media", "video_decoder_log", "1"),
        ("gfx", "gpu", args.gpu),
        ("boot", "max_seconds", int(args.max_run_seconds)),
    ]
    if press:
        overrides.append(("boot", "press", press))
    if args.exefs:
        overrides.append(("fs", "exefs", Path(args.exefs).resolve()))
    if args.romfs:
        overrides.append(("fs", "romfs", Path(args.romfs).resolve()))
    if args.ffmpeg_dir:
        overrides.append(("media", "ffmpeg_dir", Path(args.ffmpeg_dir).resolve()))
    ini_path.write_text(append_overrides(base_text, overrides), encoding="utf-8", newline="\n")

    command = [
        str(Path(args.engine).resolve()),
        "--ini",
        str(ini_path),
        "--log",
        str(log_path),
        "--max-seconds",
        str(int(args.max_run_seconds)),
        "--window" if args.window else "--headless",
    ]
    if args.exefs:
        command += ["--exefs", str(Path(args.exefs).resolve())]
    if args.romfs:
        command += ["--romfs", str(Path(args.romfs).resolve())]

    print(f"[{index}/{total}] {movie.stem}: starting ({link_kind}, {movie.stat().st_size:,} bytes)", flush=True)
    started_wall = time.monotonic()
    analysis = Analysis()
    timed_out = False
    terminated_by_runner = False
    rc: int | None = None
    tail_deadline: float | None = None

    env = remove_env_overrides(dict(os.environ))
    with out_path.open("wb") as out_handle:
        proc = subprocess.Popen(
            command,
            cwd=str(run_dir),
            stdin=subprocess.DEVNULL,
            stdout=out_handle,
            stderr=subprocess.STDOUT,
            env=env,
        )
        try:
            log_handle = log_path.open("r", encoding="utf-8", errors="replace")
            pending = ""
            while True:
                rc = proc.poll()
                lines, pending = read_lines(log_handle, pending)
                for line in lines:
                    analysis.feed(line)

                now = time.monotonic()
                if analysis.eos and tail_deadline is None:
                    tail_deadline = now + args.tail_seconds
                if tail_deadline is not None and now >= tail_deadline:
                    terminated_by_runner = stop_process(proc)
                    rc = proc.returncode
                    break
                if rc is not None:
                    break
                if now - started_wall >= args.max_run_seconds:
                    timed_out = True
                    terminated_by_runner = stop_process(proc)
                    rc = proc.returncode
                    break
                wait_until = started_wall + start_timeout
                if tail_deadline is not None:
                    wait_until = min(wait_until, tail_deadline)
                if now >= wait_until and not analysis.stats:
                    timed_out = True
                    analysis.errors.append("movie did not start before --start-timeout")
                    terminated_by_runner = stop_process(proc)
                    rc = proc.returncode
                    break
                time.sleep(0.2)

            # Drain whatever the process wrote before it ended.
            lines, pending = read_lines(log_handle, pending)
            for line in lines:
                analysis.feed(line)
            log_handle.close()
        except BaseException:
            stop_process(proc)
            raise

    run_seconds = time.monotonic() - started_wall
    if timed_out and "movie did not start before --start-timeout" not in analysis.errors:
        analysis.errors.append("run hit --max-run-seconds before end of stream")

    row = result_row(movie, args, analysis, run_seconds, rc, terminated_by_runner, ini_path, log_path, run_dir, press)
    status = row["status"]
    reason = row["reason"]
    suffix = f" - {reason}" if reason else ""
    print(
        f"[{index}/{total}] {movie.stem}: {status} in {run_seconds:.1f}s "
        f"({analysis.acquires} acquires, {len(analysis.stats)} stats){suffix}",
        flush=True,
    )

    # The mod file is disposable: a hardlink costs nothing, a copy must not
    # accumulate.  The log and ini stay for evidence.
    try:
        mod_movie.unlink()
    except OSError:
        pass
    return row


def result_row(
    movie: Path,
    args: argparse.Namespace,
    analysis: Analysis,
    run_seconds: float,
    rc: int | None,
    terminated_by_runner: bool,
    ini_path: Path,
    log_path: Path,
    run_dir: Path,
    press: str,
) -> dict[str, object]:
    reasons: list[str] = []
    opened = analysis.movie_player or analysis.started or bool(analysis.stats) or analysis.acquires > 0
    if not opened:
        reasons.append("movie player did not open the cutscene")
    if not analysis.stats:
        reasons.append("no movie stats line")
    if analysis.acquires == 0:
        reasons.append("no AcquireVideoFrame line")
    if not analysis.eos:
        reasons.append("no end-of-stream frame")
    if analysis.empty_frames:
        reasons.append(f"{analysis.empty_frames} empty acquired frame(s)")
    if analysis.errors:
        reasons.extend(analysis.errors)
    late = max([s.late for s in analysis.stats] + [analysis.late_skip_messages])
    if late:
        reasons.append(f"{late} late picture(s) skipped")
    if not analysis.gpu_actual:
        reasons.append("selected GPU not found in the log")
    elif not gpu_matches(args.gpu, analysis.gpu_actual, analysis.gpu_index):
        reasons.append(f"log used GPU {analysis.gpu_actual!r}, not {args.gpu!r}")
    if not analysis.decoder_banner:
        reasons.append("decoder banner not found in the log")
    elif args.decoder == "dav1d" and "dav1d" not in analysis.decoder_banner.casefold():
        reasons.append("requested dav1d but the decoder banner is not dav1d")
    elif args.decoder == "ffmpeg" and "ffmpeg" not in analysis.decoder_banner.casefold():
        reasons.append("requested FFmpeg but the decoder banner is not FFmpeg")
    if args.decoder == "ffmpeg" and args.hwaccel:
        if not analysis.ffmpeg_decoder_line:
            reasons.append("requested FFmpeg hardware decode but its decoder mode was not logged")
        elif not analysis.d3d11va:
            reasons.append("requested FFmpeg hardware decode but D3D11VA was not active")

    unique: list[str] = []
    for reason in reasons:
        if reason not in unique:
            unique.append(reason)
    status = "PASS" if not unique else "FAIL"
    last = analysis.stats[-1] if analysis.stats else None
    return {
        "cutscene": movie.stem,
        "status": status,
        "reason": "; ".join(unique),
        "movie_path": str(movie.resolve()),
        "bytes": movie.stat().st_size,
        "gpu_requested": args.gpu,
        "gpu_actual": analysis.gpu_actual,
        "decoder_requested": args.decoder,
        "hwaccel": 1 if args.hwaccel else 0,
        "press": press,
        "run_seconds": f"{run_seconds:.2f}",
        "exit_code": "" if rc is None else rc,
        "terminated_by_runner": "yes" if terminated_by_runner else "no",
        "movie_started_s": "" if analysis.started_at is None else f"{analysis.started_at:.3f}",
        "movie_end_s": "" if analysis.eos_at is None else f"{analysis.eos_at:.3f}",
        "stats_samples": len(analysis.stats),
        "acquire_lines": analysis.acquires,
        "eos_frame": "yes" if analysis.eos else "no",
        "packets": "" if last is None else last.packets,
        "pictures": "" if last is None else last.pictures,
        "frames_acquired": "" if last is None else last.acquired,
        "released": "" if last is None else last.released,
        "late_skipped": late,
        "decode_ms_per_packet": "" if last is None else f"{last.decode_ms:.3f}",
        "slowest_packet_ms": "" if last is None else f"{last.slowest_ms:.3f}",
        "gap_mean_ms": "" if last is None else f"{last.gap_mean_ms:.3f}",
        "gap_p95_ms": "" if last is None else f"{last.gap_p95_ms:.3f}",
        "gap_p99_ms": "" if last is None else f"{last.gap_p99_ms:.3f}",
        "gap_max_ms": "" if last is None else f"{last.gap_max_ms:.3f}",
        "gaps_over_50ms": "" if last is None else last.over50,
        "log": str(log_path.relative_to(run_dir.parent.parent)),
        "ini": str(ini_path.relative_to(run_dir.parent.parent)),
    }


HEADER = [
    "cutscene",
    "status",
    "reason",
    "movie_path",
    "bytes",
    "gpu_requested",
    "gpu_actual",
    "decoder_requested",
    "hwaccel",
    "press",
    "run_seconds",
    "exit_code",
    "terminated_by_runner",
    "movie_started_s",
    "movie_end_s",
    "stats_samples",
    "acquire_lines",
    "eos_frame",
    "packets",
    "pictures",
    "frames_acquired",
    "released",
    "late_skipped",
    "decode_ms_per_packet",
    "slowest_packet_ms",
    "gap_mean_ms",
    "gap_p95_ms",
    "gap_p99_ms",
    "gap_max_ms",
    "gaps_over_50ms",
    "log",
    "ini",
]


def dry_run_rows(movies: list[Path], args: argparse.Namespace) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    for movie in movies:
        row = {key: "" for key in HEADER}
        row.update(
            {
                "cutscene": movie.stem,
                "status": "SKIP",
                "reason": "dry run: not launched",
                "movie_path": str(movie.resolve()),
                "bytes": movie.stat().st_size,
                "gpu_requested": args.gpu or "",
                "decoder_requested": args.decoder,
                "hwaccel": 1 if args.hwaccel else 0,
                "press": args.press,
            }
        )
        rows.append(row)
    return rows


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--movies-dir", required=True, help="extracted RomFS Movie/Product folder")
    parser.add_argument("--list", action="store_true", help="print the cutscene names and exit")
    parser.add_argument("--dry-run", action="store_true", help="write a SKIP CSV without launching")
    parser.add_argument("--only", help="comma-separated cutscene stems or file names")
    parser.add_argument("--limit", type=int, help="run only the first N selected cutscenes")
    parser.add_argument("--expect-count", type=int, default=EXPECTED_CUTSCENES, help="required .webm count (0 disables)")
    parser.add_argument("--engine", help="fefw.exe to launch")
    parser.add_argument("--base-ini", help="existing fefw.ini with [fs] paths and shader caches")
    parser.add_argument("--exefs", help="override [fs] exefs")
    parser.add_argument("--romfs", help="override [fs] romfs (folder or .nup)")
    parser.add_argument("--ffmpeg-dir", help="override [media] ffmpeg_dir")
    parser.add_argument("--gpu", default="", help="Vulkan adapter name or index; asserted against the log")
    parser.add_argument("--decoder", choices=("dav1d", "ffmpeg"), default="dav1d")
    parser.add_argument("--no-hwaccel", dest="hwaccel", action="store_false", help="FFmpeg software path")
    parser.set_defaults(hwaccel=True)
    display = parser.add_mutually_exclusive_group()
    display.add_argument("--window", action="store_true", help="show the game window")
    display.add_argument("--headless", dest="window", action="store_false", help="run offscreen (default)")
    parser.set_defaults(window=False)
    parser.add_argument("--out", help="run folder (default: _recovery/cutscene-matrix/<timestamp>)")
    parser.add_argument("--press", default="A@12", help="scripted title-screen press (default A@12; empty disables)")
    parser.add_argument("--press-retry", default="A@40", help="later press retried when no movie started (default A@40; empty disables)")
    parser.add_argument("--press-retry-after", type=float, default=45.0, help="seconds to wait for the first press before retrying")
    parser.add_argument("--start-timeout", type=float, default=180.0, help="seconds allowed before the first stats line")
    parser.add_argument("--max-run-seconds", type=float, default=600.0, help="hard cap per cutscene")
    parser.add_argument("--tail-seconds", type=float, default=2.0, help="seconds kept after the end-of-stream frame")
    args = parser.parse_args(argv)

    movies_dir = resolve_movies_dir(Path(args.movies_dir))
    movies = discover_movies(movies_dir, args.expect_count)
    selected = select_movies(movies, args.only, args.limit)
    if args.list:
        for movie in selected:
            print(movie.name)
        return 0

    if args.dry_run:
        out = Path(args.out).resolve() if args.out else (
            Path(__file__).resolve().parents[1] / "_recovery" / "cutscene-matrix" /
            datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
        )
        out.mkdir(parents=True, exist_ok=True)
        csv_path = out / "matrix.csv"
        with csv_path.open("w", encoding="utf-8", newline="") as handle:
            writer = csv.DictWriter(handle, fieldnames=HEADER)
            writer.writeheader()
            writer.writerows(dry_run_rows(selected, args))
        print(f"dry run: {len(selected)} row(s) -> {csv_path}")
        return 0

    missing = [flag for flag in ("engine", "base_ini", "gpu") if not getattr(args, flag)]
    if missing:
        parser.error("the real run needs " + ", ".join("--" + flag.replace("_", "-") for flag in missing))
    if not Path(args.engine).is_file():
        parser.error(f"--engine {args.engine} is not a file")
    if not Path(args.base_ini).is_file():
        parser.error(f"--base-ini {args.base_ini} is not a file")
    gpus = check_gpu(Path(args.engine).resolve(), args.gpu)
    if gpus:
        print(f"GPU preflight: {', '.join(f'{index}: {name}' for index, name in gpus)}", flush=True)
    if args.max_run_seconds <= args.tail_seconds + 1:
        parser.error("--max-run-seconds must leave room after --tail-seconds")
    if args.start_timeout <= 0:
        parser.error("--start-timeout must be positive")
    if args.press_retry and args.press_retry_after <= 0:
        parser.error("--press-retry-after must be positive")

    out = Path(args.out).resolve() if args.out else (
        Path(__file__).resolve().parents[1] / "_recovery" / "cutscene-matrix" /
        datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    )
    if out.exists() and any(out.iterdir()):
        parser.error(f"--out {out} is not empty; choose a new folder")
    out.mkdir(parents=True, exist_ok=True)
    csv_path = out / "matrix.csv"
    manifest_path = out / "run.json"

    manifest = {
        "tool": "cutscene-matrix",
        "created": datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
        "engine": str(Path(args.engine).resolve()),
        "base_ini": str(Path(args.base_ini).resolve()),
        "movies_dir": str(movies_dir),
        "gpu": args.gpu,
        "gpus": [{"index": index, "name": name} for index, name in gpus],
        "decoder": args.decoder,
        "hwaccel": args.hwaccel,
        "headless": not args.window,
        "press": args.press,
        "press_retry": args.press_retry,
        "press_retry_after": args.press_retry_after,
        "max_run_seconds": args.max_run_seconds,
        "start_timeout": args.start_timeout,
        "tail_seconds": args.tail_seconds,
        "expected_cutscenes": args.expect_count,
        "selected": len(selected),
        "csv": str(csv_path),
    }
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")

    rows: list[dict[str, object]] = []
    interrupted = False
    print(f"cutscene matrix: {len(selected)} cutscene(s), GPU {args.gpu!r}, decoder {args.decoder}, "
          f"{'headless' if not args.window else 'window'}", flush=True)
    print(f"output: {out}", flush=True)
    with csv_path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=HEADER)
        writer.writeheader()
        handle.flush()
        try:
            for index, movie in enumerate(selected, 1):
                first_timeout = args.start_timeout
                if args.press_retry and args.press_retry != args.press:
                    first_timeout = min(first_timeout, args.press_retry_after)
                row = run_one(movie, index, len(selected), args, out, args.press, first_timeout)
                if (
                    row["status"] == "FAIL"
                    and row["stats_samples"] == 0
                    and row["acquire_lines"] == 0
                    and args.press_retry
                    and args.press_retry != args.press
                ):
                    print(
                        f"[{index}/{len(selected)}] {movie.stem}: no movie with {args.press!r}; "
                        f"retrying with {args.press_retry!r}",
                        flush=True,
                    )
                    row = run_one(
                        movie, index, len(selected), args, out, args.press_retry, args.start_timeout
                    )
                rows.append(row)
                writer.writerow(row)
                handle.flush()
        except KeyboardInterrupt:
            interrupted = True
            print("\ninterrupted: partial CSV kept", file=sys.stderr)

    passed = sum(row["status"] == "PASS" for row in rows)
    failed = sum(row["status"] == "FAIL" for row in rows)
    print(f"\n{passed} passed, {failed} failed, {len(selected) - len(rows)} not run; {csv_path}")
    for row in rows:
        if row["status"] != "PASS":
            print(f"  FAIL {row['cutscene']}: {row['reason']}")
    return 130 if interrupted else (1 if failed else 0)


if __name__ == "__main__":
    sys.exit(main())
