// SPDX-License-Identifier: GPL-3.0-or-later
// Tests of the audio renderer base port: Constants, Common/{EdgeMatrix,NodeStates,NodeIdHelper,WorkBufferAllocator}, Utils/*.
// Every expected value is derived by hand from the C# logic in Ryujinx.Audio (not from running this port).
#include <cstring>
#include <thread>
#include <utility>
#include <vector>

#include "../src/services/audren/Constants.h"
#include "../src/services/audren/Common/EdgeMatrix.h"
#include "../src/services/audren/Common/NodeIdHelper.h"
#include "../src/services/audren/Common/NodeStates.h"
#include "../src/services/audren/Common/WorkBufferAllocator.h"
#include "../src/services/audren/Utils/BitArray.h"
#include "../src/services/audren/Utils/BitUtils.h"
#include "../src/services/audren/Utils/Mailbox.h"
#include "../src/services/audren/Utils/SpanIOHelper.h"
#include "../src/services/audren/Utils/Math/MatrixHelper.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_base)

using namespace nxrt::audren;

// Hand-derived: TargetSampleRate / TargetSampleCount = 48000 / 240 = 200, so 1000000000 / 200 = 5000000 (5.00 ms).
// UpSampleEntrySize = 240 * 6 = 1440. Per session: 5760000 / 2 = 2880000. Surround coefficients are the C# literals.
NXTEST(constants_match_csharp_values) {
    CHECK_EQ(Constants::AudioProcessorMaxUpdateTimeTarget, 5000000);
    CHECK_EQ(Constants::UpSampleEntrySize, 1440);
    CHECK_EQ(Constants::AudioProcessorMaxUpdateTimePerSessions, 2880000);
    CHECK_EQ(Constants::TargetSampleSize, 2);
    CHECK_EQ(Constants::TargetSampleCount, 240);
    CHECK_EQ(Constants::ChannelCountMax, 6);
    CHECK_EQ(Constants::VoiceChannelCountMax, 6);
    CHECK_EQ(Constants::MixBufferCountMax, 24);
    CHECK_EQ(Constants::BufferAlignment, 0x40);
    CHECK_EQ(Constants::UnusedMixId, 0x7FFFFFFF);
    CHECK_EQ(Constants::UnusedSplitterId, 0xFFFFFFFFu);
    CHECK_EQ(Constants::InvalidNodeId, -268435456);
    CHECK_EQ(Constants::FinalMixId, 0);
    CHECK_EQ(Constants::DefaultSurroundToStereoCoefficients.size(), 4u);
    CHECK_EQ(Constants::DefaultSurroundToStereoCoefficients[1], 0.707f);
    CHECK_EQ(Constants::DefaultSurroundToStereoCoefficients[2], 0.251f);
}

// Hand-derived: 0x21230045 = type 2 (<< 28, top nibble), base (bits 16..27) = 0x123.
NXTEST(node_id_helper_unpacks_type_and_base) {
    int32_t mixId = (2 << 28) | (0x123 << 16) | 0x45;
    CHECK(NodeIdHelper::GetType(mixId) == NodeIdType::Mix);
    CHECK_EQ(NodeIdHelper::GetBase(mixId), 0x123);
    CHECK(NodeIdHelper::GetType(0x10000000) == NodeIdType::Voice);
    CHECK_EQ(NodeIdHelper::GetBase(0x10000000), 0);
    CHECK(NodeIdHelper::GetType(0x30000000) == NodeIdType::Sink);
}

// Hand-derived: bit 9 is byte 1, mask 0x02. Bit 8 is byte 1, mask 0x01. Bit 31 is byte 3, mask 0x80.
NXTEST(bit_array_set_test_reset) {
    alignas(8) uint8_t raw[4] = {0, 0, 0, 0};
    BitArray bits(Span<uint8_t>(raw, 4));
    bits.Set(9);
    CHECK_EQ(raw[1], 0x02);
    CHECK(bits.Test(9));
    CHECK(!bits.Test(8));
    bits.Set(8);
    CHECK_EQ(raw[1], 0x03);
    bits.Set(31);
    CHECK_EQ(raw[3], 0x80);
    bits.Reset(9);
    CHECK_EQ(raw[1], 0x01);
    CHECK(!bits.Test(9));
    bits.Reset();
    CHECK_EQ(raw[1], 0);
    CHECK_EQ(raw[3], 0);
}

// Hand-derived: BitUtils.AlignUp(81, 64) = 128, AlignUp(64, 64) = 64, AlignUp(0, 64) = 0, AlignUp(10, 16) = 16.
NXTEST(bit_utils_align_up) {
    CHECK_EQ(BitUtils::AlignUp(81, 64), 128);
    CHECK_EQ(BitUtils::AlignUp(64, 64), 64);
    CHECK_EQ(BitUtils::AlignUp(0, 64), 0);
    CHECK_EQ(BitUtils::AlignUp<uint64_t>(10, 16), 16u);
}

// Hand-derived: EdgeMatrix size = AlignUp(n*n, 64). n=3: AlignUp(9,64)=64. n=4: AlignUp(16,64)=64. n=9: AlignUp(81,64)=128.
// Bit index of (source, destination) is n*source + destination; for n=3, (0,1)->1, (1,0)->3, (2,2)->8.
NXTEST(edge_matrix_connect_disconnect_and_remove) {
    alignas(64) uint8_t buf[64];
    std::memset(buf, 0xFF, sizeof buf);

    CHECK_EQ(EdgeMatrix::GetWorkBufferSize(3), 64);
    CHECK_EQ(EdgeMatrix::GetWorkBufferSize(4), 64);
    CHECK_EQ(EdgeMatrix::GetWorkBufferSize(9), 128);

    EdgeMatrix em;
    em.Initialize(Span<uint8_t>(buf, sizeof buf), 3);
    CHECK_EQ(em.GetNodeCount(), 3);
    CHECK_EQ(buf[0], 0);  // Initialize clears the whole storage
    CHECK(!em.Connected(0, 0));

    em.Connect(0, 1);  // bit 1 -> byte 0, mask 0x02
    CHECK(em.Connected(0, 1));
    CHECK(!em.Connected(1, 0));
    em.Connect(2, 2);  // bit 8 -> byte 1, mask 0x01
    CHECK_EQ(buf[1], 0x01);
    CHECK(em.Connected(2, 2));
    em.Connect(1, 0);  // bit 3 -> byte 0, mask 0x08
    CHECK_EQ(buf[0], 0x0A);

    em.Disconnect(0, 1);
    CHECK(!em.Connected(0, 1));
    CHECK_EQ(buf[0], 0x08);

    em.RemoveEdges(1);  // clears bits 3, 4, 5
    CHECK(!em.Connected(1, 0));
    CHECK_EQ(buf[0], 0x00);
    CHECK(em.Connected(2, 2));
}

// Hand-derived: GetWorkBufferSize(n) = n*n*4 (stack) + 12*n + 2*AlignUp(n*n, 64).
// n=4: 64 + 48 + 2*64 = 240. n=3: 36 + 36 + 2*64 = 200. n=0: 0.
NXTEST(node_states_work_buffer_size) {
    CHECK_EQ(NodeStates::GetWorkBufferSize(4), 240);
    CHECK_EQ(NodeStates::GetWorkBufferSize(3), 200);
    CHECK_EQ(NodeStates::GetWorkBufferSize(0), 0);
}

namespace {

// Sorts a graph of up to 4 nodes with the given edges. Returns Sort's verdict and fills `order` with GetTsortResult.
bool SortGraph(int32_t nodeCount, const std::vector<std::pair<int32_t, int32_t>>& edges, std::vector<int32_t>& order) {
    alignas(64) uint8_t edgeBuf[64] = {};
    alignas(64) uint8_t nodeBuf[240] = {};

    EdgeMatrix em;
    em.Initialize(Span<uint8_t>(edgeBuf, sizeof edgeBuf), nodeCount);
    for (const auto& edge : edges) {
        em.Connect(edge.first, edge.second);
    }

    NodeStates ns;
    ns.Initialize(Span<uint8_t>(nodeBuf, sizeof nodeBuf), nodeCount);
    bool ok = ns.Sort(em);

    order.clear();
    Span<const int32_t> result = ns.GetTsortResult();
    for (size_t i = 0; i < result.Length; i++) {
        order.push_back(result[i]);
    }
    return ok;
}

}  // namespace

// Hand-derived DFS for edges 0->1, 0->2, 1->3, 2->3: nodes finish in the order 3, 2, 1, 0, and that finish order is the
// result, so every edge's target comes before its source: [3, 2, 1, 0].
NXTEST(node_states_sort_diamond) {
    std::vector<int32_t> order;
    bool ok = SortGraph(4, {{0, 1}, {0, 2}, {1, 3}, {2, 3}}, order);
    CHECK(ok);
    REQUIRE(order.size() == 4);
    CHECK_EQ(order[0], 3);
    CHECK_EQ(order[1], 2);
    CHECK_EQ(order[2], 1);
    CHECK_EQ(order[3], 0);
}

// Hand-derived: chain 0->1->2 finishes 2, 1, 0.
NXTEST(node_states_sort_chain) {
    std::vector<int32_t> order;
    CHECK(SortGraph(3, {{0, 1}, {1, 2}}, order));
    REQUIRE(order.size() == 3);
    CHECK_EQ(order[0], 2);
    CHECK_EQ(order[1], 1);
    CHECK_EQ(order[2], 0);
}

// Hand-derived: no edges. Each node is pushed, finished and emitted in index order: [0, 1, 2].
NXTEST(node_states_sort_disconnected) {
    std::vector<int32_t> order;
    CHECK(SortGraph(3, {}, order));
    REQUIRE(order.size() == 3);
    CHECK_EQ(order[0], 0);
    CHECK_EQ(order[1], 1);
    CHECK_EQ(order[2], 2);
}

// Hand-derived: edges 0->1, 0->2, 2->1. Node 1 is pushed twice (from 0 and from 2). Trace: stack [0,1,2,1]; 1 finishes first,
// then 2, then the stale 1 is popped as finished, then 0. Result [1, 2, 0]: every target (1 for 0->1 and 2->1, 2 for 0->2)
// comes before its source.
NXTEST(node_states_sort_cross_edge_duplicate_push) {
    std::vector<int32_t> order;
    CHECK(SortGraph(3, {{0, 1}, {0, 2}, {2, 1}}, order));
    REQUIRE(order.size() == 3);
    CHECK_EQ(order[0], 1);
    CHECK_EQ(order[1], 2);
    CHECK_EQ(order[2], 0);
}

// Hand-derived: 0->1 and 1->0 is a loop. Sort returns false and the result is reset to empty.
NXTEST(node_states_sort_detects_two_cycle) {
    std::vector<int32_t> order;
    CHECK(!SortGraph(2, {{0, 1}, {1, 0}}, order));
    CHECK(order.empty());
}

// Hand-derived: a self-edge 0->0 is a loop of length 1.
NXTEST(node_states_sort_detects_self_loop) {
    std::vector<int32_t> order;
    CHECK(!SortGraph(1, {{0, 0}}, order));
    CHECK(order.empty());
}

// Hand-derived: 0->1->2->0. The DFS reaches 2 with 0 still Discovered, so the loop is found.
NXTEST(node_states_sort_detects_three_cycle) {
    std::vector<int32_t> order;
    CHECK(!SortGraph(3, {{0, 1}, {1, 2}, {2, 0}}, order));
    CHECK(order.empty());
}

// C# returns true immediately for a node count of zero.
NXTEST(node_states_sort_empty_graph) {
    std::vector<int32_t> order;
    CHECK(SortGraph(0, {}, order));
    CHECK(order.empty());
}

// Hand-derived on a 256-byte buffer.
// Offsets: Allocate(10, 16) at 0 -> Offset 10. Allocate(4, 64) at AlignUp(10,64)=64 -> Offset 68.
// Allocate(200, 1) would end at 268 > 256 -> empty, Offset unchanged. Allocate<uint32_t>(10, 16) at AlignUp(68,16)=80 ->
// 40 bytes -> Offset 120. Allocate(136, 1) ends exactly at 256 -> fits. Allocate(1, 1) -> empty.
NXTEST(work_buffer_allocator_alignment_and_bounds) {
    alignas(64) uint8_t mem[256];
    std::memset(mem, 0xAB, sizeof mem);
    WorkBufferAllocator wba{Span<uint8_t>(mem, sizeof mem)};

    Span<uint8_t> a = wba.Allocate(10, 16);
    CHECK_EQ(a.Length, 10u);
    CHECK(a.Data == mem);
    CHECK_EQ(wba.Offset, 10u);
    CHECK_EQ(mem[9], 0);      // allocated bytes are cleared
    CHECK_EQ(mem[10], 0xAB);  // bytes past the allocation are untouched

    Span<uint8_t> b = wba.Allocate(4, 64);
    CHECK_EQ(b.Length, 4u);
    CHECK(b.Data == mem + 64);
    CHECK_EQ(wba.Offset, 68u);

    Span<uint8_t> c = wba.Allocate(0, 8);
    CHECK(c.IsEmpty());
    CHECK_EQ(wba.Offset, 68u);

    Span<uint8_t> d = wba.Allocate(200, 1);
    CHECK(d.IsEmpty());
    CHECK_EQ(wba.Offset, 68u);

    Span<uint32_t> e = wba.Allocate<uint32_t>(10, 16);
    CHECK_EQ(e.Length, 10u);
    CHECK(reinterpret_cast<uint8_t*>(e.Data) == mem + 80);
    CHECK_EQ(e[0], 0u);
    CHECK_EQ(wba.Offset, 120u);

    Span<uint8_t> f = wba.Allocate(136, 1);
    CHECK_EQ(f.Length, 136u);
    CHECK_EQ(wba.Offset, 256u);

    Span<uint8_t> g = wba.Allocate(1, 1);
    CHECK(g.IsEmpty());
    CHECK_EQ(wba.Offset, 256u);
}

// Hand-derived: GetTargetSize<T>(cur, count, align) = AlignUp(cur, align) + sizeof(T) * count.
// <uint32_t>(10, 3, 16) = 16 + 12 = 28. <uint64_t>(0, 2, 8) = 0 + 16 = 16.
NXTEST(work_buffer_allocator_target_size) {
    CHECK_EQ(WorkBufferAllocator::GetTargetSize<uint32_t>(10, 3, 16), 28u);
    CHECK_EQ(WorkBufferAllocator::GetTargetSize<uint64_t>(0, 2, 8), 16u);
}

// Hand-derived for a little-endian host: Write<uint32_t>(0x11223344) puts 44 33 22 11 at bytes 0..3; Write<uint16_t>(0xBEEF)
// puts EF BE at 4..5; Write<uint16_t>(0x0A0B) puts 0B 0A at 6..7. Remaining after the three writes: 32 - 8 = 24 bytes.
// GetWriteRef<uint64_t> at offset 8 holds 0x0102030405060708 (bytes 08 07 06 05 04 03 02 01 at 8..15). Remaining: 16 bytes.
NXTEST(span_io_helper_round_trip) {
    alignas(8) uint8_t buf[32] = {};
    SpanIOHelper<uint8_t> w{Span<uint8_t>(buf, sizeof buf)};
    w.Write<uint32_t>(0x11223344u);
    w.Write<uint16_t>(0xBEEFu);
    w.Write<uint16_t>(0x0A0Bu);
    CHECK_EQ(buf[0], 0x44);
    CHECK_EQ(buf[3], 0x11);
    CHECK_EQ(buf[4], 0xEF);
    CHECK_EQ(buf[5], 0xBE);
    CHECK_EQ(w.Remaining().Length, 24u);

    Span<uint64_t> ref = w.GetWriteRef<uint64_t>();
    CHECK_EQ(ref.Length, 1u);
    ref[0] = 0x0102030405060708ull;
    CHECK_EQ(buf[8], 0x08);
    CHECK_EQ(buf[15], 0x01);
    CHECK_EQ(w.Remaining().Length, 16u);

    SpanIOHelper<const uint8_t> r{Span<const uint8_t>(buf, sizeof buf)};
    CHECK_EQ(r.Read<uint32_t>(), 0x11223344u);
    CHECK_EQ(r.Read<uint16_t>(), 0xBEEFu);
    CHECK_EQ(r.Read<uint16_t>(), 0x0A0Bu);
    CHECK_EQ(r.Read<uint64_t>(), 0x0102030405060708ull);
    CHECK_EQ(r.Remaining().Length, 16u);
}

// Hand-derived: GetMemory(arr, 1, 3) is the one-element slice at index 1 (value 20). GetFromMemory(arr, 2, 3) is a reference to
// index 2 (value 30), so writing 31 through it changes arr[2].
NXTEST(span_io_helper_get_memory) {
    uint32_t arr[3] = {10, 20, 30};
    Span<uint32_t> sp(arr, 3);

    Span<uint32_t> slice = SpanIOHelper<uint8_t>::GetMemory(sp, 1, 3);
    CHECK_EQ(slice.Length, 1u);
    CHECK(slice.Data == arr + 1);
    CHECK_EQ(slice[0], 20u);

    uint32_t& element = SpanIOHelper<uint8_t>::GetFromMemory(sp, 2, 3);
    CHECK_EQ(element, 30u);
    element = 31;
    CHECK_EQ(arr[2], 31u);
}

// Hand-derived: Matrix2x2 A = [1 2; 3 4], B = [5 6; 7 8]. A + B = [6 8; 10 12]. A * B = [19 22; 43 50].
// A - 1 = [0 1; 2 3]. A * 2 = [2 4; 6 8].
NXTEST(matrix2x2_operators) {
    Matrix2x2 a(1, 2, 3, 4);
    Matrix2x2 b(5, 6, 7, 8);
    CHECK(a + b == Matrix2x2(6, 8, 10, 12));
    CHECK(a * b == Matrix2x2(19, 22, 43, 50));
    CHECK(a - 1.0f == Matrix2x2(0, 1, 2, 3));
    CHECK(a * 2.0f == Matrix2x2(2, 4, 6, 8));
}

// Hand-derived Vector6 arithmetic: a = (1,2,3,4,5,6), b = (6,5,4,3,2,1). a + b = (7,7,7,7,7,7). a * b = (6,10,12,12,10,6).
// a * 2 = (2,4,6,8,10,12). Vector6(3) = (3,3,3,3,3,3).
NXTEST(vector6_operators) {
    Vector6 a(1, 2, 3, 4, 5, 6);
    Vector6 b(6, 5, 4, 3, 2, 1);
    CHECK(a + b == Vector6(7, 7, 7, 7, 7, 7));
    CHECK(a * b == Vector6(6, 10, 12, 12, 10, 6));
    CHECK(a * 2.0f == Vector6(2, 4, 6, 8, 10, 12));
    CHECK(Vector6(3.0f) == Vector6(3, 3, 3, 3, 3, 3));
}

// Hand-derived 6x6 transform. Identity except M12 = 2, M16 = 3, M61 = 5. Input v = (X=1, Y=10, Z=100, W=0, V=0, U=2).
// X' = 1*1 + 2*10 + 3*2 = 27. Y' = 10. Z' = 100. W' = 0. V' = 0. U' = 5*1 + 1*2 = 7.
NXTEST(matrix6x6_transform_hand_computed) {
    Matrix6x6 m;
    m.M11 = 1;
    m.M22 = 1;
    m.M33 = 1;
    m.M44 = 1;
    m.M55 = 1;
    m.M66 = 1;
    m.M12 = 2;
    m.M16 = 3;
    m.M61 = 5;

    Vector6 v(1, 10, 100, 0, 0, 2);
    Vector6 out = MatrixHelper::Transform(v, m);
    CHECK(out == Vector6(27, 10, 100, 0, 0, 7));
}

// Hand-derived: Vector2 (3,4) times [1 2; 3 4] = (1*3 + 2*4, 3*3 + 4*4) = (11, 25).
// Vector4 (1,10,0,0) times identity with M12 = 2 = (1 + 2*10, 10, 0, 0) = (21, 10, 0, 0).
NXTEST(matrix_helper_small_transforms) {
    Numerics::Vector2 v2 = MatrixHelper::Transform(Numerics::Vector2(3, 4), Matrix2x2(1, 2, 3, 4));
    CHECK(v2 == Numerics::Vector2(11, 25));

    Numerics::Matrix4x4 m4;
    m4.M11 = 1;
    m4.M22 = 1;
    m4.M33 = 1;
    m4.M44 = 1;
    m4.M12 = 2;
    Numerics::Vector4 v4 = MatrixHelper::Transform(Numerics::Vector4(1, 10, 0, 0), m4);
    CHECK(v4 == Numerics::Vector4(21, 10, 0, 0));
}

// Hand-derived: messages and responses are independent channels. Sending message 5 and response 6, then receiving the response
// first, gives 6 then 5. A producer thread sending 1..100 through the capacity-1 queue must deliver them in order.
NXTEST(mailbox_order_and_threads) {
    Mailbox<uint32_t> mb;
    mb.SendMessage(5);
    mb.SendResponse(6);
    CHECK_EQ(mb.ReceiveResponse(), 6u);
    CHECK_EQ(mb.ReceiveMessage(), 5u);

    std::thread producer([&mb] {
        for (uint32_t i = 1; i <= 100; i++) {
            mb.SendMessage(i);
        }
    });

    bool inOrder = true;
    for (uint32_t expected = 1; expected <= 100; expected++) {
        if (mb.ReceiveMessage() != expected) {
            inOrder = false;
        }
    }
    producer.join();
    CHECK(inOrder);
}
