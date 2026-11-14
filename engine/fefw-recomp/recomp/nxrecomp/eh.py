# SPDX-License-Identifier: GPL-3.0-or-later
"""Unwind data of a module: .eh_frame CIEs and FDEs, the CFA programs run to rows, and the C++ LSDA (call sites,
landing pads, action and type tables).

What the lifter needs from it: the exact start and end of every function (the FDE list), the landing pads (they
become resume entries of the lifted functions, see program.py), and, for the runtime's unwinder, per function the
rows "from pc: CFA = reg + off, register R saved at CFA + off" plus the call-site / action / type tables.

Parsing follows the DWARF 5 and LSB .eh_frame specifications; the C++ LSDA layout is the Itanium C++ ABI's.
The FDE walk through .eh_frame_hdr's table and the record layout are adapted from smo-recomp
tools/extract/functions.py and ssbu-recomp tools/lifter/recomp_eh.py (both GPL-3.0-or-later); the CFA interpreter is
new (checked against llvm-dwarfdump in oracle/e2e).
"""
import struct
from collections import namedtuple

Fde = namedtuple('Fde', 'start end cie lsda insns pers')        # offsets (module-relative); insns = (off, len)
Cie = namedtuple('Cie', 'code_align data_align ra_reg fde_enc lsda_enc pers_enc pers aug insns')

DW_EH_PE_omit = 0xff


class Reader:
    """Reads DWARF primitive types out of img.mem at module-relative offsets."""

    def __init__(self, img):
        self.img = img
        self.m = img.mem

    def u8(self, o): return self.m[o]
    def u16(self, o): return struct.unpack_from('<H', self.m, o)[0]
    def u32(self, o): return struct.unpack_from('<I', self.m, o)[0]
    def s32(self, o): return struct.unpack_from('<i', self.m, o)[0]
    def u64(self, o): return struct.unpack_from('<Q', self.m, o)[0]

    def uleb(self, o):
        r = s = 0
        while True:
            b = self.m[o]
            o += 1
            r |= (b & 0x7f) << s
            s += 7
            if not b & 0x80:
                return r, o

    def sleb(self, o):
        r = s = 0
        while True:
            b = self.m[o]
            o += 1
            r |= (b & 0x7f) << s
            s += 7
            if not b & 0x80:
                if b & 0x40:
                    r -= 1 << s
                return r, o

    def enc(self, o, e):
        """Value of a DW_EH_PE-encoded field at o -> (value as a module-relative offset or plain number, next offset).
        pcrel is applied (relative to the field's own offset); datarel is not used by .eh_frame."""
        if e == DW_EH_PE_omit:
            return 0, o
        f = e & 0x0f
        if f == 0x00 or f == 0x04 or f == 0x0c:
            v, n = struct.unpack_from('<q', self.m, o)[0], 8
        elif f == 0x03:
            v, n = self.u32(o), 4
        elif f == 0x0b:
            v, n = self.s32(o), 4
        elif f == 0x02:
            v, n = self.u16(o), 2
        elif f == 0x0a:
            v, n = struct.unpack_from('<h', self.m, o)[0], 2
        elif f == 0x01:
            v, o2 = self.uleb(o)
            n = o2 - o
        elif f == 0x09:
            v, o2 = self.sleb(o)
            n = o2 - o
        else:
            raise ValueError('pointer encoding %x' % e)
        app = e & 0x70
        if app == 0x10:                              # pcrel: relative to the field's own address
            v += o
        elif app == 0x00:
            if self.img.abs_is_addr and f in (0x00, 0x04, 0x0c) and v >= self.img.base:
                v -= self.img.base                   # an absolute address in a static ELF -> module offset
        else:
            raise ValueError('pointer application %x' % e)
        return v, o + n


def parse_cie(rd, off):
    """The CIE at module offset `off` (the length field)."""
    length = rd.u32(off)
    p = off + 8                                      # length + CIE id
    version = rd.u8(p)
    p += 1
    e = rd.m.index(b'\0', p)
    aug = bytes(rd.m[p:e]).decode()
    p = e + 1
    ca, p = rd.uleb(p)
    da, p = rd.sleb(p)
    if version == 1:
        ra = rd.u8(p)
        p += 1
    else:
        ra, p = rd.uleb(p)
    fde_enc, lsda_enc, pers_enc, pers = 0, DW_EH_PE_omit, DW_EH_PE_omit, 0
    if aug.startswith('z'):
        alen, p = rd.uleb(p)
        aug_end = p + alen
        for ch in aug[1:]:
            if ch == 'R':
                fde_enc = rd.u8(p)
                p += 1
            elif ch == 'L':
                lsda_enc = rd.u8(p)
                p += 1
            elif ch == 'P':
                pers_enc = rd.u8(p)
                p += 1
                pers, p = rd.enc(p, pers_enc & 0x7f)
            elif ch in 'SB':
                pass
        p = aug_end
    end = off + 4 + length
    return Cie(ca, da, ra, fde_enc, lsda_enc, pers_enc, pers, aug, (p, end - p))


def parse_fde(rd, off, cies):
    """The FDE at module offset `off` -> Fde, with its CIE parsed (cached in `cies`)."""
    length = rd.u32(off)
    cie_off = off + 4 - rd.u32(off + 4)
    cie = cies.get(cie_off)
    if cie is None:
        cie = cies[cie_off] = parse_cie(rd, cie_off)
    p = off + 8
    begin, p = rd.enc(p, cie.fde_enc)
    rng, p = rd.enc(p, cie.fde_enc & 0x0f)
    lsda = 0
    if cie.aug.startswith('z'):
        alen, p = rd.uleb(p)
        q = p + alen
        if cie.lsda_enc != DW_EH_PE_omit and alen:
            lsda, _ = rd.enc(p, cie.lsda_enc)
        p = q
    end = off + 4 + length
    return Fde(begin, begin + rng, cie_off, lsda, (p, end - p), cie.pers)


def all_fdes(img):
    """Every FDE of the module: [Fde], sorted by start. Uses .eh_frame_hdr's table (NSO and static ELF), falling back
    to a sequential walk of .eh_frame (static ELF without a header)."""
    rd = Reader(img)
    cies = {}
    out = []
    frame, table = img.eh_frame_hdr()
    if table:
        for loc, fde_off in table:
            f = parse_fde(rd, fde_off, cies)
            if f.start != loc:
                raise ValueError('eh_frame_hdr / FDE disagree at %x: %x vs %x' % (fde_off, f.start, loc))
            out.append(f)
    elif img.eh_frame[1]:
        o, size = img.eh_frame
        end = o + size
        while o < end:
            length = rd.u32(o)
            if length == 0:
                break
            cie_ptr = rd.u32(o + 4)
            if cie_ptr:
                out.append(parse_fde(rd, o, cies))
            o += 4 + length
    out.sort(key=lambda f: f.start)
    return out, cies


# ---------------------------------------------------------------------------------------------------- CFA program
DW_CFA = {
    0x00: 'nop', 0x01: 'set_loc', 0x02: 'advance_loc1', 0x03: 'advance_loc2', 0x04: 'advance_loc4', 0x05: 'offset_extended',
    0x06: 'restore_extended', 0x07: 'undefined', 0x08: 'same_value', 0x09: 'register', 0x0a: 'remember_state',
    0x0b: 'restore_state', 0x0c: 'def_cfa', 0x0d: 'def_cfa_register', 0x0e: 'def_cfa_offset', 0x0f: 'def_cfa_expression',
    0x10: 'expression', 0x11: 'offset_extended_sf', 0x12: 'def_cfa_sf', 0x13: 'def_cfa_offset_sf', 0x14: 'val_offset',
    0x15: 'val_offset_sf', 0x16: 'val_expression', 0x2d: 'negate_ra_state', 0x2e: 'GNU_args_size',
    0x2f: 'GNU_negative_offset_extended'}


def cfi_rows(rd, fde, cie):
    """Run the CIE's initial instructions and the FDE's CFA program. Returns [(pc, cfa_reg, cfa_off, {reg: off})]:
    from absolute offset pc (until the next row) the CFA is x[cfa_reg] + cfa_off and reg is saved at CFA + off.
    Registers saved by anything but a plain offset rule are reported with off = None (not representable)."""
    state = {'cfa_reg': 31, 'cfa_off': 0, 'saved': {}}
    init = None
    stack = []
    rows = []
    loc = fde.start

    def run(o, n, is_cie):
        nonlocal loc, init
        end = o + n
        while o < end:
            op = rd.u8(o)
            o += 1
            hi, lo = op & 0xc0, op & 0x3f
            if hi == 0x40:
                emit_row()
                loc += lo * cie.code_align
            elif hi == 0x80:
                off, o = rd.uleb(o)
                state['saved'][lo] = off * cie.data_align
            elif hi == 0xc0:
                state['saved'].pop(lo, None)
                if init and lo in init['saved']:
                    state['saved'][lo] = init['saved'][lo]
            else:
                name = DW_CFA.get(op)
                if name == 'nop':
                    pass
                elif name == 'set_loc':
                    v, o = rd.enc(o, cie.fde_enc)
                    emit_row()
                    loc = v
                elif name in ('advance_loc1', 'advance_loc2', 'advance_loc4'):
                    sz = {'advance_loc1': 1, 'advance_loc2': 2, 'advance_loc4': 4}[name]
                    v = int.from_bytes(rd.m[o:o + sz], 'little')
                    o += sz
                    emit_row()
                    loc += v * cie.code_align
                elif name == 'offset_extended':
                    r, o = rd.uleb(o)
                    off, o = rd.uleb(o)
                    state['saved'][r] = off * cie.data_align
                elif name == 'offset_extended_sf':
                    r, o = rd.uleb(o)
                    off, o = rd.sleb(o)
                    state['saved'][r] = off * cie.data_align
                elif name == 'GNU_negative_offset_extended':
                    r, o = rd.uleb(o)
                    off, o = rd.uleb(o)
                    state['saved'][r] = -off * cie.data_align
                elif name == 'restore_extended':
                    r, o = rd.uleb(o)
                    state['saved'].pop(r, None)
                    if init and r in init['saved']:
                        state['saved'][r] = init['saved'][r]
                elif name in ('undefined', 'same_value'):
                    r, o = rd.uleb(o)
                    state['saved'].pop(r, None)
                elif name == 'register':
                    r, o = rd.uleb(o)
                    _, o = rd.uleb(o)
                    state['saved'][r] = None
                elif name == 'remember_state':
                    stack.append((state['cfa_reg'], state['cfa_off'], dict(state['saved'])))
                elif name == 'restore_state':
                    state['cfa_reg'], state['cfa_off'], state['saved'] = stack.pop()
                elif name == 'def_cfa':
                    state['cfa_reg'], o = rd.uleb(o)
                    state['cfa_off'], o = rd.uleb(o)
                elif name == 'def_cfa_sf':
                    state['cfa_reg'], o = rd.uleb(o)
                    v, o = rd.sleb(o)
                    state['cfa_off'] = v * cie.data_align
                elif name == 'def_cfa_register':
                    state['cfa_reg'], o = rd.uleb(o)
                elif name == 'def_cfa_offset':
                    state['cfa_off'], o = rd.uleb(o)
                elif name == 'def_cfa_offset_sf':
                    v, o = rd.sleb(o)
                    state['cfa_off'] = v * cie.data_align
                elif name in ('def_cfa_expression', 'val_expression', 'expression'):
                    if name != 'def_cfa_expression':
                        r, o = rd.uleb(o)
                        state['saved'][r] = None
                    n2, o = rd.uleb(o)
                    o += n2
                    if name == 'def_cfa_expression':
                        state['cfa_reg'] = None
                elif name in ('val_offset', 'val_offset_sf'):
                    r, o = rd.uleb(o)
                    _, o = (rd.uleb(o) if name == 'val_offset' else rd.sleb(o))
                    state['saved'][r] = None
                elif name == 'GNU_args_size':
                    _, o = rd.uleb(o)
                elif name == 'negate_ra_state':
                    pass
                else:
                    raise ValueError('unknown CFA opcode %x' % op)

    def emit_row():
        # a CFA offset is 64-bit two's complement: assemblers write `.cfi_def_cfa_offset -16` as the ULEB of 2^64 - 16
        off = state['cfa_off']
        if off >= 1 << 63:
            off -= 1 << 64
        rows.append((loc, state['cfa_reg'], off, dict(state['saved'])))

    run(cie.insns[0], cie.insns[1], True)
    init = {'saved': dict(state['saved'])}
    rows.clear()
    run(fde.insns[0], fde.insns[1], False)
    emit_row()
    # merge rows at the same pc (keep the last), drop a trailing row at the function end
    out = []
    for r in rows:
        if out and out[-1][0] == r[0]:
            out[-1] = r
        else:
            out.append(r)
    return [r for r in out if r[0] < fde.end or len(out) == 1]


# ---------------------------------------------------------------------------------------------------- LSDA
LsdaSite = namedtuple('LsdaSite', 'start len pad action')       # offsets; pad 0 = none; action 0 = cleanup only


def parse_lsda(rd, img, fde):
    """The C++ LSDA of an FDE -> (sites, actions, types)
       sites:   [LsdaSite]  call-site records; start/len are offsets in the module, pad the landing pad offset (0: none)
       actions: [(filter, next)]  the action table; a call site's `action` is 1-based into it (0 = cleanup)
       types:   [typeinfo offset]  1-based by filter: types[filter - 1]; 0 = catch(...) / null entry"""
    o = fde.lsda
    lpstart_enc = rd.u8(o)
    o += 1
    lpstart = fde.start
    if lpstart_enc != DW_EH_PE_omit:
        lpstart, o = rd.enc(o, lpstart_enc)
    ttype_enc = rd.u8(o)
    o += 1
    ttbase = 0
    if ttype_enc != DW_EH_PE_omit:
        off, o = rd.uleb(o)
        ttbase = o + off
    cs_enc = rd.u8(o)
    o += 1
    cs_len, o = rd.uleb(o)
    cs_end = o + cs_len
    cs_fmt = cs_enc & 0x0f
    sites, raw = [], []
    while o < cs_end:
        s, o = rd.enc(o, cs_fmt)
        l, o = rd.enc(o, cs_fmt)
        lp, o = rd.enc(o, cs_fmt)
        act, o = rd.uleb(o)
        raw.append((s, l, lp, act))
    actions_off = cs_end
    # Action table: a call site's `action` is a 1-based BYTE offset into it; each record is (filter sleb, next sleb) and
    # `next` is a displacement from the position of the next field itself (0 ends the chain). Keys keep the 1-based
    # byte offset; the value's `next` is the key of the following record (0 = end).
    actions = {}
    for s_, l_, lp_, act in raw:
        a = act
        while a and a not in actions:
            p = actions_off + a - 1
            filt, p2 = rd.sleb(p)
            nxt, _ = rd.sleb(p2)
            actions[a] = (filt, (p2 + nxt - actions_off) + 1 if nxt else 0)
            a = actions[a][1]
    lp_base = lpstart
    for s_, l_, lp_, act in raw:
        sites.append(LsdaSite(lp_base + s_, l_, (lp_base + lp_) if lp_ else 0, act))
    types = []
    if ttbase:
        maxf = max([f for f, _ in actions.values() if f > 0] or [0])
        sz = {0x00: 8, 0x04: 8, 0x0c: 8, 0x03: 4, 0x0b: 4, 0x02: 2, 0x0a: 2}[ttype_enc & 0x0f]
        for k in range(1, maxf + 1):
            p = ttbase - k * sz
            if sz == 4 and (ttype_enc & 0x0f) == 0x0b:
                v = rd.s32(p)
            elif sz == 4:
                v = rd.u32(p)
            else:
                v = rd.u64(p)
            if v == 0:
                types.append(0)
                continue
            if (ttype_enc & 0x70) == 0x10:
                v += p
            if ttype_enc & 0x80:                                  # indirect: v is the offset of a slot holding the pointer
                v = img.ptr_at(v)
            types.append(v)
    return sites, actions, types


def parse_lsda_ex(rd, img, fde):
    """parse_lsda with the type table resolved to final guest addresses, and the exception specifications.
       -> (sites, actions, types, specs)
       types: [(address, sym)] 1-based by filter like parse_lsda's list. address is the final post-relocation guest address
              (module base added; 0 = catch(...) / null entry); sym >= 0 means the entry is an undefined dynamic symbol
              (an imported typeinfo) of this module: address is 0 and the runtime resolves the symbol's name; -1 otherwise.
       specs: {raw filter (< 0): [type filter, ...]}  the exception specification lists the negative filters point at
              (a list of 1-based type filters, in the order of the LSDA)."""
    sites, actions, _ = parse_lsda(rd, img, fde)
    o = fde.lsda
    lpstart_enc = rd.u8(o)
    o += 1
    if lpstart_enc != DW_EH_PE_omit:
        _, o = rd.enc(o, lpstart_enc)
    ttype_enc = rd.u8(o)
    o += 1
    if ttype_enc == DW_EH_PE_omit:
        return sites, actions, [], {}
    off, o = rd.uleb(o)
    ttbase = o + off
    specs = {}
    for filt, _ in actions.values():
        if filt < 0 and filt not in specs:
            p = ttbase + (-filt) - 1                      # -filter - 1 = byte offset of the list from the table base
            lst = []
            while True:
                k, p = rd.uleb(p)
                if k == 0:
                    break
                lst.append(k)
            specs[filt] = lst
    maxf = max([f for f, _ in actions.values() if f > 0] + [k for l in specs.values() for k in l] + [0])
    sz = {0x00: 8, 0x04: 8, 0x0c: 8, 0x03: 4, 0x0b: 4, 0x02: 2, 0x0a: 2}[ttype_enc & 0x0f]
    rel = img.reloc_map()
    syms = None
    types = []
    for k in range(1, maxf + 1):
        p = ttbase - k * sz
        if sz == 4 and (ttype_enc & 0x0f) == 0x0b:
            v = rd.s32(p)
        elif sz == 4:
            v = rd.u32(p)
        elif sz == 2:
            v = rd.u16(p)
        else:
            v = rd.u64(p)
        if v == 0:
            types.append((0, -1))
            continue
        if (ttype_enc & 0x70) == 0x10:
            v += p
        if ttype_enc & 0x80:                                  # indirect: v is the offset of a slot holding the pointer
            r = rel.get(v)
            if r is None:
                types.append((img.ptr_at(v), -1))             # no relocation: a static image holds the final address
            elif r[0] == 0x403:                               # R_AARCH64_RELATIVE: base + addend
                types.append((img.base + r[2], -1))
            elif r[0] in (0x101, 0x401, 0x402):               # ABS64 / GLOB_DAT / JUMP_SLOT against a symbol
                if syms is None:
                    syms = img.symbols()
                s = syms[r[1]]
                if s[3]:
                    types.append((img.base + img.sym_value(s[4]) + r[2], -1))
                else:
                    types.append((0, r[1]))
            else:
                raise ValueError('typeinfo slot %x has relocation type %x' % (v, r[0]))
        else:
            types.append((img.base + v, -1))
    return sites, actions, types, specs
