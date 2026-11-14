// SPDX-License-Identifier: GPL-3.0-or-later
// Launcher helpers in apps/launcher/game_info.cpp: game folder checks, the settings file, command-line quoting.
// Only temp folders are touched; no registry, no real launcher folder, no game process is started.
#include "test_framework.h"

#include <windows.h>
#include <shellapi.h>

#include <atomic>
#include <filesystem>
#include <fstream>
#include <string>

#include "game_info.h"

#pragma comment(lib, "shell32.lib")

NXTEST_GROUP(launcher_game_info)

namespace {

namespace fs = std::filesystem;
using mprl::GameStatus;
using mprl::LauncherConfig;

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
        path = base / (L"mprl_game_info_test_" + std::to_wstring(GetCurrentProcessId()) + L"_" +
                       std::to_wstring(counter++));
        fs::remove_all(path, ec);
        fs::create_directories(path, ec);
    }
    ~TempDir() {
        std::error_code ec;
        fs::remove_all(path, ec);
    }
    std::wstring str() const { return path.wstring(); }
};

// Reads back one argument: runs CommandLineToArgvW on "prog <quoted arg>" and returns argv[1].
std::wstring split_back(const std::wstring& arg) {
    const std::wstring cmd = L"prog " + mprl::quote_arg(arg);
    int argc = 0;
    LPWSTR* argv = CommandLineToArgvW(cmd.c_str(), &argc);
    if (!argv) return L"<CommandLineToArgvW failed>";
    const std::wstring out = argc >= 2 ? std::wstring(argv[1]) : std::wstring(L"<no argument>");
    LocalFree(argv);
    return out;
}

// The setup tool's state file header and a finished run's meta lines (pipeline.cpp, State::save and run_setup).
const char* kFinishedState =
    "mprs_setup_state\t1\n"
    "meta\tbc7_quality\t3\n"
    "meta\tfinished\t1\n";

}  // namespace

// ---- quote_arg -------------------------------------------------------------------------------------------------------

NXTEST(quote_arg_plain_words_are_unchanged) {
    CHECK(mprl::quote_arg(L"plain") == L"plain");
    CHECK(mprl::quote_arg(L"C:\\Games\\MPR") == L"C:\\Games\\MPR");
    CHECK(mprl::quote_arg(L"--game") == L"--game");
}

NXTEST(quote_arg_empty_is_two_quotes) {
    CHECK(mprl::quote_arg(L"") == L"\"\"");
}

NXTEST(quote_arg_space_is_wrapped) {
    CHECK(mprl::quote_arg(L"C:\\My Games\\MPR") == L"\"C:\\My Games\\MPR\"");
}

NXTEST(quote_arg_trailing_backslash_is_doubled_before_closing_quote) {
    // C:\My Games\ must not swallow the closing quote
    CHECK(mprl::quote_arg(L"C:\\My Games\\") == L"\"C:\\My Games\\\\\"");
}

NXTEST(quote_arg_embedded_quotes_are_escaped) {
    CHECK(mprl::quote_arg(L"say \"hi\"") == L"\"say \\\"hi\\\"\"");
    // a backslash before a quote: 2 backslashes + 1 escaped quote
    CHECK(mprl::quote_arg(L"a\\\"b") == L"\"a\\\\\\\"b\"");
}

NXTEST(quote_arg_round_trips_through_command_line_parser) {
    const std::wstring cases[] = {
        L"plain",
        L"",
        L"C:\\My Games\\",
        L"say \"hi\"",
        L"a\\\"b",
        L"tab\tinside",
        L"back\\slash\\",
        L"x\\ y\\",
        L"\"",
        L"C:\\Games\\Caf\u00e9 \u4e2d\u6587\\Metroid Prime\\",
    };
    for (const std::wstring& c : cases) {
        CHECK_MSG(split_back(c) == c, "quote_arg does not survive CommandLineToArgvW");
    }
}

// ---- launcher config text --------------------------------------------------------------------------------------------

NXTEST(launcher_config_round_trip_non_ascii) {
    LauncherConfig in;
    in.game_dir = L"D:\\Games\\Caf\u00e9 \u4e2d\u6587\\Metroid Prime";
    const std::string text = mprl::format_launcher_config(in);
    CHECK(text.find("\xC3\xA9") != std::string::npos);   // e acute, as UTF-8 bytes

    LauncherConfig out;
    out.game_dir = L"unchanged";
    CHECK(mprl::parse_launcher_config(text, &out));
    CHECK(out.game_dir == in.game_dir);
}

NXTEST(launcher_config_round_trip_comment_markers_in_path) {
    // a space before ';' or '#' would start a comment, so the writer quotes the value
    const std::wstring paths[] = {L"C:\\My ;Games #2", L"C:\\Games\\a;b#c", L"C:\\Games\\#1"};
    for (const std::wstring& p : paths) {
        LauncherConfig in;
        in.game_dir = p;
        LauncherConfig out;
        CHECK(mprl::parse_launcher_config(mprl::format_launcher_config(in), &out));
        CHECK_MSG(out.game_dir == p, "comment marker path did not round-trip");
    }
}

NXTEST(launcher_config_bom_and_comments) {
    LauncherConfig out;
    const std::string bom_text = std::string("\xEF\xBB\xBF") + "; top comment\r\n# also a comment\r\n[launcher]\r\n" +
                                 "game_dir = D:\\MP ; my copy\r\n";
    CHECK(mprl::parse_launcher_config(bom_text, &out));
    CHECK(out.game_dir == L"D:\\MP");

    // ';' or '#' not after a space is part of the path
    LauncherConfig hash;
    CHECK(mprl::parse_launcher_config("[launcher]\ngame_dir = D:\\Mp#1\n", &hash));
    CHECK(hash.game_dir == L"D:\\Mp#1");
    LauncherConfig semi;
    CHECK(mprl::parse_launcher_config("[launcher]\ngame_dir=D:\\a;b\n", &semi));
    CHECK(semi.game_dir == L"D:\\a;b");
}

NXTEST(launcher_config_unknown_keys_and_sections_ignored) {
    LauncherConfig out;
    const std::string text =
        "[launcher]\n"
        "language = en\n"
        "window_width = 1280\n"
        "game_dir = E:\\X\n"
        "[other]\n"
        "game_dir = G:\\Wrong\n";
    CHECK(mprl::parse_launcher_config(text, &out));
    CHECK(out.game_dir == L"E:\\X");
}

NXTEST(launcher_config_missing_key_keeps_default) {
    LauncherConfig out;
    out.game_dir = L"F:\\Default";
    CHECK(mprl::parse_launcher_config("[launcher]\nlanguage = en\n", &out));
    CHECK(out.game_dir == L"F:\\Default");

    // the key under another section does not count
    CHECK(mprl::parse_launcher_config("[other]\ngame_dir = G:\\X\n", &out));
    CHECK(out.game_dir == L"F:\\Default");

    // an empty value keeps the default too
    CHECK(mprl::parse_launcher_config("[launcher]\ngame_dir = ; nothing here\n", &out));
    CHECK(out.game_dir == L"F:\\Default");
}

NXTEST(launcher_config_invalid_utf8_is_rejected) {
    LauncherConfig out;
    out.game_dir = L"F:\\Default";
    CHECK(!mprl::parse_launcher_config("[launcher]\ngame_dir = \xC3\x28\n", &out));
    CHECK(out.game_dir == L"F:\\Default");
}

// ---- game folder checks ----------------------------------------------------------------------------------------------

NXTEST(summary_ready_text) {
    GameStatus s;
    s.exefs = s.romfs = s.textures = s.shaders = s.setup_finished = true;
    CHECK(s.ready());
    CHECK(s.summary() == L"Ready to play.");
}

NXTEST(game_dir_empty_folder_has_nothing) {
    TempDir t;
    const GameStatus s = mprl::check_game_dir(t.str());
    CHECK(!s.exefs);
    CHECK(!s.romfs);
    CHECK(!s.textures);
    CHECK(!s.shaders);
    CHECK(!s.setup_finished);
    CHECK(!s.ready());
    CHECK(s.summary() == L"No game files in this folder yet. Run setup to create them from your own game dump.");
}

NXTEST(game_dir_empty_string_has_nothing) {
    const GameStatus s = mprl::check_game_dir(L"");
    CHECK(!s.ready());
    CHECK(s.summary() == L"No game files in this folder yet. Run setup to create them from your own game dump.");
}

NXTEST(game_dir_empty_subfolders_do_not_count) {
    TempDir t;
    write_file(t.path / L"exefs" / L"main", "nso");
    fs::create_directories(t.path / L"romfs");
    fs::create_directories(t.path / L"bc7_cache");
    fs::create_directories(t.path / L"shader_cache");
    const GameStatus s = mprl::check_game_dir(t.str());
    CHECK(s.exefs);
    CHECK(!s.romfs);
    CHECK(!s.textures);
    CHECK(!s.shaders);
    CHECK(!s.ready());
    CHECK(s.summary() == L"Some game files are missing. Run setup again.");
}

NXTEST(game_dir_exefs_and_romfs_is_ready) {
    TempDir t;
    write_file(t.path / L"exefs" / L"main", "nso");
    write_file(t.path / L"romfs" / L"a.bin", "data");
    const GameStatus s = mprl::check_game_dir(t.str());
    CHECK(s.exefs);
    CHECK(s.romfs);
    CHECK(!s.textures);
    CHECK(!s.shaders);
    CHECK(!s.setup_finished);
    CHECK(s.ready());
    CHECK(s.summary() == L"Game files found, but setup did not finish. Run setup again to finish it.");
}

NXTEST(game_dir_textures_and_shaders_without_state_file_did_not_finish) {
    TempDir t;
    write_file(t.path / L"exefs" / L"main", "nso");
    write_file(t.path / L"romfs" / L"a.bin", "data");
    write_file(t.path / L"bc7_cache" / L"tex_0001.bc7", "bc7");
    write_file(t.path / L"shader_cache" / L"spirv" / L"s_0001.spv", "spv");
    const GameStatus s = mprl::check_game_dir(t.str());
    CHECK(s.ready());
    CHECK(s.textures);
    CHECK(s.shaders);
    CHECK(!s.setup_finished);
    CHECK(s.summary() == L"Game files found, but setup did not finish. Run setup again to finish it.");
}

NXTEST(game_dir_finished_setup_is_ready_to_play) {
    TempDir t;
    write_file(t.path / L"exefs" / L"main", "nso");
    write_file(t.path / L"romfs" / L"a.bin", "data");
    write_file(t.path / L"bc7_cache" / L"tex_0001.bc7", "bc7");
    write_file(t.path / L"shader_cache" / L"spirv" / L"s_0001.spv", "spv");
    write_file(t.path / L"setup_state.tsv", kFinishedState);
    const GameStatus s = mprl::check_game_dir(t.str());
    CHECK(s.setup_finished);
    CHECK(s.ready());
    CHECK(s.summary() == L"Ready to play.");
}

NXTEST(game_dir_state_file_must_have_the_setup_header_and_a_finished_line) {
    TempDir t;
    write_file(t.path / L"exefs" / L"main", "nso");
    write_file(t.path / L"romfs" / L"a.bin", "data");
    write_file(t.path / L"bc7_cache" / L"tex_0001.bc7", "bc7");
    write_file(t.path / L"shader_cache" / L"spirv" / L"s_0001.spv", "spv");

    // other state file version: the setup tool starts fresh, so nothing counts as finished
    write_file(t.path / L"setup_state.tsv", "mprs_setup_state\t2\nmeta\tfinished\t1\n");
    CHECK(!mprl::check_game_dir(t.str()).setup_finished);

    // the last finished line wins, as in the setup tool's loader
    write_file(t.path / L"setup_state.tsv", "mprs_setup_state\t1\nmeta\tfinished\t1\nmeta\tfinished\t0\n");
    CHECK(!mprl::check_game_dir(t.str()).setup_finished);

    // a run that did not finish has no finished line
    write_file(t.path / L"setup_state.tsv", "mprs_setup_state\t1\nmeta\tbc7_quality\t3\n");
    CHECK(!mprl::check_game_dir(t.str()).setup_finished);
}

NXTEST(game_dir_only_romfs_is_missing_files) {
    TempDir t;
    write_file(t.path / L"romfs" / L"a.bin", "data");
    const GameStatus s = mprl::check_game_dir(t.str());
    CHECK(!s.exefs);
    CHECK(!s.ready());
    CHECK(s.summary() == L"Some game files are missing. Run setup again.");
}

// ---- paths and the settings file -------------------------------------------------------------------------------------

NXTEST(controls_path_is_inside_game_folder) {
    CHECK(mprl::controls_path(L"D:\\Game") == L"D:\\Game\\controls.ini");
    CHECK(mprl::controls_path(L"D:\\Game\\") == L"D:\\Game\\controls.ini");
}

NXTEST(launcher_config_save_then_load) {
    TempDir t;
    const std::wstring path = (t.path / L"mpr_launcher.ini").wstring();

    LauncherConfig in;
    in.game_dir = L"E:\\Games\\Metroid Prime \u00e9";
    std::wstring err;
    CHECK(mprl::save_launcher_config(path, in, &err));
    CHECK(!fs::exists(path + L".tmp"));

    LauncherConfig out;
    CHECK(mprl::load_launcher_config(path, &out));
    CHECK(out.game_dir == in.game_dir);

    // a second save replaces the first
    in.game_dir = L"F:\\Two";
    CHECK(mprl::save_launcher_config(path, in, &err));
    CHECK(mprl::load_launcher_config(path, &out));
    CHECK(out.game_dir == L"F:\\Two");
}

NXTEST(launcher_config_missing_file_gives_default) {
    TempDir t;
    LauncherConfig out;
    out.game_dir = L"stale";
    CHECK(!mprl::load_launcher_config((t.path / L"does_not_exist.ini").wstring(), &out));
    CHECK(out.game_dir == mprl::default_game_dir());
}
