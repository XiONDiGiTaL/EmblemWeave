// SPDX-License-Identifier: GPL-3.0-or-later
// HID shared memory (S3c): the 0x40000-byte block the guest reads controller state from, and the sampling hooks of the hid
// service. Offsets and sizes are Ryujinx's (refs ryubing src/Ryujinx.HLE/HOS/Services/Hid/Types/SharedMemory/SharedMemory.cs,
// Common/RingLifo.cs, Common/AtomicStorage.cs, Npad/*.cs; MIT) and were cross-checked against ScarletNX
// src/nx/hle/services/hid_layout.h (GPL-3.0-or-later, Eden layout). Where the two disagree, the Ryujinx value is used and the
// disagreement is written next to the constant. Switchbrew "HID services" is the reference for the field meanings.
#pragma once
#include <cstdint>

namespace nxrt::services {

namespace hidshm {

constexpr uint64_t kSize = 0x40000;
// Sections (Ryujinx SharedMemory.cs FieldOffset values).
constexpr uint64_t kDebugPad = 0x0;
constexpr uint64_t kTouchScreen = 0x400;
constexpr uint64_t kMouse = 0x3400;
constexpr uint64_t kKeyboard = 0x3800;
constexpr uint64_t kNpads = 0x9A00;             // ten entries of kNpadEntrySize; ends at 0x3BA00
constexpr uint64_t kDebugMouse = 0x3DC00;      // ScarletNX names 0x3BA00..0x3DC00 "gesture" and "console six axis"; Ryujinx leaves it unmodelled
constexpr uint64_t kNpadCondition = 0x3E200;
constexpr uint64_t kNpadEntrySize = 0x5000;
constexpr uint32_t kNpadCount = 10;
// Npad entry index: 0..7 = players 1..8, 8 = handheld, 9 = other (Ryujinx PlayerIndex: Handheld 8, Unknown 9). NOTE: this is
// the shared-memory index; input_source.h numbers the slots 8 = other, 9 = handheld.

// Ring lifo (Ryujinx RingLifo): a 0x20-byte header, then 17 entries of (u64 sampling number, state). The header is
// {unused u64, buffer_count u64 (= 17), index u64 (latest written entry), count u64}. Ryujinx's Write caps count at 16
// (MaxEntries - 1); ScarletNX says 17. The Ryujinx cap is used.
constexpr uint32_t kRingEntries = 17;
constexpr uint64_t kRingMaxCount = 16;

// Npad entry (Ryujinx NpadInternalState), offsets within one entry. Lifos are kRingHeader + 17 entries each.
constexpr uint64_t kStyleSet = 0x0;                   // u32 NpadStyleTag of the pad (0 when not connected)
constexpr uint64_t kJoyAssignmentMode = 0x4;          // u32: 0 = Dual, 1 = Single
constexpr uint64_t kFullKeyColor = 0x8;               // u32 attribute, u32 body, u32 buttons
constexpr uint64_t kJoyColor = 0x14;                  // u32 attribute, u32 left body, left buttons, right body, right buttons
constexpr uint64_t kFullKeyLifo = 0x28;               // NpadCommonState ring, 0x350 bytes
constexpr uint64_t kHandheldLifo = 0x378;
constexpr uint64_t kJoyDualLifo = 0x6C8;
constexpr uint64_t kJoyLeftLifo = 0xA18;
constexpr uint64_t kJoyRightLifo = 0xD68;
constexpr uint64_t kPalmaLifo = 0x10B8;
constexpr uint64_t kSystemExtLifo = 0x1408;
constexpr uint64_t kFullKeySixAxisLifo = 0x1758;      // SixAxisSensorState ring, 0x708 bytes
constexpr uint64_t kHandheldSixAxisLifo = 0x1E60;
constexpr uint64_t kJoyDualLeftSixAxisLifo = 0x2568;
constexpr uint64_t kJoyDualRightSixAxisLifo = 0x2C70;
constexpr uint64_t kJoyLeftSixAxisLifo = 0x3378;
constexpr uint64_t kJoyRightSixAxisLifo = 0x3A80;
constexpr uint64_t kDeviceType = 0x4188;              // u32 DeviceType bits
constexpr uint64_t kSystemProperties = 0x4190;        // u64 NpadSystemProperties
constexpr uint64_t kSystemButtonProperties = 0x4198;  // u32
constexpr uint64_t kBatteryJoyDual = 0x419C;          // u32 NpadBatteryLevel (4 = 100 percent)
constexpr uint64_t kBatteryJoyLeft = 0x41A0;
constexpr uint64_t kBatteryJoyRight = 0x41A4;
constexpr uint64_t kFooterUiAttribute = 0x41A8;
constexpr uint64_t kFooterUiType = 0x41AC;
// Ryujinx's NpadInternalState puts the GC trigger ring after a 123-byte reserved block, which C# natural alignment rounds to
// 0x4230; ScarletNX (Eden) puts it at 0x4228 (reserved 120 bytes). Not written by this service, so the choice has no effect here.

// Ring lifo header (same for every ring).
constexpr uint64_t kRingBufferCount = 0x8;
constexpr uint64_t kRingIndex = 0x10;
constexpr uint64_t kRingCount = 0x18;
constexpr uint64_t kRingHeader = 0x20;

// NpadCommonState attributes (NpadAttribute) and style tags (NpadStyleTag).
constexpr uint32_t kAttrConnected = 1u << 0;
constexpr uint32_t kAttrWired = 1u << 1;
constexpr uint32_t kAttrLeftConnected = 1u << 2;
constexpr uint32_t kAttrRightConnected = 1u << 4;
constexpr uint32_t kTagFullKey = 1u << 0;
constexpr uint32_t kTagHandheld = 1u << 1;
constexpr uint32_t kTagJoyDual = 1u << 2;
constexpr uint32_t kTagJoyLeft = 1u << 3;
constexpr uint32_t kTagJoyRight = 1u << 4;

// Structs as they sit in memory (all fields naturally aligned; the static_asserts below pin the sizes).
struct NpadCommonState {
    uint64_t sampling;
    uint64_t buttons;
    int32_t lx, ly;       // analog stick L
    int32_t rx, ry;       // analog stick R
    uint32_t attributes;
    uint32_t reserved;
};
struct SixAxisState {
    uint64_t delta_time;  // ns since the previous sample
    uint64_t sampling;
    float accel[3];
    float gyro[3];
    float angle[3];
    float direction[9];
    uint32_t attributes;  // bit 0 connected, bit 1 interpolated
    uint32_t reserved;
};
struct TouchState {
    uint64_t delta_time;
    uint32_t attributes;
    uint32_t finger;
    uint32_t x, y;
    uint32_t diameter_x, diameter_y;
    uint32_t rotation_angle;
    uint32_t reserved;
};
struct TouchScreenState {
    uint64_t sampling;
    int32_t count;
    int32_t reserved;
    TouchState touches[16];
};
struct MouseState {
    uint64_t sampling;
    int32_t x, y, dx, dy, wheel_dx, wheel_dy;
    uint32_t buttons;
    uint32_t attributes;  // bit 1 connected
};
struct KeyboardState {
    uint64_t sampling;
    uint64_t modifiers;
    uint64_t keys[4];     // 256 bits, one per HID usage id
};
struct DebugPadState {
    uint64_t sampling;
    uint32_t attributes;
    uint32_t buttons;
    int32_t rx, ry, lx, ly;
};

// One ring: header plus 17 entries of (sampling number, state).
template <class T>
struct Ring {
    uint64_t unused;
    uint64_t buffer_count;
    uint64_t index;
    uint64_t count;
    struct Entry {
        uint64_t sampling;
        T state;
    } entries[kRingEntries];
};

static_assert(sizeof(NpadCommonState) == 0x28, "NpadCommonState");
static_assert(sizeof(SixAxisState) == 0x60, "SixAxisState");
static_assert(sizeof(TouchState) == 0x28, "TouchState");
static_assert(sizeof(TouchScreenState) == 0x290, "TouchScreenState");
static_assert(sizeof(MouseState) == 0x28, "MouseState");
static_assert(sizeof(KeyboardState) == 0x30, "KeyboardState");
static_assert(sizeof(DebugPadState) == 0x20, "DebugPadState");
static_assert(sizeof(Ring<NpadCommonState>) == 0x350, "npad lifo");
static_assert(sizeof(Ring<SixAxisState>) == 0x708, "six-axis lifo");
static_assert(sizeof(Ring<TouchScreenState>) == 0x2C38, "touch lifo");
static_assert(sizeof(Ring<MouseState>) == 0x350, "mouse lifo");
static_assert(sizeof(Ring<KeyboardState>) == 0x3D8, "keyboard lifo");
static_assert(sizeof(Ring<DebugPadState>) == 0x2C8, "debug pad lifo");
static_assert(kNpads + kNpadCount * kNpadEntrySize == 0x3BA00, "npad block");
static_assert(kKeyboard + sizeof(Ring<KeyboardState>) <= kDebugMouse, "keyboard ring fits before the debug mouse");

}  // namespace hidshm

// Runs one sampling step now: reads the input source (or the default pads) and appends one entry to every lifo that is live.
// The hid service's thread calls this every 5 ms while an applet resource exists; tests call it directly.
void hid_sample_once();
// Turns the background thread's sampling on or off (default on). Tests switch it off so that their sampling is deterministic.
void hid_set_background_sampling(bool enabled);

}  // namespace nxrt::services
