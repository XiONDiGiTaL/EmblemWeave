// SPDX-License-Identifier: GPL-3.0-or-later
// Shader preparation helpers in apps/launcher/game_info.cpp: the parser for the prepare run's output lines and the two folder
// checks. Only temp folders are touched; no game process is started.
#include "test_framework.h"

#include <windows.h>

#include <atomic>
#include <filesystem>
#include <fstream>
#include <string>

#include "game_info.h"

NXTEST_GROUP(launcher_prepare)

namespace {

namespace fs = std::filesystem;
using mprl::PrepareKind;

// Writes data to a file, creating the folders above it.
void write_file(const fs::path& p, const std::string& data) {
    std::error_code ec;
    fs::create_directories(p.parent_path(), ec);
    std::ofstream f(p, std::ios::binary | std::ios::trunc);
    f.write(data.data(), static_cast<std::streamsize>(data.size()));
}

// A fresh folder under the temp directory, removed again when the test ends.
struct TempDir {
    fs::path path;
    TempDir() {
        static std::atomic<unsigned> counter{0};
        std::error_code ec;
        const fs::path base = fs::temp_directory_path(ec);
        path = base / (L"mprl_prepare_test_" + std::to_wstring(GetCurrentProcessId()) + L"_" + std::to_wstring(counter++));
        fs::remove_all(path, ec);
        fs::create_directories(path, ec);
    }
    ~TempDir() {
        std::error_code ec;
        fs::remove_all(path, ec);
    }
    std::wstring str() const { return path.wstring(); }
};

}  // namespace

// ---- parse_prepare_line ----------------------------------------------------------------------------------------------

NXTEST(prepare_line_progress) {
    const mprl::PrepareLine p = mprl::parse_prepare_line("prepare: 12300 28805");
    CHECK(p.kind == PrepareKind::progress);
    CHECK_EQ(p.done, 12300ULL);
    CHECK_EQ(p.total, 28805ULL);

    const mprl::PrepareLine z = mprl::parse_prepare_line("prepare: 0 0");
    CHECK(z.kind == PrepareKind::progress);
    CHECK_EQ(z.total, 0ULL);
}

NXTEST(prepare_line_ready) {
    CHECK(mprl::parse_prepare_line("prepare: ready").kind == PrepareKind::ready);
    CHECK(mprl::parse_prepare_line("prepare: ready\r").kind == PrepareKind::ready);
    CHECK(mprl::parse_prepare_line("   prepare:   ready  ").kind == PrepareKind::ready);
}

NXTEST(prepare_line_done_counts_failures) {
    const mprl::PrepareLine d = mprl::parse_prepare_line(
        "prepare: done programs=28805 prepared=28000 missing=3 failed=0 parts=99 seconds=212");
    CHECK(d.kind == PrepareKind::done);
    CHECK_EQ(d.failed, 0LL);
    CHECK(!d.cancelled);

    const mprl::PrepareLine c = mprl::parse_prepare_line(
        "prepare: done programs=10 prepared=4 missing=0 failed=2 parts=5 seconds=9 cancelled\r");
    CHECK(c.kind == PrepareKind::done);
    CHECK_EQ(c.failed, 2LL);
    CHECK(c.cancelled);
}

NXTEST(prepare_line_done_without_failed_count) {
    const mprl::PrepareLine d = mprl::parse_prepare_line("prepare: done programs=5");
    CHECK(d.kind == PrepareKind::done);
    CHECK_EQ(d.failed, -1LL);
}

NXTEST(prepare_line_error_keeps_text) {
    const mprl::PrepareLine e = mprl::parse_prepare_line("prepare: error no shader cache is open");
    CHECK(e.kind == PrepareKind::error);
    CHECK_MSG(e.text == "no shader cache is open", e.text);
}

NXTEST(prepare_line_everything_else_is_other) {
    const mprl::PrepareLine a = mprl::parse_prepare_line("some log text");
    CHECK(a.kind == PrepareKind::other);
    CHECK_MSG(a.text == "some log text", a.text);
    CHECK(mprl::parse_prepare_line("prepare:").kind == PrepareKind::other);
    CHECK(mprl::parse_prepare_line("prepare: 1 2 3").kind == PrepareKind::other);
    CHECK(mprl::parse_prepare_line("prepare: 12 x").kind == PrepareKind::other);
    CHECK(mprl::parse_prepare_line("prepare: readyx").kind == PrepareKind::other);
    CHECK(mprl::parse_prepare_line("").kind == PrepareKind::other);
    // a count with too many digits is not read as progress
    CHECK(mprl::parse_prepare_line("prepare: 99999999999999999999 1").kind == PrepareKind::other);
}

// ---- shader_prepare_available / shader_prepare_done ------------------------------------------------------------------

NXTEST(prepare_available_needs_programs_bin) {
    CHECK(!mprl::shader_prepare_available(L""));
    TempDir t;
    CHECK(!mprl::shader_prepare_available(t.str()));
    write_file(t.path / L"shader_cache" / L"prepare.state", "complete: 1\n");
    CHECK(!mprl::shader_prepare_available(t.str()));
    write_file(t.path / L"shader_cache" / L"programs.bin", "MPRPRG01");
    CHECK(mprl::shader_prepare_available(t.str()));
}

NXTEST(prepare_done_needs_complete_line) {
    CHECK(!mprl::shader_prepare_done(L""));
    TempDir t;
    CHECK(!mprl::shader_prepare_done(t.str()));   // no prepare.state
    const fs::path state = t.path / L"shader_cache" / L"prepare.state";
    write_file(state, "key: abc\nprograms: 5\ncomplete: 0\n");
    CHECK(!mprl::shader_prepare_done(t.str()));
    write_file(state, "complete: 10\n");
    CHECK(!mprl::shader_prepare_done(t.str()));
    write_file(state, "xcomplete: 1\n");
    CHECK(!mprl::shader_prepare_done(t.str()));
    write_file(state, "key: abc\r\ncomplete: 1\r\n");
    CHECK(mprl::shader_prepare_done(t.str()));
}
