// SPDX-License-Identifier: GPL-3.0-or-later
// The built-in temporal upscaler behind the NVN2 DLSS API (src/gfx/nvn/api_dlss_replay.cpp): one compute pass per frame, no SDK and no DLL.
// Each output pixel takes the jittered input texels around its scene point with a Gaussian of half an output pixel, reprojects the history
// with the motion vector of the nearest-depth texel, clips it to the neighbourhood's colour box (YCoCg, on a colour tone-mapped reversibly
// around the neighbourhood's mean luma, so the scene's exposure does not matter: Fortune's Weave hands over pre-exposure HDR) and blends by
// accumulated sample weight: a texel that lands on the pixel centre counts 1, one an output pixel away counts e^-2, and the
// history holds at most max_weight. A reset (or history off screen) keeps only the current frame. The spatial mode is a plain bilinear of
// the unjittered position (no history), for comparison.
// The same object also holds the input pass of the SDK upscalers (FSR 3, Streamline DLSS): the game's inputs copied into TemporalFrame
// images (temporal.h). Shaders: scripts/gen_taau_shaders.py -> taau_shaders.inc.
#pragma once
#include <volk.h>

#include <cstdint>

namespace nxrt::gfx {
class Device;
}

namespace nxrt::extras {

// One frame. The input views are read with texelFetch in the layouts given; the history images are rgba16f of size `out`, in GENERAL layout
// (history_in is read filtered, history_out written: rgb linear colour, a the accumulated weight). Coordinates are image texels.
struct TaauFrame {
    VkImageView color = VK_NULL_HANDLE, depth = VK_NULL_HANDLE, motion = VK_NULL_HANDLE, mask = VK_NULL_HANDLE;   // mask: optional
    VkImageLayout color_layout = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, depth_layout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_READ_ONLY_OPTIMAL,
                  motion_layout = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, mask_layout = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL;
    VkImageView history_in = VK_NULL_HANDLE, history_out = VK_NULL_HANDLE;
    VkExtent2D out = {};
    float in_size[2] = {}, in_base[2] = {};               // the colour region
    float jitter[2] = {};                                 // texel t of the region shows the scene at t + 0.5 - jitter (+y down)
    float mv_scale[2] = {}, mv_offset[2] = {};            // motion texel * scale + offset = previous minus current position, output UV units
    float depth_base[2] = {}, depth_ratio[2] = {1, 1};    // depth texel = base + floor((colour region texel + 0.5) * ratio)
    float mv_base[2] = {}, mv_ratio[2] = {1, 1};          // the same, or per output pixel with mv_output_res
    float mask_base[2] = {}, mask_ratio[2] = {1, 1};
    bool reset = false, depth_inverted = false, mv_output_res = false, spatial = false;
    float max_weight = 12.0f;
};

// The input pass of the SDK upscalers: colour (rgba16f), depth (r32f) and motion (rg16f, render UV units, previous minus current) of the
// region, written to images of in_size in GENERAL layout.
struct TaauPrepFrame {
    VkImageView color = VK_NULL_HANDLE, depth = VK_NULL_HANDLE, motion = VK_NULL_HANDLE;
    VkImageLayout color_layout = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, depth_layout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_READ_ONLY_OPTIMAL,
                  motion_layout = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL;
    VkImageView out_color = VK_NULL_HANDLE, out_depth = VK_NULL_HANDLE, out_motion = VK_NULL_HANDLE;
    float in_size[2] = {}, in_base[2] = {};
    float mv_scale[2] = {}, mv_offset[2] = {};
    float depth_base[2] = {}, depth_ratio[2] = {1, 1};
    float mv_base[2] = {}, mv_ratio[2] = {1, 1};
};

class Taau {
public:
    // The pipelines of `dev`, made on first use; null when they cannot be made (logged once).
    static Taau* get(gfx::Device& dev);
    // Destroys the pipelines of the device being shut down (device idle).
    static void shutdown();

    VkDescriptorSetLayout set_layout() const { return set_layout_; }
    VkDescriptorSetLayout prep_set_layout() const { return prep_set_layout_; }
    // Record into cb, outside any render pass. `set` is a set of set_layout() / prep_set_layout() that lives until the GPU has run cb. The
    // caller orders the inputs' writes before the pass and the outputs' readers after it.
    void record(VkCommandBuffer cb, VkDescriptorSet set, const TaauFrame& f);
    void record_prep(VkCommandBuffer cb, VkDescriptorSet set, const TaauPrepFrame& f);

private:
    explicit Taau(gfx::Device& dev) : dev_(&dev) {}
    bool init();
    void destroy();
    gfx::Device* dev_;
    VkSampler nearest_ = VK_NULL_HANDLE, linear_ = VK_NULL_HANDLE;
    VkDescriptorSetLayout set_layout_ = VK_NULL_HANDLE, prep_set_layout_ = VK_NULL_HANDLE;
    VkPipelineLayout layout_ = VK_NULL_HANDLE, prep_layout_ = VK_NULL_HANDLE;
    VkPipeline pipe_ = VK_NULL_HANDLE, prep_pipe_ = VK_NULL_HANDLE;
};

}  // namespace nxrt::extras
