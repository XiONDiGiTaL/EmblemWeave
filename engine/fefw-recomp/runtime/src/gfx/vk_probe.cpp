// SPDX-License-Identifier: GPL-3.0-or-later
// GPU enumeration for the launcher (include/nxrt/gfx_probe.h): enough Vulkan to list the devices, nothing more.
#include "nxrt/gfx_probe.h"

#include <volk.h>

namespace nxrt::gfx {
namespace {

const char* type_name(VkPhysicalDeviceType type) {
    switch (type) {
        case VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU: return "discrete";
        case VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU: return "integrated";
        case VK_PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU: return "virtual";
        case VK_PHYSICAL_DEVICE_TYPE_CPU: return "cpu";
        default: return "other";
    }
}

uint64_t memory_bytes(const VkPhysicalDeviceMemoryProperties& mem) {
    uint64_t device_local = 0, total = 0;
    for (uint32_t i = 0; i < mem.memoryHeapCount; ++i) {
        total += mem.memoryHeaps[i].size;
        if (mem.memoryHeaps[i].flags & VK_MEMORY_HEAP_DEVICE_LOCAL_BIT) device_local += mem.memoryHeaps[i].size;
    }
    return device_local ? device_local : total;
}

bool set_error(std::string* err, const char* text) {
    if (err) *err = text;
    return false;
}

}  // namespace

bool probe_gpus(std::vector<GpuProbe>* out, std::string* err) {
    if (!out) return set_error(err, "no output list");
    out->clear();
    if (err) err->clear();
    if (volkInitialize() != VK_SUCCESS) return set_error(err, "no Vulkan loader found (vulkan-1.dll)");

    uint32_t loader_version = 0;
    if (vkEnumerateInstanceVersion(&loader_version) != VK_SUCCESS || loader_version < VK_API_VERSION_1_3) {
        return set_error(err, "Vulkan 1.3 is required to run the game; this loader offers less");
    }

    VkApplicationInfo app{VK_STRUCTURE_TYPE_APPLICATION_INFO};
    app.pApplicationName = "nxrt GPU list";
    app.applicationVersion = 1;
    app.pEngineName = "nxrt";
    app.engineVersion = 1;
    app.apiVersion = VK_API_VERSION_1_3;

    VkInstanceCreateInfo ci{VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};
    ci.pApplicationInfo = &app;

    VkInstance instance = VK_NULL_HANDLE;
    VkResult r = vkCreateInstance(&ci, nullptr, &instance);
    if (r != VK_SUCCESS) return set_error(err, "the Vulkan loader could not create an instance");
    volkLoadInstance(instance);   // volk resolves the instance-level entry points (vkEnumeratePhysicalDevices and friends) per instance

    uint32_t count = 0;
    r = vkEnumeratePhysicalDevices(instance, &count, nullptr);
    if (r != VK_SUCCESS || count == 0) {
        vkDestroyInstance(instance, nullptr);
        return set_error(err, r != VK_SUCCESS ? "the Vulkan device list could not be read" : "no Vulkan devices found");
    }
    std::vector<VkPhysicalDevice> devices(count);
    r = vkEnumeratePhysicalDevices(instance, &count, devices.data());
    if (r != VK_SUCCESS) {
        vkDestroyInstance(instance, nullptr);
        return set_error(err, "the Vulkan device list could not be read");
    }

    out->reserve(count);
    for (uint32_t i = 0; i < count; ++i) {
        VkPhysicalDeviceProperties props{};
        vkGetPhysicalDeviceProperties(devices[i], &props);
        VkPhysicalDeviceMemoryProperties mem{};
        vkGetPhysicalDeviceMemoryProperties(devices[i], &mem);
        GpuProbe g;
        g.name = props.deviceName;
        g.type = type_name(props.deviceType);
        g.vram_bytes = memory_bytes(mem);
        out->push_back(std::move(g));
    }
    vkDestroyInstance(instance, nullptr);
    return true;
}

}  // namespace nxrt::gfx
