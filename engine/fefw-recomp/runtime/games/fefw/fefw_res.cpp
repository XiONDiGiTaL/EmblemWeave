// SPDX-License-Identifier: GPL-3.0-or-later
// Fire Emblem: Fortune's Weave: output and render size (notes/resolution.md). Offsets are in main of update 131072.
//
// The game picks one of six render configs once at boot (0x1818350) and applies it once (0x64a660): DLSS output size, render size (half
// per side) and shadow-map size. The configs are built at 0xaa8f90 as std::shared_ptrs at 0x223e0b0 (16 bytes each); the data is the
// object + 0x18: output w/h (+0, +4), render w/h (+8, +0xc), shadow-map base size (+0x10), then quality factors and flags up to +0x7c.
// The selection calls nn::oe::GetCurrentGraphicsScale after the table is built and before it reads it, so the host's answer to that call
// rewrites the sizes first; the game then makes its targets, DLSS network, UI layers and jitter sequence (8 x ratio^2 phases) itself.
// The window textures are the graphics object's seed size (3840x2160, 0x18102a4), the cap for the output; the window crop is
// min(output, display size), so the vi answer must give 2160p for outputs above 1440p (the docked default does). For an ultrawide output
// wider than 3840 (5120x1440) the host raises the seed's width and the 2160p display width with it as it answers that question (below);
// an output stays that wide only once that is done.
//
// [fefw] output = console | 720p | 1080p | 1440p | 2160p | 4k | window | WxH     (console: each config keeps its own)
// [fefw] ultrawide = auto | off                                                   (auto: an output wider than 16:9 keeps its shape,
//                                                                                  fefw_wide.cpp; a narrower one gets 16:9 inside it)
// [fefw] render_ratio = 0 (the config's own, 0.5) | 0.25..1.0 per side            (1.0: DLAA)
// [fefw] render = WxH                                                              (wins over render_ratio)
// [fefw] shadow_size = 0 (the config's) | 512..8192                                (shadow-map base size)
// [fefw] quality = console | docked | handheld                                     (docked: config 0's quality fields in every config;
//                                                                                  handheld: config 3's, the lighter ones)
// [fefw] rt_pool_overflow_mb = 3072 | 0..3072                                       (room past the render-target pool, below)
//
// The render-target pool. Graphics init makes it at 0x646f88 (`mov w9, #0x3f000000`, flags 0xa1: CPU no access, GPU cached,
// compressible) as a pool object of 0x160 bytes (0x5d640): the NVNmemoryPool at +8 over 0x3f000000 bytes of the game's heap, and at +0x108
// an nn::mem::NumberLineAllocator in 256-byte units that is given the whole pool (AddRange(0, size / 256)). Every render target and the
// shadow maps are placed by it. 1008 MiB holds the console's sizes; 4096 or 8192 shadows (64 or 256 MiB each, five of them) or a 2160p
// output rendered above half per side do not fit: the allocator answers offset -1 and the targets get no storage (a black picture).
// The back end gives a texture placed past the end of a pool the CPU cannot access GPU memory of its own (nvnTextureInitialize, host-backed
// storage). So the host stands in front of that allocator (its SDK imports): when it finds no room, a room past the pool answers instead,
// rt_pool_overflow_mb from offset 1 GiB on (first fit; Free gives the place back). The allocator itself is left as it is: whatever fits in
// the pool is placed exactly where it was before. 0 = off: such targets get offset -1 (the back end still backs them, but texture objects over
// one of them cannot share its image), and above 1440p output the render is capped at half per side.
#include "fefw_game.h"

#include <algorithm>
#include <atomic>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <iterator>
#include <map>
#include <mutex>
#include <string>
#include <utility>

#include "nxrt/abi.h"
#include "nxrt/config.h"
#include "nxrt/dispatch.h"
#include "nxrt/log.h"
#include "nxrt/memory.h"
#include "nxrt/module.h"
#include "nxrt/nvn_backend.h"
#include "nxrt/platform.h"

namespace nxrt::games::fefw {

namespace {

constexpr const char* kTag = "fefw";
constexpr const char* kGetScale = "_ZN2nn2oe23GetCurrentGraphicsScaleEv";
constexpr const char* kViMaxRes = "_ZN2nn2vi31QueryDisplayBufferMaxResolutionEPNS0_19BufferMaxResolutionEPKNS0_7DisplayE";
constexpr uint64_t kConfigs = 0x223e0b0;   // 6 x std::shared_ptr<config>
constexpr int kConfigCount = 6;
// Configs 0 to 2 are the docked ones, 3 to 5 the handheld ones. The handheld ones differ from the docked in a few quality fields (+0x10
// shadow size 1536 for 2048, and lighter effect factors and flags): what the console gives up in handheld mode. [fefw] quality = handheld
// puts config 3's fields in every config, for a PC that wants the frame rate.
constexpr int kHandheldConfig = 3;
constexpr uint64_t kConfigBytes = 0x80;
constexpr uint64_t kShadow = 0x10;
constexpr int kMaxW = 3840, kMaxH = 2160;     // the game's own window textures and 2160p display size
constexpr int kWideMaxW = 7680;               // the widest an output wider than 16:9 is drawn at, with the window size raised
constexpr int64_t kWideRoomMb = 2048;         // the least room past the render-target pool for an output wider than 3840 (5120x2160 and
                                              // smaller); larger outputs need it in proportion to their pixels (7680x2160: 3072)
constexpr uint64_t kGraphicsPtr = 0x21e60d8;  // the graphics object (its pointer)
// The constructor's (0x6464b0) copies of the seed size, each {s32 w, h}: in the object the display, current render, current output,
// config render and config output sizes, and a global.
constexpr uint64_t kObjSizes[] = {0x33f0, 0x33f8, 0x3400, 0x3408, 0x3410};
constexpr uint64_t kSizeGlobal = 0x2243290;
constexpr uint64_t kDisplayW = 0x1abc2c0;     // s32[4]: the display width by nn::vi's BufferMaxResolution (3 = 2160p)
constexpr uint64_t kDisplayW2160 = kDisplayW + 3 * 4;

struct Res {
    uint64_t base = 0;
    uint64_t sdk_fn = 0;
    int out_w = 0, out_h = 0;      // 0: the config's own
    HleFn vi_answer = nullptr;     // the host's own QueryDisplayBufferMaxResolution (core/switch2.cpp)
    std::atomic<bool> window_raised{false};   // the window size was raised for an output wider than 3840
    std::atomic<bool> raise_failed{false};    // ... or cannot be: the output stays within 3840
    int render_w = 0, render_h = 0;
    double ratio = 0;              // 0: the config's own
    int shadow = 0;
    int quality_from = -1;         // the config whose quality fields every config gets: 0 (docked), kHandheldConfig, -1 (each its own)
    std::atomic<bool> logged{false};
};

Res& rs() {
    static Res r;
    return r;
}

// ---- the render-target pool's allocator ------------------------------------------------------------------------------------------
constexpr uint64_t kRtPoolSize = 0x3f000000;   // main 0x646f88
constexpr int kPoolCpuNoAccess = 0x1;           // NVN_MEMORY_POOL_FLAGS_CPU_NO_ACCESS
constexpr uint64_t kAllocatorAt = 0x108;        // in the pool object: the NumberLineAllocator ...
constexpr uint64_t kNvnPoolAt = 0x8;            // ... and the NVNmemoryPool it places into
constexpr uint64_t kUnit = 256;                 // the allocator's unit (0x5d8c0: size + 255 >> 8, the alignment >> 8)
constexpr int64_t kOverflowStart = (1ll << 30) / kUnit;   // the room past the pool starts at 1 GiB (16 MiB past the pool's end)
constexpr int64_t kOverflowMaxMb = 3072;        // up to 4 GiB of offsets in all

// An SDK function the hooks below stand in front of. Its address is looked up at the first call: the modules are not loaded at install.
struct SdkFn {
    const char* sym;
    std::atomic<uint64_t> addr{0};
};
SdkFn g_add_range{"_ZN2nn3mem19NumberLineAllocator8AddRangeEii"};          // (this, int first, int count)
SdkFn g_alloc3{"_ZN2nn3mem19NumberLineAllocator8AllocateEPiii"};           // bool (this, int* out, int count, int align)
SdkFn g_alloc2{"_ZN2nn3mem19NumberLineAllocator8AllocateEPii"};            // bool (this, int* out, int count)
SdkFn g_free{"_ZN2nn3mem19NumberLineAllocator4FreeEi"};                    // (this, int first)
SdkFn g_finalize{"_ZN2nn3mem19NumberLineAllocator8FinalizeEv"};            // (this)

void call_sdk(::Ctx* c, SdkFn& f) {
    uint64_t a = f.addr.load(std::memory_order_acquire);
    if (!a) {
        a = module_find_symbol(f.sym);
        if (!a || hle_is_thunk(a)) trap("nn::mem::NumberLineAllocator: the SDK's own function is not found", a);
        f.addr.store(a, std::memory_order_release);
    }
    const uint64_t lr = c->x[30], fn = c->fn;
    dispatch_call(c, a);
    c->x[30] = lr;
    c->fn = fn;
}

// The room past one render-target pool, in the allocator's units: what the pool's allocator cannot place goes here (first fit).
struct Room {
    std::map<int64_t, int64_t> free;   // start -> length
    std::map<int64_t, int64_t> used;   // start -> length
    int64_t in_use = 0;
};

struct RtPool {
    bool installed = false;
    int64_t room_units = 0;            // [fefw] rt_pool_overflow_mb in units (0: off)
    std::atomic<bool> extended{false};
    std::mutex mu;
    std::map<uint64_t, Room> rooms;    // the pool's NumberLineAllocator (guest address) -> its room
    uint64_t placed = 0;               // allocations placed in a room so far (logs)
};

RtPool& rp() {
    static RtPool p;
    return p;
}

int64_t room_take(Room& r, int64_t count, int64_t align) {
    for (auto it = r.free.begin(); it != r.free.end(); ++it) {
        const int64_t at = (it->first + align - 1) / align * align, end = it->first + it->second;
        if (at + count > end) continue;
        const int64_t s0 = it->first;
        r.free.erase(it);
        if (at > s0) r.free.emplace(s0, at - s0);
        if (at + count < end) r.free.emplace(at + count, end - (at + count));
        r.used.emplace(at, count);
        r.in_use += count;
        return at;
    }
    return -1;
}

bool room_give(Room& r, int64_t at) {
    auto u = r.used.find(at);
    if (u == r.used.end()) return false;
    int64_t s0 = at, len = u->second;
    r.in_use -= len;
    r.used.erase(u);
    auto next = r.free.lower_bound(s0);
    if (next != r.free.end() && next->first == s0 + len) { len += next->second; next = r.free.erase(next); }
    if (next != r.free.begin()) {
        auto prev = std::prev(next);
        if (prev->first + prev->second == s0) { s0 = prev->first; len += prev->second; r.free.erase(prev); }
    }
    r.free.emplace(s0, len);
    return true;
}

// AddRange(int first, int count): the SDK's. The render-target pool's allocator (the whole pool as its range, and the NVNmemoryPool 0x100
// bytes before it a live pool of that size the CPU cannot access) also gets a room past the pool.
void nla_add_range(::Ctx* c) {
    RtPool& p = rp();
    const uint64_t self = c->x[0];
    const int32_t first = static_cast<int32_t>(c->x[1]), count = static_cast<int32_t>(c->x[2]);
    call_sdk(c, g_add_range);
    if (!p.room_units || first != 0 || count <= 0 || static_cast<uint64_t>(count) * kUnit != kRtPoolSize || self < kAllocatorAt) return;
    const uint64_t guest_pool = self - kAllocatorAt + kNvnPoolAt;
    uint64_t size = 0;
    int flags = 0;
    if (!mem::is_mapped(guest_pool, 16) || !nvn::memory_pool_info(reinterpret_cast<const void*>(guest_pool), &size, &flags) || size != kRtPoolSize ||
        !(flags & kPoolCpuNoAccess)) {
        static std::atomic<bool> logged{false};
        if (!logged.exchange(true))
            NXLOG_W(kTag, "an allocator over 0x%llx bytes is not the render-target pool's (pool 0x%llx, size 0x%llx, flags 0x%x): no room past it",
                    static_cast<unsigned long long>(kRtPoolSize), static_cast<unsigned long long>(guest_pool), static_cast<unsigned long long>(size),
                    static_cast<unsigned>(flags));
        return;
    }
    {
        std::lock_guard<std::mutex> lk(p.mu);
        Room& r = p.rooms[self];
        r = Room();
        r.free.emplace(kOverflowStart, p.room_units);
    }
    if (!p.extended.exchange(true))
        NXLOG_I(kTag, "render-target pool (%llu MiB, flags 0x%x): what its allocator cannot place goes to offsets %llu to %llu MiB, with GPU memory of its "
                      "own ([fefw] rt_pool_overflow_mb = %lld)",
                static_cast<unsigned long long>(kRtPoolSize >> 20), static_cast<unsigned>(flags), static_cast<unsigned long long>(kOverflowStart * kUnit >> 20),
                static_cast<unsigned long long>((kOverflowStart + p.room_units) * kUnit >> 20), static_cast<long long>(p.room_units * kUnit >> 20));
}

// Allocate(int* out, int count[, int align]): the SDK's. When it finds no room in the render-target pool, the room past the pool answers.
void nla_allocate(::Ctx* c, SdkFn& f, bool aligned) {
    RtPool& p = rp();
    const uint64_t self = c->x[0], out = c->x[1];
    const int64_t count = static_cast<int32_t>(c->x[2]), align = aligned ? std::max<int64_t>(1, static_cast<int32_t>(c->x[3])) : 1;
    call_sdk(c, f);
    if ((c->x[0] & 1) || count <= 0 || !out) return;
    std::lock_guard<std::mutex> lk(p.mu);
    auto it = p.rooms.find(self);
    if (it == p.rooms.end()) return;
    const int64_t at = room_take(it->second, count, align);
    if (at < 0 || !mem::is_mapped(out, 4)) {
        if (at >= 0) room_give(it->second, at);
        static std::atomic<bool> logged{false};
        if (!logged.exchange(true))
            NXLOG_W(kTag, "render-target pool: %lld KiB do not fit in the pool or the room past it (%lld of %lld MiB in use): the game's allocator fails, "
                          "as it did before",
                    static_cast<long long>(count * kUnit >> 10), static_cast<long long>(it->second.in_use * kUnit >> 20),
                    static_cast<long long>(p.room_units * kUnit >> 20));
        return;
    }
    const int32_t v = static_cast<int32_t>(at);
    std::memcpy(reinterpret_cast<void*>(out), &v, 4);
    c->x[0] = 1;
    if (p.placed++ < 24)
        NXLOG_I(kTag, "render-target pool full: %lld KiB placed past it at offset 0x%llx (%lld MiB past the pool in use)", static_cast<long long>(count * kUnit >> 10),
                static_cast<unsigned long long>(at * kUnit), static_cast<long long>(it->second.in_use * kUnit >> 20));
}
void nla_allocate3(::Ctx* c) { nla_allocate(c, g_alloc3, true); }
void nla_allocate2(::Ctx* c) { nla_allocate(c, g_alloc2, false); }

// Free(int first): a place in the room goes back to it; the rest to the SDK's.
void nla_free(::Ctx* c) {
    RtPool& p = rp();
    const uint64_t self = c->x[0];
    const int64_t at = static_cast<int32_t>(c->x[1]);
    if (at >= kOverflowStart) {
        std::lock_guard<std::mutex> lk(p.mu);
        auto it = p.rooms.find(self);
        if (it != p.rooms.end() && room_give(it->second, at)) return;
    }
    call_sdk(c, g_free);
}

void nla_finalize(::Ctx* c) {
    RtPool& p = rp();
    {
        std::lock_guard<std::mutex> lk(p.mu);
        p.rooms.erase(c->x[0]);
    }
    call_sdk(c, g_finalize);
}

void rt_pool_install() {
    RtPool& p = rp();
    if (p.installed) return;
    p.installed = true;
    const int64_t mb = std::clamp<int64_t>(config_get_int("fefw", "rt_pool_overflow_mb", kOverflowMaxMb), 0, kOverflowMaxMb);
    if (mb <= 0) {
        NXLOG_I(kTag, "render-target pool: no room past it ([fefw] rt_pool_overflow_mb = 0)");
        return;
    }
    p.room_units = (mb << 20) / static_cast<int64_t>(kUnit);
    const std::pair<SdkFn*, HleFn> hooks[] = {{&g_add_range, nla_add_range}, {&g_alloc3, nla_allocate3}, {&g_alloc2, nla_allocate2},
                                              {&g_free, nla_free}, {&g_finalize, nla_finalize}};
    for (const auto& h : hooks) {
        hle_register(h.first->sym, h.second);
        module_override_symbol(h.first->sym);
    }
}

bool parse_wxh(const std::string& v, int* w, int* h) {
    int a = 0, b = 0;
    if (std::sscanf(v.c_str(), "%dx%d", &a, &b) != 2 || a < 64 || b < 64) return false;
    *w = a;
    *h = b;
    return true;
}

// The window's client size at start, fitted to 16:9; with [fefw] ultrawide a wider window is taken as it is (fefw_wide.cpp).
void window_output(int* w, int* h, bool wide) {
    int pw = 0, ph = 0;
    platform::pixel_size(&pw, &ph);
    if (pw <= 0 || ph <= 0) return;
    if (wide && wide_aspect(pw, ph)) {
        *w = pw & ~1;
        *h = ph & ~1;
        return;
    }
    int fw = std::min(pw, ph * 16 / 9);
    int fh = fw * 9 / 16;
    *w = fw & ~1;
    *h = fh & ~1;
}

// ---- ultrawide outputs wider than 3840 -------------------------------------------------------------------------------------------
// The graphics object's constructor copies the seed size into the object and a global. The window textures (0x648040), the buffers read
// from the global and the first output targets are made from those copies later in graphics init. Between the two, and before anything
// reads a copy, graphics init asks nn::vi for the display's largest buffer (0x1818150) and takes the display size from a table by the
// answer. So the host's answer is where the copies' width and the table's 2160p width are raised: no code of the game is hooked. The
// table must grow too, as the window crop takes the display size once the output is larger than it and would cut off the right side.
// The larger targets also need room past the render-target pool (5120x2160 at full size does not fit with 1024 MiB).

// [fefw] wide_textures = auto | off: off keeps every output within the game's own 3840 wide (scaled up by the window), as before.
std::string wide_textures_mode() {
    std::string v = config_get_str("fefw", "wide_textures", "auto");
    for (char& ch : v) ch = static_cast<char>(std::tolower(static_cast<unsigned char>(ch)));
    return v == "off" ? "off" : "auto";
}

// The room past the render-target pool the game's targets take for an output and a render size, as measured with an 8192 shadow map:
// about 400 MiB plus 100 MiB per million pixels of the window textures (the output's width x 2160) and the render together (5120x2160
// from 3840x1620 and 5120x1440 from itself need over 2048 MiB, 7680x2160 from itself over 3072; 7680x2160 from 5760x1620 and 5120x2160
// from itself fit in 3072; 7680x2160 from 5760x1620 is estimated at 2992, close to that). An estimate from those points.
int64_t rt_need_mb(int ow, int, int rw, int rh) {
    const int64_t px = static_cast<int64_t>(ow) * kMaxH + static_cast<int64_t>(rw) * rh;
    return 400 + (100 * px + 999999) / 1000000;
}

// An output wider than 3840 needs 2048 MiB at least, and room for it with a render of half per side (the smallest apply makes).
int64_t wide_room_mb(int w, int h) { return std::max<int64_t>(kWideRoomMb, rt_need_mb(w, h, (w + 1) / 2, (h + 1) / 2)); }

bool wide_room(int w, int h, int64_t room_mb) { return room_mb >= wide_room_mb(w, h); }
int64_t room_mb_now() { return rp().room_units * static_cast<int64_t>(kUnit) >> 20; }
bool wide_room(int w, int h) { return wide_room(w, h, room_mb_now()); }

// The output's shape fitted into max_w x 2160, even sizes (an odd custom size is made even too).
void fit_into(int* w, int* h, int max_w) {
    const double s = std::min({1.0, static_cast<double>(max_w) / *w, static_cast<double>(kMaxH) / *h});
    if (s >= 1.0) {
        *w &= ~1;
        *h &= ~1;
        return;
    }
    *w = 2 * static_cast<int>(std::lround(*w * s / 2));
    *h = 2 * static_cast<int>(std::lround(*h * s / 2));
}

// The widest a w x h output wider than 16:9 is drawn at: 7680 or 5120 where the room past the pool holds it, else the game's 3840.
// room_mb: the room past the render-target pool ([fefw] rt_pool_overflow_mb).
int wide_max_w(int w, int h, bool native, int64_t room_mb) {
    if (!native) return kMaxW;
    for (const int cap : {kWideMaxW, 5120}) {
        int fw = w, fh = h;
        fit_into(&fw, &fh, cap);
        if (fw > kMaxW && wide_room(fw, fh, room_mb)) return cap;
    }
    return kMaxW;
}
// This run: also 3840 once raising the window size has failed.
int wide_max_w(int w, int h, bool native) { return rs().raise_failed ? kMaxW : wide_max_w(w, h, native, room_mb_now()); }

bool is_seed_size(uint64_t at) {
    int32_t v[2] = {};
    if (!mem::is_mapped(at, sizeof v)) return false;
    std::memcpy(v, reinterpret_cast<const void*>(at), sizeof v);
    return v[0] == kMaxW && v[1] == kMaxH;
}

// Once, at graphics init's question. Everything is checked before anything is written.
void raise_window_size(Res& r, uint32_t answer) {
    if (r.window_raised || r.raise_failed) return;
    uint64_t obj = 0;
    if (mem::is_mapped(r.base + kGraphicsPtr, 8)) std::memcpy(&obj, reinterpret_cast<const void*>(r.base + kGraphicsPtr), 8);
    if (!obj) return;   // asked before the graphics object is there: not the question this answers
    if (answer != 3) {
        r.raise_failed = true;
        NXLOG_W(kTag, "ultrawide: an output wider than 3840 needs the 2160p display, and the game is told %u ([vi] buffer_max_resolution; "
                      "handheld mode gives 1)", answer);
        return;
    }
    bool expected = is_seed_size(r.base + kSizeGlobal);
    for (const uint64_t off : kObjSizes) expected = expected && is_seed_size(obj + off);
    const uint64_t table = r.base + kDisplayW2160;
    int32_t display = 0;
    if (mem::is_mapped(table, 4)) std::memcpy(&display, reinterpret_cast<const void*>(table), 4);
    if (!expected || display != kMaxW || !mem::protect(table, 4, mem::Prot::RW)) {
        r.raise_failed = true;
        NXLOG_W(kTag, "ultrawide: the window size cannot be raised (the game's sizes are not the expected %dx%d, 2160p display width %d)",
                kMaxW, kMaxH, display);
        return;
    }
    const int32_t w = r.out_w;
    std::memcpy(reinterpret_cast<void*>(table), &w, 4);
    if (!mem::protect(table, 4, mem::Prot::R)) NXLOG_W(kTag, "ultrawide: the display width table could not be made read-only again");
    std::memcpy(reinterpret_cast<void*>(r.base + kSizeGlobal), &w, 4);
    for (const uint64_t off : kObjSizes) std::memcpy(reinterpret_cast<void*>(obj + off), &w, 4);
    r.window_raised = true;
    NXLOG_I(kTag, "ultrawide: window textures and 2160p display size %dx%d (the game's own: %dx%d)", w, kMaxH, kMaxW, kMaxH);
}

// nn::vi::QueryDisplayBufferMaxResolution(BufferMaxResolution*, const Display*): the host's own answer, then the window size.
void vi_query_buffer_max_resolution(::Ctx* c) {
    Res& r = rs();
    const uint64_t out = c->x[0];
    r.vi_answer(c);
    uint32_t answer = 0;
    if (!out) return;
    std::memcpy(&answer, reinterpret_cast<const void*>(out), 4);   // the guest's out pointer, just written by the host's answer
    raise_window_size(r, answer);
}

void read_options(Res& r) {
    std::string o = config_get_str("fefw", "output", "console");
    for (char& ch : o) ch = static_cast<char>(std::tolower(static_cast<unsigned char>(ch)));
    const bool wide = wide_mode() != "off";
    if (o == "720p") r.out_w = 1280, r.out_h = 720;
    else if (o == "1080p") r.out_w = 1920, r.out_h = 1080;
    else if (o == "1440p") r.out_w = 2560, r.out_h = 1440;
    else if (o == "2160p" || o == "4k") r.out_w = 3840, r.out_h = 2160;
    else if (o == "window") window_output(&r.out_w, &r.out_h, wide);
    else if (o != "console" && !o.empty() && !parse_wxh(o, &r.out_w, &r.out_h)) NXLOG_W(kTag, "[fefw] output = %s is not known: console", o.c_str());
    if (wide && r.out_w > 0 && static_cast<double>(r.out_w) * 9.0 * 1.01 < static_cast<double>(r.out_h) * 16.0) {
        // narrower than 16:9 (16:10, 4:3): the game's 16:9 picture inside it, letterboxed by the window
        const int h0 = r.out_h;
        r.out_h = (r.out_w * 9 / 16) & ~1;
        NXLOG_I(kTag, "[fefw] output %dx%d is narrower than 16:9: %dx%d, letterboxed ([fefw] ultrawide = auto)", r.out_w, h0, r.out_w, r.out_h);
    }
    const int w0 = r.out_w, h0 = r.out_h;
    res_fit_output(&r.out_w, &r.out_h, wide);
    if ((r.out_w != w0 || r.out_h != h0) && r.out_w == (w0 & ~1) && r.out_h == (h0 & ~1)) {
        NXLOG_I(kTag, "[fefw] output %dx%d made even: %dx%d", w0, h0, r.out_w, r.out_h);
    } else if (r.out_w != w0 || r.out_h != h0) {
        if (wide && wide_aspect(w0, h0)) {
            const bool off = wide_textures_mode() == "off";
            int fw = w0, fh = h0;
            fit_into(&fw, &fh, kWideMaxW);
            char why[80] = "";
            if (off) std::snprintf(why, sizeof why, " ([fefw] wide_textures = off)");
            else if (!wide_room(fw, fh))
                std::snprintf(why, sizeof why, " ([fefw] rt_pool_overflow_mb is below %lld)", static_cast<long long>(wide_room_mb(fw, fh)));
            NXLOG_W(kTag, "[fefw] output %dx%d is above %dx%d%s: %dx%d, the same shape (the window scales it)", w0, h0,
                    wide_max_w(w0, h0, !off), kMaxH, why, r.out_w, r.out_h);
        } else
            NXLOG_W(kTag, "[fefw] output %dx%d is above the game's 3840x2160 window textures: %dx%d", w0, h0, r.out_w, r.out_h);
    }
    double ratio = config_get_double("fefw", "render_ratio", 0.0);
    for (const double f : {1.0 / 3.0, 2.0 / 3.0})   // 0.33 / 0.67 mean thirds (1440p -> 1706x960, as the console's config 2)
        if (std::fabs(ratio - f) < 0.006) ratio = f;
    if (ratio > 0) r.ratio = std::clamp(ratio, 0.25, 1.0);
    parse_wxh(config_get_str("fefw", "render", ""), &r.render_w, &r.render_h);
    const int64_t sh = config_get_int("fefw", "shadow_size", 0);
    if (sh > 0) r.shadow = static_cast<int>(std::clamp<int64_t>(sh, 512, 8192));
    const std::string q = config_get_str("fefw", "quality", "console");
    r.quality_from = q == "docked" ? 0 : q == "handheld" ? kHandheldConfig : -1;
}

bool any_option(const Res& r) { return r.out_w > 0 || r.ratio > 0 || r.render_w > 0 || r.shadow > 0 || r.quality_from >= 0; }

const char* quality_name(const Res& r) { return r.quality_from == 0 ? "docked" : r.quality_from == kHandheldConfig ? "handheld" : "console"; }

uint64_t config_data(const Res& r, int i) {
    uint64_t p = 0;
    const uint64_t slot = r.base + kConfigs + 16ull * i;
    if (!mem::is_mapped(slot, 8)) return 0;
    std::memcpy(&p, reinterpret_cast<const void*>(slot), 8);
    return p && mem::is_mapped(p, kConfigBytes) ? p : 0;
}

// Rewrites the six configs (idempotent: every call gives the same values).
void apply(Res& r) {
    uint64_t cfg[kConfigCount];
    for (int i = 0; i < kConfigCount; ++i)
        if (!(cfg[i] = config_data(r, i))) return;   // not built yet
    // An output wider than 3840 needs the raised window size. Graphics init (0x180fe60) asks for the display size (0x18104ac) before it
    // first reads the configs (0x1813ff0), so a size not raised by now never will be.
    if (r.out_w > kMaxW && !r.window_raised) {
        const int w0 = r.out_w, h0 = r.out_h;
        r.raise_failed = true;
        res_fit_output(&r.out_w, &r.out_h, true);
        wide_output_fitted(r.out_w, r.out_h);
        NXLOG_W(kTag, "[fefw] output %dx%d: the window size was not raised: %dx%d, the same shape", w0, h0, r.out_w, r.out_h);
    }
    const bool first = !r.logged.exchange(true);
    if (first && r.window_raised && !rp().extended.load())
        NXLOG_W(kTag, "ultrawide: the window size was raised to %d but the render-target pool has no room past it: its larger targets may "
                      "not fit (a black picture); set [fefw] wide_textures = off if so", r.out_w);
    for (int i = 0; i < kConfigCount; ++i) {
        int32_t v[5];
        std::memcpy(v, reinterpret_cast<const void*>(cfg[i]), sizeof v);
        if (r.quality_from >= 0 && i != r.quality_from)
            std::memcpy(reinterpret_cast<void*>(cfg[i] + kShadow), reinterpret_cast<const void*>(cfg[r.quality_from] + kShadow), kConfigBytes - kShadow);
        int32_t ow = r.out_w > 0 ? r.out_w : v[0], oh = r.out_h > 0 ? r.out_h : v[1];
        int32_t rw = v[2], rh = v[3];
        if (r.render_w > 0) rw = r.render_w, rh = r.render_h;
        else if (r.ratio > 0)   // even sizes: the SSAO pass works on 2x2 blocks
            rw = 2 * static_cast<int32_t>(std::lround(ow * r.ratio / 2)), rh = 2 * static_cast<int32_t>(std::lround(oh * r.ratio / 2));
        else if (r.out_w > 0) rw = (ow + 1) / 2, rh = (oh + 1) / 2;
        rw = std::clamp(rw, 64, ow);
        rh = std::clamp(rh, 64, oh);
        // The game's render targets live in one 0x3f000000-byte memory pool (made at 0x646f88). Above 1440p output it holds a render of at
        // most half the output per side: 3840x2160 from 2560x1440 runs out and the game's allocator gives the HUD and post targets
        // offset -1 (a black picture). 2560x1440 at full size fits. With room past the pool (rt_pool_install) the rest goes there.
        if (!rp().extended.load() && static_cast<int64_t>(ow) * oh > 2560 * 1440 && (rw > (ow + 1) / 2 || rh > (oh + 1) / 2)) {
            if (first)
                NXLOG_W(kTag, "render %dx%d for output %dx%d does not fit the game's render-target pool: %dx%d", rw, rh, ow, oh, (ow + 1) / 2,
                        (oh + 1) / 2);
            rw = (ow + 1) / 2;
            rh = (oh + 1) / 2;
        }
        // An output wider than 3840 with a render that does not fit the room past the pool (5120x2160 at 100 % with 2048 MiB, 7680x2160
        // at 100 % with 3072): 3/4 per side, then half, whichever first fits (res_fit_render).
        if (r.window_raised) {
            const int32_t w0 = rw, h0 = rh;
            res_fit_render(ow, oh, &rw, &rh, room_mb_now());
            if ((rw != w0 || rh != h0) && first && i == 0)
                NXLOG_W(kTag, "render %dx%d for output %dx%d does not fit the room past the render-target pool (%lld MiB): %dx%d", w0, h0, ow,
                        oh, static_cast<long long>(room_mb_now()), rw, rh);
        }
        const int32_t sizes[4] = {ow, oh, rw, rh};
        std::memcpy(reinterpret_cast<void*>(cfg[i]), sizes, sizeof sizes);
        if (r.shadow > 0) std::memcpy(reinterpret_cast<void*>(cfg[i] + kShadow), &r.shadow, 4);
        int32_t shadow = 0;
        std::memcpy(&shadow, reinterpret_cast<const void*>(cfg[i] + kShadow), 4);
        if (first)
            NXLOG_I(kTag, "render config %d: output %dx%d from %dx%d, shadow %d (was %dx%d from %dx%d, shadow %d)%s", i, ow, oh, rw, rh, shadow, v[0],
                    v[1], v[2], v[3], v[4], r.quality_from >= 0 && i != r.quality_from ? (r.quality_from == 0 ? ", config 0's quality" : ", the handheld config's quality") : "");
    }
}

// nn::oe::GetCurrentGraphicsScale(): the SDK's own answer, after the configs are rewritten.
void get_current_graphics_scale(::Ctx* c) {
    Res& r = rs();
    apply(r);
    if (!r.sdk_fn) {
        const int64_t v = config_get_int("am", "graphics_scale", 10);
        c->x[0] = v == 20 ? 1 : v == 30 ? 2 : 0;
        return;
    }
    const uint64_t lr = c->x[30], fn = c->fn;
    dispatch_call(c, r.sdk_fn);
    c->x[30] = lr;
    c->fn = fn;
}

}  // namespace

bool res_rt_pool_extended() { return rp().extended.load(); }

void res_fit_render(int ow, int oh, int* rw, int* rh, int64_t room_mb) {
    if (ow <= kMaxW || rt_need_mb(ow, oh, *rw, *rh) <= room_mb) return;
    for (const int q : {3, 2}) {   // 3/4, then 1/2 per side
        const int cw = 2 * (ow * q / 8), ch = 2 * (oh * q / 8);
        if (*rw > cw || *rh > ch) {
            *rw = std::min(*rw, cw);
            *rh = std::min(*rh, ch);
        }
        if (rt_need_mb(ow, oh, *rw, *rh) <= room_mb) return;
    }
}

void res_fit_output(int* w, int* h, bool wide) {
    if (!wide || !wide_aspect(*w, *h)) {
        *w = std::min(*w, kMaxW);
        *h = std::min(*h, kMaxH);
        return;
    }
    fit_into(w, h, wide_max_w(*w, *h, wide_textures_mode() != "off"));
}

void res_fit_output_next(int* w, int* h, bool wide, bool native, int64_t room_mb, int64_t vi_buffer_max) {
    if (!wide || !wide_aspect(*w, *h)) {
        *w = std::min(*w, kMaxW);
        *h = std::min(*h, kMaxH);
        return;
    }
    const bool vi_2160p = vi_buffer_max < 0 || vi_buffer_max > 2;   // [vi] buffer_max_resolution unset or 2160p (handheld can't be known)
    fit_into(w, h, wide_max_w(*w, *h, native && vi_2160p, room_mb));
}

void res_install(uint64_t base) {
    Res& r = rs();
    if (r.base) return;
    r.base = base;
    rt_pool_install();
    read_options(r);
    if (r.out_w > kMaxW) {   // only an ultrawide output gets this far
        if ((r.vi_answer = hle_lookup(kViMaxRes))) hle_register(kViMaxRes, vi_query_buffer_max_resolution);   // the host's (core/switch2.cpp)
        else {
            r.raise_failed = true;
            NXLOG_W(kTag, "ultrawide: the host has no QueryDisplayBufferMaxResolution to follow: outputs stay within 3840 wide");
        }
    }
    if (wide_mode() != "off") wide_install(base, r.out_w, r.out_h);   // only an output wider than 16:9 installs anything
    if (!any_option(r)) return;
    r.sdk_fn = module_find_symbol(kGetScale);
    hle_register(kGetScale, get_current_graphics_scale);
    module_override_symbol(kGetScale);
    NXLOG_I(kTag, "render size: output %s, render %s, shadow %s%s", r.out_w > 0 ? (std::to_string(r.out_w) + "x" + std::to_string(r.out_h)).c_str() : "console",
            r.render_w > 0 ? (std::to_string(r.render_w) + "x" + std::to_string(r.render_h)).c_str()
            : r.ratio > 0  ? ("x" + std::to_string(r.ratio)).c_str()
                           : "console (half)",
            r.shadow > 0 ? std::to_string(r.shadow).c_str() : "console", r.quality_from >= 0 ? (std::string(", ") + quality_name(r) + " quality").c_str() : "");
}

}  // namespace nxrt::games::fefw
