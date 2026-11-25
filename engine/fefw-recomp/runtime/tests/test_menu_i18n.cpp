// SPDX-License-Identifier: GPL-3.0-or-later
// Which F1 menu table [gfx] ui_language = auto looks for on a Windows display language (include/nxrt/menu_i18n.h). The
// launcher's rules are the same (launcher/src-tauri/src/lang.rs).
#include <string>
#include <vector>

#include "nxrt/menu_i18n.h"
#include "test_framework.h"

NXTEST_GROUP(menu_i18n)

namespace {

bool offers(const char* tag, const char* name) {
    for (const std::string& c : nxrt::gfx::menu_language_candidates(tag))
        if (c == name) return true;
    return false;
}

}  // namespace

NXTEST(simplified_chinese_looks_for_zh) {
    for (const char* tag : {"zh-CN", "zh-SG", "zh_CN", "zh-Hans-HK"}) CHECK(offers(tag, "zh"));
}

NXTEST(traditional_chinese_never_gets_the_simplified_table) {
    for (const char* tag : {"zh-TW", "zh-HK", "zh-MO", "zh-Hant-CN"}) {
        CHECK(!offers(tag, "zh"));
        CHECK(offers(tag, "zh-TW"));
    }
}

NXTEST(the_full_name_comes_before_the_language) {
    const std::vector<std::string> pt = nxrt::gfx::menu_language_candidates("pt-BR");
    CHECK(pt.size() == 2 && pt[0] == "pt-BR" && pt[1] == "pt");
    CHECK(nxrt::gfx::menu_language_candidates("").empty());
}
