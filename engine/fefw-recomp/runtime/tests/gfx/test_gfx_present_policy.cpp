// SPDX-License-Identifier: GPL-3.0-or-later
// The presentation rule alone, without a GPU or a window: an Intel GPU that is not discrete gets mailbox (else FIFO) instead of the
// immediate path that can reset its Windows driver (issue #72); [gfx] present_guard can force that guard on or off, and a hand-written
// [gfx] present_mode = immediate still wins.
#include "nxrt/gfx.h"
#include "test_framework.h"
NXTEST_GROUP(gfx_present_policy)

using namespace nxrt::gfx;

namespace {
constexpr uint32_t kIntel = 0x8086;
constexpr uint32_t kNvidia = 0x10de;
const std::vector<VkPresentModeKHR> kEveryMode = {VK_PRESENT_MODE_FIFO_KHR, VK_PRESENT_MODE_FIFO_RELAXED_KHR, VK_PRESENT_MODE_MAILBOX_KHR,
                                                  VK_PRESENT_MODE_IMMEDIATE_KHR};
const std::vector<VkPresentModeKHR> kNoMailbox = {VK_PRESENT_MODE_FIFO_KHR, VK_PRESENT_MODE_IMMEDIATE_KHR};
const std::vector<VkPresentModeKHR> kFifoOnly = {VK_PRESENT_MODE_FIFO_KHR};
const int kIntegrated = VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU;
const int kDiscrete = VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU;

bool auto_guards(uint32_t vendor, int type) { return present_guard_applies(PresentGuard::Auto, vendor, type); }
}  // namespace

NXTEST(intel_integrated_immediate_becomes_mailbox) {
    CHECK(choose_present_mode(PresentMode::Immediate, false, auto_guards(kIntel, kIntegrated), kEveryMode) == PresentMode::Mailbox);
}

NXTEST(intel_integrated_immediate_without_mailbox_becomes_fifo) {
    CHECK(choose_present_mode(PresentMode::Immediate, false, auto_guards(kIntel, kIntegrated), kNoMailbox) == PresentMode::Fifo);
    CHECK(choose_present_mode(PresentMode::Immediate, false, auto_guards(kIntel, kIntegrated), kFifoOnly) == PresentMode::Fifo);
}

NXTEST(an_intel_gpu_that_reports_no_device_type_is_guarded_too) {
    CHECK(choose_present_mode(PresentMode::Immediate, false, auto_guards(kIntel, VK_PHYSICAL_DEVICE_TYPE_OTHER), kEveryMode) ==
          PresentMode::Mailbox);
}

NXTEST(a_hand_written_immediate_still_wins_on_intel) {
    CHECK(choose_present_mode(PresentMode::Immediate, true, auto_guards(kIntel, kIntegrated), kEveryMode) == PresentMode::Immediate);
}

NXTEST(intel_discrete_and_other_vendors_keep_immediate) {
    CHECK(choose_present_mode(PresentMode::Immediate, false, auto_guards(kIntel, kDiscrete), kEveryMode) == PresentMode::Immediate);
    CHECK(choose_present_mode(PresentMode::Immediate, false, auto_guards(kNvidia, kDiscrete), kEveryMode) == PresentMode::Immediate);
    CHECK(choose_present_mode(PresentMode::Immediate, false, auto_guards(kNvidia, kIntegrated), kEveryMode) == PresentMode::Immediate);
}

NXTEST(mailbox_and_fifo_relaxed_keep_their_existing_fallbacks) {
    CHECK(choose_present_mode(PresentMode::Mailbox, false, false, kEveryMode) == PresentMode::Mailbox);
    CHECK(choose_present_mode(PresentMode::Mailbox, false, false, kNoMailbox) == PresentMode::Immediate);
    CHECK(choose_present_mode(PresentMode::FifoRelaxed, false, false, kFifoOnly) == PresentMode::Fifo);
    CHECK(choose_present_mode(PresentMode::Fifo, false, true, kEveryMode) == PresentMode::Fifo);
}

NXTEST(the_guard_is_automatic_for_intel_gpus_that_are_not_discrete) {
    CHECK(auto_guards(kIntel, kIntegrated));
    CHECK(auto_guards(kIntel, VK_PHYSICAL_DEVICE_TYPE_OTHER));
    CHECK(!auto_guards(kIntel, kDiscrete));
    CHECK(!auto_guards(kNvidia, kIntegrated));
}

NXTEST(the_guard_can_be_forced_on_every_gpu_or_off) {
    // On: an Intel Arc (reported as discrete) or a driver that names the wrong type still gets the safe path.
    CHECK(present_guard_applies(PresentGuard::On, kIntel, kDiscrete));
    CHECK(present_guard_applies(PresentGuard::On, kNvidia, kDiscrete));
    CHECK(choose_present_mode(PresentMode::Immediate, false, true, kEveryMode) == PresentMode::Mailbox);
    CHECK(choose_present_mode(PresentMode::Immediate, false, true, kNoMailbox) == PresentMode::Fifo);
    // Off: immediate stays immediate even on an Intel GPU that shares system memory.
    CHECK(!present_guard_applies(PresentGuard::Off, kIntel, kIntegrated));
    CHECK(choose_present_mode(PresentMode::Immediate, false, false, kEveryMode) == PresentMode::Immediate);
}

NXTEST(present_guard_values_parse) {
    CHECK(parse_present_guard("auto") == PresentGuard::Auto);
    CHECK(parse_present_guard("") == PresentGuard::Auto);
    CHECK(parse_present_guard("nonsense") == PresentGuard::Auto);
    CHECK(parse_present_guard("on") == PresentGuard::On);
    CHECK(parse_present_guard("1") == PresentGuard::On);
    CHECK(parse_present_guard("OFF") == PresentGuard::Off);
    CHECK(parse_present_guard("0") == PresentGuard::Off);
}
