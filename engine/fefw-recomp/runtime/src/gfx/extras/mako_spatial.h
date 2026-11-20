// SPDX-License-Identifier: GPL-3.0-or-later
// MAKO spatial upscaler: one compute pass into a float16 image the size of the output, adapted from MAKO's
// spatial_scaling.comp (Copyright (C) 2025-2026 Eugenio Segala and contributors, GPL-3.0-or-later). The shader is a
// single-pass sharpened-cubic reconstruction (Catmull-Rom tension 0.8) with contrast-aware anti-ringing and bounded
// local sharpening, so it costs one dispatch where FSR 1 costs two. It runs on any GPU and, unlike NIS, has no 2x
// scale limit. The shader is in mako_shaders.inc (scripts/gen_mako_shaders.py).
#pragma once
#include <volk.h>

#include <cstdint>

#include "nxrt/gfx.h"
#include "extras/spatial_upscaler.h"

namespace nxrt::extras {

class MakoSpatial : public SpatialUpscaler {
public:
    MakoSpatial() = default;
    ~MakoSpatial() override { shutdown(); }
    MakoSpatial(const MakoSpatial&) = delete;
    MakoSpatial& operator=(const MakoSpatial&) = delete;

    bool init(gfx::Device& dev);
    void shutdown();
    bool initialised() const { return dev_ != nullptr; }

    VkImageView upscale(VkCommandBuffer cb, VkImageView src, VkExtent2D in_view, VkExtent2D in_tex, VkExtent2D out, bool linear_hdr, float prescale,
                        float sharpness) override;
    bool supports(VkExtent2D in_view, VkExtent2D out) const override;
    uint32_t frames() const { return frames_; }

    static constexpr uint32_t kSetRing = 16;

private:
    gfx::Device* dev_ = nullptr;
    VkSampler linear_ = VK_NULL_HANDLE;
    VkDescriptorSetLayout set_layout_ = VK_NULL_HANDLE;
    VkPipelineLayout layout_ = VK_NULL_HANDLE;
    VkPipeline pipe_ = VK_NULL_HANDLE;
    VkDescriptorPool pool_ = VK_NULL_HANDLE;
    VkDescriptorSet sets_[kSetRing] = {};
    uint32_t next_ = 0;
    gfx::Image img_;
    VkExtent2D extent_ = {};
    bool img_init_ = false;
    uint32_t frames_ = 0;
};

}  // namespace nxrt::extras
