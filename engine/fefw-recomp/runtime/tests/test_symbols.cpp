// SPDX-License-Identifier: GPL-3.0-or-later
// Tests for nxrt::symbols (linker-map symbolisation). Most cases build a fabricated map whose addresses are the runtime
// addresses of functions in this exe; the real-map case reads <exe>.map as written by /MAP.
#include "test_framework.h"

#include "nxrt/symbols.h"

#include <windows.h>

#include <algorithm>
#include <atomic>
#include <cctype>
#include <cstdint>
#include <cstdio>
#include <string>
#include <thread>
#include <vector>

NXTEST_GROUP(symbols)

// Marker resolved by the real-map case. extern "C" keeps its name undecorated in the map.
extern "C" __declspec(noinline) int nxrt_symtest_marker(volatile int* p) {
    *p += 1;
    return *p;
}

namespace {

// Distinct bodies, so ICF cannot fold them into one address.
__declspec(noinline) int symtest_f0(volatile int* p) { *p += 2; return *p; }
__declspec(noinline) int symtest_f1(volatile int* p) { *p += 3; return *p; }
__declspec(noinline) int symtest_f2(volatile int* p) { *p += 5; return *p; }
__declspec(noinline) int symtest_f3(volatile int* p) { *p += 7; return *p; }
__declspec(noinline) int symtest_f4(volatile int* p) { *p += 9; return *p; }

constexpr uint64_t kPreferred = 0x140000000ull;
constexpr const char* kFailMangled =
    "?fail@nxtest@@YAXPEBDH0AEBV?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@@Z";

template <class F>
uint64_t fn_addr(F* f) {
    return static_cast<uint64_t>(reinterpret_cast<uintptr_t>(f));
}

uint64_t exe_base() {
    return static_cast<uint64_t>(reinterpret_cast<uintptr_t>(GetModuleHandleA(nullptr)));
}

std::string exe_path() {
    char b[1024];
    DWORD n = GetModuleFileNameA(nullptr, b, sizeof b);
    return std::string(b, n);
}

std::string exe_name() {
    std::string p = exe_path();
    size_t s = p.find_last_of("\\/");
    return s == std::string::npos ? p : p.substr(s + 1);
}

std::string map_path() {
    std::string p = exe_path();
    size_t dot = p.find_last_of('.');
    size_t slash = p.find_last_of("\\/");
    if (dot != std::string::npos && (slash == std::string::npos || dot > slash)) p.erase(dot);
    return p + ".map";
}

std::string lower(std::string s) {
    for (auto& c : s) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    return s;
}

struct Sym {
    std::string name;
    uint64_t addr;  // runtime address of the symbol in this exe
};

// One symbol row in the layout lld-link writes; the Rva+Base column is the address rebased to the preferred base.
std::string entry_line(const Sym& s) {
    char rva[32];
    std::snprintf(rva, sizeof rva, "%016llX", static_cast<unsigned long long>(s.addr - exe_base() + kPreferred));
    return " 0001:00000000 " + s.name + " " + rva + "     test.obj\r\n";
}

// Map text with a section table (so the CODE filter applies), a Publics table and a Static symbols table.
std::string make_map(const std::vector<Sym>& publics, const std::vector<Sym>& statics = {}) {
    std::string t;
    t += " nxrt_tests\r\n\r\n";
    t += " Timestamp is 6ad920f5 (Wed Oct 21 15:30:45 2026)\r\n\r\n";
    t += " Preferred load address is 0000000140000000\r\n\r\n";
    t += " Start         Length     Name                   Class\r\n";
    t += " 0001:00000000 00001968H .text                   CODE\r\n\r\n";
    t += "  Address         Publics by Value              Rva+Base               Lib:Object\r\n\r\n";
    for (const auto& s : publics) t += entry_line(s);
    t += "\r\n entry point at         0001:00000100\r\n\r\n";
    t += " Static symbols\r\n\r\n";
    for (const auto& s : statics) t += entry_line(s);
    return t;
}

}  // namespace

NXTEST(fabricated_map_counts_and_exact_lookups) {
    const uint64_t f0 = fn_addr(&symtest_f0), f1 = fn_addr(&symtest_f1), f2 = fn_addr(&symtest_f2);
    const uint64_t f3 = fn_addr(&symtest_f3), f4 = fn_addr(&symtest_f4);
    // Deliberately unsorted; symtest_f4 sits in the Static symbols table.
    const std::string text = make_map({{"symtest_f2", f2}, {kFailMangled, f1}, {"symtest_f0", f0}, {"symtest_f3", f3}},
                                      {{"symtest_f4", f4}});
    CHECK_EQ(nxrt::symbols_load_map_text(text), 5u);
    CHECK_EQ(nxrt::symbols_count(), 5u);

    const std::string exe = exe_name();
    struct Want {
        uint64_t addr;
        const char* needle;
    };
    const Want wants[] = {{f0, "symtest_f0"}, {f1, "nxtest::fail"}, {f2, "symtest_f2"}, {f3, "symtest_f3"}, {f4, "symtest_f4"}};
    for (const auto& w : wants) {
        auto at = nxrt::symbols_lookup(w.addr);
        CHECK(at.symbol.find(w.needle) != std::string::npos);
        CHECK_EQ(at.sym_offset, 0u);
        CHECK(at.from_map);
        CHECK(at.module == exe);
        auto in = nxrt::symbols_lookup(w.addr + 1);
        CHECK(in.symbol.find(w.needle) != std::string::npos);
        CHECK_EQ(in.sym_offset, 1u);
    }
    // Gap: below every symbol there is no symbol start to report.
    const uint64_t lowest = std::min(std::min(std::min(f0, f1), std::min(f2, f3)), f4);
    auto gap = nxrt::symbols_lookup(lowest - 1);
    CHECK(gap.symbol.empty());
    CHECK(gap.module == exe);
}

NXTEST(undecorates_msvc_names) {
    nxrt::symbols_load_map_text(make_map({{kFailMangled, fn_addr(&symtest_f0)}}));
    auto s = nxrt::symbols_lookup(fn_addr(&symtest_f0));
    CHECK(s.symbol.find("nxtest::fail") != std::string::npos);
    CHECK(s.symbol.find("?fail") == std::string::npos);
}

NXTEST(real_map_resolves_marker) {
    if (GetFileAttributesA(map_path().c_str()) == INVALID_FILE_ATTRIBUTES) {
        nxtest::skip("no map");
        return;
    }
    const size_t n = nxrt::symbols_load_map();
    CHECK(n > 100);
    std::printf("    symbols loaded from real map: %zu\n", n);

    volatile int counter = 0;
    int (*volatile call)(volatile int*) = &nxrt_symtest_marker;
    CHECK_EQ(call(&counter), 1);

    const uint64_t a = fn_addr(&nxrt_symtest_marker);
    auto s = nxrt::symbols_lookup(a);
    CHECK(s.from_map);
    CHECK(s.symbol.find("nxrt_symtest_marker") != std::string::npos);
    CHECK_EQ(s.sym_offset, 0u);
    auto in = nxrt::symbols_lookup(a + 4);
    CHECK(in.symbol == s.symbol);
    CHECK_EQ(in.sym_offset, 4u);
}

NXTEST(kernel32_address_reports_module_without_symbol) {
    HMODULE k = GetModuleHandleA("kernel32.dll");
    REQUIRE(k != nullptr);
    FARPROC p = GetProcAddress(k, "Sleep");
    REQUIRE(p != nullptr);
    auto s = nxrt::symbols_lookup(fn_addr(p));
    const std::string m = lower(s.module);
    CHECK(m == "kernel32.dll" || m == "kernelbase.dll");
    CHECK(s.symbol.empty());
    CHECK(!s.from_map);
}

NXTEST(null_and_top_addresses_report_unknown_module) {
    auto lo = nxrt::symbols_lookup(0);
    CHECK(lo.module == "?");
    CHECK(lo.symbol.empty());
    auto hi = nxrt::symbols_lookup(~0ull);
    CHECK(hi.module == "?");
    CHECK(hi.symbol.empty());
}

NXTEST(missing_map_file_gives_zero_and_module_offset_only) {
    CHECK_EQ(nxrt::symbols_load_map("Z:\\nxrt-no-such-dir\\nope.map"), 0u);
    CHECK_EQ(nxrt::symbols_count(), 0u);
    const uint64_t a = fn_addr(&nxrt_symtest_marker);
    auto s = nxrt::symbols_lookup(a);
    CHECK(s.symbol.empty());
    CHECK(!s.from_map);
    CHECK(s.module == exe_name());
    CHECK_EQ(s.offset, a - exe_base());
}

NXTEST(concurrent_lookup_during_reload) {
    const uint64_t a = fn_addr(&nxrt_symtest_marker);
    const std::string text = make_map({{"nxrt_symtest_marker", a}});
    nxrt::symbols_load_map_text(text);

    const std::string exe = exe_name();
    std::atomic<int> bad{0};
    std::vector<std::thread> threads;
    for (int t = 0; t < 4; ++t) {
        threads.emplace_back([&] {
            for (int i = 0; i < 20000; ++i) {
                auto s = nxrt::symbols_lookup(a + static_cast<uint64_t>(i & 63));
                if (s.symbol != "nxrt_symtest_marker" || s.module != exe) bad++;
            }
        });
    }
    threads.emplace_back([&] {
        for (int i = 0; i < 50; ++i) nxrt::symbols_load_map_text(text);
    });
    for (auto& th : threads) th.join();

    CHECK_EQ(bad.load(), 0);
    CHECK_EQ(nxrt::symbols_count(), 1u);
}

NXTEST(describe_format) {
    const uint64_t a = fn_addr(&nxrt_symtest_marker);
    nxrt::symbols_load_map_text(make_map({{"nxrt_symtest_marker", a}}));
    char want[256];
    std::snprintf(want, sizeof want, "%s+0x%llx (nxrt_symtest_marker+0x4)", exe_name().c_str(),
                  static_cast<unsigned long long>(a + 4 - exe_base()));
    CHECK_EQ(nxrt::symbols_describe(a + 4), std::string(want));
    CHECK_EQ(nxrt::symbols_describe(0), std::string("0x0"));
}

NXTEST(describe_truncates_long_names) {
    const std::string long_name = "nxrt_symtest_" + std::string(200, 'a');
    const uint64_t a = fn_addr(&nxrt_symtest_marker);
    nxrt::symbols_load_map_text(make_map({{long_name, a}}));
    const std::string d = nxrt::symbols_describe(a);
    CHECK(d.find("(" + long_name.substr(0, 120) + "+0x0)") != std::string::npos);
    CHECK(d.find(long_name.substr(0, 121)) == std::string::npos);
}
