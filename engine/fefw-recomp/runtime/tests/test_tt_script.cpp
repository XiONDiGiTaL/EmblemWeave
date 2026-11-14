// SPDX-License-Identifier: GPL-3.0-or-later
#include "test_framework.h"

#include <cstdint>
#include <filesystem>
#include <fstream>
#include <string>

#include "nxrt/tt_script.h"

NXTEST_GROUP(tt_script)

using namespace nxrt::tt;

namespace {

constexpr const char* kGuid1 = "01234567-89ab-cdef-0123-456789abcdef";
constexpr const char* kHex1 = "67452301ab89efcd0123456789abcdef";
constexpr const char* kGuid2 = "fedcba98-7654-3210-fedc-ba9876543210";
constexpr const char* kHex2 = "98badcfe54761032fedcba9876543210";

// Region table fixture: two worlds, each with one area. "frigate" and "ruins" are world-only entries.
constexpr const char* kWorld1 = "10000000-0000-0000-0000-000000000001";
constexpr const char* kArea1 = "10000000-0000-0000-0000-0000000000a1";
constexpr const char* kWorld2 = "20000000-0000-0000-0000-000000000002";
constexpr const char* kArea2 = "20000000-0000-0000-0000-0000000000b2";

std::string region_text() {
    return std::string("# name<TAB>world<TAB>area\n") + "frigate\t" + kWorld1 + "\t\n" +
           "frigate_cargo_freight_lift\t" + kWorld1 + "\t" + kArea1 + "\n" + "\n" + "ruins\t" + kWorld2 + "\t\r\n" +
           "ruins_hall\t" + kWorld2 + "\t" + kArea2 + "\r\n";
}

uint64_t bit(int n) {
    return 1ull << n;
}

// Parses text as file "t.txt". Records a failure carrying the parser's message if it does not parse.
bool parse_ok(const std::string& text, Script& sc, const RegionTable* rt = nullptr) {
    std::string err;
    if (parse_script(text, "t.txt", rt, &sc, &err)) return true;
    ::nxtest::fail(__FILE__, __LINE__, "parse_script(text) succeeds", err);
    return false;
}

// Parses text, which must fail on `line` with a message containing `needle`.
void expect_err(const std::string& text, int line, const std::string& needle, const RegionTable* rt = nullptr) {
    Script sc;
    std::string err;
    bool ok = parse_script(text, "t.txt", rt, &sc, &err);
    CHECK_MSG(!ok, "parse unexpectedly succeeded: " + text);
    const std::string prefix = "t.txt:" + std::to_string(line) + ": ";
    CHECK_MSG(err.rfind(prefix, 0) == 0, err);
    CHECK_MSG(err.find(needle) != std::string::npos, err);
}

}  // namespace

// ---- step kinds -------------------------------------------------------------------------------------------

NXTEST(bare_and_wait_counts) {
    Script sc;
    REQUIRE(parse_ok("5\nwait 7\n0x10\nWAIT 0\n", sc));
    REQUIRE(sc.steps.size() == 4);
    CHECK_EQ(sc.steps[0].kind, StepKind::Wait);
    CHECK_EQ(sc.steps[0].count, (int64_t)5);
    CHECK_EQ(sc.steps[1].count, (int64_t)7);
    CHECK_EQ(sc.steps[2].count, (int64_t)16);
    CHECK_EQ(sc.steps[3].count, (int64_t)0);
    CHECK_EQ(sc.steps[0].timeout, (int64_t)-1);
}

NXTEST(colon_expands_to_wait_and_press) {
    Script sc;
    REQUIRE(parse_ok("5:A+B/6\n", sc));
    REQUIRE(sc.steps.size() == 2);
    CHECK_EQ(sc.steps[0].kind, StepKind::Wait);
    CHECK_EQ(sc.steps[0].count, (int64_t)5);
    CHECK_EQ(sc.steps[1].kind, StepKind::Press);
    CHECK_EQ(sc.steps[1].buttons, bit(0) | bit(1));
    CHECK_EQ(sc.steps[1].count, (int64_t)6);
    CHECK_EQ(sc.steps[0].line, 1);
    CHECK_EQ(sc.steps[1].line, 1);
    CHECK_EQ(sc.steps[1].src, std::string("5:A+B/6"));
}

NXTEST(colon_default_hold_is_four_and_start_is_plus) {
    Script sc;
    REQUIRE(parse_ok("3:start\n", sc));
    REQUIRE(sc.steps.size() == 2);
    CHECK_EQ(sc.steps[1].buttons, bit(10));
    CHECK_EQ(sc.steps[1].count, (int64_t)4);
}

NXTEST(press_with_and_without_for) {
    Script sc;
    REQUIRE(parse_ok("press a\npress A+X for 9\n", sc));
    REQUIRE(sc.steps.size() == 2);
    CHECK_EQ(sc.steps[0].kind, StepKind::Press);
    CHECK_EQ(sc.steps[0].buttons, bit(0));
    CHECK_EQ(sc.steps[0].count, (int64_t)-1);
    CHECK_EQ(sc.steps[1].buttons, bit(0) | bit(2));
    CHECK_EQ(sc.steps[1].count, (int64_t)9);
}

NXTEST(hold_and_release) {
    Script sc;
    REQUIRE(parse_ok("hold L+R\nrelease L\nrelease all\n", sc));
    REQUIRE(sc.steps.size() == 3);
    CHECK_EQ(sc.steps[0].kind, StepKind::Hold);
    CHECK_EQ(sc.steps[0].buttons, bit(6) | bit(7));
    CHECK_EQ(sc.steps[1].kind, StepKind::Release);
    CHECK_EQ(sc.steps[1].buttons, bit(6));
    CHECK_EQ(sc.steps[2].kind, StepKind::Release);
    CHECK_EQ(sc.steps[2].buttons, ~0ull);
}

NXTEST(stick_forms) {
    Script sc;
    REQUIRE(parse_ok("stick left 0.5 -1 for 30\nstick r -0.25 1\nstick L 0 0 for 2\n", sc));
    REQUIRE(sc.steps.size() == 3);
    CHECK_EQ(sc.steps[0].kind, StepKind::Stick);
    CHECK_EQ(sc.steps[0].which, 0);
    CHECK_EQ(sc.steps[0].x, 0.5);
    CHECK_EQ(sc.steps[0].y, -1.0);
    CHECK_EQ(sc.steps[0].count, (int64_t)30);
    CHECK_EQ(sc.steps[1].which, 1);
    CHECK_EQ(sc.steps[1].x, -0.25);
    CHECK_EQ(sc.steps[1].y, 1.0);
    CHECK_EQ(sc.steps[1].count, (int64_t)0);
    CHECK_EQ(sc.steps[2].which, 0);
    CHECK_EQ(sc.steps[2].count, (int64_t)2);
}

NXTEST(mouse_with_and_without_for) {
    Script sc;
    REQUIRE(parse_ok("mouse 3 -4\nmouse 1 2 for 5\n", sc));
    REQUIRE(sc.steps.size() == 2);
    CHECK_EQ(sc.steps[0].kind, StepKind::Mouse);
    CHECK_EQ(sc.steps[0].x, 3.0);
    CHECK_EQ(sc.steps[0].y, -4.0);
    CHECK_EQ(sc.steps[0].count, (int64_t)-1);
    CHECK_EQ(sc.steps[1].count, (int64_t)5);
}

NXTEST(key_verbs_with_and_without_for) {
    Script sc;
    REQUIRE(parse_ok("key W for 3\nkey Left Ctrl\nkeydown Left Ctrl\nkeyup Left Ctrl\nkey Page Up for 2\nkey Mouse1 for 0x10\nkey WheelUp\n", sc));
    REQUIRE(sc.steps.size() == 7);
    CHECK_EQ(sc.steps[0].kind, StepKind::Key);
    CHECK_EQ(sc.steps[0].text, std::string("W"));
    CHECK_EQ(sc.steps[0].count, (int64_t)3);
    CHECK_EQ(sc.steps[0].line, 1);
    CHECK_EQ(sc.steps[1].kind, StepKind::Key);
    CHECK_EQ(sc.steps[1].text, std::string("Left Ctrl"));
    CHECK_EQ(sc.steps[1].count, (int64_t)-1);
    CHECK_EQ(sc.steps[2].kind, StepKind::KeyDown);
    CHECK_EQ(sc.steps[2].text, std::string("Left Ctrl"));
    CHECK_EQ(sc.steps[3].kind, StepKind::KeyUp);
    CHECK_EQ(sc.steps[3].text, std::string("Left Ctrl"));
    CHECK_EQ(sc.steps[4].kind, StepKind::Key);
    CHECK_EQ(sc.steps[4].text, std::string("Page Up"));
    CHECK_EQ(sc.steps[4].count, (int64_t)2);
    CHECK_EQ(sc.steps[5].text, std::string("Mouse1"));
    CHECK_EQ(sc.steps[5].count, (int64_t)16);
    CHECK_EQ(sc.steps[6].text, std::string("WheelUp"));
    CHECK_EQ(sc.steps[6].count, (int64_t)-1);
    CHECK_EQ(sc.steps[6].src, std::string("key WheelUp"));
}

NXTEST(key_keywords_and_names_are_case_insensitive_and_trimmed) {
    Script sc;
    REQUIRE(parse_ok("KEY   Left   Ctrl  FOR 5  \nKeyDown  Form  \nkey Form for 2\nKEYUP Left Ctrl\n", sc));
    REQUIRE(sc.steps.size() == 4);
    CHECK_EQ(sc.steps[0].kind, StepKind::Key);
    CHECK_EQ(sc.steps[0].text, std::string("Left   Ctrl"));         // the name keeps its inner spacing as written
    CHECK_EQ(sc.steps[0].count, (int64_t)5);
    CHECK_EQ(sc.steps[1].kind, StepKind::KeyDown);
    CHECK_EQ(sc.steps[1].text, std::string("Form"));                // only a trailing " for <n>" is split off; "Form" is a name
    CHECK_EQ(sc.steps[2].text, std::string("Form"));
    CHECK_EQ(sc.steps[2].count, (int64_t)2);
    CHECK_EQ(sc.steps[3].kind, StepKind::KeyUp);
    CHECK_EQ(sc.steps[3].text, std::string("Left Ctrl"));
}

NXTEST(key_down_and_up_take_no_count) {
    // keydown / keyup are their own step kinds: no buttons, and the count stays "not given"
    Script sc;
    REQUIRE(parse_ok("keydown Shift\nkeyup Shift\n", sc));
    REQUIRE(sc.steps.size() == 2);
    CHECK_EQ(sc.steps[0].buttons, (uint64_t)0);
    CHECK_EQ(sc.steps[1].buttons, (uint64_t)0);
    CHECK_EQ(sc.steps[0].count, (int64_t)-1);
}

NXTEST(frame_wait) {
    Script sc;
    REQUIRE(parse_ok("@frame 1200\n", sc));
    REQUIRE(sc.steps.size() == 1);
    CHECK_EQ(sc.steps[0].kind, StepKind::WaitFrame);
    CHECK_EQ(sc.steps[0].count, (int64_t)1200);
}

NXTEST(area_and_world_guids_with_timeout) {
    Script sc;
    REQUIRE(parse_ok(std::string("@area ") + kGuid1 + " timeout=90\n@world " + kGuid2 + "\n", sc));
    REQUIRE(sc.steps.size() == 2);
    CHECK_EQ(sc.steps[0].kind, StepKind::WaitArea);
    CHECK_EQ(tt_guid_hex(sc.steps[0].id), std::string(kHex1));
    CHECK_EQ(sc.steps[0].timeout, (int64_t)90);
    CHECK_EQ(sc.steps[1].kind, StepKind::WaitWorld);
    CHECK_EQ(tt_guid_hex(sc.steps[1].id), std::string(kHex2));
    CHECK_EQ(sc.steps[1].timeout, (int64_t)-1);
}

NXTEST(pos_radius_and_timeout_defaults) {
    Script sc;
    REQUIRE(parse_ok("@pos 1 2 3\n@pos 1 2 3 0.5 timeout=10\n@pos 4 5 6 timeout=7\n", sc));
    REQUIRE(sc.steps.size() == 3);
    CHECK_EQ(sc.steps[0].kind, StepKind::WaitPos);
    CHECK_EQ(sc.steps[0].x, 1.0);
    CHECK_EQ(sc.steps[0].y, 2.0);
    CHECK_EQ(sc.steps[0].z, 3.0);
    CHECK_EQ(sc.steps[0].r, 1.0);
    CHECK_EQ(sc.steps[0].timeout, (int64_t)-1);
    CHECK_EQ(sc.steps[1].r, 0.5);
    CHECK_EQ(sc.steps[1].timeout, (int64_t)10);
    CHECK_EQ(sc.steps[2].x, 4.0);
    CHECK_EQ(sc.steps[2].r, 1.0);
    CHECK_EQ(sc.steps[2].timeout, (int64_t)7);
}

NXTEST(igt_seconds_and_timeout) {
    Script sc;
    REQUIRE(parse_ok("@igt 12.5 timeout=60\n@igt 3\n", sc));
    REQUIRE(sc.steps.size() == 2);
    CHECK_EQ(sc.steps[0].kind, StepKind::WaitIgt);
    CHECK_EQ(sc.steps[0].x, 12.5);
    CHECK_EQ(sc.steps[0].timeout, (int64_t)60);
    CHECK_EQ(sc.steps[1].x, 3.0);
    CHECK_EQ(sc.steps[1].timeout, (int64_t)-1);
}

NXTEST(shot_hash_log_timeout_pass) {
    Script sc;
    REQUIRE(parse_ok("@shot shot_01\n@hash\n@hash after warp\n@log  loading area # kept  \n@timeout 0\n@end\n", sc));
    REQUIRE(sc.steps.size() == 6);
    CHECK_EQ(sc.steps[0].kind, StepKind::Shot);
    CHECK_EQ(sc.steps[0].text, std::string("shot_01"));
    CHECK_EQ(sc.steps[1].kind, StepKind::Hash);
    CHECK_EQ(sc.steps[1].text, std::string(""));
    CHECK_EQ(sc.steps[2].text, std::string("after warp"));
    CHECK_EQ(sc.steps[3].kind, StepKind::Log);
    CHECK_EQ(sc.steps[3].text, std::string("loading area # kept"));
    CHECK_EQ(sc.steps[4].kind, StepKind::Timeout);
    CHECK_EQ(sc.steps[4].count, (int64_t)0);
    CHECK_EQ(sc.steps[5].kind, StepKind::Pass);
}

NXTEST(fail_with_and_without_message) {
    Script sc;
    REQUIRE(parse_ok("@fail boom # not a comment\n@fail\n@pass\n", sc));
    REQUIRE(sc.steps.size() == 3);
    CHECK_EQ(sc.steps[0].kind, StepKind::Fail);
    CHECK_EQ(sc.steps[0].text, std::string("boom # not a comment"));
    CHECK_EQ(sc.steps[1].kind, StepKind::Fail);
    CHECK_EQ(sc.steps[1].text, std::string(""));
    CHECK_EQ(sc.steps[2].kind, StepKind::Pass);
}

NXTEST(assert_numeric_fields_map_to_enums) {
    const char* names[] = {"frame", "igt", "health", "px", "py", "pz", "vx", "vy", "vz", "speed", "state"};
    const AssertField fields[] = {AssertField::Frame,  AssertField::Igt,  AssertField::Health, AssertField::PosX,
                                  AssertField::PosY,  AssertField::PosZ, AssertField::VelX,   AssertField::VelY,
                                  AssertField::VelZ, AssertField::Speed, AssertField::State};
    std::string text;
    for (const char* n : names) text += std::string("@assert ") + n + " == 2\n";
    Script sc;
    REQUIRE(parse_ok(text, sc));
    REQUIRE(sc.steps.size() == 11);
    for (std::size_t i = 0; i < 11; i++) {
        CHECK_EQ(sc.steps[i].kind, StepKind::Assert);
        CHECK_EQ(sc.steps[i].field, fields[i]);
        CHECK_EQ(sc.steps[i].x, 2.0);
    }
}

NXTEST(assert_operators) {
    Script sc;
    REQUIRE(parse_ok("@assert frame == 1\n@assert frame != 2\n@assert frame < 3\n@assert frame <= 4\n"
                     "@assert frame > 5\n@assert frame >= 6\n",
                     sc));
    REQUIRE(sc.steps.size() == 6);
    CHECK_EQ(sc.steps[0].op, AssertOp::Eq);
    CHECK_EQ(sc.steps[1].op, AssertOp::Ne);
    CHECK_EQ(sc.steps[2].op, AssertOp::Lt);
    CHECK_EQ(sc.steps[3].op, AssertOp::Le);
    CHECK_EQ(sc.steps[4].op, AssertOp::Gt);
    CHECK_EQ(sc.steps[5].op, AssertOp::Ge);
    CHECK_EQ(sc.steps[5].x, 6.0);
    CHECK_EQ(sc.steps[5].text, std::string("6"));
}

NXTEST(assert_area_and_world_take_ids) {
    Script sc;
    REQUIRE(parse_ok(std::string("@assert area == ") + kGuid1 + "\n@assert world != " + kGuid2 + "\n", sc));
    REQUIRE(sc.steps.size() == 2);
    CHECK_EQ(sc.steps[0].field, AssertField::Area);
    CHECK_EQ(sc.steps[0].op, AssertOp::Eq);
    CHECK_EQ(tt_guid_hex(sc.steps[0].id), std::string(kHex1));
    CHECK_EQ(sc.steps[1].field, AssertField::World);
    CHECK_EQ(sc.steps[1].op, AssertOp::Ne);
    CHECK_EQ(tt_guid_hex(sc.steps[1].id), std::string(kHex2));
}

NXTEST(case_insensitive_keywords) {
    Script sc;
    REQUIRE(parse_ok("PRESS a FOR 3\n@FRAME 5\nStick Left 0 0 FOR 2\n@PaSs\nWAIT 1\nHOLD b\nRELEASE ALL\n"
                     "MOUSE 1 1\n@Timeout 3\n@ASSERT FRAME == 2\n",
                     sc));
    REQUIRE(sc.steps.size() == 10);
    CHECK_EQ(sc.steps[0].kind, StepKind::Press);
    CHECK_EQ(sc.steps[0].count, (int64_t)3);
    CHECK_EQ(sc.steps[1].kind, StepKind::WaitFrame);
    CHECK_EQ(sc.steps[2].kind, StepKind::Stick);
    CHECK_EQ(sc.steps[2].count, (int64_t)2);
    CHECK_EQ(sc.steps[3].kind, StepKind::Pass);
    CHECK_EQ(sc.steps[4].kind, StepKind::Wait);
    CHECK_EQ(sc.steps[5].kind, StepKind::Hold);
    CHECK_EQ(sc.steps[6].kind, StepKind::Release);
    CHECK_EQ(sc.steps[6].buttons, ~0ull);
    CHECK_EQ(sc.steps[7].kind, StepKind::Mouse);
    CHECK_EQ(sc.steps[8].kind, StepKind::Timeout);
    CHECK_EQ(sc.steps[9].kind, StepKind::Assert);
    CHECK_EQ(sc.steps[9].field, AssertField::Frame);
    CHECK_EQ(sc.steps[9].op, AssertOp::Eq);
}

// ---- lines, comments, line endings ------------------------------------------------------------------------

NXTEST(comments_and_blank_lines) {
    Script sc;
    REQUIRE(parse_ok("# header\n\n   \n5 # trailing\n\t# indented comment\n@log keep # this\n", sc));
    REQUIRE(sc.steps.size() == 2);
    CHECK_EQ(sc.steps[0].kind, StepKind::Wait);
    CHECK_EQ(sc.steps[0].line, 4);
    CHECK_EQ(sc.steps[0].src, std::string("5"));
    CHECK_EQ(sc.steps[1].kind, StepKind::Log);
    CHECK_EQ(sc.steps[1].line, 6);
    CHECK_EQ(sc.steps[1].text, std::string("keep # this"));
    CHECK_EQ(sc.steps[1].src, std::string("@log keep # this"));
}

NXTEST(crlf_input) {
    Script sc;
    REQUIRE(parse_ok("wait 3\r\npress A\r\n@end\r\n", sc));
    REQUIRE(sc.steps.size() == 3);
    CHECK_EQ(sc.steps[0].line, 1);
    CHECK_EQ(sc.steps[1].line, 2);
    CHECK_EQ(sc.steps[2].line, 3);
    CHECK_EQ(sc.steps[0].src, std::string("wait 3"));
    CHECK_EQ(sc.steps[1].src, std::string("press A"));
    CHECK_EQ(sc.steps[2].kind, StepKind::Pass);
}

NXTEST(utf8_bom_is_skipped) {
    Script sc;
    REQUIRE(parse_ok(std::string("\xEF\xBB\xBF") + "wait 1\n", sc));
    REQUIRE(sc.steps.size() == 1);
    CHECK_EQ(sc.steps[0].line, 1);
    CHECK_EQ(sc.steps[0].count, (int64_t)1);
}

// ---- errors (line numbers) --------------------------------------------------------------------------------

NXTEST(error_unknown_keyword_line) {
    expect_err("wait 1\n\nfoo 2\n", 3, "unknown keyword 'foo'");
}

NXTEST(error_bad_button_line) {
    expect_err("press A\npress A+Q\n", 2, "unknown button 'Q'");
    expect_err("wait 1\n5:Z\n", 2, "unknown button 'Z'");
}

NXTEST(error_stick_out_of_range_line) {
    expect_err("stick left 1.5 0\n", 1, "out of range");
    expect_err("wait 1\nstick right 0 -1.01\n", 2, "out of range");
}

NXTEST(error_wrong_argument_count_line) {
    expect_err("press\n", 1, "expected");
    expect_err("wait 1\n@pos 1 2\n", 2, "expected");
    expect_err("stick left 0\n", 1, "expected");
    expect_err("wait 1\n\n@warp frigate\n", 3, "expected");
}

NXTEST(error_key_verbs_need_a_name) {
    expect_err("key\n", 1, "expected");
    expect_err("wait 1\nkeydown\n", 2, "expected");
    expect_err("keyup   \n", 1, "expected");
    expect_err("key for 3\n", 1, "expected");                 // no name before the trailing "for 3"
    expect_err("key W for x\n", 1, "bad number 'x'");
}

NXTEST(error_bad_number_line) {
    expect_err("wait 1x\n", 1, "bad number");
    expect_err("wait 1\n@frame -5\n", 2, "bad number");
    expect_err("@igt nan\n", 1, "bad number");
    expect_err("@area " + std::string(kGuid1) + " timeout=x\n", 1, "bad number");
}

NXTEST(error_bad_guid_line) {
    expect_err("wait 1\n@area 01234567-89ab-cdef-0123-45678\n", 2, "not a GUID");
    expect_err("@warp 0123 0123\n", 1, "not a GUID");
}

NXTEST(error_unknown_region_name_line) {
    RegionTable rt;
    std::string e;
    CHECK_MSG(rt.load_text(region_text(), &e), e);
    expect_err("wait 1\n\n@world nosuch\n", 3, "nosuch", &rt);
    expect_err("@warp frigate nosuch\n", 1, "nosuch", &rt);
}

NXTEST(error_area_name_without_area_line) {
    RegionTable rt;
    std::string e;
    CHECK_MSG(rt.load_text(region_text(), &e), e);
    expect_err("wait 1\n@area frigate\n", 2, "has no area", &rt);
    expect_err("@warp frigate ruins\n", 1, "has no area", &rt);
}

NXTEST(error_assert_id_ordering_line) {
    expect_err("wait 1\n@assert area < " + std::string(kGuid1) + "\n", 2, "only == and !=");
}

NXTEST(error_log_without_text_line) {
    expect_err("@log\n", 1, "needs text");
}

NXTEST(error_timeout_bad_value_line) {
    expect_err("wait 1\n\n@area " + std::string(kGuid1) + " timeout=\n", 3, "bad number");
    expect_err("@area " + std::string(kGuid1) + " timeout=1 timeout=2\n", 1, "timeout given twice");
}

NXTEST(error_unknown_assert_field_line) {
    expect_err("@assert hp == 3\n", 1, "unknown assert field 'hp'");
    expect_err("@assert frame ~ 3\n", 1, "unknown operator");
}

// ---- region table and @warp -------------------------------------------------------------------------------

NXTEST(region_table_round_trip) {
    RegionTable rt;
    std::string e;
    CHECK_MSG(rt.load_text(region_text(), &e), e);
    REQUIRE(rt.entries.size() == 4);

    const RegionTable::Entry* world1 = rt.find("FRIGATE");
    REQUIRE(world1 != nullptr);
    CHECK_EQ(world1->has_area, false);
    CHECK_EQ(tt_guid_to_text(world1->world), std::string(kWorld1));

    const RegionTable::Entry* area1 = rt.find("frigate_cargo_freight_lift");
    REQUIRE(area1 != nullptr);
    CHECK_EQ(area1->has_area, true);
    CHECK_EQ(tt_guid_to_text(area1->world), std::string(kWorld1));
    CHECK_EQ(tt_guid_to_text(area1->area), std::string(kArea1));

    const RegionTable::Entry* area2 = rt.find("Ruins_Hall");
    REQUIRE(area2 != nullptr);
    CHECK_EQ(tt_guid_to_text(area2->area), std::string(kArea2));
    CHECK_EQ(tt_guid_to_text(area2->world), std::string(kWorld2));

    CHECK(rt.find("missing") == nullptr);
}

NXTEST(warp_resolves_region_names) {
    RegionTable rt;
    std::string e;
    CHECK_MSG(rt.load_text(region_text(), &e), e);

    Script sc;
    REQUIRE(parse_ok(std::string("@warp frigate frigate_cargo_freight_lift\n@warp RUINS ruins_hall\n") + "@warp " +
                         kWorld1 + " " + kArea2 + "\n",
                     sc, &rt));
    REQUIRE(sc.steps.size() == 3);
    CHECK_EQ(sc.steps[0].kind, StepKind::Warp);
    CHECK_EQ(sc.steps[0].name1, std::string("frigate"));
    CHECK_EQ(sc.steps[0].name2, std::string("frigate_cargo_freight_lift"));
    CHECK_EQ(tt_guid_to_text(sc.steps[0].id), std::string(kArea1));
    CHECK_EQ(tt_guid_to_text(sc.steps[0].id2), std::string(kWorld1));
    CHECK_EQ(tt_guid_to_text(sc.steps[1].id), std::string(kArea2));
    CHECK_EQ(tt_guid_to_text(sc.steps[1].id2), std::string(kWorld2));
    CHECK_EQ(tt_guid_to_text(sc.steps[2].id), std::string(kArea2));
    CHECK_EQ(tt_guid_to_text(sc.steps[2].id2), std::string(kWorld1));
}

NXTEST(region_table_bad_guid_reports_line) {
    RegionTable rt;
    std::string e;
    CHECK(!rt.load_text("# c\n\nname\tnot-a-guid\t\n", &e));
    CHECK_EQ(e.rfind("line 3: ", 0), (std::size_t)0);
}

NXTEST(region_table_load_from_file) {
    namespace fs = std::filesystem;
    const fs::path file = fs::temp_directory_path() / "nxrt_tt_script_regions_test.tsv";
    {
        std::ofstream f(file, std::ios::binary);
        f << "\xEF\xBB\xBF" << region_text();
    }
    RegionTable rt;
    std::string e;
    bool ok = rt.load(file.string(), &e);
    std::error_code ec;
    fs::remove(file, ec);
    CHECK_MSG(ok, e);
    CHECK_EQ(rt.entries.size(), (std::size_t)4);

    RegionTable missing;
    std::string e2;
    CHECK(!missing.load((fs::temp_directory_path() / "nxrt_tt_script_no_such_file.tsv").string(), &e2));
    CHECK_EQ(e2.rfind("cannot open ", 0), (std::size_t)0);
}

NXTEST(parse_script_file_reads_bom_and_crlf) {
    namespace fs = std::filesystem;
    const fs::path file = fs::temp_directory_path() / "nxrt_tt_script_test.txt";
    {
        std::ofstream f(file, std::ios::binary);
        f << "\xEF\xBB\xBF" << "wait 2\r\n@pass\r\n";
    }
    Script sc;
    std::string e;
    bool ok = parse_script_file(file.string(), nullptr, &sc, &e);
    std::error_code ec;
    fs::remove(file, ec);
    CHECK_MSG(ok, e);
    REQUIRE(sc.steps.size() == 2);
    CHECK_EQ(sc.steps[1].kind, StepKind::Pass);
    CHECK_EQ(sc.steps[1].line, 2);
    CHECK_EQ(sc.path, file.string());
}

NXTEST(parse_script_file_missing_file) {
    Script sc;
    std::string e;
    const std::string path = "no_such_dir_for_tt_script/missing.tt";
    CHECK(!parse_script_file(path, nullptr, &sc, &e));
    CHECK_EQ(e, std::string("cannot open ") + path);
}

// ---- GUID helpers -----------------------------------------------------------------------------------------

NXTEST(guid_round_trip_and_hex) {
    uint8_t b[16];
    {   // the real game's Frigate world id as read from its memory under Eden
        uint8_t f[16];
        REQUIRE(tt_guid_to_bytes("4b5e8635-0f43-4ffb-9427-2e62186e517d", f));
        CHECK_EQ(tt_guid_hex(f), std::string("35865e4b430ffb4f94272e62186e517d"));
        CHECK_EQ(tt_guid_to_text(f), std::string("4b5e8635-0f43-4ffb-9427-2e62186e517d"));
    }
    REQUIRE(tt_guid_to_bytes(kGuid1, b));
    CHECK_EQ(b[0], (uint8_t)0x67);
    CHECK_EQ(b[15], (uint8_t)0xef);
    CHECK_EQ(tt_guid_hex(b), std::string(kHex1));
    CHECK_EQ(tt_guid_to_text(b), std::string(kGuid1));

    // Upper case in, lower case out.
    uint8_t c[16];
    REQUIRE(tt_guid_to_bytes("FEDCBA98-7654-3210-FEDC-BA9876543210", c));
    CHECK_EQ(tt_guid_to_text(c), std::string(kGuid2));
    CHECK_EQ(tt_guid_hex(c), std::string(kHex2));
}

NXTEST(guid_malformed_text_rejected) {
    uint8_t b[16];
    CHECK(!tt_guid_to_bytes("0123456789abcdef0123456789abcdef", b));   // no dashes
    CHECK(!tt_guid_to_bytes("01234567_89ab-cdef-0123-456789abcdef", b));  // wrong separator
    CHECK(!tt_guid_to_bytes("01234567-89ab-cdef-0123-456789abcdeg", b));  // not hex
    CHECK(!tt_guid_to_bytes("01234567-89ab-cdef-0123-456789abcde", b));   // too short
    CHECK(!tt_guid_to_bytes("", b));
}

NXTEST(gameplay_steps_sleep_run_mash_give_rate) {
    Script sc;
    REQUIRE(parse_ok("@sleep 1500\n@run 2.5 timeout=100\n@give MorphBall 1\n@give charge_beam 1\n@give 4 50\n@rate 144\n@rate 0\n@mash A 5000 area 01234567-89ab-cdef-0123-456789abcdef timeout=1000\n", sc));
    REQUIRE(sc.steps.size() == 8);
    CHECK_EQ(sc.steps[0].kind, StepKind::WaitMs);
    CHECK_EQ(sc.steps[0].x, 1500.0);
    CHECK_EQ(sc.steps[1].kind, StepKind::WaitRun);
    CHECK_EQ(sc.steps[1].x, 2.5);
    CHECK_EQ(sc.steps[1].timeout, (int64_t)100);
    CHECK_EQ(sc.steps[2].kind, StepKind::Give);
    CHECK_EQ(sc.steps[2].x, 16.0);          // MorphBall
    CHECK_EQ(sc.steps[3].x, 10.0);          // ChargeBeam
    CHECK_EQ(sc.steps[4].x, 4.0);
    CHECK_EQ(sc.steps[4].y, 50.0);
    CHECK_EQ(sc.steps[5].kind, StepKind::Rate);
    CHECK_EQ(sc.steps[5].x, 144.0);
    CHECK_EQ(sc.steps[6].x, 0.0);
    CHECK_EQ(sc.steps[7].kind, StepKind::Mash);
    CHECK_EQ(sc.steps[7].x, 5000.0);
    CHECK_EQ(sc.steps[7].timeout, (int64_t)1000);
    expect_err("@rate -1\n", 1, "@rate hz");
    expect_err("@give Nonsense 1\n", 1, "unknown item");
    expect_err("@mash A 5000 room x\n", 1, "@mash");
}

// ---- S15 steps: @mash, @run, @grant, @call ----------------------------------------------------------------

NXTEST(mashuntil_parses_keys_interval_condition_and_timeout) {
    Script sc;
    REQUIRE(parse_ok("@mashuntil A+B every 90 until health > 0 timeout=500\n@mashuntil START every 5 until state >= 0\n", sc));
    REQUIRE(sc.steps.size() == 2);
    CHECK_EQ(sc.steps[0].kind, StepKind::PressUntil);
    CHECK_EQ(sc.steps[0].buttons, bit(0) | bit(1));
    CHECK_EQ(sc.steps[0].count, (int64_t)90);
    CHECK_EQ(sc.steps[0].field, AssertField::Health);
    CHECK_EQ(sc.steps[0].op, AssertOp::Gt);
    CHECK_EQ(sc.steps[0].x, 0.0);
    CHECK_EQ(sc.steps[0].timeout, (int64_t)500);
    CHECK_EQ(sc.steps[1].field, AssertField::State);
    CHECK_EQ(sc.steps[1].op, AssertOp::Ge);
    CHECK_EQ(sc.steps[1].timeout, (int64_t)-1);
}

NXTEST(mashuntil_errors) {
    expect_err("@mashuntil A every 90 until health\n", 1, "@mashuntil <keys> every");
    expect_err("@mashuntil A every 3 until health > 0\n", 1, "at least 5");
    expect_err("@mashuntil A every 90 until bogus > 0\n", 1, "unknown assert field");
    expect_err("@mashuntil A each 90 until health > 0\n", 1, "@mashuntil <keys> every");
}

NXTEST(run_parses_seconds_and_timeout) {
    Script sc;
    REQUIRE(parse_ok("@run 2.5\n@run 0 timeout=40\n", sc));
    REQUIRE(sc.steps.size() == 2);
    CHECK_EQ(sc.steps[0].kind, StepKind::WaitRun);
    CHECK_EQ(sc.steps[0].x, 2.5);
    CHECK_EQ(sc.steps[1].timeout, (int64_t)40);
    expect_err("@run -1\n", 1, "must be >= 0");
}

NXTEST(grant_names_numbers_and_all) {
    Script sc;
    REQUIRE(parse_ok("@grant MorphBall\n@grant missiles 250 250\n@grant Charge_Beam\n@grant 0x12 1\n@grant all\n", sc));
    REQUIRE(sc.steps.size() == 5);
    CHECK_EQ(sc.steps[0].kind, StepKind::Grant);
    CHECK_EQ(sc.steps[0].count, (int64_t)0x10);
    CHECK_EQ(sc.steps[0].x, 1.0);
    CHECK_EQ(sc.steps[1].count, (int64_t)4);
    CHECK_EQ(sc.steps[1].x, 250.0);
    CHECK_EQ(sc.steps[1].y, 250.0);
    CHECK_EQ(sc.steps[2].count, (int64_t)0xa);
    CHECK_EQ(sc.steps[3].count, (int64_t)0x12);
    CHECK_EQ(sc.steps[4].count, (int64_t)-1);
    expect_err("@grant sword\n", 1, "unknown item");
}

NXTEST(call_parses_offset_and_argument_text) {
    Script sc;
    REQUIRE(parse_ok("@call 0xb03a58 $mgr\n@call 0xc88f30 $ps 4 250\n", sc));
    REQUIRE(sc.steps.size() == 2);
    CHECK_EQ(sc.steps[0].kind, StepKind::Call);
    CHECK_EQ(sc.steps[0].count, (int64_t)0xb03a58);
    CHECK_EQ(sc.steps[0].text, std::string("$mgr"));
    CHECK_EQ(sc.steps[1].text, std::string("$ps 4 250"));
    expect_err("@call\n", 1, "@call <offset>");
}

// ---- S25 step: @watch -------------------------------------------------------------------------------

NXTEST(watch_on_defaults_to_both_axes) {
    Script sc;
    REQUIRE(parse_ok("@watch on\n", sc));
    REQUIRE(sc.steps.size() == 1);
    CHECK_EQ(sc.steps[0].kind, StepKind::Watch);
    CHECK_EQ(sc.steps[0].text, std::string("on"));
    CHECK_EQ(sc.steps[0].y, 3.0);
}

NXTEST(watch_on_selects_vely_or_posy) {
    Script sc;
    REQUIRE(parse_ok("@watch on vely\n@watch on posy\n@watch on both\n", sc));
    REQUIRE(sc.steps.size() == 3);
    CHECK_EQ(sc.steps[0].kind, StepKind::Watch);
    CHECK_EQ(sc.steps[0].y, 1.0);
    CHECK_EQ(sc.steps[1].y, 2.0);
    CHECK_EQ(sc.steps[2].y, 3.0);
}

NXTEST(watch_off_parses) {
    Script sc;
    REQUIRE(parse_ok("@watch off\n", sc));
    REQUIRE(sc.steps.size() == 1);
    CHECK_EQ(sc.steps[0].kind, StepKind::Watch);
    CHECK_EQ(sc.steps[0].text, std::string("off"));
}

NXTEST(watch_errors) {
    expect_err("@watch\n", 1, "@watch on [vely|posy|both]");
    expect_err("@watch maybe\n", 1, "expected on or off");
    expect_err("@watch on bogus\n", 1, "expected vely, posy or both");
}

// ---- S26 step: @rays -------------------------------------------------------------------------------

NXTEST(rays_parses_in_and_out_files) {
    Script sc;
    REQUIRE(parse_ok("@rays a.txt b.txt\n", sc));
    REQUIRE(sc.steps.size() == 1);
    CHECK_EQ(sc.steps[0].kind, StepKind::Rays);
    CHECK_EQ(sc.steps[0].text, std::string("a.txt b.txt"));
}

NXTEST(rays_wrong_argument_count_is_usage_error) {
    expect_err("@rays\n", 1, "@rays <in-file> <out-file>");
    expect_err("@rays a.txt\n", 1, "@rays <in-file> <out-file>");
    expect_err("@rays a b c\n", 1, "@rays <in-file> <out-file>");
}

NXTEST(objs_parses_class_list) {
    Script sc;
    REQUIRE(parse_ok("@objs *\n", sc));
    REQUIRE(sc.steps.size() == 1);
    CHECK_EQ(sc.steps[0].kind, StepKind::Objs);
    CHECK_EQ(sc.steps[0].text, std::string("*"));
}

NXTEST(peek_parses_in_and_out_files) {
    Script sc;
    REQUIRE(parse_ok("@peek a.txt b.txt\n", sc));
    REQUIRE(sc.steps.size() == 1);
    CHECK_EQ(sc.steps[0].kind, StepKind::Peek);
    CHECK_EQ(sc.steps[0].text, std::string("a.txt b.txt"));
}

NXTEST(peek_wrong_argument_count_is_usage_error) {
    expect_err("@peek\n", 1, "@peek <in-file> <out-file>");
    expect_err("@peek a.txt\n", 1, "@peek <in-file> <out-file>");
    expect_err("@peek a b c\n", 1, "@peek <in-file> <out-file>");
}

NXTEST(probe_parses_in_and_out_files) {
    Script sc;
    REQUIRE(parse_ok("@probe a.txt b.txt\n", sc));
    REQUIRE(sc.steps.size() == 1);
    CHECK_EQ(sc.steps[0].kind, StepKind::Probe);
    CHECK_EQ(sc.steps[0].text, std::string("a.txt b.txt"));
}

NXTEST(poke_parses_one_file_and_probe_needs_two) {
    Script sc;
    REQUIRE(parse_ok("@poke a.txt\n", sc));
    REQUIRE(sc.steps.size() == 1);
    CHECK_EQ(sc.steps[0].kind, StepKind::Poke);
    expect_err("@poke\n", 1, "@poke <in-file>");
    expect_err("@probe a.txt\n", 1, "@probe <in-file> <out-file>");
}
