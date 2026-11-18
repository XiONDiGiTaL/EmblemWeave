// SPDX-License-Identifier: GPL-3.0-or-later
#include "extras/hdr_nvn.h"

#include <cmath>
#include <mutex>
#include <map>
#include <set>

#include "nxrt/extras.h"
#include "nxrt/log.h"
#include "extras/fsr1_spatial.h"
#include "extras/nis_spatial.h"
#include "extras/hdr_tonemap_patch.h"
#include "extras/extras_shaders.inc"

namespace nxrt::extras {
namespace {
constexpr int kFlagDisplay = 0x1, kFlagCompressible = 0x8;   // NVN_TEXTURE_FLAGS_DISPLAY, NVN_TEXTURE_FLAGS_COMPRESSIBLE
}

VkFormat hdr_promote_target(VkFormat chosen, int flags, bool is_2d, uint32_t levels, uint32_t layers, uint32_t samples, bool nvn_srgb) {
    if (!nvn_srgb || chosen != VK_FORMAT_R8G8B8A8_SRGB || !is_2d || levels != 1 || layers != 1 || samples > 1) return VK_FORMAT_UNDEFINED;
    if (!(flags & (kFlagDisplay | kFlagCompressible))) return VK_FORMAT_UNDEFINED;
    if (!hdr_scene_active()) return VK_FORMAT_UNDEFINED;
    return VK_FORMAT_R16G16B16A16_SFLOAT;
}

bool hdr_tonemap_variant(const std::vector<uint32_t>& words, const std::string& hash, std::vector<uint32_t>& out) {
    if (!hdr_scene_active()) return false;
    static std::mutex mu;
    static std::set<std::string> tried;
    // the curve tone map (Metroid Prime Remastered; small shaders), else the GT curve (Fire Emblem Fortune's Weave; small shaders),
    // else the LUT tone map (Pokopia's composite pass)
    const char* kind = "curve";
    bool ok = false;
    if (words.size() <= 16384) {
        ok = spirv_hdr_tonemap(words, out);
        const bool wide = settings().hdr.wide_gamut;
        if (!ok && spirv_hdr_tonemap_gt(words, hdr_state().peak_ratio, out, wide)) {
            ok = true;
            kind = wide ? "GT shoulder moved to peak, BT.2020 colours kept" : "GT shoulder moved to peak";
        }
    }
    if (!ok && spirv_hdr_tonemap_lut(words, hdr_state().peak_ratio, out)) {
        ok = true;
        kind = "LUT highlights extended";
    }
    std::lock_guard<std::mutex> lk(mu);
    if (tried.insert(hash).second && ok)
        NXLOG_I("hdr", "tone-map shader %s: HDR variant made (%s, peak ratio %.2f)", hash.c_str(), kind, static_cast<double>(hdr_state().peak_ratio));
    return ok;
}

bool hdr_is_gt_tonemap(const std::vector<uint32_t>& words) {
    std::vector<uint32_t> out;
    return words.size() <= 16384 && spirv_hdr_tonemap_gt(words, 2.0f, out);
}

const std::vector<uint32_t>& hdr_gt_grade_words() {
    static const std::vector<uint32_t> w(kFefwGtGradeFrag, kFefwGtGradeFrag + sizeof kFefwGtGradeFrag / sizeof kFefwGtGradeFrag[0]);
    return w;
}

const std::vector<uint32_t>& sharpen_words() {
    static const std::vector<uint32_t> w(kFefwSharpenFrag, kFefwSharpenFrag + sizeof kFefwSharpenFrag / sizeof kFefwSharpenFrag[0]);
    return w;
}

bool hdr_gt_grade_push(float out[12]) {
    const HdrSettings h = settings().hdr;
    const bool neutral = h.grade_exposure == 0 && h.grade_contrast == 1 && h.grade_shadows == 1 && h.grade_highlights == 1 &&
                         h.grade_saturation == 1 && h.highlight_saturation == 0 && h.hue_preserve == 0;
    out[0] = hdr_state().peak_ratio;
    out[1] = static_cast<float>(std::exp2(h.grade_exposure));
    out[2] = static_cast<float>(h.grade_contrast);
    out[3] = static_cast<float>(h.grade_saturation);
    out[4] = static_cast<float>(h.grade_shadows);
    out[5] = static_cast<float>(h.grade_highlights);
    out[6] = static_cast<float>(h.highlight_saturation);
    out[7] = h.wide_gamut ? 1.0f : 0.0f;
    out[8] = h.compare ? 1.0f : 0.0f;   // split screen: the left half as the game draws it
    out[9] = static_cast<float>(h.hue_preserve);
    out[10] = out[11] = 0.0f;
    return !neutral || h.compare;
}

bool hdr_is_tonemap(const std::vector<uint32_t>& words, const std::string& hash) {
    static std::mutex mu;
    static std::map<std::string, bool> cache;
    {
        std::lock_guard<std::mutex> lk(mu);
        auto it = cache.find(hash);
        if (it != cache.end()) return it->second;
    }
    std::vector<uint32_t> scratch;
    // the curve, GT curve or LUT composite; the patch result is only used as a match test here
    bool ok = false;
    if (words.size() <= 16384) {
        ok = spirv_hdr_tonemap(words, scratch);
        if (!ok) ok = spirv_hdr_tonemap_gt(words, 2.0f, scratch);
    }
    if (!ok) {   // no size limit: Pokopia's composite shader is large (hdr_tonemap_variant matches it the same way)
        scratch.clear();
        ok = spirv_hdr_tonemap_lut(words, 2.0f, scratch);
    }
    std::lock_guard<std::mutex> lk(mu);
    cache[hash] = ok;
    return ok;
}

bool scene_upscale_wanted() {
    const UpscaleMode m = upscale_mode();
    return m == UpscaleMode::Fsr1 || m == UpscaleMode::Nis;
}

SpatialUpscaler* scene_upscaler(void* gfx_device, VkExtent2D in, VkExtent2D out) {
    static Fsr1Spatial* fsr = nullptr;
    static NisSpatial* nis = nullptr;
    static bool tried_fsr = false, tried_nis = false, told_range = false;
    if (!gfx_device) return nullptr;
    gfx::Device& dev = *static_cast<gfx::Device*>(gfx_device);
    if (upscale_mode() == UpscaleMode::Nis) {
        if (!tried_nis) {
            tried_nis = true;
            auto* n = new NisSpatial();
            if (n->init(dev)) nis = n;
            else delete n;
        }
        if (nis && nis->supports(in, out)) return nis;
        if (nis && !told_range) {
            told_range = true;
            NXLOG_I("hdr", "scene upscale %ux%u -> %ux%u is beyond NIS's 2x: FSR 1 is used for it", in.width, in.height, out.width, out.height);
        }
    }
    if (!tried_fsr) {
        tried_fsr = true;
        auto* f = new Fsr1Spatial();
        if (f->init(dev)) fsr = f;
        else delete f;
    }
    return fsr;
}

float scene_upscale_prescale() { return 0.1f; }

}  // namespace nxrt::extras
