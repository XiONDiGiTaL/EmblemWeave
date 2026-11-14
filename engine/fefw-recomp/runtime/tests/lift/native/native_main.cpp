// SPDX-License-Identifier: GPL-3.0-or-later
// Native driver of a test program: runs test_main() and writes results[] (NRES little-endian u64 words) to argv[1].
#include <cstdio>
#include <cstdint>
extern uint64_t results[512];   // defined by the program (t.h: u64 results[NRES])
extern "C" void test_main(void);
int main(int argc, char** argv) {
    if (argc < 2) { std::fprintf(stderr, "usage: native_run <out.expected>\n"); return 2; }
    test_main();
    FILE* f = std::fopen(argv[1], "wb");
    if (!f) { std::fprintf(stderr, "cannot write %s\n", argv[1]); return 3; }
    std::fwrite(results, sizeof(uint64_t), 512, f);
    std::fclose(f);
    return 0;
}
