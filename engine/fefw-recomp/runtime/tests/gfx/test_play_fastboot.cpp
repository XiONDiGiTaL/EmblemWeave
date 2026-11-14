// SPDX-License-Identifier: GPL-3.0-or-later
// The [tweaks] keys of S46 (src/gfx/play/play_fastboot.cpp) in the settings file: defaults, reading, writing, bad values.
#include <string>
#include <vector>

#include "play/play_types.h"
#include "test_framework.h"
NXTEST_GROUP(play_fastboot)

namespace {
using namespace nxrt::play;
}

NXTEST(defaults_are_vanilla) {
    const Settings s = default_settings();
    CHECK(!s.fastboot.skip_intro);
    CHECK(!s.fastboot.skip_boot_logos);
    CHECK(!s.fastboot.fast_menus);
    CHECK_EQ(s.fastboot.menu_speed, 2);
    CHECK_EQ(fastboot_menu_rate(s.fastboot), 1.0);
}

NXTEST(read_and_write) {
    Settings s;
    std::string err;
    REQUIRE(parse_settings("[tweaks]\nskip_intro = 1\nSkip_Boot_Logos = yes\nfast_menus = on\nmenu_speed = 4\n", &s, &err));
    CHECK(s.fastboot.skip_intro);
    CHECK(s.fastboot.skip_boot_logos);
    CHECK(s.fastboot.fast_menus);
    CHECK_EQ(s.fastboot.menu_speed, 4);
    CHECK_EQ(fastboot_menu_rate(s.fastboot), 4.0);
    const std::string text = format_settings(s);
    CHECK(text.find("[tweaks]\nskip_intro = 1\nskip_boot_logos = 1\nfast_menus = 1\nmenu_speed = 4\n") != std::string::npos);
    Settings back;
    REQUIRE(parse_settings(text, &back, &err));
    CHECK(back.fastboot.skip_intro && back.fastboot.skip_boot_logos && back.fastboot.fast_menus);
    CHECK_EQ(back.fastboot.menu_speed, 4);
}

NXTEST(bad_values_and_other_keys) {
    Settings s;
    std::string err;
    CHECK(!parse_settings("[tweaks]\nmenu_speed = 5\n", &s, &err));
    CHECK(err.find("menu_speed") != std::string::npos);
    CHECK(!parse_settings("[tweaks]\nmenu_speed = 0\n", &s, &err));
    CHECK(!parse_settings("[tweaks]\nskip_intro = maybe\n", &s, &err));
    std::vector<std::string> warnings;
    REQUIRE(parse_settings("[tweaks]\nsome_other_tweak = 1\nfast_menus = 1\n", &s, &err, &warnings));   // a key of another tweak: a warning, not an error
    CHECK(s.fastboot.fast_menus);
    FastbootSettings f;
    f.menu_speed = 3;
    CHECK_EQ(fastboot_menu_rate(f), 1.0);   // the speed counts only with fast_menus on
}
