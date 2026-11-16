#!/usr/bin/env python3
"""Write the tiny, real `.nup` container the parser tests open.

The fixture is generated once and checked in, but this script is kept beside it
so the bytes are auditable: every table offset and hash below comes from the
layout documented in `src/nup.rs`, not from a player's dump. It contains no
game data, only the words "MAIN" and "GAME".
"""

import hashlib
import struct
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE / "tiny.nup"
MAGIC = 0xCDEFBCDD


def u16(v):
    return struct.pack("<H", v)


def u32(v):
    return struct.pack("<I", v)


def u64(v):
    return struct.pack("<Q", v)


def name_pool_offsets(names):
    pool = bytearray()
    offsets = {}
    for name in names:
        offsets[name] = len(pool)
        pool += name.encode("ascii") + b"\0"
    return bytes(pool), offsets


def main():
    names = ["root", "exefs", "romfs", "main", "data.bin"]
    pool, at = name_pool_offsets(names)
    n_pkg, n_dir, n_file = 1, 3, 2
    toc_size = 0x30 + n_pkg * 0x40 + n_dir * 0x18 + n_file * 0x50 + 4 * 4 + len(pool)
    data_off = (0x80 + toc_size + 0x7F) & ~0x7F

    main_data = b"MAIN\n"
    data_data = b"GAME\n"
    main_off = data_off
    data_file_off = main_off + len(main_data)

    meta = bytearray()
    for tag, value in [(4, b"1.0.0\0"), (5, b"Tiny Fixture\0"), (6, b"EmblemWeave tests\0")]:
        meta += u32(tag) + u32(len(value)) + value
    meta_off = data_file_off + len(data_data)
    total = meta_off + len(meta)

    toc = bytearray()
    toc += b"TOC\x01" + u32(n_pkg) + u64(n_dir) + u64(n_file)
    toc += u64(0x30 + n_pkg * 0x40 + n_dir * 0x18 + n_file * 0x50 + 4 * 4)
    toc += u64(len(pool)) + u64(0)
    assert len(toc) == 0x30, len(toc)

    # Package: title id, root, exefs, romfs, version, kind, flags, metadata.
    toc += u64(0x04007EF01F8AE000) + u32(0) + u32(1) + u32(2) + u32(1)
    toc += bytes([0, 0]) + u16(0) + u32(0) + u64(meta_off) + u32(len(meta)) + b"\0" * 20

    def directory(name, first, count):
        return u32(at[name]) + u32(len(name)) + u32(first) + u32(count) + u64(0)

    toc += directory("root", 0, 2)
    toc += directory("exefs", 2, 1)
    toc += directory("romfs", 3, 1)

    def file_entry(name, data, offset):
        return (
            u32(at[name])
            + u32(len(name))
            + u64(len(data))
            + u64(offset)
            + u64(len(data))
            + hashlib.sha256(data).digest()
            + bytes([0])
            + b"\0" * 15
        )

    toc += file_entry("main", main_data, main_off)
    toc += file_entry("data.bin", data_data, data_file_off)
    toc += u32(1) + u32(2) + u32(n_dir + 0) + u32(n_dir + 1)
    toc += pool
    assert len(toc) == toc_size, (len(toc), toc_size)

    superblock = bytearray(0x80)
    superblock[0:4] = b"NUP\x01"
    superblock[4:6] = u16(0x80)
    superblock[6:8] = u16(1)
    superblock[8:16] = u64(0x80)
    superblock[16:24] = u64(toc_size)
    superblock[24:56] = hashlib.sha256(toc).digest()
    superblock[56:64] = u64(data_off)
    superblock[64:72] = u64(total)
    superblock[72:76] = u32(n_pkg)
    superblock[76:80] = u32(0)
    superblock[80:96] = b"EW-TINY-FIXTURE\0"

    image = bytearray(total)
    image[0:0x80] = superblock
    image[0x80 : 0x80 + len(toc)] = toc
    image[main_off : main_off + len(main_data)] = main_data
    image[data_file_off : data_file_off + len(data_data)] = data_data
    image[meta_off:total] = meta
    assert len(image) == total
    OUT.write_bytes(image)
    print(f"wrote {OUT} ({total} bytes, TocSha={hashlib.sha256(toc).hexdigest()})")


if __name__ == "__main__":
    main()
