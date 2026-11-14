// SPDX-License-Identifier: GPL-3.0-or-later
#include "test_framework.h"

#include "nxrt/tt_inputfile.h"

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

NXTEST_GROUP(tt_inputfile)

using nxrt::tt::InputTrace;
using nxrt::tt::PadFrame;

namespace {

constexpr uint64_t kA = 1ull << 0;

// Synthetic pad sequence: A held 10..20, left stick sweep 40..60 (then back to 0 at 61), mouse impulses at 30 and 31,
// right stick at 80 (back to 0 at 81). Everything else is idle.
PadFrame synth(uint64_t f) {
    PadFrame p;
    if (f >= 10 && f <= 20) p.buttons |= kA;
    if (f >= 40 && f <= 60) p.lx = static_cast<int16_t>((static_cast<int>(f) - 40) * 1000);
    if (f == 30) {
        p.mouse_dx = 3.0f;
        p.mouse_dy = -2.0f;
    }
    if (f == 31) {
        p.mouse_dx = 1.5f;
        p.mouse_dy = 0.25f;
    }
    if (f == 80) {
        p.rx = -32767;
        p.ry = 32767;
    }
    return p;
}

void record_all(InputTrace& t, uint64_t frames) {
    for (uint64_t f = 0; f < frames; f++) t.record(f, synth(f));
}

bool same_entries(const InputTrace& a, const InputTrace& b) {
    if (a.entries.size() != b.entries.size()) return false;
    for (size_t i = 0; i < a.entries.size(); i++) {
        if (a.entries[i].frame != b.entries[i].frame || !(a.entries[i].pad == b.entries[i].pad)) return false;
    }
    return true;
}

bool same_bits(float a, float b) { return std::memcmp(&a, &b, sizeof a) == 0; }

}  // namespace

NXTEST(record_100_frames_minimal_entries) {
    InputTrace t;
    record_all(t, 100);
    CHECK_EQ(t.end_frame, uint64_t(100));

    // Frames 0, 10, 21 (A press/release), 30, 31, 32 (impulses and the zeroing line), 41..61 (stick sweep), 80, 81.
    std::vector<uint64_t> expect = {0, 10, 21, 30, 31, 32};
    for (uint64_t f = 41; f <= 61; f++) expect.push_back(f);
    expect.push_back(80);
    expect.push_back(81);
    REQUIRE(t.entries.size() == expect.size());
    CHECK_EQ(t.entries.size(), size_t(29));
    for (size_t i = 0; i < expect.size(); i++) {
        CHECK_MSG(t.entries[i].frame == expect[i], "entry " + std::to_string(i) + " at frame " + std::to_string(t.entries[i].frame));
    }
}

NXTEST(at_returns_fed_state_every_frame) {
    InputTrace t;
    record_all(t, 100);
    for (uint64_t f = 0; f < 100; f++) {
        PadFrame got = t.at(f);
        PadFrame want = synth(f);
        CHECK_MSG(got == want, "frame " + std::to_string(f));
    }
    // Past the end, the held state of the last entry (idle) with no impulse.
    CHECK(t.at(150) == PadFrame{});
}

NXTEST(text_round_trip) {
    InputTrace t;
    record_all(t, 100);
    std::string text = t.to_text();

    InputTrace u;
    std::string err;
    bool ok = u.from_text(text, &err);
    CHECK_MSG(ok, err);
    REQUIRE(ok);
    CHECK(same_entries(t, u));
    CHECK_EQ(u.end_frame, t.end_frame);
    CHECK_EQ(u.to_text(), text);
}

NXTEST(empty_trace_text) {
    InputTrace t;
    CHECK_EQ(t.to_text(), std::string("# nxin 1\n"));
    CHECK(t.at(0) == PadFrame{});
}

NXTEST(file_round_trip) {
    const char* path = "tt_inputfile_test.nxin";
    InputTrace t;
    record_all(t, 100);

    std::string werr;
    bool wok = t.write(path, &werr);
    InputTrace u;
    std::string rerr;
    bool rok = wok && u.read(path, &rerr);
    std::remove(path);

    CHECK_MSG(wok, werr);
    CHECK_MSG(rok, rerr);
    REQUIRE(wok);
    REQUIRE(rok);
    CHECK(same_entries(t, u));
    CHECK_EQ(u.end_frame, t.end_frame);
}

NXTEST(read_missing_file_reports_path) {
    InputTrace u;
    std::string err;
    CHECK(!u.read("tt_inputfile_no_such_file.nxin", &err));
    CHECK_EQ(err, std::string("cannot open tt_inputfile_no_such_file.nxin"));
}

NXTEST(floats_survive_bit_exact) {
    const float vals[4] = {0.1f, 1e-7f, 1.0f / 3.0f, -123456.79f};
    InputTrace t;
    for (uint64_t f = 0; f < 4; f++) {
        PadFrame p;
        p.mouse_dx = vals[f];
        p.mouse_dy = -vals[f];
        t.record(f, p);
    }
    REQUIRE(t.entries.size() == 4);

    InputTrace u;
    std::string err;
    bool ok = u.from_text(t.to_text(), &err);
    CHECK_MSG(ok, err);
    REQUIRE(ok);
    REQUIRE(u.entries.size() == 4);
    for (uint64_t f = 0; f < 4; f++) {
        CHECK_MSG(same_bits(u.entries[f].pad.mouse_dx, vals[f]), "dx at frame " + std::to_string(f));
        CHECK_MSG(same_bits(u.entries[f].pad.mouse_dy, -vals[f]), "dy at frame " + std::to_string(f));
        CHECK_MSG(same_bits(u.at(f).mouse_dx, vals[f]), "at dx frame " + std::to_string(f));
    }
}

NXTEST(tolerates_crlf_blank_and_comments) {
    const std::string text = "# nxin 1\r\n\r\n# a comment\r\n3 0x2 1 -1 2 -2 0 0\r\n   \r\n# another\r\nend 9\r\n";
    InputTrace t;
    std::string err;
    bool ok = t.from_text(text, &err);
    CHECK_MSG(ok, err);
    REQUIRE(ok);
    REQUIRE(t.entries.size() == 1);
    CHECK_EQ(t.entries[0].frame, uint64_t(3));
    CHECK_EQ(t.entries[0].pad.buttons, uint64_t(2));
    CHECK(t.entries[0].pad.lx == 1);
    CHECK(t.entries[0].pad.ly == -1);
    CHECK(t.entries[0].pad.rx == 2);
    CHECK(t.entries[0].pad.ry == -2);
    CHECK_EQ(t.end_frame, uint64_t(9));
}

NXTEST(from_text_replaces_contents) {
    InputTrace t;
    record_all(t, 100);
    std::string err;
    bool ok = t.from_text("# nxin 1\n", &err);
    CHECK_MSG(ok, err);
    CHECK(t.entries.empty());
    CHECK_EQ(t.end_frame, uint64_t(0));
}

NXTEST(from_text_error_line_numbers) {
    struct Case {
        const char* text;
        int line;
    };
    const Case cases[] = {
        {"1 0x0 0 0 0 0 0 0\n", 1},                           // no header
        {"", 1},                                              // empty
        {"# nxin 2\n", 1},                                    // wrong version
        {"# foo\n# nxin 1\n", 1},                             // header not first non-blank line
        {"\n\n# nxin 2\n", 3},                                // wrong header after blank lines
        {"# nxin 1\n0 0x0 0 0 0 0 0\n", 2},                   // 7 fields
        {"# nxin 1\n# c\n0 0x0 0 0 0 0 0 0 0\n", 3},          // 9 fields
        {"# nxin 1\n0 0x0 0 0 0 0 0 zz\n", 2},                // non-numeric mouse
        {"# nxin 1\nx 0x0 0 0 0 0 0 0\n", 2},                 // non-numeric frame
        {"# nxin 1\n-1 0x0 0 0 0 0 0 0\n", 2},                // negative frame
        {"# nxin 1\n0 0 0 0 0 0 0 0\n", 2},                   // buttons without 0x
        {"# nxin 1\n0 0x1ffffffffffffffff 0 0 0 0 0 0\n", 2}, // buttons out of 64-bit range
        {"# nxin 1\n0 0x0 32768 0 0 0 0 0\n", 2},             // stick above range
        {"# nxin 1\n0 0x0 0 -32768 0 0 0 0\n", 2},            // stick below range
        {"# nxin 1\n5 0x0 0 0 0 0 0 0\n3 0x1 0 0 0 0 0 0\n", 3},  // frame decreases
        {"# nxin 1\n5 0x0 0 0 0 0 0 0\n5 0x1 0 0 0 0 0 0\n", 3},  // frame repeats
        {"# nxin 1\n0 0x0 0 0 0 0 0 0\nend 5\n1 0x0 0 0 0 0 0 0\n", 3},  // end not last
        {"# nxin 1\n0 0x0 0 0 0 0 0 0\n7 0x0 0 0 0 0 0 0\nend 6\n", 4},  // end below last frame
        {"# nxin 1\nend x\n", 2},                             // non-numeric end
        {"# nxin 1\nend 1 2\n", 2},                           // end with two values
        {"# nxin 1\nend 3\nend 4\n", 3},                      // duplicate end
    };
    for (const Case& c : cases) {
        InputTrace t;
        std::string err;
        bool ok = t.from_text(c.text, &err);
        std::string prefix = "line " + std::to_string(c.line) + ": ";
        CHECK_MSG(!ok, std::string("accepted: ") + c.text);
        CHECK_MSG(err.compare(0, prefix.size(), prefix) == 0, "expected '" + prefix + "...', got '" + err + "'");
    }
}

NXTEST(skipped_frames_record_as_described) {
    PadFrame held;
    held.buttons = kA;
    held.lx = 5;
    PadFrame imp = held;
    imp.mouse_dx = 2;
    imp.mouse_dy = 2;

    // Frames 0, 1, 5, 6 recorded: 1 is unchanged (no entry), 5 has an impulse, 6 zeroes it.
    InputTrace t;
    t.record(0, held);
    t.record(1, held);
    t.record(5, imp);
    t.record(6, held);
    REQUIRE(t.entries.size() == 3);
    CHECK_EQ(t.entries[0].frame, uint64_t(0));
    CHECK_EQ(t.entries[1].frame, uint64_t(5));
    CHECK_EQ(t.entries[2].frame, uint64_t(6));
    CHECK_EQ(t.end_frame, uint64_t(7));
    CHECK(t.at(3) == held);
    CHECK(t.at(4) == held);
    CHECK(t.at(5) == imp);
    CHECK(t.at(6) == held);

    // Impulse at 5, frames 6..8 skipped: the zeroing entry lands on the next recorded frame, 9.
    InputTrace u;
    u.record(0, held);
    u.record(5, imp);
    u.record(9, held);
    REQUIRE(u.entries.size() == 3);
    CHECK_EQ(u.entries[2].frame, uint64_t(9));
    CHECK(u.at(6) == held);
    CHECK(u.at(8) == held);
    CHECK(u.at(9) == held);
    CHECK(u.at(5) == imp);
}

NXTEST(at_before_first_entry_is_zero) {
    PadFrame held;
    held.buttons = kA;
    held.rx = 100;
    InputTrace t;
    t.record(10, held);
    REQUIRE(t.entries.size() == 1);
    for (uint64_t f = 0; f < 10; f++) CHECK_MSG(t.at(f) == PadFrame{}, "frame " + std::to_string(f));
    CHECK(t.at(10) == held);
}

NXTEST(large_button_masks_survive) {
    PadFrame p;
    p.buttons = (1ull << 27) | (1ull << 63);
    InputTrace t;
    t.record(0, p);
    std::string text = t.to_text();
    CHECK_MSG(text.find("0x8000000008000000") != std::string::npos, text);

    InputTrace u;
    std::string err;
    bool ok = u.from_text(text, &err);
    CHECK_MSG(ok, err);
    REQUIRE(ok);
    CHECK_EQ(u.at(0).buttons, p.buttons);
    CHECK_EQ(u.entries[0].pad.buttons, uint64_t((1ull << 27) | (1ull << 63)));
}

NXTEST(record_ignores_out_of_order_frames) {
    PadFrame held;
    held.buttons = kA;
    PadFrame imp = held;
    imp.mouse_dx = 4;
    InputTrace t;
    t.record(5, held);
    t.record(3, imp);
    CHECK_EQ(t.entries.size(), size_t(1));
    CHECK_EQ(t.end_frame, uint64_t(6));
}
