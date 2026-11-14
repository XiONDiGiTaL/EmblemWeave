// SPDX-License-Identifier: GPL-3.0-or-later
// Power services (S3c): apm, apm:am, apm:p, apm:sys, psm and their sessions.
// Request bytes and expected replies are written from the Switchbrew tables (APM_services, PTM_services, Error_codes):
//   apm / apm:am IManager: 0 OpenSession, 1 GetPerformanceMode, 6 IsCpuOverclockEnabled
//   apm:p IManagerPrivileged: 0 OpenSession
//   ISession: 0 SetPerformanceConfiguration(u32 mode, u32 config), 1 GetPerformanceConfiguration(u32 mode) -> u32, 2 SetCpuOverclockEnabled(bool)
//   apm:sys ISystemManager: 0 RequestPerformanceMode(u32), 1 GetPerformanceEvent(u32 target) -> event, 2 GetThrottlingState,
//     3 GetLastThrottlingState, 4 ClearLastThrottlingState, 5 LoadAndApplySettings, 6 SetCpuBoostMode(u32), 7 GetCurrentPerformanceConfiguration
//   psm IPsmServer: 0 GetBatteryChargePercentage, 1 GetChargerType, 2 EnableBatteryCharging, 3 DisableBatteryCharging,
//     4 IsBatteryChargingEnabled, 7 OpenSession
//   psm IPsmSession: 0 BindStateChangeEvent, 1 UnbindStateChangeEvent, 2/3/4 Set*ChangeEventEnabled(bool)
// Values: PerformanceMode Normal 0, Boost 1; PerformanceConfiguration 0x00020003 is the Normal default (Switchbrew); ChargerType
// EnoughPower 1 (Switchbrew). The apm module is 148 (Switchbrew Error_codes); InvalidParameters = module | (1 << 9) = 0x294
// (description 1 from ryubing: Switchbrew lists no apm error rows, so that one value is not confirmed against Switchbrew).
#include "nxrt/services_api.h"
#include "svc_test_util.h"

NXTEST_GROUP(svc_power)

using namespace nxrt;

constexpr uint32_t kInvalidParameters = 0x294;

NXTEST(svc_power_apm_defaults) {
    nxsvc::Env env;
    nxsvc::Session apm = nxsvc::connect("apm");
    REQUIRE(apm.valid());
    nxsvc::Reply mode = apm.call(1);                    // GetPerformanceMode
    CHECK_EQ(mode.result, 0u);
    CHECK(mode.out.size() >= 4u);
    CHECK_EQ(mode.get<uint32_t>(0), 1u);                // docked by default: Boost
    nxsvc::Reply oc = apm.call(6);                      // IsCpuOverclockEnabled
    CHECK_EQ(oc.result, 0u);
    CHECK(oc.out.size() >= 1u);
    CHECK_EQ(oc.get<uint8_t>(0), 0u);
}

NXTEST(svc_power_session_configuration) {
    nxsvc::Env env;
    nxsvc::Session apm = nxsvc::connect("apm");
    REQUIRE(apm.valid());
    nxsvc::Reply open = apm.call(0);                    // OpenSession
    CHECK_EQ(open.result, 0u);
    REQUIRE(open.move.size() == 1);
    nxsvc::Session ses = apm.sub(open);
    // Normal default is 0x00020003 (Switchbrew).
    nxsvc::Reply g0 = ses.call(1, nxsvc::Args().u32(0));
    CHECK_EQ(g0.result, 0u);
    CHECK(g0.out.size() >= 4u);
    CHECK_EQ(g0.get<uint32_t>(0), 0x00020003u);
    // Set Boost, then read it back; Normal is untouched.
    nxsvc::Reply s1 = ses.call(0, nxsvc::Args().u32(1).u32(0x92220007));
    CHECK_EQ(s1.result, 0u);
    CHECK_EQ(ses.call(1, nxsvc::Args().u32(1)).get<uint32_t>(0), 0x92220007u);
    CHECK_EQ(ses.call(1, nxsvc::Args().u32(0)).get<uint32_t>(0), 0x00020003u);
    // Set Normal to 0x00020005 (CPU 1020 MHz, GPU 307.2 MHz, EMC 1065.6 MHz).
    CHECK_EQ(ses.call(0, nxsvc::Args().u32(0).u32(0x00020005)).result, 0u);
    CHECK_EQ(ses.call(1, nxsvc::Args().u32(0)).get<uint32_t>(0), 0x00020005u);
}

NXTEST(svc_power_session_invalid_mode) {
    nxsvc::Env env;
    nxsvc::Session apm = nxsvc::connect("apm");
    REQUIRE(apm.valid());
    nxsvc::Session ses = apm.sub(apm.call(0));
    // Mode 2 and mode -1 (0xFFFFFFFF) are not PerformanceMode values.
    CHECK_EQ(ses.call(0, nxsvc::Args().u32(2).u32(0x00020005)).result, kInvalidParameters);
    CHECK_EQ(ses.call(1, nxsvc::Args().u32(2)).result, kInvalidParameters);
    CHECK_EQ(ses.call(1, nxsvc::Args().u32(0xFFFFFFFFu)).result, kInvalidParameters);
    // The rejected Set changed nothing: Normal still reads its default.
    CHECK_EQ(ses.call(1, nxsvc::Args().u32(0)).get<uint32_t>(0), 0x00020003u);
}

NXTEST(svc_power_cpu_overclock_flag) {
    nxsvc::Env env;
    nxsvc::Session apm = nxsvc::connect("apm");
    REQUIRE(apm.valid());
    nxsvc::Session ses = apm.sub(apm.call(0));
    CHECK_EQ(ses.call(2, nxsvc::Args().u8(1)).result, 0u);     // SetCpuOverclockEnabled(true), no output
    nxsvc::Reply on = apm.call(6);
    CHECK_EQ(on.result, 0u);
    CHECK_EQ(on.get<uint8_t>(0), 1u);
    CHECK_EQ(ses.call(2, nxsvc::Args().u8(0)).result, 0u);
    CHECK_EQ(apm.call(6).get<uint8_t>(0), 0u);
}

NXTEST(svc_power_apm_am_and_privileged) {
    nxsvc::Env env;
    services::amctl::set_docked(false);   // handheld: apm starts in Normal
    // apm:am is the same IManager as apm (8.0+): the same mode and overclock state.
    nxsvc::Session am = nxsvc::connect("apm:am");
    REQUIRE(am.valid());
    CHECK_EQ(am.call(1).get<uint32_t>(0), 0u);
    nxsvc::Reply open = am.call(0);
    CHECK_EQ(open.result, 0u);
    REQUIRE(open.move.size() == 1);
    nxsvc::Session ses = am.sub(open);
    CHECK_EQ(ses.call(2, nxsvc::Args().u8(1)).result, 0u);
    CHECK_EQ(am.call(6).get<uint8_t>(0), 1u);
    // apm:p (1.0.0-7.0.1, OpenSession only): OpenSession answers an ISession, command 6 is not in its table.
    nxsvc::Session p = nxsvc::connect("apm:p");
    REQUIRE(p.valid());
    nxsvc::Reply popen = p.call(0);
    CHECK_EQ(popen.result, 0u);
    REQUIRE(popen.move.size() == 1);
    CHECK_EQ(p.call(6).result, kernel::kStubService);
}

NXTEST(svc_power_sys_performance_mode_and_config) {
    nxsvc::Env env;
    services::amctl::set_docked(false);   // handheld: apm starts in Normal
    nxsvc::Session apm = nxsvc::connect("apm");
    nxsvc::Session sys = nxsvc::connect("apm:sys");
    REQUIRE(apm.valid());
    REQUIRE(sys.valid());
    nxsvc::Session ses = apm.sub(apm.call(0));
    CHECK_EQ(ses.call(0, nxsvc::Args().u32(1).u32(0x92220007)).result, 0u);   // Boost config
    // Current configuration follows the requested mode.
    CHECK_EQ(sys.call(7).get<uint32_t>(0), 0x00020003u);                    // Normal (default mode)
    nxsvc::Reply req = sys.call(0, nxsvc::Args().u32(1));                   // RequestPerformanceMode(Boost)
    CHECK_EQ(req.result, 0u);
    CHECK_EQ(apm.call(1).get<uint32_t>(0), 1u);                             // GetPerformanceMode sees Boost
    CHECK_EQ(sys.call(7).result, 0u);
    CHECK_EQ(sys.call(7).get<uint32_t>(0), 0x92220007u);
    CHECK_EQ(sys.call(0, nxsvc::Args().u32(0)).result, 0u);                 // back to Normal
    CHECK_EQ(apm.call(1).get<uint32_t>(0), 0u);
    CHECK_EQ(sys.call(7).get<uint32_t>(0), 0x00020003u);
}

NXTEST(svc_power_sys_throttling_and_settings) {
    nxsvc::Env env;
    nxsvc::Session sys = nxsvc::connect("apm:sys");
    REQUIRE(sys.valid());
    // ThrottlingState is 0x28 bytes, all zero: nothing is throttled.
    nxsvc::Reply t = sys.call(2);
    CHECK_EQ(t.result, 0u);
    REQUIRE(t.out.size() >= 0x28u);
    for (size_t i = 0; i < 0x28; ++i) CHECK_EQ(t.out[i], 0u);
    nxsvc::Reply last = sys.call(3);
    CHECK_EQ(last.result, 0u);
    REQUIRE(last.out.size() >= 0x28u);
    for (size_t i = 0; i < 0x28; ++i) CHECK_EQ(last.out[i], 0u);
    CHECK_EQ(sys.call(4).result, 0u);                       // ClearLastThrottlingState
    CHECK_EQ(sys.call(5).result, 0u);                       // LoadAndApplySettings
    nxsvc::Reply boost = sys.call(6, nxsvc::Args().u32(1)); // SetCpuBoostMode(Boost CPU)
    CHECK_EQ(boost.result, 0u);
}

NXTEST(svc_power_sys_performance_event) {
    nxsvc::Env env;
    nxsvc::Session apm = nxsvc::connect("apm");
    nxsvc::Session sys = nxsvc::connect("apm:sys");
    REQUIRE(apm.valid());
    REQUIRE(sys.valid());
    // GetPerformanceEvent(PerformanceModeChanged = 0) returns one copy handle; it is valid and not signaled.
    nxsvc::Reply ev = sys.call(1, nxsvc::Args().u32(0));
    CHECK_EQ(ev.result, 0u);
    REQUIRE(ev.copy.size() == 1);
    const uint32_t h = ev.copy[0];
    int32_t idx = -1;
    CHECK_EQ(kernel::wait_synchronization(&h, 1, 0, &idx), kernel::kTimedOut);
    // A mode change does not signal it (the event never fires in this console).
    CHECK_EQ(sys.call(0, nxsvc::Args().u32(1)).result, 0u);
    CHECK_EQ(kernel::wait_synchronization(&h, 1, 0, &idx), kernel::kTimedOut);
    // Target 3 does not exist.
    nxsvc::Reply bad = sys.call(1, nxsvc::Args().u32(3));
    CHECK_EQ(bad.result, kInvalidParameters);
    CHECK_EQ(bad.copy.size(), 0u);
    kernel::handle_close(h);
}

NXTEST(svc_power_unknown_command) {
    nxsvc::Env env;
    nxsvc::Session apm = nxsvc::connect("apm");
    nxsvc::Session sys = nxsvc::connect("apm:sys");
    nxsvc::Session psm = nxsvc::connect("psm");
    REQUIRE(apm.valid());
    REQUIRE(sys.valid());
    REQUIRE(psm.valid());
    CHECK_EQ(apm.call(99).result, kernel::kStubService);
    // 8 GetCurrentPerformanceMode is 17.0.0+: not in the 12.x table we serve.
    CHECK_EQ(sys.call(8).result, kernel::kStubService);
    nxsvc::Session ses = apm.sub(apm.call(0));
    CHECK_EQ(ses.call(42).result, kernel::kStubService);
    nxsvc::Session pses = psm.sub(psm.call(7));
    CHECK_EQ(pses.call(42).result, kernel::kStubService);
    CHECK_EQ(psm.call(42).result, kernel::kStubService);
}

NXTEST(svc_power_psm_server) {
    nxsvc::Env env;
    nxsvc::Session psm = nxsvc::connect("psm");
    REQUIRE(psm.valid());
    // Battery is always 100 percent: u32 100 = 0x64.
    nxsvc::Reply pct = psm.call(0);
    CHECK_EQ(pct.result, 0u);
    CHECK(pct.out.size() >= 4u);
    CHECK_EQ(pct.get<uint32_t>(0), 100u);
    // Docked: an official charger, ChargerType EnoughPower = 1.
    nxsvc::Reply ct = psm.call(1);
    CHECK_EQ(ct.result, 0u);
    CHECK_EQ(ct.get<uint32_t>(0), 1u);
    // Charging starts enabled; disable and enable switch it, IsBatteryChargingEnabled reads it back.
    nxsvc::Reply en0 = psm.call(4);
    CHECK_EQ(en0.result, 0u);
    CHECK(en0.out.size() >= 1u);
    CHECK_EQ(en0.get<uint8_t>(0), 1u);
    CHECK_EQ(psm.call(3).result, 0u);                  // DisableBatteryCharging
    CHECK_EQ(psm.call(4).get<uint8_t>(0), 0u);
    CHECK_EQ(psm.call(2).result, 0u);                  // EnableBatteryCharging
    CHECK_EQ(psm.call(4).get<uint8_t>(0), 1u);
}

NXTEST(svc_power_psm_session_events) {
    nxsvc::Env env;
    nxsvc::Session psm = nxsvc::connect("psm");
    REQUIRE(psm.valid());
    nxsvc::Reply open = psm.call(7);                   // OpenSession
    CHECK_EQ(open.result, 0u);
    REQUIRE(open.move.size() == 1);
    nxsvc::Session ses = psm.sub(open);
    // BindStateChangeEvent: a copy handle, valid and not signaled.
    nxsvc::Reply bind = ses.call(0);
    CHECK_EQ(bind.result, 0u);
    REQUIRE(bind.copy.size() == 1);
    const uint32_t h = bind.copy[0];
    int32_t idx = -1;
    CHECK_EQ(kernel::wait_synchronization(&h, 1, 0, &idx), kernel::kTimedOut);
    // The three Set*Enabled commands take a bool and have no output.
    for (uint32_t cmd : {2u, 3u, 4u}) {
        nxsvc::Reply r = ses.call(cmd, nxsvc::Args().u8(1));
        CHECK_EQ(r.result, 0u);
    }
    CHECK_EQ(ses.call(2, nxsvc::Args().u8(0)).result, 0u);
    CHECK_EQ(kernel::wait_synchronization(&h, 1, 0, &idx), kernel::kTimedOut);
    // UnbindStateChangeEvent has no input and no output; the handle the guest holds still waits without signaling.
    nxsvc::Reply unbind = ses.call(1);
    CHECK_EQ(unbind.result, 0u);
    CHECK_EQ(kernel::wait_synchronization(&h, 1, 0, &idx), kernel::kTimedOut);
    // Binding again gives a new valid handle.
    nxsvc::Reply again = ses.call(0);
    CHECK_EQ(again.result, 0u);
    REQUIRE(again.copy.size() == 1);
    CHECK_EQ(kernel::wait_synchronization(&again.copy[0], 1, 0, &idx), kernel::kTimedOut);
    kernel::handle_close(h);
    kernel::handle_close(again.copy[0]);
}

NXTEST(svc_power_reset_state) {
    {
        nxsvc::Env env;
        nxsvc::Session apm = nxsvc::connect("apm");
        nxsvc::Session sys = nxsvc::connect("apm:sys");
        nxsvc::Session psm = nxsvc::connect("psm");
        REQUIRE(apm.valid());
        REQUIRE(sys.valid());
        REQUIRE(psm.valid());
        CHECK_EQ(sys.call(0, nxsvc::Args().u32(1)).result, 0u);
        nxsvc::Session ses = apm.sub(apm.call(0));
        CHECK_EQ(ses.call(2, nxsvc::Args().u8(1)).result, 0u);
        CHECK_EQ(ses.call(0, nxsvc::Args().u32(0).u32(0x00010000)).result, 0u);
        CHECK_EQ(psm.call(3).result, 0u);
    }
    // A new Env resets every service (power_reset runs inside reset_all).
    nxsvc::Env env;
    services::amctl::set_docked(false);
    nxsvc::Session apm = nxsvc::connect("apm");
    nxsvc::Session sys = nxsvc::connect("apm:sys");
    nxsvc::Session psm = nxsvc::connect("psm");
    REQUIRE(apm.valid());
    REQUIRE(sys.valid());
    REQUIRE(psm.valid());
    CHECK_EQ(apm.call(1).get<uint32_t>(0), 0u);
    CHECK_EQ(apm.call(6).get<uint8_t>(0), 0u);
    CHECK_EQ(sys.call(7).get<uint32_t>(0), 0x00020003u);
    nxsvc::Session ses = apm.sub(apm.call(0));
    CHECK_EQ(ses.call(1, nxsvc::Args().u32(0)).get<uint32_t>(0), 0x00020003u);
    CHECK_EQ(psm.call(4).get<uint8_t>(0), 1u);
}
