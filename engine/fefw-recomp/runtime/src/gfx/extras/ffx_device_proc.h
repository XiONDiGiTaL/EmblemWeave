// SPDX-License-Identifier: GPL-3.0-or-later
// The device function resolver FFX's Vulkan backend is handed (ffx_api_vk.h's vkDeviceProcAddr). FFX asks for KHR aliases of core
// functions (vkGetBufferMemoryRequirements2KHR) and calls them unchecked; on a Vulkan 1.3 device without the old extensions those names
// are null, so the first evaluation jumps to 0x0. Fall back to the core name, as a 1.3 loader does for the game's own calls.
#pragma once

#include <volk.h>

#include <cstddef>
#include <cstring>
#include <string>

#include "nxrt/log.h"

namespace nxrt::extras {

inline VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL ffx_device_proc(VkDevice d, const char* name, const char* tag) {
    if (PFN_vkVoidFunction f = vkGetDeviceProcAddr(d, name)) return f;
    const size_t n = name ? std::strlen(name) : 0;
    if (n > 3 && std::strcmp(name + n - 3, "KHR") == 0) return vkGetDeviceProcAddr(d, std::string(name, n - 3).c_str());
    if (tag) NXLOG_W(tag, "FFX asked for %s: the device has no such function", name ? name : "(null)");
    return nullptr;
}

}  // namespace nxrt::extras
