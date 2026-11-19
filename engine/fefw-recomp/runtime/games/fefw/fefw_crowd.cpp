// SPDX-License-Identifier: GPL-3.0-or-later
// Fire Emblem: Fortune's Weave: how many townspeople the hubs show ([display] npc_distance in the controls file, 1 = the game's own;
// [fefw] npc_distance in the ini wins over it).
//
// The hub crowd update (0x1041880, map::zv crowd) shows and hides the townspeople through 0xd039e0 (chara, mode: 0 shown, 1 hidden,
// 2 the crowd's far / released state). Three of the game's limits decide who is drawn, and npc_distance k multiplies all three (tests
// with each one alone still left NPCs popping in a few metres away):
//   - the model limit: "hub character model hash limit" (70), the most distinct NPC models the crowd keeps loaded. At the limit a
//     townsperson whose model is not loaded is put in the far state and not drawn, even close by. This is most of the hub pop-in.
//   - the display distance: "hub character display distance" (80 m), the radius the crowd shows townspeople within.
//   - the draw budget: townspeople are shown nearest first while max(cost / 600000, count / 80) stays under 1.0; once it is reached,
//     every further townsperson that frame is hidden.
// k = 1 installs the hooks but leaves every value as the game has it.
#include "fefw_game.h"

#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <unordered_map>

#include "nxrt/abi.h"
#include "nxrt/config.h"
#include "nxrt/hooks.h"
#include "nxrt/log.h"

namespace nxrt::games::fefw {

namespace {

constexpr const char* kTag = "fefw";

uint32_t fbits(float f) {
    uint32_t u;
    std::memcpy(&u, &f, 4);
    return u;
}
float bitsf(uint32_t u) {
    float f;
    std::memcpy(&f, &u, 4);
    return f;
}

double g_npc = 1.0;                                                // npc_distance: the factor shown on the F1 page
double g_npc_model = 1.0, g_npc_radius = 1.0, g_npc_budget = 1.0;   // what each limit uses (npc_distance unless an ini key overrides it)

// ---- display distance -----------------------------------------------------------------------------------------------------------------
// The update reads the game parameter "hub character display distance" (an int in metres) into s8, then stores R^2, R^2, (R + 40)^2 and
// (R + 100)^2 as the distances at which it shows and hides the townspeople. k multiplies R (the 40 / 100 m margins stay as they are).
constexpr uint64_t kHubRadius = 0x1041c54;     // inline before `fmul s0, s8, s8`: s8 = R
std::atomic<uint32_t> g_last_r{0xffffffff};

void hook_hub_radius(::Ctx* c, uint64_t, void*) {
    const float r = c->v[8].f[0];
    const float scaled = static_cast<float>(r * g_npc_radius);
    c->v[8].f[0] = scaled;
    if (g_last_r.exchange(fbits(r), std::memory_order_relaxed) != fbits(r))
        NXLOG_I(kTag, "hub NPC display distance: the game's %.0f m -> %.0f m (x%.2f)", r, scaled, g_npc_radius);
}

// ---- draw budget ----------------------------------------------------------------------------------------------------------------------
// Each of the update's three passes sets s9 = 1.0 (the limit of max(cost / 600000, count / 80), the same table as the field's
// VisibleController); once it is reached the update sets crowd + 20 and hides every further townsperson that frame. k raises s9 to k.
constexpr uint64_t kCrowdBudget[] = {0x1042680, 0x1042898, 0x1042c20};   // inline after each `fmov s9, #1.0`

void hook_crowd_budget(::Ctx* c, uint64_t, void*) { c->v[9].f[0] = static_cast<float>(g_npc_budget); }

// ---- model limit ----------------------------------------------------------------------------------------------------------------------
// "hub character model hash limit" is stored at crowd + 16 (an int, 70 in the game; crowd + 24 counts the loaded models). k multiplies
// it (rounded up).
constexpr uint64_t kModelLimit = 0x1041b18;    // inline before `str w8, [x19, #16]`: w8 = the limit

void hook_model_limit(::Ctx* c, uint64_t, void*) {
    const int64_t game = static_cast<int32_t>(c->x[8]);
    const int64_t ours = g_npc_model > 1.0 && game > 0 ? static_cast<int64_t>(std::ceil(static_cast<double>(game) * g_npc_model)) : game;
    c->x[8] = static_cast<uint32_t>(ours);
    static std::atomic<int64_t> last{-1};
    if (last.exchange(game) != game)
        NXLOG_I(kTag, "hub NPC model limit: the game's %lld -> %lld", static_cast<long long>(game), static_cast<long long>(ours));
}

// ---- diagnostic ([fefw] npc_log = 1): every character show / hide change through 0xd039e0, with its caller and distance ----------------
constexpr uint64_t kCrowdRef = 0x1041d7c;      // inline: v2 = the crowd update's reference position (the player or the camera)
constexpr uint64_t kShowHide = 0xd039e0;       // entry: x0 = the character, w1 = mode, x30 = the caller
constexpr int kMaxLogLines = 2000;
uint64_t g_crowd_base = 0;
std::atomic<uint32_t> g_ref[3];
std::mutex g_diag_mu;
std::unordered_map<uint64_t, int> g_state;     // character -> last mode
int g_diag_lines = 0;

bool safe_read(uint64_t addr, void* out, size_t n) {   // a guest read through a pointer that is not verified
    if (addr < 0x10000 || addr > 0x7fffffffffffULL) return false;
    __try {
        std::memcpy(out, reinterpret_cast<const void*>(addr), n);
        return true;
    } __except (1) {
        return false;
    }
}

void hook_crowd_ref(::Ctx* c, uint64_t, void*) {
    for (int i = 0; i < 3; ++i) g_ref[i].store(fbits(c->v[2].f[i]), std::memory_order_relaxed);
}

int hook_show_hide(::Ctx* c, uint64_t, void*) {
    const uint64_t ch = c->x[0], caller = c->x[30] - g_crowd_base;
    const int mode = static_cast<int>(c->x[1] & 0xff);
    std::lock_guard<std::mutex> lk(g_diag_mu);
    auto [it, fresh] = g_state.try_emplace(ch, mode);
    if (!fresh && it->second == mode) return 0;
    it->second = mode;
    if (g_diag_lines >= kMaxLogLines) return 0;
    ++g_diag_lines;
    float p[3] = {0, 0, 0};
    uint64_t obj = 0;
    const bool have = safe_read(ch + 16, &obj, 8) && safe_read(obj + 560, p, 12);
    const float r[3] = {bitsf(g_ref[0].load()), bitsf(g_ref[1].load()), bitsf(g_ref[2].load())};
    const float d = std::sqrt((p[0] - r[0]) * (p[0] - r[0]) + (p[1] - r[1]) * (p[1] - r[1]) + (p[2] - r[2]) * (p[2] - r[2]));
    NXLOG_I(kTag, "npc %s: chara %llx from 0x%llx, %s%.1f m from the crowd reference (%.1f %.1f %.1f)",
            mode == 0 ? "shown" : mode == 1 ? "hidden" : "far", static_cast<unsigned long long>(ch),
            static_cast<unsigned long long>(caller), fresh ? "first call, " : "", have ? d : -1.0f, r[0], r[1], r[2]);
    return 0;
}

}  // namespace

void crowd_install(uint64_t base, double npc_distance) {
    const double ini = config_get_double("fefw", "npc_distance", 0.0);   // set in the ini: wins over the F1 page
    const double npc = ini > 0.0 ? ini : npc_distance;
    g_npc = npc > 1.0 ? std::fmin(npc, 4.0) : 1.0;
    // Testing / modding: each of the three limits can be set on its own in the ini (a factor, 1 = the game's own); not on the F1 page.
    auto own = [](const char* key) {
        const double v = config_get_double("fefw", key, 0.0);
        return v > 0.0 ? (v > 1.0 ? std::fmin(v, 8.0) : 1.0) : -1.0;
    };
    const double m = own("npc_model_limit"), r = own("npc_radius"), b = own("npc_budget");
    g_npc_model = m > 0 ? m : g_npc;
    g_npc_radius = r > 0 ? r : g_npc;
    g_npc_budget = b > 0 ? b : g_npc;
    hook_add_inline(base + kHubRadius, &hook_hub_radius);
    hook_add_inline(base + kModelLimit, &hook_model_limit);
    if (g_npc_budget > 1.0)
        for (uint64_t off : kCrowdBudget) hook_add_inline(base + off, &hook_crowd_budget);
    NXLOG_I(kTag, "hub NPCs x%.2f (%s; 1 = the game's own): model limit x%.2f%s, display distance x%.2f%s, budget x%.2f%s", g_npc,
            ini > 0.0 ? "[fefw] npc_distance" : "[display] npc_distance", g_npc_model, m > 0 ? " ([fefw] npc_model_limit)" : "",
            g_npc_radius, r > 0 ? " ([fefw] npc_radius)" : "", g_npc_budget, b > 0 ? " ([fefw] npc_budget)" : "");
    if (config_get_bool("fefw", "npc_log", false)) {
        g_crowd_base = base;
        hook_add_inline(base + kCrowdRef, &hook_crowd_ref);
        hook_add_entry(base + kShowHide, &hook_show_hide);
        NXLOG_I(kTag, "npc log: character show / hide changes are logged ([fefw] npc_log)");
    }
}

double npc_distance_applied() { return g_npc; }

}  // namespace nxrt::games::fefw
