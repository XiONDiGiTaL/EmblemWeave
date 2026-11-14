// SPDX-License-Identifier: GPL-3.0-or-later
// NVNprogram entry points through the typed thunks: Initialize/Finalize binding, the debug label, SetShaders against the
// offline shader cache (blocks of work/shader_cache/corpus_code), and the accepted-and-ignored entries.
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>

#include "nvn_core.h"
#include "nvn_impl.h"
#include "nvn_shaders.h"
#include "nxrt/nvn.h"
#include "test_framework.h"
#include "project_root.h"
NXTEST_GROUP(gfx_nvn_program)

using namespace nxrt;
using namespace nxrt::nvn;
using namespace nxrt::nvn::be;   // ShaderStage, shader_cache(), shader_hash() (nvn_shaders.h)
namespace fs = std::filesystem;

namespace {

const char* kNames[] = {
    "nvnProgramInitialize",   "nvnProgramFinalize",          "nvnProgramSetDebugLabel",
    "nvnProgramSetShaders",   "nvnProgramSetSubroutineLinkage", "nvnProgramSetSampleShading",
};

// nvnProgramSetSampleShading has no typedef in the NVN header; this is the assumed signature (see api_program.cpp).
using PFNSAMPLESHADINGASSUMED = void (*)(NVNprogram*, int);

const fs::path kCorpus = nxtest::project_path("work/shader_cache/corpus_code");
const fs::path kSpirv = nxtest::project_path("work/shader_cache/spirv");

void setup() {
    install();
    abi::reset_mismatches();
    register_api_program();
    CHECK_EQ(abi::mismatches(), 0u);
}

bool read_bytes(const fs::path& p, std::vector<uint8_t>& out) {
    std::ifstream f(p, std::ios::binary);
    if (!f) return false;
    out.assign(std::istreambuf_iterator<char>(f), std::istreambuf_iterator<char>());
    return !out.empty();
}

std::vector<std::string> split_tabs(const std::string& line) {
    std::vector<std::string> out;
    std::string cur;
    for (char ch : line) {
        if (ch == '\t') {
            out.push_back(cur);
            cur.clear();
        } else {
            cur += ch;
        }
    }
    out.push_back(cur);
    return out;
}

// One block of the corpus: its code and control files, and the hash the manifest records for the code.
struct Block {
    std::vector<uint8_t> code, ctrl;
    std::string hash;
};

// The first manifest row of the given stage ("vertex", "fragment"...). Columns: 1 stage, 10 code_sha1_16, 11 code_file,
// 12 ctrl_file.
bool load_block(const char* stage, Block& out) {
    std::ifstream m(kCorpus / "manifest.tsv");
    if (!m) return false;
    std::string line;
    std::getline(m, line);   // header
    while (std::getline(m, line)) {
        const std::vector<std::string> f = split_tabs(line);
        if (f.size() < 13 || f[1] != stage) continue;
        out.hash = f[10];
        return read_bytes(kCorpus / f[11], out.code) && read_bytes(kCorpus / f[12], out.ctrl);
    }
    return false;
}

// The corpus and the cache's results.tsv are present and the process cache opens on them.
bool cache_ready() {
    return fs::exists(kCorpus / "manifest.tsv") && fs::exists(kSpirv / "results.tsv") && shader_cache().is_open();
}

uint64_t addr(const void* p) { return static_cast<uint64_t>(reinterpret_cast<uintptr_t>(p)); }

NVNshaderData shader_data(const std::vector<uint8_t>& code, const std::vector<uint8_t>& ctrl) {
    NVNshaderData d;
    d.data = addr(code.data());
    d.control = ctrl.data();
    return d;
}

}  // namespace

NXTEST(entry_points_are_registered) {
    setup();
    for (const char* name : kNames) CHECK_MSG(is_implemented(name), name);
}

NXTEST(initialize_binds_and_finalize_releases) {
    setup();
    auto init = abi::proc<PFNNVNPROGRAMINITIALIZEPROC>("nvnProgramInitialize");
    auto fin = abi::proc<PFNNVNPROGRAMFINALIZEPROC>("nvnProgramFinalize");
    NVNprogram prog;
    std::memset(&prog, 0, sizeof prog);
    CHECK_EQ(init(nullptr, nullptr), 0);
    CHECK_EQ(init(&prog, nullptr), 1);
    be::Program* hp = be::host_of<be::Program>(&prog);
    CHECK(hp != nullptr);
    CHECK(hp && !hp->linked);
    CHECK(hp && hp->serial == 0u);
    fin(&prog);
    CHECK(be::host_of<be::Program>(&prog) == nullptr);
    fin(nullptr);   // no crash
}

NXTEST(debug_label_is_stored_and_null_clears_it) {
    setup();
    auto init = abi::proc<PFNNVNPROGRAMINITIALIZEPROC>("nvnProgramInitialize");
    auto label = abi::proc<PFNNVNPROGRAMSETDEBUGLABELPROC>("nvnProgramSetDebugLabel");
    auto fin = abi::proc<PFNNVNPROGRAMFINALIZEPROC>("nvnProgramFinalize");
    NVNprogram prog;
    std::memset(&prog, 0, sizeof prog);
    init(&prog, nullptr);
    label(&prog, "terrain fragment");
    CHECK(be::host_of<be::Program>(&prog)->label == "terrain fragment");
    label(&prog, nullptr);
    CHECK(be::host_of<be::Program>(&prog)->label.empty());
    fin(&prog);
}

NXTEST(not_ours_objects_are_tolerated) {
    setup();
    auto fin = abi::proc<PFNNVNPROGRAMFINALIZEPROC>("nvnProgramFinalize");
    auto label = abi::proc<PFNNVNPROGRAMSETDEBUGLABELPROC>("nvnProgramSetDebugLabel");
    auto set_shaders = abi::proc<PFNNVNPROGRAMSETSHADERSPROC>("nvnProgramSetShaders");
    auto set_sub = abi::proc<PFNNVNPROGRAMSETSUBROUTINELINKAGEPROC>("nvnProgramSetSubroutineLinkage");
    auto set_shade = abi::proc<PFNSAMPLESHADINGASSUMED>("nvnProgramSetSampleShading");
    NVNprogram prog;
    std::memset(&prog, 0xAB, sizeof prog);   // not a program of ours: no guest header magic
    fin(&prog);
    label(&prog, "x");
    NVNshaderData d{};
    CHECK_EQ(set_shaders(&prog, 1, &d), 0);
    CHECK_EQ(set_sub(&prog, 0, nullptr), 0);
    set_shade(&prog, 1);
    CHECK(be::host_of<be::Program>(&prog) == nullptr);
    CHECK_EQ(static_cast<unsigned>(static_cast<unsigned char>(prog.reserved[0])), 0xABu);   // untouched
    CHECK_EQ(set_shaders(nullptr, 0, nullptr), 0);
}

NXTEST(subroutine_linkage_and_sample_shading_are_accepted) {
    setup();
    auto init = abi::proc<PFNNVNPROGRAMINITIALIZEPROC>("nvnProgramInitialize");
    auto fin = abi::proc<PFNNVNPROGRAMFINALIZEPROC>("nvnProgramFinalize");
    auto set_sub = abi::proc<PFNNVNPROGRAMSETSUBROUTINELINKAGEPROC>("nvnProgramSetSubroutineLinkage");
    auto set_shade = abi::proc<PFNSAMPLESHADINGASSUMED>("nvnProgramSetSampleShading");
    NVNprogram prog;
    std::memset(&prog, 0, sizeof prog);
    init(&prog, nullptr);
    CHECK_EQ(set_sub(&prog, 1, nullptr), 1);   // accepted, ignored
    set_shade(&prog, 1);                        // accepted, ignored
    set_shade(&prog, 0);
    CHECK(be::host_of<be::Program>(&prog) != nullptr);
    fin(&prog);
}

NXTEST(set_shaders_vertex_and_fragment_from_cache) {
    if (!cache_ready()) {
        nxtest::skip("shader cache or corpus missing");
        return;
    }
    setup();
    Block v, f;
    if (!load_block("vertex", v) || !load_block("fragment", f)) {
        nxtest::skip("corpus has no vertex/fragment block");
        return;
    }
    auto init = abi::proc<PFNNVNPROGRAMINITIALIZEPROC>("nvnProgramInitialize");
    auto fin = abi::proc<PFNNVNPROGRAMFINALIZEPROC>("nvnProgramFinalize");
    auto set_shaders = abi::proc<PFNNVNPROGRAMSETSHADERSPROC>("nvnProgramSetShaders");
    NVNprogram prog;
    std::memset(&prog, 0, sizeof prog);
    init(&prog, nullptr);
    const NVNshaderData d[2] = {shader_data(v.code, v.ctrl), shader_data(f.code, f.ctrl)};
    CHECK_EQ(set_shaders(&prog, 2, d), 1);
    be::Program* hp = be::host_of<be::Program>(&prog);
    CHECK(hp != nullptr);
    if (hp) {
        CHECK(hp->linked);
        CHECK(hp->serial == 1u);
        CHECK(hp->stage[0] != nullptr);
        CHECK(hp->stage[1] != nullptr);
        CHECK(hp->stage[0] && hp->stage[0]->stage == ShaderStage::Vertex);
        CHECK(hp->stage[1] && hp->stage[1]->stage == ShaderStage::Fragment);
        CHECK(hp->stage[2] == nullptr);
        CHECK(hp->stage[5] == nullptr);
        CHECK(hp->code_hash[0] == std::stoull(v.hash, nullptr, 16));
        CHECK(hp->code_hash[1] == std::stoull(f.hash, nullptr, 16));
    }
    fin(&prog);
}

NXTEST(unknown_block_leaves_program_unlinked_and_reports_success) {
    if (!cache_ready()) {
        nxtest::skip("shader cache or corpus missing");
        return;
    }
    setup();
    Block v;
    if (!load_block("vertex", v)) {
        nxtest::skip("corpus has no vertex block");
        return;
    }
    auto init = abi::proc<PFNNVNPROGRAMINITIALIZEPROC>("nvnProgramInitialize");
    auto fin = abi::proc<PFNNVNPROGRAMFINALIZEPROC>("nvnProgramFinalize");
    auto set_shaders = abi::proc<PFNNVNPROGRAMSETSHADERSPROC>("nvnProgramSetShaders");
    NVNprogram prog;
    std::memset(&prog, 0, sizeof prog);
    init(&prog, nullptr);
    // All-zero code with a valid control section copied from a real block: its hash has no translated file.
    std::vector<uint8_t> zeros(v.code.size(), 0);
    CHECK(shader_hash(zeros.data(), static_cast<uint32_t>(zeros.size())) != v.hash);
    const NVNshaderData d = shader_data(zeros, v.ctrl);
    // [nvn] accept_unlinked (default on): a block the cache lacks still reports success, so the game does not rebuild the shader on every bind (S18); the program stays unlinked
    CHECK_EQ(set_shaders(&prog, 1, &d), 1);
    be::Program* hp = be::host_of<be::Program>(&prog);
    CHECK(hp != nullptr);
    if (hp) {
        CHECK(!hp->linked);
        CHECK(hp->stage[0] == nullptr);
        CHECK(hp->code_hash[0] == 0u);
    }
    fin(&prog);
}

NXTEST(found_stages_are_kept_when_another_is_missing) {
    if (!cache_ready()) {
        nxtest::skip("shader cache or corpus missing");
        return;
    }
    setup();
    Block v;
    if (!load_block("vertex", v)) {
        nxtest::skip("corpus has no vertex block");
        return;
    }
    auto init = abi::proc<PFNNVNPROGRAMINITIALIZEPROC>("nvnProgramInitialize");
    auto fin = abi::proc<PFNNVNPROGRAMFINALIZEPROC>("nvnProgramFinalize");
    auto set_shaders = abi::proc<PFNNVNPROGRAMSETSHADERSPROC>("nvnProgramSetShaders");
    NVNprogram prog;
    std::memset(&prog, 0, sizeof prog);
    init(&prog, nullptr);
    std::vector<uint8_t> zeros(v.code.size(), 0);
    const NVNshaderData d[2] = {shader_data(v.code, v.ctrl), shader_data(zeros, v.ctrl)};
    CHECK_EQ(set_shaders(&prog, 2, d), 1);      // accept_unlinked, see above
    be::Program* hp = be::host_of<be::Program>(&prog);
    CHECK(hp != nullptr);
    if (hp) {
        CHECK(!hp->linked);
        CHECK(hp->stage[0] != nullptr && hp->stage[0]->stage == ShaderStage::Vertex);
        CHECK(hp->stage[1] == nullptr);
    }
    // A later call clears the earlier stages first: count 0 leaves none.
    CHECK_EQ(set_shaders(&prog, 0, nullptr), 1);
    if (hp) {
        CHECK(hp->stage[0] == nullptr);
        CHECK(hp->code_hash[0] == 0u);
        CHECK(hp->linked);
    }
    fin(&prog);
}

NXTEST(copies_of_the_guest_object_share_the_host_object) {
    setup();
    auto init = abi::proc<PFNNVNPROGRAMINITIALIZEPROC>("nvnProgramInitialize");
    NVNprogram a, b;
    std::memset(&a, 0, sizeof a);
    init(&a, nullptr);
    std::memcpy(&b, &a, sizeof a);   // the game copies objects around; the header still names the same host object
    CHECK(be::host_of<be::Program>(&b) == be::host_of<be::Program>(&a));
    CHECK(be::host_of<be::Program>(&a) != nullptr);
}

// NVN2's program builder (api_program.cpp): it keeps the device and the shader array with its count; nvnProgramInitializeFromBuilder is
// nvnProgramInitialize + SetSampleShading + SetShaders and answers what SetShaders answers (no stage: initialised, 1).
NXTEST(program_builder_initializes_the_program) {
    setup();
    struct Builder {
        uint8_t bytes[0x80];   // the room nn::gfx gives it (Fortune's Weave main 0x604CB8)
    };
    abi::Caller<void(Builder*)> set_defaults(proc_address("nvnProgramBuilderSetDefaults"));
    abi::Caller<void(Builder*, NVNdevice*)> set_device(proc_address("nvnProgramBuilderSetDevice"));
    abi::Caller<NVNdevice*(const Builder*)> get_device(proc_address("nvnProgramBuilderGetDevice"));
    abi::Caller<void(Builder*, int, const NVNshaderData*)> set_shaders(proc_address("nvnProgramBuilderSetShaders"));
    abi::Caller<int(const Builder*)> shader_count(proc_address("nvnProgramBuilderGetShaderCount"));
    abi::Caller<const NVNshaderData*(const Builder*)> get_shaders(proc_address("nvnProgramBuilderGetShaders"));
    abi::Caller<NVNboolean(NVNprogram*, const Builder*)> init(proc_address("nvnProgramInitializeFromBuilder"));
    auto fin = abi::proc<PFNNVNPROGRAMFINALIZEPROC>("nvnProgramFinalize");
    Builder b;
    std::memset(&b, 0xCD, sizeof b);
    set_defaults(&b);
    CHECK(get_device(&b) == nullptr);
    CHECK_EQ(shader_count(&b), 0);
    NVNdevice dev;
    set_device(&b, &dev);
    CHECK(get_device(&b) == &dev);
    NVNshaderData d[2]{};
    set_shaders(&b, 2, d);
    CHECK_EQ(shader_count(&b), 2);
    CHECK(get_shaders(&b) == d);
    set_shaders(&b, 0, nullptr);
    NVNprogram prog;
    std::memset(&prog, 0, sizeof prog);
    CHECK_EQ(init(&prog, &b), 1);
    CHECK(be::host_of<be::Program>(&prog) != nullptr);
    fin(&prog);
    CHECK_EQ(init(nullptr, &b), 0);
}
