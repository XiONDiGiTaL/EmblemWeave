# SPDX-License-Identifier: GPL-3.0-or-later
"""The reference half of the end-to-end oracle: run a static AArch64 test ELF under Unicorn.

    python unicorn_side.py <prog.elf>        runs test_main, prints a JSON summary (also used by run_e2e.py)

Layout (identical on the x64 side, see x64_runner.c):
    image            the ELF's own addresses (linked at 0x7100000000)
    STACK            1 MiB at 0x7108000000, SP starts 0x100 below its top
    TLS / TLSRO      64 KiB each at 0x7109000000 / 0x7109100000, filled with a fixed pattern; TPIDR_EL0 / TPIDRRO_EL0 point
                     at their starts
    SENTINEL         0x7107ff0000: the return address (x30) of test_main; reaching it ends the run
Every register (x0..x30, SP, v0..v31, NZCV) and a SHA-256 of every writable region are reported.
"""
import hashlib
import json
import os
import struct
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..'))
from nxrecomp.image import Image          # noqa: E402

STACK, STACK_SIZE = 0x7108000000, 0x100000
TLS, TLSRO, TLS_SIZE = 0x7109000000, 0x7109100000, 0x10000
SENTINEL = 0x7107ff0000
MAX_STEPS = int(os.environ.get('E2E_MAX_STEPS', '60000000'))
M64 = (1 << 64) - 1
HOST_BASE = 0x7110000000      # host services of progs/rt.h (raise / resume), intercepted below


def pattern(n, seed):
    out = bytearray()
    s = seed
    while len(out) < n:
        s = (s * 6364136223846793005 + 1442695040888963407) & M64
        out += struct.pack('<Q', s ^ (s >> 29))
    return bytes(out[:n])


class Unwinder:
    """Unwinds the guest stack from the .eh_frame of the ELF (parsed by nxrecomp/eh.py, independently of the tables the lifter
    emits for the x64 side) and resumes at the landing pad, like rt/eh_rt.cpp does for the lifted code."""

    def __init__(self, img):
        from nxrecomp import eh
        self.eh, self.img = eh, img
        self.rd = eh.Reader(img)
        self.fdes, self.cies = eh.all_fdes(img)
        self.rows, self.lsda = {}, {}
        self.cur_type = 0

    def fde(self, pc):
        for f in self.fdes:
            if self.img.base + f.start <= pc < self.img.base + f.end:
                return f
        return None

    def rows_of(self, f):
        r = self.rows.get(f.start)
        if r is None:
            r = self.rows[f.start] = self.eh.cfi_rows(self.rd, f, self.eh.parse_cie(self.rd, f.cie))
        return r

    def sites_of(self, f):
        r = self.lsda.get(f.start)
        if r is None:
            r = self.lsda[f.start] = self.eh.parse_lsda(self.rd, self.img, f) if f.lsda else ([], {}, [])
        return r

    def step(self, uc, f, st):
        """(cfa, caller state). st: dict pc, sp, x[31], d[8]."""
        pcm1 = st['pc'] - 1 - self.img.base
        row = None
        for r in self.rows_of(f):
            if r[0] <= pcm1:
                row = r
        if row is None:
            raise RuntimeError('no CFI row at %x' % st['pc'])
        cfa = (st['sp'] if row[1] == 31 else st['x'][row[1]]) + row[2]
        n = {'pc': 0, 'sp': cfa, 'x': list(st['x']), 'd': list(st['d'])}
        for reg, off in row[3].items():
            if off is None:
                continue
            v = struct.unpack('<Q', bytes(uc.mem_read(cfa + off, 8)))[0]
            if reg <= 30:
                n['x'][reg] = v
            elif 72 <= reg < 80:
                n['d'][reg - 72] = v
        n['pc'] = n['x'][30]
        return cfa, n

    def pick(self, f, pc, typ):
        sites, actions, types = self.sites_of(f)
        pcm1 = pc - 1 - self.img.base
        for s in sites:
            if not (s.start <= pcm1 < s.start + s.len):
                continue
            if not s.pad:
                return None
            a = s.action
            if not a:
                return self.img.base + s.pad, 0
            while a:
                filt, nxt = actions[a]
                if filt == 0:
                    return self.img.base + s.pad, 0
                if filt > 0:
                    t = types[filt - 1] if filt - 1 < len(types) else None
                    if t == 0 or t == typ:
                        return self.img.base + s.pad, filt
                a = nxt
            return None
        return None

    def host_call(self, uc, A, xr, svc):
        x = [uc.reg_read(r) for r in xr]
        obj, typ = x[0], x[1]
        if svc == 0:
            self.cur_type = typ
        else:
            typ = self.cur_type
        st = {'pc': x[30], 'sp': uc.reg_read(A.UC_ARM64_REG_SP), 'x': x,
              'd': [uc.reg_read(getattr(A, 'UC_ARM64_REG_V%d' % (8 + i))) & M64 for i in range(8)]}
        f = self.fde(st['pc'] - 1)
        if f is None:
            raise RuntimeError('no FDE for the stub')
        _, st = self.step(uc, f, st)
        for _ in range(10000):
            if st['pc'] == 0:
                raise RuntimeError('unwound off the stack')
            f = self.fde(st['pc'] - 1)
            if f is None:
                raise RuntimeError('no FDE at %x' % st['pc'])
            got = self.pick(f, st['pc'], typ)
            cfa, nxt = self.step(uc, f, st)
            if got:
                pad, sel = got
                for i in range(19, 31):
                    uc.reg_write(xr[i], st['x'][i])
                uc.reg_write(A.UC_ARM64_REG_SP, st['sp'])
                for i in range(8):
                    uc.reg_write(getattr(A, 'UC_ARM64_REG_V%d' % (8 + i)), st['d'][i])
                uc.reg_write(xr[0], obj)
                uc.reg_write(xr[1], sel)
                return pad
            st = nxt
        raise RuntimeError('unwind depth exceeded')


def make_uc(model='max'):
    from unicorn import Uc, UC_ARCH_ARM64, UC_MODE_ARM
    from unicorn import arm64_const as A
    cpu = {'max': A.UC_CPU_ARM64_MAX, 'a57': A.UC_CPU_ARM64_A57}[model]
    uc = Uc(UC_ARCH_ARM64, UC_MODE_ARM, cpu=cpu)
    uc.reg_write(A.UC_ARM64_REG_CPACR_EL1, 0x300000)         # FP / SIMD access
    return uc, A


def rw_regions(img):
    """[(name, address, size)] of every writable region of a run."""
    out = []
    for i, (off, size, flags) in enumerate(img.segments):
        if flags & 2:
            out.append(('seg%d' % i, img.base + off, size))
    out += [('stack', STACK, STACK_SIZE), ('tls', TLS, TLS_SIZE), ('tlsro', TLSRO, TLS_SIZE)]
    return out


def initial_memory(img):
    """[(address, bytes)] everything that must be mapped and filled before the run."""
    mem = []
    mem.append((img.base, bytes(img.mem)))
    if img.bss[1]:
        mem.append((img.base + img.bss[0], bytes(img.bss[1])))
    mem.append((STACK, pattern(STACK_SIZE, 1) if False else bytes(STACK_SIZE)))
    mem.append((TLS, pattern(TLS_SIZE, 2)))
    mem.append((TLSRO, pattern(TLS_SIZE, 3)))
    return mem


def run(elf_path, model='max', entry_symbol='test_main', pokes=None, count_addrs=None):
    """pokes: {address: instruction word} written before the run; count_addrs: addresses whose executions are counted
    (res['hits'][address])."""
    img = Image.from_static_elf(elf_path)
    uc, A = make_uc(model)
    page = 0x1000
    end = (img.end + page - 1) // page * page
    uc.mem_map(img.base, end - img.base)
    uc.mem_write(img.base, bytes(img.mem))
    uc.mem_map(STACK, STACK_SIZE)
    uc.mem_map(TLS, TLS_SIZE)
    uc.mem_map(TLSRO, TLS_SIZE)
    uc.mem_write(TLS, pattern(TLS_SIZE, 2))
    uc.mem_write(TLSRO, pattern(TLS_SIZE, 3))
    uc.mem_map(SENTINEL, 0x1000)
    for a, w in (pokes or {}).items():
        uc.mem_write(a, struct.pack('<I', w))
    hits = {a: 0 for a in (count_addrs or [])}
    if hits:
        from unicorn import UC_HOOK_CODE

        def on_count(uc_, address, size, user):
            if address in hits:
                hits[address] += 1
        uc.hook_add(UC_HOOK_CODE, on_count, begin=img.base + img.text_start, end=img.base + img.text_start + img.text_size)
    entry = None
    for name, typ, bind, shndx, value, size in img.symbols():
        if name == entry_symbol:
            entry = value
    if entry is None:
        raise SystemExit('no symbol %s' % entry_symbol)
    for i in range(31):
        uc.reg_write(getattr(A, 'UC_ARM64_REG_X%d' % i), 0)
    uc.reg_write(A.UC_ARM64_REG_SP, STACK + STACK_SIZE - 0x100)
    uc.reg_write(A.UC_ARM64_REG_X30, SENTINEL)
    uc.reg_write(A.UC_ARM64_REG_TPIDR_EL0, TLS)
    uc.reg_write(A.UC_ARM64_REG_TPIDRRO_EL0, TLSRO)
    uc.mem_map(HOST_BASE, 0x1000)
    unw = Unwinder(img) if ('.eh_frame' in img.sections or img.eh_hdr) else None
    state = {'restart': None, 'error': None}

    def on_host(uc_, address, size, user):
        try:
            state['restart'] = unw.host_call(uc_, A, xr_all, (address - HOST_BASE) // 16)
        except Exception as e:               # noqa: BLE001
            state['error'] = str(e)
        uc_.emu_stop()
    xr_all = [getattr(A, 'UC_ARM64_REG_X%d' % i) for i in range(31)]
    if unw:
        from unicorn import UC_HOOK_CODE
        uc.hook_add(UC_HOOK_CODE, on_host, begin=HOST_BASE, end=HOST_BASE + 0xfff)
    pc0 = entry
    for _ in range(100000):
        uc.emu_start(pc0, SENTINEL, count=MAX_STEPS)
        if state['restart'] is not None:
            pc0, state['restart'] = state['restart'], None
            continue
        break
    if state['error']:
        raise RuntimeError('unwinder: ' + state['error'])
    xr = [getattr(A, 'UC_ARM64_REG_X%d' % i) for i in range(31)]
    res = {'x': [uc.reg_read(r) for r in xr], 'sp': uc.reg_read(A.UC_ARM64_REG_SP),
           'v': ['%032x' % uc.reg_read(getattr(A, 'UC_ARM64_REG_V%d' % i)) for i in range(32)],
           'nzcv': (uc.reg_read(A.UC_ARM64_REG_NZCV) >> 28) & 0xf, 'pc': uc.reg_read(A.UC_ARM64_REG_PC)}
    res['finished'] = res['pc'] == SENTINEL
    res['hits'] = hits
    res['mem'] = {}
    for name, addr, size in rw_regions(img):
        res['mem'][name] = hashlib.sha256(bytes(uc.mem_read(addr, size))).hexdigest()
    # the results[] array, if the program has one, for diagnostics
    for name, typ, bind, shndx, value, size in img.symbols():
        if name == 'results' and size:
            res['results'] = [struct.unpack_from('<Q', bytes(uc.mem_read(value, size)), 8 * i)[0] for i in range(size // 8)]
    return res


if __name__ == '__main__':
    r = run(sys.argv[1])
    if 'results' in r:
        r['results'] = ['%x' % x for x in r['results'][:16]] + ['...']
    print(json.dumps(r, indent=1))
