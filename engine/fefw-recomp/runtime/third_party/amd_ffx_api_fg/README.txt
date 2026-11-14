AMD FidelityFX SDK v1.1.4 API headers for FSR 3 frame generation on Vulkan (ffx_api.h, ffx_api_types.h, ffx_framegeneration.h,
vk/ffx_api_vk.h), MIT licence (LICENSE.txt). Source: https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK tag v1.1.4, path
ffx-api/include/ffx_api. Unmodified. Kept apart from amd_ffx_api (v2.4.0, upscaler only): v2.x ships the frame-generation swapchain for
DX12 only, v1.1.4's amd_fidelityfx_vk.dll has it for Vulkan. Only src/gfx/extras/framegen_ffx.cpp includes these.
