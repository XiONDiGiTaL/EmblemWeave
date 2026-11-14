// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once
#include "nxrt/gfx.h"
#include <vector>

namespace nxrt::nvn::be::gpu {
struct SubgroupShader {
    bool used = false;
    uint32_t local[3] = {0, 0, 0}; // Unknown sizes must not bypass the compute limit.
};

// Inspect instructions and subgroup built-ins in the final module (variants too).
// Capabilities alone are insufficient: Ryujinx declares them unconditionally.
inline SubgroupShader subgroup_shader(const std::vector<uint32_t>& words) {
    SubgroupShader out;
    for (size_t i = 5; i < words.size();) {
        const uint32_t n = words[i] >> 16, op = words[i] & 0xffff;
        if (!n || n > words.size() - i) break;
        if (op >= 333 && op <= 366) out.used = true; // OpGroupNonUniform*
        if (op == 71 && n >= 4 && words[i + 2] == 11) { // OpDecorate BuiltIn
            const auto builtin = words[i + 3];
            out.used |= (builtin >= 36 && builtin <= 41) || (builtin >= 4416 && builtin <= 4420);
        }
        if (op == 16 && n == 6 && words[i + 2] == 17) // OpExecutionMode LocalSize
            for (int d = 0; d < 3; ++d) out.local[d] = words[i + 3 + d];
        i += n;
    }
    return out;
}

// Storage belongs to the caller and must outlive vkCreate*Pipelines. Preserve
// existing stage extensions, particularly per-stage UBO robustness.
inline bool require_guest_subgroup(VkPipelineShaderStageCreateInfo& stage,
                                  VkPipelineShaderStageRequiredSubgroupSizeCreateInfo& size,
                                  const SubgroupShader& shader, const gfx::DeviceFeatures& f,
                                  const gfx::DeviceLimitsExt& l) {
    if (!shader.used || !f.subgroup_size_control || l.min_subgroup_size > 32 || l.max_subgroup_size < 32 ||
        !(l.required_subgroup_size_stages & stage.stage)) return false;
    if (stage.stage == VK_SHADER_STAGE_COMPUTE_BIT) {
        uint64_t threads = 1;
        for (auto d : shader.local) {
            if (!d || threads > (uint64_t(32) * l.max_compute_workgroup_subgroups) / d) return false;
            threads *= d;
        }
        // Full subgroups additionally require LocalSize.x to be a multiple of
        // the requested size. Partial guest warps remain legal without the flag.
        if (f.compute_full_subgroups && shader.local[0] % 32 == 0)
            stage.flags |= VK_PIPELINE_SHADER_STAGE_CREATE_REQUIRE_FULL_SUBGROUPS_BIT;
    }
    stage.flags &= ~VK_PIPELINE_SHADER_STAGE_CREATE_ALLOW_VARYING_SUBGROUP_SIZE_BIT;
    size = {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_REQUIRED_SUBGROUP_SIZE_CREATE_INFO, stage.pNext, 32};
    stage.pNext = &size;
    return true;
}
} // namespace nxrt::nvn::be::gpu
