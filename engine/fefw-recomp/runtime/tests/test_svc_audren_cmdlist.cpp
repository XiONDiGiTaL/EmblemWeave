// SPDX-License-Identifier: GPL-3.0-or-later
// CommandList / ICommand / IHardwareDevice: buffer addressing and in-order command execution.
#include "../src/services/audren/Dsp/Command/CommandList.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_cmdlist)

using namespace nxrt::audren;

namespace {
struct AddOne : ICommand {
    int idx;
    std::vector<int>* order;
    int id;
    AddOne(int index, std::vector<int>* o, int i) : idx(index), order(o), id(i) { Enabled = true; }
    int32_t GetNodeId() const override { return id; }
    CommandType GetCommandType() const override { return CommandType::Volume; }
    uint32_t GetEstimatedProcessingTime() const override { return 0; }
    void Process(CommandList& l) override {
        for (float& f : l.GetBuffer(idx)) f += 1.0f;
        order->push_back(id);
    }
};
struct Dev : IHardwareDevice {
    uint32_t channels = 2;
    void SetVolume(float) override {}
    float GetVolume() override { return 1.0f; }
    uint32_t GetSampleRate() override { return 48000; }
    uint32_t GetChannelCount() override { return channels; }
    void AppendBuffer(Span<const int16_t>, uint32_t) override {}
};
}  // namespace

NXTEST(command_list_runs_enabled_commands_in_order) {
    std::vector<float> mix(4 * 8, 0.0f);   // 4 buffers of 8 samples
    CommandList list(Span<float>(mix), 8, 48000, 3, 1);
    CHECK_EQ(list.BufferCount, 4u);
    std::vector<int> order;
    list.AddCommand(std::make_unique<AddOne>(1, &order, 10));
    list.AddCommand(std::make_unique<AddOne>(1, &order, 11));
    auto off = std::make_unique<AddOne>(2, &order, 12);
    off->Enabled = false;
    list.AddCommand(std::move(off));
    list.AddCommand(std::make_unique<AddOne>(3, &order, 13));
    Dev dev;
    list.Process(&dev);
    CHECK_EQ(order.size(), 3u);
    CHECK_EQ(order[0], 10);
    CHECK_EQ(order[1], 11);
    CHECK_EQ(order[2], 13);
    CHECK_EQ(mix[8], 2.0f);    // buffer 1
    CHECK_EQ(mix[15], 2.0f);
    CHECK_EQ(mix[16], 0.0f);   // buffer 2 untouched (command disabled)
    CHECK_EQ(mix[24], 1.0f);   // buffer 3
    CHECK(list.EndTime >= list.StartTime);
    CHECK(list.OutputDevice == &dev);
}

NXTEST(command_list_buffer_helpers) {
    std::vector<float> mix(3 * 4, 5.0f);
    CommandList list(Span<float>(mix), 4, 48000, 2, 1);
    list.ClearBuffer(1);
    CHECK_EQ(mix[3], 5.0f);
    CHECK_EQ(mix[4], 0.0f);
    CHECK_EQ(mix[7], 0.0f);
    CHECK_EQ(mix[8], 5.0f);
    list.GetBuffer(0).Fill(2.0f);
    list.CopyBuffer(2, 0);
    CHECK_EQ(mix[11], 2.0f);
    CHECK(list.GetBuffer(3).IsEmpty());   // out of range: empty span
    list.ClearBuffers();
    CHECK_EQ(mix[0], 0.0f);
    CHECK_EQ(mix[11], 0.0f);
    Dev d;
    d.channels = 6;
    CHECK(!d.NeedDownmixing());
    d.channels = 2;
    CHECK(d.NeedDownmixing());
}
