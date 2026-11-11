#!/usr/bin/env python3
"""Extract the .text section of a static ELF object as raw bytes.

Used by the verification scripts to turn a compiled AArch64 object into the flat
guest image the lifter consumes. This is deliberately a few lines of struct
unpacking rather than a dependency: the toolchain already produces the object,
and all that is needed is the section bytes.
"""

import struct
import sys


def text_section(path):
    data = open(path, "rb").read()
    if data[:4] != b"\x7fELF" or data[4] != 2:
        raise SystemExit(f"{path}: not a 64-bit ELF object")
    shoff = struct.unpack_from("<Q", data, 0x28)[0]
    shentsize = struct.unpack_from("<H", data, 0x3A)[0]
    shnum = struct.unpack_from("<H", data, 0x3C)[0]
    shstrndx = struct.unpack_from("<H", data, 0x3E)[0]

    def header(i):
        # name, type, flags, addr, offset, size
        return struct.unpack_from("<IIQQQQ", data, shoff + i * shentsize)

    _, _, _, _, names_off, _ = header(shstrndx)
    for i in range(shnum):
        name, _typ, _flags, _addr, offset, size = header(i)
        end = data.index(b"\0", names_off + name)
        if data[names_off + name:end] == b".text":
            return data[offset:offset + size]
    raise SystemExit(f"{path}: no .text section")


def main():
    if len(sys.argv) != 3:
        raise SystemExit("usage: elf_text.py <object.o> <out.bin>")
    section = text_section(sys.argv[1])
    open(sys.argv[2], "wb").write(section)
    print(f"{len(section)} bytes of .text -> {sys.argv[2]}")


if __name__ == "__main__":
    main()
