// SPDX-License-Identifier: GPL-3.0-or-later
// Exercises nxrt/platform.h directly, on whichever backend it was built with: the point of the split is that the runtime
// asks for these five things and nothing else, so a backend that answers them wrongly is caught here and not in a game.
// Run on its own (`nxrt_platform_selftest`) or through CTest as `platform_selftest`.
#include <atomic>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

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
            // The third argument is the POSIX-style protection mask, not a bool: false/true would be PROT_NONE and
            // PROT_READ, and reading the page after `false` faults.
            check(protect(p, span, PROT_READ), "protect read-only");
            check(p[page] == 2, "the data survives the protection change");
            check(protect(p, span, PROT_READ | PROT_WRITE), "protect writable again");
            p[page] = 3;
            check(p[page] == 3, "a writable page takes a write after the protection round trip");
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

    // files: positioned I/O, truncate, rename, times, and an update open that does not truncate.
    {
        const std::string dir = temp_dir();
        const std::string a = dir + "/pos.bin", b = dir + "/pos-renamed.bin";
        File* w = file_create_new(a.c_str());
        check(w != nullptr, "file_create_new creates a new file");
        check(file_create_new(a.c_str()) == nullptr, "file_create_new refuses a file that exists");
        if (w) {
            const char digits[] = "0123456789";
            check(file_write_at(w, 4, digits, 10), "file_write_at writes at an offset");
            check(file_size(w) == 14, "the file grows to the end of the positioned write");
            char four[4] = {};
            std::size_t got = 0;
            check(file_read_at(w, 2, four, 4, &got) && got == 4, "file_read_at reads across the gap");
            check(four[0] == 0 && four[1] == 0 && four[2] == '0' && four[3] == '1', "the gap reads as zeros and the data follows it");
            check(file_truncate(w, 6), "file_truncate shortens the file");
            check(file_size(w) == 6, "the size follows the truncate");
            check(file_flush(w), "file_flush succeeds");
            file_close(w);
        }
        check(file_sync(a.c_str()), "file_sync flushes a file by name");
        FileTimes times{};
        check(file_times(a.c_str(), &times) && times.modified > 0, "file_times reports a write time");
        check(file_rename(a.c_str(), b.c_str(), true), "file_rename moves the file");
        check(!file_exists(a.c_str()) && file_exists(b.c_str()), "the renamed file is the one that exists");
        File* u = file_open_update(b.c_str());
        check(u != nullptr, "file_open_update opens an existing file for writing");
        if (u) {
            char first = 1;
            std::size_t got = 0;
            check(file_read_at(u, 0, &first, 1, &got) && got == 1 && first == 0, "the update open did not truncate the file");
            file_close(u);
        }
    }

    // named mutex: the name is kept while a handle is open, and the explicit lock is what a caller holds it with. The
    // save folder takes this lock around commit and recovery, so a second process cannot apply the same commit.
    {
        const std::string name = "nxrt_selftest_mutex_" + std::to_string(process_id());
        bool already = false;
        void* m = named_mutex_create(name.c_str(), &already);
        check(m != nullptr, "named_mutex_create creates a named mutex");
        check(!already, "a fresh name is not already held");
        check(named_mutex_lock(m), "named_mutex_lock takes the mutex");
        named_mutex_unlock(m);
        check(named_mutex_lock(m), "named_mutex_lock takes it again after an unlock");
        named_mutex_unlock(m);
        named_mutex_destroy(m);
    }

    // process identity, library names, the CSPRNG and the scheduler snapshot.
    check(process_id() != 0, "process_id names this process");
    check(process_alive(process_id()), "process_alive sees this process");
    check(!process_alive(0x7FFFFFF0u), "process_alive does not see a pid nothing runs under");
    {
        int version = 0;
#if defined(_WIN32)
        check(library_file_name("avcodec", "avcodec-62.dll", &version) && version == 62, "library_file_name knows avcodec-62.dll (version 62)");
#else
        check(library_file_name("avcodec", "libavcodec.so.62", &version) && version == 62, "library_file_name knows libavcodec.so.62 (version 62)");
#endif
        check(!library_file_name("avcodec", "avcodec.txt", &version), "library_file_name refuses a file that is not a library");
    }
    {
        unsigned char bytes[16] = {};
        check(random_bytes(bytes, sizeof bytes), "random_bytes fills a buffer");
    }
    {
        std::vector<std::uint32_t> ids;
        process_thread_ids(&ids);
        bool self = false;
        for (const std::uint32_t id : ids) self = self || id == thread_id();
        check(!ids.empty() && self, "process_thread_ids lists the calling thread");
        check(host_thread_ideal_processor(thread_id()) >= 0, "host_thread_ideal_processor answers for this thread");
        check(physical_core_mask(0) != 0, "physical_core_mask answers for processor 0");
        HostSnapshot snapshot;
        check(host_snapshot(&snapshot) && !snapshot.threads.empty(), "host_snapshot lists this process's threads");
        check(!snapshot.processes.empty(), "host_snapshot lists other processes");
    }

    std::printf("%d failure(s)\n", failures);
    return failures == 0 ? 0 : 1;
}
