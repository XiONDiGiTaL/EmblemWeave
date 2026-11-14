// SPDX-License-Identifier: GPL-3.0-or-later
// nxrt::extras::spirv_hdr_tonemap (hdr_tonemap_patch.h) against the game's real tone-map fragment shader from the local shader cache.
// The shader is game-derived data: it is read at run time from the absolute cache path and never copied into the repository. When the cache
// is absent the cases skip.
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <string>
#include <system_error>
#include <vector>

#include "test_framework.h"
#include "project_root.h"

#include "extras/hdr_tonemap_patch.h"

NXTEST_GROUP(hdr_tonemap_patch)

namespace {

namespace fs = std::filesystem;
namespace hdr = nxrt::extras;

const fs::path kSpvDir = nxtest::project_path("work/shader_cache/spirv");
const fs::path kTonemapPath = kSpvDir / "e728113b14f3564a.spv";
const char* kSpirvVal = "C:\\VulkanSDK\\1.4.363.0\\Bin\\spirv-val.exe";

constexpr uint32_t kOpExtInst = 12, kOpFMul = 133, kOpStore = 62, kOpImageSampleImplicitLod = 87;
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

// ---- case 4: negative inputs return false and leave out empty -----------------------------------------------------------------------------

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
