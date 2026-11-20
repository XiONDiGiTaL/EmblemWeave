// SPDX-License-Identifier: GPL-3.0-or-later
// MAKO spatial upscaler (mako_spatial.h), adapted from MAKO's spatial_scaling.comp (Copyright (C) 2025-2026 Eugenio
// Segala and contributors, GPL-3.0-or-later).
#include "extras/mako_spatial.h"

#include <algorithm>
#include <cstddef>
#include <iterator>
#include <string>

#include "nxrt/log.h"

namespace nxrt::extras {
namespace {
constexpr const char* kTag = "mako";
constexpr uint32_t kBlockW = 8, kBlockH = 8;

#include "extras/mako_shaders.inc"

struct Push {
    float sizes[4];    // xy: source view size in texels; zw: output size in texels
    float tuning[4];   // x: sharpness 0..1; y: pre-scale k (0 = no HDR compression); zw: whole source size in texels
};
static_assert(sizeof(Push) == 32, "the shader's push constants are 32 bytes");

void to_general(VkCommandBuffer cb, VkImage img, VkImageLayout from, VkPipelineStageFlags2 src_stage, VkAccessFlags2 src_access,
                VkPipelineStageFlags2 dst_stage, VkAccessFlags2 dst_access) {
    VkImageMemoryBarrier2 b = {VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2};
    b.srcStageMask = src_stage;
    b.srcAccessMask = src_access;
    b.dstStageMask = dst_stage;
    b.dstAccessMask = dst_access;
    b.oldLayout = from;
    b.newLayout = VK_IMAGE_LAYOUT_GENERAL;
    b.srcQueueFamilyIndex = b.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    b.image = img;
    b.subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
    VkDependencyInfo di = {VK_STRUCTURE_TYPE_DEPENDENCY_INFO};
    di.imageMemoryBarrierCount = 1;
    di.pImageMemoryBarriers = &b;
    vkCmdPipelineBarrier2(cb, &di);
}
}  // namespace

bool MakoSpatial::init(gfx::Device& dev) {
    if (dev_) return true;
    dev_ = &dev;
    VkDevice d = dev.device();
    auto fail = [&](const char* what) {
        NXLOG_E(kTag, "%s failed", what);
        shutdown();
        return false;
    };
    VkSamplerCreateInfo sci = {VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO};
    sci.magFilter = sci.minFilter = VK_FILTER_LINEAR;
    sci.mipmapMode = VK_SAMPLER_MIPMAP_MODE_NEAREST;
    sci.addressModeU = sci.addressModeV = sci.addressModeW = VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE;
    sci.maxLod = 0.0f;
    if (vkCreateSampler(d, &sci, nullptr, &linear_) != VK_SUCCESS) return fail("sampler");

    const VkDescriptorSetLayoutBinding b[2] = {{0, VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, 1, VK_SHADER_STAGE_COMPUTE_BIT, nullptr},
                                               {1, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 1, VK_SHADER_STAGE_COMPUTE_BIT, nullptr}};
    VkDescriptorSetLayoutCreateInfo sli = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO};
    sli.bindingCount = 2;
    sli.pBindings = b;
    if (vkCreateDescriptorSetLayout(d, &sli, nullptr, &set_layout_) != VK_SUCCESS) return fail("set layout");
    VkPushConstantRange pcr = {VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof(Push)};
    VkPipelineLayoutCreateInfo pli = {VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO};
    pli.setLayoutCount = 1;
    pli.pSetLayouts = &set_layout_;
    pli.pushConstantRangeCount = 1;
    pli.pPushConstantRanges = &pcr;
    if (vkCreatePipelineLayout(d, &pli, nullptr, &layout_) != VK_SUCCESS) return fail("pipeline layout");

    const VkDescriptorPoolSize ps[2] = {{VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, kSetRing}, {VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, kSetRing}};
    VkDescriptorPoolCreateInfo dpi = {VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO};
    dpi.maxSets = kSetRing;
    dpi.poolSizeCount = 2;
    dpi.pPoolSizes = ps;
    if (vkCreateDescriptorPool(d, &dpi, nullptr, &pool_) != VK_SUCCESS) return fail("descriptor pool");
    VkDescriptorSetLayout ls[kSetRing];
    for (auto& x : ls) x = set_layout_;
    VkDescriptorSetAllocateInfo ai = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO};
    ai.descriptorPool = pool_;
    ai.descriptorSetCount = kSetRing;
    ai.pSetLayouts = ls;
    if (vkAllocateDescriptorSets(d, &ai, sets_) != VK_SUCCESS) return fail("descriptor sets");

    VkShaderModuleCreateInfo mi = {VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO};
    mi.codeSize = sizeof kMakoScaleComp;
    mi.pCode = kMakoScaleComp;
    VkShaderModule m = VK_NULL_HANDLE;
    if (vkCreateShaderModule(d, &mi, nullptr, &m) != VK_SUCCESS) return fail("shader module");
    VkComputePipelineCreateInfo ci = {VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO};
    ci.stage = {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, nullptr, 0, VK_SHADER_STAGE_COMPUTE_BIT, m, "main", nullptr};
    ci.layout = layout_;
    const VkResult r = vkCreateComputePipelines(d, VK_NULL_HANDLE, 1, &ci, nullptr, &pipe_);
    vkDestroyShaderModule(d, m, nullptr);
    if (r != VK_SUCCESS) return fail("pipeline");
    NXLOG_I(kTag, "MAKO spatial scaler ready (single pass)");
    return true;
}

void MakoSpatial::shutdown() {
    if (!dev_) return;
    VkDevice d = dev_->device();
    dev_->wait_idle();
    if (img_) dev_->destroy_image(img_);
    img_ = gfx::Image();
    extent_ = {};
    img_init_ = false;
    if (pipe_) vkDestroyPipeline(d, pipe_, nullptr);
    if (layout_) vkDestroyPipelineLayout(d, layout_, nullptr);
    if (pool_) vkDestroyDescriptorPool(d, pool_, nullptr);
    if (set_layout_) vkDestroyDescriptorSetLayout(d, set_layout_, nullptr);
    if (linear_) vkDestroySampler(d, linear_, nullptr);
    pipe_ = VK_NULL_HANDLE;
    layout_ = VK_NULL_HANDLE;
    pool_ = VK_NULL_HANDLE;
    set_layout_ = VK_NULL_HANDLE;
    linear_ = VK_NULL_HANDLE;
    std::fill(std::begin(sets_), std::end(sets_), VK_NULL_HANDLE);
    next_ = 0;
    dev_ = nullptr;
}

bool MakoSpatial::supports(VkExtent2D in_view, VkExtent2D out) const {
    return in_view.width && in_view.height && out.width && out.height;
}

VkImageView MakoSpatial::upscale(VkCommandBuffer cb, VkImageView src, VkExtent2D in_view, VkExtent2D in_tex, VkExtent2D out, bool linear_hdr, float prescale,
                                 float sharpness) {
    if (!dev_ || !src || !supports(in_view, out)) return VK_NULL_HANDLE;
    if (!img_ || extent_.width != out.width || extent_.height != out.height) {
        if (img_) {
            dev_->wait_idle();
            dev_->destroy_image(img_);
            img_ = gfx::Image();
        }
        gfx::ImageDesc id;
        id.format = VK_FORMAT_R16G16B16A16_SFLOAT;
        id.width = out.width;
        id.height = out.height;
        id.usage = VK_IMAGE_USAGE_STORAGE_BIT | VK_IMAGE_USAGE_SAMPLED_BIT;
        id.name = "mako out";
        std::string why;
        if (dev_->create_image(id, &img_, &why) != VK_SUCCESS) {
            NXLOG_E(kTag, "output image: %s", why.c_str());
            img_ = gfx::Image();
            return VK_NULL_HANDLE;
        }
        extent_ = out;
        img_init_ = false;
    }
    to_general(cb, img_.image, img_init_ ? VK_IMAGE_LAYOUT_GENERAL : VK_IMAGE_LAYOUT_UNDEFINED,
               VK_PIPELINE_STAGE_2_FRAGMENT_SHADER_BIT | VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_READ_BIT, VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT,
               VK_ACCESS_2_SHADER_WRITE_BIT);
    img_init_ = true;

    Push pc = {};
    pc.sizes[0] = static_cast<float>(in_view.width);
    pc.sizes[1] = static_cast<float>(in_view.height);
    pc.sizes[2] = static_cast<float>(out.width);
    pc.sizes[3] = static_cast<float>(out.height);
    pc.tuning[0] = std::clamp(sharpness, 0.0f, 1.0f);
    pc.tuning[1] = linear_hdr ? (prescale > 0.0f ? prescale : 1.0f) : 0.0f;
    pc.tuning[2] = static_cast<float>(in_tex.width);
    pc.tuning[3] = static_cast<float>(in_tex.height);

    VkDescriptorSet set = sets_[next_];
    next_ = (next_ + 1) % kSetRing;
    const VkDescriptorImageInfo in = {linear_, src, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL};
    const VkDescriptorImageInfo dst = {VK_NULL_HANDLE, img_.view, VK_IMAGE_LAYOUT_GENERAL};
    VkWriteDescriptorSet w[2] = {{VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET}, {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET}};
    w[0].dstSet = set; w[0].dstBinding = 0; w[0].descriptorCount = 1; w[0].descriptorType = VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER; w[0].pImageInfo = &in;
    w[1].dstSet = set; w[1].dstBinding = 1; w[1].descriptorCount = 1; w[1].descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_IMAGE; w[1].pImageInfo = &dst;
    vkUpdateDescriptorSets(dev_->device(), 2, w, 0, nullptr);
    vkCmdBindPipeline(cb, VK_PIPELINE_BIND_POINT_COMPUTE, pipe_);
    vkCmdBindDescriptorSets(cb, VK_PIPELINE_BIND_POINT_COMPUTE, layout_, 0, 1, &set, 0, nullptr);
    vkCmdPushConstants(cb, layout_, VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof pc, &pc);
    vkCmdDispatch(cb, (out.width + kBlockW - 1) / kBlockW, (out.height + kBlockH - 1) / kBlockH, 1);
    to_general(cb, img_.image, VK_IMAGE_LAYOUT_GENERAL, VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_WRITE_BIT,
               VK_PIPELINE_STAGE_2_FRAGMENT_SHADER_BIT | VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_READ_BIT);
    ++frames_;
    return img_.view;
}

}  // namespace nxrt::extras
