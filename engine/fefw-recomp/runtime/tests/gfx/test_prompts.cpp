// SPDX-License-Identifier: GPL-3.0-or-later
// Button prompts (S17): the icon resolver (prompt_resolve.cpp), the icon pack reader and the shipped pack (prompts.cpp,
// assets/prompts/prompt_icons.pak), the texture override hook of the NVN upload path (nxrt/tex_override.h) and the block height rule
// that makes the game's small BC7 textures come out right (nvn_texture_layout.cpp).
#include <SDL3/SDL_scancode.h>

#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <set>
#include <string>
#include <vector>

#include "nvn_core.h"
#include "nvn_format.h"
#include "nvn_texdata.h"
#include "nvn_texture_layout.h"
#include "nxrt/config.h"
#include "nxrt/input_source.h"
#include "nxrt/tex_override.h"
#include "play/play_types.h"
#include "play/prompt_resolve.h"
#include "play/prompts.h"
#include "tegra_tiling.h"
#include "test_framework.h"
#include "project_root.h"
#if __has_include("pak.h")   // Metroid Prime Remastered pak/TXTR readers (mprs): only the real-data cases below use them
#include "pak.h"
#include "txtr.h"
#define NXTEST_HAVE_MPRS 1
#endif
NXTEST_GROUP(prompt_icons)

using namespace nxrt;
using namespace nxrt::play;
namespace fs = std::filesystem;

namespace {

fs::path pack_path() { return fs::path(__FILE__).parent_path().parent_path().parent_path() / "assets" / "prompts" / "prompt_icons.pak"; }

std::string name_of(const Settings& s, PromptStyle st, Ctx c, Icon i, bool label = true) {
    ResolveIn r;
    r.style = st;
    r.label_abxy = label;
    r.ctx = c;
    r.settings = &s;
    return resolve_icon(r, i);
}

// A minimal pack in memory: header + directory + blocks of one colour per entry.
std::vector<uint8_t> make_pack(const std::vector<std::pair<std::string, uint8_t>>& entries, uint32_t w = 36, uint32_t h = 36) {
    std::vector<uint8_t> b(24 + entries.size() * 48);
    auto put = [&](size_t off, uint32_t v) { for (int k = 0; k < 4; ++k) b[off + k] = uint8_t(v >> (8 * k)); };
    std::memcpy(b.data(), "MPIP", 4);
    put(4, 1);
    put(8, uint32_t(entries.size()));
    put(12, w);
    put(16, h);
    const uint32_t px = w * h * 4;
    for (size_t i = 0; i < entries.size(); ++i) {
        const size_t e = 24 + i * 48;
        std::memcpy(b.data() + e, entries[i].first.data(), entries[i].first.size());
        put(e + 40, uint32_t(b.size()));
        put(e + 44, px);
        b.insert(b.end(), px, entries[i].second);
    }
    return b;
}

}  // namespace

// ---- the resolver ------------------------------------------------------------------------------------------------------------------
NXTEST(switch_style_keeps_the_games_icons) {
    const Settings s = default_settings();
    for (int i = 0; i < kIconCount; ++i) CHECK_EQ(name_of(s, PromptStyle::Switch, Ctx::Menu, static_cast<Icon>(i)), std::string());
}

NXTEST(xbox_and_playstation_names_by_label_and_by_position) {
    const Settings s = default_settings();
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::A), std::string("xbox/a"));
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::B), std::string("xbox/b"));
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::X), std::string("xbox/x"));
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::Y), std::string("xbox/y"));
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::L), std::string("xbox/lb"));
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::ZR), std::string("xbox/rt"));
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::Plus), std::string("xbox/menu"));
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::Minus), std::string("xbox/view"));
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::DPad), std::string("xbox/dpad"));
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Menu, Icon::A), std::string("ps/cross"));
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Menu, Icon::B), std::string("ps/circle"));
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Menu, Icon::X), std::string("ps/square"));
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Menu, Icon::Y), std::string("ps/triangle"));
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Menu, Icon::R), std::string("ps/r1"));
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Menu, Icon::ZL), std::string("ps/l2"));
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Menu, Icon::Plus), std::string("ps/options"));
    // by position (label_abxy off) the Switch A button is the east button of the pad: Xbox B, circle; Switch B is the south button
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::A, false), std::string("xbox/b"));
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::B, false), std::string("xbox/a"));
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::X, false), std::string("xbox/y"));
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::Y, false), std::string("xbox/x"));
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Menu, Icon::A, false), std::string("ps/circle"));
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Menu, Icon::B, false), std::string("ps/cross"));
    // the pressed twin
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::APressed), std::string("xbox/a_pressed"));
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Menu, Icon::ZRPressed), std::string("ps/r2_pressed"));
}

NXTEST(keyboard_style_follows_the_bindings_of_the_context) {
    const Settings s = default_settings();
    // menu: confirm = Return, back = Escape; gameplay: jump (B) = Space, morph (Y) = Left Ctrl, fire (ZR) = left mouse button
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Menu, Icon::A), std::string("kbm/return"));
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Menu, Icon::B), std::string("kbm/escape"));
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Play, Icon::B), std::string("kbm/space"));
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Play, Icon::Y), std::string("kbm/left ctrl"));
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Play, Icon::ZR), std::string("kbm/mouse1"));
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Play, Icon::BPressed), std::string("kbm/space_pressed"));
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Menu, Icon::L), std::string("kbm/q"));
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Menu, Icon::R), std::string("kbm/e"));
    // A is only bound in menus and the map: in gameplay the prompt still shows the menu key
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Play, Icon::A), std::string("kbm/return"));
    // X is bound only in the map (its Key / Legend button): the map shows it, and the other contexts find it there
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Map, Icon::X), std::string("kbm/x"));
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Menu, Icon::X), std::string("kbm/x"));
}

NXTEST(keyboard_style_follows_a_rebind) {
    Settings s = default_settings();
    InputId in;
    CHECK(parse_input("G", &in));
    Binding b;
    b.in = in;
    b.ctx = Ctx::Play;
    b.act = Act::Jump;
    s.bindings.insert(s.bindings.begin(), b);           // G jumps now, ahead of Space
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Play, Icon::B), std::string("kbm/g"));
    // the [switch] table: jump on A instead of B moves the prompt
    Settings t = default_settings();
    Output o;
    CHECK(parse_output("A", &o));
    t.out[static_cast<int>(Act::Jump)] = o;
    CHECK_EQ(name_of(t, PromptStyle::Keyboard, Ctx::Play, Icon::A), std::string("kbm/space"));
    // a context binding beats the `all` layer for the same input: Space confirms in menus, so it is not the key of jump there
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Menu, Icon::B), std::string("kbm/escape"));
}

NXTEST(keyboard_style_follows_a_rebound_jump_key) {
    // the one binding of jump (Space in gameplay) moves to G: the B prompt follows it
    Settings s = default_settings();
    InputId g;
    CHECK(parse_input("G", &g));
    int changed = 0;
    for (Binding& b : s.bindings) {
        if (b.act != Act::Jump) continue;
        b.in = g;
        ++changed;
    }
    CHECK_EQ(changed, 1);
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Play, Icon::B), std::string("kbm/g"));
}

NXTEST(pad_styles_follow_a_pad_remap) {
    // South remapped to the Switch X button: the X icon is the bottom button, which prints A (Xbox) or cross (PlayStation)
    Settings s = default_settings();
    s.pad.remap[static_cast<int>(PadSrc::South)] = hid::btn(hid::kBtnX);
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::X), std::string("xbox/a"));
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Menu, Icon::X), std::string("ps/cross"));
    // South was the only pad button of Switch A (label mode): no pad button presses A now, so the game keeps its own icon
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::A), std::string());
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Menu, Icon::A), std::string());
    // the buttons that are not remapped keep their icons
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::B), std::string("xbox/b"));
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::Y), std::string("xbox/y"));
    // by position Switch B was South: now nothing presses it
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::B, false), std::string());
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::A, false), std::string("xbox/b"));
}

NXTEST(a_switch_button_nothing_presses_has_no_pad_icon) {
    Settings s = default_settings();
    s.pad.remap[static_cast<int>(PadSrc::LeftShoulder)] = 0;   // remapped to nothing: L has no pad button now
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::L), std::string());
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Menu, Icon::L), std::string());
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::R), std::string("xbox/rb"));
    // a stick click has no icon of its own: a Switch button remapped to a stick click gets no icon
    Settings t = default_settings();
    t.pad.remap[static_cast<int>(PadSrc::LeftStick)] = hid::btn(hid::kBtnX);
    CHECK_EQ(name_of(t, PromptStyle::Xbox, Ctx::Menu, Icon::X), std::string());
    CHECK_EQ(name_of(t, PromptStyle::Xbox, Ctx::Menu, Icon::Y), std::string("xbox/y"));
}

NXTEST(map_sticks_show_the_mouse_buttons_that_drive_the_map) {
    const Settings s = default_settings();
    // the map: left drag rotates it (button 1), middle drag moves it (button 2)
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Map, Icon::LStick), std::string("kbm/mouse1"));
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Map, Icon::RStick), std::string("kbm/mouse2"));
    // no settings at all: the stick icons do not need the bindings
    ResolveIn r;
    r.style = PromptStyle::Keyboard;
    r.ctx = Ctx::Map;
    CHECK_EQ(resolve_icon(r, Icon::LStick), std::string("kbm/mouse1"));
}

NXTEST(stick_icons_keep_the_games_icon_outside_the_keyboard_map) {
    const Settings s = default_settings();
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Menu, Icon::LStick), std::string());
    CHECK_EQ(name_of(s, PromptStyle::Keyboard, Ctx::Play, Icon::RStick), std::string());
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Map, Icon::LStick), std::string());
    CHECK_EQ(name_of(s, PromptStyle::PlayStation, Ctx::Map, Icon::RStick), std::string());
    CHECK_EQ(name_of(s, PromptStyle::Switch, Ctx::Map, Icon::RStick), std::string());
    CHECK_EQ(name_of(s, PromptStyle::Xbox, Ctx::Menu, Icon::RStick), std::string());
}

NXTEST(auto_follows_the_last_used_device) {
    nxrt::config_set("host", "handheld", "off");   // desktop behaviour, also when the tests run on a Deck (host_is_handheld reads it once)
    CHECK(auto_style(LastDevice::KeyboardMouse, 1, 1) == PromptStyle::Keyboard);
    CHECK(auto_style(LastDevice::Pad, 2, -1) == PromptStyle::PlayStation);
    CHECK(auto_style(LastDevice::Pad, 1, -1) == PromptStyle::Xbox);
    CHECK(auto_style(LastDevice::Pad, 3, -1) == PromptStyle::Switch);
    CHECK(auto_style(LastDevice::Pad, 0, -1) == PromptStyle::Xbox);          // a pad of no known family looks like an Xbox pad
    CHECK(auto_style(LastDevice::None, 0, -1) == PromptStyle::Keyboard);     // nothing used yet and no pad
    CHECK(auto_style(LastDevice::None, 0, 2) == PromptStyle::PlayStation);   // nothing used yet, a DualSense is connected
}

NXTEST(every_icon_has_an_id_and_a_pak) {
    std::set<std::string> uuids;
    for (int i = 0; i < kIconCount; ++i) {
        const Icon ic = static_cast<Icon>(i);
        CHECK_EQ(std::strlen(icon_asset_uuid(ic)), size_t(36));
        CHECK(uuids.insert(icon_asset_uuid(ic)).second);
        CHECK(std::strstr(icon_asset_name(ic), "TXTR_") == icon_asset_name(ic));
        CHECK(std::strstr(icon_pak_file(ic), ".pak") != nullptr);
        CHECK(std::strlen(icon_pak_file(ic)) > 0);
        const auto canonical = [](const char* id) {   // lower-case hex and dashes, the canonical text of the pak ids
            for (const char* c = id; *c; ++c)
                if (!((*c >= 'a' && *c <= 'z') || (*c >= '0' && *c <= '9') || *c == '-')) return false;
            return true;
        };
        CHECK(canonical(icon_asset_uuid(ic)));
        if (const char* front = icon_front_uuid(ic)) {   // the front end's own copy (title screen, menus)
            CHECK_EQ(std::strlen(front), size_t(36));
            CHECK(canonical(front));
            CHECK(uuids.insert(front).second);
        }
    }
    int with_front = 0;
    for (int i = 0; i < kIconCount; ++i) with_front += icon_front_uuid(static_cast<Icon>(i)) != nullptr;
    CHECK_EQ(with_front, 18);   // every icon but the d-pad (whose id is the same in both interfaces), the two sticks included
}

// ---- the pack ------------------------------------------------------------------------------------------------------------------------
NXTEST(pack_reader_finds_entries_and_rejects_bad_files) {
    IconPack p;
    CHECK(p.load_memory(make_pack({{"xbox/a", 1}, {"kbm/w", 2}, {"ps/cross", 3}})));
    CHECK_EQ(p.size(), size_t(3));
    CHECK(p.find("kbm/w") && p.find("kbm/w")[0] == 2 && p.find("kbm/w")[36 * 36 * 4 - 1] == 2);
    CHECK(p.find("xbox/a") && p.find("xbox/a")[100] == 1);
    CHECK(p.find("ps/cross") && p.find("ps/cross")[5] == 3);
    CHECK(p.find("xbox/b") == nullptr);
    IconPack bad;
    std::vector<uint8_t> v = make_pack({{"a", 1}});
    v[0] = 'X';
    CHECK(!bad.load_memory(v));
    v = make_pack({{"a", 1}});
    v.resize(v.size() - 10);                  // the last block is cut off
    CHECK(!bad.load_memory(v));
    CHECK(!bad.load_memory(std::vector<uint8_t>(10, 0)));
    CHECK(!bad.load_file("no/such/file.pak"));
}

NXTEST(the_shipped_pack_has_every_icon_every_style_can_ask_for) {
    IconPack p;
    CHECK_MSG(p.load_file(pack_path().string()), p.error());
    CHECK_EQ(p.width(), 36u);
    CHECK_EQ(p.height(), 36u);
    CHECK(p.find("kbm/any") != nullptr);
    // pad styles, both label modes
    for (PromptStyle st : {PromptStyle::Xbox, PromptStyle::PlayStation})
        for (bool label : {true, false})
            for (int i = 0; i < kIconCount; ++i) {
                const Icon ic = static_cast<Icon>(i);
                const Settings s = default_settings();
                const std::string n = name_of(s, st, Ctx::Menu, ic, label);
                if (icon_button(ic) == Btn::None) {   // the stick icons: a pad shows the game's own
                    CHECK(n.empty());
                    continue;
                }
                CHECK_MSG(!n.empty() && p.find(n) != nullptr, n);
            }
    // keyboard: the key of every name SDL has for a scancode (a key the player can bind) has an entry, and its pressed twin
    int checked = 0;
    for (int sc = 4; sc < 285; ++sc) {
        InputId id{InKind::Key, sc};
        const std::string key = kbm_pack_name(id);
        if (key.empty()) continue;
        CHECK_MSG(p.find(key) != nullptr, key);
        CHECK_MSG(p.find(key + "_pressed") != nullptr, key + "_pressed");
        ++checked;
    }
    CHECK(checked > 150);
    for (const char* m : {"kbm/mouse1", "kbm/mouse2", "kbm/mouse3", "kbm/mouse4", "kbm/mouse5", "kbm/wheelup", "kbm/wheeldown"})
        CHECK_MSG(p.find(m) != nullptr, m);
}

NXTEST(hash_separates_and_is_stable) {
    std::vector<uint8_t> a(3072, 7), b = a;
    b[2000] ^= 1;
    CHECK(prompt_hash(a.data(), a.size()) != prompt_hash(b.data(), b.size()));
    CHECK_EQ(prompt_hash(a.data(), a.size()), prompt_hash(a.data(), a.size()));
    CHECK_EQ(prompt_hash("a", 1), 0xaf63dc4c8601ec8cull);   // FNV-1a 64 of "a"
}

// ---- the block height rule -----------------------------------------------------------------------------------------------------------
NXTEST(console_block_height_rule) {
    using nxrt::nvn::be::console_block_height;
    CHECK_EQ(console_block_height(1), 1u);
    CHECK_EQ(console_block_height(9), 1u);      // a 36 texel high BC7 icon: 1 GOB (the smallest-fit rule said 2 and broke the icons)
    CHECK_EQ(console_block_height(10), 1u);
    CHECK_EQ(console_block_height(11), 2u);
    CHECK_EQ(console_block_height(21), 2u);
    CHECK_EQ(console_block_height(22), 4u);
    CHECK_EQ(console_block_height(42), 4u);
    CHECK_EQ(console_block_height(43), 8u);
    CHECK_EQ(console_block_height(85), 8u);
    CHECK_EQ(console_block_height(86), 16u);
    CHECK_EQ(console_block_height(5000), 16u);
}

NXTEST(a_36x36_bc7_texture_is_laid_out_with_one_gob) {
    using namespace nxrt::nvn;
    TextureBuilderInfo b;
    b.target = NVN_TEXTURE_TARGET_2D;
    b.width = 36;
    b.height = 36;
    b.depth = 1;
    b.levels = 1;
    b.format = NVN_FORMAT_BPTC_UNORM_SRGB;
    const be::TexLayout L = be::compute_texture_layout(b);
    CHECK(L.valid);
    CHECK_EQ(L.layout.levels[0].gob_blocks_y, 1u);
    CHECK_EQ(L.storage_size, 3072u);   // 3 GOBs wide, 2 GOB rows: what the game's TXTR buffer holds
}

// ---- the override hook in the upload path ----------------------------------------------------------------------------------------------
namespace {
struct HookData {
    uint64_t want = 0;
    uint8_t value = 0;
    int calls = 0;
};
bool test_hook(const gfx::TexOverrideReq& r, uint8_t* rgba, void* user) {
    HookData* d = static_cast<HookData*>(user);
    ++d->calls;
    if (r.width != 36 || r.height != 36 || r.tiled_size < 3072 || prompt_hash(r.tiled, 3072) != d->want) return false;
    for (size_t i = 0; i < 36u * 36u; ++i) {
        rgba[4 * i] = d->value;
        rgba[4 * i + 1] = 0;
        rgba[4 * i + 2] = 0;
        rgba[4 * i + 3] = 255;
    }
    return true;
}
}  // namespace

NXTEST(the_upload_path_asks_the_hook_and_replaces_the_pixels) {
    using namespace nxrt::nvn;
    using namespace nxrt::nvn::be;
    config_set("nvn", "bc7_cache", "");
    TextureBuilderInfo info;
    info.target = NVN_TEXTURE_TARGET_2D;
    info.width = 36;
    info.height = 36;
    info.depth = 1;
    info.levels = 1;
    info.format = NVN_FORMAT_BPTC_UNORM_SRGB;
    const be::TexLayout L = be::compute_texture_layout(info);
    CHECK(L.valid);
    std::vector<uint8_t> lin(L.layout.linear_size);
    for (size_t i = 0; i < lin.size(); ++i) lin[i] = uint8_t(i * 31 + 5);
    std::vector<uint8_t> mem(L.storage_size, 0);
    CHECK(tegra::tile(L.desc, L.layout, lin.data(), mem.data()));
    Ref<Texture> tex(new Texture());
    tex->b = info;
    tex->fmt = L.fmt;
    tex->mem = mem.data();
    tex->storage_size = L.storage_size;
    tex->block_linear = true;
    tex->levels = 1;
    tex->layers = 1;

    // no hook: the game's blocks come out unchanged
    gfx::set_texture_override(nullptr, nullptr);
    be::PreparedTexture out;
    CHECK_MSG(be::prepare_texture(*tex, be::TexCaps(), out), out.error);
    CHECK(out.data == lin);

    // a hook that does not know the texture leaves it alone
    HookData d;
    d.want = 1;
    gfx::set_texture_override(&test_hook, &d);
    CHECK_MSG(be::prepare_texture(*tex, be::TexCaps(), out), out.error);
    CHECK(d.calls >= 1);
    CHECK(out.data == lin);

    // a hook that knows it (by the hash of the tiled bytes) replaces the pixels with its own, encoded to BC7 at the same size
    d.want = prompt_hash(mem.data(), 3072);
    d.value = 200;
    CHECK_MSG(be::prepare_texture(*tex, be::TexCaps(), out), out.error);
    CHECK_EQ(out.data.size(), lin.size());
    CHECK(out.data != lin);
    CHECK_EQ(out.format, tex->fmt->vk);
    gfx::set_texture_override(nullptr, nullptr);
}

#if NXTEST_HAVE_MPRS
// ---- real data: the game's own icon (skipped without the paks) ------------------------------------------------------------------------
NXTEST(real_icon_untiles_to_valid_blocks_with_the_console_rule) {
    const char* env = std::getenv("NXRT_FS_ROMFS");
    const fs::path romfs = env && *env ? fs::path(env) : fs::path(nxtest::project_path("game/romfs"));
    const fs::path pak_file = romfs / "Preload/MPR1/GuiSysMP1.pak";
    if (!fs::exists(pak_file)) {
        nxtest::skip("the game's GuiSysMP1.pak is not here");
        return;
    }
    auto pak = mprs::Pak::open(mprs::open_file_storage(pak_file.string()));
    int tested = 0;
    for (const mprs::PakEntry& e : pak->assets()) {
        if (std::strcmp(e.type, "TXTR") != 0 || e.id.str() != icon_asset_uuid(Icon::A)) continue;
        const mprs::TxtrInfo info = mprs::txtr_read_info(*pak, e);
        const std::vector<mprs::u8> tiled = mprs::txtr_read_tiled(*pak, e, info);
        CHECK_EQ(info.width, 36u);
        CHECK_EQ(tiled.size(), size_t(3072));
        // untile with 1 GOB (the console rule) and with 2 GOBs (the old rule): a valid BC7 image has no all-zero blocks (mode bits 0 are
        // reserved); the wrong layout leaves the right half of the icon as zero blocks
        auto zero_blocks = [&](uint32_t gy) {
            tegra::TextureDesc d;
            d.width = 36;
            d.height = 36;
            d.depth = 1;
            d.levels = 1;
            d.layers = 1;
            d.block_w = 4;
            d.block_h = 4;
            d.bytes_per_block = 16;
            d.gob_blocks_y = gy;
            d.gob_blocks_z = 1;
            d.linear_stride_align = 1;
            const tegra::Layout L = tegra::compute_layout(d);
            std::vector<uint8_t> lin(L.linear_size);
            CHECK(tegra::untile(d, L, tiled.data(), lin.data()));
            int zeros = 0;
            for (size_t b = 0; b < lin.size(); b += 16) {
                bool z = true;
                for (size_t k = 0; k < 16; ++k) z = z && lin[b + k] == 0;
                zeros += z ? 1 : 0;
            }
            return zeros;
        };
        CHECK_EQ(zero_blocks(1), 0);
        CHECK(zero_blocks(2) > 0);
        ++tested;
    }
    CHECK_EQ(tested, 1);
}

// Every id of the table is a 36x36 one-mip BC7 texture of 3072 bytes in its pak: the in-game copies in GuiSysMP1.pak (blue) and the front end's
// own in PreloadFrontEndMPT.pak (red, drawn by the title screen and the menus: they were the textures the first version never recognised).
NXTEST(real_icon_ids_exist_in_both_interfaces) {
    const char* env = std::getenv("NXRT_FS_ROMFS");
    const fs::path romfs = env && *env ? fs::path(env) : fs::path(nxtest::project_path("game/romfs"));
    if (!fs::exists(romfs / "Preload/MPR1/GuiSysMP1.pak") || !fs::exists(romfs / "Preload/MPRT/PreloadFrontEndMPT.pak")) {
        nxtest::skip("the game's paks are not here");
        return;
    }
    const auto count = [&](const char* rel, const std::set<std::string>& ids) {
        auto pak = mprs::Pak::open(mprs::open_file_storage((romfs / rel).string()));
        std::set<std::string> seen;
        for (const mprs::PakEntry& e : pak->assets()) {
            if (std::strcmp(e.type, "TXTR") != 0 || !ids.count(e.id.str())) continue;
            const mprs::TxtrInfo info = mprs::txtr_read_info(*pak, e);
            CHECK_EQ(info.width, 36u);
            CHECK_EQ(info.height, 36u);
            CHECK_EQ(info.mips, 1u);
            CHECK_EQ(mprs::txtr_read_tiled(*pak, e, info).size(), size_t(3072));
            seen.insert(e.id.str());
        }
        return seen.size();
    };
    std::set<std::string> gui, front;
    for (int i = 0; i < kIconCount; ++i) {
        const Icon ic = static_cast<Icon>(i);
        (std::strcmp(icon_pak_file(ic), "Preload/MPR1/GuiSysMP1.pak") == 0 ? gui : front).insert(icon_asset_uuid(ic));
        if (const char* f = icon_front_uuid(ic)) front.insert(f);
    }
    CHECK_EQ(count("Preload/MPR1/GuiSysMP1.pak", gui), gui.size());
    CHECK_EQ(count("Preload/MPRT/PreloadFrontEndMPT.pak", front), front.size());
}
#endif  // NXTEST_HAVE_MPRS
