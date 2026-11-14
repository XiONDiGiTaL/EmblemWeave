// SPDX-License-Identifier: GPL-3.0-or-later
// NVN2 DLSS on the host (api_dlss.cpp: the entry points; api_dlss_replay.cpp: the evaluation on the render thread). The game's own DLSS
// module (subsdk0 "dlss", NGX) and the CUDA driver (subsdk1) never run: nvnCommandBufferEvaluateDlss records an EvaluateDlss command and
// the replay upscales with a host upscaler ([dlss] upscaler: the built-in temporal one by default, extras/taau.h).
//
// The builder and parameter objects keep the layout of Fortune's Weave's NVN 231.40 driver (sdk 0x659B70.. 0x659FE0, notes/dlss.md); the
// network object holds a header naming its host side.
#pragma once
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <mutex>
#include <string>
#include <vector>

#include "nvn_core.h"

namespace nxrt::nvn::be {

namespace gpu {
struct Frame;
}

struct DlssBuilderData {               // NVNdlssNetworkBuilder
    uint64_t device;                   // 0x00 (SetDefaults keeps it)
    uint64_t texture_pool;             // 0x08
    uint64_t memory;                   // 0x10 storage
    uint64_t memory_size;              // 0x18
    int32_t flags;                     // 0x20
    int32_t method;                    // 0x24 0 up to 2x, 1 3x, 2 1x (as the game picks them)
    int32_t preset;                    // 0x28
    int32_t out_w, out_h;              // 0x2C
    int32_t max_entries;               // 0x34 (1 after SetDefaults)
    int32_t ids[32];                   // 0x38 texture ids
    uint64_t ext;                      // 0xB8
};
static_assert(sizeof(DlssBuilderData) == 0xC0, "NVNdlssNetworkBuilder");

struct DlssParamsData {                // NVNdlssParameters
    uint64_t color, color_view;        // 0x00
    uint64_t output, output_view;      // 0x10
    uint64_t motion, motion_view;      // 0x20
    uint64_t depth, depth_view;        // 0x30
    uint64_t mask, mask_view;          // 0x40 bias-current-colour mask
    float jitter[2];                   // 0x50
    float mv_scale[2];                 // 0x58 (1, 1 after SetDefaults)
    float pre_exposure;                // 0x60 (1 after SetDefaults)
    uint32_t pad64;
    uint64_t exposure, exposure_view;  // 0x68
    uint8_t reset;                     // 0x78
    uint8_t pad79[3];
    int32_t eval_index;                // 0x7C
    int32_t in_size[2];                // 0x80 input subrectangle size
    int32_t color_base[2];             // 0x88
    int32_t depth_base[2];             // 0x90
    int32_t mv_base[2];                // 0x98
    int32_t mask_base[2];              // 0xA0
    int32_t out_base[2];               // 0xA8
    int32_t overlay[4];                // 0xB0 debug layer, full screen, show NaNs, jitter debug (-1 after SetDefaults)
};
static_assert(sizeof(DlssParamsData) == 0xC0, "NVNdlssParameters");

constexpr size_t kDlssNetworkSize = 0x258;   // what the 231.40 driver's Initialize clears

// NVN builder flags (the driver turns them into NGX feature flags: bit 0 clear = HDR input).
constexpr int kDlssFlagLdr = 0x1, kDlssFlagMvLowRes = 0x2, kDlssFlagMvJittered = 0x4, kDlssFlagDepthInverted = 0x8, kDlssFlagAutoExposure = 0x40;

// The host side of an initialized NVNdlssNetwork.
struct DlssNet : Obj {
    DlssNet() : Obj(Kind::None) {}
    ~DlssNet() override;
    uint32_t id = 0;
    int flags = 0, method = 0, preset = 0, max_entries = 1;
    int max_w = 0, max_h = 0;                     // output resolution at Initialize; Resize stays within it
    std::atomic<int> out_w{0}, out_h{0};          // current output resolution
    uint64_t texture_pool = 0;
    int32_t ids[32] = {};
    // render thread: the upscaler's state (api_dlss_replay.cpp), freed through free_gpu
    void* gpu = nullptr;
    void (*free_gpu)(void*) = nullptr;
};

// One nvnCommandBufferEvaluateDlss, held by the command list it was recorded in.
struct DlssEval : Obj {
    DlssEval() : Obj(Kind::None) {}
    Ref<DlssNet> net;
    Ref<Texture> color, output, motion, depth, mask;
    TextureViewInfo color_view, output_view, motion_view, depth_view, mask_view;
    bool has_color_view = false, has_output_view = false, has_motion_view = false, has_depth_view = false, has_mask_view = false;
    DlssParamsData p = {};
    int out_w = 0, out_h = 0;                     // the network's output resolution when it was recorded
};

// NGX's optimal settings for an output size and a method, as the console's DLSS module computes them (subsdk0 0x7D200): optimal, minimum
// and maximum render sizes.
struct DlssOptimal {
    int opt_w = 0, opt_h = 0, min_w = 0, min_h = 0, max_w = 0, max_h = 0;
};
DlssOptimal dlss_optimal(int out_w, int out_h, int method);
// The storage size GetStorageSize answers (the host keeps nothing in it).
constexpr size_t kDlssStorageSize = 0x10000;

// The names of the DLSS-family entry points api_dlss.cpp implements (tests).
std::vector<std::string> dlss_proc_names();

// Render thread (api_dlss_replay.cpp): runs one recorded evaluation; dlss_replay_shutdown frees every network's GPU state (device idle).
void dlss_replay(gpu::Frame& f, Obj* eval);
void dlss_replay_shutdown();
// The upscaler the last evaluation ran ("taau (built-in temporal)", "FSR 3", ...), nullptr before the first. Any thread (the F1 menu).
const char* dlss_upscaler_in_use();

}  // namespace nxrt::nvn::be
