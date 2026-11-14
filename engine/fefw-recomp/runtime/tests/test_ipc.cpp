// SPDX-License-Identifier: GPL-3.0-or-later
// HIPC/CMIF service framework: sessions, sm:, stub services, domains, TIPC, control messages and the SVC entry points.
#include "test_framework.h"

#include <malloc.h>

#include <algorithm>
#include <atomic>
#include <cstdint>
#include <cstring>
#include <memory>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

#include "data/hipc_vectors.h"
#include "nxrt/hipc.h"
#include "nxrt/ipc.h"
#include "nxrt/kernel.h"
#include "nxrt/svc.h"

NXTEST_GROUP(ipc)

namespace {

using nxrt::hipc::BufDesc;
using nxrt::hipc::Message;
using nxrt::ipc::Request;
using nxrt::ipc::Response;
using nxrt::ipc::Service;
using nxrt::ipc::StubService;
using nxrt_test::HipcVec;

constexpr uint32_t kSfci = 0x49434653u;
constexpr uint32_t kSfco = 0x4F434653u;
constexpr uint32_t kStub = nxrt::kernel::kStubService;
constexpr uint32_t kMsgWords = 0x100 / 4;
constexpr uint64_t kAddr39 = 1ull << 39;

uint32_t g_copy_handle = 0;   // handed back as a copy handle by test:svc command 7 when non-zero

// A 16-byte aligned 0x1000-byte message buffer, like the TLS region.
struct Buf {
    uint32_t* p;
    Buf() : p(static_cast<uint32_t*>(_aligned_malloc(0x1000, 16))) { std::memset(p, 0, 0x1000); }
    ~Buf() { _aligned_free(p); }
    Buf(const Buf&) = delete;
    Buf& operator=(const Buf&) = delete;
    uint64_t addr() const { return reinterpret_cast<uint64_t>(p); }
};

template <class T>
void put(std::vector<uint8_t>& v, T x) {
    const size_t o = v.size();
    v.resize(o + sizeof(T));
    std::memcpy(v.data() + o, &x, sizeof(T));
}

template <class... Ts>
std::vector<uint8_t> pack(Ts... xs) {
    std::vector<uint8_t> v;
    (put(v, xs), ...);
    return v;
}

// CmifInHeader + payload.
std::vector<uint8_t> raw_sfci(uint32_t method, const std::vector<uint8_t>& payload = {}) {
    std::vector<uint8_t> v = pack(kSfci, uint16_t{0}, uint16_t{0}, method, uint32_t{0});
    v.insert(v.end(), payload.begin(), payload.end());
    return v;
}

// A CMIF request (message type 4) for `method`.
Message cmif(uint32_t method, const std::vector<uint8_t>& payload = {}) {
    Message m;
    m.type = nxrt::hipc::kInvoke2MethodOld;
    m.raw = raw_sfci(method, payload);
    return m;
}

// A domain request (type 4): CmifDomainMessageInHeader, then the SFCI request for an invoke, then the object ids.
Message domain_msg(uint32_t target, uint8_t kind, uint32_t method, const std::vector<uint8_t>& payload,
                   const std::vector<uint32_t>& objs = {}) {
    Message m;
    m.type = nxrt::hipc::kInvoke2MethodOld;
    const std::vector<uint8_t> sf = kind == 1 ? raw_sfci(method, payload) : std::vector<uint8_t>{};
    m.raw = pack(kind, static_cast<uint8_t>(objs.size()), static_cast<uint16_t>(sf.size()), target, uint32_t{0}, uint32_t{0});
    m.raw.insert(m.raw.end(), sf.begin(), sf.end());
    for (uint32_t id : objs) put(m.raw, id);
    return m;
}

// Writes the message into `b` without sending it.
bool build_into(Buf& b, const Message& m) {
    std::string err;
    if (nxrt::hipc::build(m, b.p, kMsgWords, &err) != 0) return true;
    CHECK_MSG(false, "hipc::build failed: " + err);
    return false;
}

// Writes the request into `b`, sends it on `h` (0x100 bytes) and returns the SVC result. The reply then sits in `b`.
uint32_t send_msg(Buf& b, uint32_t h, const Message& m) {
    if (!build_into(b, m)) return 0xFFFFFFFFu;
    return nxrt::ipc::send_sync_request(nullptr, h, b.addr(), 0x100);
}

struct Reply {
    bool ok = false;
    Message msg;
};

Reply reply_of(const Buf& b) {
    Reply r;
    std::string err;
    r.ok = nxrt::hipc::parse(b.p, kMsgWords, r.msg, &err);
    if (!r.ok) std::fprintf(stderr, "    reply does not parse: %s\n", err.c_str());
    return r;
}

// A little-endian u32 at `off`, or a marker value when the data is shorter.
uint32_t at32(const std::vector<uint8_t>& v, size_t off) {
    uint32_t x = 0xDEADDEADu;
    if (off + 4 <= v.size()) std::memcpy(&x, v.data() + off, 4);
    return x;
}

// CMIF result of a non-domain reply (SFCO at +0) or of a domain reply (SFCO at +16).
uint32_t result_of(const Reply& r, size_t sfco_at = 0) { return at32(r.msg.raw, sfco_at + 8); }

bool contains(const std::vector<std::string>& v, const std::string& s) { return std::find(v.begin(), v.end(), s) != v.end(); }

uint32_t connect(const char* name) {
    uint32_t h = 0;
    CHECK_EQ(nxrt::ipc::connect_named_port(name, &h), 0u);
    return h;
}

// sm GetServiceHandle through the sm session: the move handle of the new session (0 on failure).
uint32_t get_service(uint32_t sm, const char* name) {
    char n[8] = {};
    std::strncpy(n, name, 8);
    Buf b;
    CHECK_EQ(send_msg(b, sm, cmif(1, std::vector<uint8_t>(n, n + 8))), 0u);
    Reply r = reply_of(b);
    CHECK_EQ(result_of(r), 0u);
    return r.msg.move.empty() ? 0 : r.msg.move[0];
}

// Converts the session `h` into a domain; returns the object id of its service (1).
uint32_t convert(uint32_t h) {
    Buf b;
    Message m;
    m.type = nxrt::hipc::kInvoke2ManagerMethod;
    m.raw = raw_sfci(0);
    CHECK_EQ(send_msg(b, h, m), 0u);
    Reply r = reply_of(b);
    CHECK_EQ(result_of(r), 0u);
    return at32(r.msg.raw, 16);
}

// The domain tests attach an object to a domain session directly (KSession::dom is public).
std::shared_ptr<nxrt::ipc::DomainState> domain_of(uint32_t h) {
    auto s = nxrt::kernel::handle_as<nxrt::ipc::KSession>(h);
    return s ? s->dom : nullptr;
}

// ---- test services ------------------------------------------------------------------------------------------------------

std::shared_ptr<Service> make_obj_service() {
    auto s = std::make_shared<Service>("test:obj");
    s->add_cmd(1, "value", [](Request&, Response& o) -> uint32_t {
        o.push<uint32_t>(0xCAFEu);
        return 0;
    });
    return s;
}

std::shared_ptr<Service> make_test_service() {
    auto s = std::make_shared<Service>("test:svc");
    // 5 add(u32 a, u32 b) -> u32 sum, u64 constant
    s->add_cmd(5, "add", [](Request& r, Response& o) -> uint32_t {
        o.push<uint32_t>(r.arg<uint32_t>(0) + r.arg<uint32_t>(4));
        o.push<uint64_t>(0x0123456789ABCDEFull);
        return 0;
    });
    // 6 upper-cases the A buffer into the B buffer
    s->add_cmd(6, "upper", [](Request& r, Response&) -> uint32_t {
        const nxrt::ipc::Buffer in = r.in_buf(0);
        const nxrt::ipc::Buffer out = r.out_buf(0);
        if (!in.addr || !out.addr) return nxrt::kernel::kInvalidAddress;
        const uint64_t n = std::min(in.size, out.size);
        const char* src = reinterpret_cast<const char*>(in.addr);
        char* dst = reinterpret_cast<char*>(out.addr);
        for (uint64_t i = 0; i < n; ++i) dst[i] = (src[i] >= 'a' && src[i] <= 'z') ? static_cast<char>(src[i] - 32) : src[i];
        return 0;
    });
    // 7 returns a new object (test:obj) and, when set, a copy handle
    s->add_cmd(7, "make_obj", [](Request&, Response& o) -> uint32_t {
        o.push_object(make_obj_service());
        if (g_copy_handle) o.out_copy.push_back(g_copy_handle);
        return 0;
    });
    // 8 fails with 0x2A02 after writing output (which must be dropped)
    s->add_cmd(8, "fail", [](Request&, Response& o) -> uint32_t {
        o.push<uint32_t>(0xDEADu);
        return 0x2A02;
    });
    // 10 reports how many interfaces it got moved in, and whether the first is a session
    s->add_cmd(10, "in_services", [](Request& r, Response& o) -> uint32_t {
        o.push<uint32_t>(static_cast<uint32_t>(r.in_services.size()));
        o.push<uint32_t>(!r.in_services.empty() && r.in_services[0] ? 1u : 0u);
        return 0;
    });
    // 11 throws
    s->add_cmd(11, "throws", [](Request&, Response&) -> uint32_t { throw std::runtime_error("boom"); });
    return s;
}

std::shared_ptr<Service> make_big_service() {
    auto s = std::make_shared<Service>("test:big");
    s->add_cmd(1, "big", [](Request&, Response& o) -> uint32_t {
        o.out.assign(300, 0x5A);   // does not fit a 0x100-byte message
        return 0;
    });
    return s;
}

void register_test_services() {
    nxrt::ipc::register_service("test:svc", [] { return make_test_service(); });
    nxrt::ipc::register_service("test:obj", [] { return make_obj_service(); });
    nxrt::ipc::register_service("test:big", [] { return make_big_service(); });
}

// Every case starts from an empty registry and handle table, with the test services registered.
void fresh() {
    nxrt::ipc::registry_reset();
    nxrt::kernel::handles_reset();
    g_copy_handle = 0;
    register_test_services();
}

}  // namespace

NXTEST(sm_register_client_replies_ok) {
    fresh();
    const uint32_t sm = connect("sm:");
    Buf b;
    Message m = cmif(0);
    m.has_pid = true;
    m.pid = 0x1234;
    CHECK_EQ(send_msg(b, sm, m), 0u);
    Reply r = reply_of(b);
    REQUIRE(r.ok);
    CHECK_EQ(r.msg.type, 0);
    CHECK_EQ(at32(r.msg.raw, 0), kSfco);
    CHECK_EQ(result_of(r), 0u);
}

NXTEST(sm_unregistered_service_is_a_stub) {
    fresh();
    const uint32_t sm = connect("sm:");
    const uint32_t h = get_service(sm, "set:sys");
    CHECK(h != 0);
    CHECK_MSG(contains(nxrt::ipc::stub_names_requested(), "set:sys"), "set:sys not recorded as a requested stub");
    Buf b;
    CHECK_EQ(send_msg(b, h, cmif(3)), 0u);
    Reply r = reply_of(b);
    CHECK_EQ(result_of(r), kStub);
    bool found = false;
    for (const auto& st : nxrt::ipc::service_stats()) {
        if (st.name != "set:sys") continue;
        found = st.stub && st.requests == 1 && st.unhandled.count(3) && st.unhandled.at(3) == 1;
    }
    CHECK_MSG(found, "service_stats does not count one unhandled command 3 for set:sys");
}

NXTEST(registered_service_add_returns_sum_and_constant) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    Buf b;
    CHECK_EQ(send_msg(b, h, cmif(5, pack(uint32_t{40}, uint32_t{2}))), 0u);
    Reply r = reply_of(b);
    REQUIRE(r.ok);
    CHECK_EQ(result_of(r), 0u);
    CHECK_EQ(at32(r.msg.raw, 16), 42u);
    CHECK_EQ(at32(r.msg.raw, 20), 0x89ABCDEFu);
    CHECK_EQ(at32(r.msg.raw, 24), 0x01234567u);
}

NXTEST(buffers_round_trip_through_a_and_b) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    std::vector<char> src = {'h', 'e', 'l', 'l', 'o', ',', ' ', 'n', 'x', 'r', 't'};
    std::vector<char> dst(src.size(), 0);
    CHECK_MSG(reinterpret_cast<uint64_t>(src.data()) < kAddr39 && reinterpret_cast<uint64_t>(dst.data()) < kAddr39,
              "host buffers above 39 bits cannot be described in a message");
    Message m = cmif(6);
    m.send.push_back(BufDesc{reinterpret_cast<uint64_t>(src.data()), src.size(), 0});
    m.recv.push_back(BufDesc{reinterpret_cast<uint64_t>(dst.data()), dst.size(), 0});
    Buf b;
    CHECK_EQ(send_msg(b, h, m), 0u);
    Reply r = reply_of(b);
    CHECK_EQ(result_of(r), 0u);
    CHECK(std::string(dst.begin(), dst.end()) == "HELLO, NXRT");
}

NXTEST(returned_object_is_a_session_and_copy_handle_comes_back) {
    fresh();
    const uint32_t ev = nxrt::kernel::handle_add(std::make_shared<nxrt::kernel::KEvent>());
    g_copy_handle = ev;
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    Buf b;
    CHECK_EQ(send_msg(b, h, cmif(7)), 0u);
    Reply r = reply_of(b);
    REQUIRE(r.ok);
    CHECK_EQ(result_of(r), 0u);
    REQUIRE(r.msg.move.size() == 1);
    REQUIRE(r.msg.copy.size() == 1);
    CHECK_EQ(r.msg.copy[0], ev);
    Buf b2;
    CHECK_EQ(send_msg(b2, r.msg.move[0], cmif(1)), 0u);
    Reply r2 = reply_of(b2);
    CHECK_EQ(result_of(r2), 0u);
    CHECK_EQ(at32(r2.msg.raw, 16), 0xCAFEu);
}

NXTEST(error_result_is_delivered_and_output_dropped) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    Buf b;
    CHECK_EQ(send_msg(b, h, cmif(8)), 0u);
    Reply r = reply_of(b);
    REQUIRE(r.ok);
    CHECK_EQ(result_of(r), 0x2A02u);
    CHECK_MSG(at32(r.msg.raw, 16) == 0 || r.msg.raw.size() < 20, "output of a failed command reached the reply");
    CHECK(r.msg.move.empty());
}

NXTEST(unknown_command_of_real_service_is_stub_result) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    Buf b;
    CHECK_EQ(send_msg(b, h, cmif(99)), 0u);
    CHECK_EQ(result_of(reply_of(b)), kStub);
    bool found = false;
    for (const auto& st : nxrt::ipc::service_stats())
        if (st.name == "test:svc") found = st.unhandled.count(99) && st.unhandled.at(99) == 1;
    CHECK_MSG(found, "unhandled command 99 not counted for test:svc");
}

NXTEST(convert_to_domain_then_domain_invoke) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    CHECK_EQ(convert(h), 1u);
    Buf b;
    CHECK_EQ(send_msg(b, h, domain_msg(1, 1, 5, pack(uint32_t{40}, uint32_t{2}))), 0u);
    Reply r = reply_of(b);
    REQUIRE(r.ok);
    CHECK_EQ(at32(r.msg.raw, 0), 0u);           // OutObjectCount
    CHECK_EQ(result_of(r, 16), 0u);
    CHECK_EQ(at32(r.msg.raw, 32), 42u);         // payload after the SFCO header
}

NXTEST(domain_returns_object_ids_and_close_removes_them) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    REQUIRE(convert(h) == 1);
    Buf b;
    CHECK_EQ(send_msg(b, h, domain_msg(1, 1, 7, {})), 0u);
    Reply r = reply_of(b);
    REQUIRE(r.ok);
    CHECK_EQ(result_of(r, 16), 0u);
    CHECK_EQ(at32(r.msg.raw, 0), 1u);           // one out object
    CHECK_EQ(at32(r.msg.raw, 32), 2u);          // its id, after the (empty) payload
    CHECK(r.msg.move.empty());

    Buf b2;
    CHECK_EQ(send_msg(b2, h, domain_msg(2, 1, 1, {})), 0u);
    Reply r2 = reply_of(b2);
    CHECK_EQ(result_of(r2, 16), 0u);
    CHECK_EQ(at32(r2.msg.raw, 32), 0xCAFEu);

    Buf b3;
    CHECK_EQ(send_msg(b3, h, domain_msg(2, 2, 0, {})), 0u);   // close object 2
    Reply r3 = reply_of(b3);
    REQUIRE(r3.ok);
    CHECK_EQ(at32(r3.msg.raw, 0), 0u);
    CHECK_EQ(result_of(r3, 16), 0u);

    Buf b4;
    CHECK_EQ(send_msg(b4, h, domain_msg(2, 1, 1, {})), 0u);
    CHECK_EQ(result_of(reply_of(b4), 16), 0xF601u);
}

NXTEST(clone_current_object_gives_second_working_session) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    Buf b;
    Message m;
    m.type = nxrt::hipc::kInvoke2ManagerMethod;
    m.raw = raw_sfci(2);
    CHECK_EQ(send_msg(b, h, m), 0u);
    Reply r = reply_of(b);
    REQUIRE(r.ok);
    CHECK_EQ(result_of(r), 0u);
    REQUIRE(r.msg.move.size() == 1);
    Buf b2;
    CHECK_EQ(send_msg(b2, r.msg.move[0], cmif(5, pack(uint32_t{40}, uint32_t{2}))), 0u);
    CHECK_EQ(at32(reply_of(b2).msg.raw, 16), 42u);
}

NXTEST(query_pointer_buffer_size_answers_0x8000) {
    fresh();
    Buf b;
    Message m;
    m.type = nxrt::hipc::kInvoke2ManagerMethod;
    m.raw = raw_sfci(3);
    CHECK_EQ(send_msg(b, connect("sm:"), m), 0u);
    Reply r = reply_of(b);
    CHECK_EQ(result_of(r), 0u);
    CHECK_EQ(at32(r.msg.raw, 16) & 0xFFFFu, 0x8000u);
}

NXTEST(copy_from_current_domain_gives_session_to_object) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    REQUIRE(convert(h) == 1);
    Buf b;
    Message m;
    m.type = nxrt::hipc::kInvoke2ManagerMethod;
    m.raw = raw_sfci(1, pack(uint32_t{1}));
    CHECK_EQ(send_msg(b, h, m), 0u);
    Reply r = reply_of(b);
    CHECK_EQ(result_of(r), 0u);
    REQUIRE(r.msg.move.size() == 1);
    Buf b2;
    CHECK_EQ(send_msg(b2, r.msg.move[0], cmif(5, pack(uint32_t{1}, uint32_t{1}))), 0u);
    CHECK_EQ(at32(reply_of(b2).msg.raw, 16), 2u);
}

NXTEST(tipc_get_service_handle_and_tipc_invoke) {
    fresh();
    const uint32_t sm = connect("sm:");
    // TIPC: type 17 = command 1; the raw data is the 8-byte name, no SFCI header, no padding.
    Message m;
    m.type = nxrt::hipc::kTipcBase + 1;
    m.raw = {'t', 'e', 's', 't', ':', 's', 'v', 'c'};
    Buf b;
    CHECK_EQ(send_msg(b, sm, m), 0u);
    // The reply: header0 type 0, special header with one move handle, then the result word directly (TIPC is unpadded).
    CHECK_EQ(b.p[0] & 0xFFFFu, 0u);
    CHECK_EQ(b.p[1] >> 31, 1u);
    CHECK_EQ((b.p[2] >> 5) & 0xF, 1u);
    const uint32_t h = b.p[3];
    CHECK(h != 0);
    CHECK_EQ(b.p[4], 0u);

    // TIPC command 5 (type 21) on that session: raw = two u32; the reply carries the result, then the sum, unpadded.
    Message m2;
    m2.type = nxrt::hipc::kTipcBase + 5;
    m2.raw = pack(uint32_t{3}, uint32_t{4});
    Buf b2;
    CHECK_EQ(send_msg(b2, h, m2), 0u);
    CHECK_EQ(b2.p[2], 0u);
    CHECK_EQ(b2.p[3], 7u);
}

NXTEST(release_keeps_the_handle_until_closed) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    Buf b;
    Message m;
    m.type = nxrt::hipc::kRelease;
    CHECK_EQ(send_msg(b, h, m), 0u);
    // The client closes the handle itself afterwards (svcCloseHandle): it must still be there.
    CHECK(nxrt::kernel::handle_as<nxrt::ipc::KSession>(h) != nullptr);
    nxrt::kernel::handle_close(h);
    Buf b2;
    CHECK_EQ(send_msg(b2, h, cmif(5, pack(uint32_t{1}, uint32_t{2}))), nxrt::kernel::kInvalidHandle);
}

NXTEST(invalid_session_handle_returns_e401) {
    fresh();
    Buf b;
    CHECK_EQ(nxrt::ipc::send_sync_request(nullptr, 0x7FFFFFF0u, b.addr(), 0x100), nxrt::kernel::kInvalidHandle);
}

NXTEST(malformed_messages_return_e801_without_crashing) {
    fresh();
    const uint32_t sm = connect("sm:");
    Buf b;
    // shorter than the two header words
    b.p[0] = 4;
    CHECK_EQ(nxrt::ipc::send_sync_request(nullptr, sm, b.addr(), 4), 0xE801u);
    // header claims more raw words than the message size holds
    CHECK(build_into(b, cmif(0)));
    CHECK_EQ(nxrt::ipc::send_sync_request(nullptr, sm, b.addr(), 0x10), 0xE801u);
    // an all-ones buffer: special header, counts and raw size out of range
    std::memset(b.p, 0xFF, 0x100);
    CHECK_EQ(nxrt::ipc::send_sync_request(nullptr, sm, b.addr(), 0x100), 0xE801u);
    // a message type that is not a request (type 9)
    Message u;
    u.type = 9;
    u.raw = raw_sfci(0);
    CHECK_EQ(send_msg(b, sm, u), 0xE801u);
}

NXTEST(bad_sfci_magic_answers_f601_in_the_header) {
    fresh();
    const uint32_t sm = connect("sm:");
    Message m = cmif(0);
    m.raw[0] ^= 0xFF;
    Buf b;
    CHECK_EQ(send_msg(b, sm, m), 0u);
    CHECK_EQ(result_of(reply_of(b)), 0xF601u);
}

NXTEST(domain_unknown_object_answers_f601) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    REQUIRE(convert(h) == 1);
    Buf b;
    CHECK_EQ(send_msg(b, h, domain_msg(99, 1, 1, {})), 0u);
    CHECK_EQ(result_of(reply_of(b), 16), 0xF601u);
}

NXTEST(svc_connect_and_send_sync_request) {
    fresh();
    nxrt::svc_init();
    ::Ctx c{};
    c.x[1] = reinterpret_cast<uint64_t>("sm:");
    nxrt::svc_dispatch(&c, 0x1F);   // ConnectToNamedPort
    CHECK_EQ(c.x[0], 0u);
    const uint32_t sm = static_cast<uint32_t>(c.x[1]);
    CHECK(sm != 0);

    // SendSyncRequest: the request is in the TLS buffer at TPIDRRO_EL0
    Message m = cmif(0);
    m.has_pid = true;
    m.pid = 7;
    Buf tls;
    CHECK(build_into(tls, m));
    c.x[0] = sm;
    c.tpidrro = tls.addr();
    nxrt::svc_dispatch(&c, 0x21);
    CHECK_EQ(c.x[0], 0u);
    CHECK_EQ(result_of(reply_of(tls)), 0u);

    // SendSyncRequestWithUserBuffer: GetServiceHandle("test:svc") in a user buffer
    Buf ub;
    char name[8] = {'t', 'e', 's', 't', ':', 's', 'v', 'c'};
    CHECK(build_into(ub, cmif(1, std::vector<uint8_t>(name, name + 8))));
    c.x[0] = ub.addr();
    c.x[1] = 0x100;
    c.x[2] = sm;
    nxrt::svc_dispatch(&c, 0x22);
    CHECK_EQ(c.x[0], 0u);
    Reply r = reply_of(ub);
    REQUIRE(r.ok);
    CHECK_EQ(r.msg.move.size(), 1u);
}

NXTEST(connect_name_rules_truncate_and_reject_empty) {
    fresh();
    nxrt::svc_init();
    ::Ctx c{};
    c.x[1] = reinterpret_cast<uint64_t>("abcdefghijklmnop");
    nxrt::svc_dispatch(&c, 0x1F);
    CHECK_EQ(c.x[0], 0u);
    CHECK(c.x[1] != 0);
    CHECK_MSG(contains(nxrt::ipc::stub_names_requested(), "abcdefghijkl"), "name not truncated at 12 characters");
    c.x[1] = reinterpret_cast<uint64_t>("");
    nxrt::svc_dispatch(&c, 0x1F);
    CHECK_EQ(c.x[0], 0xCC01u);
    c.x[1] = 0;
    nxrt::svc_dispatch(&c, 0x1F);
    CHECK_EQ(c.x[0], 0xCC01u);
}

NXTEST(handler_exception_becomes_invalid_state) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    Buf b;
    CHECK_EQ(send_msg(b, h, cmif(11)), 0u);
    CHECK_EQ(result_of(reply_of(b)), nxrt::kernel::kInvalidState);
}

NXTEST(hipc_vectors_route_to_stubs) {
    fresh();
    int routed = 0;
    for (size_t i = 0; i < nxrt_test::kHipcVectorCount; ++i) {
        const HipcVec& v = nxrt_test::kHipcVectors[i];
        const std::string name = v.name;
        const uint32_t h = nxrt::ipc::create_session_handle(std::make_shared<StubService>("vec:test"));
        const bool domain = name == "domain_request" || name == "domain_close";
        if (domain) {
            REQUIRE(convert(h) == 1);
            auto dom = domain_of(h);
            REQUIRE(dom != nullptr);
            dom->objects[5] = std::make_shared<StubService>("vec:obj5");
        }
        Buf b;
        std::memcpy(b.p, v.words, v.n_words * 4u);
        const uint32_t rc = nxrt::ipc::send_sync_request(nullptr, h, b.addr(), 0x100);
        if (v.type == 0) {   // a response sent as a request
            CHECK_MSG(rc == 0xE801u, name + ": response vector not rejected");
        } else if (v.type == nxrt::hipc::kRelease) {
            CHECK_MSG(rc == 0 && nxrt::kernel::handle_as<nxrt::ipc::KSession>(h) != nullptr, name + ": handle must stay open until svcCloseHandle");
        } else {
            CHECK_MSG(rc == 0, name + ": svc result " + std::to_string(rc));
            if (v.type == nxrt::hipc::kTipcBase + 1) {
                CHECK_MSG(b.p[2] == kStub, name + ": TIPC result is not the stub result");
            } else if (name == "control_convert_domain" || name == "control_query_pointer_buffer") {
                Reply r = reply_of(b);
                CHECK_MSG(r.ok, name + ": reply does not parse");
            } else if (name == "domain_close") {
                Reply r = reply_of(b);
                CHECK_MSG(r.ok && result_of(r, 16) == 0 && at32(r.msg.raw, 0) == 0, name + ": empty domain reply expected");
            } else {
                Reply r = reply_of(b);
                CHECK_MSG(r.ok, name + ": reply does not parse");
                const uint32_t want = domain ? 16 : 0;
                CHECK_MSG(result_of(r, want) == kStub, name + ": stub result expected");
            }
        }
        ++routed;
    }
    CHECK_EQ(routed, static_cast<int>(nxrt_test::kHipcVectorCount));
}

NXTEST(service_stats_and_summary_count_requests) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    Buf b;
    for (int i = 0; i < 3; ++i) CHECK_EQ(send_msg(b, h, cmif(5)), 0u);
    CHECK_EQ(send_msg(b, h, cmif(99)), 0u);
    bool found = false;
    for (const auto& st : nxrt::ipc::service_stats())
        if (st.name == "test:svc") found = st.requests == 4 && !st.stub;
    CHECK_MSG(found, "test:svc requests not counted as 4");
    const std::string s = nxrt::ipc::ipc_summary();
    CHECK_MSG(s.find("test:svc: 4 requests, 1 unhandled") != std::string::npos, "summary: " + s);
}

NXTEST(stub_name_is_recorded_once) {
    fresh();
    connect("foo:bar");
    connect("foo:bar");
    int n = 0;
    for (const std::string& s : nxrt::ipc::stub_names_requested()) n += s == "foo:bar";
    CHECK_EQ(n, 1);
}

NXTEST(registry_reset_keeps_sm) {
    fresh();
    CHECK(nxrt::ipc::has_service("test:svc"));
    nxrt::ipc::registry_reset();
    CHECK(!nxrt::ipc::has_service("test:svc"));
    CHECK(nxrt::ipc::has_service("sm:"));
    CHECK(nxrt::ipc::make_service("sm:") != nullptr);
    CHECK(!nxrt::ipc::make_service("sm:")->is_stub());
    CHECK_EQ(connect("sm:") != 0, true);
}

NXTEST(oversized_reply_answers_f601) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:big");
    REQUIRE(h != 0);
    Buf b;
    CHECK_EQ(send_msg(b, h, cmif(1)), 0u);
    CHECK_EQ(result_of(reply_of(b)), 0xF601u);
}

NXTEST(moved_handles_reach_the_service_and_are_consumed) {
    fresh();
    const uint32_t h = get_service(connect("sm:"), "test:svc");
    REQUIRE(h != 0);
    Buf b;
    CHECK_EQ(send_msg(b, h, cmif(7)), 0u);
    Reply r = reply_of(b);
    REQUIRE(r.msg.move.size() == 1);
    const uint32_t obj = r.msg.move[0];

    Message m = cmif(10);
    m.move.push_back(obj);
    Buf b2;
    CHECK_EQ(send_msg(b2, h, m), 0u);
    Reply r2 = reply_of(b2);
    CHECK_EQ(result_of(r2), 0u);
    CHECK_EQ(at32(r2.msg.raw, 16), 1u);
    CHECK_EQ(at32(r2.msg.raw, 20), 1u);
    // a move transfers the handle: the client no longer has it
    Buf b3;
    CHECK_EQ(send_msg(b3, obj, cmif(1)), nxrt::kernel::kInvalidHandle);

    // a moved handle that is not a session arrives as an empty interface
    const uint32_t ev = nxrt::kernel::handle_add(std::make_shared<nxrt::kernel::KEvent>());
    Message m2 = cmif(10);
    m2.move.push_back(ev);
    Buf b4;
    CHECK_EQ(send_msg(b4, h, m2), 0u);
    CHECK_EQ(at32(reply_of(b4).msg.raw, 16), 1u);
    CHECK_EQ(at32(reply_of(b4).msg.raw, 20), 0u);
}

NXTEST(sm_empty_name_is_invalid_service_name) {
    fresh();
    Buf b;
    CHECK_EQ(send_msg(b, connect("sm:"), cmif(1, std::vector<uint8_t>(8, 0))), 0u);
    CHECK_EQ(result_of(reply_of(b)), nxrt::kernel::make_result(21, 6));
}

NXTEST(stress_four_threads_four_sessions) {
    fresh();
    const uint32_t sm = connect("sm:");
    uint32_t h[4];
    for (uint32_t& x : h) {
        x = get_service(sm, "test:svc");
        REQUIRE(x != 0);
    }
    std::atomic<int> bad{0};
    std::vector<std::thread> threads;
    for (uint32_t t = 0; t < 4; ++t) {
        threads.emplace_back([&bad, &h, t] {
            Buf b;
            for (uint32_t i = 0; i < 2000; ++i) {
                Message m = cmif(5, pack(i, t));
                std::string err;
                if (nxrt::hipc::build(m, b.p, kMsgWords, &err) == 0) {
                    bad++;
                    continue;
                }
                if (nxrt::ipc::send_sync_request(nullptr, h[t], b.addr(), 0x100) != 0) {
                    bad++;
                    continue;
                }
                Reply r;
                if (!nxrt::hipc::parse(b.p, kMsgWords, r.msg, &err) || result_of(r) != 0 || at32(r.msg.raw, 16) != i + t) bad++;
            }
        });
    }
    for (auto& th : threads) th.join();
    CHECK_EQ(bad.load(), 0);
}
