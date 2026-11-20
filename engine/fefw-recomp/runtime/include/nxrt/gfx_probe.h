// SPDX-License-Identifier: GPL-3.0-or-later
// A quick Vulkan device list for the launcher's GPU picker. It creates an instance, lists every physical device and
// destroys it again; nothing else of the graphics layer is started ([gfx] gpu takes the index this list prints).
#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace nxrt::gfx {

struct GpuProbe {
    std::string name;               // the driver's device name
    std::string type;               // discrete, integrated, virtual, cpu or other
    uint64_t vram_bytes = 0;        // device-local memory (all memory when the driver exposes no device-local heap)
};

// Fills *out in Vulkan's device order. False with *err set when there is no Vulkan 1.3 loader, no device, or instance creation fails.
bool probe_gpus(std::vector<GpuProbe>* out, std::string* err);

}  // namespace nxrt::gfx
