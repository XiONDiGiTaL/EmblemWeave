// SPDX-License-Identifier: GPL-3.0-or-later
// The translated shaders in the runtime, NVN2 parts (notes/shaderint.md): push constants (graphics constant bank 24 + slot, compute c[0]
// after its 0x10-byte header), the NVN2 compute table after the push words, and the texture and sampler pools as descriptor arrays.
// Meanings and the move of the pool arrays are checked on the reflection of test shaders (tests/gfx/shaderint_spirv.inc, written for these
// tests), push constants as recorded, and three scenes run the back end on the GPU with those shaders served from a shader cache this file
// writes (a code block per shader, its SPIR-V under the block's hash, a results.tsv record), read back by copy:
//   graphics push constants reach bank 24 at the pushed offset, keep the other bytes and persist across command buffers;
//   a compute shader with 28 push words finds its storage buffer after them and reads the pushed words and the work-group counts in c[0];
//   a fragment shader samples the texture pool by handle through the 2D and the cube array, before and after an id is registered again,
//   and gets the dummy for a texture finalized while still registered.
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <functional>
#include <sstream>
#include <string>
#include <vector>

#include "nvn_app.h"
#include "nvn_cmd.h"
#include "nvn_gpu_internal.h"
#include "nvn_impl.h"
#include "nvn_shaders.h"
#include "nxrt/config.h"
#include "shaderint_spirv.inc"
NXTEST_GROUP(gfx_nvn_shaderint)

using namespace nxrt::nvn;
using namespace nxrt::nvn::be;
using nvnapp::App;
namespace fs = std::filesystem;

#define REQUIRE_WHY(cond, why)                                                        \
    do {                                                                              \
        if (!(cond)) {                                                                \
            ::nxtest::fail(__FILE__, __LINE__, #cond, (why));                         \
            return;                                                                   \
        }                                                                             \
    } while (0)

namespace {

constexpr int kW = 16, kH = 16;

template <size_t N>
std::vector<uint32_t> spv(const uint32_t (&a)[N]) {
    return std::vector<uint32_t>(a, a + N);
}

// The module without its OpName instructions: the runtime then has only the binding numbers to go by.
std::vector<uint32_t> without_names(const std::vector<uint32_t>& w) {
    std::vector<uint32_t> out(w.begin(), w.begin() + 5);
    for (size_t i = 5; i < w.size();) {
        const uint32_t n = w[i] >> 16;
        if ((w[i] & 0xffff) != 5) out.insert(out.end(), w.begin() + static_cast<std::ptrdiff_t>(i), w.begin() + static_cast<std::ptrdiff_t>(i + n));
        i += n;
    }
    return out;
}

const DescriptorBinding* by_name(const ShaderReflection& r, const char* name) {
    for (const auto& b : r.bindings)
        if (b.name == name) return &b;
    return nullptr;
}

const DescriptorBinding* at(const ShaderReflection& r, uint32_t set, uint32_t binding) {
    for (const auto& b : r.bindings)
        if (b.set == set && b.binding == binding) return &b;
    return nullptr;
}

std::vector<std::string> tokens(const std::string& s) {
    std::vector<std::string> t;
    std::istringstream in(s);
    for (std::string x; in >> x;) t.push_back(x);
    std::sort(t.begin(), t.end());
    return t;
}

// A test shader as a game hands it in: a code block (its hash names the SPIR-V) and an NVN2 control section (version 201.20, 0xe0 bytes).
struct Block {
    std::vector<uint8_t> code, control;
    std::vector<uint32_t> spirv;
    std::string resources;   // its results.tsv record
};

void put32(std::vector<uint8_t>& v, size_t off, uint32_t x) { std::memcpy(v.data() + off, &x, 4); }
void put16(std::vector<uint8_t>& v, size_t off, uint16_t x) { std::memcpy(v.data() + off, &x, 2); }

// stage: 0 vertex, 1 fragment, 5 compute. Compute: `push_words` words of push constants, `ssbos` storage buffers and `images` images in its
// c[0] table.
Block make_block(uint8_t seed, int stage, std::vector<uint32_t> spirv, std::string resources, uint8_t push_words = 0, uint8_t ssbos = 0,
                 uint8_t images = 0) {
    Block b;
    b.code.resize(0x100);
    for (size_t i = 0; i < b.code.size(); ++i) b.code[i] = static_cast<uint8_t>(seed * 31 + i * 7);
    b.control.assign(0xe0, 0);
    put32(b.control, 0, kShaderControlMagic);
    put32(b.control, 4, 201);
    put32(b.control, 8, 20);
    put32(b.control, 0x38, 0x100);
    put32(b.control, 0x44, 0x100);
    put16(b.control, 0x56, static_cast<uint16_t>(stage));
    if (stage == 5) {
        for (int i = 0; i < 3; ++i) put32(b.control, 0x58 + 4 * i, 1);
        b.control[0xa9] = push_words;
        b.control[0xb2] = ssbos;
        b.control[0xb6] = images;
        put16(b.control, 0xba, static_cast<uint16_t>(0x10 + 4 * push_words + 0x10 * ssbos + 8 * images));
    }
    b.spirv = std::move(spirv);
    b.resources = std::move(resources);
    return b;
}

// The test shaders with the records the harness prints for them.
Block& vs_block() {
    static Block b = make_block(1, 0, spv(kSpvFullscreenVs), "");
    return b;
}
Block& push_fs_block() {
    static Block b = make_block(2, 1, spv(kSpvPushFs), "cb24=s0b111");
    return b;
}
Block& pool_fs_block() {
    static Block b = make_block(3, 1, spv(kSpvPoolFs), "cb3=s0b22 texTexture2D[c0:1073741824x65536]=s2b640 texNone[c0:1073741824x4096]=s2b641 "
                                                       "texTextureCube[c0:1073741824x65536]=s2b642");
    return b;
}
// storage buffer 0 after the 0x10-byte header and 28 push words: byte 0x80, word 0x20
Block& push_cs_block() {
    static Block b = make_block(4, 5, spv(kSpvPushCs), "cb0=s0b1 sb[c0+0x20]=s1b0", 28, 1);
    return b;
}
// a texture handle at word 36 = byte 0x90: with 28 push words and one storage buffer (0x80) that is image slot 0, not a texture slot
Block& imgtex_cs_block() {
    static Block b = make_block(5, 5, spv(kSpvImageTexCs), "sb[c0+0x20]=s1b0 texTexture2D[c0:36]=s2b0", 28, 1, 1);
    return b;
}
// a storage image at image slot 0 (word 36, as above), read and written (float16_scene_promotes_at_the_first_store)
Block& storeimg_cs_block() {
    static Block b = make_block(6, 5, spv(kSpvStoreImageCs), "sb[c0+0x20]=s1b0 imgTexture2D[c0:36]=s3b0", 28, 1, 1);
    return b;
}

// The shader cache of these scenes, written once, made [nvn] shader_cache and opened as the back end's cache.
bool cache_ready() {
    static int state = -1;
    if (state >= 0) return state == 1;
    state = 0;
    const fs::path dir = fs::temp_directory_path() / "nxrt_test_shaderint" / "spirv";
    std::error_code ec;
    fs::remove_all(dir.parent_path(), ec);
    fs::create_directories(dir, ec);
    std::string tsv = "hash\tstage\tstatus\tresources\n";
    for (Block* b : {&vs_block(), &push_fs_block(), &pool_fs_block(), &push_cs_block(), &imgtex_cs_block(), &storeimg_cs_block()}) {
        const std::string h = shader_hash(b->code.data(), static_cast<uint32_t>(b->code.size()));
        std::ofstream(dir / (h + ".spv"), std::ios::binary).write(reinterpret_cast<const char*>(b->spirv.data()), static_cast<std::streamsize>(b->spirv.size() * 4));
        tsv += h + "\t-\tok\t" + b->resources + "\n";
    }
    std::ofstream(dir / "results.tsv", std::ios::binary) << tsv;
    if (!fs::exists(dir / "results.tsv")) return false;
    const std::string d = dir.generic_string();
    _putenv_s("NXRT_NVN_SHADER_CACHE", d.c_str());       // over test_main's work-folder default (the environment wins over the config)
    nxrt::config_set("nvn", "shader_cache", d.c_str());
    nxrt::config_set("nvn", "shader_order", "");      // no boot preload order file in the test's folder
    nxrt::config_set("nvn", "shader_pack", "false");
    // The back end opens its cache once per process. Open it on this folder now in case an earlier group (DLSS, in --all)
    // already made the back end look for one and found none; a cache already open on another folder cannot be moved.
    ShaderCache& cache = shader_cache();
    if (!cache.is_open()) cache.open(d);
    if (!cache.is_open() || fs::path(cache.dir()) != fs::path(d)) {
        std::fprintf(stderr, "shader cache is open on %s, not the test cache\n", cache.is_open() ? cache.dir().c_str() : "nothing");
        return false;
    }
    state = 1;
    return true;
}

using PushFn = void(NVNcommandBuffer*, int, int, int, const void*);
using PushDeferredFn = void(NVNcommandBuffer*, int, int, int, NVNbufferAddress);
template <class Sig>
abi::Caller<Sig> nvn2_proc(const char* name) {
    return abi::Caller<Sig>(proc_address(name));
}

App::ShaderBlob blob(const Block& b, const char* stage) { return App::ShaderBlob{b.code, b.control, "", stage}; }

// Render state at the NVN defaults with culling off.
struct States {
    NVNblendState bs;
    NVNcolorState cs;
    NVNchannelMaskState ms;
    NVNpolygonState ps;
    NVNdepthStencilState ds;
    NVNmultisampleState mss;
    void bind(App& app, NVNcommandBuffer* c) {
        auto& api = app.api;
        api.nvnBlendStateSetDefaults(&bs);
        api.nvnColorStateSetDefaults(&cs);
        api.nvnChannelMaskStateSetDefaults(&ms);
        api.nvnPolygonStateSetDefaults(&ps);
        api.nvnPolygonStateSetCullFace(&ps, NVN_FACE_NONE);
        api.nvnDepthStencilStateSetDefaults(&ds);
        api.nvnMultisampleStateSetDefaults(&mss);
        api.nvnCommandBufferBindBlendState(c, &bs);
        api.nvnCommandBufferBindColorState(c, &cs);
        api.nvnCommandBufferBindChannelMaskState(c, &ms);
        api.nvnCommandBufferBindPolygonState(c, &ps);
        api.nvnCommandBufferBindDepthStencilState(c, &ds);
        api.nvnCommandBufferBindMultisampleState(c, &mss);
    }
};

// One full-screen triangle with `prog` into `rt` (what `before` records comes first), copied into `out`; the texels after the queue is idle.
std::vector<uint8_t> render(App& app, NVNprogram* prog, nvnapp::Tex& rt, const App::Buf& out, const std::function<void(NVNcommandBuffer*)>& before) {
    auto cb = app.make_cmdbuf();
    if (!cb) return {};
    NVNcommandBuffer* c = cb->obj.get();
    auto& api = app.api;
    api.nvnCommandBufferBeginRecording(c);
    NVNtexture* t = rt.obj.get();
    api.nvnCommandBufferSetRenderTargets(c, 1, &t, nullptr, nullptr, nullptr);
    api.nvnCommandBufferSetViewport(c, 0, 0, kW, kH);
    api.nvnCommandBufferSetScissor(c, 0, 0, kW, kH);
    States st;
    st.bind(app, c);
    api.nvnCommandBufferBindProgram(c, prog, NVN_SHADER_STAGE_VERTEX_BIT | NVN_SHADER_STAGE_FRAGMENT_BIT);
    before(c);
    api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    api.nvnCommandBufferCopyTextureToBuffer(c, t, nullptr, nullptr, out.address, 0);
    app.end_and_submit(*cb);
    app.finish();
    return std::vector<uint8_t>(out.ptr, out.ptr + out.size);
}

// Every pixel within `tol` of (r, g, b) (and a, when a >= 0). Prints the first one that is not.
bool all_pixels(const std::vector<uint8_t>& px, int r, int g, int b, int a, int tol, const char* what) {
    if (px.size() < static_cast<size_t>(kW) * kH * 4) {
        std::printf("      %s: no picture\n", what);
        return false;
    }
    for (int i = 0; i < kW * kH; ++i) {
        const uint8_t* p = &px[static_cast<size_t>(i) * 4];
        if (std::abs(p[0] - r) > tol || std::abs(p[1] - g) > tol || std::abs(p[2] - b) > tol || (a >= 0 && std::abs(p[3] - a) > tol)) {
            std::printf("      %s: pixel %d is %d %d %d %d, want %d %d %d %d\n", what, i, p[0], p[1], p[2], p[3], r, g, b, a);
            return false;
        }
    }
    std::printf("      %s: all %d pixels %d %d %d\n", what, kW * kH, r, g, b);
    return true;
}

void fill(const nvnapp::Tex& t, uint8_t r, uint8_t g, uint8_t b) {
    uint8_t* p = t.memory();
    for (size_t i = 0; i + 4 <= t.size; i += 4) {
        p[i] = r;
        p[i + 1] = g;
        p[i + 2] = b;
        p[i + 3] = 255;
    }
}

// A 4x4 cube map (six faces), not registered.
std::unique_ptr<nvnapp::Tex> make_cube(App& app, NVNformat format = NVN_FORMAT_RGBA8) {
    auto t = std::make_unique<nvnapp::Tex>();
    t->obj = std::make_unique<NVNtexture>();
    std::memset(t->obj.get(), 0, sizeof(NVNtexture));
    auto& api = app.api;
    NVNtextureBuilder b;
    api.nvnTextureBuilderSetDevice(&b, app.device.get());
    api.nvnTextureBuilderSetDefaults(&b);
    api.nvnTextureBuilderSetTarget(&b, NVN_TEXTURE_TARGET_CUBEMAP);
    api.nvnTextureBuilderSetSize2D(&b, 4, 4);
    api.nvnTextureBuilderSetFormat(&b, format);
    const size_t size = api.nvnTextureBuilderGetStorageSize(&b), align = api.nvnTextureBuilderGetStorageAlignment(&b);
    t->pool = app.pool_main.get();
    t->offset = t->pool->take(size, std::max<size_t>(align, 256));
    t->size = size;
    t->width = t->height = 4;
    t->format = format;
    api.nvnTextureBuilderSetStorage(&b, t->pool->obj.get(), static_cast<ptrdiff_t>(t->offset));
    if (!api.nvnTextureInitialize(t->obj.get(), &b)) return nullptr;
    return t;
}

}  // namespace

// ---- meanings ----------------------------------------------------------------------------------------------------------

// c[0x18] is push constant slot 0, by name and (names stripped) by binding 91 + 14 * stage slot + (24 - 18).
NXTEST(bank_24_is_push_constant_slot_0) {
    for (bool named : {true, false}) {
        ShaderReflection r;
        const std::vector<uint32_t> w = named ? spv(kSpvPushFs) : without_names(spv(kSpvPushFs));
        REQUIRE(reflect_spirv(w.data(), w.size(), r));
        const DescriptorBinding* b = at(r, 0, 111);
        REQUIRE(b != nullptr);
        CHECK_EQ(static_cast<int>(b->nvn.source), static_cast<int>(NvnSource::PushConstants));
        CHECK_EQ(b->nvn.bank, 24);
        CHECK_EQ(b->nvn.index, 0);
    }
}

// The pool arrays get their meaning from the translator's "_cb0_40000000" names; make_shader_info moves them to kPoolSet (decorations and
// reflection) and keeps the translator's numbers for the harness record.
NXTEST(pool_arrays_mean_the_pools_and_move_to_their_set) {
    const Block& blk = pool_fs_block();
    ShaderReflection r;
    REQUIRE(reflect_spirv(blk.spirv.data(), blk.spirv.size(), r));
    const DescriptorBinding* t2d = by_name(r, "fp_t2d_cb0_40000000");
    const DescriptorBinding* cube = by_name(r, "fp_tcube_cb0_40000000");
    const DescriptorBinding* smp = by_name(r, "fp_s_cb0_40000000");
    const DescriptorBinding* c3 = by_name(r, "fp_c3");
    REQUIRE(t2d && cube && smp && c3);
    CHECK_EQ(static_cast<int>(t2d->nvn.source), static_cast<int>(NvnSource::TexturePoolArray));
    CHECK_EQ(static_cast<int>(cube->nvn.source), static_cast<int>(NvnSource::TexturePoolArray));
    CHECK_EQ(static_cast<int>(smp->nvn.source), static_cast<int>(NvnSource::SamplerPoolArray));
    CHECK_EQ(static_cast<int>(c3->nvn.source), static_cast<int>(NvnSource::UniformBlock));
    CHECK_EQ(t2d->count, kPoolTextureCount);
    CHECK_EQ(smp->count, kPoolSamplerCount);
    CHECK_EQ(pool_binding_of(*t2d), static_cast<int>(kPoolTex2D));
    CHECK_EQ(pool_binding_of(*cube), static_cast<int>(kPoolTexCube));
    CHECK_EQ(pool_binding_of(*smp), static_cast<int>(kPoolSamplers));

    ShaderInfo info;
    std::string err;
    REQUIRE_WHY(make_shader_info(blk.code.data(), blk.control.data(), blk.spirv, &blk.resources, info, &err), err.c_str());
    for (uint32_t b : {uint32_t(kPoolSamplers), uint32_t(kPoolTex2D), uint32_t(kPoolTexCube)}) CHECK(at(info.refl, kPoolSet, b) != nullptr);
    CHECK(at(info.refl, 2, 640) == nullptr);
    const DescriptorBinding* moved = at(info.refl, kPoolSet, kPoolTex2D);
    if (moved) {
        CHECK_EQ(moved->declared_set, 2u);
        CHECK_EQ(moved->declared_binding, 640u);
    }
    CHECK(tokens(describe_like_harness(info.refl)) == tokens(blk.resources));
    // the module's decorations say the same: the moved words reflect at kPoolSet
    ShaderReflection again;
    REQUIRE(reflect_spirv(info.spirv.data(), info.spirv.size(), again));
    CHECK(at(again, kPoolSet, kPoolTex2D) != nullptr && at(again, kPoolSet, kPoolTexCube) != nullptr && at(again, kPoolSet, kPoolSamplers) != nullptr);
    CHECK(at(again, 2, 640) == nullptr && at(again, 2, 641) == nullptr && at(again, 2, 642) == nullptr);
    // words read again (a stub whose SPIR-V was dropped) move with the moved reflection
    std::vector<uint32_t> reread = blk.spirv;
    ShaderReflection moved_refl = info.refl;
    CHECK(move_pool_arrays(reread, moved_refl));
    CHECK(reread == info.spirv);
    // a shape the pool set has no array for is refused
    ShaderReflection odd = r;
    for (auto& b : odd.bindings)
        if (b.nvn.source == NvnSource::TexturePoolArray) b.image.arrayed = true;
    std::vector<uint32_t> w = blk.spirv;
    CHECK(!move_pool_arrays(w, odd, &err));
}

// NVN2 compute: the storage buffer descriptors follow the header and control +0xa9 push words.
NXTEST(nvn2_compute_table_starts_after_the_push_words) {
    const Block& blk = push_cs_block();
    const ShaderControl sc = read_control(blk.control.data());
    CHECK_EQ(static_cast<int>(sc.c0_push_words), 28);
    CHECK_EQ(static_cast<int>(sc.c0_table_size), 0x90);
    struct Case { uint8_t push; const char* record; NvnSource source; int index; };
    const Case cases[] = {
        {28, "cb0=s0b1 sb[c0+0x20]=s1b0", NvnSource::StorageBuffer, 0},   // 0x10 + 0x70
        {0, "cb0=s0b1 sb[c0+0x4]=s1b0", NvnSource::StorageBuffer, 0},     // 0x10, no push words
        {28, "cb0=s0b1 sb[c0+0x4]=s1b0", NvnSource::Unknown, -1},         // inside the push words: no descriptor there
    };
    for (const Case& k : cases) {
        const Block b = make_block(9, 5, blk.spirv, k.record, k.push, 1);
        ShaderInfo info;
        std::string err;
        REQUIRE_WHY(make_shader_info(b.code.data(), b.control.data(), b.spirv, &b.resources, info, &err), err.c_str());
        const DescriptorBinding* s = by_name(info.refl, "cp_s0");
        const DescriptorBinding* c0 = by_name(info.refl, "cp_c0");
        REQUIRE(s && c0);
        CHECK_EQ(static_cast<int>(s->nvn.source), static_cast<int>(k.source));
        CHECK_EQ(s->nvn.index, k.index);
        CHECK_EQ(static_cast<int>(c0->nvn.source), static_cast<int>(NvnSource::DriverConstants));
    }
    // a texture whose handle word is an image slot reads that image (its handle: texture id, sampler 0)
    const Block& it = imgtex_cs_block();
    ShaderInfo info;
    std::string err;
    REQUIRE_WHY(make_shader_info(it.code.data(), it.control.data(), it.spirv, &it.resources, info, &err), err.c_str());
    const DescriptorBinding* t = by_name(info.refl, "cp_t_cb0_24");
    const DescriptorBinding* s = by_name(info.refl, "cp_s0");
    REQUIRE(t && s);
    CHECK_EQ(static_cast<int>(t->nvn.source), static_cast<int>(NvnSource::ImageBinding));
    CHECK_EQ(t->nvn.index, 0);
    CHECK_EQ(static_cast<int>(s->nvn.source), static_cast<int>(NvnSource::StorageBuffer));
}

// ---- recording ---------------------------------------------------------------------------------------------------------

// slot & 7, offset & 0xff, size clamped to 256 whole words, nothing under 4 bytes; the Deferred forms keep the address instead of data.
NXTEST(push_constants_are_recorded_as_the_driver_sends_them) {
    install();
    register_api_cmdbuf();
    register_api_cmd_work();
    auto gfx = nvn2_proc<PushFn>("nvnCommandBufferPushGraphicsConstants");
    auto cmp = nvn2_proc<PushFn>("nvnCommandBufferPushComputeConstants");
    auto gfx_deferred = nvn2_proc<PushDeferredFn>("nvnCommandBufferPushGraphicsConstantsDeferred");
    REQUIRE(gfx && cmp && gfx_deferred);
    NVNcommandBuffer c;
    std::memset(&c, 0, sizeof c);
    REQUIRE(abi::proc<PFNNVNCOMMANDBUFFERINITIALIZEPROC>("nvnCommandBufferInitialize")(&c, nullptr));
    abi::proc<PFNNVNCOMMANDBUFFERBEGINRECORDINGPROC>("nvnCommandBufferBeginRecording")(&c);
    uint8_t data[300];
    for (size_t i = 0; i < sizeof data; ++i) data[i] = static_cast<uint8_t>(i * 3);
    gfx(&c, 9, 0x104, 300, data);
    cmp(&c, 0, 0, 0x70, data);
    gfx(&c, 0, 0, 3, data);
    gfx(&c, 0, 0, 6, data);
    gfx_deferred(&c, 2, 8, 16, 0x12340000);
    const NVNcommandHandle h = abi::proc<PFNNVNCOMMANDBUFFERENDRECORDINGPROC>("nvnCommandBufferEndRecording")(&c);
    Ref<CmdList> list = handle_lookup(h);
    REQUIRE(list);
    RecordReader r(*list);
    struct Want { uint32_t compute, slot, offset, size; uint64_t address; };
    const Want want[] = {{0, 1, 4, 256, 0}, {1, 0, 0, 0x70, 0}, {0, 0, 0, 4, 0}, {0, 2, 8, 16, 0x12340000}};
    for (const Want& w : want) {
        REQUIRE(r.next());
        REQUIRE(r.op() == Op::PushConstants);
        const auto& p = r.payload<P_PushConstants>();
        CHECK_EQ(p.compute, w.compute);
        CHECK_EQ(p.slot, w.slot);
        CHECK_EQ(p.offset, w.offset);
        CHECK_EQ(p.size, w.size);
        CHECK_EQ(p.address, w.address);
        if (w.address) CHECK_EQ(r.size(), static_cast<uint32_t>(sizeof(RecHeader) + sizeof(P_PushConstants)));
        else CHECK(std::memcmp(r.blob<P_PushConstants>(), data, w.size) == 0);
    }
    CHECK(!r.next());
}

// ---- scenes on the GPU -------------------------------------------------------------------------------------------------

NXTEST(graphics_push_constants_reach_bank_24) {
    REQUIRE(cache_ready());
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE_WHY(app.init(o), app.error.c_str());
    App::ShaderBlob vs = blob(vs_block(), "vertex"), fs = blob(push_fs_block(), "fragment");
    auto prog = app.make_program({&vs, &fs});
    REQUIRE(prog != nullptr);
    auto rt = app.make_texture_2d(NVN_FORMAT_RGBA8, kW, kH, 0, 1, false);
    REQUIRE(rt != nullptr);
    const App::Buf out = app.make_buffer(static_cast<size_t>(kW) * kH * 4);
    auto push = nvn2_proc<PushFn>("nvnCommandBufferPushGraphicsConstants");
    REQUIRE(push);
    const auto s0 = nxrt::nvn::backend_stats();
    // rows 0 and 1 of slot 0: the shader shows row 1
    const float rows[8] = {9, 9, 9, 9, 0.25f, 0.5f, 0.75f, 1.0f};
    auto px = render(app, prog.get(), *rt, out, [&](NVNcommandBuffer* c) { push(c, 0, 0, sizeof rows, rows); });
    CHECK(all_pixels(px, 64, 128, 191, 255, 1, "slot 0 rows 0 and 1"));
    // 16 bytes at offset 16 replace row 1; slot 1 (bank 25) leaves slot 0 alone
    const float red[4] = {1, 0, 0, 1}, green[4] = {0, 1, 0, 1};
    px = render(app, prog.get(), *rt, out, [&](NVNcommandBuffer* c) {
        push(c, 0, 16, sizeof red, red);
        push(c, 1, 16, sizeof green, green);
    });
    CHECK(all_pixels(px, 255, 0, 0, 255, 0, "row 1 at offset 16"));
    // nothing pushed: the constants stay as the last command buffer left them
    px = render(app, prog.get(), *rt, out, [](NVNcommandBuffer*) {});
    CHECK(all_pixels(px, 255, 0, 0, 255, 0, "kept across command buffers"));
    const auto s1 = nxrt::nvn::backend_stats();
    CHECK_EQ(s1.draws - s0.draws, 3ull);
    CHECK_EQ(s1.draws_skipped - s0.draws_skipped, 0ull);
    CHECK_EQ(app.validation().errors, 0ull);
}

NXTEST(compute_push_constants_and_the_table_after_them) {
    REQUIRE(cache_ready());
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE_WHY(app.init(o), app.error.c_str());
    App::ShaderBlob cs = blob(push_cs_block(), "compute");
    auto prog = app.make_program({&cs});
    REQUIRE(prog != nullptr);
    const App::Buf sb = app.make_buffer(256);
    uint32_t* w = reinterpret_cast<uint32_t*>(sb.ptr);
    for (int i = 0; i < 64; ++i) w[i] = 0xFFFFFFFFu;
    uint32_t words[28];
    for (uint32_t i = 0; i < 28; ++i) words[i] = 0x1000 + i;
    auto push = nvn2_proc<PushFn>("nvnCommandBufferPushComputeConstants");
    REQUIRE(push);
    auto cb = app.make_cmdbuf();
    REQUIRE(cb != nullptr);
    NVNcommandBuffer* c = cb->obj.get();
    const auto s0 = nxrt::nvn::backend_stats();
    app.api.nvnCommandBufferBeginRecording(c);
    app.api.nvnCommandBufferBindProgram(c, prog.get(), NVN_SHADER_STAGE_COMPUTE_BIT);
    app.api.nvnCommandBufferBindStorageBuffer(c, NVN_SHADER_STAGE_COMPUTE, 0, sb.address, 256);
    push(c, 0, 0, sizeof words, words);
    app.api.nvnCommandBufferDispatchCompute(c, 3, 2, 1);
    app.end_and_submit(*cb);
    app.finish();
    const uint32_t want[7] = {0x1000, 0x1001, 3, 2, 1, 0x1000 + 27, 0xFFFFFFFFu};
    std::printf("    words 0..6: %x %x %x %x %x %x %x\n", w[0], w[1], w[2], w[3], w[4], w[5], w[6]);
    for (int i = 0; i < 7; ++i) CHECK_EQ(w[i], want[i]);
    const auto s1 = nxrt::nvn::backend_stats();
    CHECK_EQ(s1.dispatches - s0.dispatches, 1ull);
    CHECK_EQ(app.validation().errors, 0ull);
}

// The texture read through image slot 0 is the image bound with nvnCommandBufferBindImage.
NXTEST(compute_reads_its_image_through_the_image_slot) {
    REQUIRE(cache_ready());
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE_WHY(app.init(o), app.error.c_str());
    App::ShaderBlob cs = blob(imgtex_cs_block(), "compute");
    auto prog = app.make_program({&cs});
    REQUIRE(prog != nullptr);
    auto img = app.make_texture_2d(NVN_FORMAT_RGBA8, 4, 4, NVN_TEXTURE_FLAGS_IMAGE, 1, false);
    REQUIRE(img != nullptr);
    fill(*img, 64, 128, 192);
    app.api.nvnTexturePoolRegisterImage(app.tex_pool.get(), 7, img->obj.get(), nullptr);
    const App::Buf sb = app.make_buffer(256);
    uint32_t* w = reinterpret_cast<uint32_t*>(sb.ptr);
    for (int i = 0; i < 64; ++i) w[i] = 0xFFFFFFFFu;
    auto cb = app.make_cmdbuf();
    REQUIRE(cb != nullptr);
    NVNcommandBuffer* c = cb->obj.get();
    app.api.nvnCommandBufferBeginRecording(c);
    app.api.nvnCommandBufferSetTexturePool(c, app.tex_pool.get());
    app.api.nvnCommandBufferSetSamplerPool(c, app.smp_pool.get());
    app.api.nvnCommandBufferBindProgram(c, prog.get(), NVN_SHADER_STAGE_COMPUTE_BIT);
    app.api.nvnCommandBufferBindStorageBuffer(c, NVN_SHADER_STAGE_COMPUTE, 0, sb.address, 256);
    app.api.nvnCommandBufferBindImage(c, NVN_SHADER_STAGE_COMPUTE, 0, app.api.nvnDeviceGetImageHandle(app.device.get(), 7));
    app.api.nvnCommandBufferDispatchCompute(c, 1, 1, 1);
    app.end_and_submit(*cb);
    app.finish();
    float r = 0, g = 0;
    std::memcpy(&r, &w[0], 4);
    std::memcpy(&g, &w[1], 4);
    std::printf("    texel (0, 0) through the image slot: %g %g\n", r, g);
    CHECK(std::fabs(r - 64.0f / 255.0f) < 1e-3f);
    CHECK(std::fabs(g - 128.0f / 255.0f) < 1e-3f);
    CHECK_EQ(app.validation().errors, 0ull);
}

NXTEST(pool_arrays_sample_by_handle) {
    REQUIRE(cache_ready());
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE_WHY(app.init(o), app.error.c_str());
    App::ShaderBlob vs = blob(vs_block(), "vertex"), fs = blob(pool_fs_block(), "fragment");
    auto prog = app.make_program({&vs, &fs});
    REQUIRE(prog != nullptr);
    auto& api = app.api;
    // 2D textures at ids 5 (red) and 9 (green), a cube at 12 (32, 0, 0), all filled before their first use
    auto red = app.make_texture_2d(NVN_FORMAT_RGBA8, 4, 4, 0, 1, false), green = app.make_texture_2d(NVN_FORMAT_RGBA8, 4, 4, 0, 1, false),
         blue = app.make_texture_2d(NVN_FORMAT_RGBA8, 4, 4, 0, 1, false);
    auto cube = make_cube(app);
    REQUIRE(red && green && blue && cube);
    fill(*red, 255, 0, 0);
    fill(*green, 0, 255, 0);
    fill(*blue, 0, 0, 255);
    fill(*cube, 32, 0, 0);
    api.nvnTexturePoolRegisterTexture(app.tex_pool.get(), 5, red->obj.get(), nullptr);
    api.nvnTexturePoolRegisterTexture(app.tex_pool.get(), 9, green->obj.get(), nullptr);
    api.nvnTexturePoolRegisterTexture(app.tex_pool.get(), 12, cube->obj.get(), nullptr);
    const int smp = app.make_nearest_sampler();
    const App::Buf ub = app.make_buffer(256);
    uint32_t* handles = reinterpret_cast<uint32_t*>(ub.ptr);
    auto rt = app.make_texture_2d(NVN_FORMAT_RGBA8, kW, kH, 0, 1, false);
    REQUIRE(rt != nullptr);
    const App::Buf out = app.make_buffer(static_cast<size_t>(kW) * kH * 4);
    auto frame = [&](uint32_t h2d, uint32_t hcube) {
        handles[0] = h2d;
        handles[1] = hcube;
        return render(app, prog.get(), *rt, out, [&](NVNcommandBuffer* c) {
            api.nvnCommandBufferSetTexturePool(c, app.tex_pool.get());
            api.nvnCommandBufferSetSamplerPool(c, app.smp_pool.get());
            api.nvnCommandBufferBindUniformBuffer(c, NVN_SHADER_STAGE_FRAGMENT, 0, ub.address, 256);
        });
    };
    const auto s0 = nxrt::nvn::backend_stats();
    const uint32_t h5 = static_cast<uint32_t>(app.texture_handle(5, smp)), h9 = static_cast<uint32_t>(app.texture_handle(9, smp)),
                   h12 = static_cast<uint32_t>(app.texture_handle(12, smp));
    CHECK(all_pixels(frame(h9, h12), 32, 255, 0, 255, 1, "2D id 9 green + cube id 12"));
    CHECK(all_pixels(frame(h5, h12), 255, 0, 0, 255, 1, "2D id 5 red + cube id 12"));
    // id 9 registered again: the next draw samples the new texture (only that id is written)
    api.nvnTexturePoolRegisterTexture(app.tex_pool.get(), 9, blue->obj.get(), nullptr);
    CHECK(all_pixels(frame(h9, h12), 32, 0, 255, 255, 1, "id 9 now blue + cube id 12"));
    // A cube as 2D reads its first face; a 2D texture as cube gets the dummy.
    CHECK(all_pixels(frame(h12, h9), 32, 0, 0, -1, 1, "cube first face as 2D, flat texture as cube dummy"));
    // finalized while still registered at id 5, no live texture on its storage: the dummy, not the released image
    api.nvnTextureFinalize(red->obj.get());
    CHECK(all_pixels(frame(h5, h12), 32, 0, 0, -1, 1, "id 5 finalized + cube id 12"));
    // A float-backed probe registered through an explicit integer view must still be a float
    // image in the pool: the translated TEX returns register bits through OpTypeFloat.
    auto probe = make_cube(app, NVN_FORMAT_RGBA32F);
    REQUIRE(probe != nullptr);
    NVNcopyRegion region = {0, 0, 0, 4, 4, 6};
    std::vector<float> texels(4 * 4 * 6 * 4, 0.0f);
    for (size_t i = 0; i < texels.size(); i += 4) { texels[i] = 0.5f; texels[i + 3] = 1.0f; }
    api.nvnTextureWriteTexels(probe->obj.get(), nullptr, &region, texels.data());
    NVNtextureView view;
    api.nvnTextureViewSetDefaults(&view);
    api.nvnTextureViewSetFormat(&view, NVN_FORMAT_RGBA32UI);
    api.nvnTexturePoolRegisterTexture(app.tex_pool.get(), 12, probe->obj.get(), &view);
    CHECK(all_pixels(frame(h5, h12), 128, 0, 0, -1, 1, "float probe via explicit integer view"));
    auto integer_probe = make_cube(app, NVN_FORMAT_RGBA32UI);
    REQUIRE(integer_probe != nullptr);
    api.nvnTextureWriteTexels(integer_probe->obj.get(), nullptr, &region, texels.data());
    api.nvnTexturePoolRegisterTexture(app.tex_pool.get(), 12, integer_probe->obj.get(), nullptr);
    CHECK(all_pixels(frame(h5, h12), 128, 0, 0, -1, 1, "integer probe holding float register bits"));
    api.nvnTextureViewSetFormat(&view, NVN_FORMAT_RGBA32F);
    api.nvnTexturePoolRegisterTexture(app.tex_pool.get(), 12, integer_probe->obj.get(), &view);
    CHECK(all_pixels(frame(h5, h12), 128, 0, 0, -1, 1, "integer probe via explicit float view"));
    const auto s1 = nxrt::nvn::backend_stats();
    CHECK_EQ(s1.draws - s0.draws, 8ull);
    CHECK_EQ(s1.draws_skipped - s0.draws_skipped, 0ull);
    CHECK_EQ(app.validation().errors, 0ull);
}

// [gfx] float16_scene: an R11G11B10F texture with the image flag is made R11G11B10F (one that is only ever sampled keeps 4 bytes a texel),
// and remade as RGBA16F when a shader first stores to it, its CPU-filled content copied over exactly. Run in a fresh process: the setting
// is read once per process.
namespace {
VkFormat backend_format(const nvnapp::Tex& t) {
    VkFormat f = VK_FORMAT_UNDEFINED;
    be::gpu::run_on_render_thread([&] { f = be::gpu::tex_vk_format(be::host_of<be::Texture>(t.obj.get())); });
    return f;
}
float word_float(uint32_t w) {
    float f;
    std::memcpy(&f, &w, 4);
    return f;
}
}  // namespace

NXTEST_CHILD(float16_scene_store_child) {
    nxrt::config_set("gfx", "float16_scene", "on");
    if (!cache_ready()) return 2;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    if (!app.init(o)) { std::printf("    init: %s\n", app.error.c_str()); return 3; }
    App::ShaderBlob cs = blob(storeimg_cs_block(), "compute");
    auto prog = app.make_program({&cs});
    if (!prog) return 4;
    // every texel (0.7578125, 0.25, 2): red 0.5 * (1 + 33/64) (exponent 14, mantissa 33), green exponent 13, blue exponent 16
    constexpr uint32_t kTexel = (0x380u | 33u) | (0x340u << 11) | (0x200u << 22);
    auto img = app.make_texture_2d(NVN_FORMAT_R11G11B10F, 4, 4, NVN_TEXTURE_FLAGS_IMAGE, 1, false);
    auto sampled = app.make_texture_2d(NVN_FORMAT_R11G11B10F, 4, 4, NVN_TEXTURE_FLAGS_IMAGE, 1, false);
    if (!img || !sampled) return 5;
    for (size_t i = 0; i + 4 <= img->size; i += 4) std::memcpy(img->memory() + i, &kTexel, 4);
    app.api.nvnTexturePoolRegisterImage(app.tex_pool.get(), 7, img->obj.get(), nullptr);
    const App::Buf sb = app.make_buffer(256);
    uint32_t* w = reinterpret_cast<uint32_t*>(sb.ptr);
    int bad = 0;
    const auto expect = [&](const char* what, bool ok) {
        std::printf("    %s: %s\n", what, ok ? "ok" : "WRONG");
        bad += ok ? 0 : 1;
    };
    expect("an image-flag R11G11B10F texture is made R11G11B10F", backend_format(*sampled) == VK_FORMAT_B10G11R11_UFLOAT_PACK32);
    for (int run = 0; run < 2; ++run) {
        for (int i = 0; i < 64; ++i) w[i] = 0xFFFFFFFFu;
        auto cb = app.make_cmdbuf();
        if (!cb) return 6;
        NVNcommandBuffer* c = cb->obj.get();
        app.api.nvnCommandBufferBeginRecording(c);
        app.api.nvnCommandBufferSetTexturePool(c, app.tex_pool.get());
        app.api.nvnCommandBufferSetSamplerPool(c, app.smp_pool.get());
        app.api.nvnCommandBufferBindProgram(c, prog.get(), NVN_SHADER_STAGE_COMPUTE_BIT);
        app.api.nvnCommandBufferBindStorageBuffer(c, NVN_SHADER_STAGE_COMPUTE, 0, sb.address, 256);
        app.api.nvnCommandBufferBindImage(c, NVN_SHADER_STAGE_COMPUTE, 0, app.api.nvnDeviceGetImageHandle(app.device.get(), 7));
        app.api.nvnCommandBufferDispatchCompute(c, 1, 1, 1);
        app.end_and_submit(*cb);
        app.finish();
        const float r = word_float(w[0]), g = word_float(w[1]), b = word_float(w[2]), r10 = word_float(w[3]);
        std::printf("    run %d: texel (0, 0) %.9g %.9g %.9g, texel (1, 0) red %.9g\n", run, r, g, b, r10);
        expect("texel (0, 0) is the CPU's, exactly", r == 0.7578125f && g == 0.25f && b == 2.0f);
        if (run == 0) expect("texel (1, 0) is the CPU's before the store", r10 == 0.7578125f);
        else expect("texel (1, 0) keeps the store's 1 + 2^-9 (float16, not R11G11B10F's 1)", r10 == 1.001953125f);
    }
    expect("the stored-to texture is RGBA16F", backend_format(*img) == VK_FORMAT_R16G16B16A16_SFLOAT);
    expect("the sampled-only texture is still R11G11B10F", backend_format(*sampled) == VK_FORMAT_B10G11R11_UFLOAT_PACK32);
    // Buffer copies of the RGBA16F image use the NVN texel size (4 bytes): out of the image, texel (1, 0) is the store rounded back to
    // R11G11B10F (1, 0.25, 0.5), and the 16 words after the 64 bytes asked for stay untouched; into it, a buffer of other texels lands whole.
    constexpr uint32_t kStored = 0x3c0u | (0x340u << 11) | (0x1c0u << 22);
    constexpr uint32_t kOther = (0x3c0u | 5u) | (0x380u << 11) | (0x1e0u << 22);   // (1 + 5/64, 0.5, 1)
    const App::Buf cb_buf = app.make_buffer(256);
    uint32_t* cw = reinterpret_cast<uint32_t*>(cb_buf.ptr);
    const auto copy = [&](bool to_texture) {
        auto cb = app.make_cmdbuf();
        if (!cb) return false;
        NVNcommandBuffer* c = cb->obj.get();
        app.api.nvnCommandBufferBeginRecording(c);
        app.api.nvnCommandBufferSetCopyRowStride(c, 16);
        NVNcopyRegion reg{0, 0, 0, 4, 4, 1};
        if (to_texture) app.api.nvnCommandBufferCopyBufferToTexture(c, cb_buf.address, img->obj.get(), nullptr, &reg, NVN_COPY_FLAGS_NONE);
        else app.api.nvnCommandBufferCopyTextureToBuffer(c, img->obj.get(), nullptr, &reg, cb_buf.address, NVN_COPY_FLAGS_NONE);
        app.end_and_submit(*cb);
        app.finish();
        return true;
    };
    for (int i = 0; i < 64; ++i) cw[i] = 0xEEEEEEEEu;
    if (!copy(false)) return 7;
    std::printf("    copied out: texel (0, 0) 0x%08x, (1, 0) 0x%08x, word 16 0x%08x\n", cw[0], cw[1], cw[16]);
    bool rest = true;
    for (int i = 2; i < 16; ++i) rest = rest && cw[i] == (i == 5 ? cw[i] : kTexel);   // texel (1, 1) is word 5: untouched by the shader too
    expect("a copy out of it gives NVN texels", cw[0] == kTexel && cw[1] == kStored && rest);
    bool guard = true;
    for (int i = 16; i < 64; ++i) guard = guard && cw[i] == 0xEEEEEEEEu;
    expect("a copy out of it writes only the bytes asked for", guard);
    for (int i = 0; i < 16; ++i) cw[i] = kOther;
    if (!copy(true)) return 8;
    for (int i = 0; i < 64; ++i) cw[i] = 0xEEEEEEEEu;
    if (!copy(false)) return 9;
    bool all = true;
    for (int i = 0; i < 16; ++i) all = all && cw[i] == kOther;
    std::printf("    after a copy in: texel (0, 0) 0x%08x, (3, 3) 0x%08x\n", cw[0], cw[15]);
    expect("a copy into it lands exactly", all && cw[16] == 0xEEEEEEEEu);
    const uint64_t errors = app.validation().errors;
    std::printf("    validation errors: %llu\n", static_cast<unsigned long long>(errors));
    bad += errors ? 1 : 0;
    return bad;
}

NXTEST(float16_scene_promotes_at_the_first_store) {
    char exe[MAX_PATH] = {};
    GetModuleFileNameA(nullptr, exe, MAX_PATH);
    // cmd /c strips the outer pair of quotes, so the whole command is wrapped in one more pair.
    const std::string cmd = std::string("\"\"") + exe + "\" --child float16_scene_store_child\"";
    const int rc = std::system(cmd.c_str());
    CHECK_MSG(rc == 0, "child returned " + std::to_string(rc));
}
