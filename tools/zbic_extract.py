#!/usr/bin/env python3
"""Decompress ZBIC-compressed NSO0 segments using the public `zbic` package.

This is a development-time helper only: it never ships Nintendo content, it just
lets the analysis pipeline read the user's own dump. NSO segment layout is the
one documented in docs/FEASIBILITY.md (also implemented in Rust in ew-format).

Usage:
    python tools/zbic_extract.py <exefs-dir> <out-dir> [module ...]
"""

from __future__ import annotations

import pathlib
import struct
import sys

try:
    import zbic
except ImportError:  # pragma: no cover
    sys.exit("the `zbic` package is required: pip install zbic")

SEGMENTS = ("text", "rodata", "data")


def parse_header(data: bytes) -> dict:
    if data[0:4] != b"NSO0":
        raise ValueError("not an NSO0 module")
    flags = struct.unpack_from("<I", data, 0x0C)[0]
    segs = []
    for base in (0x10, 0x20, 0x30):
        file_off, mem_off, size = struct.unpack_from("<III", data, base)
        segs.append({"file_offset": file_off, "mem_offset": mem_off, "size": size})
    header = {
        "flags": flags,
        "segments": segs,
        "header_size": struct.unpack_from("<I", data, 0x1C)[0],
        "name_len": struct.unpack_from("<I", data, 0x2C)[0],
        "bss_size": struct.unpack_from("<I", data, 0x3C)[0],
    }
    name_end = segs[0]["file_offset"]
    name_start = name_end - header["name_len"]
    header["name"] = data[name_start:name_end].decode("ascii", "replace").rstrip("\0")
    return header


def segment_bytes(data: bytes, header: dict, index: int) -> bytes:
    start = header["segments"][index]["file_offset"]
    end = (
        header["segments"][index + 1]["file_offset"]
        if index + 1 < len(header["segments"])
        else len(data)
    )
    return data[start:end]


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    exefs = pathlib.Path(sys.argv[1])
    out_root = pathlib.Path(sys.argv[2])
    only = set(sys.argv[3:])

    modules = [
        p
        for p in sorted(exefs.iterdir())
        if p.is_file() and p.suffix != ".npdm" and p.read_bytes()[:4] == b"NSO0"
    ]
    if not modules:
        print(f"no NSO0 modules found in {exefs}")
        return 1

    rc = 0
    for path in modules:
        if only and path.name not in only:
            continue
        data = path.read_bytes()
        header = parse_header(data)
        zbic_used = bool(header["flags"] & 0x80)
        out_dir = out_root / path.name
        out_dir.mkdir(parents=True, exist_ok=True)
        print(
            f"{path.name}: module={header['name']!r} flags=0x{header['flags']:02x} "
            f"zbic={zbic_used} bss=0x{header['bss_size']:x}"
        )
        for index, label in enumerate(SEGMENTS):
            blob = segment_bytes(data, header, index)
            declared = header["segments"][index]["size"]
            compressed = bool(header["flags"] & (1 << index))
            try:
                if not compressed:
                    raw = blob[:declared]
                elif zbic_used:
                    raw = zbic.decompress(blob)
                else:
                    raise RuntimeError("LZ4 segments are handled by the Rust pipeline")
            except Exception as exc:  # noqa: BLE001 - report and continue
                print(f"  {label:<6} FAILED: {exc}")
                rc = 1
                continue
            if len(raw) != declared:
                print(
                    f"  {label:<6} size mismatch: got {len(raw)} declared {declared}"
                )
                rc = 1
            (out_dir / f"{label}.bin").write_bytes(raw)
            print(
                f"  {label:<6} stored={len(blob):>9} decoded={len(raw):>9} "
                f"(declared {declared:>9}){' OK' if len(raw) == declared else ' MISMATCH'}"
            )
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
