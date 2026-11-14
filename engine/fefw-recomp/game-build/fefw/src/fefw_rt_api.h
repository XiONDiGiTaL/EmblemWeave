// SPDX-License-Identifier: GPL-3.0-or-later
// The one call between fefw.exe (linked on the player's PC: the lifted game libraries + fefw_stub.cpp) and fefw_rt.dll (the runtime,
// shipped prebuilt with no game code). Plain C, so the two sides may come from different compilers (zig/clang mingw for the exe,
// clang-cl for the DLL). Everything else the lifted code calls (recomp ctx.h: call_indirect, host_import, guest_svc, ...) is exported by
// fefw_rt.dll as well (src/fefw_rt.def).
#pragma once
#include <stdint.h>

#define FEFW_RT_API 1u     // bump when this struct or the exported set changes

#ifdef __cplusplus
extern "C" {
#endif
struct NxModule;
typedef struct FefwRtModules {
    uint32_t size;         // sizeof(FefwRtModules)
    uint32_t api;          // FEFW_RT_API
    const struct NxModule *rtld, *main, *sdk, *subsdk0, *subsdk1, *subsdk2;
    const uint32_t *rtld_abi, *main_abi, *sdk_abi, *subsdk0_abi, *subsdk1_abi, *subsdk2_abi;
} FefwRtModules;

int fefw_rt_main(int argc, char** argv, const FefwRtModules* mods);
#ifdef __cplusplus
}
#endif
