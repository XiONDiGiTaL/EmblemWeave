// SPDX-License-Identifier: GPL-3.0-or-later
// Cross-service checks: every service in the title's Service Access Control list (game/exefs/main.npdm) answers, and the start-up
// sequence the real game performed (am launch parameter -> acc -> EnsureSaveData -> fsp-srv OpenSaveDataFileSystem) agrees on the user id.
#define _SILENCE_CXX20_U8PATH_DEPRECATION_WARNING
#include <windows.h>

#include <algorithm>
#include <cstring>
#include <filesystem>

#include "nxrt/config.h"
#include "nxrt/services_api.h"
#include "svc_test_util.h"

NXTEST_GROUP(svc_boot_flow)

using namespace nxrt;
using nxsvc::Args;
using nxsvc::Reply;
using nxsvc::Session;

NXTEST(sac_services_are_all_registered) {
    nxsvc::Env env;
    const std::vector<std::string> n = services::registered_names();
    // main.npdm ACI0 service access list, 46 names (read from the dump with the project's parser).
    const char* sac[] = {"acc:u0", "aoc:u", "apm", "appletOE", "audin:u", "audout:u", "audren:u", "banana", "bcat:u", "bsd:u", "bsdcfg", "caps:su",
                         "caps:u", "csrng", "ectx:aw", "friend:u", "fsp-srv", "hid", "htc", "htc:tenv", "htcs", "hwopus", "irs", "ldn:u", "ldr:ro", "lm",
                         "mii:u", "mm:u", "nfc:mf:u", "nfc:user", "nfp:user", "ngct:u", "nifm:u", "nim:eca", "notif:a", "nsd:u", "ntc", "nvdrv", "pcm",
                         "pctl", "pl:u", "prepo:u", "set", "sfdnsres", "ssl", "time:u", "vi:u"};
    CHECK_EQ(sizeof(sac) / sizeof(sac[0]), 47u);   // 47 names: the list printed by the dump parser has one more entry than the plan's "46"
    for (const char* name : sac) CHECK_MSG(std::find(n.begin(), n.end(), name) != n.end(), std::string("not registered: ") + name);
    // The SDK's client library also reaches these without the title being allowed to use them; they exist so nothing falls over.
    for (const char* name : {"set:sys", "psm", "fatal:u", "time:a", "time:s", "ngc:u"}) CHECK_MSG(std::find(n.begin(), n.end(), name) != n.end(), std::string("not registered: ") + name);
}

NXTEST(sac_services_answer_an_unknown_command_without_crashing) {
    nxsvc::Env env;
    for (const std::string& name : services::registered_names()) {
        Session s = nxsvc::connect(name.c_str());
        CHECK_MSG(s.valid(), "connect failed: " + name);
        if (!s.valid()) continue;
        Reply r = s.call(0x7FFF0);   // an id no service has
        CHECK_MSG(r.svc_rc == 0, "SendSyncRequest failed on " + name);
        CHECK_MSG(r.result == kernel::kStubService, "unknown command id not answered with the stub result on " + name);
    }
}

NXTEST(startup_user_id_agrees_between_am_acc_and_fs) {
    nxsvc::Env env;
    const std::string dir = (std::filesystem::temp_directory_path() / ("nxrt_flow_" + std::to_string(GetCurrentProcessId()))).string();
    std::filesystem::remove_all(dir);
    config_set("fs", "save_dir", dir.c_str());
    services::profile_reload();

    // appletOE: OpenApplicationProxy(pid, process handle) -> IApplicationProxy -> GetApplicationFunctions
    Session oe = nxsvc::connect("appletOE");
    REQUIRE(oe.valid());
    Reply r = oe.call(0, Args().with_pid().u64(0).hcopy(0xFFFF8001));
    REQUIRE(r.ok());
    Session proxy = oe.sub(r);
    Session app = proxy.sub(proxy.call(20));
    REQUIRE(app.valid());

    // PopLaunchParameter(PreselectedUser): magic, flag, uid
    Session storage = app.sub(app.call(1, Args().u32(2)));
    REQUIRE(storage.valid());
    Session accessor = storage.sub(storage.call(0));
    REQUIRE(accessor.valid());
    uint8_t lp[0x88] = {};
    REQUIRE(accessor.call(11, Args().i64(0).buf_b(lp, sizeof lp)).ok());
    uint32_t magic;
    std::memcpy(&magic, lp, 4);
    CHECK_EQ(magic, 0xC79497CAu);
    uint8_t uid[16];
    std::memcpy(uid, lp + 8, 16);

    // acc:u0 must know that user: GetBaasAccountManagerForApplication(uid), GetUserExistence(uid)
    Session acc = nxsvc::connect("acc:u0");
    REQUIRE(acc.valid());
    Args a;
    a.bytes(uid, 16);
    CHECK(acc.call(1, a).ok());
    CHECK(acc.call(1, a).get<uint8_t>(0) == 1);
    CHECK(acc.call(101, a).ok());

    // EnsureSaveData(uid) then OpenSaveDataFileSystem(User, account attribute of that uid) must find the save
    uint64_t hi, lo;
    std::memcpy(&hi, uid, 8);
    std::memcpy(&lo, uid + 8, 8);
    CHECK(app.call(20, Args().u64(hi).u64(lo)).ok());
    Session fsp = nxsvc::connect("fsp-srv");
    REQUIRE(fsp.valid());
    Args o;
    o.u8(1).pad(7);
    uint8_t attr[0x40] = {};
    std::memcpy(attr + 8, uid, 16);
    attr[32] = 1;   // SaveDataType::Account
    o.bytes(attr, sizeof attr);
    Reply s = fsp.call(51, o);
    CHECK_EQ(s.result, 0u);

    std::filesystem::remove_all(dir);
    config_clear();
    services::profile_reload();
}
