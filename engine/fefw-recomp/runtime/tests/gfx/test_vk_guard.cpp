// SPDX-License-Identifier: GPL-3.0-or-later
// Memory guard discount of the Vulkan layer: which heap's usage is subtracted from the process commit (vk_gauges.h). No GPU needed.
#include <cstring>

#include "test_framework.h"
#include "vk_gauges.h"
NXTEST_GROUP(gfx_guard)

namespace {

constexpr VkDeviceSize kGiB = 1ull << 30;

void heap(VkPhysicalDeviceMemoryProperties& mp, uint32_t i, VkDeviceSize size, bool device_local) {
    mp.memoryHeaps[i].size = size;
    mp.memoryHeaps[i].flags = device_local ? VK_MEMORY_HEAP_DEVICE_LOCAL_BIT : 0;
    if (i + 1 > mp.memoryHeapCount) mp.memoryHeapCount = i + 1;
}
void mtype(VkPhysicalDeviceMemoryProperties& mp, uint32_t i, uint32_t heap_index, VkMemoryPropertyFlags f) {
    mp.memoryTypes[i].heapIndex = heap_index;
    mp.memoryTypes[i].propertyFlags = f;
    if (i + 1 > mp.memoryTypeCount) mp.memoryTypeCount = i + 1;
}

}  // namespace

NXTEST(guard_discrete_gpu_discounts_the_largest_device_local_heap) {
    VkPhysicalDeviceMemoryProperties mp{};
    heap(mp, 0, 16 * kGiB, false);   // system RAM
    heap(mp, 1, 8 * kGiB, true);     // VRAM
    heap(mp, 2, 256ull << 20, true); // the host-visible VRAM window (small)
    mtype(mp, 0, 0, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT);
    mtype(mp, 1, 1, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);
    mtype(mp, 2, 2, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT | VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT);
    CHECK_EQ(nxrt::gfx::vk_guard_discount_heap(mp, VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU), 1);
}

NXTEST(guard_integrated_gpu_discounts_nothing) {
    // the one device-local heap is system RAM: every type on it is host-visible
    VkPhysicalDeviceMemoryProperties mp{};
    heap(mp, 0, 16 * kGiB, true);
    mtype(mp, 0, 0, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT | VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);
    CHECK_EQ(nxrt::gfx::vk_guard_discount_heap(mp, VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU), -1);
    CHECK_EQ(nxrt::gfx::vk_guard_discount_heap(mp, VK_PHYSICAL_DEVICE_TYPE_CPU), -1);
}

NXTEST(guard_non_discrete_gpu_with_a_private_device_local_heap) {
    VkPhysicalDeviceMemoryProperties mp{};
    heap(mp, 0, 16 * kGiB, true);
    heap(mp, 1, 2 * kGiB, true);
    mtype(mp, 0, 0, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT | VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT);
    mtype(mp, 1, 1, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);
    CHECK_EQ(nxrt::gfx::vk_guard_discount_heap(mp, VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU), 1);
}

NXTEST(guard_no_device_local_heap) {
    VkPhysicalDeviceMemoryProperties mp{};
    heap(mp, 0, 16 * kGiB, false);
    CHECK_EQ(nxrt::gfx::vk_guard_discount_heap(mp, VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU), -1);
}

NXTEST(guard_gpu_bytes_is_zero_without_an_allocator) {
    CHECK_EQ(nxrt::gfx::vk_guard_gpu_bytes(), 0ull);   // no device in this group: the null allocator reads as 0
}
