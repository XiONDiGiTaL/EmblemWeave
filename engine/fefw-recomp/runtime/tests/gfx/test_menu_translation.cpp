// SPDX-License-Identifier: GPL-3.0-or-later
// Which lines of an F1 menu table are refused because printf would read other arguments than the English key's
// (src/gfx/menu_i18n.cpp, menu_translation_problem).
#include "nxrt/menu_i18n.h"
#include "test_framework.h"

NXTEST_GROUP(menu_translation)

namespace {

bool accepted(const char* key, const char* value) { return nxrt::gfx::menu_translation_problem(key, value) == nullptr; }

}  // namespace

NXTEST(matching_specifiers_are_accepted) {
    CHECK(accepted("every %d s", "chaque %d s"));
    CHECK(accepted("avg %.2f ms  99%% %.1f ms", "moy. %.2f ms  99%% %.1f ms"));
    CHECK(accepted("% of one thread", "单线程占用"));
    CHECK(accepted("% of one thread", "% of a thread"));
    CHECK(accepted("100% (native)", "100%（原生）"));
}

NXTEST(different_specifiers_are_refused) {
    CHECK(!accepted("every %d s", "chaque %s s"));
    CHECK(!accepted("every %d s", "chaque seconde"));
}

NXTEST(a_space_is_a_printf_flag) {
    CHECK(!accepted("off", "99% de moins"));
    CHECK(!accepted("avg %.1f ms", "99% de %.1f ms"));
    CHECK(!accepted("% of one thread", "% d"));
}

NXTEST(percent_n_is_refused_in_every_spelling) {
    CHECK(!accepted("off", "%n"));
    CHECK(!accepted("off", "% n"));
    CHECK(!accepted("off", "%- n"));
}
