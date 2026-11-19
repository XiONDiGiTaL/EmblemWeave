// SPDX-License-Identifier: GPL-3.0-or-later
// Fire Emblem: Fortune's Weave: lighter graphics data for slower PCs ([fefw] reflections, ambient_occlusion, crowd_detail, scenery_detail).
// The game reads its effect settings from MessagePack files in the RomFS; these keys edit them in memory as the game's file
// system is set up (services/mods_dll.cpp add_runtime_edit, the code mods' edit_file path), so they apply at the next start.
//
//   Graphics/Settings/**/*.set     (a keyframed value per setting: <section>.<key>[n].value)
//     SSR.Enabled true -> false               [fefw] reflections = off
//     SSR.RaySamplingCount 32 -> 8            [fefw] reflections = reduced  (a little grainier; mostly helps in the hubs)
//     AO.SSAO_Enabled true -> false           [fefw] ambient_occlusion = 0  (flatter corners, saves GPU time everywhere)
//   Graphics/Settings/**/lodPresets.lod
//     presets.CharacterMob / CharacterLegion .Thresholds[i] (screen-size fractions) x 2   [fefw] crowd_detail = reduced
//     presets.Map*.Thresholds[i] (distances in metres) x 0.6                                [fefw] scenery_detail = reduced
//
// The files and keys are the ones Fortune's Eden (TheSneakyBadger, GPL-3.0-or-later) changes for its Performance settings: technique only,
// this code is our own. Every change keeps each value's MessagePack encoding, so the file keeps its size and layout; a file that does not
// parse is left as it is. The Low graphics preset (fefw_presets.h) turns all four down.
#include "fefw_game.h"

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

#include "nxrt/config.h"
#include "nxrt/log.h"
#include "nxrt/mod_api.h"
#include "nxrt/services_api.h"

namespace nxrt::games::fefw {
namespace {

constexpr const char* kTag = "fefw";

struct Detail {
    int reflections = 0;          // 0 full, 1 reduced (8 rays), 2 off
    bool ao = true;
    bool crowd_reduced = false;
    bool scenery_reduced = false;
    bool any() const { return reflections != 0 || !ao || crowd_reduced || scenery_reduced; }
};
Detail g_detail;

std::string lower(std::string s) {
    for (char& c : s) c = static_cast<char>(c >= 'A' && c <= 'Z' ? c + 32 : c);
    return s;
}

// ---- MessagePack: every scalar with its key path ------------------------------------------------------------------------------------
struct Scalar {
    std::vector<std::string> path;   // map keys; array elements as "[n]"
    size_t at;                       // offset of the type byte
    uint8_t type;
};

class Reader {
public:
    Reader(const uint8_t* d, size_t n) : d_(d), n_(n) {}
    // Several top-level values may follow each other. False on anything unknown: the file is then left alone.
    bool all(std::vector<Scalar>* out) {
        std::vector<std::string> path;
        while (p_ < n_)
            if (!value(path, out, 0)) return false;
        return true;
    }

private:
    bool has(size_t k) const { return p_ + k <= n_; }
    uint64_t be(size_t at, size_t w) const {
        uint64_t v = 0;
        for (size_t i = 0; i < w; ++i) v = v << 8 | d_[at + i];
        return v;
    }
    // map keys are strings here; anything else stops the walk
    bool key(std::string* k) {
        if (!has(1)) return false;
        const uint8_t t = d_[p_];
        size_t len = 0, hdr = 1;
        if (t >= 0xa0 && t <= 0xbf) len = t & 0x1f;
        else if (t == 0xd9 && has(2)) len = d_[p_ + 1], hdr = 2;
        else if (t == 0xda && has(3)) len = static_cast<size_t>(be(p_ + 1, 2)), hdr = 3;
        else return false;
        if (!has(hdr + len)) return false;
        k->assign(reinterpret_cast<const char*>(d_ + p_ + hdr), len);
        p_ += hdr + len;
        return true;
    }
    bool scalar(const std::vector<std::string>& path, std::vector<Scalar>* out, size_t size) {
        if (!has(size)) return false;
        out->push_back({path, p_, d_[p_]});
        p_ += size;
        return true;
    }
    bool value(std::vector<std::string>& path, std::vector<Scalar>* out, int depth) {
        if (depth > 40 || !has(1)) return false;
        const uint8_t t = d_[p_];
        // fixint, negative fixint, nil, false, true
        if (t <= 0x7f || t >= 0xe0 || t == 0xc0 || t == 0xc2 || t == 0xc3) return scalar(path, out, 1);
        // fixstr, str 8/16/32, bin 8/16/32
        if (t >= 0xa0 && t <= 0xbf) return scalar(path, out, 1 + (t & 0x1f));
        if (t == 0xd9 || t == 0xc4) return has(2) && scalar(path, out, 2 + d_[p_ + 1]);
        if (t == 0xda || t == 0xc5) return has(3) && scalar(path, out, 3 + static_cast<size_t>(be(p_ + 1, 2)));
        if (t == 0xdb || t == 0xc6) return has(5) && scalar(path, out, 5 + static_cast<size_t>(be(p_ + 1, 4)));
        switch (t) {
            case 0xcc: case 0xd0: return scalar(path, out, 2);
            case 0xcd: case 0xd1: return scalar(path, out, 3);
            case 0xca: case 0xce: case 0xd2: return scalar(path, out, 5);
            case 0xcb: case 0xcf: case 0xd3: return scalar(path, out, 9);
            default: break;
        }
        // maps and arrays
        size_t count = 0;
        bool map = false;
        if (t >= 0x80 && t <= 0x8f) count = t & 0x0f, map = true, p_ += 1;
        else if (t >= 0x90 && t <= 0x9f) count = t & 0x0f, p_ += 1;
        else if ((t == 0xde || t == 0xdc) && has(3)) count = static_cast<size_t>(be(p_ + 1, 2)), map = t == 0xde, p_ += 3;
        else if ((t == 0xdf || t == 0xdd) && has(5)) count = static_cast<size_t>(be(p_ + 1, 4)), map = t == 0xdf, p_ += 5;
        else return false;   // ext types and the reserved byte: not expected in these files
        for (size_t i = 0; i < count; ++i) {
            std::string k;
            if (map) {
                if (!key(&k)) return false;
            } else {
                k = "[" + std::to_string(i) + "]";
            }
            path.push_back(std::move(k));
            const bool ok = value(path, out, depth + 1);
            path.pop_back();
            if (!ok) return false;
        }
        return true;
    }

    const uint8_t* d_;
    size_t n_;
    size_t p_ = 0;
};

// A number scaled where it stands, in its own encoding: positive fixint, uint 8/16/32, float 32/64. False when it does not fit.
bool scale_number(std::vector<uint8_t>& b, const Scalar& s, double f) {
    const size_t at = s.at;
    auto rd = [&](size_t w) {
        uint64_t v = 0;
        for (size_t i = 0; i < w; ++i) v = v << 8 | b[at + 1 + i];
        return v;
    };
    auto wr = [&](size_t w, uint64_t v) {
        for (size_t i = w; i-- > 0; v >>= 8) b[at + 1 + i] = static_cast<uint8_t>(v);
    };
    if (s.type <= 0x7f) {
        const double v = s.type * f + 0.5;
        if (v < 0 || v > 127) return false;
        b[at] = static_cast<uint8_t>(v);
        return true;
    }
    const size_t w = s.type == 0xcc ? 1 : s.type == 0xcd ? 2 : s.type == 0xce ? 4 : 0;
    if (w) {
        const double v = static_cast<double>(rd(w)) * f + 0.5;
        if (v < 0 || v >= static_cast<double>(uint64_t{1} << (8 * w))) return false;
        wr(w, static_cast<uint64_t>(v));
        return true;
    }
    if (s.type == 0xca) {
        float x;
        const uint32_t raw = static_cast<uint32_t>(rd(4));
        std::memcpy(&x, &raw, 4);
        x = static_cast<float>(x * f);
        uint32_t out;
        std::memcpy(&out, &x, 4);
        wr(4, out);
        return true;
    }
    if (s.type == 0xcb) {
        double x;
        const uint64_t raw = rd(8);
        std::memcpy(&x, &raw, 8);
        x *= f;
        uint64_t out;
        std::memcpy(&out, &x, 8);
        wr(8, out);
        return true;
    }
    return false;
}

// <section>.<key>[...].value: how the .set files keep a setting (one value per keyframe)
bool is_setting(const Scalar& s, const char* section, const char* key) {
    return s.path.size() >= 3 && s.path[0] == section && s.path[1] == key && s.path.back() == "value";
}

struct Counts {
    int ssr_off = 0, ssr_rays = 0, ao_off = 0, crowd = 0, scenery = 0, files = 0, unparsed = 0;
};
Counts g_counts;

void edit_set(const NxModFile* file, void*) {
    std::vector<uint8_t> b(file->data, file->data + file->size);
    std::vector<Scalar> all;
    if (!Reader(b.data(), b.size()).all(&all)) {
        ++g_counts.unparsed;
        NXLOG_W(kTag, "graphics detail: %s does not parse as MessagePack: left as it is", file->path);
        return;
    }
    bool changed = false;
    for (const Scalar& s : all) {
        uint8_t& t = b[s.at];
        if (g_detail.reflections == 2 && is_setting(s, "SSR", "Enabled") && t == 0xc3) {
            t = 0xc2;   // true -> false
            ++g_counts.ssr_off, changed = true;
        } else if (g_detail.reflections == 1 && is_setting(s, "SSR", "RaySamplingCount") && t <= 0x7f && t > 8) {
            t = 8;      // a positive fixint stays one byte
            ++g_counts.ssr_rays, changed = true;
        } else if (!g_detail.ao && is_setting(s, "AO", "SSAO_Enabled") && t == 0xc3) {
            t = 0xc2;
            ++g_counts.ao_off, changed = true;
        }
    }
    if (!changed) return;
    ++g_counts.files;
    file->replace(file, b.data(), b.size());
}

void edit_lod(const NxModFile* file, void*) {
    std::vector<uint8_t> b(file->data, file->data + file->size);
    std::vector<Scalar> all;
    if (!Reader(b.data(), b.size()).all(&all)) {
        ++g_counts.unparsed;
        NXLOG_W(kTag, "graphics detail: %s does not parse as MessagePack: left as it is", file->path);
        return;
    }
    bool changed = false;
    for (const Scalar& s : all) {
        if (s.path.size() != 4 || s.path[0] != "presets" || s.path[2] != "Thresholds") continue;
        const std::string& name = s.path[1];
        // crowds: screen-size fractions, so a larger threshold switches to the simpler model sooner
        if (g_detail.crowd_reduced && (name == "CharacterMob" || name == "CharacterLegion") && scale_number(b, s, 2.0)) {
            ++g_counts.crowd, changed = true;
        // scenery: distances in metres (50/100/150/200, hidden at 1000), so 60 % brings each step nearer; 0 stays 0
        } else if (g_detail.scenery_reduced && name.rfind("Map", 0) == 0 && b[s.at] != 0 && scale_number(b, s, 0.6)) {
            ++g_counts.scenery, changed = true;
        }
    }
    if (!changed) return;
    ++g_counts.files;
    file->replace(file, b.data(), b.size());
}

bool ends_with(const std::string& s, const char* tail) {
    const size_t n = std::strlen(tail);
    return s.size() >= n && s.compare(s.size() - n, n, tail) == 0;
}
bool match_set(const std::string& key) { return key.rfind("/graphics/settings/", 0) == 0 && ends_with(key, ".set"); }
bool match_lod(const std::string& key) { return key.rfind("/graphics/settings/", 0) == 0 && ends_with(key, "/lodpresets.lod"); }

Detail read_detail() {
    Detail d;
    const std::string r = lower(config_get_str("fefw", "reflections", "full"));
    d.reflections = r == "off" || r == "0" ? 2 : r == "reduced" || r == "low" ? 1 : 0;
    d.ao = config_get_bool("fefw", "ambient_occlusion", true);
    d.crowd_reduced = lower(config_get_str("fefw", "crowd_detail", "full")) == "reduced";
    d.scenery_reduced = lower(config_get_str("fefw", "scenery_detail", "full")) == "reduced";
    return d;
}

}  // namespace

void detail_install() {
    g_detail = read_detail();
    static const char* const kRefl[] = {"full", "reduced (8 rays)", "off"};
    if (!g_detail.any()) return;
    const bool sets = g_detail.reflections != 0 || !g_detail.ao, lod = g_detail.crowd_reduced || g_detail.scenery_reduced;
    bool ok = true;
    if (sets) ok = services::modapi::add_runtime_edit("graphics detail", &match_set, &edit_set, nullptr) && ok;
    if (lod) ok = services::modapi::add_runtime_edit("graphics detail", &match_lod, &edit_lod, nullptr) && ok;
    NXLOG_I(kTag, "graphics detail: reflections %s, ambient occlusion %s, crowd detail %s, scenery detail %s%s", kRefl[g_detail.reflections],
            g_detail.ao ? "on" : "off", g_detail.crowd_reduced ? "reduced" : "full", g_detail.scenery_reduced ? "reduced" : "full",
            ok ? " (the game's graphics settings files are edited as they load)" : ": too late to edit the files, applies at the next start");
}

// For the log once the RomFS is built (the clock thread's first line): how many values each change touched.
std::string detail_summary() {
    if (!g_detail.any()) return "";
    char b[256];
    std::snprintf(b, sizeof b, "graphics detail: %d files changed (reflections off in %d, 8 rays in %d, ambient occlusion off in %d, %d crowd and "
                               "%d scenery thresholds)%s",
                  g_counts.files, g_counts.ssr_off, g_counts.ssr_rays, g_counts.ao_off, g_counts.crowd, g_counts.scenery,
                  g_counts.unparsed ? ", some files did not parse" : "");
    return b;
}

}  // namespace nxrt::games::fefw
