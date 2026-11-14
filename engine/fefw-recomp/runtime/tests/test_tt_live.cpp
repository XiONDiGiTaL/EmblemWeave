// SPDX-License-Identifier: GPL-3.0-or-later
// Live mode of the scripted-run runner (tt_runner.cpp, Runner::poll_live): when the script has no steps left, the run waits for lines
// appended to the [test] live file, runs them, and writes <live>.status each time it goes idle. Covers the status handshake and @end, and
// a line that does not parse: it is skipped with a note in the status, and the run goes on.
#include "test_framework.h"

#include <windows.h>

#include <cstdlib>
#include <fstream>
#include <sstream>
#include <string>
#include <thread>

#include "nxrt/config.h"
#include "nxrt/tt.h"

NXTEST_GROUP(tt_live)

namespace {

using namespace nxrt::tt;

int g_exit_seen = -999;
void on_exit_code(int code) { g_exit_seen = code; }

std::string slurp(const std::string& p) {
    std::ifstream f(p, std::ios::binary);
    std::stringstream ss;
    ss << f.rdbuf();
    return ss.str();
}
void spit(const std::string& p, const std::string& t, bool append = false) {
    std::ofstream f(p, std::ios::binary | (append ? std::ios::app : std::ios::trunc));
    f << t;
}

// The status file's text once it contains `what`; empty after about 10 s.
std::string wait_status(const std::string& status, const char* what) {
    for (int waited = 0; waited < 10000; waited += 10) {
        const std::string s = slurp(status);
        if (s.find(what) != std::string::npos) return s;
        Sleep(10);
    }
    return std::string();
}

// the number after "key": in a status line, -1 when absent
long long field(const std::string& s, const char* key) {
    const std::string k = std::string("\"") + key + "\":";
    const size_t p = s.find(k);
    return p == std::string::npos ? -1 : std::atoll(s.c_str() + p + k.size());
}

// One live run on a helper thread: frame_boundary blocks inside the runner's poll while the script is idle, so the test thread only
// watches and appends files. The destructor ends the run (appends @end if it is still waiting) and joins the thread.
struct LiveRun {
    std::string live, status, out;
    std::thread th;
    explicit LiveRun(const char* name, const std::string& script_text) {
        CreateDirectoryA("tt_test_out", nullptr);
        out = std::string("tt_test_out\\") + name;
        CreateDirectoryA(out.c_str(), nullptr);
        live = out + "\\live.nxs";
        status = live + ".status";
        DeleteFileA(status.c_str());
        spit(live, "");
        spit(out + "\\script.nxs", script_text);
        nxrt::config_clear();
        reset_for_tests();
        set_exit_handler(on_exit_code);
        g_exit_seen = -999;
        nxrt::config_set("test", "out", out.c_str());
        nxrt::config_set("test", "frame_source", "manual");
        nxrt::config_set("test", "script", (out + "\\script.nxs").c_str());
        nxrt::config_set("test", "live", live.c_str());
        init();
        th = std::thread([] {
            ::Ctx ctx{};
            for (int i = 0; i < 1000 && !finished(); i++) frame_boundary(&ctx);
        });
    }
    void join() {
        if (th.joinable()) th.join();
    }
    ~LiveRun() {
        if (th.joinable()) {
            spit(live, "@end\n", true);
            th.join();
        }
        shutdown();
    }
};

}  // namespace

NXTEST(lines_appended_while_idle_run_and_end_the_run) {
    LiveRun r("tt_live_run", "@timeout 0\n");
    const std::string s0 = wait_status(r.status, "\"lines\":0,");
    CHECK(!s0.empty());
    spit(r.live, "wait 2\n@log hello\n", true);
    const std::string s2 = wait_status(r.status, "\"lines\":2,");
    CHECK(!s2.empty());
    CHECK(field(s2, "frame") >= field(s0, "frame") + 2);     // the wait takes two frames, the log line runs after them
    spit(r.live, "@end\n", true);
    r.join();
    CHECK_EQ(g_exit_seen, 0);
    CHECK(slurp(r.out + "\\result.json").find("\"verdict\":\"pass\"") != std::string::npos);
}

NXTEST(a_line_that_does_not_parse_is_skipped_and_the_run_goes_on) {
    LiveRun r("tt_live_bad", "@timeout 0\n");
    CHECK(!wait_status(r.status, "\"lines\":0,").empty());
    spit(r.live, "bogus step\n", true);                       // its own append: a parse error skips the whole append
    const std::string sb = wait_status(r.status, "parse error");
    CHECK(!sb.empty());
    CHECK_EQ(field(sb, "lines"), 1);
    spit(r.live, "wait 3\n@end\n", true);                     // the lines after the bad one still run, and @end ends the run
    r.join();
    CHECK_EQ(g_exit_seen, 0);
}
