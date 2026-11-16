// SPDX-License-Identifier: GPL-3.0-or-later
// nxrt::extras::spirv_hdr_tonemap (hdr_tonemap_patch.h) against the game's real tone-map fragment shader from the local shader cache.
// The shader is game-derived data: it is read at run time from the absolute cache path and never copied into the repository. When the cache
// is absent the cases skip.
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <set>
#include <string>
#include <system_error>
#include <vector>

#include "test_framework.h"
#include "project_root.h"

#include "extras/hdr_nvn.h"
#include "extras/hdr_tonemap_patch.h"

NXTEST_GROUP(hdr_tonemap_patch)

namespace {

namespace fs = std::filesystem;
namespace hdr = nxrt::extras;

const fs::path kSpvDir = nxtest::project_path("work/shader_cache/spirv");
const fs::path kTonemapPath = kSpvDir / "e728113b14f3564a.spv";
const char* kSpirvVal = "C:\\VulkanSDK\\1.4.363.0\\Bin\\spirv-val.exe";

constexpr uint32_t kOpExtInst = 12, kOpFMul = 133, kOpStore = 62, kOpImageSampleImplicitLod = 87;
constexpr uint32_t kOpExtInstImport = 11, kOpEntryPoint = 15, kOpTypeInt = 21, kOpTypeFloat = 22, kOpTypePointer = 32;
constexpr uint32_t kOpTypeFunction = 33, kOpConstant = 43, kOpFunction = 54, kOpFunctionEnd = 56, kOpVariable = 59;
constexpr uint32_t kOpLoad = 61, kOpAccessChain = 65, kGlslExp2 = 29;
constexpr uint32_t kGlslFMax = 40;

// Reads a .spv file as little-endian 32-bit words. Returns false if it is missing, unreadable or not a whole number of words.
bool load_words(const fs::path& p, std::vector<uint32_t>& w) {
    std::error_code ec;
    if (!fs::is_regular_file(p, ec)) return false;
    std::ifstream f(p, std::ios::binary | std::ios::ate);
    if (!f) return false;
    const std::streamsize n = f.tellg();
    if (n <= 0 || n % 4 != 0) return false;
    std::vector<unsigned char> b(static_cast<size_t>(n));
    f.seekg(0);
    f.read(reinterpret_cast<char*>(b.data()), n);
    if (!f) return false;
    w.resize(b.size() / 4);
    for (size_t i = 0; i < w.size(); ++i) {
        w[i] = uint32_t(b[4 * i]) | (uint32_t(b[4 * i + 1]) << 8) | (uint32_t(b[4 * i + 2]) << 16) | (uint32_t(b[4 * i + 3]) << 24);
    }
    return true;
}

// Writes words as little-endian bytes.
bool write_words(const fs::path& p, const std::vector<uint32_t>& w) {
    std::ofstream f(p, std::ios::binary | std::ios::trunc);
    if (!f) return false;
    for (uint32_t v : w) {
        const unsigned char b[4] = {static_cast<unsigned char>(v), static_cast<unsigned char>(v >> 8), static_cast<unsigned char>(v >> 16),
                                    static_cast<unsigned char>(v >> 24)};
        f.write(reinterpret_cast<const char*>(b), 4);
    }
    return static_cast<bool>(f);
}

// Instruction census over the body (from word 5). ok is false if the walk hits a zero word count or overruns, or does not end exactly on the
// last word.
struct Census {
    bool ok = false;
    uint32_t fmax = 0, fmul = 0, store = 0, sample = 0;
};

Census census(const std::vector<uint32_t>& w) {
    Census c;
    size_t i = 5;
    while (i < w.size()) {
        const uint32_t len = w[i] >> 16, op = w[i] & 0xffffu;
        if (len == 0 || i + len > w.size()) return c;
        if (op == kOpExtInst && len >= 5 && w[i + 4] == kGlslFMax) ++c.fmax;   // word 4 = extended instruction number
        if (op == kOpFMul) ++c.fmul;
        if (op == kOpStore) ++c.store;
        if (op == kOpImageSampleImplicitLod) ++c.sample;
        i += len;
    }
    c.ok = (i == w.size());
    return c;
}

bool shader_present() {
    std::error_code ec;
    return fs::is_regular_file(kTonemapPath, ec);
}

uint32_t float_bits(float f) {
    uint32_t bits;
    std::memcpy(&bits, &f, sizeof bits);
    return bits;
}

void emit(std::vector<uint32_t>& words, uint32_t op, std::initializer_list<uint32_t> body) {
    words.push_back((static_cast<uint32_t>(body.size()) + 1) << 16 | op);
    words.insert(words.end(), body.begin(), body.end());
}

// A small structural GT shader: one P load at (cb, 0, 2, 0), one float load at another offset, three exp2 uses and the
// four BT.2020 -> BT.709 constants. It is not a complete Vulkan shader; this test exercises the patch's ID rewriting only.
std::vector<uint32_t> synthetic_gt_shader(bool matching) {
    constexpr uint32_t kGlsl = 1, kFloat = 2, kInt = 3, kPointer = 4, kFunctionType = 7, kUniform = 6;
    constexpr uint32_t kP = 40, kPLoad = 41, kOther = 42, kOtherLoad = 43;
    constexpr uint32_t kZero = 30, kTwo = 31, kOne = 32;
    std::vector<uint32_t> body;
    emit(body, kOpExtInstImport, {kGlsl, 0x4c534c47u, 0x6474732eu, 0x3035342eu, 0});   // "GLSL.std.450"
    emit(body, kOpEntryPoint, {4, 20, 0x6e69616du, 0});                                 // Fragment, "main"
    emit(body, kOpTypeFloat, {kFloat, 32});
    emit(body, kOpTypeInt, {kInt, 32, 0});
    emit(body, kOpTypePointer, {kPointer, 2, kFloat});                                   // UniformConstant float pointer
    emit(body, kOpConstant, {kInt, kZero, 0});
    emit(body, kOpConstant, {kInt, kTwo, 2});
    emit(body, kOpConstant, {kInt, kOne, 1});
    const float matrix[4] = {1.66049027f, -0.587639093f, 1.11872983f, -0.124549903f};
    for (uint32_t i = 0; i < (matching ? 4u : 3u); ++i) emit(body, kOpConstant, {kFloat, 60 + i, float_bits(matrix[i])});
    emit(body, kOpTypeFunction, {kFunctionType, kFloat});
    emit(body, kOpVariable, {kPointer, kUniform, 2});
    emit(body, kOpFunction, {kFloat, 20, 0, kFunctionType});
    emit(body, kOpAccessChain, {kPointer, kP, kUniform, kZero, kTwo, kZero});
    emit(body, kOpLoad, {kFloat, kPLoad, kP});
    emit(body, kOpAccessChain, {kPointer, kOther, kUniform, kZero, kOne, kZero});
    emit(body, kOpLoad, {kFloat, kOtherLoad, kOther});
    for (uint32_t id : {50u, 51u, 52u}) emit(body, kOpExtInst, {kFloat, id, kGlsl, kGlslExp2, kPLoad});
    emit(body, kOpFunctionEnd, {});
    std::vector<uint32_t> words = {0x07230203u, 0x00010000u, 0, 100, 0};
    words.insert(words.end(), body.begin(), body.end());
    return words;
}

struct Instruction {
    size_t pos;
    uint32_t op, len;
};

std::vector<Instruction> instructions(const std::vector<uint32_t>& words) {
    std::vector<Instruction> out;
    for (size_t at = 5; at < words.size();) {
        const uint32_t len = words[at] >> 16;
        if (len == 0 || at + len > words.size()) return {};
        out.push_back({at, words[at] & 0xffffu, len});
        at += len;
    }
    return out;
}

uint32_t result_id(const std::vector<uint32_t>& words, const Instruction& in) {
    switch (in.op) {
        case kOpExtInstImport: case kOpTypeInt: case kOpTypeFloat: case kOpTypePointer: case kOpTypeFunction: return words[in.pos + 1];
        case kOpConstant: case kOpVariable: case kOpFunction: case kOpAccessChain: case kOpLoad: case kOpExtInst: return words[in.pos + 2];
        default: return 0;
    }
}

}  // namespace

// ---- case 1: the real shader is patched into a well-formed module ------------------------------------------------------------------------

NXTEST(real_shader_patch_shape) {
    if (!shader_present()) {
        nxtest::skip("shader cache not present");
        return;
    }
    std::vector<uint32_t> in, out;
    CHECK(load_words(kTonemapPath, in));
    CHECK(in.size() >= 8);
    if (in.size() < 8) return;

    const bool ok = hdr::spirv_hdr_tonemap(in, out);
    CHECK(ok);
    printf("hdr_tonemap_patch: input %zu words, output %zu words (+%zd)\n", in.size(), out.size(),
           static_cast<ptrdiff_t>(out.size()) - static_cast<ptrdiff_t>(in.size()));
    CHECK(out.size() > in.size());
    const ptrdiff_t grown = static_cast<ptrdiff_t>(out.size()) - static_cast<ptrdiff_t>(in.size());
    CHECK(grown >= 20 && grown <= 60);
    if (out.size() < 5) return;

    for (size_t i = 0; i < 5; ++i) {
        if (i == 3) {
            CHECK(out[3] > in[3]);                 // id bound grew
        } else {
            CHECK_EQ(out[i], in[i]);               // magic, version, generator, schema
        }
    }

    // Running the patch again on its own output must not crash; the result is not asserted.
    std::vector<uint32_t> again;
    const bool second = hdr::spirv_hdr_tonemap(out, again);
    printf("hdr_tonemap_patch: second pass on patched output returned %s\n", second ? "true" : "false");

    CHECK(census(out).ok);
}

// ---- case 2: the instruction content of the patch -----------------------------------------------------------------------------------------

NXTEST(real_shader_patch_instruction_delta) {
    if (!shader_present()) {
        nxtest::skip("shader cache not present");
        return;
    }
    std::vector<uint32_t> in, out;
    CHECK(load_words(kTonemapPath, in));
    if (in.size() < 8) return;
    if (!hdr::spirv_hdr_tonemap(in, out)) {
        CHECK(false);
        return;
    }
    const Census ci = census(in), co = census(out);
    CHECK(ci.ok);
    CHECK(co.ok);
    CHECK_EQ(co.fmax, ci.fmax + 3);
    CHECK_EQ(co.fmul, ci.fmul + 1);
    CHECK_EQ(co.store, ci.store);
    CHECK_EQ(co.sample, ci.sample);
}

// ---- case 3: the Vulkan validator accepts the patched module ------------------------------------------------------------------------------

NXTEST(spirv_val_accepts_patched_output) {
    if (!shader_present()) {
        nxtest::skip("shader cache not present");
        return;
    }
    std::error_code ec;
    if (!fs::is_regular_file(kSpirvVal, ec)) {
        nxtest::skip("spirv-val not present");
        return;
    }
    std::vector<uint32_t> in, out;
    CHECK(load_words(kTonemapPath, in));
    if (in.size() < 8) return;
    if (!hdr::spirv_hdr_tonemap(in, out)) {
        CHECK(false);
        return;
    }
    const char* temp = std::getenv("TEMP");
    const fs::path tmp = std::string(temp ? temp : ".") + "\\nxrt_hdr_tm.spv";
    CHECK(write_words(tmp, out));
    // cmd.exe strips the outermost pair of quotes when the line starts with one, so the whole line is wrapped in an extra pair.
    const std::string cmd = std::string("\"\"") + kSpirvVal + "\" --target-env vulkan1.3 \"" + tmp.string() + "\"\"";
    const int rc = std::system(cmd.c_str());
    printf("hdr_tonemap_patch: spirv-val exit code %d\n", rc);
    CHECK_EQ(rc, 0);
    fs::remove(tmp, ec);
}

// ---- case 4: a synthetic GT shader has only its P loads wrapped, and its IDs stay in bounds ------------------------------

NXTEST(gt_synthetic_only_p_loads_are_wrapped) {
    const std::vector<uint32_t> in = synthetic_gt_shader(true);
    std::vector<uint32_t> out;
    REQUIRE(hdr::spirv_hdr_tonemap_gt(in, 4.0f, out));
    CHECK_EQ(out.size(), in.size() + 11);                       // one float constant (4 words) + one FMax (7 words)
    CHECK_EQ(out[0], in[0]);
    CHECK_EQ(out[1], in[1]);
    CHECK_EQ(out[2], in[2]);
    CHECK_EQ(out[3], 102u);                                    // two new IDs: the peak and the raw load
    CHECK_EQ(out[4], in[4]);

    const std::vector<Instruction> after = instructions(out);
    REQUIRE(!after.empty());
    size_t fmax_at = SIZE_MAX;
    int fmax_count = 0, p_loads = 0, other_loads = 0;
    uint32_t p_raw = 0, other_result = 0;
    for (const Instruction& n : after) {
        if (n.op == kOpExtInst && n.len >= 5 && out[n.pos + 3] == 1 && out[n.pos + 4] == kGlslFMax) {
            ++fmax_count;
            fmax_at = n.pos;
        }
        if (n.op == kOpLoad && n.len == 4 && out[n.pos + 1] == 2) {
            if (out[n.pos + 3] == 40) {
                ++p_loads;
                p_raw = out[n.pos + 2];
            } else if (out[n.pos + 3] == 42) {
                ++other_loads;
                other_result = out[n.pos + 2];
            }
        }
    }
    CHECK_EQ(fmax_count, 1);
    CHECK_EQ(p_loads, 1);
    CHECK_EQ(other_loads, 1);
    REQUIRE(fmax_at != SIZE_MAX);
    CHECK_EQ(p_raw, 101u);
    CHECK_EQ(out[fmax_at + 2], 41u);                           // the original result id now names the max
    CHECK_EQ(out[fmax_at + 5], p_raw);
    CHECK_EQ(out[fmax_at + 6], 100u);
    CHECK_EQ(other_result, 43u);                               // the non-P load is untouched
    CHECK(out[fmax_at + 5] != other_result);
    CHECK(out[fmax_at + 6] != other_result);

    bool peak_ok = false;
    for (const Instruction& n : after) {
        if (n.op == kOpConstant && out[n.pos + 2] == 100u) {
            peak_ok = out[n.pos + 1] == 2 && out[n.pos + 3] == float_bits(4.0f);
        }
    }
    CHECK(peak_ok);

    std::set<uint32_t> ids;
    for (const Instruction& n : after) {
        const uint32_t id = result_id(out, n);
        if (id == 0) continue;
        CHECK(id < out[3]);
        CHECK(ids.insert(id).second);
    }
}

NXTEST(gt_synthetic_nonmatching_shader_is_untouched) {
    const std::vector<uint32_t> keep = {0xdeadbeefu, 7u};
    std::vector<uint32_t> out = keep;
    CHECK(!hdr::spirv_hdr_tonemap_gt(synthetic_gt_shader(false), 4.0f, out));
    CHECK(out == keep);

    out = keep;
    CHECK(!hdr::spirv_hdr_tonemap_gt(synthetic_gt_shader(true), 1.0f, out));
    CHECK(out == keep);
}

NXTEST(gt_synthetic_shader_is_recognized_as_a_tonemap) {
    CHECK(hdr::hdr_is_tonemap(synthetic_gt_shader(true), "synthetic-gt-tonemap"));
    CHECK(!hdr::hdr_is_tonemap(synthetic_gt_shader(false), "synthetic-gt-not-tonemap"));
}

// ---- case 5: negative inputs return false and leave out empty -----------------------------------------------------------------------------

NXTEST(rejects_empty_input) {
    std::vector<uint32_t> in, out;
    CHECK(!hdr::spirv_hdr_tonemap(in, out));
    CHECK(out.empty());
}

NXTEST(rejects_wrong_magic) {
    if (!shader_present()) {
        nxtest::skip("shader cache not present");
        return;
    }
    std::vector<uint32_t> in, out;
    CHECK(load_words(kTonemapPath, in));
    if (in.size() < 8) return;
    in[0] ^= 1u;
    CHECK(!hdr::spirv_hdr_tonemap(in, out));
    CHECK(out.empty());
}

NXTEST(rejects_truncated_shader) {
    if (!shader_present()) {
        nxtest::skip("shader cache not present");
        return;
    }
    std::vector<uint32_t> in, out;
    CHECK(load_words(kTonemapPath, in));
    if (in.size() < 40) return;
    in.resize(40);
    CHECK(!hdr::spirv_hdr_tonemap(in, out));
    CHECK(out.empty());
}

NXTEST(other_cache_shaders_never_crash) {
    std::error_code ec;
    if (!fs::is_directory(kSpvDir, ec)) {
        nxtest::skip("shader cache not present");
        return;
    }
    int tried = 0, accepted = 0;
    for (const auto& ent : fs::directory_iterator(kSpvDir, ec)) {
        if (tried >= 40) break;
        if (!ent.is_regular_file(ec) || ent.path().extension() != ".spv") continue;
        if (ent.path().filename() == kTonemapPath.filename()) continue;
        std::vector<uint32_t> w, out;
        if (!load_words(ent.path(), w)) continue;
        ++tried;
        if (hdr::spirv_hdr_tonemap(w, out)) ++accepted;
    }
    printf("hdr_tonemap_patch: %d of %d other cache shaders accepted by the patch\n", accepted, tried);
}
