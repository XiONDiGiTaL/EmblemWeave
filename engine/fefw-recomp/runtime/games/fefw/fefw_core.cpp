// SPDX-License-Identifier: GPL-3.0-or-later
// Fire Emblem: Fortune's Weave, the core part of the game module (fefw_game.h): the data part plus the title facts. Needs the runtime
// library, not the graphics one.
#include "fefw_game.h"

#include <zstd.h>

#include <algorithm>
#include <array>
#include <cstdint>
#include <cstring>

#include "nxrt/services.h"

namespace nxrt::games::fefw {

namespace {

// A save slot that the game can open. The final zstd frame is validated here before the game sees the bytes: its own
// zstd caller does not check ZSTD_isError and would use a decoder error as a copy length (#77). The game already
// handles an absent Auto slot by starting without an autosave, so a bad slot is hidden from it rather than decoded.
constexpr uint32_t kSlotMagic = 0xCDEFBCDD;
constexpr uint8_t kZstdMagic[4] = {0x28, 0xB5, 0x2F, 0xFD};
constexpr size_t kSlotHeader = 0x124;
constexpr uint64_t kMaxSavePayloadBytes = 64ull << 20;

uint32_t slot_u32(const unsigned char* p) {
    uint32_t v;
    std::memcpy(&v, p, sizeof v);
    return v;
}

uint32_t fnv1a32(const unsigned char* p, size_t n) {
    uint32_t h = 0x811C9DC5u;
    for (size_t i = 0; i < n; ++i) h = (h ^ p[i]) * 0x01000193u;
    return h;
}

bool zstd_frame_decodes(const unsigned char* frame, size_t size, uint32_t expected_fnv) {
    if (!frame || !size) return false;
    ZSTD_DStream* d = ZSTD_createDStream();
    if (!d) return false;
    size_t r = ZSTD_initDStream(d);
    if (ZSTD_isError(r)) {
        ZSTD_freeDStream(d);
        return false;
    }
    ZSTD_inBuffer in{frame, size, 0};
    std::array<unsigned char, 64 * 1024> chunk{};
    uint64_t produced = 0;
    uint32_t fnv = 0x811C9DC5u;
    bool complete = false;
    while (in.pos < in.size) {
        ZSTD_outBuffer out{chunk.data(), chunk.size(), 0};
        r = ZSTD_decompressStream(d, &out, &in);
        if (ZSTD_isError(r)) break;
        produced += out.pos;
        for (size_t i = 0; i < out.pos; ++i) fnv = (fnv * 0x01000193u) ^ chunk[i];
        if (produced > kMaxSavePayloadBytes) break;   // the game's save buffer is 64 MiB
        if (!out.pos && !in.pos) break;                // no progress: refuse to spin
        if (!r) {
            complete = in.pos == in.size;
            break;
        }
    }
    ZSTD_freeDStream(d);
    return complete && fnv == expected_fnv;
}

// The account Auto slot is the file the game opens at boot and when it reloads the title. Its container is
// 0x124 bytes followed by one zstd frame; 0x108 is the frame length and 0x104 repeats the slot magic. Other save
// files (Global, device/cache saves) are not this format and are left alone.
bool auto_slot_readable(const char* label, const char* path, const unsigned char* data, size_t size) {
    if (!label || !path || !data) return false;
    if (std::strncmp(label, "save:account/", 13) != 0 || std::strcmp(path, "/000/Auto") != 0) return true;
    if (size < kSlotHeader || slot_u32(data) != kSlotMagic || slot_u32(data + 0x104) != kSlotMagic) return false;
    const uint32_t header_window = slot_u32(data + 4);
    if (!header_window || header_window > size - 0x1F ||
        fnv1a32(data + 0x1F, header_window) != slot_u32(data + 8)) return false;
    const size_t frame_len = slot_u32(data + 0x108);
    if (!frame_len) return false;
    const unsigned char* end = data + size;
    const unsigned char* at = std::search(data + kSlotHeader, end, kZstdMagic, kZstdMagic + sizeof kZstdMagic);
    if (at == end) return false;
    const size_t zstd_at = static_cast<size_t>(at - data);
    if (zstd_at > size || frame_len > size - zstd_at) return false;
    if (ZSTD_findFrameCompressedSize(at, frame_len) != frame_len) return false;
    return zstd_frame_decodes(at, frame_len, slot_u32(data + 0x10C));
}

// main.npdm of update 131072: main thread 1 MiB / priority 44 / core 0, SystemResourceSize 64 MiB (the SDK's VAMM is on), ThreadInfo
// priorities 28..59 on cores 0..5. Save sizes are placeholders until control.nacp is known (the boot log shows what the game asks for).
void title_defaults(services::TitleProfile& p) {
    p.program_id = data_info().program_id;
    p.name = data_info().title;
    p.display_version = data_info().version;
    p.supported_languages = data_info().languages;
    p.account_save_size = 0x4000000;
    p.account_save_journal = 0x4000000;
    p.device_save_size = 0x4000000;
    p.device_save_journal = 0x4000000;
    p.main_thread_stack = 0x100000;
    p.main_thread_priority = 44;
    p.main_thread_core = 0;
    // main.npdm says 0x4000000, which turns on the SDK's VAMM: the game then reserves 36 GiB (after a 62 GiB probe that maps all free
    // memory once) in the alias region, 8 GiB here. 0 makes the SDK take its other, equally supported path: SetMemoryHeapSize(free memory)
    // + AllocateMemoryBlock (main 0x651EBC), i.e. the runtime's heap section, which NVN pools import from. notes/boot.md, VAMM.
    p.system_resource_size = 0;
    p.firmware = "23.0.0";                     // built with SDK 22.2.0; Eden-NS2, which runs the game, reports 23.0.0
    p.user_nickname = "Player";
    p.user_uid_hi = 0x0000000000000001ull;
    p.user_uid_lo = 0x4645465752636d70ull;     // "FEFWRcmp"
}

}  // namespace

const game::Info& core_info() {
    static const game::Info info = [] {
        game::Info g = data_info();
        g.title_defaults = &title_defaults;
        g.save_guard_path = "/000/Auto";
        g.save_file_readable = &auto_slot_readable;
        return g;
    }();
    return info;
}

}  // namespace nxrt::games::fefw

void nxrt_register_game_fefw_core() { nxrt::game::set_current(&nxrt::games::fefw::core_info()); }
