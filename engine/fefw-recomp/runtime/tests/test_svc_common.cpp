// SPDX-License-Identifier: GPL-3.0-or-later
// Service library framework: registration, events, shared memory, the title profile, the test client.
#include "svc_test_util.h"

#include <algorithm>

#include "nxrt/config.h"
#include "nxrt/game.h"

NXTEST_GROUP(svc_common)

using namespace nxrt;

NXTEST(common_connect_through_sm) {
    nxsvc::Env env;
    // A service nobody implements is still reachable as a stub (the old behaviour), a registered one is not a stub.
    nxsvc::Session s = nxsvc::connect("no:such");
    CHECK(s.valid());
    nxsvc::Reply r = s.call(0);
    CHECK_EQ(r.result, kernel::kStubService);
}

NXTEST(common_event_wait) {
    nxsvc::Env env;
    services::ServiceEvent ev;
    const uint32_t h = ev.new_handle();
    REQUIRE(h != 0);
    int32_t idx = -1;
    CHECK_EQ(kernel::wait_synchronization(&h, 1, 0, &idx), kernel::kTimedOut);
    ev.signal();
    CHECK_EQ(kernel::wait_synchronization(&h, 1, 0, &idx), 0u);
    CHECK_EQ(idx, 0);
    ev.clear();
    CHECK_EQ(kernel::wait_synchronization(&h, 1, 0, &idx), kernel::kTimedOut);
    // A second handle sees the same state.
    const uint32_t h2 = ev.new_handle();
    ev.signal();
    CHECK_EQ(kernel::wait_synchronization(&h2, 1, 0, &idx), 0u);
    kernel::handle_close(h);
    kernel::handle_close(h2);
}

NXTEST(common_shared_memory_map) {
    nxsvc::Env env;
    services::ServiceSharedMemory sm(0x3000, "test");
    REQUIRE(sm.ok());
    CHECK_EQ(sm.size(), 0x3000u);
    std::memset(reinterpret_cast<void*>(sm.host()), 0xAB, 16);
    const uint32_t h = sm.new_handle();
    REQUIRE(h != 0);
    auto obj = kernel::handle_as<kernel::KSharedMemory>(h);
    REQUIRE(obj != nullptr);
    CHECK_EQ(obj->size, 0x3000u);
    CHECK_EQ(reinterpret_cast<uint8_t*>(sm.host())[3], 0xAB);
    kernel::handle_close(h);
}

NXTEST(common_profile_defaults) {
    nxsvc::Env env;
    config_clear();
    services::profile_reload();
    const services::TitleProfile& p = services::profile();
    // the registered game's title defaults (game::Info::title_defaults), not one game's numbers
    services::TitleProfile want;
    if (game::info().title_defaults) game::info().title_defaults(want);
    CHECK_EQ(p.program_id, game::info().program_id);
    CHECK_EQ(p.account_save_size, want.account_save_size);
    CHECK_EQ(p.account_save_journal, want.account_save_journal);
    CHECK_EQ(static_cast<uint32_t>(p.language), static_cast<uint32_t>(services::Language::AmericanEnglish));
    CHECK(p.docked);
    CHECK_EQ(p.supported_languages, want.supported_languages);   // control.nacp SupportedLanguage
    CHECK_EQ(services::language_code_u64(services::Language::AmericanEnglish), 0x53552d6e65ull);   // "en-US"
}

NXTEST(common_profile_overrides) {
    nxsvc::Env env;
    config_clear();
    config_set("system", "language", "Japanese");
    config_set("system", "region", "Europe");
    config_set("am", "operation_mode", "handheld");
    config_set("account", "uid", "00000000000000010000000000000002");
    services::profile_reload();
    const services::TitleProfile& p = services::profile();
    CHECK_EQ(static_cast<uint32_t>(p.language), static_cast<uint32_t>(services::Language::Japanese));
    CHECK_EQ(p.region, 2);
    CHECK(!p.docked);
    CHECK_EQ(p.user_uid_hi, 1ull);
    CHECK_EQ(p.user_uid_lo, 2ull);
    config_clear();
    services::profile_reload();
}

NXTEST(common_program_id_default) {
    config_clear();
    CHECK_EQ(services::program_id_now(), game::info().program_id);
    config_set("process", "program_id", "0x1234");
    CHECK_EQ(services::program_id_now(), 0x1234ull);
    config_clear();
}
