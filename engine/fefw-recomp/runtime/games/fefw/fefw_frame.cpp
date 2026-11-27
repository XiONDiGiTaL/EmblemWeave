// SPDX-License-Identifier: GPL-3.0-or-later
// Fire Emblem: Fortune's Weave: per-frame amounts kept at their real speed at 60 and 120 fps ([fefw] frame_fixes). Offsets are in
// main of update 131072 (build DB93B012...); the sites are in recomp/games/fefw/main.sites.
//
// Gameplay was made for 30 fps. [fefw] fps60 / fps120 raise the game's target (fefw_play.cpp) and its time manager then steps the
// game clock by real time, but many systems add a fixed amount per frame (a 1/30 s step, a +0.1, a x0.95 easing): at 60 they run twice as
// fast and at 120 four times. The site list and the scaling rule come from Fortune's Eden (TheSneakyBadger, GPL-3.0-or-later; its map started
// from Jbuck7's 60 FPS cheat), which patches the same main: technique only, this code is our own. All 57 of its sites were checked to hold
// the same instructions in our main.
//
// The rule, with s = the frame's share of a 1/30 s step (0.5 at 60 fps, 0.25 at 120): per-frame amounts x s, accelerations x s^2, per-frame
// factors q -> q^s (an ease k -> 1 - (1 - k)^s), frame counts / s. s comes from the step the time manager took this frame (fefw_play.cpp
// hook_step_choice): s = step x native / 60, where native is the target the game itself asked for (30 in gameplay, 60 on the title and menus,
// which run at 60 on the console too and keep s = 1). Frame counts (the illusion clone period, the marquee delay) use native / target, so they
// do not jitter. s is kept within [1/8, 1]: a slow frame does not speed these systems past their console behaviour.
//
// Each hook sits after the instruction that makes the game's value and changes the register in place; the movk sites first check the register
// holds the game's constant. The cutscene quake block (evt::demo::Seq, 0x11e9b50) draws a new random offset each frame, so it runs on 30 Hz
// ticks only: a midhook at its start (0x11ea5cc) runs the function's rest from the block's end (0x11eb084, also a midhook site, which gives
// the lifted f_11eb084) on the other frames. Spoken text the same way: the talk sequence (0x1154754) asks the text step (0x115d330, an
// entry hook) for 3 more letters each frame; off the 30 Hz ticks the entry hook answers "not finished" (0) without running it, as
// Fortune's Eden v1.6.8.6 did. Showing the whole line (a button press asks for more than 3) is left alone.
//
// Cloth gravity and wind ([fefw] cloth_gravity): each cloth point is moved by (velocity kept + gravity + wind) and then turned a share k
// of the way back to its parent bone's direction. That turn acts on the position, so where a point settles is about gravity x (1 - k) / k.
// With k -> 1 - (1 - k)^s (about k x s) and gravity x s^2 (`accel`, the default), a point settles s times as far from its rest pose as
// at 30 fps: half the sag at 60, a quarter at 120. Hair whose rest pose points up (a high ponytail) then stands up. `match` (opt-in)
// scales gravity and wind by F = Q (1 - Q^s) / ((1 - Q) Q^s), Q = 1 - k, which keeps the settled position of the 30 fps step for every k
// (F is about s for a weak turn and tends to s^2 only as k -> 0, where nothing holds the point and gravity is a plain acceleration).
//
// [fefw] frame_fixes = 1 (default; the menu changes it live) and [fefw] frame_fixes_skip = a comma list of groups to leave alone: dt, shader,
// physics, cloth, quake, anim, map, ui, text. At 30 fps every hook leaves the game's values as they are.
#include "fefw_game.h"
#include "fefw_cloth.h"

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <string>
#include <unordered_map>

#include "nxrt/config.h"
#include "nxrt/dispatch.h"
#include "nxrt/hooks.h"
#include "nxrt/log.h"
#include "nxrt/memory.h"
#include "nxrt/services_api.h"

namespace nxrt::games::fefw {
namespace {

constexpr const char* kTag = "fefw";

enum Group : uint8_t { kDt, kShader, kPhysics, kCloth, kQuake, kAnim, kMap, kUi, kText, kGroups };
constexpr const char* kGroupNames[kGroups] = {"dt", "shader", "physics", "cloth", "quake", "anim", "map", "ui", "text"};

// How a movk site's constant becomes the frame's value.
enum class Rule : uint8_t {
    Mul,     // x s: an amount per frame
    Mul2,    // x s^2: an acceleration per frame
    Pow,     // q^s: a factor per frame
    Ease,    // 1 - (1 - k)^s: an ease / lerp weight per frame
    Div,     // / s: frames to hold
    Force,   // the cloth step's gravity: x s^2 ([fefw] cloth_gravity = accel) or the factor that keeps the settled position (match)
};

struct MovkSite {
    uint32_t off;        // the movk; the hook is at the next instruction
    uint8_t reg;
    uint32_t bits;       // the 32-bit constant mov + movk made
    Rule rule;
    Group group;
};

constexpr MovkSite kMovk[] = {
    // the per-frame dt fallback 1/30 (0x3d08ab86)
    {0x107ead8, 9, 0x3d08ab86, Rule::Mul, kDt}, {0x1080498, 8, 0x3d08ab86, Rule::Mul, kDt}, {0x10804c8, 9, 0x3d08ab86, Rule::Mul, kDt},
    {0x1084ee4, 8, 0x3d08ab86, Rule::Mul, kDt}, {0x1087654, 8, 0x3d08ab86, Rule::Mul, kDt}, {0x1089590, 8, 0x3d08ab86, Rule::Mul, kDt},
    {0x10896a4, 8, 0x3d08ab86, Rule::Mul, kDt}, {0x1089844, 8, 0x3d08ab86, Rule::Mul, kDt}, {0x1089a60, 9, 0x3d08ab86, Rule::Mul, kDt},
    {0x10ad644, 8, 0x3d08ab86, Rule::Mul, kDt}, {0x1121a5c, 11, 0x3d08ab86, Rule::Mul, kDt}, {0x1121ab0, 8, 0x3d08ab86, Rule::Mul, kDt},
    {0x11271d8, 9, 0x3d08ab86, Rule::Mul, kDt}, {0x1128580, 9, 0x3d08ab86, Rule::Mul, kDt}, {0x1129e04, 9, 0x3d08ab86, Rule::Mul, kDt},
    {0x113ed6c, 10, 0x3d08ab86, Rule::Mul, kDt}, {0x113edb4, 8, 0x3d08ab86, Rule::Mul, kDt},
    {0x0ab248c, 8, 0x3d08ab85, Rule::Mul, kShader},     // shader time per frame
    {0x0da1f48, 9, 0x3d088889, Rule::Mul, kPhysics},    // PhysX async step 1/30
    {0x14cfdbc, 8, 0x3d088889, Rule::Mul, kPhysics},    // PhysX sync step
    {0x0a687e4, 8, 0xbc888889, Rule::Force, kCloth},    // cloth gravity per step
    {0x112bfc8, 9, 0x3f733333, Rule::Pow, kAnim},       // CustomModifier_Auto x0.95 per frame
    {0x112c010, 9, 0x3f733333, Rule::Pow, kAnim},
    {0x11e62c0, 9, 0x3f4ccccd, Rule::Div, kAnim},       // CharacterBuilder hold frames
    {0x133fd18, 8, 0x3ecccccd, Rule::Ease, kMap},       // map marker glide lerp 0.4
    {0x159b814, 8, 0x3dcccccd, Rule::Mul, kUi},         // ProcHeader +0.1
    {0x16665e8, 8, 0x41855555, Rule::Mul, kUi},         // UnitMake timers +16.667 ms per frame
    {0x17cc0e8, 8, 0x41855555, Rule::Mul, kUi}, {0x17cea74, 8, 0x41855555, Rule::Mul, kUi}, {0x17cee40, 8, 0x41855555, Rule::Mul, kUi},
    {0x16a8ef8, 8, 0x3dcccccd, Rule::Mul, kUi},         // BGM marquee +0.1 px per frame
    {0x16a8f40, 8, 0x3dcccccd, Rule::Mul, kUi},
    {0x17cf378, 10, 0xbf19999a, Rule::Mul, kUi},        // avatar turntable acceleration
    {0x17cf474, 10, 0xbf19999a, Rule::Mul, kUi},
};

// The other sites (the hook address; what each does is at its function below).
constexpr uint32_t kClothPhase = 0xa686ec, kClothBlend = 0xa68738, kClothDamp = 0xa687f4, kClothWind = 0xa68994, kClothPull = 0xa68a9c;
constexpr uint32_t kQuakeStart = 0x11ea5cc, kQuakeEnd = 0x11eb084;
constexpr uint32_t kTextStep = 0x115d330;           // entry hook: (x0 = the talk window's text, w1 = letters to add) -> w0 finished
constexpr uint32_t kIllusion = 0x112291c, kLookAt = 0x10d7324;
constexpr uint32_t kMapAngle = 0x133fb94, kMapHeight = 0x133feb8, kMapTimer = 0xf3a968;
constexpr uint32_t kSubsteps = 0x14cf5dc;
constexpr uint32_t kMarqueeSpeed = 0x16a8d6c, kMarqueeDelay = 0x16a8d78, kMarqueeLerp = 0x16a8ef8;
constexpr uint32_t kTurnRelease[] = {0x17cf3fc, 0x17cf444, 0x17cf540, 0x17cf580};   // after `fmov s2, #6.0`
constexpr uint32_t kTurnAngle[] = {0x17cf418, 0x17cf498, 0x17cf55c, 0x17cf5a4};     // after `ldr s1, [x19, #0xd4]`
// The game's own target stores into the graphics object (+0x1dc): gameplay 30, the title and menus 60 (`mov w9, #...` before each).
constexpr uint32_t kStoreGameplay = 0x7bf1c0, kStoreMenuA = 0x652960, kStoreMenuB = 0x66af3c;

struct State {
    uint64_t base = 0x7100000000ull;
    bool installed = false;
    std::atomic<bool> on{true};
    uint32_t skip = 0;                          // groups left alone (bit per Group)
    std::atomic<int> native{0};                 // the target the game asked for last (0: none seen)
    std::atomic<float> s{1.0f};                 // the frame's share of a 1/30 s step
    std::atomic<float> s_count{1.0f};           // native / target, for frame counts
    std::atomic<bool> tick{true};               // a 30 Hz step falls in this frame
    std::atomic<bool> cloth_match{false};       // [fefw] cloth_gravity = match (default: accel)
    std::atomic<bool> cloth_logged{false};
    float tick_acc = 0.0f;                      // main thread (frame_step)
    std::atomic<uint64_t> applied[kGroups] = {};
    std::atomic<uint64_t> quake_held{0};
    std::atomic<void (*)(::Ctx*)> quake_rest{nullptr};   // f_11eb084, looked up at the first quake (the modules register after play mode starts)
    std::atomic<int> quake_state{0};            // 0 not looked up, 1 found, -1 missing
};

State& st() {
    static State s;
    return s;
}

// The scale for a group, or 1 when the group is off or the frame is a whole 30 fps step.
inline float scale(Group g) {
    State& s = st();
    if (!s.on.load(std::memory_order_relaxed) || (s.skip >> g & 1u)) return 1.0f;
    return s.s.load(std::memory_order_relaxed);
}
inline float count_scale(Group g) {
    State& s = st();
    if (!s.on.load(std::memory_order_relaxed) || (s.skip >> g & 1u)) return 1.0f;
    return s.s_count.load(std::memory_order_relaxed);
}
inline bool full(float s) { return s >= 0.999f; }
inline void count(Group g) { st().applied[g].fetch_add(1, std::memory_order_relaxed); }

inline float bits_f(uint32_t b) {
    float f;
    std::memcpy(&f, &b, 4);
    return f;
}
inline uint32_t f_bits(float f) {
    uint32_t b;
    std::memcpy(&b, &f, 4);
    return b;
}
// q^s with a small per-thread cache: the cloth hooks run for every cloth module every frame (about 500,000 calls a second at 120 fps), with
// a handful of distinct factors and one s per frame
float pow_cached(float q, float s) {
    struct Entry { float q = -1.0f, s = -1.0f, r = 0.0f; };
    thread_local Entry cache[16];
    uint32_t qb;
    std::memcpy(&qb, &q, 4);
    Entry& e = cache[(qb ^ (qb >> 7) ^ (qb >> 15)) & 15];
    if (e.q != q || e.s != s) e = Entry{q, s, std::pow(q, s)};
    return e.r;
}
inline float ease(float k, float s) { return 1.0f - pow_cached(std::clamp(1.0f - k, 0.0f, 1.0f), s); }

// What the cloth step's gravity and wind are multiplied by for a point turned back by k each step (see the top of the file). The gravity
// site sets it for the point; the wind site, later in the same pass of the loop, uses it.
thread_local float t_cloth_force = 1.0f;
float cloth_force(::Ctx* c, float s) {
    State& st_ = st();
    const float k = c->v[5].f[0];      // s5: the point's turn back to its parent's direction (its own or the module's)
    float f = s * s;
    if (st_.cloth_match.load(std::memory_order_relaxed) && k > 1e-4f && k < 1.0f) {
        const float q = 1.0f - k;
        f = cloth_force_factor(k, s, pow_cached(q, s));
    }
    if (!st_.cloth_logged.load(std::memory_order_relaxed) && !st_.cloth_logged.exchange(true)) {
        float step = 0.0f;             // sp + 0x90: min(the step's argument, 1), which the whole move is multiplied by
        if (c->x[31] >= 0x10000) std::memcpy(&step, reinterpret_cast<const void*>(c->x[31] + 0x90), 4);
        NXLOG_I(kTag, "frame-rate fixes: cloth gravity ([fefw] cloth_gravity = %s): first point: s %.3f, turn back k %.4f, the step's own "
                      "factor %.3f, gravity and wind x %.4f (accel would be %.4f)",
                st_.cloth_match.load() ? "match" : "accel", s, k, step, f, s * s);
    }
    t_cloth_force = f;
    return f;
}

void hook_movk(::Ctx* c, uint64_t, void* user) {
    const MovkSite& m = *static_cast<const MovkSite*>(user);
    const float s = scale(m.group);
    if (full(s) || static_cast<uint32_t>(c->x[m.reg]) != m.bits) return;
    const float v = bits_f(m.bits);
    float r = v;
    switch (m.rule) {
        case Rule::Mul: r = v * s; break;
        case Rule::Mul2: r = v * s * s; break;
        case Rule::Pow: r = std::pow(v, s); break;
        case Rule::Ease: r = ease(v, s); break;
        case Rule::Div: r = v / s; break;
        case Rule::Force: r = v * cloth_force(c, s); break;
    }
    c->x[m.reg] = f_bits(r);      // as the movk left it: the upper half zero
    count(m.group);
}

// ---- cloth (wld::ClothModule's Verlet step 0xa68640, capes and hair; once per frame, made for 1/30 s) -------------------------------------
// the wind phase: s0 = +-min(dt, 1) / 60 just divided out, added to the module's phase (+0x8c)
void hook_cloth_phase(::Ctx* c, uint64_t, void*) {
    const float s = scale(kCloth);
    if (full(s)) return;
    c->v[0].f[0] *= s;
    count(kCloth);
}
// the pull towards the animation: s1 = 1 - k (k = the argument's first lane, kept at sp + 0x50 and read again from there) -> (1 - k)^s
void hook_cloth_blend(::Ctx* c, uint64_t, void*) {
    const float s = scale(kCloth);
    const float q = c->v[1].f[0];
    if (full(s) || !(q >= 0.0f && q <= 1.0f)) return;
    const float qs = pow_cached(q, s), k = 1.0f - qs;
    c->v[1].f[0] = qs;
    if (mem::is_mapped(c->x[31] + 0x50, 4)) std::memcpy(reinterpret_cast<void*>(c->x[31] + 0x50), &k, 4);
    count(kCloth);
}
// the velocity kept per step: s2 = 1 - damping -> ^s
void hook_cloth_damp(::Ctx* c, uint64_t, void*) {
    const float s = scale(kCloth);
    const float q = c->v[2].f[0];
    if (full(s) || !(q >= 0.0f && q <= 1.0f)) return;
    c->v[2].f[0] = pow_cached(q, s);
    count(kCloth);
}
// the wind force added this step (v0, four lanes): scaled like the gravity of the same point
void hook_cloth_wind(::Ctx* c, uint64_t, void*) {
    const float s = scale(kCloth);
    if (full(s)) return;
    const float k = st().cloth_match.load(std::memory_order_relaxed) ? t_cloth_force : s * s;
    for (float& f : c->v[0].f) f *= k;
    count(kCloth);
}
// the pull towards the parent bone's direction: s2 = 1 - k (s5 = k) -> s2 = (1 - k)^s, s5 = 1 - that
void hook_cloth_pull(::Ctx* c, uint64_t, void*) {
    const float s = scale(kCloth);
    const float q = c->v[2].f[0];
    if (full(s) || !(q >= 0.0f && q <= 1.0f)) return;
    const float qs = pow_cached(q, s);
    c->v[2].f[0] = qs;
    c->v[5].f[0] = 1.0f - qs;
    count(kCloth);
}

// ---- cutscene quake (evt::demo::Seq 0x11e9b50): the block from 0x11ea5cc to 0x11eb084 runs on 30 Hz ticks -----------------------------------
int hook_quake(::Ctx* c, uint64_t, void*) {
    State& s = st();
    if (full(scale(kQuake)) || s.tick.load(std::memory_order_relaxed)) return 0;
    int qs = s.quake_state.load(std::memory_order_acquire);
    if (qs == 0) {
        auto fn = reinterpret_cast<void (*)(::Ctx*)>(fn_lookup(s.base + kQuakeEnd));
        s.quake_rest.store(fn, std::memory_order_relaxed);
        qs = fn ? 1 : -1;
        s.quake_state.store(qs, std::memory_order_release);
        if (fn) NXLOG_I(kTag, "frame-rate fixes: the cutscene quake runs on 30 Hz ticks (f_11eb084 found)");
        else NXLOG_W(kTag, "frame-rate fixes: no f_11eb084 in this build (main.sites midhook 0x71011eb084): the cutscene quake runs every frame");
    }
    if (qs < 0) return 0;
    s.quake_rest.load(std::memory_order_relaxed)(c);          // the function from the block's end, as when the block has nothing to do; it returns from the function
    s.quake_held.fetch_add(1, std::memory_order_relaxed);
    count(kQuake);
    return 1;
}

// ---- spoken text: 3 letters a frame, on 30 Hz ticks only -------------------------------------------------------------------------------
int hook_text_step(::Ctx* c, uint64_t, void*) {
    if (static_cast<uint32_t>(c->x[1]) != 3 || full(scale(kText)) || st().tick.load(std::memory_order_relaxed)) return 0;
    c->x[0] = 0;              // not finished: the line goes on typing at the next tick
    count(kText);
    return 1;
}

// ---- animation, map, UI ----------------------------------------------------------------------------------------------------------------
// CharacterIllusion: clone index = frame / period (w12, 30 fps frames) -> period / (native / target)
void hook_illusion(::Ctx* c, uint64_t, void*) {
    const float s = count_scale(kAnim);
    const int32_t p = static_cast<int32_t>(c->x[12]);
    if (full(s) || p <= 0) return;
    c->x[12] = static_cast<uint32_t>(std::max<int32_t>(1, static_cast<int32_t>(std::lround(static_cast<float>(p) / s))));
    count(kAnim);
}
// CharacterLookAt: the weight blends towards its target by s2 = k per frame (s0 += (s1 - s0) * k)
void hook_look_at(::Ctx* c, uint64_t, void*) {
    const float s = scale(kAnim);
    const float k = c->v[2].f[0];
    if (full(s) || !(k > 0.0f && k <= 1.0f)) return;
    c->v[2].f[0] = ease(k, s);
    count(kAnim);
}
// map UI angle: s1 = 3 degrees per frame
void hook_map_angle(::Ctx* c, uint64_t, void*) {
    const float s = scale(kMap);
    if (full(s) || c->v[1].f[0] != 3.0f) return;
    c->v[1].f[0] = 3.0f * s;
    count(kMap);
}
// map marker height: s0 = (target - height) * k (s2 = k, 0.5) was just made: the same with the eased k
void hook_map_height(::Ctx* c, uint64_t, void*) {
    const float s = scale(kMap);
    const float k = c->v[2].f[0];
    if (full(s) || !(k > 0.0f && k < 1.0f)) return;
    c->v[0].f[0] *= ease(k, s) / k;
    count(kMap);
}
// map timer: s0 = 60 / s1 with s1 = 60 / target is about to give the target; with s1 / (native / target) it gives the native rate
void hook_map_timer(::Ctx* c, uint64_t, void*) {
    const float s = count_scale(kMap);
    if (full(s) || !(c->v[1].f[0] > 0.0f)) return;
    c->v[1].f[0] /= s;
    count(kMap);
}
// PhysX substeps of the frame's step: w8 = 2, w9 = 4 (when sped up) -> about 1/60 s each
void hook_substeps(::Ctx* c, uint64_t, void*) {
    const float s = scale(kPhysics);
    if (full(s) || static_cast<uint32_t>(c->x[8]) != 2 || static_cast<uint32_t>(c->x[9]) != 4) return;
    c->x[8] = static_cast<uint32_t>(std::max(1L, std::lround(2.0f * s)));
    c->x[9] = static_cast<uint32_t>(std::max(1L, std::lround(4.0f * s)));
    count(kPhysics);
}
// BGM option marquee: speed s11 px per frame (after its fcsel)
void hook_marquee_speed(::Ctx* c, uint64_t, void*) {
    const float s = scale(kUi);
    if (full(s)) return;
    c->v[11].f[0] *= s;
    count(kUi);
}
// ... its start delay w8 (frames, after its csel)
void hook_marquee_delay(::Ctx* c, uint64_t, void*) {
    const float s = count_scale(kUi);
    const int32_t d = static_cast<int32_t>(c->x[8]);
    if (full(s) || d <= 0) return;
    c->x[8] = static_cast<uint32_t>(std::lround(static_cast<float>(d) / s));
    count(kUi);
}
// ... its speed lerp v2.2s = 1/32 per frame
void hook_marquee_lerp(::Ctx* c, uint64_t, void*) {
    const float s = scale(kUi);
    if (full(s) || c->v[2].f[0] != 0.03125f) return;
    c->v[2].f[0] = c->v[2].f[1] = ease(0.03125f, s);
    count(kUi);
}
// avatar turntable release: v += (+-1 - v) / s2 per frame, s2 = 6 -> 1 / (1 - (5/6)^s)
void hook_turn_release(::Ctx* c, uint64_t, void*) {
    const float s = scale(kUi);
    if (full(s) || c->v[2].f[0] != 6.0f) return;
    c->v[2].f[0] = 1.0f / ease(1.0f / 6.0f, s);
    count(kUi);
}
// ... and angle (s1, just loaded) += v (s0) per frame: angle + v * (s - 1) here, + v next
void hook_turn_angle(::Ctx* c, uint64_t, void*) {
    const float s = scale(kUi);
    if (full(s)) return;
    c->v[1].f[0] += c->v[0].f[0] * (s - 1.0f);
    count(kUi);
}

void hook_store_gameplay(::Ctx*, uint64_t, void*) { st().native.store(30, std::memory_order_relaxed); }
void hook_store_menu(::Ctx*, uint64_t, void*) { st().native.store(60, std::memory_order_relaxed); }

// frame_install checks every hook against the site list the game was lifted with (<exe>.sites, written by the game build). A missing site
// means the callout was not compiled into the lifted code, so its hook would never run; name it instead of leaving a silent hole.
void check_site(uint64_t base, const char* kind, uint32_t off, const char* what, size_t& checked, size_t& missing) {
    ++checked;
    if (services::modapi::site_in_build(base, kind, base + off)) return;
    ++missing;
    NXLOG_E(kTag, "frame-rate fixes: %s site 0x%llx is missing from this build's fefw.sites (%s): that hook will never run; rebuild with "
                  "recomp/games/fefw/main.sites",
            kind, static_cast<unsigned long long>(base + off), what);
}

uint32_t parse_skip(const std::string& list) {
    uint32_t m = 0;
    size_t i = 0;
    while (i <= list.size()) {
        size_t j = list.find(',', i);
        if (j == std::string::npos) j = list.size();
        std::string w = list.substr(i, j - i);
        w.erase(0, w.find_first_not_of(" \t"));
        w.erase(w.find_last_not_of(" \t") + 1);
        for (char& ch : w) ch = static_cast<char>(std::tolower(static_cast<unsigned char>(ch)));
        bool found = w.empty();
        for (int g = 0; g < kGroups; ++g)
            if (w == kGroupNames[g]) m |= 1u << g, found = true;
        if (!found) NXLOG_W(kTag, "[fefw] frame_fixes_skip: unknown group \"%s\" (dt, shader, physics, cloth, quake, anim, map, ui, text)", w.c_str());
        i = j + 1;
    }
    return m;
}

// ---- combat death cries (anim group; ported from ObeliskChamber#217) -------------------------------------------------------------------
// game::combat::ActionDamageCore::DoDie (0x1084840) starts a disposable::EveryFrameProc (0x10c8cc0) of 1.0 s with a callback (a
// std::function; its operator() 0x1085350: x0 = the closure, +8 the unit index; x1 -> t, the proc's elapsed / 1.0 s). Each frame the proc
// (0x10c9690) adds its clock's step and calls the callback; it ends when the callback returns 0 or t reaches 1. The callback waits for
// t > 0.04 and for the dying unit's own clock (unit [+88] + 304, its step this frame at + 16, frames of 1/59.94 s) to have moved more than
// 0.0001 s this frame, then asks for the voice "Damage_D" or "Damage_H" (0x10b0500 on the unit + 872) and returns 0.
// At a death the unit's clock stands still (the slow death camera) and then eases back. At 30 fps it passes 0.0001 s a frame at about
// t = 0.97, two frames before the proc ends. The threshold is per frame, so at 60 fps the same clock speed moves half as far a frame and
// passes it only after the proc has ended: no cry (at 120 a quarter). The callback is only ever called through its std::function, so it is
// wrapped through the function table (no hook site needed): the step it reads is divided by s for the call (the 30 fps step for the same
// clock speed) and put back after. [fefw] diag_voice (off) logs each death's first wait and how it ended.
constexpr uint32_t kDieCb = 0x1085350, kVoiceBusy = 0x100100, kUnitTableGot = 0x216b488;
GuestFn g_die_cb_orig = nullptr;
bool g_voice_log = false;
std::atomic<bool> g_die_cb_tried{false};
template <class T> bool vrd(uint64_t a, T* out) {
    if (!a || !mem::is_mapped(a, sizeof(T))) return false;
    std::memcpy(out, reinterpret_cast<const void*>(a), sizeof(T));
    return true;
}
uint64_t vcall(::Ctx* c, uint64_t fn, uint64_t x0) {
    const ::Ctx saved = *c;
    c->x[0] = x0;
    dispatch_call(c, fn);
    const uint64_t r = c->x[0];
    *c = saved;
    return r;
}
void die_cb_wrap(::Ctx* c) {
    const uint64_t base = st().base, closure = c->x[0];
    float t = -1.f, step = -1.f;
    uint32_t unit = 0xffffffffu;
    uint64_t got = 0, table = 0, unit_obj = 0, inner = 0, clock = 0;
    vrd(c->x[1], &t);
    vrd(closure + 8, &unit);
    const bool have = vrd(base + kUnitTableGot, &got) && vrd(got, &table) && table && unit <= 3 && vrd(table + unit * 0x18 + 40, &unit_obj) &&
                      vrd(unit_obj + 88, &inner) && vrd(inner + 304, &clock) && vrd(clock + 16, &step);
    const float sc = scale(kAnim);
    const bool scaled = have && sc < 1.0f && step > 0.0f;
    const float seen = scaled ? step / sc : step;
    if (scaled) std::memcpy(reinterpret_cast<void*>(clock + 16), &seen, sizeof(float));
    std::string why;
    if (g_voice_log && t > 0.04f && seen * 16.683f * 0.001f > 1e-4f) {     // the frame the callback asks for the voice: will 0x10b0500 play it?
        int busy = -1;
        uint8_t mode = 0;
        uint64_t combat = 0, comp = 0, v = 0, vh = 0;
        if (vrd(table + 8, &combat) && combat) vrd(combat + 113, &mode);
        if (vrd(unit_obj + 872 + 8, &comp) && vrd(comp + 88, &v) && v && vrd(v + 136, &vh) && vh)
            busy = static_cast<int>(vcall(c, base + kVoiceBusy, vh + 104) & 1);
        why = (mode >> 7 & 1) ? ", voices off: not played" : busy == 1 ? ", the unit's voice is busy: not played" : busy == 0 ? ", played" : ", voice handle not found";
    }
    g_die_cb_orig(c);
    if (scaled) {
        std::memcpy(reinterpret_cast<void*>(clock + 16), &step, sizeof(float));
        count(kAnim);
    }
    if (!g_voice_log) return;
    static uint64_t last_closure = 0;   // main thread
    const bool waits = static_cast<uint32_t>(c->x[0]) & 1, first = closure != last_closure;
    last_closure = closure;
    const char* end = !waits ? "asks for the voice" : t >= 0.999f ? "the wait ran out: no cry" : first ? "waits" : nullptr;
    if (!end) return;
    NXLOG_I(kTag, "voice: death cry, unit %u, t %.3f, unit clock step %.4f (the callback sees %.4f) -> %s%s", unit, t, step, seen, end, why.c_str());
}
// The lifted modules are registered after play mode installs its hooks: wrap the callback at the first frame step.
void die_cb_try_install() {
    if (g_die_cb_tried.exchange(true)) return;
    g_voice_log = config_get_bool("fefw", "diag_voice", false);
    const uint64_t at = st().base + kDieCb;
    g_die_cb_orig = fn_lookup(at);
    if (!g_die_cb_orig || !fn_lookup(st().base + kVoiceBusy)) {
        NXLOG_W(kTag, "frame-rate fixes: the combat death cry callback (0x%x) is not in this build: death cries stay as they are", kDieCb);
        return;
    }
    fn_register(at, &die_cb_wrap);
    NXLOG_I(kTag, "frame-rate fixes: combat death cries wrapped (anim)%s", g_voice_log ? ", logged ([fefw] diag_voice)" : "");
}

// ---- talk portraits (Spine): one call per 30 Hz step (ported from ObeliskChamber#217) --------------------------------------------------
// The talk portraits are Spine skeletons (spine-cpp). The game's wrapper (0x1947dd0) poses each one at its skeleton data's fps
// (+0x358) from an accumulator of its own (+0x35c): when it passes one step it applies the animation state, updates the world
// transform and advances the state by that step; otherwise it still calls spine::AnimationState::update(0.0001), with no apply.
// At 30 fps every call is a step. At 60 every other call is not, and when the game starts an animation with no mix (the eye blink,
// talk::Seq::BindFaceEyeBlinkSync: mix duration 0) just before such a call, that update gives the new entry a mix time above 0 and
// Spine drops the entry it replaces as finished mixing before it was ever applied mixing out. What that entry had set (the open eye's
// images) is never reset while the blink moves the lids: half-closed lids over open eyes, on several of Fortuna's lines.
// The manager that calls the wrapper for every skeleton (0x1940d60, only called through a pointer) is wrapped through the function
// table: between 30 Hz steps it is not called at all, and on a step it is called with the clock frames since the last one
// ([+0x48] + 0x68, frames of 1/59.94 s; put back after the call), so every skeleton is called once per 30 Hz step, as at 30 fps.
// Group anim: off with the frame-rate fixes, and with [fefw] frame_fixes_skip = anim.
constexpr uint32_t kSpineManager = 0x1940d60;
GuestFn g_spine_orig = nullptr;
std::mutex g_spine_mu;
std::unordered_map<uint64_t, float> g_spine_acc;   // manager -> clock frames since its last 30 Hz step; under g_spine_mu
std::atomic<bool> g_spine_tried{false};
void spine_mgr_wrap(::Ctx* c) {
    const uint64_t mgr = c->x[0];
    uint64_t clock = 0;
    float step = 0.f;
    if (full(scale(kAnim)) || !vrd(mgr + 0x48, &clock) || !vrd(clock + 0x68, &step) || !(step >= 0.f && step < 64.f)) {
        g_spine_orig(c);
        return;
    }
    float use = 0.f;
    {
        std::lock_guard<std::mutex> lk(g_spine_mu);
        if (g_spine_acc.size() > 256) g_spine_acc.clear();
        float& acc = g_spine_acc[mgr];
        acc += step;
        if (st().tick.load(std::memory_order_relaxed)) use = acc, acc = 0.f;
    }
    count(kAnim);
    if (use == 0.f) return;   // no 30 Hz step in this frame
    std::memcpy(reinterpret_cast<void*>(clock + 0x68), &use, 4);
    g_spine_orig(c);
    float now = 0.f;
    if (vrd(clock + 0x68, &now) && now == use) std::memcpy(reinterpret_cast<void*>(clock + 0x68), &step, 4);
}
void spine_try_install() {
    if (g_spine_tried.exchange(true)) return;
    const uint64_t at = st().base + kSpineManager;
    if (!(g_spine_orig = fn_lookup(at))) {
        NXLOG_W(kTag, "frame-rate fixes: the Spine manager (0x%x) is not in this build: talk portraits stay as they are", kSpineManager);
        return;
    }
    fn_register(at, &spine_mgr_wrap);
    NXLOG_I(kTag, "frame-rate fixes: talk portraits (Spine) called once per 30 Hz step (anim)");
}

}  // namespace

void frame_install(uint64_t base) {
    State& s = st();
    if (s.installed) return;
    s.installed = true;
    s.base = base;
    s.on.store(config_get_bool("fefw", "frame_fixes", true));
    s.skip = parse_skip(config_get_str("fefw", "frame_fixes_skip", ""));
    {
        const std::string g = config_get_str("fefw", "cloth_gravity", "accel");
        if (g != "accel" && g != "match")
            NXLOG_W(kTag, "[fefw] cloth_gravity = %s: not match or accel, accel is used", g.c_str());
        s.cloth_match.store(g == "match");
    }
    size_t n = 0, checked = 0, missing = 0;
    if (!services::modapi::site_list_known())
        NXLOG_W(kTag, "frame-rate fixes: no fefw.sites next to the exe, so the frame-fix hook sites cannot be checked against this build");
    for (const MovkSite& m : kMovk) {
        check_site(base, "inline", m.off + 4, "per-frame constant", checked, missing);
        n += hook_add_inline(base + m.off + 4, &hook_movk, const_cast<MovkSite*>(&m)) ? 1 : 0;
    }
    struct { uint32_t off; InlineHookFn fn; const char* what; } const items[] = {
        {kClothPhase, hook_cloth_phase, "cloth wind phase"}, {kClothBlend, hook_cloth_blend, "cloth blend"},
        {kClothDamp, hook_cloth_damp, "cloth damping"}, {kClothWind, hook_cloth_wind, "cloth wind force"},
        {kClothPull, hook_cloth_pull, "cloth pull"}, {kIllusion, hook_illusion, "CharacterIllusion period"},
        {kLookAt, hook_look_at, "CharacterLookAt weight"}, {kMapAngle, hook_map_angle, "map UI angle"},
        {kMapHeight, hook_map_height, "map marker height"}, {kMapTimer, hook_map_timer, "map timer"},
        {kSubsteps, hook_substeps, "PhysX substeps"}, {kMarqueeSpeed, hook_marquee_speed, "BGM marquee speed"},
        {kMarqueeDelay, hook_marquee_delay, "BGM marquee delay"}, {kMarqueeLerp, hook_marquee_lerp, "BGM marquee lerp"},
        {kTurnRelease[0], hook_turn_release, "avatar turntable release 1"}, {kTurnRelease[1], hook_turn_release, "avatar turntable release 2"},
        {kTurnRelease[2], hook_turn_release, "avatar turntable release 3"}, {kTurnRelease[3], hook_turn_release, "avatar turntable release 4"},
        {kTurnAngle[0], hook_turn_angle, "avatar turntable angle 1"}, {kTurnAngle[1], hook_turn_angle, "avatar turntable angle 2"},
        {kTurnAngle[2], hook_turn_angle, "avatar turntable angle 3"}, {kTurnAngle[3], hook_turn_angle, "avatar turntable angle 4"},
        {kStoreGameplay, hook_store_gameplay, "gameplay target store"}, {kStoreMenuA, hook_store_menu, "menu target store A"},
        {kStoreMenuB, hook_store_menu, "menu target store B"},
    };
    for (const auto& it : items) {
        check_site(base, "inline", it.off, it.what, checked, missing);
        n += hook_add_inline(base + it.off, it.fn) ? 1 : 0;
    }
    // the quake's 30 Hz block needs the lifted rest of its function (the midhook site 0x11eb084 in main.sites), looked up at the first quake:
    // the lifted modules are registered after play mode installs its hooks
    check_site(base, "midhook", kQuakeStart, "cutscene quake start", checked, missing);
    check_site(base, "midhook", kQuakeEnd, "cutscene quake rest", checked, missing);
    n += hook_add_mid(base + kQuakeStart, &hook_quake) ? 1 : 0;
    check_site(base, "entryhook", kTextStep, "dialogue text step", checked, missing);
    n += hook_add_entry(base + kTextStep, &hook_text_step) ? 1 : 0;
    if (missing)
        NXLOG_E(kTag, "frame-rate fixes: %zu of %zu hook sites are missing from this build; those fixes are inactive (main.sites and the "
                      "game build must match)", missing, checked);
    std::string skipped;
    for (int g = 0; g < kGroups; ++g)
        if (s.skip >> g & 1u) skipped += std::string(skipped.empty() ? "" : ", ") + kGroupNames[g];
    NXLOG_I(kTag, "frame-rate fixes ([fefw] frame_fixes = %d): %zu hooks, %zu sites checked%s%s", s.on.load() ? 1 : 0, n, checked,
            skipped.empty() ? "" : ", skipped: ", skipped.c_str());
}

// Once per frame from the time manager's step (main thread): step = the step it takes (frames of 1/59.94 s), target = graphics +0x1dc.
void frame_step(float step, int target) {
    State& s = st();
    if (!s.installed || target <= 0) return;
    die_cb_try_install();
    spine_try_install();
    int native = s.native.load(std::memory_order_relaxed);
    if (native <= 0) native = target;
    const float sc = std::clamp(static_cast<float>(native) / static_cast<float>(target), 0.125f, 1.0f);
    const float sf = step > 0.0f ? std::clamp(step * static_cast<float>(native) / 60.0f, 0.125f, 1.0f) : sc;
    s.s.store(sf, std::memory_order_relaxed);
    s.s_count.store(sc, std::memory_order_relaxed);
    bool tick = true;
    if (!full(sf)) {
        s.tick_acc += sf;
        tick = s.tick_acc >= 0.999f;
        if (tick) s.tick_acc = std::min(s.tick_acc - 1.0f, 1.0f);
    } else {
        s.tick_acc = 0.0f;
    }
    s.tick.store(tick, std::memory_order_relaxed);
}

void frame_set_enabled(bool on) {
    if (st().on.exchange(on) != on) NXLOG_I(kTag, "[live] frame-rate fixes %s", on ? "on" : "off");
}

FrameFixInfo frame_info() {
    const State& s = st();
    FrameFixInfo i;
    i.installed = s.installed;
    i.on = s.on.load(std::memory_order_relaxed);
    i.scale = s.s.load(std::memory_order_relaxed);
    i.native = s.native.load(std::memory_order_relaxed);
    i.quake = s.quake_state.load(std::memory_order_relaxed) > 0;
    for (int g = 0; g < kGroups; ++g) i.applied[g] = s.applied[g].load(std::memory_order_relaxed);
    i.quake_held = s.quake_held.load(std::memory_order_relaxed);
    return i;
}

const char* frame_group_name(int g) { return g >= 0 && g < kGroups ? kGroupNames[g] : "?"; }

}  // namespace nxrt::games::fefw
