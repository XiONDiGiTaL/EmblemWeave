// SPDX-License-Identifier: GPL-3.0-or-later
// Tests for the audren performance manager port (Server/Performance). Expected values are derived by hand from the C# layouts,
// not from running the port:
//   C# StructLayout Size attributes (Pack = 1): frame header v1 0x18, v2 0x30; entry v1/v2 0x10/0x18; detail v1/v2 0x10/0x18.
//   Field offsets: header v1 Magic 0, EntryCount 4, EntryDetailCount 8, NextOffset 12, TotalProcessingTime 16, VoiceDropCount 20;
//     header v2 adds StartRenderingTicks 24 (u64), Index 32, IsDspRunningBehind 36 (bool).
//     entry v1/v2 NodeId 0, StartTime 4, ProcessingTime 8, EntryType 12 (byte).
//     detail v1/v2 NodeId 0, StartTime 4, ProcessingTime 8, DetailType 12 (byte), EntryType 13 (byte).
//   Frame = header + entryCount * entrySize + 100 * detailSize, entryCount = voice + effect + submix + sink + 1 (detail max is the
//   fixed C# constant 100, not a parameter).
#include <cstddef>
#include <cstring>
#include <vector>

#include "../src/services/audren/Server/Performance/PerformanceManager.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_perf)

using namespace nxrt::audren;

namespace {

// Stand-in for BehaviourInfo (not ported yet): only the performance-metrics data format is used here.
struct FakeBehaviourInfo {
    uint32_t Format;
    uint32_t GetPerformanceMetricsDataFormat() const { return Format; }
};

AudioRendererConfiguration MakeConfig(uint32_t voice, uint32_t effect, uint32_t submix, uint32_t sink) {
    AudioRendererConfiguration c{};
    c.voice_count = voice;
    c.effect_count = effect;
    c.sub_mix_buffer_count = submix;
    c.sink_count = sink;
    return c;
}

// Config MakeConfig(1, 0, 0, 1) gives entryCount = 3. Frame size: v1 = 16*3 + 16*100 + 24 = 1672, v2 = 24*3 + 24*100 + 48 = 2520.
constexpr size_t kFrameV2 = 2520;
constexpr size_t kFrameV1 = 1672;

template <class T>
T At(const std::vector<uint8_t>& b, size_t off) {
    T t{};
    std::memcpy(&t, b.data() + off, sizeof t);
    return t;
}

int32_t I32At(const std::vector<uint8_t>& b, size_t off) { return At<int32_t>(b, off); }

}  // namespace

NXTEST(layout_sizes_match_csharp) {
    CHECK_EQ(sizeof(PerformanceFrameHeaderVersion1), size_t{0x18});
    CHECK_EQ(sizeof(PerformanceFrameHeaderVersion2), size_t{0x30});
    CHECK_EQ(sizeof(PerformanceEntryVersion1), size_t{0x10});
    CHECK_EQ(sizeof(PerformanceEntryVersion2), size_t{0x18});
    CHECK_EQ(sizeof(PerformanceDetailVersion1), size_t{0x10});
    CHECK_EQ(sizeof(PerformanceDetailVersion2), size_t{0x18});
}

NXTEST(layout_field_offsets_match_csharp) {
    CHECK_EQ(offsetof(PerformanceFrameHeaderVersion1, Magic), size_t{0});
    CHECK_EQ(offsetof(PerformanceFrameHeaderVersion1, EntryCount), size_t{4});
    CHECK_EQ(offsetof(PerformanceFrameHeaderVersion1, EntryDetailCount), size_t{8});
    CHECK_EQ(offsetof(PerformanceFrameHeaderVersion1, NextOffset), size_t{12});
    CHECK_EQ(offsetof(PerformanceFrameHeaderVersion1, TotalProcessingTime), size_t{16});
    CHECK_EQ(offsetof(PerformanceFrameHeaderVersion1, VoiceDropCount), size_t{20});

    CHECK_EQ(offsetof(PerformanceFrameHeaderVersion2, Magic), size_t{0});
    CHECK_EQ(offsetof(PerformanceFrameHeaderVersion2, EntryCount), size_t{4});
    CHECK_EQ(offsetof(PerformanceFrameHeaderVersion2, VoiceDropCount), size_t{20});
    CHECK_EQ(offsetof(PerformanceFrameHeaderVersion2, StartRenderingTicks), size_t{24});
    CHECK_EQ(offsetof(PerformanceFrameHeaderVersion2, Index), size_t{32});
    CHECK_EQ(offsetof(PerformanceFrameHeaderVersion2, IsDspRunningBehind), size_t{36});

    CHECK_EQ(offsetof(PerformanceEntryVersion1, NodeId), size_t{0});
    CHECK_EQ(offsetof(PerformanceEntryVersion1, StartTime), size_t{4});
    CHECK_EQ(offsetof(PerformanceEntryVersion1, ProcessingTime), size_t{8});
    CHECK_EQ(offsetof(PerformanceEntryVersion1, EntryType), size_t{12});
    CHECK_EQ(offsetof(PerformanceEntryVersion2, EntryType), size_t{12});

    CHECK_EQ(offsetof(PerformanceDetailVersion1, NodeId), size_t{0});
    CHECK_EQ(offsetof(PerformanceDetailVersion1, StartTime), size_t{4});
    CHECK_EQ(offsetof(PerformanceDetailVersion1, ProcessingTime), size_t{8});
    CHECK_EQ(offsetof(PerformanceDetailVersion1, DetailType), size_t{12});
    CHECK_EQ(offsetof(PerformanceDetailVersion1, EntryType), size_t{13});
    CHECK_EQ(offsetof(PerformanceDetailVersion2, EntryType), size_t{13});

    // The accessor offsets the C# returns (used to patch the entry from the outside).
    PerformanceEntryVersion2 e{};
    CHECK_EQ(e.GetStartTimeOffset(), 4);
    CHECK_EQ(e.GetProcessingTimeOffset(), 8);
    PerformanceDetailVersion1 d{};
    CHECK_EQ(d.GetStartTimeOffset(), 4);
    CHECK_EQ(d.GetProcessingTimeOffset(), 8);
}

NXTEST(required_buffer_size_hand_computed) {
    FakeBehaviourInfo v1{1};
    FakeBehaviourInfo v2{2};

    // entryCount 3 (1 voice + 1 sink + 1): v1 16*3 + 1600 + 24 = 1672, v2 24*3 + 2400 + 48 = 2520.
    auto a = MakeConfig(1, 0, 0, 1);
    CHECK_EQ(PerformanceManager::GetRequiredBufferSizeForPerformanceMetricsPerFrame(a, v1), uint64_t{1672});
    CHECK_EQ(PerformanceManager::GetRequiredBufferSizeForPerformanceMetricsPerFrame(a, v2), uint64_t{2520});

    // entryCount 39 (24 + 8 + 4 + 2 + 1): v1 16*39 + 1600 + 24 = 2248, v2 24*39 + 2400 + 48 = 3384.
    auto b = MakeConfig(24, 8, 4, 2);
    CHECK_EQ(PerformanceManager::GetRequiredBufferSizeForPerformanceMetricsPerFrame(b, v1), uint64_t{2248});
    CHECK_EQ(PerformanceManager::GetRequiredBufferSizeForPerformanceMetricsPerFrame(b, v2), uint64_t{3384});

    // entryCount 1 (nothing but the final sink slot): v1 16 + 1600 + 24 = 1640, v2 24 + 2400 + 48 = 2472.
    auto c = MakeConfig(0, 0, 0, 0);
    CHECK_EQ(PerformanceManager::GetRequiredBufferSizeForPerformanceMetricsPerFrame(c, v1), uint64_t{1640});
    CHECK_EQ(PerformanceManager::GetRequiredBufferSizeForPerformanceMetricsPerFrame(c, v2), uint64_t{2472});
}

NXTEST(create_v1_entry_addresses) {
    // Header v1 is 24 bytes, so entry 0 starts at 24 and detail 0 starts after the 3 entries (24 + 3*16 = 72).
    auto cfg = MakeConfig(1, 0, 0, 1);
    std::vector<uint8_t> buf(3 * kFrameV1, 0);
    std::unique_ptr<PerformanceManager> mgr = PerformanceManager::Create(Span<uint8_t>(buf), cfg, FakeBehaviourInfo{1});
    REQUIRE(mgr != nullptr);

    PerformanceEntryAddresses e;
    CHECK(mgr->GetNextEntry(e, PerformanceEntryType::Voice, 4));
    CHECK_EQ(e.EntryCountOffset, uint32_t{4});
    CHECK_EQ(e.StartTimeOffset, uint32_t{24 + 4});
    CHECK_EQ(e.ProcessingTimeOffset, uint32_t{24 + 8});

    PerformanceEntryAddresses d;
    CHECK(mgr->GetNextEntry(d, PerformanceDetailType::Mix, PerformanceEntryType::Voice, 4));
    CHECK_EQ(d.StartTimeOffset, uint32_t{72 + 4});
    CHECK_EQ(d.ProcessingTimeOffset, uint32_t{72 + 8});
}

NXTEST(entries_fill_in_order_and_extra_are_dropped) {
    // Config (1,0,0,1): 3 entries per frame, 100 detail slots. Header v2 is 48 bytes; entries at 48, details at 48 + 3*24 = 120.
    auto cfg = MakeConfig(1, 0, 0, 1);
    std::vector<uint8_t> buf(4 * kFrameV2, 0);
    std::unique_ptr<PerformanceManager> mgr = PerformanceManager::Create(Span<uint8_t>(buf), cfg, FakeBehaviourInfo{2});
    REQUIRE(mgr != nullptr);

    mgr->SetTargetNodeId(42);
    CHECK(mgr->IsTargetNodeId(42));
    CHECK(!mgr->IsTargetNodeId(7));

    PerformanceEntryAddresses a;
    for (int32_t i = 0; i < 3; i++) {
        CHECK(mgr->GetNextEntry(a, PerformanceEntryType::Voice, 10 + i));
    }
    // The 4th entry request exceeds the 3 slots: dropped, empty addresses returned.
    CHECK(!mgr->GetNextEntry(a, PerformanceEntryType::Voice, 99));
    CHECK(a.BaseMemory.IsEmpty());
    for (size_t i = 0; i < 3; i++) {
        auto e = At<PerformanceEntryVersion2>(buf, 48 + 24 * i);
        CHECK_EQ(e.NodeId, static_cast<int32_t>(10 + i));
        CHECK_EQ(static_cast<uint8_t>(e.EntryType), static_cast<uint8_t>(PerformanceEntryType::Voice));
    }

    // 100 detail slots fill in order; the 101st is dropped.
    for (int32_t i = 0; i < 100; i++) {
        CHECK(mgr->GetNextEntry(a, PerformanceDetailType::Mix, PerformanceEntryType::SubMix, 200 + i));
    }
    CHECK(!mgr->GetNextEntry(a, PerformanceDetailType::Mix, PerformanceEntryType::SubMix, 999));
    CHECK(a.BaseMemory.IsEmpty());
    for (size_t i = 0; i < 100; i += 99) {   // first and last slot
        auto d = At<PerformanceDetailVersion2>(buf, 120 + 24 * i);
        CHECK_EQ(d.NodeId, static_cast<int32_t>(200 + i));
        CHECK_EQ(static_cast<uint8_t>(d.DetailType), static_cast<uint8_t>(PerformanceDetailType::Mix));
        CHECK_EQ(static_cast<uint8_t>(d.EntryType), static_cast<uint8_t>(PerformanceEntryType::SubMix));
    }
}

NXTEST(entry_addresses_point_at_fields) {
    auto cfg = MakeConfig(1, 0, 0, 1);
    std::vector<uint8_t> buf(4 * kFrameV2, 0);
    std::unique_ptr<PerformanceManager> mgr = PerformanceManager::Create(Span<uint8_t>(buf), cfg, FakeBehaviourInfo{2});
    REQUIRE(mgr != nullptr);

    // First entry: base 48, so start time at byte 52 and processing time at byte 56, entry count at byte 4.
    PerformanceEntryAddresses a;
    CHECK(mgr->GetNextEntry(a, PerformanceEntryType::Voice, 7));
    CHECK(a.BaseMemory.Data == reinterpret_cast<int32_t*>(buf.data()));
    CHECK_EQ(a.EntryCountOffset, uint32_t{4});
    CHECK_EQ(a.StartTimeOffset, uint32_t{52});
    CHECK_EQ(a.ProcessingTimeOffset, uint32_t{56});
    a.SetStartTime(1'000'000);          // 1000 us
    a.SetProcessingTime(3'000'000);     // 3000 us end, minus the 1000 us start = 2000 us
    CHECK_EQ(I32At(buf, 52), 1000);
    CHECK_EQ(I32At(buf, 56), 2000);
    a.IncrementEntryCount();
    CHECK_EQ(I32At(buf, 4), 1);

    // Second entry: base 72.
    PerformanceEntryAddresses b;
    CHECK(mgr->GetNextEntry(b, PerformanceEntryType::SubMix, 8));
    CHECK_EQ(b.StartTimeOffset, uint32_t{76});
    CHECK_EQ(b.ProcessingTimeOffset, uint32_t{80});

    // First detail: base 48 + 3*24 = 120, start at 124, processing at 128.
    PerformanceEntryAddresses d;
    CHECK(mgr->GetNextEntry(d, PerformanceDetailType::Mix, PerformanceEntryType::SubMix, 3));
    CHECK_EQ(d.StartTimeOffset, uint32_t{124});
    CHECK_EQ(d.ProcessingTimeOffset, uint32_t{128});
    d.SetStartTime(500'000);            // 500 us
    d.SetProcessingTime(1'500'000);     // 1500 us end, minus 500 us start = 1000 us
    CHECK_EQ(I32At(buf, 124), 500);
    CHECK_EQ(I32At(buf, 128), 1000);
}

NXTEST(copy_histories_filters_and_fills_header) {
    auto cfg = MakeConfig(1, 0, 0, 1);
    std::vector<uint8_t> buf(4 * kFrameV2, 0);
    std::unique_ptr<PerformanceManager> mgr = PerformanceManager::Create(Span<uint8_t>(buf), cfg, FakeBehaviourInfo{2});
    REQUIRE(mgr != nullptr);

    // Entry 0: start 1000 us, processing 2000 us. Entry 1: no times, so CopyHistories drops it.
    PerformanceEntryAddresses e0;
    CHECK(mgr->GetNextEntry(e0, PerformanceEntryType::Voice, 5));
    e0.SetStartTime(1'000'000);
    e0.SetProcessingTime(3'000'000);
    e0.IncrementEntryCount();
    PerformanceEntryAddresses e1;
    CHECK(mgr->GetNextEntry(e1, PerformanceEntryType::SubMix, 6));
    e1.IncrementEntryCount();

    // Detail 0: start 500 us, processing 1000 us. Ryujinx never increments EntryDetailCount, so the test sets it to 1 directly.
    PerformanceEntryAddresses d0;
    CHECK(mgr->GetNextEntry(d0, PerformanceDetailType::Mix, PerformanceEntryType::SubMix, 7));
    d0.SetStartTime(500'000);
    d0.SetProcessingTime(1'500'000);
    reinterpret_cast<PerformanceFrameHeaderVersion2*>(buf.data())->SetEntryDetailCount(1);

    mgr->TapFrame(true, 2, 0x1234);

    // TapFrame resets the current frame header.
    CHECK_EQ(I32At(buf, 4), 0);

    std::vector<uint8_t> out(4096, 0xAA);
    // Output layout: header 0..47, entry at 48 (24 bytes), detail at 72 (24 bytes), blank header at 96. Returned size 96.
    CHECK_EQ(mgr->CopyHistories(Span<uint8_t>(out)), uint32_t{96});

    auto oh = At<PerformanceFrameHeaderVersion2>(out, 0);
    CHECK_EQ(oh.Magic, uint32_t{0x46524550});
    CHECK_EQ(oh.EntryCount, 1);            // entry 1 dropped
    CHECK_EQ(oh.EntryDetailCount, 1);
    CHECK_EQ(oh.NextOffset, 96);
    CHECK_EQ(oh.TotalProcessingTime, 2000);   // detail processing time is not added to the total
    CHECK_EQ(oh.VoiceDropCount, uint32_t{2});
    CHECK_EQ(oh.StartRenderingTicks, uint64_t{0x1234});
    CHECK_EQ(oh.Index, uint32_t{0});
    CHECK(oh.IsDspRunningBehind);

    auto oe = At<PerformanceEntryVersion2>(out, 48);
    CHECK_EQ(oe.NodeId, 5);
    CHECK_EQ(oe.StartTime, 1000);
    CHECK_EQ(oe.ProcessingTime, 2000);
    CHECK_EQ(static_cast<uint8_t>(oe.EntryType), static_cast<uint8_t>(PerformanceEntryType::Voice));

    auto od = At<PerformanceDetailVersion2>(out, 72);
    CHECK_EQ(od.NodeId, 7);
    CHECK_EQ(od.StartTime, 500);
    CHECK_EQ(od.ProcessingTime, 1000);
    CHECK_EQ(static_cast<uint8_t>(od.DetailType), static_cast<uint8_t>(PerformanceDetailType::Mix));
    CHECK_EQ(static_cast<uint8_t>(od.EntryType), static_cast<uint8_t>(PerformanceEntryType::SubMix));

    // Final blank header written at 96 (zeroed over the 0xAA fill).
    auto blank = At<PerformanceFrameHeaderVersion2>(out, 96);
    CHECK_EQ(blank.Magic, uint32_t{0});
    CHECK_EQ(blank.NextOffset, 0);

    // History is consumed: a second copy returns nothing.
    CHECK_EQ(mgr->CopyHistories(Span<uint8_t>(out)), uint32_t{0});
}
