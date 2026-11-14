// SPDX-License-Identifier: GPL-3.0-or-later
// Server/Splitter and Server/Mix ports of the audio renderer (SplitterContext, SplitterState, SplitterDestination, MixInfo, MixContext).
//
// Every expected value below is derived by hand from the C# sources under refs/ryubing/src/Ryujinx.Audio/Renderer/Server/{Splitter,Mix}
// (and Renderer/Common/NodeStates.cs, EdgeMatrix.cs, Utils/WorkBufferAllocator.cs), not from running the port. The derivation is noted
// next to each group.
#include <cstring>
#include <vector>

#include "../src/services/audren/Server/BehaviourInfo.h"
#include "../src/services/audren/Server/Mix/MixContext.h"
#include "../src/services/audren/Server/Mix/MixInfo.h"
#include "../src/services/audren/Server/RendererSystemContext.h"
#include "../src/services/audren/Server/Splitter/SplitterContext.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_mix)

namespace {

using namespace nxrt::audren;

// ---- helpers ----

// C# BehaviourInfo with UserRevision = BaseRevisionMagic + (N << 24). The revision constants (Revision5, ...) are the C# ones.
BehaviourInfo MakeBehaviour(int32_t revision) {
    BehaviourInfo bi;
    bi.SetUserRevision(BehaviourInfo::BaseRevisionMagic + revision);
    return bi;
}

// Append the raw bytes of a value to a byte buffer (the user input buffer of the splitter update).
template <class T>
void Append(std::vector<uint8_t>& buffer, const T& value) {
    size_t offset = buffer.size();
    buffer.resize(offset + sizeof(T));
    std::memcpy(buffer.data() + offset, &value, sizeof(T));
}

// C# SplitterInParameterHeader with the valid magic.
SplitterInParameterHeader MakeHeader(uint32_t splitterCount, uint32_t destinationCount) {
    SplitterInParameterHeader header{};
    header.magic = SplitterInParameterHeader::ValidMagic;
    header.splitter_count = splitterCount;
    header.splitter_destination_count = destinationCount;
    return header;
}

// C# SplitterInParameter with the valid magic.
SplitterInParameter MakeSplitter(int32_t id, uint32_t sampleRate, int32_t destinationCount) {
    SplitterInParameter splitter{};
    splitter.magic = SplitterInParameter::ValidMagic;
    splitter.id = id;
    splitter.sample_rate = sampleRate;
    splitter.destination_count = destinationCount;
    return splitter;
}

// Version 1 destination parameter (0x70 bytes). mix_buffer_volume[0] is set to firstVolume, the rest is zero.
SplitterDestinationInParameterVersion1 MakeDestV1(int32_t id, int32_t destinationId, float firstVolume, bool isUsed) {
    SplitterDestinationInParameterVersion1 p{};
    p.magic = SplitterDestinationInParameterVersion1::ValidMagic;
    p.id = id;
    p.destination_id = destinationId;
    p.mix_buffer_volume[0] = firstVolume;
    p.is_used = isUsed;
    p.reset_prev_volume = false;
    return p;
}

// Version 2a destination parameter (0x90 bytes, 12-byte biquad filters). Biquad filters are zero.
SplitterDestinationInParameterVersion2a MakeDestV2a(int32_t id, int32_t destinationId, float firstVolume, bool isUsed) {
    SplitterDestinationInParameterVersion2a p{};
    p.magic = SplitterDestinationInParameterVersion2a::ValidMagic;
    p.id = id;
    p.destination_id = destinationId;
    p.mix_buffer_volume[0] = firstVolume;
    p.is_used = isUsed;
    p.reset_prev_volume = false;
    return p;
}

// Version 2b destination parameter (0xA8 bytes, float biquad filters). Biquad filters are zero.
SplitterDestinationInParameterVersion2b MakeDestV2b(int32_t id, int32_t destinationId, float firstVolume, bool isUsed) {
    SplitterDestinationInParameterVersion2b p{};
    p.magic = SplitterDestinationInParameterVersion2b::ValidMagic;
    p.id = id;
    p.destination_id = destinationId;
    p.mix_buffer_volume[0] = firstVolume;
    p.is_used = isUsed;
    p.reset_prev_volume = false;
    return p;
}

// Build a complete splitter update input: header, one splitter parameter, its destination ids, 12 unknown bytes, then the destination
// parameters. The buffer is padded to a multiple of 16 bytes (the C# update aligns the consumed count to 0x10, so the padding is what it
// consumes at the end).
std::vector<uint8_t> BuildInput(const SplitterInParameterHeader& header, const SplitterInParameter& splitter, const std::vector<int32_t>& ids,
                                const std::vector<uint8_t>& destinations) {
    std::vector<uint8_t> buffer;
    Append(buffer, header);
    Append(buffer, splitter);
    for (int32_t id : ids) {
        Append(buffer, id);
    }
    buffer.resize(buffer.size() + 12, 0);
    buffer.insert(buffer.end(), destinations.begin(), destinations.end());
    buffer.resize((buffer.size() + 15) / 16 * 16, 0);
    return buffer;
}

// Build the SplitterContext for a revision, with splitterCount splitters and destinationCount destinations. The storage (work buffer and
// biquad states) is owned by the caller. Returns the result of SplitterContext.Initialize.
bool InitSplitter(SplitterContext& context, BehaviourInfo& behaviour, uint32_t splitterCount, uint32_t destinationCount,
                  std::vector<uint8_t>& workBuffer, std::vector<BiquadFilterState>& biquadStates) {
    AudioRendererConfiguration configuration{};
    configuration.splitter_count = splitterCount;
    configuration.splitter_destination_count = destinationCount;

    uint64_t size = SplitterContext::GetWorkBufferSize(0, behaviour, configuration);
    workBuffer.assign(static_cast<size_t>(size), 0);

    WorkBufferAllocator allocator{Span<uint8_t>(workBuffer)};
    biquadStates.assign(static_cast<size_t>(destinationCount) * SplitterContext::BqfStatesPerDestination, BiquadFilterState{});

    return context.Initialize(behaviour, configuration, allocator, Span<BiquadFilterState>(biquadStates));
}

// An effect context with no effects (only the members MixInfo.Update reads are provided).
struct NoEffects {
    struct Effect {
        int32_t MixId = 0;
        uint32_t ProcessingOrder = 0;
    };

    int32_t GetCount() const { return 0; }
    Effect& GetEffect(int32_t) { return _unused; }

    Effect _unused;
};

// MixParameter with the given fields. Only the fields used by the tests are set.
MixParameter MakeMix(int32_t mixId, uint32_t bufferCount, int32_t destinationMixId, uint32_t destinationSplitterId) {
    MixParameter p{};
    p.volume = 1.0f;
    p.sample_rate = 48000;
    p.buffer_count = bufferCount;
    p.is_used = true;
    p.mix_id = mixId;
    p.node_id = mixId;
    p.destination_mix_id = destinationMixId;
    p.destination_splitter_id = destinationSplitterId;
    return p;
}

}  // namespace

// Derivation (C# SplitterContext.GetWorkBufferSize, WorkBufferAllocator.GetTargetSize):
//   revision 5 (version 1, bug fixed): 0x20 (one SplitterState) + 2 * 0xE0 (version 1) + 2 * sizeof(int) = 32 + 448 + 8 = 488.
//   revision 12 (version 2):           32 + 2 * 0x128 + 8 = 32 + 592 + 8 = 632.
//   revision 1 (no splitters):          0.
// The C++ sizeof values are also pinned by static_asserts in the headers (0x20, 0xE0, 0x128, 0x940).
NXTEST(splitter_work_buffer_size_matches_csharp) {
    AudioRendererConfiguration configuration{};
    configuration.splitter_count = 1;
    configuration.splitter_destination_count = 2;

    BehaviourInfo revision5 = MakeBehaviour(BehaviourInfo::Revision5);
    CHECK_EQ(SplitterContext::GetWorkBufferSize(0, revision5, configuration), 488u);

    BehaviourInfo revision12 = MakeBehaviour(BehaviourInfo::Revision12);
    CHECK_EQ(SplitterContext::GetWorkBufferSize(0, revision12, configuration), 632u);

    BehaviourInfo revision1 = MakeBehaviour(BehaviourInfo::Revision1);
    CHECK_EQ(SplitterContext::GetWorkBufferSize(0, revision1, configuration), 0u);
}

// Derivation (C# SplitterContext.Update, SplitterState.Update, UpdateData, Revision 5 = version 1):
//   header 0x20 bytes                                  -> consumed 32
//   SplitterInParameter 0x10 bytes                     -> 48
//   destination ids 2 * 4 bytes (destinationCount is the parameter count, as the bug fix trusts it) -> 56
//   unknown 0xC bytes (Advance(0xC))                   -> 68 (0x44)
//   two version 1 destination parameters, 2 * 0x70      -> 68 + 224 = 292
//   AlignUp(292, 0x10) = 304                           -> Update returns true, consumed 304.
//   Destination list: ids [0, 1]. Destination 0 -> mix 2, volume[0] 0.5 (IsUsed was false and the parameter is used, so the previous volume is
//   reset from the new one). Destination 1 -> mix 1, volume[0] 0.25.
NXTEST(splitter_update_version1_rev5) {
    BehaviourInfo behaviour = MakeBehaviour(BehaviourInfo::Revision5);
    SplitterContext context;
    std::vector<uint8_t> workBuffer;
    std::vector<BiquadFilterState> biquadStates;
    REQUIRE(InitSplitter(context, behaviour, 1, 2, workBuffer, biquadStates));

    CHECK_EQ(context.Version, 1);
    CHECK(context.IsBugFixed);
    CHECK(!context.IsSplitterPrevVolumeResetSupported);
    CHECK(context.UsingSplitter());

    std::vector<uint8_t> destinations;
    Append(destinations, MakeDestV1(0, 2, 0.5f, true));
    Append(destinations, MakeDestV1(1, 1, 0.25f, true));
    std::vector<uint8_t> input = BuildInput(MakeHeader(1, 2), MakeSplitter(0, 48000, 2), {0, 1}, destinations);

    SequenceReader reader{Span<const uint8_t>(input)};
    CHECK(context.Update(reader));
    CHECK_EQ(reader.Consumed(), 304);

    SplitterState& splitter = context.GetState(0);
    CHECK_EQ(splitter.Id, 0);
    CHECK_EQ(splitter.SampleRate, 48000u);
    CHECK_EQ(splitter.DestinationCount, 2);
    CHECK(splitter.HasNewConnection);

    SplitterDestination first = splitter.Destination();
    CHECK_EQ(first.Id(), 0);
    CHECK_EQ(first.DestinationId(), 2);
    CHECK(first.IsConfigured());
    CHECK(first.GetMixVolume(0) == 0.5f);
    CHECK(first.GetMixVolumePrev(0) == 0.5f);

    SplitterDestination second = first.Next();
    CHECK_EQ(second.Id(), 1);
    CHECK_EQ(second.DestinationId(), 1);
    CHECK(second.GetMixVolume(0) == 0.25f);
    CHECK(second.Next().IsNull());

    // Derivation: 1 splitter, 2 destinations, IsBugFixed: SplitterDestinationCount of the state is 2 and the storage holds 2 entries.
    CHECK_EQ(context.GetDestinationCountPerStateForCompatibility(), 2);
}

// Derivation (C# SplitterContext.Update): the header magic is wrong, so the header is rewound and the update fails. The consumed count stays
// at 0 (read 0x20, then Rewind(0x20)).
NXTEST(splitter_update_rejects_bad_header_magic) {
    BehaviourInfo behaviour = MakeBehaviour(BehaviourInfo::Revision5);
    SplitterContext context;
    std::vector<uint8_t> workBuffer;
    std::vector<BiquadFilterState> biquadStates;
    REQUIRE(InitSplitter(context, behaviour, 1, 2, workBuffer, biquadStates));

    std::vector<uint8_t> input(64, 0);
    SequenceReader reader{Span<const uint8_t>(input)};

    CHECK(!context.Update(reader));
    CHECK_EQ(reader.Consumed(), 0);
}

// Derivation (C# SplitterContext.Initialize): revision 1 does not support splitters, so the context is set up empty and UsingSplitter is
// false. Update returns true at once without consuming anything (C# "if (!UsingSplitter()) return true").
NXTEST(splitter_inactive_before_revision_2) {
    BehaviourInfo behaviour = MakeBehaviour(BehaviourInfo::Revision1);
    SplitterContext context;
    std::vector<uint8_t> workBuffer;
    std::vector<BiquadFilterState> biquadStates;
    REQUIRE(InitSplitter(context, behaviour, 1, 2, workBuffer, biquadStates));

    CHECK(!context.UsingSplitter());

    std::vector<uint8_t> input(64, 0);
    SequenceReader reader{Span<const uint8_t>(input)};

    CHECK(context.Update(reader));
    CHECK_EQ(reader.Consumed(), 0);
}

// Derivation (C# SplitterContext.Initialize and UpdateData, Revision 12 = version 2, float filters off below revision 15):
//   version 2a parameters, 0x90 bytes each -> consumed 68 + 2 * 0x90 = 68 + 288 = 356, AlignUp(356, 0x10) = 368.
//   Previous volume: revision 12 has no explicit reset (below revision 13), so it is reset from the new one (!IsUsed && IsUsed).
NXTEST(splitter_update_version2_rev12_uses_v2a_size) {
    BehaviourInfo behaviour = MakeBehaviour(BehaviourInfo::Revision12);
    SplitterContext context;
    std::vector<uint8_t> workBuffer;
    std::vector<BiquadFilterState> biquadStates;
    REQUIRE(InitSplitter(context, behaviour, 1, 2, workBuffer, biquadStates));

    CHECK_EQ(context.Version, 2);
    CHECK(!context.IsBiquadFilterParameterFloatSupported);
    CHECK(!context.IsSplitterPrevVolumeResetSupported);

    std::vector<uint8_t> destinations;
    Append(destinations, MakeDestV2a(0, 2, 0.5f, true));
    Append(destinations, MakeDestV2a(1, 1, 0.25f, true));
    std::vector<uint8_t> input = BuildInput(MakeHeader(1, 2), MakeSplitter(0, 48000, 2), {0, 1}, destinations);

    SequenceReader reader{Span<const uint8_t>(input)};
    CHECK(context.Update(reader));
    CHECK_EQ(reader.Consumed(), 368);

    SplitterDestination first = context.GetState(0).Destination();
    CHECK_EQ(first.Id(), 0);
    CHECK_EQ(first.DestinationId(), 2);
    CHECK(first.GetMixVolume(0) == 0.5f);
    CHECK(first.GetMixVolumePrev(0) == 0.5f);
    CHECK_EQ(first.Next().Id(), 1);
}

// Derivation (C# SplitterContext.UpdateData, Revision 15 = version 2b, float filters on):
//   version 2b parameters, 0xA8 bytes each -> consumed 68 + 2 * 0xA8 = 68 + 336 = 404, AlignUp(404, 0x10) = 416.
//   Explicit reset supported (revision 13 and later): the previous volume follows reset_prev_volume, which is false here, so it stays 0.
NXTEST(splitter_update_version2_rev15_uses_v2b_size) {
    BehaviourInfo behaviour = MakeBehaviour(BehaviourInfo::Revision15);
    SplitterContext context;
    std::vector<uint8_t> workBuffer;
    std::vector<BiquadFilterState> biquadStates;
    REQUIRE(InitSplitter(context, behaviour, 1, 2, workBuffer, biquadStates));

    CHECK_EQ(context.Version, 2);
    CHECK(context.IsBiquadFilterParameterFloatSupported);
    CHECK(context.IsSplitterPrevVolumeResetSupported);

    std::vector<uint8_t> destinations;
    Append(destinations, MakeDestV2b(0, 2, 0.5f, true));
    Append(destinations, MakeDestV2b(1, 1, 0.25f, true));
    std::vector<uint8_t> input = BuildInput(MakeHeader(1, 2), MakeSplitter(0, 48000, 2), {0, 1}, destinations);

    SequenceReader reader{Span<const uint8_t>(input)};
    CHECK(context.Update(reader));
    CHECK_EQ(reader.Consumed(), 416);

    SplitterDestination first = context.GetState(0).Destination();
    CHECK_EQ(first.DestinationId(), 2);
    CHECK(first.GetMixVolume(0) == 0.5f);
    CHECK(first.GetMixVolumePrev(0) == 0.0f);
}

// Derivation (C# MixInfo constructor, revision 5 splitter path, MixInfo.Update):
//   constructor: MixId = Unused, DestinationMixId = Unused, DestinationSplitterId = Unused, Distance = Invalid (0x80000000),
//   IsLongSizePreDelaySupported = (revision >= 3) = true, effect order cleared to -1.
//   Update from parameter (mix 1, volume 0.75, rate 48000, 2 buffers, destination mix 0, no splitter):
//     isDirty = UpdateConnection: destination changed (Unused -> 0), so RemoveEdges(1), Connect(1, 0), returns true.
//   Second Update with only the volume changed: destination and splitter unchanged, no new splitter connection -> returns false (dirty only
//   when the connections change).
//   Third Update with destination 2: returns true, edge (1, 0) removed and (1, 2) added.
//   BufferOffset is not set by Update (MixContext sets it), so it stays 0.
NXTEST(mix_info_update_from_parameter_rev5) {
    BehaviourInfo behaviour = MakeBehaviour(BehaviourInfo::Revision5);

    std::vector<int32_t> effectOrder(4, 99);
    MixInfo mix(Span<int32_t>(effectOrder), behaviour);

    CHECK_EQ(mix.MixId, Constants::UnusedMixId);
    CHECK_EQ(mix.DestinationMixId, Constants::UnusedMixId);
    CHECK_EQ(mix.DestinationSplitterId, Constants::UnusedSplitterId);
    CHECK_EQ(mix.DistanceFromFinalMix, MixInfo::InvalidDistanceFromFinalMix);
    CHECK(mix.IsLongSizePreDelaySupported);
    for (int32_t order : effectOrder) {
        CHECK_EQ(order, -1);
    }

    std::vector<uint8_t> edgeWorkBuffer(static_cast<size_t>(EdgeMatrix::GetWorkBufferSize(3)), 0);
    EdgeMatrix edges;
    edges.Initialize(Span<uint8_t>(edgeWorkBuffer), 3);

    SplitterContext splitters;  // not initialized: only read for splitter ids
    NoEffects effects;

    MixParameter parameter = MakeMix(1, 2, 0, Constants::UnusedSplitterId);
    parameter.volume = 0.75f;
    parameter.node_id = 0x1234;
    parameter.mix_buffer_volume[0 * 24 + 0] = 1.0f;
    parameter.mix_buffer_volume[0 * 24 + 1] = 0.5f;
    parameter.mix_buffer_volume[5 * 24 + 7] = 0.125f;

    CHECK(mix.Update(edges, parameter, effects, splitters, behaviour));

    CHECK(mix.Volume == 0.75f);
    CHECK_EQ(mix.SampleRate, 48000u);
    CHECK_EQ(mix.BufferCount, 2u);
    CHECK(mix.IsUsed);
    CHECK_EQ(mix.MixId, 1);
    CHECK_EQ(mix.NodeId, 0x1234);
    CHECK_EQ(mix.DestinationMixId, 0);
    CHECK_EQ(mix.DestinationSplitterId, Constants::UnusedSplitterId);
    CHECK(mix.GetMixBufferVolume(0, 0) == 1.0f);
    CHECK(mix.GetMixBufferVolume(0, 1) == 0.5f);
    CHECK(mix.GetMixBufferVolume(5, 7) == 0.125f);
    CHECK(mix.GetMixBufferVolume(1, 1) == 0.0f);
    CHECK_EQ(mix.BufferOffset, 0u);
    CHECK(edges.Connected(1, 0));
    CHECK(!edges.Connected(0, 1));
    for (int32_t order : effectOrder) {
        CHECK_EQ(order, -1);
    }

    // Dirty-only: a volume change alone does not change the connections.
    parameter.volume = 0.5f;
    CHECK(!mix.Update(edges, parameter, effects, splitters, behaviour));
    CHECK(mix.Volume == 0.5f);

    // A destination change does.
    parameter.destination_mix_id = 2;
    CHECK(mix.Update(edges, parameter, effects, splitters, behaviour));
    CHECK(!edges.Connected(1, 0));
    CHECK(edges.Connected(1, 2));
}

// Derivation (C# MixInfo.Update, revision 1: IsSplitterSupported is false):
//   the destination is copied with no edge update, and the dirty flag is (destination changed).
//   First Update: Unused -> 0 is a change, returns true. Second Update: same destination, returns false.
//   IsLongSizePreDelaySupported is false (revision < 3). The edge matrix stays empty.
NXTEST(mix_info_update_without_splitters_rev1) {
    BehaviourInfo behaviour = MakeBehaviour(BehaviourInfo::Revision1);

    std::vector<int32_t> effectOrder(2, 0);
    MixInfo mix(Span<int32_t>(effectOrder), behaviour);
    CHECK(!mix.IsLongSizePreDelaySupported);

    std::vector<uint8_t> edgeWorkBuffer(static_cast<size_t>(EdgeMatrix::GetWorkBufferSize(3)), 0);
    EdgeMatrix edges;
    edges.Initialize(Span<uint8_t>(edgeWorkBuffer), 3);

    SplitterContext splitters;
    NoEffects effects;
    MixParameter parameter = MakeMix(1, 2, 0, Constants::UnusedSplitterId);

    CHECK(mix.Update(edges, parameter, effects, splitters, behaviour));
    CHECK(!mix.Update(edges, parameter, effects, splitters, behaviour));
    CHECK_EQ(mix.DestinationMixId, 0);
    CHECK_EQ(mix.DestinationSplitterId, Constants::UnusedSplitterId);
    CHECK(!edges.Connected(1, 0));
}

// Derivation (C# MixInfo.UpdateConnection with a destination splitter, revision 5):
//   splitter 0 has two destinations: id 0 -> mix 0 and id 1 -> mix 2 (splitter input built the same way as in splitter_update_version1_rev5,
//   with destination ids 0 and 2).
//   Update with destination_mix_id Unused and destination_splitter_id 0: the destinations are walked and edges (1,0) and (1,2) are added.
//   Returns true. DestinationSplitterId = 0, DestinationMixId = Unused, HasAnyDestination() = true.
//   Second Update with the same parameter returns true again, because the splitter still has HasNewConnection set (cleared only at the next
//   splitter update), so the connections are rebuilt.
NXTEST(mix_info_destination_splitter_connections_rev5) {
    BehaviourInfo behaviour = MakeBehaviour(BehaviourInfo::Revision5);

    SplitterContext splitters;
    std::vector<uint8_t> workBuffer;
    std::vector<BiquadFilterState> biquadStates;
    REQUIRE(InitSplitter(splitters, behaviour, 1, 2, workBuffer, biquadStates));

    std::vector<uint8_t> destinations;
    Append(destinations, MakeDestV1(0, 0, 0.5f, true));
    Append(destinations, MakeDestV1(1, 2, 0.5f, true));
    std::vector<uint8_t> input = BuildInput(MakeHeader(1, 2), MakeSplitter(0, 48000, 2), {0, 1}, destinations);
    SequenceReader reader{Span<const uint8_t>(input)};
    REQUIRE(splitters.Update(reader));

    std::vector<int32_t> effectOrder(2, 0);
    MixInfo mix(Span<int32_t>(effectOrder), behaviour);

    std::vector<uint8_t> edgeWorkBuffer(static_cast<size_t>(EdgeMatrix::GetWorkBufferSize(3)), 0);
    EdgeMatrix edges;
    edges.Initialize(Span<uint8_t>(edgeWorkBuffer), 3);
    NoEffects effects;

    MixParameter parameter = MakeMix(1, 2, Constants::UnusedMixId, 0);

    CHECK(mix.Update(edges, parameter, effects, splitters, behaviour));
    CHECK_EQ(mix.DestinationSplitterId, 0u);
    CHECK_EQ(mix.DestinationMixId, Constants::UnusedMixId);
    CHECK(mix.HasAnyDestination());
    CHECK(edges.Connected(1, 0));
    CHECK(edges.Connected(1, 2));
    CHECK(!edges.Connected(1, 1));

    CHECK(mix.Update(edges, parameter, effects, splitters, behaviour));
    CHECK(edges.Connected(1, 0));
    CHECK(edges.Connected(1, 2));
}

// Derivation (C# MixContext.Sort(SplitterContext) with splitters, edge matrix sort via NodeStates):
//   mixes: 0 = final (buffers 2, no destination), 1 = sub mix (buffers 3, destination 0), 2 = sub mix (buffers 4, destination 0).
//   Edges: (1,0), (2,0). NodeStates.Sort is a depth-first post-order: node 0 finishes first (result [0]), then node 1 ([0,1]), then node 2
//   ([0,1,2]). The sorted order is the reverse of the result: [2, 1, 0]. Sources come first, the final mix last.
//   Buffer offsets follow index order, not sorted order: mix 0 -> 0, mix 1 -> 0 + 2 = 2, mix 2 -> 2 + 3 = 5.
NXTEST(mix_context_edge_sort_puts_final_mix_last) {
    BehaviourInfo behaviour = MakeBehaviour(BehaviourInfo::Revision5);

    SplitterContext splitters;
    std::vector<uint8_t> splitterWork;
    std::vector<BiquadFilterState> biquadStates;
    REQUIRE(InitSplitter(splitters, behaviour, 1, 2, splitterWork, biquadStates));

    std::vector<std::vector<int32_t>> effectOrders(3, std::vector<int32_t>(2, 0));
    std::vector<MixInfo> mixes;
    for (auto& order : effectOrders) {
        mixes.emplace_back(Span<int32_t>(order), behaviour);
    }

    std::vector<uint8_t> nodeStatesWork(static_cast<size_t>(NodeStates::GetWorkBufferSize(3)), 0);
    std::vector<uint8_t> edgeWork(static_cast<size_t>(EdgeMatrix::GetWorkBufferSize(3)), 0);
    std::vector<int32_t> sorted(3, 0);

    MixContext context;
    context.Initialize(Span<int32_t>(sorted), Span<MixInfo>(mixes), Span<uint8_t>(nodeStatesWork), Span<uint8_t>(edgeWork));

    NoEffects effects;
    MixParameter finalMix = MakeMix(0, 2, Constants::UnusedMixId, Constants::UnusedSplitterId);
    MixParameter subA = MakeMix(1, 3, 0, Constants::UnusedSplitterId);
    MixParameter subB = MakeMix(2, 4, 0, Constants::UnusedSplitterId);
    mixes[0].Update(context.EdgeMatrix, finalMix, effects, splitters, behaviour);
    mixes[1].Update(context.EdgeMatrix, subA, effects, splitters, behaviour);
    mixes[2].Update(context.EdgeMatrix, subB, effects, splitters, behaviour);

    CHECK(context.Sort(splitters));

    CHECK_EQ(context.GetSortedState(0).MixId, 2);
    CHECK_EQ(context.GetSortedState(1).MixId, 1);
    CHECK_EQ(context.GetSortedState(2).MixId, 0);
    CHECK_EQ(context.GetFinalState().MixId, 0);

    CHECK_EQ(context.GetState(0).BufferOffset, 0u);
    CHECK_EQ(context.GetState(1).BufferOffset, 2u);
    CHECK_EQ(context.GetState(2).BufferOffset, 5u);

    CHECK(context.EdgeMatrix.Connected(1, 0));
    CHECK(context.EdgeMatrix.Connected(2, 0));
    CHECK(!context.EdgeMatrix.Connected(0, 1));
    CHECK_EQ(context.GetCount(), 3u);
}

// Derivation (C# NodeStates.Sort returns false on a loop and resets; MixContext.Sort then leaves the sorted indices and the buffer offsets
// as they were):
//   mixes: 1 -> 2 and 2 -> 1 (a two-node cycle), 0 is the final mix. NodeStates: node 1 is discovered, node 2 is discovered from it, and
//   then node 1 is reached again while it is still discovered: the loop is found and Sort returns false.
//   The sorted indices stay the identity from Initialize, and UpdateMixBufferOffset is not called, so every BufferOffset stays 0.
NXTEST(mix_context_cycle_is_rejected) {
    BehaviourInfo behaviour = MakeBehaviour(BehaviourInfo::Revision5);

    SplitterContext splitters;
    std::vector<uint8_t> splitterWork;
    std::vector<BiquadFilterState> biquadStates;
    REQUIRE(InitSplitter(splitters, behaviour, 1, 2, splitterWork, biquadStates));

    std::vector<std::vector<int32_t>> effectOrders(3, std::vector<int32_t>(2, 0));
    std::vector<MixInfo> mixes;
    for (auto& order : effectOrders) {
        mixes.emplace_back(Span<int32_t>(order), behaviour);
    }

    std::vector<uint8_t> nodeStatesWork(static_cast<size_t>(NodeStates::GetWorkBufferSize(3)), 0);
    std::vector<uint8_t> edgeWork(static_cast<size_t>(EdgeMatrix::GetWorkBufferSize(3)), 0);
    std::vector<int32_t> sorted(3, 0);

    MixContext context;
    context.Initialize(Span<int32_t>(sorted), Span<MixInfo>(mixes), Span<uint8_t>(nodeStatesWork), Span<uint8_t>(edgeWork));

    NoEffects effects;
    mixes[0].Update(context.EdgeMatrix, MakeMix(0, 2, Constants::UnusedMixId, Constants::UnusedSplitterId), effects, splitters, behaviour);
    mixes[1].Update(context.EdgeMatrix, MakeMix(1, 3, 2, Constants::UnusedSplitterId), effects, splitters, behaviour);
    mixes[2].Update(context.EdgeMatrix, MakeMix(2, 4, 1, Constants::UnusedSplitterId), effects, splitters, behaviour);

    CHECK(!context.Sort(splitters));

    for (int32_t i = 0; i < 3; i++) {
        CHECK_EQ(context.GetSortedState(i).MixId, i);
        CHECK_EQ(context.GetState(i).BufferOffset, 0u);
    }
}

// Derivation (C# MixContext.Sort(SplitterContext) with UsingSplitter() false): no graph sort, the sorted indices stay the identity and the
// buffer offsets are set in index order: 0, 2 (after 2 buffers), 5 (after 3 more).
NXTEST(mix_context_sort_without_splitters_keeps_index_order) {
    BehaviourInfo behaviour = MakeBehaviour(BehaviourInfo::Revision5);

    SplitterContext splitters;  // not initialized: UsingSplitter() is false

    std::vector<std::vector<int32_t>> effectOrders(3, std::vector<int32_t>(2, 0));
    std::vector<MixInfo> mixes;
    for (auto& order : effectOrders) {
        mixes.emplace_back(Span<int32_t>(order), behaviour);
    }

    std::vector<uint8_t> nodeStatesWork(static_cast<size_t>(NodeStates::GetWorkBufferSize(3)), 0);
    std::vector<uint8_t> edgeWork(static_cast<size_t>(EdgeMatrix::GetWorkBufferSize(3)), 0);
    std::vector<int32_t> sorted(3, 0);

    MixContext context;
    context.Initialize(Span<int32_t>(sorted), Span<MixInfo>(mixes), Span<uint8_t>(nodeStatesWork), Span<uint8_t>(edgeWork));

    // Set directly (no Update call): the mix id and the buffer count are what Sort reads.
    for (int32_t i = 0; i < 3; i++) {
        mixes[static_cast<size_t>(i)].MixId = i;
    }
    mixes[0].BufferCount = 2;
    mixes[1].BufferCount = 3;
    mixes[2].BufferCount = 4;

    CHECK(context.Sort(splitters));

    for (int32_t i = 0; i < 3; i++) {
        CHECK_EQ(context.GetSortedState(i).MixId, i);
    }
    CHECK_EQ(context.GetState(0).BufferOffset, 0u);
    CHECK_EQ(context.GetState(1).BufferOffset, 2u);
    CHECK_EQ(context.GetState(2).BufferOffset, 5u);
}

// Derivation (C# MixContext.Sort() without splitters: UpdateDistancesFromFinalMix, then Array.Sort with a descending distance comparison),
// revision 1 so that MixInfo.Update only sets DestinationMixId:
//   chain 2 -> 1 -> 0 (final). Distances: mix 0 is the final mix, so it gets InvalidDistanceFromFinalMix (0x80000000). Mix 1 reaches the
//   final mix directly: distance 0. Mix 2 reaches mix 1, which already has distance 0: distance 1.
//   The comparison sorts the larger distance first, so the order is [0 (invalid, largest), 2 (1), 1 (0)] = mixes [0, 2, 1].
//   Array.Sort on 3 elements uses the sorting network (0,1), (0,2), (1,2) of ArraySortHelper, which gives the same order here.
//   Buffer offsets follow index order: 0, 2, 5.
NXTEST(mix_context_distance_sort_orders_by_distance) {
    BehaviourInfo behaviour = MakeBehaviour(BehaviourInfo::Revision1);

    SplitterContext splitters;  // not used by the distance sort

    std::vector<std::vector<int32_t>> effectOrders(3, std::vector<int32_t>(2, 0));
    std::vector<MixInfo> mixes;
    for (auto& order : effectOrders) {
        mixes.emplace_back(Span<int32_t>(order), behaviour);
    }

    std::vector<uint8_t> nodeStatesWork(static_cast<size_t>(NodeStates::GetWorkBufferSize(3)), 0);
    std::vector<uint8_t> edgeWork(static_cast<size_t>(EdgeMatrix::GetWorkBufferSize(3)), 0);
    std::vector<int32_t> sorted(3, 0);

    MixContext context;
    context.Initialize(Span<int32_t>(sorted), Span<MixInfo>(mixes), Span<uint8_t>(nodeStatesWork), Span<uint8_t>(edgeWork));

    NoEffects effects;
    mixes[0].Update(context.EdgeMatrix, MakeMix(0, 2, Constants::UnusedMixId, Constants::UnusedSplitterId), effects, splitters, behaviour);
    mixes[1].Update(context.EdgeMatrix, MakeMix(1, 3, 0, Constants::UnusedSplitterId), effects, splitters, behaviour);
    mixes[2].Update(context.EdgeMatrix, MakeMix(2, 4, 1, Constants::UnusedSplitterId), effects, splitters, behaviour);

    context.Sort();

    CHECK_EQ(context.GetSortedState(0).MixId, 0);
    CHECK_EQ(context.GetSortedState(1).MixId, 2);
    CHECK_EQ(context.GetSortedState(2).MixId, 1);

    CHECK_EQ(context.GetState(0).BufferOffset, 0u);
    CHECK_EQ(context.GetState(1).BufferOffset, 2u);
    CHECK_EQ(context.GetState(2).BufferOffset, 5u);

    CHECK_EQ(context.GetState(1).DistanceFromFinalMix, 0u);
    CHECK_EQ(context.GetState(2).DistanceFromFinalMix, 1u);
    CHECK_EQ(context.GetState(0).DistanceFromFinalMix, MixInfo::InvalidDistanceFromFinalMix);
}

// The render system context is a plain holder. Checked here only that it is constructible with its defaults.
NXTEST(renderer_system_context_defaults) {
    RendererSystemContext rendererContext;
    CHECK_EQ(rendererContext.SessionId, 0);
    CHECK_EQ(rendererContext.MixBufferCount, 0u);
    CHECK(rendererContext.BehaviourInfo == nullptr);
    CHECK(rendererContext.UpsamplerManager == nullptr);
}
