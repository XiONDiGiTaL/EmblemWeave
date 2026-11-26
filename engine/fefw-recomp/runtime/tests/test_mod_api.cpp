// SPDX-License-Identifier: GPL-3.0-or-later
// NxModApi version 6: the appended get_interface member, its registry, and the layout every existing mod relies on.
// The fixture DLL (tests/mods/test_mod.cpp) is the other side of the boundary: a real mod binary built against this
// header, called with the same table load_dll_mods builds.
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <string>

#include "nxrt/mod_api.h"
#include "nxrt/mod_interfaces.h"
#include "nxrt/services_api.h"
#include "test_framework.h"

#ifdef _WIN32
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>

#include <filesystem>
#endif

NXTEST_GROUP(mod_api)

namespace {

// The version-5 table: every member EmblemWeave exposed before v6, in the same order. These assertions are the
// compatibility test an existing mod binary relies on: version 6 appends one pointer and moves nothing.
struct NxModApiV5 {
    uint32_t version;
    uint32_t size;
    const char* game_id;
    uint64_t main_base;
    void (*log)(const NxModApi*, int, const char*);
    uint32_t (*hook_inline)(uint64_t, NxModInlineFn, void*);
    uint32_t (*hook_mid)(uint64_t, NxModSiteFn, void*);
    uint32_t (*hook_entry)(uint64_t, NxModSiteFn, void*);
    int (*hook_remove)(uint32_t);
    uint64_t (*call_guest)(Ctx*, uint64_t, const uint64_t*, int);
    int (*is_mapped)(uint64_t, uint64_t);
    void (*on_frame)(NxModFrameFn, void*);
    int (*is_function)(uint64_t);
    uint64_t (*make_function)(NxModFunctionFn, void*);
    int (*make_writable)(uint64_t, uint64_t);
    int (*edit_file)(const char*, NxModFileFn, void*);
    uint32_t (*mod_count)();
    const char* (*mod_folder)(uint32_t);
    int64_t (*read_file)(const char*, void*, uint64_t);
    int (*write_file)(const char*, const void*, uint64_t);
    int64_t (*zstd_decompress)(const void*, uint64_t, void*, uint64_t);
    int64_t (*zstd_compress)(const void*, uint64_t, void*, uint64_t, int);
    void* (*alloc)(uint64_t);
    void (*free_mem)(void*);
    int (*add_panel)(const char*, NxModPanelFn, void*);
    int (*panel_visible)();
};

#define SAME_OFFSET(field) static_assert(offsetof(NxModApi, field) == offsetof(NxModApiV5, field), #field " must not move")
SAME_OFFSET(version);
SAME_OFFSET(size);
SAME_OFFSET(game_id);
SAME_OFFSET(main_base);
SAME_OFFSET(log);
SAME_OFFSET(hook_inline);
SAME_OFFSET(hook_mid);
SAME_OFFSET(hook_entry);
SAME_OFFSET(hook_remove);
SAME_OFFSET(call_guest);
SAME_OFFSET(is_mapped);
SAME_OFFSET(on_frame);
SAME_OFFSET(is_function);
SAME_OFFSET(make_function);
SAME_OFFSET(make_writable);
SAME_OFFSET(edit_file);
SAME_OFFSET(mod_count);
SAME_OFFSET(mod_folder);
SAME_OFFSET(read_file);
SAME_OFFSET(write_file);
SAME_OFFSET(zstd_decompress);
SAME_OFFSET(zstd_compress);
SAME_OFFSET(alloc);
SAME_OFFSET(free_mem);
SAME_OFFSET(add_panel);
SAME_OFFSET(panel_visible);
#undef SAME_OFFSET

static_assert(sizeof(NxModApiV5) == offsetof(NxModApi, get_interface),
              "get_interface must be appended directly after panel_visible");
static_assert(sizeof(NxModApi) == offsetof(NxModApi, get_interface) + sizeof(NxModGetInterfaceFn),
              "get_interface is the last member of version 6");
static_assert(NXMOD_API_VERSION == 6, "this build implements version 6");

struct TestTable {
    uint32_t size;
    uint32_t version;
    uint64_t value;
};

TestTable g_table_v1{sizeof(TestTable), 1, 0x1111};
TestTable g_table_v2{sizeof(TestTable), 2, 0x2222};
TestTable g_table_v3{sizeof(TestTable), 3, 0x3333};

}  // namespace

NXTEST(version_and_layout) {
    CHECK_EQ(NXMOD_API_VERSION, 6u);
    CHECK_EQ(sizeof(NxModApi), offsetof(NxModApi, get_interface) + sizeof(NxModGetInterfaceFn));
    CHECK_EQ(offsetof(NxModApi, get_interface), sizeof(NxModApiV5));
    // The version-4 tail is still where existing mods read it.
    CHECK_EQ(offsetof(NxModApi, add_panel), offsetof(NxModApiV5, add_panel));
    CHECK_EQ(offsetof(NxModApi, panel_visible), offsetof(NxModApiV5, panel_visible));
}

NXTEST(registry_add_and_find) {
    nxrt::nxmod::InterfaceRegistry reg;
    CHECK(reg.empty());
    CHECK(reg.add("emblemweave.test", 2, &g_table_v2));
    CHECK_EQ(reg.count(), 1u);

    CHECK(reg.find("emblemweave.test", 2, nullptr) == &g_table_v2);
    CHECK(reg.find("emblemweave.test", 0, nullptr) == &g_table_v2);   // 0 = any
    CHECK(reg.find("emblemweave.test", 1, nullptr) == &g_table_v2);   // an older request gets the newer table
    CHECK(reg.find("emblemweave.test", 3, nullptr) == nullptr);       // that version does not exist
    CHECK(reg.find("emblemweave.other", 0, nullptr) == nullptr);
    CHECK(reg.find("EmblemWeave.test", 0, nullptr) == nullptr);       // case-sensitive
    CHECK(reg.find("", 0, nullptr) == nullptr);
    CHECK(reg.find(nullptr, 0, nullptr) == nullptr);
}

NXTEST(registry_version_compatibility) {
    nxrt::nxmod::InterfaceRegistry reg;
    CHECK(reg.add("obeliskchamber.list", 3, &g_table_v3));
    CHECK(reg.find("obeliskchamber.list", 1, nullptr) == &g_table_v3);
    CHECK(reg.find("obeliskchamber.list", 2, nullptr) == &g_table_v3);
    CHECK(reg.find("obeliskchamber.list", 3, nullptr) == &g_table_v3);
    CHECK(reg.find("obeliskchamber.list", 4, nullptr) == nullptr);
}

NXTEST(registry_rejects_bad_registrations) {
    nxrt::nxmod::InterfaceRegistry reg;
    CHECK(!reg.add(nullptr, 1, &g_table_v1));
    CHECK(!reg.add("", 1, &g_table_v1));
    CHECK(!reg.add("nxp", 1, &g_table_v1));           // one segment
    CHECK(!reg.add("nxp.", 1, &g_table_v1));          // trailing dot
    CHECK(!reg.add(".nxp.ui", 1, &g_table_v1));       // leading dot
    CHECK(!reg.add("nxp..ui", 1, &g_table_v1));       // empty segment
    CHECK(!reg.add("NXP.ui", 1, &g_table_v1));        // upper-case
    CHECK(!reg.add("nxp.-ui", 1, &g_table_v1));       // segment starts with '-'
    CHECK(!reg.add("nxp.ui_", 1, &g_table_v1));       // not a segment character
    CHECK(!reg.add("nxp/ui", 1, &g_table_v1));
    CHECK(!reg.add("nxp ui", 1, &g_table_v1));
    CHECK(!reg.add("nxp.ui", 0, &g_table_v1));        // interface versions start at 1
    CHECK(!reg.add("nxp.ui", 1, nullptr));            // a table is required
    CHECK(!reg.add_resolver("nxp.ui", 1, nullptr));   // a resolver is required
    CHECK(reg.empty());

    std::string at_limit(120, 'a');
    at_limit += ".b";                                 // 122 bytes: still valid
    CHECK(nxrt::nxmod::valid_name(at_limit.c_str()));
    std::string too_long(127, 'a');
    too_long += ".b";                                 // 129 bytes: over the limit
    CHECK(!nxrt::nxmod::valid_name(too_long.c_str()));
}

NXTEST(registry_refuses_duplicates) {
    nxrt::nxmod::InterfaceRegistry reg;
    CHECK(reg.add("emblemweave.dup", 1, &g_table_v1));
    CHECK(!reg.add("emblemweave.dup", 1, &g_table_v1));   // same table is still a second registration
    CHECK(!reg.add("emblemweave.dup", 2, &g_table_v2));   // never replace the first table
    CHECK_EQ(reg.count(), 1u);
    CHECK(reg.find("emblemweave.dup", 0, nullptr) == &g_table_v1);
}

NXTEST(registry_resolver) {
    nxrt::nxmod::InterfaceRegistry reg;
    CHECK(reg.add_resolver("nxp.nxmod", 1, [](const void* context, void*) { return context; }));
    int sentinel = 0;
    CHECK(reg.find("nxp.nxmod", 0, &sentinel) == &sentinel);
    CHECK(reg.find("nxp.nxmod", 1, &sentinel) == &sentinel);
    CHECK(reg.find("nxp.nxmod", 2, &sentinel) == nullptr);
    CHECK(reg.find("nxp.nxmod", 1, nullptr) == nullptr);
}

NXTEST(bridge_nxp_nxmod) {
    NxModApi api{};
    api.version = NXMOD_API_VERSION;
    api.size = sizeof(NxModApi);
    CHECK(api.version >= 6);

    // The bridge answers the caller's own table, not a copy.
    CHECK(nxrt::nxmod::get_interface(&api, nxrt::nxmod::kNativeApi, 0) == &api);
    CHECK(nxrt::nxmod::get_interface(&api, nxrt::nxmod::kNativeApi, 1) == &api);
    CHECK(nxrt::nxmod::get_interface(&api, nxrt::nxmod::kNativeApi, 2) == nullptr);
    CHECK(nxrt::nxmod::get_interface(nullptr, nxrt::nxmod::kNativeApi, 0) == nullptr);

    // A host-specific name is not synthesized by this host.
    CHECK(nxrt::nxmod::get_interface(&api, "obeliskchamber.list", 0) == nullptr);
    CHECK(nxrt::nxmod::get_interface(&api, "fefw-recomp.sites", 1) == nullptr);
    CHECK(nxrt::nxmod::get_interface(&api, "NXP.nxmod", 0) == nullptr);
    CHECK(nxrt::nxmod::get_interface(&api, "", 0) == nullptr);
    CHECK(nxrt::nxmod::get_interface(&api, nullptr, 0) == nullptr);

    // A table registered by the host is reachable through the same lookup.
    CHECK(nxrt::nxmod::interfaces().add("emblemweave.probe", 2, &g_table_v2));
    CHECK(nxrt::nxmod::get_interface(&api, "emblemweave.probe", 0) == &g_table_v2);
    CHECK(nxrt::nxmod::get_interface(&api, "emblemweave.probe", 3) == nullptr);
}

NXTEST(mod_binaries) {
#ifndef _WIN32
    nxtest::skip("the fixture mod is a Windows DLL");
#else
#ifdef NXRT_TEST_MOD_PATH
    NxModApi api{};
    nxrt::services::modapi::make_mod_api(&api, 0x7100000000ull);
    CHECK_EQ(api.version, NXMOD_API_VERSION);
    CHECK_EQ(api.size, sizeof(NxModApi));
    CHECK(api.game_id != nullptr);
    CHECK(api.log && api.hook_inline && api.hook_mid && api.hook_entry && api.hook_remove);
    CHECK(api.call_guest && api.is_mapped && api.on_frame && api.is_function);
    CHECK(api.make_function && api.make_writable);
    CHECK(api.edit_file && api.mod_count && api.mod_folder);
    CHECK(api.read_file && api.write_file && api.zstd_decompress && api.zstd_compress);
    CHECK(api.alloc && api.free_mem && api.add_panel && api.panel_visible);
    CHECK(api.get_interface != nullptr);

    const std::filesystem::path path(std::string(NXRT_TEST_MOD_PATH));
    HMODULE dll = LoadLibraryExW(path.wstring().c_str(), nullptr,
                                 LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    CHECK_MSG(dll != nullptr, "LoadLibraryExW failed: " + std::to_string(GetLastError()));
    REQUIRE(dll != nullptr);

    using InitFn = int (*)(const NxModApi*, const char*);
    const auto v3 = reinterpret_cast<InitFn>(GetProcAddress(dll, "nxmod_init_v3"));
    const auto v4 = reinterpret_cast<InitFn>(GetProcAddress(dll, "nxmod_init"));
    const auto v6 = reinterpret_cast<InitFn>(GetProcAddress(dll, "nxmod_init_v6"));
    CHECK(v3 != nullptr);
    CHECK(v4 != nullptr);
    CHECK(v6 != nullptr);
    if (v3) CHECK_EQ(v3(&api, ""), 0);   // a version-3 mod sees its members where it left them
    if (v4) CHECK_EQ(v4(&api, ""), 0);   // a version-4 mod never reads the new member
    if (v6) CHECK_EQ(v6(&api, ""), 0);   // a version-6 mod gets the bridge and only this host's names
    FreeLibrary(dll);
#else
    nxtest::skip("NXRT_TEST_MOD_PATH is not defined");
#endif
#endif
}
