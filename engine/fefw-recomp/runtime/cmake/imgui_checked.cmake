# Local failure-handling patch for the pinned ImGui Vulkan backend. Fail on pin drift.
file(READ "${imgui_SOURCE_DIR}/backends/imgui_impl_vulkan.cpp" backend)
set(before [====[static void CreateOrResizeBuffer(VkBuffer& buffer, VkDeviceMemory& buffer_memory, VkDeviceSize& buffer_size, VkDeviceSize new_size, VkBufferUsageFlagBits usage)
{
    ImGui_ImplVulkan_Data* bd = ImGui_ImplVulkan_GetBackendData();
    ImGui_ImplVulkan_InitInfo* v = &bd->VulkanInitInfo;
    VkResult err;
    if (buffer != VK_NULL_HANDLE)
        vkDestroyBuffer(v->Device, buffer, v->Allocator);
    if (buffer_memory != VK_NULL_HANDLE)
        vkFreeMemory(v->Device, buffer_memory, v->Allocator);

    VkDeviceSize buffer_size_aligned = AlignBufferSize(IM_MAX(v->MinAllocationSize, new_size), bd->BufferMemoryAlignment);
    VkBufferCreateInfo buffer_info = {};
    buffer_info.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;
    buffer_info.size = buffer_size_aligned;
    buffer_info.usage = usage;
    buffer_info.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
    err = vkCreateBuffer(v->Device, &buffer_info, v->Allocator, &buffer);
    check_vk_result(err);

    VkMemoryRequirements req;
    vkGetBufferMemoryRequirements(v->Device, buffer, &req);
    bd->BufferMemoryAlignment = (bd->BufferMemoryAlignment > req.alignment) ? bd->BufferMemoryAlignment : req.alignment;
    VkMemoryAllocateInfo alloc_info = {};
    alloc_info.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
    alloc_info.allocationSize = req.size;
    alloc_info.memoryTypeIndex = ImGui_ImplVulkan_MemoryType(VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT, req.memoryTypeBits);
    err = vkAllocateMemory(v->Device, &alloc_info, v->Allocator, &buffer_memory);
    check_vk_result(err);

    err = vkBindBufferMemory(v->Device, buffer, buffer_memory, 0);
    check_vk_result(err);
    buffer_size = buffer_size_aligned;
}

]====])
set(after [====[static bool CreateOrResizeBuffer(VkBuffer& buffer, VkDeviceMemory& buffer_memory, VkDeviceSize& buffer_size, VkDeviceSize new_size, VkBufferUsageFlagBits usage)
{
    ImGui_ImplVulkan_Data* bd = ImGui_ImplVulkan_GetBackendData();
    ImGui_ImplVulkan_InitInfo* v = &bd->VulkanInitInfo;
    VkResult err;
    VkBuffer next_buffer = VK_NULL_HANDLE;
    VkDeviceMemory next_memory = VK_NULL_HANDLE;
    VkDeviceSize buffer_size_aligned = AlignBufferSize(IM_MAX(v->MinAllocationSize, new_size), bd->BufferMemoryAlignment);
    VkBufferCreateInfo buffer_info = {};
    buffer_info.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;
    buffer_info.size = buffer_size_aligned;
    buffer_info.usage = usage;
    buffer_info.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
    err = vkCreateBuffer(v->Device, &buffer_info, v->Allocator, &next_buffer);
    check_vk_result(err);
    if (err != VK_SUCCESS) {
        if (next_buffer) vkDestroyBuffer(v->Device, next_buffer, v->Allocator);
        if (next_memory) vkFreeMemory(v->Device, next_memory, v->Allocator);
        return false;
    }

    VkMemoryRequirements req;
    vkGetBufferMemoryRequirements(v->Device, next_buffer, &req);
    bd->BufferMemoryAlignment = (bd->BufferMemoryAlignment > req.alignment) ? bd->BufferMemoryAlignment : req.alignment;
    VkMemoryAllocateInfo alloc_info = {};
    alloc_info.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
    alloc_info.allocationSize = req.size;
    alloc_info.memoryTypeIndex = ImGui_ImplVulkan_MemoryType(VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT, req.memoryTypeBits);
    err = vkAllocateMemory(v->Device, &alloc_info, v->Allocator, &next_memory);
    check_vk_result(err);
    if (err != VK_SUCCESS) {
        if (next_buffer) vkDestroyBuffer(v->Device, next_buffer, v->Allocator);
        if (next_memory) vkFreeMemory(v->Device, next_memory, v->Allocator);
        return false;
    }

    err = vkBindBufferMemory(v->Device, next_buffer, next_memory, 0);
    check_vk_result(err);
    if (err != VK_SUCCESS) {
        if (next_buffer) vkDestroyBuffer(v->Device, next_buffer, v->Allocator);
        if (next_memory) vkFreeMemory(v->Device, next_memory, v->Allocator);
        return false;
    }
    if (buffer) vkDestroyBuffer(v->Device, buffer, v->Allocator);
    if (buffer_memory) vkFreeMemory(v->Device, buffer_memory, v->Allocator);
    buffer = next_buffer;
    buffer_memory = next_memory;
    buffer_size = buffer_size_aligned;
    return true;
}

]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[            CreateOrResizeBuffer(rb->VertexBuffer, rb->VertexBufferMemory, rb->VertexBufferSize, vertex_size, VK_BUFFER_USAGE_VERTEX_BUFFER_BIT);]====])
set(after [====[            if (!CreateOrResizeBuffer(rb->VertexBuffer, rb->VertexBufferMemory, rb->VertexBufferSize, vertex_size, VK_BUFFER_USAGE_VERTEX_BUFFER_BIT)) return;]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[            CreateOrResizeBuffer(rb->IndexBuffer, rb->IndexBufferMemory, rb->IndexBufferSize, index_size, VK_BUFFER_USAGE_INDEX_BUFFER_BIT);]====])
set(after [====[            if (!CreateOrResizeBuffer(rb->IndexBuffer, rb->IndexBufferMemory, rb->IndexBufferSize, index_size, VK_BUFFER_USAGE_INDEX_BUFFER_BIT)) return;]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[            err = vkAllocateMemory(v->Device, &alloc_info, v->Allocator, &backend_tex->Memory);
            check_vk_result(err);]====])
set(after [====[            err = vkAllocateMemory(v->Device, &alloc_info, v->Allocator, &backend_tex->Memory);
            check_vk_result(err);
            if (err != VK_SUCCESS) {
                vkDestroyImage(v->Device, backend_tex->Image, v->Allocator);
                IM_DELETE(backend_tex);
                return;
            }]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[            err = vkAllocateMemory(v->Device, &alloc_info, v->Allocator, &upload_buffer_memory);
            check_vk_result(err);]====])
set(after [====[            err = vkAllocateMemory(v->Device, &alloc_info, v->Allocator, &upload_buffer_memory);
            check_vk_result(err);
            if (err != VK_SUCCESS) {
                vkDestroyBuffer(v->Device, upload_buffer, v->Allocator);
                return;
            }]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[                ImGui_ImplVulkan_UpdateTexture(tex);

    ImGui_ImplVulkan_Data*]====])
set(after [====[                ImGui_ImplVulkan_UpdateTexture(tex);
    if (draw_data->Textures != nullptr)
        for (ImTextureData* tex : *draw_data->Textures)
            if (tex->Status == ImTextureStatus_WantCreate || tex->Status == ImTextureStatus_WantUpdates)
                return; // Retry next frame before binding any incomplete texture.

    ImGui_ImplVulkan_Data*]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[    if (tex->Status == ImTextureStatus_WantCreate)
    {]====])
set(after [====[    if (tex->Status == ImTextureStatus_WantCreate && tex->BackendUserData == nullptr)
    {]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[            err = vkCreateImage(v->Device, &info, v->Allocator, &backend_tex->Image);
            check_vk_result(err);]====])
set(after [====[            err = vkCreateImage(v->Device, &info, v->Allocator, &backend_tex->Image);
            check_vk_result(err);
            if (err != VK_SUCCESS) { IM_DELETE(backend_tex); return; }]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[            err = vkBindImageMemory(v->Device, backend_tex->Image, backend_tex->Memory, 0);
            check_vk_result(err);]====])
set(after [====[            err = vkBindImageMemory(v->Device, backend_tex->Image, backend_tex->Memory, 0);
            check_vk_result(err);
            if (err != VK_SUCCESS) { vkDestroyImage(v->Device, backend_tex->Image, v->Allocator); vkFreeMemory(v->Device, backend_tex->Memory, v->Allocator); IM_DELETE(backend_tex); return; }]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[            err = vkCreateImageView(v->Device, &info, v->Allocator, &backend_tex->ImageView);
            check_vk_result(err);]====])
set(after [====[            err = vkCreateImageView(v->Device, &info, v->Allocator, &backend_tex->ImageView);
            check_vk_result(err);
            if (err != VK_SUCCESS) { vkDestroyImage(v->Device, backend_tex->Image, v->Allocator); vkFreeMemory(v->Device, backend_tex->Memory, v->Allocator); IM_DELETE(backend_tex); return; }]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[            err = vkCreateBuffer(v->Device, &buffer_info, v->Allocator, &upload_buffer);
            check_vk_result(err);]====])
set(after [====[            err = vkCreateBuffer(v->Device, &buffer_info, v->Allocator, &upload_buffer);
            check_vk_result(err);
            if (err != VK_SUCCESS) {  return; }]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[            err = vkBindBufferMemory(v->Device, upload_buffer, upload_buffer_memory, 0);
            check_vk_result(err);]====])
set(after [====[            err = vkBindBufferMemory(v->Device, upload_buffer, upload_buffer_memory, 0);
            check_vk_result(err);
            if (err != VK_SUCCESS) { vkDestroyBuffer(v->Device, upload_buffer, v->Allocator); vkFreeMemory(v->Device, upload_buffer_memory, v->Allocator); return; }]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[            err = vkMapMemory(v->Device, upload_buffer_memory, 0, upload_size, 0, (void**)(&map));
            check_vk_result(err);]====])
set(after [====[            err = vkMapMemory(v->Device, upload_buffer_memory, 0, upload_size, 0, (void**)(&map));
            check_vk_result(err);
            if (err != VK_SUCCESS) { vkDestroyBuffer(v->Device, upload_buffer, v->Allocator); vkFreeMemory(v->Device, upload_buffer_memory, v->Allocator); return; }]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[        check_vk_result(err);
        err = vkMapMemory(v->Device, rb->IndexBufferMemory]====])
set(after [====[        check_vk_result(err);
        if (err != VK_SUCCESS) return;
        err = vkMapMemory(v->Device, rb->IndexBufferMemory]====])
string(FIND "${backend}" "${before}" found)
if(found EQUAL -1)
 message(FATAL_ERROR "ImGui Vulkan failure patch no longer matches pinned source")
endif()
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[        check_vk_result(err);
        for (const ImDrawList* draw_list]====])
set(after [====[        check_vk_result(err);
        if (err != VK_SUCCESS) { vkUnmapMemory(v->Device, rb->VertexBufferMemory); return; }
        for (const ImDrawList* draw_list]====])
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[    VkDescriptorSet descriptor_set;]====])
set(after [====[    VkDescriptorSet descriptor_set = VK_NULL_HANDLE;]====])
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[        VkResult err = vkAllocateDescriptorSets(v->Device, &alloc_info, &descriptor_set);
        check_vk_result(err);]====])
set(after [====[        VkResult err = vkAllocateDescriptorSets(v->Device, &alloc_info, &descriptor_set);
        check_vk_result(err);
        if (err != VK_SUCCESS) return VK_NULL_HANDLE;]====])
string(REPLACE "${before}" "${after}" backend "${backend}")
set(before [====[        backend_tex->DescriptorSet = ImGui_ImplVulkan_AddTexture(backend_tex->ImageView, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);]====])
set(after [====[        backend_tex->DescriptorSet = ImGui_ImplVulkan_AddTexture(backend_tex->ImageView, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);
        if (!backend_tex->DescriptorSet) {
            vkDestroyImageView(v->Device, backend_tex->ImageView, v->Allocator);
            vkDestroyImage(v->Device, backend_tex->Image, v->Allocator);
            vkFreeMemory(v->Device, backend_tex->Memory, v->Allocator);
            IM_DELETE(backend_tex);
            return;
        }]====])
string(REPLACE "${before}" "${after}" backend "${backend}")
file(WRITE "${CMAKE_CURRENT_BINARY_DIR}/imgui_impl_vulkan_checked.cpp" "${backend}")
