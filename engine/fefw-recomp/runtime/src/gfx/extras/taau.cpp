// SPDX-License-Identifier: GPL-3.0-or-later
// The built-in temporal upscaler of the NVN2 DLSS API (taau.h).
#include "extras/taau.h"

#include <mutex>

#include "nxrt/gfx.h"
#include "nxrt/log.h"

namespace nxrt::extras {
namespace {

constexpr const char* kTag = "taau";

#include "extras/taau_shaders.inc"

struct TaauPush {
    float in_size[2], in_base[2], out_size[2], jitter[2], mv_scale[2], mv_offset[2];
    float depth_base[2], depth_ratio[2], mv_base[2], mv_ratio[2], mask_base[2], mask_ratio[2];
    uint32_t flags;
    float max_weight;
};
static_assert(sizeof(TaauPush) == 104, "push block of taau.comp");

struct PrepPush {
    float in_size[2], in_base[2], mv_scale[2], mv_offset[2], depth_base[2], depth_ratio[2], mv_base[2], mv_ratio[2];
};
static_assert(sizeof(PrepPush) == 64, "push block of taau_prep.comp");

constexpr uint32_t kReset = 1, kDepthInverted = 2, kMotionOut = 4, kMask = 8, kSpatial = 16;

std::mutex g_mu;
Taau* g_inst = nullptr;
VkDevice g_inst_dev = VK_NULL_HANDLE;
bool g_failed = false;

void copy2(float* d, const float* s) {
    d[0] = s[0];
    d[1] = s[1];
}

}  // namespace

Taau* Taau::get(gfx::Device& dev) {
    std::lock_guard<std::mutex> lk(g_mu);
    if (g_inst && g_inst_dev == dev.device()) return g_inst;
    if (g_failed && g_inst_dev == dev.device()) return nullptr;
    if (g_inst) {   // another device (tests): the old one is gone with its device
        delete g_inst;
        g_inst = nullptr;
    }
    g_inst_dev = dev.device();
    g_failed = false;
    auto* t = new Taau(dev);
    if (!t->init()) {
        t->destroy();
        delete t;
        g_failed = true;
        NXLOG_E(kTag, "the upscaler's pipelines could not be made: the DLSS pass leaves its output untouched");
        return nullptr;
    }
    g_inst = t;
    return t;
}

void Taau::shutdown() {
    std::lock_guard<std::mutex> lk(g_mu);
    if (g_inst) {
        g_inst->destroy();
        delete g_inst;
    }
    g_inst = nullptr;
    g_inst_dev = VK_NULL_HANDLE;
    g_failed = false;
}

bool Taau::init() {
    VkDevice d = dev_->device();
    VkSamplerCreateInfo sci = {VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO};
    sci.magFilter = sci.minFilter = VK_FILTER_NEAREST;
    sci.mipmapMode = VK_SAMPLER_MIPMAP_MODE_NEAREST;
    sci.addressModeU = sci.addressModeV = sci.addressModeW = VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE;
    sci.maxLod = 0.0f;
    if (vkCreateSampler(d, &sci, nullptr, &nearest_) != VK_SUCCESS) return false;
    sci.magFilter = sci.minFilter = VK_FILTER_LINEAR;
    if (vkCreateSampler(d, &sci, nullptr, &linear_) != VK_SUCCESS) return false;

    auto make_layout = [&](uint32_t samplers, uint32_t images, VkDescriptorSetLayout* out) {
        VkDescriptorSetLayoutBinding b[8] = {};
        for (uint32_t i = 0; i < samplers + images; ++i) {
            b[i].binding = i;
            b[i].descriptorType = i < samplers ? VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER : VK_DESCRIPTOR_TYPE_STORAGE_IMAGE;
            b[i].descriptorCount = 1;
            b[i].stageFlags = VK_SHADER_STAGE_COMPUTE_BIT;
        }
        VkDescriptorSetLayoutCreateInfo li = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO};
        li.bindingCount = samplers + images;
        li.pBindings = b;
        return vkCreateDescriptorSetLayout(d, &li, nullptr, out) == VK_SUCCESS;
    };
    if (!make_layout(5, 1, &set_layout_) || !make_layout(3, 3, &prep_set_layout_)) return false;

    auto make_pipe = [&](VkDescriptorSetLayout sl, uint32_t push, const uint32_t* code, size_t code_size, VkPipelineLayout* pl, VkPipeline* pipe) {
        VkPushConstantRange pcr = {VK_SHADER_STAGE_COMPUTE_BIT, 0, push};
        VkPipelineLayoutCreateInfo pli = {VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO};
        pli.setLayoutCount = 1;
        pli.pSetLayouts = &sl;
        pli.pushConstantRangeCount = 1;
        pli.pPushConstantRanges = &pcr;
        if (vkCreatePipelineLayout(d, &pli, nullptr, pl) != VK_SUCCESS) return false;
        VkShaderModuleCreateInfo mi = {VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO};
        mi.codeSize = code_size;
        mi.pCode = code;
        VkShaderModule mod = VK_NULL_HANDLE;
        if (vkCreateShaderModule(d, &mi, nullptr, &mod) != VK_SUCCESS) return false;
        VkComputePipelineCreateInfo ci = {VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO};
        ci.stage = {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, nullptr, 0, VK_SHADER_STAGE_COMPUTE_BIT, mod, "main", nullptr};
        ci.layout = *pl;
        const VkResult r = vkCreateComputePipelines(d, VK_NULL_HANDLE, 1, &ci, nullptr, pipe);
        vkDestroyShaderModule(d, mod, nullptr);
        return r == VK_SUCCESS;
    };
    return make_pipe(set_layout_, sizeof(TaauPush), kTaauComp, sizeof(kTaauComp), &layout_, &pipe_) &&
           make_pipe(prep_set_layout_, sizeof(PrepPush), kTaauPrepComp, sizeof(kTaauPrepComp), &prep_layout_, &prep_pipe_);
}

void Taau::destroy() {
    VkDevice d = dev_->device();
    if (pipe_) vkDestroyPipeline(d, pipe_, nullptr);
    if (prep_pipe_) vkDestroyPipeline(d, prep_pipe_, nullptr);
    if (layout_) vkDestroyPipelineLayout(d, layout_, nullptr);
    if (prep_layout_) vkDestroyPipelineLayout(d, prep_layout_, nullptr);
    if (set_layout_) vkDestroyDescriptorSetLayout(d, set_layout_, nullptr);
    if (prep_set_layout_) vkDestroyDescriptorSetLayout(d, prep_set_layout_, nullptr);
    if (nearest_) vkDestroySampler(d, nearest_, nullptr);
    if (linear_) vkDestroySampler(d, linear_, nullptr);
    pipe_ = prep_pipe_ = VK_NULL_HANDLE;
    layout_ = prep_layout_ = VK_NULL_HANDLE;
    set_layout_ = prep_set_layout_ = VK_NULL_HANDLE;
    nearest_ = linear_ = VK_NULL_HANDLE;
}

void Taau::record(VkCommandBuffer cb, VkDescriptorSet set, const TaauFrame& f) {
    // an absent mask binds the colour view (valid, never read: the mask flag is off)
    const VkImageView mask = f.mask ? f.mask : f.color;
    const VkImageLayout mask_layout = f.mask ? f.mask_layout : f.color_layout;
    const VkDescriptorImageInfo ii[6] = {{nearest_, f.color, f.color_layout},
                                         {nearest_, f.depth, f.depth_layout},
                                         {nearest_, f.motion, f.motion_layout},
                                         {nearest_, mask, mask_layout},
                                         {linear_, f.history_in, VK_IMAGE_LAYOUT_GENERAL},
                                         {VK_NULL_HANDLE, f.history_out, VK_IMAGE_LAYOUT_GENERAL}};
    VkWriteDescriptorSet w[6];
    for (uint32_t i = 0; i < 6; ++i) {
        w[i] = {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET};
        w[i].dstSet = set;
        w[i].dstBinding = i;
        w[i].descriptorCount = 1;
        w[i].descriptorType = i < 5 ? VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER : VK_DESCRIPTOR_TYPE_STORAGE_IMAGE;
        w[i].pImageInfo = &ii[i];
    }
    vkUpdateDescriptorSets(dev_->device(), 6, w, 0, nullptr);
    TaauPush p = {};
    copy2(p.in_size, f.in_size);
    copy2(p.in_base, f.in_base);
    p.out_size[0] = static_cast<float>(f.out.width);
    p.out_size[1] = static_cast<float>(f.out.height);
    copy2(p.jitter, f.jitter);
    copy2(p.mv_scale, f.mv_scale);
    copy2(p.mv_offset, f.mv_offset);
    copy2(p.depth_base, f.depth_base);
    copy2(p.depth_ratio, f.depth_ratio);
    copy2(p.mv_base, f.mv_base);
    copy2(p.mv_ratio, f.mv_ratio);
    copy2(p.mask_base, f.mask_base);
    copy2(p.mask_ratio, f.mask_ratio);
    p.flags = (f.reset ? kReset : 0) | (f.depth_inverted ? kDepthInverted : 0) | (f.mv_output_res ? kMotionOut : 0) | (f.mask ? kMask : 0) |
              (f.spatial ? kSpatial : 0);
    p.max_weight = f.max_weight > 1.0f ? f.max_weight : 1.0f;
    vkCmdBindPipeline(cb, VK_PIPELINE_BIND_POINT_COMPUTE, pipe_);
    vkCmdBindDescriptorSets(cb, VK_PIPELINE_BIND_POINT_COMPUTE, layout_, 0, 1, &set, 0, nullptr);
    vkCmdPushConstants(cb, layout_, VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof p, &p);
    vkCmdDispatch(cb, (f.out.width + 7) / 8, (f.out.height + 7) / 8, 1);
}

void Taau::record_prep(VkCommandBuffer cb, VkDescriptorSet set, const TaauPrepFrame& f) {
    const VkDescriptorImageInfo ii[6] = {{nearest_, f.color, f.color_layout},
                                         {nearest_, f.depth, f.depth_layout},
                                         {nearest_, f.motion, f.motion_layout},
                                         {VK_NULL_HANDLE, f.out_color, VK_IMAGE_LAYOUT_GENERAL},
                                         {VK_NULL_HANDLE, f.out_depth, VK_IMAGE_LAYOUT_GENERAL},
                                         {VK_NULL_HANDLE, f.out_motion, VK_IMAGE_LAYOUT_GENERAL}};
    VkWriteDescriptorSet w[6];
    for (uint32_t i = 0; i < 6; ++i) {
        w[i] = {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET};
        w[i].dstSet = set;
        w[i].dstBinding = i;
        w[i].descriptorCount = 1;
        w[i].descriptorType = i < 3 ? VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER : VK_DESCRIPTOR_TYPE_STORAGE_IMAGE;
        w[i].pImageInfo = &ii[i];
    }
    vkUpdateDescriptorSets(dev_->device(), 6, w, 0, nullptr);
    PrepPush p = {};
    copy2(p.in_size, f.in_size);
    copy2(p.in_base, f.in_base);
    copy2(p.mv_scale, f.mv_scale);
    copy2(p.mv_offset, f.mv_offset);
    copy2(p.depth_base, f.depth_base);
    copy2(p.depth_ratio, f.depth_ratio);
    copy2(p.mv_base, f.mv_base);
    copy2(p.mv_ratio, f.mv_ratio);
    vkCmdBindPipeline(cb, VK_PIPELINE_BIND_POINT_COMPUTE, prep_pipe_);
    vkCmdBindDescriptorSets(cb, VK_PIPELINE_BIND_POINT_COMPUTE, prep_layout_, 0, 1, &set, 0, nullptr);
    vkCmdPushConstants(cb, prep_layout_, VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof p, &p);
    const uint32_t w0 = static_cast<uint32_t>(f.in_size[0]), h0 = static_cast<uint32_t>(f.in_size[1]);
    vkCmdDispatch(cb, (w0 + 7) / 8, (h0 + 7) / 8, 1);
}

}  // namespace nxrt::extras
