// SPDX-License-Identifier: GPL-3.0-or-later
#include "nxrt/log.h"

#include "nxrt/config.h"

#ifdef _WIN32
#include <windows.h>
#else
#include <pthread.h>
#endif

#include "nxrt/platform.h"

#include <atomic>
#include <condition_variable>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <mutex>
#include <string>
#include <thread>
#include <unordered_map>
#include <vector>

namespace nxrt {
namespace {

constexpr size_t kMaxMessage = 2000;     // longer messages are cut to this many bytes and get "..."
constexpr size_t kFormatBuffer = 4096;   // vsnprintf target; anything longer is cut anyway
constexpr unsigned kFlushEvery = 100;    // lines between flushes for Info and below
constexpr char kLevelLetters[] = "EWIDT";
constexpr int kLevelCount = 5;

// One lock (g_mu) guards the file, counters, sink and time base. g_level and g_inited are atomics so the
// level check on the hot path takes no lock. Config is read only before g_mu is taken (see resolve()).
std::recursive_mutex g_mu;   // recursive so a sink that logs does not deadlock
std::atomic<int> g_level{static_cast<int>(LogLevel::Info)};
std::atomic<bool> g_inited{false};
std::FILE* g_file = nullptr;
std::string g_path;          // path of the open file; empty when none
LogSink g_sink = nullptr;
void* g_sink_user = nullptr;
bool g_to_stderr = true;
bool g_stderr_all = false;
uint64_t g_counts[kLevelCount] = {};
unsigned g_since_flush = 0;
int64_t g_t0 = 0;           // platform::now_ns() at log_init

thread_local char t_name[64] = {};

// [log] async (default true; env NXRT_LOG_ASYNC): Info and lower lines go to a writer thread, which writes and flushes them, so the
// thread that logs (the render thread's hitch and census lines, the replay's stats) no longer pays for the file write and the flush. Errors,
// log_flush and log_shutdown write everything queued and the line itself on the calling thread, in order, so a crash report is complete.
// Warnings are queued too but the writer flushes at once. g_fmu orders the file: the writer and the synchronous paths take it, then take
// the queue (g_qmu), so lines stay in the order they were logged. Read once at log start.
std::recursive_mutex g_fmu;                // the file writes (recursive: a crash inside fwrite may log again on the same thread)
std::mutex g_qmu;
std::condition_variable g_qcv;
std::vector<std::string> g_queue;          // formatted lines, newline included
bool g_queue_flush = false;                // a queued warning: the writer flushes after this batch
bool g_writer_started = false;
int g_async = -1;                          // -1: not read yet

// [log] repeat_limit (default 30; env NXRT_LOG_REPEAT_LIMIT; 0 = off): at most this many Info / Debug / Trace lines from one call site
// (one format string) in 10 s; the rest are counted and named in one line when the site logs again after the window. Errors, warnings
// and the start-up settings dump (tag "config") are never held back.
int g_repeat_limit = -1;
struct Site {
    long long window_start = 0;     // platform::now_ns()
    unsigned lines = 0, dropped = 0;
    const char* tag = nullptr;
};
std::unordered_map<const char*, Site> g_sites;   // g_mu held

// The file part of one batch. g_fmu held.
void write_lines_locked(const std::vector<std::string>& lines, bool flush) {
    if (!g_file) return;
    for (const std::string& l : lines) std::fwrite(l.data(), 1, l.size(), g_file);
    if (flush) std::fflush(g_file);
}

// Writes what the queue holds (and, when given, one more line after it) on the calling thread.
void drain_sync(const std::string* extra, bool flush) {
    std::lock_guard<std::recursive_mutex> f(g_fmu);
    std::vector<std::string> batch;
    {
        std::lock_guard<std::mutex> q(g_qmu);
        batch.swap(g_queue);
        g_queue_flush = false;
    }
    if (extra) batch.push_back(*extra);
    write_lines_locked(batch, flush);
}

void writer_loop() {
    std::vector<std::string> batch;
    for (;;) {
        {
            std::unique_lock<std::mutex> q(g_qmu);
            g_qcv.wait(q, [] { return !g_queue.empty(); });
        }
        std::lock_guard<std::recursive_mutex> f(g_fmu);
        bool flush;
        {
            std::lock_guard<std::mutex> q(g_qmu);
            batch.swap(g_queue);
            flush = g_queue_flush;
            g_queue_flush = false;
        }
        // a batch is written and flushed whole: the file stays at most one batch behind, and a warning reaches the disk at once
        write_lines_locked(batch, true);
        (void)flush;
        batch.clear();
    }
}

bool async_on() { return g_async > 0; }   // g_mu held

struct Resolved {
    std::string path;
    LogLevel level;
    bool to_stderr;
    bool stderr_all;
    bool keep_previous;
    int async;
    int repeat_limit;
};

// Final settings: env > [log] config keys > options. Reads config, so it must be called without g_mu held.
Resolved resolve(const LogOptions& opt) {
    Resolved r{};
    r.path = opt.path ? opt.path : "nxrt.log";
    r.level = opt.level;
    r.to_stderr = opt.to_stderr;
    r.stderr_all = opt.stderr_all;
    r.keep_previous = opt.keep_previous;
    if (const char* e = std::getenv("NXRT_LOG")) {
        r.path = e;
    } else if (config_has("log", "path")) {
        r.path = config_get_str("log", "path");
    }
    // [log] async / repeat_limit: read here, without g_mu (a config read can itself log)
    if (const char* e = std::getenv("NXRT_LOG_ASYNC")) r.async = (e[0] != '0' && e[0] != 'f' && e[0] != 'F') ? 1 : 0;
    else r.async = config_get_bool("log", "async", true) ? 1 : 0;
    if (const char* e = std::getenv("NXRT_LOG_REPEAT_LIMIT")) r.repeat_limit = std::atoi(e);
    else r.repeat_limit = static_cast<int>(config_get_int("log", "repeat_limit", 30));
    if (const char* e = std::getenv("NXRT_LOG_LEVEL")) {
        r.level = log_parse_level(e, r.level);
    } else if (config_has("log", "level")) {
        std::string lv = config_get_str("log", "level");
        r.level = log_parse_level(lv.c_str(), r.level);
    }
    return r;
}

// Moves an existing file to <path>.prev, replacing any older .prev.
void rotate_previous(const std::string& path) {
    std::FILE* old = std::fopen(path.c_str(), "rb");
    if (!old) return;
    std::fclose(old);
    std::string prev = path + ".prev";
    std::remove(prev.c_str());
    std::rename(path.c_str(), prev.c_str());
}

// Applies resolved settings: closes the old file, resets counters, opens the new file.
void apply(const Resolved& r, bool only_if_uninit) {
    std::lock_guard<std::recursive_mutex> g(g_mu);
    if (only_if_uninit && g_inited.load(std::memory_order_acquire)) return;
    drain_sync(nullptr, true);
    std::lock_guard<std::recursive_mutex> f(g_fmu);
    g_sites.clear();
    if (g_file) {
        std::fclose(g_file);
        g_file = nullptr;
    }
    g_path.clear();
    g_level.store(static_cast<int>(r.level), std::memory_order_relaxed);
    g_to_stderr = r.to_stderr;
    g_async = r.async;
    g_repeat_limit = r.repeat_limit;
    g_stderr_all = r.stderr_all;
    std::memset(g_counts, 0, sizeof g_counts);
    g_since_flush = 0;

    g_t0 = platform::now_ns();

    if (!r.path.empty()) {
        if (r.keep_previous) rotate_previous(r.path);
        if (std::FILE* f = std::fopen(r.path.c_str(), "w")) {
            g_file = f;
            g_path = r.path;
        } else {
            std::fprintf(stderr, "nxrt: cannot open log file '%s'\n", r.path.c_str());
        }
    }
    g_inited.store(true, std::memory_order_release);
}

double seconds_since_init_locked() {
    return static_cast<double>(platform::now_ns() - g_t0) * 1e-9;
}

}  // namespace

void log_init(const LogOptions& opt) {
    Resolved r = resolve(opt);  // config reads happen before the log lock is taken
    apply(r, false);
}

void log_shutdown() {
    std::lock_guard<std::recursive_mutex> g(g_mu);
    drain_sync(nullptr, true);
    std::lock_guard<std::recursive_mutex> f(g_fmu);
    if (g_file) {
        std::fflush(g_file);
        std::fclose(g_file);
        g_file = nullptr;
    }
    g_path.clear();
    // g_inited stays set: later logf calls do not re-open anything until log_init is called again.
}

void log_flush() {
    std::lock_guard<std::recursive_mutex> g(g_mu);
    drain_sync(nullptr, true);
}

void log_set_level(LogLevel l) { g_level.store(static_cast<int>(l), std::memory_order_relaxed); }

LogLevel log_level() { return static_cast<LogLevel>(g_level.load(std::memory_order_relaxed)); }

bool log_enabled(LogLevel l) { return static_cast<int>(l) <= g_level.load(std::memory_order_relaxed); }

const char* log_path() {
    std::lock_guard<std::recursive_mutex> g(g_mu);
    return g_path.c_str();  // valid until the next log_init or log_shutdown
}

void log_set_thread_name(const char* name) {
    std::snprintf(t_name, sizeof t_name, "%s", name ? name : "");
#ifdef _WIN32
    // Also the Windows thread description, so the [diag] budget thread table and external profilers see the runtime's worker names.
    using SetDescFn = HRESULT(WINAPI*)(HANDLE, PCWSTR);
    static const SetDescFn set_desc = reinterpret_cast<SetDescFn>(
        reinterpret_cast<void*>(GetProcAddress(GetModuleHandleW(L"kernel32.dll"), "SetThreadDescription")));
    if (set_desc && t_name[0]) {
        wchar_t w[sizeof t_name];
        size_t i = 0;
        for (; t_name[i] && i + 1 < sizeof t_name; ++i) w[i] = static_cast<wchar_t>(static_cast<unsigned char>(t_name[i]));
        w[i] = 0;
        set_desc(GetCurrentThread(), w);
    }
#else
    // The same name in the kernel's comm field, which top/htop and debuggers show (Linux keeps 15 characters).
    if (t_name[0]) pthread_setname_np(pthread_self(), t_name);
#endif
}

const char* log_thread_name() { return t_name; }

static std::string trimmed_lower(const char* s) {
    std::string v = s;
    size_t b = v.find_first_not_of(" \t\r\n");
    size_t e = v.find_last_not_of(" \t\r\n");
    v = (b == std::string::npos) ? std::string() : v.substr(b, e - b + 1);
    for (char& c : v)
        if (c >= 'A' && c <= 'Z') c = static_cast<char>(c - 'A' + 'a');
    return v;
}

LogLevel log_parse_level(const char* s, LogLevel fallback) {
    if (!s) return fallback;
    std::string v = trimmed_lower(s);
    if (v == "error") return LogLevel::Error;
    if (v == "warn") return LogLevel::Warn;
    if (v == "info") return LogLevel::Info;
    if (v == "debug") return LogLevel::Debug;
    if (v == "trace") return LogLevel::Trace;
    if (v.size() == 1 && v[0] >= '0' && v[0] <= '4') return static_cast<LogLevel>(v[0] - '0');
    return fallback;
}

const char* log_level_name(LogLevel l) {
    switch (l) {
        case LogLevel::Error: return "error";
        case LogLevel::Warn: return "warn";
        case LogLevel::Info: return "info";
        case LogLevel::Debug: return "debug";
        case LogLevel::Trace: return "trace";
    }
    return "?";
}

uint64_t log_count(LogLevel l) {
    int i = static_cast<int>(l);
    if (i < 0 || i >= kLevelCount) return 0;
    std::lock_guard<std::recursive_mutex> g(g_mu);
    return g_counts[i];
}

void log_set_sink(LogSink sink, void* user) {
    std::lock_guard<std::recursive_mutex> g(g_mu);
    g_sink = sink;
    g_sink_user = user;
}

void logv(LogLevel l, const char* tag, const char* fmt, va_list ap) {
    if (!g_inited.load(std::memory_order_acquire)) {
        Resolved r = resolve(LogOptions{});  // first use: defaults, plus env and config
        apply(r, true);
    }
    const int li = static_cast<int>(l);
    if (li < 0 || li >= kLevelCount) return;
    if (li > g_level.load(std::memory_order_relaxed)) return;  // filtered: not counted, not written

    char body[kFormatBuffer];
    body[0] = '\0';
    std::vsnprintf(body, sizeof body, fmt ? fmt : "", ap);
    std::string msg = body;
    for (char& c : msg)
        if (c == '\n' || c == '\r') c = ' ';
    if (msg.size() > kMaxMessage) {
        size_t cut = kMaxMessage;
        while (cut > 0 && (static_cast<unsigned char>(msg[cut]) & 0xC0) == 0x80) --cut;  // do not split UTF-8
        msg.resize(cut);
        msg += "...";
    }

    std::lock_guard<std::recursive_mutex> g(g_mu);
    const bool async = async_on();
    std::string note;   // a repeat-limit summary that goes before this line
    // the start-up settings dump ("config" lines, all from one format) is always kept whole
    if (g_repeat_limit > 0 && li > static_cast<int>(LogLevel::Warn) && fmt && !(tag && std::strcmp(tag, "config") == 0)) {
        const long long now = platform::now_ns();
        Site& st = g_sites[fmt];
        if (!st.lines || now - st.window_start >= 10'000'000'000ll) {
            if (st.dropped) {
                char b[200];
                std::snprintf(b, sizeof b, "log: %u more lines from %s's \"%.60s\" in 10 s were left out ([log] repeat_limit = %d)", st.dropped,
                              st.tag ? st.tag : "?", fmt, g_repeat_limit);
                note = b;
            }
            st.window_start = now;
            st.lines = 0;
            st.dropped = 0;
            st.tag = tag;
        }
        if (++st.lines > static_cast<unsigned>(g_repeat_limit)) {
            st.dropped++;
            return;   // not counted, not written
        }
    }
    char prefix[256];
    std::snprintf(prefix, sizeof prefix, "[%10.6f %c t%lu %s] ", seconds_since_init_locked(), kLevelLetters[li],
                  static_cast<unsigned long>(platform::thread_id()), t_name[0] ? t_name : "-");
    std::string line = prefix;
    if (!note.empty()) {
        std::string nl = line + note;
        line += tag ? tag : "";
        line += ": ";
        line += msg;
        note = nl;
    } else {
        line += tag ? tag : "";
        line += ": ";
        line += msg;
    }

    g_counts[li]++;
    if (g_file) {
        std::string out;
        if (!note.empty()) out = note + "\n";
        out += line;
        out += '\n';
        if (async && li > static_cast<int>(LogLevel::Error)) {
            {
                std::lock_guard<std::mutex> q(g_qmu);
                g_queue.push_back(std::move(out));
                if (li <= static_cast<int>(LogLevel::Warn)) g_queue_flush = true;
            }
            if (!g_writer_started) {
                g_writer_started = true;
                std::thread(writer_loop).detach();
            }
            g_qcv.notify_one();
        } else {
            // synchronous: errors (a crash report must be on disk), or [log] async = false (the old path: flush on warnings and every 100 lines)
            const bool flush = li <= static_cast<int>(LogLevel::Warn) || ++g_since_flush >= kFlushEvery;
            if (flush) g_since_flush = 0;
            drain_sync(&out, flush || async);
        }
    }
    if (g_to_stderr && (g_stderr_all || li <= static_cast<int>(LogLevel::Warn))) {
        if (!note.empty()) std::fprintf(stderr, "%s\n", note.c_str());
        std::fprintf(stderr, "%s\n", line.c_str());
    }
    if (g_sink) {
        if (!note.empty()) g_sink(LogLevel::Info, note.c_str(), g_sink_user);
        g_sink(l, line.c_str(), g_sink_user);
    }
}

void logf(LogLevel l, const char* tag, const char* fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    logv(l, tag, fmt, ap);
    va_end(ap);
}

}  // namespace nxrt
