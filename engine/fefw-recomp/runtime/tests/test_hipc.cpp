// SPDX-License-Identifier: GPL-3.0-or-later
#include "test_framework.h"

#include <windows.h>

#include <cstdint>
#include <cstring>
#include <random>
#include <string>
#include <vector>

#include "data/hipc_vectors.h"
#include "nxrt/hipc.h"

NXTEST_GROUP(hipc)

namespace {

using nxrt::hipc::BufDesc;
using nxrt::hipc::Message;
using nxrt::hipc::PtrDesc;
using nxrt::hipc::RecvDesc;
using nxrt_test::HipcVec;
using nxtest::to_s;

constexpr uint64_t kMaxAddr39 = (1ull << 39) - 1;
constexpr uint64_t kMaxMap36 = (1ull << 36) - 1;
constexpr std::size_t kPage = 4096;

const HipcVec* find_vec(const char* name) {
    for (std::size_t i = 0; i < nxrt_test::kHipcVectorCount; ++i)
        if (!std::strcmp(nxrt_test::kHipcVectors[i].name, name)) return &nxrt_test::kHipcVectors[i];
    return nullptr;
}

// The Message a vector describes: raw = the vector's payload bytes, no out-pointer sizes.
Message message_from_vec(const HipcVec& v) {
    Message m;
    m.type = v.type;
    m.has_pid = v.has_pid;
    m.pid = v.pid;
    m.copy.assign(v.copy, v.copy + v.n_copy);
    m.move.assign(v.move, v.move + v.n_move);
    for (uint32_t i = 0; i < v.n_ptr; ++i) m.ptrs.push_back(PtrDesc{v.ptr[i].addr, v.ptr[i].size, v.ptr[i].index});
    for (uint32_t i = 0; i < v.n_send; ++i) m.send.push_back(BufDesc{v.send[i].addr, v.send[i].size, v.send[i].attr});
    for (uint32_t i = 0; i < v.n_recv; ++i) m.recv.push_back(BufDesc{v.recv[i].addr, v.recv[i].size, v.recv[i].attr});
    for (uint32_t i = 0; i < v.n_exch; ++i) m.exch.push_back(BufDesc{v.exch[i].addr, v.exch[i].size, v.exch[i].attr});
    m.recv_list_flags = v.recv_list_flags;
    for (uint32_t i = 0; i < v.n_recv_list; ++i)
        m.recv_list.push_back(RecvDesc{v.recv_list[i].addr, v.recv_list[i].size});
    if (v.payload_size) m.raw.assign(v.payload, v.payload + v.payload_size);
    return m;
}

bool same(const PtrDesc& a, const PtrDesc& b) { return a.addr == b.addr && a.size == b.size && a.index == b.index; }
bool same(const BufDesc& a, const BufDesc& b) { return a.addr == b.addr && a.size == b.size && a.attr == b.attr; }
bool same(const RecvDesc& a, const RecvDesc& b) { return a.addr == b.addr && a.size == b.size; }

template <class T>
bool same_list(const std::vector<T>& a, const std::vector<T>& b) {
    if (a.size() != b.size()) return false;
    for (std::size_t i = 0; i < a.size(); ++i)
        if (!same(a[i], b[i])) return false;
    return true;
}

// Empty when every field of m matches the vector v, otherwise the name of the first field that differs.
std::string vec_mismatch(const Message& m, const HipcVec& v) {
    const Message e = message_from_vec(v);
    if (m.type != e.type) return "type";
    if (m.has_pid != e.has_pid) return "has_pid";
    if (m.pid != e.pid) return "pid";
    if (m.copy != e.copy) return "copy handles";
    if (m.move != e.move) return "move handles";
    if (!same_list(m.ptrs, e.ptrs)) return "X descriptors";
    if (!same_list(m.send, e.send)) return "A descriptors";
    if (!same_list(m.recv, e.recv)) return "B descriptors";
    if (!same_list(m.exch, e.exch)) return "W descriptors";
    if (m.recv_list_flags != e.recv_list_flags) return "receive list flags";
    if (!same_list(m.recv_list, e.recv_list)) return "receive list entries";
    if (m.pad_before != v.raw_pad_before_bytes) return "pad_before";
    if (m.raw.size() != v.raw_size_words * 4u - v.raw_pad_before_bytes) return "raw size";
    if (m.raw.size() < v.payload_size) return "raw shorter than payload";
    if (v.payload_size && std::memcmp(m.raw.data(), v.payload, v.payload_size) != 0) return "raw payload bytes";
    if (!m.out_ptr_sizes.empty()) return "out_ptr_sizes not empty";
    return "";
}

// Bytes from message start to the end of the descriptors (what the alignment pad is measured from).
std::size_t expected_desc_end_bytes(const Message& m) {
    std::size_t b = 8;
    if (m.has_pid || !m.copy.empty() || !m.move.empty())
        b += 4 + (m.has_pid ? 8 : 0) + 4 * (m.copy.size() + m.move.size());
    return b + 8 * m.ptrs.size() + 12 * (m.send.size() + m.recv.size() + m.exch.size());
}

// Empty when parse(build(m)) gives back m. The raw section must be m.raw, then the pad, then the u16 sizes, zero-filled
// to a whole word.
std::string roundtrip_mismatch(const Message& m, const Message& p) {
    if (p.type != m.type) return "type";
    if (p.has_pid != m.has_pid) return "has_pid";
    if (p.pid != (m.has_pid ? m.pid : uint64_t{0})) return "pid";
    if (p.copy != m.copy) return "copy";
    if (p.move != m.move) return "move";
    if (!same_list(p.ptrs, m.ptrs)) return "X";
    if (!same_list(p.send, m.send)) return "A";
    if (!same_list(p.recv, m.recv)) return "B";
    if (!same_list(p.exch, m.exch)) return "W";
    if (p.recv_list_flags != m.recv_list_flags) return "recv flags";
    if (!same_list(p.recv_list, m.recv_list)) return "recv list";
    if (!p.out_ptr_sizes.empty()) return "out_ptr_sizes not empty after parse";
    const bool aligned = nxrt::hipc::has_aligned_raw(m.type);
    const std::size_t pad = aligned ? (16 - expected_desc_end_bytes(m) % 16) % 16 : 0;
    if (p.pad_before != pad) return "pad_before " + to_s(p.pad_before) + ", expected " + to_s(pad);
    std::vector<uint8_t> exp = m.raw;
    if (aligned) exp.resize(exp.size() + (16 - pad), 0);
    for (uint16_t s : m.out_ptr_sizes) {
        exp.push_back(static_cast<uint8_t>(s & 0xFF));
        exp.push_back(static_cast<uint8_t>(s >> 8));
    }
    exp.resize((exp.size() + 3) & ~std::size_t{3}, 0);
    if (p.raw != exp) return "raw (payload, pad, out sizes)";
    return "";
}

// Random message within the encoder's documented ranges.
Message random_message(std::mt19937& rng) {
    auto U = [&rng](uint64_t lo, uint64_t hi) { return std::uniform_int_distribution<uint64_t>(lo, hi)(rng); };
    Message m;
    const uint64_t t = U(0, 23);
    m.type = static_cast<uint16_t>(t < 8 ? t : t + 8);  // 0..7 and 16..31
    m.has_pid = U(0, 1) != 0;
    m.pid = m.has_pid ? U(0, ~0ull) : 0;
    for (uint64_t i = U(0, 3); i > 0; --i) m.copy.push_back(static_cast<uint32_t>(U(0, 0xFFFFFFFFull)));
    for (uint64_t i = U(0, 3); i > 0; --i) m.move.push_back(static_cast<uint32_t>(U(0, 0xFFFFFFFFull)));
    for (uint64_t i = U(0, 4); i > 0; --i)
        m.ptrs.push_back(PtrDesc{U(0, kMaxAddr39), static_cast<uint32_t>(U(0, 0xFFFF)), static_cast<uint32_t>(U(0, 15))});
    for (uint64_t i = U(0, 3); i > 0; --i)
        m.send.push_back(BufDesc{U(0, kMaxAddr39), U(0, kMaxMap36), static_cast<uint32_t>(U(0, 3))});
    for (uint64_t i = U(0, 3); i > 0; --i)
        m.recv.push_back(BufDesc{U(0, kMaxAddr39), U(0, kMaxMap36), static_cast<uint32_t>(U(0, 3))});
    for (uint64_t i = U(0, 2); i > 0; --i)
        m.exch.push_back(BufDesc{U(0, kMaxAddr39), U(0, kMaxMap36), static_cast<uint32_t>(U(0, 3))});
    static constexpr uint32_t kFlags[] = {0, 2, 3, 4, 5, 6};
    const uint32_t f = kFlags[U(0, 5)];
    m.recv_list_flags = f;
    const std::size_t n_rl = f == 0 ? 0 : (f == 2 ? 1 : f - 2);
    for (std::size_t i = 0; i < n_rl; ++i)
        m.recv_list.push_back(RecvDesc{U(0, kMaxAddr39), static_cast<uint32_t>(U(0, 0xFFFF))});
    m.raw.resize(static_cast<std::size_t>(4 * U(0, 50)));
    for (auto& b : m.raw) b = static_cast<uint8_t>(U(0, 255));
    if (nxrt::hipc::has_aligned_raw(m.type))
        for (uint64_t i = U(0, 5); i > 0; --i) m.out_ptr_sizes.push_back(static_cast<uint16_t>(U(0, 0xFFFF)));
    return m;
}

// Two committed pages followed by the guard: the second page is PAGE_NOACCESS, so any read or write that runs
// past guard() faults the test process.
class GuardedPages {
public:
    GuardedPages() {
        base_ = static_cast<uint8_t*>(VirtualAlloc(nullptr, 2 * kPage, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE));
        if (!base_) return;
        DWORD old = 0;
        if (!VirtualProtect(base_ + kPage, kPage, PAGE_NOACCESS, &old)) {
            VirtualFree(base_, 0, MEM_RELEASE);
            base_ = nullptr;
        }
    }
    ~GuardedPages() {
        if (base_) VirtualFree(base_, 0, MEM_RELEASE);
    }
    GuardedPages(const GuardedPages&) = delete;
    GuardedPages& operator=(const GuardedPages&) = delete;

    bool ok() const { return base_ != nullptr; }
    // First word address of the guard page.
    uint32_t* guard_words() const { return reinterpret_cast<uint32_t*>(base_ + kPage); }

private:
    uint8_t* base_ = nullptr;
};

void expect_parse_fails(const char* what, const uint32_t* w, std::size_t n) {
    Message m;
    std::string err;
    const bool ok = nxrt::hipc::parse(w, n, m, &err);
    CHECK_MSG(!ok && !err.empty(), std::string("parse accepted malformed input: ") + what + " (err='" + err + "')");
}

void expect_build_rejects(const char* what, const Message& m) {
    std::vector<uint32_t> buf(2048, 0);
    std::string err;
    const std::size_t n = nxrt::hipc::build(m, buf.data(), buf.size(), &err);
    CHECK_MSG(n == 0 && !err.empty() && nxrt::hipc::built_size_words(m) == 0,
              std::string("build accepted an invalid message: ") + what);
}

}  // namespace

NXTEST(vectors_parse_fields_match) {
    for (std::size_t i = 0; i < nxrt_test::kHipcVectorCount; ++i) {
        const HipcVec& v = nxrt_test::kHipcVectors[i];
        Message m;
        std::string err;
        std::string why;
        if (!nxrt::hipc::parse(v.words, v.n_words, m, &err))
            why = "parse failed: " + err;
        else
            why = vec_mismatch(m, v);
        CHECK_MSG(why.empty(), std::string(v.name) + ": " + why);
    }
}

NXTEST(vectors_build_exact_words) {
    for (std::size_t i = 0; i < nxrt_test::kHipcVectorCount; ++i) {
        const HipcVec& v = nxrt_test::kHipcVectors[i];
        const Message m = message_from_vec(v);
        uint32_t buf[32];
        for (auto& w : buf) w = 0xDEADBEEFu;
        std::string err;
        const std::size_t n = nxrt::hipc::build(m, buf, 32, &err);
        std::string why;
        if (n != v.n_words) {
            why = "build returned " + to_s(n) + " words, expected " + to_s(v.n_words);
        } else if (nxrt::hipc::built_size_words(m) != v.n_words) {
            why = "built_size_words disagrees with build";
        } else {
            for (std::size_t k = 0; k < n && why.empty(); ++k)
                if (buf[k] != v.words[k])
                    why = "word " + to_s(k) + " is " + to_s(buf[k]) + ", expected " + to_s(v.words[k]);
            for (std::size_t k = n; k < 32 && why.empty(); ++k)
                if (buf[k] != 0xDEADBEEFu) why = "wrote past word " + to_s(n);
        }
        CHECK_MSG(why.empty(), std::string(v.name) + ": " + why + " [err '" + err + "']");
    }
}

NXTEST(built_size_words_matches_vectors_and_grows_with_padding) {
    for (std::size_t i = 0; i < nxrt_test::kHipcVectorCount; ++i) {
        const HipcVec& v = nxrt_test::kHipcVectors[i];
        CHECK_MSG(nxrt::hipc::built_size_words(message_from_vec(v)) == v.n_words, std::string(v.name));
    }
    // Same 12-byte payload as a CMIF request (12 bytes pad before, 4 after: 7 raw words) and as TIPC (no pad: 3 words).
    Message t;
    t.type = 17;
    t.send.push_back(BufDesc{0x1000, 0x20, 0});
    t.raw.assign(12, 0x5A);
    Message c = t;
    c.type = 4;
    CHECK_EQ(nxrt::hipc::built_size_words(t), std::size_t{8});
    CHECK_EQ(nxrt::hipc::built_size_words(c), std::size_t{12});
}

NXTEST(random_roundtrip_2000_messages) {
    std::mt19937 rng(0x48495043u);  // "HIPC"
    std::vector<uint32_t> buf(4096, 0);
    for (int iter = 0; iter < 2000; ++iter) {
        const Message m = random_message(rng);
        std::string err;
        const std::size_t n = nxrt::hipc::build(m, buf.data(), buf.size(), &err);
        std::string why;
        if (n == 0) {
            why = "build failed: " + err;
        } else if (nxrt::hipc::built_size_words(m) != n) {
            why = "built_size_words disagrees with build";
        } else {
            Message p;
            if (!nxrt::hipc::parse(buf.data(), n, p, &err))
                why = "parse failed: " + err;
            else
                why = roundtrip_mismatch(m, p);
        }
        CHECK_MSG(why.empty(), "iteration " + to_s(iter) + ": " + why);
        if (!why.empty()) break;
    }
}

NXTEST(truncated_input_never_reads_past_end) {
    GuardedPages gp;
    REQUIRE(gp.ok());
    for (std::size_t i = 0; i < nxrt_test::kHipcVectorCount; ++i) {
        const HipcVec& v = nxrt_test::kHipcVectors[i];
        const std::size_t n = v.n_words;
        // The words end exactly where the guard page starts.
        uint32_t* at = gp.guard_words() - n;
        std::memcpy(at, v.words, n * sizeof(uint32_t));
        std::string why;
        for (std::size_t k = 0; k < n && why.empty(); ++k) {
            Message m;
            std::string err;
            if (nxrt::hipc::parse(at, k, m, &err))
                why = "accepted " + to_s(k) + " of " + to_s(n) + " words";
            else if (err.empty())
                why = "no error text for " + to_s(k) + " words";
        }
        Message full;
        std::string err;
        if (why.empty() && !nxrt::hipc::parse(at, n, full, &err)) why = "full message rejected: " + err;
        CHECK_MSG(why.empty(), std::string(v.name) + ": " + why);
    }
}

NXTEST(build_small_buffer_writes_nothing_past_cap) {
    GuardedPages gp;
    REQUIRE(gp.ok());
    const HipcVec* v = find_vec("copy_move_handles");
    REQUIRE(v != nullptr);
    const Message m = message_from_vec(*v);
    const std::size_t n = v->n_words;
    std::string err;
    for (std::size_t cap : {std::size_t{0}, n - 4, n - 1}) {
        uint32_t* out = gp.guard_words() - cap;  // cap words end right before the guard
        for (std::size_t k = 0; k < cap; ++k) out[k] = 0xCCCCCCCCu;
        const std::size_t got = nxrt::hipc::build(m, out, cap, &err);
        bool untouched = true;
        for (std::size_t k = 0; k < cap; ++k)
            if (out[k] != 0xCCCCCCCCu) untouched = false;
        CHECK_MSG(got == 0 && untouched, "build into " + to_s(cap) + "-word buffer returned " + to_s(got));
    }
    uint32_t* out = gp.guard_words() - n;
    CHECK_EQ(nxrt::hipc::build(m, out, n, &err), n);
    CHECK(std::memcmp(out, v->words, n * sizeof(uint32_t)) == 0);
}

NXTEST(parse_truncated_or_bad_fields_fail_with_reason) {
    const uint32_t raw_too_big[] = {0x00000004u, 0x000003FFu};
    expect_parse_fails("raw size 1023 words, none present", raw_too_big, 2);
    const uint32_t pid_cut[] = {0x00000004u, 0x80000008u, 0x00000001u};
    expect_parse_fails("pid announced, not present", pid_cut, 3);
    const uint32_t handles_cut[] = {0x00000004u, 0x80000008u, 0x0000001Eu, 0x00000001u, 0x00000002u};
    expect_parse_fails("15 copy handles announced, 2 present", handles_cut, 5);
    const uint32_t a_cut[] = {0x00100004u, 0x00000008u};
    expect_parse_fails("one A descriptor announced, none present", a_cut, 2);
    const uint32_t pad_short[] = {0x00000004u, 0x00000001u, 0x00000000u};
    expect_parse_fails("aligned raw section smaller than its 8-byte pad", pad_short, 3);
    expect_parse_fails("empty input", nullptr, 0);
    expect_parse_fails("1-word input", pad_short, 1);

    // Receive list flags 5 announce three 2-word entries after the raw data; the message has none.
    const HipcVec* v = find_vec("recv_list_inline");
    REQUIRE(v != nullptr);
    std::vector<uint32_t> w(v->words, v->words + v->n_words);
    w[1] = (w[1] & ~(0xFu << 10)) | (5u << 10);
    expect_parse_fails("receive list flags 5 with no entries", w.data(), w.size());
}

NXTEST(build_rejects_out_of_range_fields) {
    Message base;
    base.type = 4;
    base.raw.assign(16, 0);

    Message m = base;
    m.ptrs.assign(16, PtrDesc{});
    expect_build_rejects("16 X descriptors", m);

    m = base;
    m.copy.assign(16, 1u);
    expect_build_rejects("16 copy handles", m);

    m = base;
    m.move.assign(16, 1u);
    expect_build_rejects("16 move handles", m);

    m = base;
    m.send.assign(16, BufDesc{});
    expect_build_rejects("16 A descriptors", m);

    m = base;
    m.recv.assign(16, BufDesc{});
    expect_build_rejects("16 B descriptors", m);

    m = base;
    m.exch.assign(16, BufDesc{});
    expect_build_rejects("16 W descriptors", m);

    m = base;
    m.ptrs.push_back(PtrDesc{0x1000, 0x10, 16});
    expect_build_rejects("X index 16", m);

    m = base;
    m.ptrs.push_back(PtrDesc{0x1000, 0x10000, 0});
    expect_build_rejects("X size 0x10000", m);

    m = base;
    m.ptrs.push_back(PtrDesc{1ull << 39, 0x10, 0});
    expect_build_rejects("X address 2^39", m);

    m = base;
    m.send.push_back(BufDesc{0x1000, 1ull << 36, 0});
    expect_build_rejects("A size 2^36", m);

    m = base;
    m.send.push_back(BufDesc{1ull << 39, 0x10, 0});
    expect_build_rejects("A address 2^39", m);

    m = base;
    m.recv.push_back(BufDesc{0x1000, 0x10, 4});
    expect_build_rejects("B attribute 4", m);

    m = base;
    m.recv_list_flags = 3;  // needs one entry
    expect_build_rejects("receive list flags 3 with no entries", m);

    m = base;
    m.recv_list_flags = 1;  // inline: no entries
    m.recv_list.push_back(RecvDesc{0x1000, 0x10});
    expect_build_rejects("receive list flags 1 with an entry", m);

    m = base;
    m.recv_list_flags = 16;
    expect_build_rejects("receive list flags 16", m);

    m = base;
    m.raw.assign(4096, 0);
    expect_build_rejects("payload 4096 bytes", m);

    m = base;
    m.raw.assign(4080, 0);  // 8 + 4080 + 8 = 4096 bytes = 1024 words, one over the 10-bit size field
    expect_build_rejects("payload 4080 bytes (raw 1024 words)", m);

    // Accepted boundaries.
    Message edge = base;
    edge.ptrs.assign(15, PtrDesc{0x1000, 0x10, 15});
    std::vector<uint32_t> buf(2048, 0);
    std::string err;
    CHECK(nxrt::hipc::build(edge, buf.data(), buf.size(), &err) > 0);

    Message big;
    big.type = 4;
    big.raw.assign(4076, 0);  // 8 + 4076 + 8 = 4092 bytes = 1023 words, the largest raw size
    CHECK_EQ(nxrt::hipc::built_size_words(big), std::size_t{1025});
    CHECK_EQ(nxrt::hipc::build(big, buf.data(), buf.size(), &err), std::size_t{1025});
}

NXTEST(types_6_and_7_take_aligned_raw_like_4_and_5) {
    CHECK_EQ(static_cast<int>(nxrt::hipc::kInvoke2Method), 6);
    CHECK_EQ(static_cast<int>(nxrt::hipc::kInvoke2ManagerMethod), 7);
    CHECK(nxrt::hipc::has_aligned_raw(6));
    CHECK(nxrt::hipc::has_aligned_raw(7));

    struct Pair {
        const char* vec;
        uint16_t type;
    };
    const Pair pairs[] = {{"request_u32_payload", 6}, {"control_query_pointer_buffer", 7}};
    for (const Pair& pr : pairs) {
        const HipcVec* v = find_vec(pr.vec);
        REQUIRE(v != nullptr);
        Message m = message_from_vec(*v);
        m.type = pr.type;
        uint32_t buf[32];
        std::string err;
        const std::size_t n = nxrt::hipc::build(m, buf, 32, &err);
        std::string why;
        if (n != v->n_words) {
            why = "word count " + to_s(n);
        } else {
            if ((buf[0] & 0xFFFFu) != pr.type || (buf[0] & 0xFFFF0000u) != (v->words[0] & 0xFFFF0000u))
                why = "header0 " + to_s(buf[0]);
            for (std::size_t k = 1; k < n && why.empty(); ++k)
                if (buf[k] != v->words[k]) why = "word " + to_s(k) + " differs from the type-" + to_s(v->type) + " vector";
        }
        if (why.empty()) {
            Message p;
            if (!nxrt::hipc::parse(buf, n, p, &err)) {
                why = "parse failed: " + err;
            } else if (p.type != pr.type || p.pad_before != v->raw_pad_before_bytes ||
                       p.raw.size() != v->raw_size_words * 4u - v->raw_pad_before_bytes ||
                       std::memcmp(p.raw.data(), v->payload, v->payload_size) != 0) {
                why = "parsed raw section does not match the aligned layout";
            }
        }
        CHECK_MSG(why.empty(), std::string(pr.vec) + ": " + why);
    }
}

NXTEST(tipc_and_close_have_no_alignment_padding) {
    Message tipc;
    tipc.type = 17;
    tipc.send.push_back(BufDesc{0x1000, 0x20, 0});  // header 8 + one A descriptor 12 = offset 20: a CMIF raw would pad 12
    tipc.raw.assign(12, 0x5A);
    Message cmif = tipc;
    cmif.type = 4;
    Message close = tipc;
    close.type = 2;
    close.raw.clear();

    CHECK_EQ(nxrt::hipc::built_size_words(tipc), std::size_t{8});
    CHECK_EQ(nxrt::hipc::built_size_words(cmif), std::size_t{12});
    CHECK_EQ(nxrt::hipc::built_size_words(close), std::size_t{5});

    std::vector<uint32_t> buf(16, 0);
    std::string err;
    Message p;
    std::size_t n = nxrt::hipc::build(tipc, buf.data(), buf.size(), &err);
    CHECK(nxrt::hipc::parse(buf.data(), n, p, &err));
    CHECK_EQ(p.pad_before, uint32_t{0});
    CHECK_EQ(p.raw.size(), std::size_t{12});

    n = nxrt::hipc::build(cmif, buf.data(), buf.size(), &err);
    CHECK(nxrt::hipc::parse(buf.data(), n, p, &err));
    CHECK_EQ(p.pad_before, uint32_t{12});
    CHECK_EQ(p.raw.size(), std::size_t{16});  // 28 raw bytes minus the 12-byte pad

    n = nxrt::hipc::build(close, buf.data(), buf.size(), &err);
    CHECK(nxrt::hipc::parse(buf.data(), n, p, &err));
    CHECK_EQ(p.pad_before, uint32_t{0});
    CHECK(p.raw.empty());
}
