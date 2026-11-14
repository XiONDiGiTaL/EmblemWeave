# SPDX-License-Identifier: GPL-3.0-or-later
"""Write an uncompressed copy of a Switch / Switch 2 NSO.

    python tools/nso/nso_unpack.py <exefs>/main <out>/main

The console's ExeFS `main` of some Switch 2 games is ZBIC-compressed (NSO flag bit 7: a zstd variant with its own entropy-table coding).
Neither the lifter (recomp/nxrecomp/nso.py) nor the runtime's NSO loader reads ZBIC, so setup writes a plain copy and both read that.
ZBIC needs the `zbic` package (https://github.com/kinnay/zbic, `pip install zbic`); LZ4 segments need the `lz4` package.

The output is the same NSO with every segment stored uncompressed: the header is kept (flags keep only the hash bits 3-5), the segments
follow the 0x100-byte header back to back, and each segment's stored size equals its memory size. The segment SHA-256 hashes in the
header are checked against the decompressed data when the flags say they are present. Nothing in the input is changed.
"""
import argparse
import hashlib
import os
import struct
import sys

SEG_HDR = (0x10, 0x20, 0x30)        # (file offset, memory offset, size) of text, ro, data
SEG_FILE_SIZE = (0x60, 0x64, 0x68)  # stored size of each segment
SEG_HASH = 0xA0                     # 3 x SHA-256
FLAG_ZBIC = 0x80


def _decompress(blob, size, zbic_flag):
    if zbic_flag:
        try:
            import zbic
        except ImportError:
            sys.exit('nso_unpack: this NSO is ZBIC-compressed; install the zbic package (pip install zbic)')
        out = zbic.decompress(blob)
    else:
        try:
            import lz4.block
        except ImportError:
            sys.exit('nso_unpack: this NSO is LZ4-compressed; install the lz4 package (pip install lz4)')
        out = lz4.block.decompress(blob, uncompressed_size=size)
    if len(out) != size:
        raise ValueError(f'segment decompressed to {len(out)} bytes, the header says {size}')
    return out


def unpack(data):
    if data[:4] != b'NSO0':
        raise ValueError('not an NSO (no NSO0 magic): is the dump decrypted?')
    hdr = bytearray(data[:0x100])
    flags = struct.unpack_from('<I', data, 0x0C)[0]
    segs = []
    for i in range(3):
        foff, moff, size = struct.unpack_from('<III', data, SEG_HDR[i])
        fsize = struct.unpack_from('<I', data, SEG_FILE_SIZE[i])[0]
        blob = data[foff:foff + fsize]
        seg = _decompress(blob, size, flags & FLAG_ZBIC) if flags & (1 << i) else bytes(blob)
        if len(seg) != size:
            raise ValueError(f'segment {i}: {len(seg)} bytes stored, {size} expected')
        if flags & (8 << i):
            want = data[SEG_HASH + 32 * i:SEG_HASH + 32 * (i + 1)]
            if hashlib.sha256(seg).digest() != want:
                raise ValueError(f'segment {i}: SHA-256 does not match the header (a damaged or different dump)')
        segs.append(seg)
    off = 0x100
    for i, seg in enumerate(segs):
        _, moff, size = struct.unpack_from('<III', hdr, SEG_HDR[i])
        struct.pack_into('<III', hdr, SEG_HDR[i], off, moff, size)
        struct.pack_into('<I', hdr, SEG_FILE_SIZE[i], size)
        off += size
    struct.pack_into('<I', hdr, 0x0C, flags & 0x38)
    return bytes(hdr) + b''.join(segs), flags


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('input', help='the NSO to read (for example <dump>/exefs/main)')
    ap.add_argument('output', help='where to write the uncompressed NSO')
    a = ap.parse_args()
    with open(a.input, 'rb') as f:
        data = f.read()
    out, flags = unpack(data)
    os.makedirs(os.path.dirname(os.path.abspath(a.output)), exist_ok=True)
    tmp = a.output + '.tmp'
    with open(tmp, 'wb') as f:
        f.write(out)
    os.replace(tmp, a.output)
    kind = 'ZBIC' if flags & FLAG_ZBIC else ('LZ4' if flags & 7 else 'uncompressed')
    print(f'{a.input}: {kind}, {len(data)} -> {len(out)} bytes, sha256 {hashlib.sha256(out).hexdigest()}')


if __name__ == '__main__':
    main()
