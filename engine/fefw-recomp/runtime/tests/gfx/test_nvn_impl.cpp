// SPDX-License-Identifier: GPL-3.0-or-later
// All NVN implementations registered together: every signature agrees with the generated table, the implemented count, and the
// coverage dump that docs/S4d.md is generated from (NXRT_NVN_DUMP_COVERAGE=<file>).
#include <cstdio>
#include <cstdlib>
#include <set>
#include <string>

#include "nvn_impl.h"
#include "test_framework.h"
NXTEST_GROUP(gfx_nvn_impl)

using namespace nxrt;

NXTEST(all_implementations_register_without_signature_mismatches) {
    nvn::install();
    nvn::abi::reset_mismatches();
    nvn::register_all_impls();
    CHECK_EQ(nvn::abi::mismatches(), 0u);
    size_t impl = 0;
    std::set<std::string> missing;
    for (size_t i = 0; i < nvn::entry_count(); ++i) {
        const char* n = nvn::entry_at(i).name;
        if (nvn::is_implemented(n)) ++impl;
        else missing.insert(n);
    }
    std::printf("    %zu of %zu NVN entry points implemented, %zu still logging stubs\n", impl, nvn::entry_count(), missing.size());
    CHECK(impl >= 400);
    if (const char* path = std::getenv("NXRT_NVN_DUMP_COVERAGE")) {
        FILE* f = std::fopen(path, "w");
        REQUIRE(f != nullptr);
        std::fprintf(f, "name\tret\targs\tsources\timplemented\n");
        for (size_t i = 0; i < nvn::entry_count(); ++i) {
            const nvn::Entry& e = nvn::entry_at(i);
            std::fprintf(f, "%s\t%c\t%s\t%u\t%d\n", e.name, e.ret, e.args, static_cast<unsigned>(e.sources), nvn::is_implemented(e.name) ? 1 : 0);
        }
        std::fclose(f);
    }
}
