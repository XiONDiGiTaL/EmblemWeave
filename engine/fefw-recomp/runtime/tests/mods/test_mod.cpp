// SPDX-License-Identifier: GPL-3.0-or-later
// A code mod built against nxrt/mod_api.h, loaded by tests/test_mod_api.cpp through the same table load_dll_mods builds.
// It is deliberately small: each export checks the contract from the mod's side and returns 0, or the number of the check
// that failed. nxmod_init is an ordinary version-4 mod; nxmod_init_v3 only touches version-3 members.
#include <cstddef>
#include <cstdint>
#include <cstring>

#include "nxrt/mod_api.h"

#if defined(_WIN32)
#define TEST_MOD_EXPORT extern "C" __declspec(dllexport)
#else
#define TEST_MOD_EXPORT extern "C" __attribute__((visibility("default")))
#endif

namespace {

int check_v3(const NxModApi* api) {
    if (!api) return 1;
    if (api->version < 3) return 2;
    if (api->size < offsetof(NxModApi, free_mem) + sizeof(api->free_mem)) return 3;
    if (!api->log || !api->hook_inline || !api->hook_mid || !api->hook_entry || !api->hook_remove) return 4;
    if (!api->call_guest || !api->is_mapped || !api->on_frame || !api->is_function) return 5;
    if (!api->edit_file || !api->mod_count || !api->mod_folder) return 6;
    if (!api->read_file || !api->write_file || !api->zstd_decompress || !api->zstd_compress) return 7;
    if (!api->alloc || !api->free_mem) return 8;
    return 0;
}

}  // namespace

// A version-3 mod: it never reads a member added after free_mem.
TEST_MOD_EXPORT int nxmod_init_v3(const NxModApi* api, const char*) { return check_v3(api); }

// A version-4 mod: the members it uses are exactly the ones EmblemWeave exposed before version 6, and none moved.
TEST_MOD_EXPORT int nxmod_init(const NxModApi* api, const char*) {
    const int rc = check_v3(api);
    if (rc) return rc;
    if (api->version < 4) return 10;
    if (api->size < offsetof(NxModApi, panel_visible) + sizeof(api->panel_visible)) return 11;
    if (!api->add_panel || !api->panel_visible) return 12;
    return 0;
}

// A version-6 mod: the appended lookup, the bridge back to this table, and the names this host does not own.
TEST_MOD_EXPORT int nxmod_init_v6(const NxModApi* api, const char*) {
    const int rc = check_v3(api);
    if (rc) return rc;
    if (api->version < 6) return 20;
    if (api->size < offsetof(NxModApi, get_interface) + sizeof(api->get_interface)) return 21;
    if (!api->get_interface) return 22;
    if (api->get_interface(api, "nxp.nxmod", 0) != api) return 23;
    if (api->get_interface(api, "nxp.nxmod", 1) != api) return 24;
    if (api->get_interface(api, "nxp.nxmod", 2) != nullptr) return 25;
    if (api->get_interface(api, "obeliskchamber.list", 0) != nullptr) return 26;
    if (api->get_interface(api, "fefw-recomp.sites", 1) != nullptr) return 27;
    if (api->get_interface(api, "NXP.nxmod", 0) != nullptr) return 28;
    if (api->get_interface(api, "", 0) != nullptr) return 29;
    if (api->get_interface(api, nullptr, 0) != nullptr) return 30;
    return 0;
}
