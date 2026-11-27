// SPDX-License-Identifier: GPL-3.0-or-later
// Haptics (src/gfx/haptic.cpp): the pure parts. No pad, no library.
#include "test_framework.h"

#include <cstdint>

#include "nxrt/haptic.h"

NXTEST_GROUP(haptic)

namespace haptic = nxrt::gfx::haptic;

// The shape of the two descriptors: input and output items under a report id. Over Bluetooth a DualSense has the output
// reports 0x31 to 0x39, on a cable only 0x02.
NXTEST(dualsense_bluetooth_is_told_from_the_output_reports) {
    const uint8_t usb[] = {0x05, 0x01, 0x09, 0x05, 0xA1, 0x01, 0x85, 0x01, 0x75, 0x08, 0x95, 0x3F, 0x81, 0x02,
                           0x85, 0x02, 0x95, 0x2F, 0x91, 0x02, 0x85, 0x31, 0x95, 0x2F, 0xB1, 0x02, 0xC0};
    const uint8_t bt[] = {0x05, 0x01, 0x09, 0x05, 0xA1, 0x01, 0x85, 0x01, 0x75, 0x08, 0x95, 0x07, 0x81, 0x02,
                          0x85, 0x31, 0x95, 0x4D, 0x81, 0x02, 0x85, 0x31, 0x95, 0x4D, 0x91, 0x02, 0xC0};
    CHECK(!haptic::dualsense_descriptor_is_bluetooth(usb, sizeof(usb)));   // 0x31 as a feature report is not enough
    CHECK(haptic::dualsense_descriptor_is_bluetooth(bt, sizeof(bt)));
}

NXTEST(dualsense_descriptor_edges) {
    CHECK(!haptic::dualsense_descriptor_is_bluetooth(nullptr, 0));
    const uint8_t cut[] = {0x85, 0x31, 0x91};                 // the Output item's value byte is missing
    CHECK(!haptic::dualsense_descriptor_is_bluetooth(cut, sizeof(cut)));
    const uint8_t input_only[] = {0x85, 0x31, 0x81, 0x02};
    CHECK(!haptic::dualsense_descriptor_is_bluetooth(input_only, sizeof(input_only)));
    const uint8_t long_item[] = {0xFE, 0x02, 0x00, 0x91, 0x31, 0x85, 0x31, 0x91, 0x02};
    CHECK(haptic::dualsense_descriptor_is_bluetooth(long_item, sizeof(long_item)));
    const uint8_t cut_long[] = {0xFE};
    CHECK(!haptic::dualsense_descriptor_is_bluetooth(cut_long, sizeof(cut_long)));
}
