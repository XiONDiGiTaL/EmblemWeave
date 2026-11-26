// SPDX-License-Identifier: GPL-3.0-or-later
// The release check's parsing and version order (src/diag/update_check.cpp), which follow the launcher's (update.rs). No network.
#include "nxrt/update_check.h"
#include "test_framework.h"

NXTEST_GROUP(update_check)

using nxrt::update::is_newer;
using nxrt::update::release_version;

NXTEST(reads_the_tag) {
    CHECK_EQ(release_version(R"({"url":"x","tag_name":"v0.4.0","name":"0.4.0"})"), std::string("0.4.0"));
    CHECK_EQ(release_version("{\n  \"tag_name\" : \"1.2.3-rc1\"\n}"), std::string("1.2.3-rc1"));
    CHECK_EQ(release_version(R"({"body":"say \"tag_name\": \"9.9.9\"","tag_name":"V0.5.0"})"), std::string("0.5.0"));
}

NXTEST(refuses_other_answers) {
    CHECK_EQ(release_version(""), std::string());
    CHECK_EQ(release_version(R"({"message":"Not Found"})"), std::string());
    CHECK_EQ(release_version(R"({"tag_name":null})"), std::string());
    CHECK_EQ(release_version(R"({"tag_name":"0.4.0)"), std::string());
    CHECK_EQ(release_version(R"({"tag_name":""})"), std::string());
    CHECK_EQ(release_version(R"({"tag_name":"0.4.0 %s"})"), std::string());
}

NXTEST(versions_compare_by_number) {
    CHECK(is_newer("0.4.0", "0.3.0", false));
    CHECK(is_newer("0.10.0", "0.9.9", false));
    CHECK(is_newer("v1.0", "0.9.9", false));
    CHECK(!is_newer("0.3.0", "0.3.0", false));
    CHECK(!is_newer("0.2.9", "0.3.0", false));
    CHECK(!is_newer("0.3.0-rc1", "0.3.0", false));
    CHECK(!is_newer("nightly", "0.3.0", false));
    CHECK(!is_newer("0.4.0", "", false));
}

NXTEST(by_tag_any_other_release_is_newer) {
    CHECK(is_newer("abc1234", "def5678", true));
    CHECK(!is_newer("abc1234", "vabc1234", true));
    CHECK(!is_newer("", "abc1234", true));
}
