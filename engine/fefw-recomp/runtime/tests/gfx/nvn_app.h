// SPDX-License-Identifier: GPL-3.0-or-later
// A tiny "game" for the NVN back end: it talks to NVN only through nvnBootstrapLoader / nvnDeviceGetProcAddress and the
// function pointers they return (typed callers over the AArch64 thunks, src/gfx/nvn/nvn_abi.h), written against the NVN header of
// the practice mod (third_party/nvn/nvn.h). It owns a headless gfx::Device + Renderer with validation, installs the back end, and
// creates what a game creates: device, queue, memory pools, a window with display textures, texture and sampler pools, command
// buffers. Scene tests build on it (tests/gfx/test_nvn_scene_*.cpp). Nothing here is game data: shaders and textures of the real
// game are read at run time from work/ (shader_cache/corpus_code and survey/textures), never committed.
#pragma once
#include <windows.h>

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <memory>
#include <string>
#include <vector>

#include "nxrt/gfx.h"
#include "nxrt/log.h"
#include "nxrt/nvn.h"
#include "nxrt/nvn_backend.h"
#include "nvn_abi.h"
#include "nvn_gpu.h"
#include "project_root.h"
#include "nvn_shaders.h"
#include "test_framework.h"

namespace nvnapp {

using nxrt::nvn::abi::Caller;
using nxrt::nvn::abi::PfnTraits;

template <class Pfn>
using Fn = Caller<typename PfnTraits<Pfn>::Sig>;

// Every NVN function with a PFN typedef, as a typed member: api.nvnTextureInitialize(&tex, &builder).
struct Api {
#define X(name, pfn) Fn<pfn> name{0};
#include "nvn_api.inc"
#undef X
    // Resolves every function through nvnBootstrapLoader -> nvnDeviceGetProcAddress, as a game does. Returns the count.
    int load(NVNdevice* device) {
        Caller<void*(const char*)> bootstrap(nxrt::hle_address("nvnBootstrapLoader"));
        void* get_proc = bootstrap("nvnDeviceGetProcAddress");
        Caller<void*(const NVNdevice*, const char*)> gp(reinterpret_cast<uint64_t>(get_proc));
        int n = 0;
#define X(name, pfn) { name = Fn<pfn>(reinterpret_cast<uint64_t>(gp(device, #name))); n += name ? 1 : 0; }
#include "nvn_api.inc"
#undef X
        return n;
    }
};

inline std::string shader_corpus_dir() {
    const char* e = std::getenv("NXRT_NVN_CORPUS");
    return e ? e : nxtest::project_path("work/shader_cache/corpus_code");
}
inline std::string shader_cache_dir() {
    const char* e = std::getenv("NXRT_NVN_SHADER_CACHE");
    return e ? e : nxtest::project_path("work/shader_cache/spirv");
}
inline bool real_data_available() {
    std::ifstream a(shader_corpus_dir() + "/manifest.tsv"), b(shader_cache_dir() + "/results.tsv");
    return a.good() && b.good();
}

// Host memory for pools: a pagefile-backed section viewed twice, like the runtime's heap (the back end imports the second view).
struct HostMem {
    HANDLE section = nullptr;
    uint8_t* view = nullptr;
    size_t size = 0;
    bool alloc(size_t n) {
        size = (n + 0xfff) & ~size_t(0xfff);
        section = CreateFileMappingW(INVALID_HANDLE_VALUE, nullptr, PAGE_READWRITE, static_cast<DWORD>(static_cast<uint64_t>(size) >> 32), static_cast<DWORD>(size), nullptr);
        if (!section) return false;
        view = static_cast<uint8_t*>(MapViewOfFile(section, FILE_MAP_ALL_ACCESS, 0, 0, size));
        return view != nullptr;
    }
    void release() {
        if (view) UnmapViewOfFile(view);
        if (section) CloseHandle(section);
        view = nullptr;
        section = nullptr;
    }
};

struct Pool {
    std::unique_ptr<NVNmemoryPool> obj;
    HostMem mem;
    size_t bump = 0;
    uint8_t* base() const { return mem.view; }
    size_t size() const { return mem.size; }
    // Bump allocation inside the pool: offset aligned to `align`.
    size_t take(size_t n, size_t align = 256) {
        bump = (bump + align - 1) & ~(align - 1);
        const size_t at = bump;
        bump += n;
        return at;
    }
};

struct Tex {
    std::unique_ptr<NVNtexture> obj;
    Pool* pool = nullptr;
    size_t offset = 0, size = 0;
    int width = 0, height = 0, format = 0, levels = 1, id = -1;
    uint8_t* memory() const { return pool->base() + offset; }
};

struct AppOpts {
    uint32_t width = 256, height = 256;
    bool upper_left = true;     // nvnDeviceSetWindowOriginMode
    bool depth_zero = true;     // nvnDeviceSetDepthMode NEAR_IS_ZERO
    int window_textures = 2;
    bool register_entry_points = true;
};

class App {
public:
    Api api;
    std::unique_ptr<nxrt::gfx::Device> dev;
    std::unique_ptr<nxrt::gfx::Renderer> rend;
    std::unique_ptr<NVNdevice> device;
    std::unique_ptr<NVNqueue> queue;
    std::unique_ptr<NVNwindow> window;
    std::unique_ptr<NVNsync> sync;
    std::vector<std::unique_ptr<Tex>> window_textures;
    std::unique_ptr<Pool> pool_main;        // general purpose GPU memory
    std::unique_ptr<Pool> pool_cmd;         // command memory
    std::unique_ptr<Pool> pool_desc;        // texture / sampler pool memory
    std::unique_ptr<NVNtexturePool> tex_pool;
    std::unique_ptr<NVNsamplerPool> smp_pool;
    std::vector<uint8_t> control_mem;
    uint32_t width = 0, height = 0;
    int next_tex_id = 0, next_smp_id = 0;
    bool ok = false;
    std::string error;

    using Opts = AppOpts;

    bool init() { return init(AppOpts()); }
    bool init(const AppOpts& o) {
        width = o.width;
        height = o.height;
        nxrt::gfx::DeviceDesc dd;
        dd.validation = 1;
        dd.headless = 1;
        std::string err;
        dev = nxrt::gfx::Device::create(dd, &err);
        if (!dev) { error = "device: " + err; return false; }
        nxrt::gfx::RendererDesc rd;
        rd.headless_width = o.width;
        rd.headless_height = o.height;
        rd.present_mode = nxrt::gfx::PresentMode::Immediate;
        rend = nxrt::gfx::Renderer::create(*dev, rd, &err);
        if (!rend) { error = "renderer: " + err; return false; }
        nxrt::nvn::BackendDesc bd;
        bd.device = dev.get();
        bd.renderer = rend.get();
        bd.register_entry_points = o.register_entry_points;
        if (!nxrt::nvn::install_backend(bd)) { error = "install_backend failed"; return false; }
        device = std::make_unique<NVNdevice>();
        std::memset(device.get(), 0, sizeof(NVNdevice));
        // the function table is needed before the device exists: the bootstrap loader answers nvnDeviceGetProcAddress for any device
        api.load(device.get());
        NVNdeviceBuilder db;
        api.nvnDeviceBuilderSetDefaults(&db);
        api.nvnDeviceBuilderSetFlags(&db, 0);
        if (!api.nvnDeviceInitialize(device.get(), &db)) { error = "nvnDeviceInitialize failed"; return false; }
        api.load(device.get());
        api.nvnDeviceSetWindowOriginMode(device.get(), o.upper_left ? NVN_WINDOW_ORIGIN_MODE_UPPER_LEFT : NVN_WINDOW_ORIGIN_MODE_LOWER_LEFT);
        api.nvnDeviceSetDepthMode(device.get(), o.depth_zero ? NVN_DEPTH_MODE_NEAR_IS_ZERO : NVN_DEPTH_MODE_NEAR_IS_MINUS_W);

        queue = std::make_unique<NVNqueue>();
        std::memset(queue.get(), 0, sizeof(NVNqueue));
        NVNqueueBuilder qb;
        api.nvnQueueBuilderSetDevice(&qb, device.get());
        api.nvnQueueBuilderSetDefaults(&qb);
        if (!api.nvnQueueInitialize(queue.get(), &qb)) { error = "nvnQueueInitialize failed"; return false; }

        pool_main = make_pool(64u << 20);
        pool_cmd = make_pool(4u << 20);
        pool_desc = make_pool(1u << 20);
        control_mem.assign(1u << 20, 0);

        // descriptor pools
        tex_pool = std::make_unique<NVNtexturePool>();
        smp_pool = std::make_unique<NVNsamplerPool>();
        std::memset(tex_pool.get(), 0, sizeof(NVNtexturePool));
        std::memset(smp_pool.get(), 0, sizeof(NVNsamplerPool));
        const size_t toff = pool_desc->take(256 * 32 * 4, 256), soff = pool_desc->take(256 * 32, 256);
        if (!api.nvnTexturePoolInitialize(tex_pool.get(), pool_desc->obj.get(), static_cast<ptrdiff_t>(toff), 256 * 4)) { error = "texture pool"; return false; }
        if (!api.nvnSamplerPoolInitialize(smp_pool.get(), pool_desc->obj.get(), static_cast<ptrdiff_t>(soff), 256)) { error = "sampler pool"; return false; }

        sync = std::make_unique<NVNsync>();
        std::memset(sync.get(), 0, sizeof(NVNsync));
        api.nvnSyncInitialize(sync.get(), device.get());

        // window: display textures
        std::vector<NVNtexture*> tp;
        for (int i = 0; i < o.window_textures; ++i) {
            auto t = make_texture_2d(NVN_FORMAT_RGBA8, static_cast<int>(o.width), static_cast<int>(o.height), NVN_TEXTURE_FLAGS_DISPLAY | NVN_TEXTURE_FLAGS_COMPRESSIBLE, 1, false);
            if (!t) { error = "window texture"; return false; }
            tp.push_back(t->obj.get());
            window_textures.push_back(std::move(t));
        }
        window = std::make_unique<NVNwindow>();
        std::memset(window.get(), 0, sizeof(NVNwindow));
        NVNwindowBuilder wb;
        api.nvnWindowBuilderSetDevice(&wb, device.get());
        api.nvnWindowBuilderSetDefaults(&wb);
        api.nvnWindowBuilderSetTextures(&wb, static_cast<int>(tp.size()), tp.data());
        api.nvnWindowBuilderSetPresentInterval(&wb, 1);
        if (!api.nvnWindowInitialize(window.get(), &wb)) { error = "nvnWindowInitialize failed"; return false; }
        ok = true;
        return true;
    }

    // ---- objects -----------------------------------------------------------------------------------------------------------
    std::unique_ptr<Pool> make_pool(size_t size, int flags = NVN_MEMORY_POOL_FLAGS_CPU_UNCACHED | NVN_MEMORY_POOL_FLAGS_GPU_CACHED) {
        auto p = std::make_unique<Pool>();
        p->obj = std::make_unique<NVNmemoryPool>();
        std::memset(p->obj.get(), 0, sizeof(NVNmemoryPool));
        if (!p->mem.alloc(size)) return nullptr;
        NVNmemoryPoolBuilder b;
        api.nvnMemoryPoolBuilderSetDevice(&b, device.get());
        api.nvnMemoryPoolBuilderSetDefaults(&b);
        api.nvnMemoryPoolBuilderSetStorage(&b, p->mem.view, p->mem.size);
        api.nvnMemoryPoolBuilderSetFlags(&b, flags);
        if (!api.nvnMemoryPoolInitialize(p->obj.get(), &b)) return nullptr;
        return p;
    }

    std::unique_ptr<Tex> make_texture_2d(int format, int w, int h, int flags = 0, int levels = 1, bool register_in_pool = true, Pool* pool = nullptr) {
        auto t = std::make_unique<Tex>();
        t->obj = std::make_unique<NVNtexture>();
        std::memset(t->obj.get(), 0, sizeof(NVNtexture));
        Pool* pl = pool ? pool : pool_main.get();
        NVNtextureBuilder b;
        api.nvnTextureBuilderSetDevice(&b, device.get());
        api.nvnTextureBuilderSetDefaults(&b);
        api.nvnTextureBuilderSetFlags(&b, flags);
        api.nvnTextureBuilderSetTarget(&b, NVN_TEXTURE_TARGET_2D);
        api.nvnTextureBuilderSetSize2D(&b, w, h);
        api.nvnTextureBuilderSetFormat(&b, static_cast<NVNformat>(format));
        api.nvnTextureBuilderSetLevels(&b, levels);
        const size_t size = api.nvnTextureBuilderGetStorageSize(&b);
        const size_t align = api.nvnTextureBuilderGetStorageAlignment(&b);
        t->offset = pl->take(size, std::max<size_t>(align, 256));
        t->size = size;
        t->pool = pl;
        api.nvnTextureBuilderSetStorage(&b, pl->obj.get(), static_cast<ptrdiff_t>(t->offset));
        if (!api.nvnTextureInitialize(t->obj.get(), &b)) return nullptr;
        t->width = w;
        t->height = h;
        t->format = format;
        t->levels = levels;
        if (register_in_pool) {
            t->id = next_tex_id++;
            api.nvnTexturePoolRegisterTexture(tex_pool.get(), t->id, t->obj.get(), nullptr);
        }
        return t;
    }

    // Registers a sampler (built with the defaults plus whatever `setup` does) and returns its pool id.
    template <class F>
    int make_sampler(F&& setup) {
        NVNsamplerBuilder sb;
        api.nvnSamplerBuilderSetDevice(&sb, device.get());
        api.nvnSamplerBuilderSetDefaults(&sb);
        setup(sb);
        const int id = next_smp_id++;
        api.nvnSamplerPoolRegisterSamplerBuilder(smp_pool.get(), id, &sb);
        return id;
    }
    int make_nearest_sampler() {
        return make_sampler([&](NVNsamplerBuilder& sb) {
            api.nvnSamplerBuilderSetMinMagFilter(&sb, NVN_MIN_FILTER_NEAREST, NVN_MAG_FILTER_NEAREST);
            api.nvnSamplerBuilderSetWrapMode(&sb, NVN_WRAP_MODE_CLAMP, NVN_WRAP_MODE_CLAMP, NVN_WRAP_MODE_CLAMP);
        });
    }
    int make_linear_sampler() {
        return make_sampler([&](NVNsamplerBuilder& sb) {
            api.nvnSamplerBuilderSetMinMagFilter(&sb, NVN_MIN_FILTER_LINEAR, NVN_MAG_FILTER_LINEAR);
            api.nvnSamplerBuilderSetWrapMode(&sb, NVN_WRAP_MODE_CLAMP, NVN_WRAP_MODE_CLAMP, NVN_WRAP_MODE_CLAMP);
        });
    }
    uint64_t texture_handle(int tex_id, int sampler_id) { return api.nvnDeviceGetTextureHandle(device.get(), tex_id, sampler_id); }

    // A buffer in pool_main (no NVNbuffer object needed for vertex/index/uniform data: GPU addresses are what commands take).
    struct Buf {
        uint64_t address = 0;
        uint8_t* ptr = nullptr;
        size_t size = 0;
    };
    Buf make_buffer(size_t size, const void* data = nullptr, size_t align = 256) {
        Buf b;
        const size_t off = pool_main->take(size, align);
        b.ptr = pool_main->base() + off;
        b.size = size;
        b.address = api.nvnMemoryPoolGetBufferAddress(pool_main->obj.get()) + off;
        if (data) std::memcpy(b.ptr, data, size);
        return b;
    }

    // ---- translated shaders of the real game (skipped when the survey data is not on this machine) ----------------------------
    struct ShaderBlob {
        std::vector<uint8_t> code, control;
        std::string hash, stage;
    };
    // Loads the block with this code hash from the corpus (work/shader_cache/corpus_code).
    static bool load_shader(const std::string& hash, ShaderBlob* out) {
        auto rd = [](const std::string& p, std::vector<uint8_t>* v) {
            std::ifstream f(p, std::ios::binary);
            if (!f) return false;
            v->assign(std::istreambuf_iterator<char>(f), std::istreambuf_iterator<char>());
            return true;
        };
        out->hash = hash;
        return rd(shader_corpus_dir() + "/code_" + hash + ".bin", &out->code) && rd(shader_corpus_dir() + "/ctrl_" + hash + ".bin", &out->control);
    }

    // An NVNprogram from blobs (vertex first, then fragment). The blobs must stay alive while the program is used.
    std::unique_ptr<NVNprogram> make_program(const std::vector<const ShaderBlob*>& stages) {
        auto p = std::make_unique<NVNprogram>();
        std::memset(p.get(), 0, sizeof(NVNprogram));
        if (!api.nvnProgramInitialize(p.get(), device.get())) return nullptr;
        std::vector<NVNshaderData> sd;
        for (const ShaderBlob* s : stages) {
            NVNshaderData d;
            d.data = reinterpret_cast<NVNbufferAddress>(s->code.data());
            d.control = s->control.data();
            sd.push_back(d);
        }
        if (!api.nvnProgramSetShaders(p.get(), static_cast<int>(sd.size()), sd.data())) return nullptr;
        return p;
    }

    // ---- recording ---------------------------------------------------------------------------------------------------------
    struct CmdBuf {
        std::unique_ptr<NVNcommandBuffer> obj;
        size_t cmd_off = 0, cmd_size = 0;
    };
    std::unique_ptr<CmdBuf> make_cmdbuf() {
        auto c = std::make_unique<CmdBuf>();
        c->obj = std::make_unique<NVNcommandBuffer>();
        std::memset(c->obj.get(), 0, sizeof(NVNcommandBuffer));
        if (!api.nvnCommandBufferInitialize(c->obj.get(), device.get())) return nullptr;
        c->cmd_size = 256 * 1024;
        c->cmd_off = pool_cmd->take(c->cmd_size, 256);
        api.nvnCommandBufferAddCommandMemory(c->obj.get(), pool_cmd->obj.get(), static_cast<ptrdiff_t>(c->cmd_off), c->cmd_size);
        api.nvnCommandBufferAddControlMemory(c->obj.get(), control_mem.data(), control_mem.size());
        return c;
    }
    NVNcommandHandle end_and_submit(CmdBuf& c) {
        const NVNcommandHandle h = api.nvnCommandBufferEndRecording(c.obj.get());
        api.nvnQueueSubmitCommands(queue.get(), 1, &h);
        return h;
    }
    void finish() { api.nvnQueueFinish(queue.get()); }

    // Acquire the next window texture; returns its index.
    int acquire() {
        int idx = -1;
        api.nvnWindowAcquireTexture(window.get(), sync.get(), &idx);
        return idx;
    }
    // Presents window texture `idx` and returns what the renderer showed (the headless frame, RGBA8, width x height).
    bool present_and_capture(int idx, std::vector<uint8_t>* px) {
        uint32_t w = 0, h = 0;
        rend->capture_next_frame(px, &w, &h);
        api.nvnQueuePresentTexture(queue.get(), window.get(), idx);
        return w == width && h == height && !px->empty();
    }

    nxrt::gfx::ValidationStats validation() const { return dev->validation_stats(); }

    void shutdown() {
        if (!dev) return;
        if (queue && ok) api.nvnQueueFinish(queue.get());
        nxrt::nvn::shutdown_backend();
        rend.reset();
        dev.reset();
        for (Tex*& t : std::vector<Tex*>{}) (void)t;
        window_textures.clear();
        for (auto* p : {pool_main.get(), pool_cmd.get(), pool_desc.get()})
            if (p) p->mem.release();
        ok = false;
    }
    ~App() { shutdown(); }
};

}  // namespace nvnapp
