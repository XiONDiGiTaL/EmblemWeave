// SPDX-License-Identifier: GPL-3.0-or-later
// Linux diagnostics for the native runtime: prints the facts a Linux bug report needs before anyone reads a log --
// kernel, page size and core count from the platform layer, the paths the launcher would use, and whether a Vulkan
// loader is present and which physical devices it enumerates. It links nxrt_platform, so a broken POSIX backend shows
// up here as a wrong number rather than as a crash in a game.
//
// Build: cmake -S engine/fefw-recomp/runtime -B build-linux -G Ninja && cmake --build build-linux
// Run:   ./build-linux/nxrt_linux_probe
#include <dlfcn.h>
#include <sys/utsname.h>
#include <unistd.h>
#include <vulkan/vulkan.h>

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

#include "nxrt/platform.h"

namespace {

std::string vk_version(uint32_t v) {
    char buf[32];
    std::snprintf(buf, sizeof buf, "%u.%u.%u", VK_VERSION_MAJOR(v), VK_VERSION_MINOR(v), VK_VERSION_PATCH(v));
    return buf;
}

const char* device_type_name(VkPhysicalDeviceType type) {
    switch (type) {
        case VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU: return "integrated";
        case VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU: return "discrete";
        case VK_PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU: return "virtual";
        case VK_PHYSICAL_DEVICE_TYPE_CPU: return "cpu";
        default: return "other";
    }
}

std::string config_root() {
    const char* xdg = std::getenv("XDG_CONFIG_HOME");
    if (xdg && *xdg) return std::string(xdg) + "/EmblemWeave";
    const char* home = std::getenv("HOME");
    return std::string(home && *home ? home : ".") + "/.config/EmblemWeave";
}

std::string cache_root() {
    const char* xdg = std::getenv("XDG_CACHE_HOME");
    if (xdg && *xdg) return std::string(xdg) + "/EmblemWeave";
    const char* home = std::getenv("HOME");
    return std::string(home && *home ? home : ".") + "/.cache/EmblemWeave";
}

}  // namespace

int main() {
    std::printf("nxrt linux probe\n");

    utsname u{};
    if (uname(&u) == 0) std::printf("kernel: %s %s %s\n", u.sysname, u.release, u.machine);
    std::printf("page size: %zu\n", nxrt::platform::page_size());
    std::printf("cores: %d physical, %ld online\n", nxrt::platform::physical_core_count(),
                sysconf(_SC_NPROCESSORS_ONLN));
    std::printf("config root: %s\n", config_root().c_str());
    std::printf("cache root: %s\n", cache_root().c_str());

    // The engine opens the loader itself (volk-style), so the probe does too rather than linking against it: the
    // question is whether libvulkan.so.1 can be loaded and what it reports, not whether the build host had headers.
    void* loader = dlopen("libvulkan.so.1", RTLD_NOW | RTLD_LOCAL);
    if (!loader) {
        std::printf("vulkan loader: NOT FOUND (%s)\n", dlerror());
        return 2;
    }
    std::printf("vulkan loader: libvulkan.so.1 opened\n");
    auto get_instance_proc = reinterpret_cast<PFN_vkGetInstanceProcAddr>(dlsym(loader, "vkGetInstanceProcAddr"));
    if (!get_instance_proc) {
        std::printf("vulkan loader: vkGetInstanceProcAddr missing\n");
        return 2;
    }
    if (auto enumerate_version = reinterpret_cast<PFN_vkEnumerateInstanceVersion>(
            get_instance_proc(nullptr, "vkEnumerateInstanceVersion"))) {
        uint32_t version = 0;
        if (enumerate_version(&version) == VK_SUCCESS) std::printf("loader api version: %s\n", vk_version(version).c_str());
    }

    auto create_instance = reinterpret_cast<PFN_vkCreateInstance>(get_instance_proc(nullptr, "vkCreateInstance"));
    if (!create_instance) {
        std::printf("vulkan loader: vkCreateInstance missing\n");
        return 2;
    }
    VkApplicationInfo app{VK_STRUCTURE_TYPE_APPLICATION_INFO};
    app.pApplicationName = "nxrt_linux_probe";
    app.apiVersion = VK_API_VERSION_1_0;   // the lowest version the engine can run on
    VkInstanceCreateInfo info{VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};
    info.pApplicationInfo = &app;
    VkInstance instance = VK_NULL_HANDLE;
    const VkResult created = create_instance(&info, nullptr, &instance);
    if (created != VK_SUCCESS) {
        std::printf("vulkan instance: FAILED (VkResult %d)\n", static_cast<int>(created));
        return 3;
    }

    auto enumerate_devices =
        reinterpret_cast<PFN_vkEnumeratePhysicalDevices>(get_instance_proc(instance, "vkEnumeratePhysicalDevices"));
    auto get_properties = reinterpret_cast<PFN_vkGetPhysicalDeviceProperties>(
        get_instance_proc(instance, "vkGetPhysicalDeviceProperties"));
    if (!enumerate_devices || !get_properties) {
        std::printf("vulkan instance: device enumeration entry points missing\n");
        return 3;
    }
    uint32_t count = 0;
    VkResult r = enumerate_devices(instance, &count, nullptr);
    std::vector<VkPhysicalDevice> devices(count);
    if (r == VK_SUCCESS && count) r = enumerate_devices(instance, &count, devices.data());
    std::printf("vulkan devices: %u\n", r == VK_SUCCESS ? count : 0);
    if (r == VK_SUCCESS) {
        for (uint32_t i = 0; i < count; ++i) {
            VkPhysicalDeviceProperties p{};
            get_properties(devices[i], &p);
            std::printf("  [%u] %s | %s | api %s | driver 0x%08x | vendor 0x%04x device 0x%04x\n", i, p.deviceName,
                        device_type_name(p.deviceType), vk_version(p.apiVersion).c_str(), p.driverVersion,
                        p.vendorID, p.deviceID);
        }
    }

    auto destroy_instance = reinterpret_cast<PFN_vkDestroyInstance>(get_instance_proc(instance, "vkDestroyInstance"));
    if (destroy_instance) destroy_instance(instance, nullptr);
    dlclose(loader);
    return count ? 0 : 1;   // a loaded loader with no device is a real, reportable state
}
