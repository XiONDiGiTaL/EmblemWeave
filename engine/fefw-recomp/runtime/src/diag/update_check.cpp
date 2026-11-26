// SPDX-License-Identifier: GPL-3.0-or-later
// include/nxrt/update_check.h. WinHTTP, which Windows and Wine / Proton both have; a native Linux build has no request yet (it logs that).
#include "nxrt/update_check.h"

#ifdef _WIN32
#include <windows.h>
#include <winhttp.h>
#endif

#include <atomic>
#include <cstdint>
#include <mutex>
#include <thread>

#include "nxrt/config.h"
#include "nxrt/log.h"

namespace nxrt::update {
namespace {

constexpr const char* kTag = "update";

#ifdef NXRT_RELEASE_VERSION
constexpr const char* kCurrent = NXRT_RELEASE_VERSION;
#else
constexpr const char* kCurrent = "";
#endif
#ifdef NXRT_UPDATE_REPO
constexpr const char* kRepo = NXRT_UPDATE_REPO;
#else
constexpr const char* kRepo = "";
#endif
#ifdef NXRT_RELEASE_BY_TAG
constexpr bool kByTag = true;
#else
constexpr bool kByTag = false;
#endif

constexpr size_t kMaxAnswer = 4u << 20;   // release notes and asset lists are a few tens of KiB

struct State {
    std::atomic<bool> started{false};
    std::mutex mu;
    std::string found;   // the newer version, "" while unknown or when there is none
};
// never destroyed: the request's thread may outlive static destruction at exit
State& state() {
    static State* s = new State;
    return *s;
}

bool enabled() { return config_get_bool("update", "check", true); }

// The release this game belongs to and where it is published. EmblemWeave builds the runtime on the player's PC, so its launcher writes
// [update] version and repo into fefw.ini at every Play; a release package built elsewhere may name them at build time instead.
// Owner/name only (ASCII letters, digits, '-', '_', '.'), so the request path cannot be steered anywhere else.
bool repo_ok(const std::string& r) {
    const size_t slash = r.find('/');
    if (r.empty() || r.size() > 100 || slash == std::string::npos || slash == 0 || slash + 1 == r.size() || r.find('/', slash + 1) != std::string::npos)
        return false;
    for (const char c : r) {
        const bool ok = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '-' || c == '_' || c == '.' || c == '/';
        if (!ok) return false;
    }
    return true;
}
std::string current() {
    const std::string v = config_get_str("update", "version", "");
    return v.empty() ? std::string(kCurrent) : v;
}
std::string repo() {
    const std::string r = config_get_str("update", "repo", "");
    return repo_ok(r) ? r : (repo_ok(kRepo) ? std::string(kRepo) : std::string());
}

struct Handle {
    HINTERNET h = nullptr;
    ~Handle() {
        if (h) WinHttpCloseHandle(h);
    }
    explicit operator bool() const { return h != nullptr; }
};

#ifdef _WIN32
// The body of GET https://api.github.com/repos/<repo>/releases/latest, false with *why.
bool fetch_latest(const char* repo, std::string* body, std::string* why) {
    const auto failed = [why](const char* what) {
        *why = std::string(what) + " failed, error " + std::to_string(GetLastError());
        return false;
    };
    Handle session{WinHttpOpen(L"fefw-recomp", WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_NO_PROXY_NAME, WINHTTP_NO_PROXY_BYPASS, 0)};
    if (!session) return failed("WinHttpOpen");
    WinHttpSetTimeouts(session.h, 10000, 10000, 10000, 10000);
    Handle connect{WinHttpConnect(session.h, L"api.github.com", INTERNET_DEFAULT_HTTPS_PORT, 0)};
    if (!connect) return failed("WinHttpConnect");
    std::wstring path = L"/repos/";
    for (const char* p = repo; *p; ++p) path.push_back(static_cast<unsigned char>(*p));   // owner/name: ASCII
    path += L"/releases/latest";
    Handle request{WinHttpOpenRequest(connect.h, L"GET", path.c_str(), nullptr, WINHTTP_NO_REFERER, WINHTTP_DEFAULT_ACCEPT_TYPES,
                                      WINHTTP_FLAG_SECURE)};
    if (!request) return failed("WinHttpOpenRequest");
    if (!WinHttpSendRequest(request.h, L"Accept: application/vnd.github+json\r\n", static_cast<DWORD>(-1), WINHTTP_NO_REQUEST_DATA, 0, 0, 0))
        return failed("WinHttpSendRequest");
    if (!WinHttpReceiveResponse(request.h, nullptr)) return failed("WinHttpReceiveResponse");
    DWORD status = 0, size = sizeof status;
    if (!WinHttpQueryHeaders(request.h, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_HEADER_NAME_BY_INDEX, &status, &size,
                             WINHTTP_NO_HEADER_INDEX))
        return failed("WinHttpQueryHeaders");
    if (status != 200) {
        *why = "GitHub answered " + std::to_string(status);
        return false;
    }
    body->clear();
    for (;;) {
        char buf[16384];
        DWORD got = 0;
        if (!WinHttpReadData(request.h, buf, sizeof buf, &got)) return failed("WinHttpReadData");
        if (!got) return true;
        if (body->size() + got > kMaxAnswer) {
            *why = "the answer is too large";
            return false;
        }
        body->append(buf, got);
    }
}

#else
bool fetch_latest(const char*, std::string*, std::string* why) {
    *why = "this platform has no request yet";
    return false;
}
#endif

void run(std::string repo_name, std::string current_version) {
#ifdef _WIN32
    SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
#endif
    const char* kCurrent = current_version.c_str();   // the name the messages below use
    std::string body, why;
    if (!fetch_latest(repo_name.c_str(), &body, &why)) {
        NXLOG_I(kTag, "no answer about newer releases: %s", why.c_str());
        return;
    }
    const std::string latest = release_version(body);
    if (latest.empty()) {
        NXLOG_I(kTag, "GitHub's answer names no release");
        return;
    }
    if (!is_newer(latest, kCurrent, kByTag)) {
        NXLOG_I(kTag, "this is release %s; the latest published is %s", kCurrent, latest.c_str());
        return;
    }
    NXLOG_I(kTag, "release %s is available (this is %s)", latest.c_str(), kCurrent);
    State& s = state();
    std::lock_guard<std::mutex> lk(s.mu);
    s.found = latest;
}

// "v1.2.3" or "1.2.3-x" -> {1, 2, 3}; false without a leading number
bool parse_version(std::string_view s, uint64_t (&out)[3]) {
    while (!s.empty() && (s.front() == ' ' || s.front() == 'v' || s.front() == 'V')) s.remove_prefix(1);
    out[0] = out[1] = out[2] = 0;
    for (int part = 0; part < 3; ++part) {
        size_t n = 0;
        while (n < s.size() && s[n] >= '0' && s[n] <= '9') {
            if (out[part] > 1000000000u) return false;
            out[part] = out[part] * 10 + static_cast<uint64_t>(s[n++] - '0');
        }
        if (!n) return part > 0;
        const size_t dot = s.find('.');
        if (dot == std::string_view::npos) break;
        s.remove_prefix(dot + 1);
    }
    return true;
}

std::string_view without_v(std::string_view s) {
    while (!s.empty() && (s.front() == 'v' || s.front() == 'V')) s.remove_prefix(1);
    return s;
}

}  // namespace

std::string current_version() { return current(); }

void start() {
    std::string cur = current(), rep = repo();
    if (cur.empty() || rep.empty() || !enabled()) return;
    if (state().started.exchange(true)) return;
    std::thread(run, std::move(rep), std::move(cur)).detach();
}

std::string available() {
    if (!enabled()) return {};
    State& s = state();
    std::lock_guard<std::mutex> lk(s.mu);
    return s.found;
}

std::string release_version(std::string_view json) {
    // in a string value (the release notes) the key's quotes are escaped, so this only finds the key
    constexpr std::string_view kKey = "\"tag_name\"";
    size_t i = json.find(kKey);
    if (i == std::string_view::npos) return {};
    i += kKey.size();
    const auto skip_space = [&] {
        while (i < json.size() && (json[i] == ' ' || json[i] == '\t' || json[i] == '\r' || json[i] == '\n')) ++i;
    };
    skip_space();
    if (i >= json.size() || json[i++] != ':') return {};
    skip_space();
    if (i >= json.size() || json[i++] != '"') return {};
    const size_t end = json.find('"', i);
    if (end == std::string_view::npos) return {};
    const std::string_view tag = without_v(json.substr(i, end - i));
    if (tag.empty() || tag.size() > 64) return {};
    for (const char c : tag) {
        const bool ok = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '.' || c == '_' || c == '+' || c == '-';
        if (!ok) return {};
    }
    return std::string(tag);
}

bool is_newer(std::string_view latest, std::string_view current, bool by_tag) {
    if (by_tag) return !latest.empty() && without_v(latest) != without_v(current);
    uint64_t a[3], b[3];
    if (!parse_version(latest, a) || !parse_version(current, b)) return false;
    for (int i = 0; i < 3; ++i)
        if (a[i] != b[i]) return a[i] > b[i];
    return false;
}

}  // namespace nxrt::update
