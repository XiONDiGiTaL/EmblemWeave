// SPDX-License-Identifier: GPL-3.0-or-later
// The hid adapter of the test tooling (tt_hid.cpp): only built when S3c's input_source.h is in the tree (the `services` branch).
#include "test_framework.h"

#if __has_include("nxrt/input_source.h")
#include <fstream>
#include <memory>

#include "nxrt/config.h"
#include "nxrt/input_source.h"
#include "nxrt/tt.h"

NXTEST_GROUP(tt_hid)

NXTEST(replayed_pad_reaches_the_hid_source) {
    using namespace nxrt;
    // a live source with player 1 connected and the stick pushed: replay must override player 1 and keep the rest
    auto live = std::make_shared<hid::ManualInputSource>();
    live->connect(0, hid::NpadStyle::FullKey);
    live->press(0, hid::btn(hid::kBtnZL));
    hid::set_input_source(live);

    tt::InputTrace tr;
    tt::PadFrame p;
    tr.record(0, p);
    p.buttons = hid::btn(hid::kBtnA) | hid::btn(hid::kBtnUp);
    p.ly = 20000;
    tr.record(3, p);
    tr.end_frame = 10;
    std::string err;
    REQUIRE(tr.write("tt_hid_test.nxin", &err));

    nxrt::config_clear();
    tt::reset_for_tests();
    nxrt::config_set("test", "replay", "tt_hid_test.nxin");
    nxrt::config_set("test", "frame_source", "manual");
    nxrt::config_set("test", "out", "tt_test_out/hid");
    REQUIRE(tt::init());
    for (int i = 0; i < 5; i++) {
        tt::frame_boundary(nullptr);
        hid::InputSnapshot s;
        auto src = hid::input_source();
        REQUIRE(src != nullptr);
        src->poll(s);
        if (i < 3) CHECK_EQ(s.npad[0].buttons, 0ull);
        else {
            CHECK_EQ(s.npad[0].buttons, hid::btn(hid::kBtnA) | hid::btn(hid::kBtnUp));
            CHECK_EQ(s.npad[0].left.y, 20000);
        }
    }
    tt::shutdown();
    std::remove("tt_hid_test.nxin");
    // the live source is what the recorder samples when nothing is replayed
    hid::set_input_source(live);
    tt::reset_for_tests();
    nxrt::config_clear();
    nxrt::config_set("test", "record", "tt_hid_rec.nxin");
    nxrt::config_set("test", "frame_source", "manual");
    nxrt::config_set("test", "out", "tt_test_out/hid2");
    REQUIRE(tt::init());
    for (int i = 0; i < 3; i++) tt::frame_boundary(nullptr);
    tt::shutdown();
    tt::InputTrace rec;
    REQUIRE(rec.read("tt_hid_rec.nxin", &err));
    CHECK_EQ(rec.at(1).buttons, hid::btn(hid::kBtnZL));
    std::remove("tt_hid_rec.nxin");
    hid::set_input_source(nullptr);
}

namespace {
bool accept_any_key(const char*, bool) { return true; }
}  // namespace

NXTEST(script_with_key_steps_merges_the_live_pad) {
    using namespace nxrt;
    // live source: A pressed and the left stick pushed (what the play layer produces from the keyboard)
    auto live = std::make_shared<hid::ManualInputSource>();
    live->connect(0, hid::NpadStyle::FullKey);
    live->press(0, hid::btn(hid::kBtnA));
    hid::InputSnapshot base = live->get();
    base.npad[0].left.x = 1000;
    base.npad[0].left.y = 2000;
    live->set(base);
    hid::set_input_source(live);

    // with a key step: the script's B is added to the live A, and the live stick stays because the script's stick is (0, 0)
    {
        { std::ofstream f("tt_hid_merge.nxs"); f << "hold B\nkey W for 10\n"; }
        nxrt::config_clear();
        tt::reset_for_tests();
        nxrt::config_set("test", "script", "tt_hid_merge.nxs");
        nxrt::config_set("test", "frame_source", "manual");
        nxrt::config_set("test", "out", "tt_test_out/hidmerge");
        REQUIRE(tt::init());
        tt::set_key_sink(accept_any_key);
        tt::frame_boundary(nullptr);
        hid::InputSnapshot s;
        auto src = hid::input_source();
        REQUIRE(src != nullptr);
        src->poll(s);
        CHECK_EQ(s.npad[0].buttons, hid::btn(hid::kBtnA) | hid::btn(hid::kBtnB));
        CHECK_EQ(s.npad[0].left.x, 1000);
        CHECK_EQ(s.npad[0].left.y, 2000);
        tt::shutdown();
        tt::set_key_sink(nullptr);
        std::remove("tt_hid_merge.nxs");
    }

    // without key steps the live pad is replaced as before: A is dropped, the stick is the script's (0, 0)
    {
        { std::ofstream f("tt_hid_merge.nxs"); f << "hold B\n3\n"; }
        nxrt::config_clear();
        tt::reset_for_tests();
        nxrt::config_set("test", "script", "tt_hid_merge.nxs");
        nxrt::config_set("test", "frame_source", "manual");
        nxrt::config_set("test", "out", "tt_test_out/hidmerge2");
        REQUIRE(tt::init());
        tt::frame_boundary(nullptr);
        hid::InputSnapshot s;
        auto src = hid::input_source();
        REQUIRE(src != nullptr);
        src->poll(s);
        CHECK_EQ(s.npad[0].buttons, hid::btn(hid::kBtnB));
        CHECK_EQ(s.npad[0].left.x, 0);
        CHECK_EQ(s.npad[0].left.y, 0);
        tt::shutdown();
        std::remove("tt_hid_merge.nxs");
    }
    hid::set_input_source(nullptr);
}
#else
NXTEST_GROUP(tt_hid)
NXTEST(skipped_without_input_source_header) { nxtest::skip("nxrt/input_source.h (S3c, services branch) is not in this tree"); }
#endif
