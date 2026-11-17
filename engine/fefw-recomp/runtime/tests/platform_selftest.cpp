// SPDX-License-Identifier: GPL-3.0-or-later
// Exercises nxrt/platform.h directly, on whichever backend it was built with: the point of the split is that the runtime
// asks for these five things and nothing else, so a backend that answers them wrongly is caught here and not in a game.
// Run on its own (`nxrt_platform_selftest`) or through CTest as `platform_selftest`.
#include <atomic>
#include <cstdio>
#include <cstring>
#include <string>

#include "nxrt/platform.h"

namespace {

int failures = 0;

void check(bool ok, const char* what) {
    std::printf("%s %s\n", ok ? "ok  " : "FAIL", what);
    if (!ok) ++failures;
}

std::string temp_dir() {
#if defined(_WIN32)
    const char* temp = std::getenv("TEMP");
    return std::string(temp && *temp ? temp : ".") + "\\nxrt-platform-test";
#else
    return "/tmp/nxrt-platform-test";
#endif
}

}  // namespace

int main() {
    using namespace nxrt::platform;

    // memory: reserve, commit, write, read back, protect, release.
    const std::size_t page = page_size();
    check(page >= 4096 && (page & (page - 1)) == 0, "page_size is a power of two at least 4 KiB");
    {
        const std::size_t span = page * 8;
        auto* p = static_cast<unsigned char*>(reserve(span));
        check(p != nullptr, "reserve returns an address");
        if (p) {
            check((reinterpret_cast<std::uintptr_t>(p) % page) == 0, "reserve is page aligned");
            check(commit(p, span), "commit the reserved range");
            for (std::size_t i = 0; i < span; i += page) p[i] = static_cast<unsigned char>(i / page + 1);   // faults if not writable
            check(p[page] == 2, "committed pages hold what was written");
            check(protect(p, span, false), "protect read-only");
            check(p[page] == 2, "the data survives the protection change");
            check(protect(p, span, true), "protect writable again");
            release(p, span);
        }
    }

    // time: monotonic, and a wait of at least the span asked for.
    {
        const std::uint64_t hz = clock_hz();
        check(hz != 0, "clock_hz is non-zero");
        const std::uint64_t a = clock_ticks();
        const std::uint64_t b = clock_ticks();
        check(b >= a, "clock_ticks does not go backwards");
        const std::int64_t n0 = now_ns();
        sleep_ns(2000000);   // 2 ms
        const std::int64_t n1 = now_ns();
        check(n1 - n0 >= 1500000, "sleep_ns waits at least most of the span (>= 1.5 ms of 2 ms)");
        check(n1 - n0 < 500000000, "sleep_ns does not hang (2 ms took under 500 ms)");
    }

    // synchronisation and threads: a worker signals a condition, the main thread joins it.
    {
        Mutex* m = mutex_create();
        Cond* c = cond_create();
        int flag = 0;
        struct Shared {
            Mutex* m;
            Cond* c;
            int* flag;
        } shared{m, c, &flag};
        Thread* t = thread_start("nxrt-platform-test", [](void* arg) {
            auto* s = static_cast<Shared*>(arg);
            mutex_lock(s->m);
            *s->flag = 1;
            mutex_unlock(s->m);
            cond_signal(s->c);
        }, &shared);
        check(t != nullptr, "thread_start returns a handle");
        mutex_lock(m);
        while (!flag) cond_wait(c, m);
        mutex_unlock(m);
        thread_join(t);
        check(flag == 1, "the worker ran and signalled");
        cond_destroy(c);
        mutex_destroy(m);
    }
    check(thread_id() != 0, "thread_id names the calling thread");

    // files: create, write, size, seek, read back, exists, mkdirs.
    {
        const std::string dir = temp_dir();
        check(file_mkdirs(dir.c_str()), "file_mkdirs creates the folder");
        const std::string path = dir + "/selftest.bin";
        const char payload[] = "nxrt platform file io";
        File* w = file_open(path.c_str(), true);
        check(w != nullptr, "file_open for writing creates the file");
        if (w) {
            check(file_write(w, payload, sizeof payload) == sizeof payload, "file_write writes every byte");
            check(file_size(w) == static_cast<int64_t>(sizeof payload), "file_size reports the bytes written");
            file_close(w);
        }
        check(file_exists(path.c_str()), "file_exists finds it");
        File* r = file_open(path.c_str(), false);
        check(r != nullptr, "file_open for reading finds it");
        if (r) {
            char back[sizeof payload]{};
            check(file_read(r, back, sizeof back) == sizeof payload, "file_read reads every byte");
            check(std::memcmp(back, payload, sizeof payload) == 0, "the bytes read back are the bytes written");
            check(file_seek(r, 0), "file_seek returns to the start");
            char first = 0;
            check(file_read(r, &first, 1) == 1 && first == 'n', "the byte after the seek is the first one");
            file_close(r);
        }
        check(file_open((dir + "/nope.bin").c_str(), false) == nullptr, "opening a missing file fails instead of crashing");
    }

    std::printf("%d failure(s)\n", failures);
    return failures == 0 ? 0 : 1;
}
