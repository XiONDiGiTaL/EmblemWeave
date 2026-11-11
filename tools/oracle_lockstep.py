#!/usr/bin/env python3
"""Lockstep comparison of the EmblemWeave interpreter against Unicorn.

`ew-run --snapshot-at ADDR` writes the guest state the first time the dispatcher
reaches ADDR (build/snapshot/regs.txt and mem.bin) and then the register file
after every executed instruction (trace.txt). This script loads the same state
into Unicorn, an independent AArch64 model (QEMU's), executes one instruction
at a time, and reports the first instruction whose result differs.

Unicorn is a development oracle only; it is never part of the product.

Usage: python tools/oracle_lockstep.py [snapshot-dir]
"""

from __future__ import annotations

import pathlib
import struct
import sys

try:
    from unicorn import Uc, UcError, UC_ARCH_ARM64, UC_MODE_ARM, UC_PROT_ALL
    from unicorn import arm64_const as A
except ImportError:  # pragma: no cover
    sys.exit("the `unicorn` package is required: pip install unicorn")

PAGE = 4096


def xreg(i: int) -> int:
    """Unicorn's enum is not contiguous at x29/x30 (FP/LR)."""
    if i == 29:
        return A.UC_ARM64_REG_X29
    if i == 30:
        return A.UC_ARM64_REG_X30
    return A.UC_ARM64_REG_X0 + i


def parse_regs(line: str):
    f = line.split()
    pc, sp, nzcv = (int(f[i], 16) for i in range(3))
    x = [int(v, 16) for v in f[3:34]]
    v = []
    for item in f[34:66]:
        hi, lo = item.split(":")
        v.append((int(hi, 16) << 64) | int(lo, 16))
    rest = f[66:]
    return pc, sp, nzcv, x, v, rest


def main() -> int:
    snap = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "build/snapshot")
    pc, sp, nzcv, x, v, rest = parse_regs((snap / "regs.txt").read_text())
    text_end = int(rest[0], 16) if rest else 0
    def trace_lines():
        with open(snap / "trace.txt", encoding="ascii") as f:
            for line in f:
                if line.strip():
                    yield parse_regs(line)

    trace = trace_lines()
    total = 0

    uc = Uc(UC_ARCH_ARM64, UC_MODE_ARM)
    # FPEN: allow SIMD/FP at EL0.
    uc.reg_write(A.UC_ARM64_REG_CPACR_EL1, 3 << 20)
    data = (snap / "mem.bin").read_bytes()
    pages = {}
    for off in range(0, len(data), 8 + PAGE):
        addr = struct.unpack_from("<Q", data, off)[0]
        pages[addr] = data[off + 8 : off + 8 + PAGE]

    def map_page(addr: int) -> None:
        base = addr & ~(PAGE - 1)
        try:
            uc.mem_map(base, PAGE, UC_PROT_ALL)
        except UcError:
            return
        if base in pages:
            uc.mem_write(base, pages[base])

    for addr in pages:
        map_page(addr)

    # Pages that were all zero in the snapshot are mapped on first touch.
    def on_unmapped(uc_, access, address, size, value, user):
        map_page(address)
        if (address + size - 1) & ~(PAGE - 1) != address & ~(PAGE - 1):
            map_page(address + size - 1)
        return True

    from unicorn import UC_HOOK_MEM_UNMAPPED

    uc.hook_add(UC_HOOK_MEM_UNMAPPED, on_unmapped)

    for i in range(31):
        uc.reg_write(xreg(i), x[i])
    uc.reg_write(A.UC_ARM64_REG_SP, sp)
    uc.reg_write(A.UC_ARM64_REG_NZCV, nzcv << 28)
    for i in range(32):
        uc.reg_write(A.UC_ARM64_REG_Q0 + i, v[i])

    stubs = {}
    if (snap / "stubs.txt").exists():
        for line in (snap / "stubs.txt").read_text().splitlines():
            a, name = line.split(" ", 1)
            stubs[int(a, 16)] = name

    def host_call(name: str) -> bool:
        """Apply the memory routines the oracle can reproduce exactly."""
        x0, x1, x2 = (uc.reg_read(xreg(i)) for i in range(3))
        if name == "memset":
            if x2:
                uc.mem_write(x0, bytes([x1 & 0xFF]) * x2)
        elif name in ("memcpy", "memmove"):
            if x2:
                uc.mem_write(x0, bytes(uc.mem_read(x1, x2)))
        else:
            return False
        uc.reg_write(A.UC_ARM64_REG_PC, uc.reg_read(xreg(30)))
        return True

    cur = pc
    calls = 0
    for step, (tpc, tsp, tnzcv, tx, tv, _) in enumerate(trace):
        while cur in stubs:
            if not host_call(stubs[cur]):
                print(f"step {step}: host call {stubs[cur]} is not reproduced by the oracle; stopping here")
                return 0
            calls += 1
            cur = uc.reg_read(A.UC_ARM64_REG_PC)
        if tpc != cur:
            print(f"step {step}: control flow differs: oracle at 0x{cur:x}, interpreter at 0x{tpc:x}")
            return 1
        if text_end and cur >= text_end:
            print(f"step {step}: reached 0x{cur:x} outside the text (a host call); stopping the comparison here")
            return 0
        word = struct.unpack("<I", bytes(uc.mem_read(cur, 4)))[0]
        total = step + 1
        try:
            if word & 0xFFFFFBFF == 0xD65F0BFF:
                # retaa/retab: the host models pointer authentication as a
                # plain return; Unicorn would raise without PAuth keys.
                uc.reg_write(A.UC_ARM64_REG_PC, uc.reg_read(xreg(30)))
            else:
                uc.emu_start(cur, cur + 0x1000_0000, count=1)
        except UcError as e:
            print(f"step {step}: oracle fault at 0x{cur:x} (word 0x{word:08x}): {e}")
            return 1
        ox = [uc.reg_read(xreg(i)) for i in range(31)]
        osp = uc.reg_read(A.UC_ARM64_REG_SP)
        onzcv = (uc.reg_read(A.UC_ARM64_REG_NZCV) >> 28) & 0xF
        ov = [uc.reg_read(A.UC_ARM64_REG_Q0 + i) for i in range(32)]
        diffs = []
        for i in range(31):
            if ox[i] != tx[i]:
                diffs.append(f"x{i}: oracle 0x{ox[i]:x}, interpreter 0x{tx[i]:x}")
        if osp != tsp:
            diffs.append(f"sp: oracle 0x{osp:x}, interpreter 0x{tsp:x}")
        if onzcv != tnzcv:
            diffs.append(f"nzcv: oracle {onzcv:04b}, interpreter {tnzcv:04b}")
        for i in range(32):
            if ov[i] != tv[i]:
                diffs.append(f"v{i}: oracle 0x{ov[i]:032x}, interpreter 0x{tv[i]:032x}")
        if diffs:
            print(f"step {step}: first divergence at 0x{cur:x} (word 0x{word:08x})")
            for d in diffs:
                print("  " + d)
            return 1
        cur = uc.reg_read(A.UC_ARM64_REG_PC)
    print(f"OK: {total} instructions identical to the oracle ({calls} host calls reproduced)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
