// SPDX-License-Identifier: GPL-3.0-or-later
// The generated pass-through tessellation control shader (src/gfx/nvn/nvn_tess_gen.h) must pass spirv-val (vulkan1.3) for the
// interface shapes the replay feeds it: the translator's vec4 varyings, per-patch inputs, evaluation inputs the vertex shader does not
// write, type and component-count conversions, packed components, every patch size, gl_ClipDistance / gl_CullDistance copies. Skips when spirv-val is absent
// (NXRT_SPIRV_VAL names it; default the MSYS2 clang64 one).
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>

#include "nvn_tess_gen.h"
#include "test_framework.h"
NXTEST_GROUP(gfx_nvn_tess_gen)

using namespace nxrt::nvn::be;
namespace fs = std::filesystem;

namespace {

std::string spirv_val() {
    if (const char* e = std::getenv("NXRT_SPIRV_VAL"); e && *e) return e;
    return "spirv-val";   // from PATH (Vulkan SDK)
}

int run_val(const std::vector<uint32_t>& w, const std::string& tag, std::string& log) {
    const std::string spv = tag + ".spv", txt = tag + ".log";
    {
        std::ofstream f(spv, std::ios::binary);
        f.write(reinterpret_cast<const char*>(w.data()), std::streamsize(w.size() * 4));
        if (!f) return -1;
    }
    const std::string cmd = "\"\"" + spirv_val() + "\" --target-env vulkan1.3 " + spv + " > " + txt + " 2>&1\"";
    const int rc = std::system(cmd.c_str());
    std::ifstream f(txt);
    log.assign(std::istreambuf_iterator<char>(f), std::istreambuf_iterator<char>());
    if (log.size() > 600) log.resize(600);
    std::error_code ec;
    if (rc == 0) fs::remove(spv, ec);   // a failing module stays for spirv-dis
    fs::remove(txt, ec);
    return rc;
}

tessgen::Io io(uint32_t loc, uint32_t comps = 4, tessgen::Kind k = tessgen::Kind::Float, uint32_t comp = 0) {
    tessgen::Io r;
    r.location = loc;
    r.components = comps;
    r.kind = k;
    r.component = comp;
    return r;
}

}  // namespace

NXTEST(validates) {
    std::error_code ec;
    if (!fs::exists(spirv_val(), ec)) { nxtest::skip("spirv-val not found (NXRT_SPIRV_VAL)"); return; }
    using tessgen::Kind;
    struct Case {
        const char* tag;
        std::vector<tessgen::Io> vs, tes_pv, tes_patch;
        uint32_t vertices;
        bool clip = false, cull = false;
    };
    // a corpus evaluation shader (0029ff6444fa1caf): per-vertex vec4 0..6, per-patch vec4 7 and 8
    std::vector<tessgen::Io> corpus_pv, corpus_patch = {io(7), io(8)}, corpus_vs;
    for (uint32_t l = 0; l <= 6; ++l) corpus_pv.push_back(io(l));
    for (uint32_t l : {0u, 2u, 3u, 4u, 5u, 6u, 7u, 8u, 10u, 11u}) corpus_vs.push_back(io(l));   // a vertex shader without location 1
    const Case cases[] = {
        {"tess_gen_empty", {}, {}, {}, 3},
        {"tess_gen_corpus3", corpus_vs, corpus_pv, corpus_patch, 3},
        {"tess_gen_corpus4", corpus_vs, corpus_pv, corpus_patch, 4},
        {"tess_gen_corpus1", corpus_vs, corpus_pv, corpus_patch, 1},
        {"tess_gen_corpus32", corpus_vs, corpus_pv, corpus_patch, 32},
        {"tess_gen_convert",
         {io(0, 4, Kind::Float), io(1, 2, Kind::Float), io(2, 4, Kind::Sint), io(3, 1, Kind::Uint), io(4, 2, Kind::Float, 2)},
         {io(0, 4, Kind::Sint), io(1, 4, Kind::Float), io(2, 3, Kind::Float), io(3, 4, Kind::Uint), io(4, 2, Kind::Float, 2), io(5, 1, Kind::Uint)},
         {io(9, 1, Kind::Sint), io(10, 3)},
         16},
        {"tess_gen_clip", corpus_vs, corpus_pv, corpus_patch, 3, true, false},
        {"tess_gen_cull", corpus_vs, corpus_pv, corpus_patch, 4, false, true},
        {"tess_gen_clip_cull", {}, {}, {}, 32, true, true},
    };
    for (const Case& c : cases) {
        const std::vector<uint32_t> w = tessgen::passthrough_tcs(tessgen::match(c.vs, c.tes_pv), c.tes_patch, c.vertices, 7, 1, c.clip, c.cull);
        std::string log;
        const int rc = run_val(w, c.tag, log);
        CHECK_MSG(rc == 0, std::string(c.tag) + ": " + log);
    }
}

NXTEST(matches_by_location_and_component) {
    using tessgen::Kind;
    const auto m = tessgen::match({io(0), io(1, 2, Kind::Float, 2)}, {io(0), io(1, 2, Kind::Float, 0), io(1, 2, Kind::Float, 2), io(5)});
    CHECK_EQ(m.size(), size_t(4));
    CHECK(m[0].has_src);
    CHECK(!m[1].has_src);
    CHECK(m[2].has_src);
    CHECK(!m[3].has_src);
}

NXTEST(copies_clip_and_cull_only_when_asked) {
    // OpAccessChain (opcode 65) with gl_PerVertex member 2 / 3 as the last index: counted per module
    auto chains_to_member = [](const std::vector<uint32_t>& w, uint32_t member) {
        // the constant ids of int `member`: OpConstant (43) %int <id> member, the int type being OpTypeInt 32 1
        uint32_t t_int = 0;
        std::vector<uint32_t> ids;
        for (size_t i = 5; i < w.size(); i += w[i] >> 16) {
            const uint32_t op = w[i] & 0xffff, n = w[i] >> 16;
            if (op == 21 && n == 4 && w[i + 3] == 1) t_int = w[i + 1];
            if (op == 43 && n == 4 && w[i + 1] == t_int && w[i + 3] == member) ids.push_back(w[i + 2]);
        }
        int count = 0;
        for (size_t i = 5; i < w.size(); i += w[i] >> 16) {
            const uint32_t op = w[i] & 0xffff, n = w[i] >> 16;
            if (op == 65 && n == 6)   // type, result, base, invocation, member
                for (uint32_t id : ids) count += w[i + 5] == id;
        }
        return count;
    };
    const auto none = tessgen::passthrough_tcs({}, {}, 3);
    const auto both = tessgen::passthrough_tcs({}, {}, 3, 7, 1, true, true);
    CHECK_EQ(chains_to_member(none, 2), 0);
    CHECK_EQ(chains_to_member(none, 3), 0);
    CHECK_EQ(chains_to_member(both, 2), 2);   // gl_in and gl_out
    CHECK_EQ(chains_to_member(both, 3), 2);
}
