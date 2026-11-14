#!/usr/bin/env python3
"""Generate HIPC (Horizon IPC) test vectors.

The message packing is re-implemented from libnx (switchbrew/libnx, master):
  nx/include/switch/sf/hipc.h   hipcMakeRequest, hipcMakeSendStatic, hipcMakeBuffer,
                                hipcMakeRecvStatic, hipcParseRequest, hipcParseResponse
  nx/include/switch/sf/cmif.h   cmifMakeRequest, cmifMakeControlRequest,
                                cmifMakeCloseRequest, cmifGetAlignedDataStart, cmifParseResponse
  nx/include/switch/sf/tipc.h   tipcMakeRequest (request id + 16)
  nx/include/switch/result.h    MAKERESULT, R_MODULE, R_DESCRIPTION

The bit layouts are cross-checked against the Switchbrew wiki page IPC_Marshalling. The
check functions use explicit bit positions written out by hand, separate from the
libnx-style bitfield declarations used by the writer and the parsers.

libnx master has no nx/include/switch/kernel/ipc.h and no nx/source/sf/cmif_client.c
(both 404). The equivalent code is in sf/hipc.h and sf/cmif.h (header-only).

Run from the worktree root:  python -I tests/tools/gen_hipc_vectors.py
Writes tests/data/hipc_vectors.h and tests/data/hipc_vectors.md (deterministic).
"""

import os

HERE = os.path.dirname(os.path.abspath(__file__))
DATA_DIR = os.path.normpath(os.path.join(HERE, "..", "data"))
HEADER_PATH = os.path.join(DATA_DIR, "hipc_vectors.h")
MD_PATH = os.path.join(DATA_DIR, "hipc_vectors.md")

U32 = 0xFFFFFFFF
ADDR_LIMIT = 1 << 39  # Switchbrew: 39-bit addresses (bits 36-38, 32-35, and 0-31)

CMIF_MAGIC_IN = 0x49434653   # "SFCI", libnx cmif.h CMIF_IN_HEADER_MAGIC
CMIF_MAGIC_OUT = 0x4F434653  # "SFCO", libnx cmif.h CMIF_OUT_HEADER_MAGIC
TYPE_CLOSE = 2               # CmifCommandType_Close
TYPE_REQUEST = 4             # CmifCommandType_Request
TYPE_CONTROL = 5             # CmifCommandType_Control
DOMAIN_SEND_MESSAGE = 1      # CmifDomainRequestType_SendMessage
DOMAIN_CLOSE = 2             # CmifDomainRequestType_Close
TIPC_REQUEST_OFFSET = 16     # libnx tipc.h: fmt.request_id = request_id + 16
ERR_INVALID_CMIF_OUT = "LibnxError_InvalidCmifOutHeader"
NAME_SET_SYS = b"set:sys\x00"


def make_result(module, description):
    """libnx result.h: MAKERESULT(module, description)."""
    return (module & 0x1FF) | ((description & 0x1FFF) << 9)


def r_module(res):
    """libnx result.h: R_MODULE."""
    return res & 0x1FF


def r_description(res):
    """libnx result.h: R_DESCRIPTION."""
    return (res >> 9) & 0x1FFF


# ----------------------------------------------------------------- byte helpers

def le(value, nbytes):
    if not 0 <= value < (1 << (8 * nbytes)):
        raise ValueError("%#x does not fit in %d bytes" % (value, nbytes))
    return value.to_bytes(nbytes, "little")


def u8(buf, off):
    return buf[off]


def u16(buf, off):
    return int.from_bytes(buf[off:off + 2], "little")


def u32(buf, off):
    return int.from_bytes(buf[off:off + 4], "little")


def u64(buf, off):
    return int.from_bytes(buf[off:off + 8], "little")


def words_of(buf):
    if len(buf) % 4:
        raise ValueError("message is not a whole number of 32-bit words")
    return [u32(buf, off) for off in range(0, len(buf), 4)]


def pack_bits(fields):
    """C bitfield packing: (value, width) pairs in declaration order, LSB first."""
    value = 0
    shift = 0
    for field, width in fields:
        if not 0 <= field < (1 << width):
            raise ValueError("bitfield value %#x does not fit in %d bits" % (field, width))
        value |= field << shift
        shift += width
    return value


def unpack_bits(value, fields):
    out = {}
    shift = 0
    for name, width in fields:
        out[name] = (value >> shift) & ((1 << width) - 1)
        shift += width
    return out


# libnx hipc.h bitfield declarations, in declaration order
HIPC_HEADER_FIELDS = [
    ("type", 16), ("num_send_statics", 4), ("num_send_buffers", 4),
    ("num_recv_buffers", 4), ("num_exch_buffers", 4), ("num_data_words", 10),
    ("recv_static_mode", 4), ("padding", 6), ("recv_list_offset", 11),
    ("has_special_header", 1),
]
HIPC_SPECIAL_FIELDS = [("send_pid", 1), ("num_copy_handles", 4), ("num_move_handles", 4),
                       ("padding", 23)]


# ------------------------------------------------------------- libnx-style writer

def _check_addr(addr):
    if not 0 <= addr < ADDR_LIMIT:
        raise ValueError("address %#x is outside the 39-bit HIPC range" % addr)


def _static_desc(addr, size, index):
    """libnx hipcMakeSendStatic: word0 = index:6 | address_high:6 | address_mid:4 | size:16,
    word1 = address_low."""
    _check_addr(addr)
    w0 = pack_bits([(index, 6), ((addr >> 36) & 0x3F, 6), ((addr >> 32) & 0xF, 4), (size, 16)])
    return le(w0, 4) + le(addr & U32, 4)


def _buffer_desc(addr, size, mode):
    """libnx hipcMakeBuffer: size_low, address_low, then
    mode:2 | address_high:22 | size_high:4 | address_mid:4."""
    _check_addr(addr)
    if not 0 <= size < (1 << 36):
        raise ValueError("buffer size %#x does not fit in 36 bits" % size)
    w2 = pack_bits([(mode, 2), ((addr >> 36) & 0x3FFFFF, 22), ((size >> 32) & 0xF, 4),
                    ((addr >> 32) & 0xF, 4)])
    return le(size & U32, 4) + le(addr & U32, 4) + le(w2, 4)


def _recv_entry(addr, size):
    """libnx hipcMakeRecvStatic: address_low, then address_high:16 | size:16."""
    _check_addr(addr)
    w1 = pack_bits([((addr >> 32) & 0xFFFF, 16), (size, 16)])
    return le(addr & U32, 4) + le(w1, 4)


class HipcMessage:
    """One HIPC message, written the way libnx's hipcMakeRequest lays it out.

    The ipc* methods use the names from the older libnx API. In the current libnx the same work is
    done by hipcMakeRequest (header and descriptor layout) and the cmif*/tipc* helpers.
    Message start is 16-byte aligned; raw data starts at the next 16-byte boundary.
    """

    def __init__(self):
        self.type = 0
        self.send_pid = False
        self.pid_value = 0
        self.copy_handles = []
        self.move_handles = []
        self.send_statics = []   # (addr, size, index)
        self.send_buffers = []   # (addr, size, mode)
        self.recv_buffers = []
        self.exch_buffers = []
        self.recv_list = []      # (addr, size)
        self.recv_list_mode = None  # None = libnx rule; 1 = inline (Switchbrew only)
        self.raw = b""
        self.raw_words = 0
        self.raw_aligned = True
        self.layout = None

    def ipcSendPid(self, pid_value=0):
        self.send_pid = True
        self.pid_value = pid_value

    def ipcSendHandleCopy(self, handle):
        self.copy_handles.append(handle)

    def ipcSendHandleMove(self, handle):
        self.move_handles.append(handle)

    def ipcAddSendStatic(self, addr, size, index):
        self.send_statics.append((addr, size, index))

    def ipcAddSendBuffer(self, addr, size, mode=0):
        self.send_buffers.append((addr, size, mode))

    def ipcAddRecvBuffer(self, addr, size, mode=0):
        self.recv_buffers.append((addr, size, mode))

    def ipcAddExchBuffer(self, addr, size, mode=0):
        self.exch_buffers.append((addr, size, mode))

    def ipcAddRecvStatic(self, addr, size):
        self.recv_list.append((addr, size))

    def ipcPrepareHeader(self, msg_type, raw, raw_words, aligned=True):
        self.type = msg_type
        self.raw = raw
        self.raw_words = raw_words
        self.raw_aligned = aligned

    def build(self):
        has_special = self.send_pid or bool(self.copy_handles) or bool(self.move_handles)
        if self.recv_list_mode is not None:
            mode = self.recv_list_mode
        else:
            mode = 0 if not self.recv_list else 2 + len(self.recv_list)
        hdr = pack_bits([
            (self.type, 16),
            (len(self.send_statics), 4), (len(self.send_buffers), 4),
            (len(self.recv_buffers), 4), (len(self.exch_buffers), 4),
            (self.raw_words, 10), (mode, 4), (0, 6), (0, 11), (int(has_special), 1),
        ])
        out = bytearray(le(hdr, 8))
        if has_special:
            out += le(pack_bits([(int(self.send_pid), 1), (len(self.copy_handles), 4),
                                 (len(self.move_handles), 4), (0, 23)]), 4)
        if self.send_pid:
            out += le(self.pid_value, 8)
        for h in self.copy_handles:
            out += le(h, 4)
        for h in self.move_handles:
            out += le(h, 4)
        for addr, size, index in self.send_statics:
            out += _static_desc(addr, size, index)
        for addr, size, mode_ in self.send_buffers:
            out += _buffer_desc(addr, size, mode_)
        for addr, size, mode_ in self.recv_buffers:
            out += _buffer_desc(addr, size, mode_)
        for addr, size, mode_ in self.exch_buffers:
            out += _buffer_desc(addr, size, mode_)

        raw_start = len(out)
        raw_end = raw_start + 4 * self.raw_words
        if self.raw_words == 0:
            if self.raw:
                raise ValueError("raw bytes given with zero raw words")
            aligned = raw_start
        else:
            aligned = ((raw_start + 15) & ~15) if self.raw_aligned else raw_start
            if aligned + len(self.raw) > raw_end:
                raise ValueError("raw content overflows the raw section")
            out += bytes(aligned - raw_start)
            out += self.raw
            out += bytes(raw_end - len(out))
        assert len(out) == raw_end
        for addr, size in self.recv_list:
            out += _recv_entry(addr, size)
        self.layout = {
            "raw_start": raw_start,
            "raw_aligned": aligned,
            "raw_pad_before": aligned - raw_start,
            "raw_end": raw_end,
        }
        return bytes(out)


# ------------------------------------------------------------- libnx cmif helpers

def cmif_in_header(command_id, version=0, token=0):
    return le(CMIF_MAGIC_IN, 4) + le(version, 4) + le(command_id, 4) + le(token, 4)


def cmif_out_header(result, version=0, token=0):
    return le(CMIF_MAGIC_OUT, 4) + le(version, 4) + le(result, 4) + le(token, 4)


def domain_in_header(req_kind, num_in_objects, data_size, object_id, token=0):
    """CmifDomainInHeader: u8 type, u8 num_in_objects, u16 data_size, u32 object_id,
    u32 padding, u32 token."""
    return (le(req_kind, 1) + le(num_in_objects, 1) + le(data_size, 2) + le(object_id, 4)
            + le(0, 4) + le(token, 4))


def domain_out_header(num_out_objects):
    """CmifDomainOutHeader: u32 num_out_objects, u32 padding[3]."""
    return le(num_out_objects, 4) + le(0, 4) * 3


def cmifMakeRequest(command_id, data=b"", object_id=0, objects=(), send_pid=False):
    """libnx cmifMakeRequest with context = 0."""
    if object_id:
        content = (domain_in_header(DOMAIN_SEND_MESSAGE, len(objects), 16 + len(data), object_id, 0)
                   + cmif_in_header(command_id, 0, 0) + data
                   + b"".join(le(o, 4) for o in objects))
        actual = 16 + (16 + 4 * len(objects)) + (16 + len(data))
    else:
        content = cmif_in_header(command_id, 0, 0) + data
        actual = 16 + (16 + len(data))
    actual = (actual + 1) & ~1          # hword align
    raw_words = (actual + 3) // 4       # no OutPointer size table
    msg = HipcMessage()
    if send_pid:
        msg.ipcSendPid()
    msg.ipcPrepareHeader(TYPE_REQUEST, content, raw_words)
    return msg


def cmifMakeControlRequest(command_id, data=b""):
    """libnx cmifMakeControlRequest."""
    actual = 16 + 16 + len(data)
    msg = HipcMessage()
    msg.ipcPrepareHeader(TYPE_CONTROL, cmif_in_header(command_id, 0, 0) + data, (actual + 3) // 4)
    return msg


def cmifMakeCloseRequest(object_id=0):
    """libnx cmifMakeCloseRequest."""
    msg = HipcMessage()
    if object_id:
        msg.ipcPrepareHeader(TYPE_REQUEST,
                             domain_in_header(DOMAIN_CLOSE, 0, 0, object_id, 0), (16 + 16) // 4)
    else:
        msg.ipcPrepareHeader(TYPE_CLOSE, b"", 0)
    return msg


def tipcMakeRequest(request_id, data):
    """libnx tipc.h tipcMakeRequest: type = request_id + 16, raw data straight after the header."""
    msg = HipcMessage()
    msg.ipcPrepareHeader(request_id + TIPC_REQUEST_OFFSET, data, (len(data) + 3) // 4, aligned=False)
    return msg


def response_message(content, move_handles=()):
    """Server reply. Type 0 header. Raw words follow the Switchbrew rule: 0x10 bytes of
    padding in total, so raw = content + 16 bytes."""
    msg = HipcMessage()
    for h in move_handles:
        msg.ipcSendHandleMove(h)
    msg.ipcPrepareHeader(0, content, (16 + len(content) + 3) // 4)
    return msg


# ------------------------------------------------------------- libnx-style parsers

def libnx_parse_request(buf):
    """libnx hipc.h hipcParseRequest, reduced to counts and offsets."""
    hdr = unpack_bits(u64(buf, 0), HIPC_HEADER_FIELDS)
    off = 8
    mode = hdr["recv_static_mode"]
    # libnx maps mode 0 -> none, 2 -> auto, >2 -> mode-2. Mode 1 falls through as 0.
    n_recv = mode - 2 if mode > 2 else 0
    sp = {"send_pid": 0, "num_copy_handles": 0, "num_move_handles": 0}
    pid = None
    if hdr["has_special_header"]:
        sp = unpack_bits(u32(buf, off), HIPC_SPECIAL_FIELDS)
        off += 4
        if sp["send_pid"]:
            pid = u64(buf, off)
            off += 8
    n_copy = sp["num_copy_handles"]
    n_move = sp["num_move_handles"]
    copies = [u32(buf, off + 4 * i) for i in range(n_copy)]
    off += 4 * n_copy
    moves = [u32(buf, off + 4 * i) for i in range(n_move)]
    off += 4 * n_move
    off += 8 * hdr["num_send_statics"]
    off += 12 * (hdr["num_send_buffers"] + hdr["num_recv_buffers"] + hdr["num_exch_buffers"])
    return {
        "type": hdr["type"],
        "num_send_statics": hdr["num_send_statics"],
        "num_send_buffers": hdr["num_send_buffers"],
        "num_recv_buffers": hdr["num_recv_buffers"],
        "num_exch_buffers": hdr["num_exch_buffers"],
        "num_copy_handles": n_copy,
        "num_move_handles": n_move,
        "copies": copies,
        "moves": moves,
        "pid": pid,
        "recv_entries": n_recv,
        "num_data_words": hdr["num_data_words"],
        "data_off": off,
    }


def libnx_parse_response(buf):
    """libnx hipc.h hipcParseResponse."""
    hdr = unpack_bits(u64(buf, 0), HIPC_HEADER_FIELDS)
    off = 8
    n_copy = n_move = 0
    pid = None
    if hdr["has_special_header"]:
        sp = unpack_bits(u32(buf, off), HIPC_SPECIAL_FIELDS)
        off += 4
        n_copy = sp["num_copy_handles"]
        n_move = sp["num_move_handles"]
        if sp["send_pid"]:
            pid = u64(buf, off)
            off += 8
    copies = [u32(buf, off + 4 * i) for i in range(n_copy)]
    off += 4 * n_copy
    moves = [u32(buf, off + 4 * i) for i in range(n_move)]
    off += 4 * n_move
    off += 8 * hdr["num_send_statics"]
    return {"copy": copies, "move": moves, "pid": pid, "data_off": off,
            "num_data_words": hdr["num_data_words"]}


def libnx_cmif_parse_response(buf, is_domain, size):
    """libnx cmif.h cmifParseResponse. Returns ("ok", fields), ("failed", result) or
    ("error", name)."""
    resp = libnx_parse_response(buf)
    start = (resp["data_off"] + 15) & ~15  # cmifGetAlignedDataStart(data_words, base)
    if is_domain:
        n_objects = u32(buf, start)
        out_hdr = start + 16
        objects_off = out_hdr + 16 + size
    else:
        out_hdr = start
    if u32(buf, out_hdr) != CMIF_MAGIC_OUT:
        return ("error", ERR_INVALID_CMIF_OUT)
    result = u32(buf, out_hdr + 8)
    if result != 0:
        return ("failed", result)
    fields = {"data": buf[out_hdr + 16:out_hdr + 16 + size], "copy": resp["copy"],
              "move": resp["move"], "objects": []}
    if is_domain:
        fields["objects"] = [u32(buf, objects_off + 4 * i) for i in range(n_objects)]
    return ("ok", fields)


# --------------------------------------------- Switchbrew wiki bit positions (by hand)

def sb_word0(msg_type, n_ptr, n_send, n_recv, n_exch):
    """Header0Tag: Tag 0-15, PointerCount 16-19, SendCount 20-23, ReceiveCount 24-27,
    ExchangeCount 28-31."""
    for n in (n_ptr, n_send, n_recv, n_exch):
        assert n <= 0xF
    return msg_type | (n_ptr << 16) | (n_send << 20) | (n_recv << 24) | (n_exch << 28)


def sb_word1(raw_count, rl_count, special):
    """Header1Tag: RawCount 0-9, ReceiveListCount 10-13, reserved 14-19, offset 20-30,
    SpecialCount 31."""
    assert raw_count <= 0x3FF and rl_count <= 0xF
    return raw_count | (rl_count << 10) | (special << 31)


def sb_special(pid, n_copy, n_move):
    """SpecialTag: Pid bit 0, CopyHandleCount 1-4, MoveHandleCount 5-8."""
    assert n_copy <= 0xF and n_move <= 0xF
    return int(bool(pid)) | (n_copy << 1) | (n_move << 5)


def sb_pointer(index, addr, size):
    """Pointer0Tag: PointerIndex 0-3, PointerAddress36 6-8, PointerAddress32 12-15,
    PointerSize 16-31. Pointer1Tag: PointerAddress0 (bits 0-31)."""
    assert index <= 0xF and size <= 0xFFFF and addr < ADDR_LIMIT
    w0 = index | (((addr >> 36) & 0x7) << 6) | (((addr >> 32) & 0xF) << 12) | (size << 16)
    return (w0, addr & U32)


def sb_map(addr, size, attr):
    """Map0Tag: MapSizeLow. Map1Tag: MapAddress0. Map2Tag: MapTransferAttribute 0-1,
    MapAddress36 2-4, MapSizeHi 24-27, MapAddress32 28-31."""
    assert size < (1 << 36) and addr < ADDR_LIMIT and attr <= 3
    w2 = attr | (((addr >> 36) & 0x7) << 2) | (((size >> 32) & 0xF) << 24) \
        | (((addr >> 32) & 0xF) << 28)
    return (size & U32, addr & U32, w2)


def sb_recv_list(addr, size):
    """ReceiveList0Tag: address low 0-31. ReceiveList1Tag: address hi 0-6, size 16-31."""
    assert size <= 0xFFFF and addr < ADDR_LIMIT
    return (addr & U32, ((addr >> 32) & 0x7F) | (size << 16))


# --------------------------------------------------------------- vector definitions

class Vec:
    def __init__(self, name, what, msg, exp, is_response):
        self.name = name
        self.what = what
        self.msg = msg
        self.exp = exp
        self.is_response = is_response
        self.buf = None
        self.layout = None


def E(kind, msg_type, payload=b"", method=0, result=0, objects=(), obj_id=0, dom_req=0,
      pid=False, pid_value=0, copy=(), move=(), ptr=(), send=(), recv=(), exch=(),
      rl_mode=0, rl=()):
    """Expected contents of one vector. kind: none | raw | in | out | dom_in | dom_out |
    dom_close. Content length is derived from kind and the payload, not from the writer."""
    payload = bytes(payload)
    objects = list(objects)
    if kind == "none":
        content_len = 0
        rule = "none"
    elif kind == "raw":
        content_len = len(payload)
        rule = "raw"
    elif kind in ("in", "out"):
        content_len = 16 + len(payload)
        rule = "cmif"
    elif kind in ("dom_in", "dom_out"):
        content_len = 16 + 16 + len(payload) + 4 * len(objects)
        rule = "cmif"
    elif kind == "dom_close":
        content_len = 16
        rule = "cmif"
    else:
        raise ValueError(kind)
    return {
        "kind": kind, "rule": rule, "content_len": content_len, "type": msg_type,
        "payload": payload, "method": method, "result": result, "objects": objects,
        "obj_id": obj_id, "dom_req": dom_req, "pid": pid, "pid_value": pid_value,
        "copy": list(copy), "move": list(move), "ptr": list(ptr), "send": list(send),
        "recv": list(recv), "exch": list(exch), "rl_mode": rl_mode, "rl": list(rl),
    }


def raw_words_expected(e):
    if e["rule"] == "none":
        return 0
    if e["rule"] == "raw":
        return (e["content_len"] + 3) // 4
    return (16 + e["content_len"] + 3) // 4  # Switchbrew: total raw padding is 0x10 bytes


def build_vectors():
    out = []

    def add(name, what, msg, exp, is_response=False):
        out.append(Vec(name, what, msg, exp, is_response))

    p_u32 = le(0x11223344, 4) + le(0x55667788, 4)
    add("request_empty",
        "CMIF Request (type 4), command 7, no payload; header only, raw padded to 16 bytes",
        cmifMakeRequest(7),
        E("in", TYPE_REQUEST, method=7))

    add("request_u32_payload",
        "CMIF Request, command 3, payload two u32 values 0x11223344 and 0x55667788",
        cmifMakeRequest(3, data=p_u32),
        E("in", TYPE_REQUEST, method=3, payload=p_u32))

    add("sm_initialize_pid",
        "CMIF Request with send-PID flag (ipcSendPid), sm command 0 (RegisterClient), "
        "8-byte pid placeholder 0",
        cmifMakeRequest(0, data=le(0, 8), send_pid=True),
        E("in", TYPE_REQUEST, method=0, payload=le(0, 8), pid=True, pid_value=0))

    add("sm_get_service",
        "CMIF Request, sm command 1 (GetService) with 8-byte name set:sys",
        cmifMakeRequest(1, data=NAME_SET_SYS),
        E("in", TYPE_REQUEST, method=1, payload=NAME_SET_SYS))

    m = cmifMakeRequest(5)
    m.ipcSendHandleCopy(0x1000A)
    m.ipcSendHandleCopy(0x1000B)
    m.ipcSendHandleMove(0x20001)
    add("copy_move_handles",
        "CMIF Request, command 5, 2 copy handles and 1 move handle (special header counts)",
        m,
        E("in", TYPE_REQUEST, method=5, copy=[0x1000A, 0x1000B], move=[0x20001]))

    m = cmifMakeRequest(9)
    m.ipcAddSendStatic(0x7100123456, 0x234, 1)
    add("pointer_x_desc",
        "CMIF Request, command 9, one static send (X) descriptor, index 1, address above 32 bits",
        m,
        E("in", TYPE_REQUEST, method=9, ptr=[(0x7100123456, 0x234, 1)]))

    m = cmifMakeRequest(11)
    m.ipcAddSendBuffer(0x7123456000, 0x100001000, 0)
    m.ipcAddRecvBuffer(0x7A00000000, 0x10000, 0)
    add("send_a_buffer_high_addr",
        "CMIF Request, command 11, A send buffer of size 0x1_0000_1000 (above 4 GiB) at a high "
        "address, plus B receive buffer",
        m,
        E("in", TYPE_REQUEST, method=11,
          send=[(0x7123456000, 0x100001000, 0)], recv=[(0x7A00000000, 0x10000, 0)]))

    m = cmifMakeRequest(12)
    m.ipcAddExchBuffer(0x7100400000, 0x40, 0)
    add("exchange_w_buffer",
        "CMIF Request, command 12, one W exchange buffer (address and size chosen by the author)",
        m,
        E("in", TYPE_REQUEST, method=12, exch=[(0x7100400000, 0x40, 0)]))

    m = cmifMakeRequest(13)
    m.recv_list_mode = 1
    add("recv_list_inline",
        "CMIF Request, command 13, ReceiveList mode 1 (inline; Switchbrew defines it, libnx "
        "cannot emit it; no entries in the message)",
        m,
        E("in", TYPE_REQUEST, method=13, rl_mode=1))

    recv_multi = [(0x7100300000, 0x100), (0x7100301000, 0x200), (0x7100302000, 0x300)]
    m = cmifMakeRequest(14)
    for addr, size in recv_multi:
        m.ipcAddRecvStatic(addr, size)
    add("recv_list_multi",
        "CMIF Request, command 14, ReceiveList mode 5 (three ReceiveList entries)",
        m,
        E("in", TYPE_REQUEST, method=14, rl_mode=5, rl=recv_multi))

    add("control_convert_domain",
        "CMIF Control (type 5), command 0 ConvertCurrentObjectToDomain, no payload",
        cmifMakeControlRequest(0),
        E("in", TYPE_CONTROL, method=0))

    add("control_query_pointer_buffer",
        "CMIF Control (type 5), command 3 QueryPointerBufferSize, no payload",
        cmifMakeControlRequest(3),
        E("in", TYPE_CONTROL, method=3))

    p8 = bytes(range(1, 9))
    add("domain_request",
        "Domain InvokeMethod on object 5 (CmifDomainInHeader, InRawSize 24), CMIF method 21, "
        "8-byte payload, input object id 9 after the payload",
        cmifMakeRequest(21, data=p8, object_id=5, objects=[9]),
        E("dom_in", TYPE_REQUEST, method=21, payload=p8, objects=[9], obj_id=5, dom_req=1))

    add("domain_close",
        "Domain Close (RequestKind 2) for object 5, type 4",
        cmifMakeCloseRequest(5),
        E("dom_close", TYPE_REQUEST, obj_id=5, dom_req=2))

    add("close_session",
        "CMIF Close (type 2): header words only, no raw data",
        cmifMakeCloseRequest(0),
        E("none", TYPE_CLOSE))

    add("tipc_request",
        "TIPC request to sm: GetServiceHandle, request id 1 + 16 = type 17, raw data is the "
        "name directly after the header words (no CMIF header, no alignment)",
        tipcMakeRequest(1, NAME_SET_SYS),
        E("raw", 17, payload=NAME_SET_SYS))

    p64 = le(0x0123456789ABCDEF, 8)
    add("response_ok_with_move",
        "Response: special header with 1 move handle 0x30002, SFCO result 0, 8-byte payload",
        response_message(cmif_out_header(0) + p64, move_handles=[0x30002]),
        E("out", 0, result=0, payload=p64, move=[0x30002]),
        is_response=True)

    err = make_result(2, 0x1A5)
    add("response_error",
        "Response: no special header, SFCO result MAKERESULT(2, 0x1A5) = 0x00034A02, no payload",
        response_message(cmif_out_header(err)),
        E("out", 0, result=err),
        is_response=True)

    p4 = le(0x2A, 4)
    add("response_domain_out_objects",
        "Domain response: CmifDomainOutHeader with 2 out objects (3, 4) after a 4-byte payload",
        response_message(domain_out_header(2) + cmif_out_header(0) + p4 + le(3, 4) + le(4, 4)),
        E("dom_out", 0, result=0, payload=p4, objects=[3, 4]),
        is_response=True)
    return out


# -------------------------------------------------------------------------- checks

def check_content(buf, pos, e, name):
    """Decode the CMIF/domain content at pos (Switchbrew CmifInHeader, CmifOutHeader,
    CmifDomainMessageInHeader/OutHeader offsets)."""
    start = pos
    kind = e["kind"]
    payload = e["payload"]
    if kind == "raw":
        assert buf[pos:pos + len(payload)] == payload, name + ": raw payload"
        return
    if kind in ("dom_in", "dom_close"):
        assert u8(buf, pos) == e["dom_req"], name + ": RequestKind"
        assert u8(buf, pos + 1) == (len(e["objects"]) if kind == "dom_in" else 0), name + ": InObjectCount"
        assert u16(buf, pos + 2) == (16 + len(payload) if kind == "dom_in" else 0), name + ": InRawSize"
        assert u32(buf, pos + 4) == e["obj_id"], name + ": TargetObjectId"
        assert u32(buf, pos + 8) == 0 and u32(buf, pos + 12) == 0, name + ": domain reserved/token"
        pos += 16
    if kind == "dom_out":
        assert u32(buf, pos) == len(e["objects"]), name + ": OutObjectCount"
        assert u32(buf, pos + 4) == 0 and u32(buf, pos + 8) == 0 and u32(buf, pos + 12) == 0
        pos += 16
    if kind in ("in", "dom_in"):
        assert u32(buf, pos) == CMIF_MAGIC_IN, name + ": SFCI"
        assert u32(buf, pos + 4) == 0, name + ": version"
        assert u32(buf, pos + 8) == e["method"], name + ": MethodId"
        assert u32(buf, pos + 12) == 0, name + ": token"
        pos += 16
    if kind in ("out", "dom_out"):
        assert u32(buf, pos) == CMIF_MAGIC_OUT, name + ": SFCO"
        assert u32(buf, pos + 4) == 0, name + ": out version"
        assert u32(buf, pos + 8) == e["result"], name + ": Result"
        assert u32(buf, pos + 12) == 0, name + ": out token"
        pos += 16
    assert buf[pos:pos + len(payload)] == payload, name + ": payload bytes"
    pos += len(payload)
    for obj in e["objects"]:
        assert u32(buf, pos) == obj, name + ": object id"
        pos += 4
    assert pos - start == e["content_len"], name + ": content length"


def check_switchbrew(v, buf, layout):
    e = v.exp
    name = v.name
    w = words_of(buf)
    special = bool(e["pid"] or e["copy"] or e["move"])
    raw_count = raw_words_expected(e)

    assert w[0] == sb_word0(e["type"], len(e["ptr"]), len(e["send"]), len(e["recv"]),
                            len(e["exch"])), name + ": Header0 rebuilt from fields"
    assert w[1] == sb_word1(raw_count, e["rl_mode"], int(special)), name + ": Header1 rebuilt from fields"

    off = 8
    if special:
        assert w[off // 4] == sb_special(e["pid"], len(e["copy"]), len(e["move"])), name + ": SpecialHeader"
        off += 4
    if e["pid"]:
        assert w[off // 4] == e["pid_value"] & U32, name + ": ProcessId low"
        assert w[off // 4 + 1] == e["pid_value"] >> 32, name + ": ProcessId high"
        off += 8
    for h in e["copy"]:
        assert w[off // 4] == h, name + ": CopyHandle"
        off += 4
    for h in e["move"]:
        assert w[off // 4] == h, name + ": MoveHandle"
        off += 4
    for addr, size, index in e["ptr"]:
        assert tuple(w[off // 4:off // 4 + 2]) == sb_pointer(index, addr, size), name + ": PointerData"
        off += 8
    for addr, size, attr in e["send"]:
        assert tuple(w[off // 4:off // 4 + 3]) == sb_map(addr, size, attr), name + ": SendData"
        off += 12
    for addr, size, attr in e["recv"]:
        assert tuple(w[off // 4:off // 4 + 3]) == sb_map(addr, size, attr), name + ": ReceiveData"
        off += 12
    for addr, size, attr in e["exch"]:
        assert tuple(w[off // 4:off // 4 + 3]) == sb_map(addr, size, attr), name + ": ExchangeData"
        off += 12

    raw_start = off
    if raw_count:
        cmif = e["rule"] == "cmif"
        aligned = ((raw_start + 15) & ~15) if cmif else raw_start
        raw_end = raw_start + 4 * raw_count
        assert aligned - raw_start == layout["raw_pad_before"], name + ": pad before raw data"
        assert aligned == layout["raw_aligned"] and raw_end == layout["raw_end"], name + ": raw layout"
        if cmif:
            assert raw_count * 4 - e["content_len"] == 16, name + ": total raw padding is 0x10"
        check_content(buf, aligned, e, name)
        off = raw_end
    else:
        assert layout["raw_end"] == raw_start, name + ": no raw data"
        off = raw_start
    for addr, size in e["rl"]:
        assert tuple(w[off // 4:off // 4 + 2]) == sb_recv_list(addr, size), name + ": ReceiveListData"
        off += 8
    assert off == len(buf), name + ": total message length"
    return raw_count


def check_libnx_request(v, buf, layout):
    e = v.exp
    name = v.name
    p = libnx_parse_request(buf)
    assert p["type"] == e["type"], name + ": libnx parse type"
    assert p["num_send_statics"] == len(e["ptr"]), name + ": libnx parse statics"
    assert p["num_send_buffers"] == len(e["send"]), name + ": libnx parse send"
    assert p["num_recv_buffers"] == len(e["recv"]), name + ": libnx parse recv"
    assert p["num_exch_buffers"] == len(e["exch"]), name + ": libnx parse exch"
    assert p["num_copy_handles"] == len(e["copy"]), name + ": libnx parse copy"
    assert p["num_move_handles"] == len(e["move"]), name + ": libnx parse move"
    assert p["copies"] == e["copy"] and p["moves"] == e["move"], name + ": libnx handles"
    assert p["pid"] == (e["pid_value"] if e["pid"] else None), name + ": libnx pid"
    assert p["recv_entries"] == len(e["rl"]), name + ": libnx recv entries"
    assert p["num_data_words"] == raw_words_expected(e), name + ": libnx data words"
    assert p["data_off"] == layout["raw_start"], name + ": libnx data offset"


def check_libnx_response(v, buf):
    e = v.exp
    name = v.name
    resp = libnx_parse_response(buf)
    assert resp["copy"] == e["copy"] and resp["move"] == e["move"], name + ": libnx response handles"
    assert resp["data_off"] == v.layout["raw_start"], name + ": libnx response data offset"
    status, val = libnx_cmif_parse_response(buf, e["kind"] == "dom_out", len(e["payload"]))
    if e["result"] != 0:
        assert (status, val) == ("failed", e["result"]), name + ": cmifParseResponse result"
        if name == "response_error":
            assert r_module(val) == 2 and r_description(val) == 0x1A5, name + ": R_MODULE/R_DESCRIPTION"
    else:
        assert status == "ok", name + ": cmifParseResponse status"
        assert val["data"] == e["payload"], name + ": parsed payload"
        assert val["move"] == e["move"], name + ": parsed move handles"
        if e["kind"] == "dom_out":
            assert val["objects"] == e["objects"], name + ": parsed out objects"


def struct_fields(v):
    e = v.exp
    lay = v.layout
    raw_count = raw_words_expected(e)
    if raw_count:
        start = lay["raw_aligned"]
        payload = v.buf[start:start + e["content_len"]]
        pad = lay["raw_pad_before"]
    else:
        payload = b""
        pad = 0
    return {
        "type": e["type"], "has_pid": e["pid"], "pid": e["pid_value"] if e["pid"] else 0,
        "copy": e["copy"], "move": e["move"], "ptr": e["ptr"], "send": e["send"],
        "recv": e["recv"], "exch": e["exch"], "rl_flags": e["rl_mode"], "rl": e["rl"],
        "pad": pad, "raw_words": raw_count, "payload": payload,
    }


# ------------------------------------------------------------------------ rendering

def c_u32(value):
    return "0x%08Xu" % value


def c_u64(value):
    return "0x%016XULL" % value


def render_entry(v):
    s = v.fields
    words = words_of(v.buf)

    def arr(cells, filler, n=4):
        return "{" + ", ".join(cells + [filler] * (n - len(cells))) + "}"

    fields = [
        '"%s"' % v.name,
        "%d" % s["type"],
        "true" if s["has_pid"] else "false",
        c_u64(s["pid"]),
        "%du" % len(s["copy"]), arr([c_u32(x) for x in s["copy"]], "0u"),
        "%du" % len(s["move"]), arr([c_u32(x) for x in s["move"]], "0u"),
        "%du" % len(s["ptr"]),
        arr(["{%s, %s, %s}" % (c_u64(a), c_u32(z), c_u32(i)) for a, z, i in s["ptr"]],
            "{0ULL, 0u, 0u}"),
        "%du" % len(s["send"]),
        arr(["{%s, %s, %s}" % (c_u64(a), c_u64(z), c_u32(t)) for a, z, t in s["send"]],
            "{0ULL, 0ULL, 0u}"),
        "%du" % len(s["recv"]),
        arr(["{%s, %s, %s}" % (c_u64(a), c_u64(z), c_u32(t)) for a, z, t in s["recv"]],
            "{0ULL, 0ULL, 0u}"),
        "%du" % len(s["exch"]),
        arr(["{%s, %s, %s}" % (c_u64(a), c_u64(z), c_u32(t)) for a, z, t in s["exch"]],
            "{0ULL, 0ULL, 0u}"),
        c_u32(s["rl_flags"]),
        "%du" % len(s["rl"]),
        arr(["{%s, %s}" % (c_u64(a), c_u32(z)) for a, z in s["rl"]], "{0ULL, 0u}"),
        c_u32(s["pad"]),
        c_u32(s["raw_words"]),
        ("kHipcPayload_%s" % v.name) if s["payload"] else "nullptr",
        c_u32(len(s["payload"])),
        c_u32(len(words)),
        "kHipcWords_%s" % v.name,
    ]
    return "    {" + ", ".join(fields) + "},"


def render_header(vecs):
    lines = []
    a = lines.append
    a("// Generated by tests/tools/gen_hipc_vectors.py. Do not edit by hand.")
    a("// Independent HIPC (Horizon IPC) test vectors. Packing follows libnx; bit layouts")
    a("// cross-checked against the Switchbrew IPC_Marshalling page.")
    a("#ifndef NXRT_TEST_DATA_HIPC_VECTORS_H")
    a("#define NXRT_TEST_DATA_HIPC_VECTORS_H")
    a("")
    a("#include <cstddef>")
    a("#include <cstdint>")
    a("")
    a("namespace nxrt_test {")
    a("")
    a("struct HipcVec {")
    a("    const char* name;")
    a("    uint16_t type;")
    a("    bool has_pid;")
    a("    uint64_t pid;")
    a("    uint32_t n_copy;")
    a("    uint32_t copy[4];")
    a("    uint32_t n_move;")
    a("    uint32_t move[4];")
    a("    uint32_t n_ptr;")
    a("    struct { uint64_t addr; uint32_t size; uint32_t index; } ptr[4];")
    a("    uint32_t n_send;")
    a("    struct { uint64_t addr; uint64_t size; uint32_t attr; } send[4];")
    a("    uint32_t n_recv;")
    a("    struct { uint64_t addr; uint64_t size; uint32_t attr; } recv[4];")
    a("    uint32_t n_exch;")
    a("    struct { uint64_t addr; uint64_t size; uint32_t attr; } exch[4];")
    a("    uint32_t recv_list_flags;")
    a("    uint32_t n_recv_list;")
    a("    struct { uint64_t addr; uint32_t size; } recv_list[4];")
    a("    uint32_t raw_pad_before_bytes;")
    a("    uint32_t raw_size_words;")
    a("    const uint8_t* payload;")
    a("    uint32_t payload_size;")
    a("    uint32_t n_words;")
    a("    const uint32_t* words;")
    a("};")
    a("")
    for v in vecs:
        a("// %s: %s" % (v.name, v.what))
        if v.fields["payload"]:
            cells = ", ".join("0x%02X" % b for b in v.fields["payload"])
            a("static const uint8_t kHipcPayload_%s[] = {%s};" % (v.name, cells))
        words = words_of(v.buf)
        a("static const uint32_t kHipcWords_%s[] = {" % v.name)
        for i in range(0, len(words), 6):
            a("    " + ", ".join(c_u32(x) for x in words[i:i + 6]) + ",")
        a("};")
        a("")
    a("// One entry per vector, in the order above. Fields follow struct HipcVec.")
    a("[[maybe_unused]] static const HipcVec kHipcVectors[] = {")
    for v in vecs:
        a("    // " + v.name)
        a(render_entry(v))
    a("};")
    a("[[maybe_unused]] static const std::size_t kHipcVectorCount = sizeof(kHipcVectors) / sizeof(kHipcVectors[0]);")
    a("")
    a("}  // namespace nxrt_test")
    a("")
    a("#endif  // NXRT_TEST_DATA_HIPC_VECTORS_H")
    return "\n".join(lines) + "\n"


def render_markdown(vecs):
    lines = [
        "# HIPC test vectors",
        "",
        "Generated by `tests/tools/gen_hipc_vectors.py` (`python -I tests/tools/gen_hipc_vectors.py`). "
        "Do not edit by hand.",
        "",
        "Message start is 16-byte aligned. Header0 and Header1 use the Switchbrew bit positions.",
        "",
        "| # | Name | Exercises | Words | Header0 | Header1 |",
        "|---|---|---|---|---|---|",
    ]
    for i, v in enumerate(vecs, 1):
        w = words_of(v.buf)
        lines.append("| %d | `%s` | %s | %d | `0x%08X` | `0x%08X` |"
                     % (i, v.name, v.what, len(w), w[0], w[1]))
    lines += ["", "## Word lists", ""]
    for i, v in enumerate(vecs, 1):
        w = words_of(v.buf)
        lines.append("### %d. `%s` (%d words)" % (i, v.name, len(w)))
        lines.append("")
        lines.append("```")
        for j in range(0, len(w), 4):
            lines.append("  ".join("[%02d] 0x%08X" % (k, w[k]) for k in range(j, min(j + 4, len(w)))))
        lines.append("```")
        lines.append("")
    return "\n".join(lines) + "\n"


def main():
    vecs = build_vectors()
    for v in vecs:
        v.buf = v.msg.build()
        v.layout = v.msg.layout
        check_switchbrew(v, v.buf, v.layout)
        if v.is_response:
            check_libnx_response(v, v.buf)
        else:
            check_libnx_request(v, v.buf, v.layout)
        v.fields = struct_fields(v)

    header = render_header(vecs)
    md = render_markdown(vecs)
    os.makedirs(DATA_DIR, exist_ok=True)
    with open(HEADER_PATH, "w", encoding="utf-8", newline="\n") as f:
        f.write(header)
    with open(MD_PATH, "w", encoding="utf-8", newline="\n") as f:
        f.write(md)
    print("checked and wrote %d vectors" % len(vecs))
    print(HEADER_PATH)
    print(MD_PATH)


if __name__ == "__main__":
    main()
