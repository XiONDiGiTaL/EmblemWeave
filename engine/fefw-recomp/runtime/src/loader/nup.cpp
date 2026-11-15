// SPDX-License-Identifier: GPL-3.0-or-later
// .nup package reader (nxrt/nup.h). Layout from the format's SPEC.md (github.com/a55uka/nup): a 0x80-byte superblock, the table of
// contents at toc_offset (package records 0x40, dir records 0x18, file records 0x50, a u32 children array, the string pool), file
// extents at absolute offsets. The package is a file the player supplies: every index, name and extent is bounds checked.
#include "nxrt/nup.h"

#include <cstring>
#include <filesystem>
#include <fstream>

namespace nxrt::nup {
namespace {

constexpr uint32_t kNoDir = 0xFFFFFFFFu;
constexpr uint8_t kKindBase = 0, kKindUpdate = 1;
constexpr uint64_t kMaxToc = 1ull << 30;
constexpr int kMaxDepth = 64;

std::filesystem::path fs_path(const std::string& utf8) { return std::filesystem::path(std::u8string(utf8.begin(), utf8.end())); }

bool fail(std::string* err, std::string msg) {
    if (err) *err = std::move(msg);
    return false;
}

uint32_t u32(const std::vector<uint8_t>& b, uint64_t o) {
    uint32_t v;
    std::memcpy(&v, b.data() + o, 4);
    return v;
}
uint64_t u64(const std::vector<uint8_t>& b, uint64_t o) {
    uint64_t v;
    std::memcpy(&v, b.data() + o, 8);
    return v;
}

struct Toc {
    std::vector<uint8_t> b;
    uint64_t dirs = 0, files = 0, dir_at = 0, file_at = 0, child_at = 0, child_n = 0, pool_at = 0, pool_n = 0, file_size = 0;
};

bool name_of(const Toc& t, uint64_t rec, std::string& out) {
    const uint64_t off = u32(t.b, rec), len = u32(t.b, rec + 4);
    if (off > t.pool_n || len > t.pool_n - off || len == 0) return false;
    out.assign(reinterpret_cast<const char*>(t.b.data() + t.pool_at + off), len);
    return out != "." && out != ".." && out.find_first_of("/\\:") == std::string::npos && out.find('\0') == std::string::npos;
}

// Lists the files under dir (paths relative to it).
bool walk(const Toc& t, uint32_t dir, const std::string& prefix, int depth, std::vector<File>& out, std::string* err) {
    if (depth > kMaxDepth || dir >= t.dirs) return fail(err, "the .nup folder tree is invalid");
    const uint64_t rec = t.dir_at + 0x18ull * dir;
    const uint64_t first = u32(t.b, rec + 8), count = u32(t.b, rec + 12);
    if (first > t.child_n || count > t.child_n - first) return fail(err, "the .nup folder listing is out of bounds");
    for (uint64_t s = first; s < first + count; s++) {
        const uint32_t id = u32(t.b, t.child_at + 4 * s);
        std::string name;
        if (id < t.dirs) {
            if (!name_of(t, t.dir_at + 0x18ull * id, name)) return fail(err, "the .nup holds an invalid folder name");
            if (!walk(t, id, prefix + name + "/", depth + 1, out, err)) return false;
            continue;
        }
        if (id - t.dirs >= t.files) return fail(err, "the .nup refers to a file that does not exist");
        const uint64_t fr = t.file_at + 0x50ull * (id - t.dirs);
        if (!name_of(t, fr, name)) return fail(err, "the .nup holds an invalid file name");
        File f;
        f.path = prefix + name;
        f.size = u64(t.b, fr + 8);
        f.offset = u64(t.b, fr + 0x10);
        const uint64_t stored = u64(t.b, fr + 0x18);
        const uint8_t codec = t.b[fr + 0x40];
        if (f.size > 0 && codec != 0)
            return fail(err, f.path + " is compressed in the .nup; the game reads packages directly and needs stored files. Repack without "
                             "compression (nup pack with the zstandard module unavailable writes stored files)");
        if (f.size > 0 && (stored != f.size || f.offset > t.file_size || f.size > t.file_size - f.offset))
            return fail(err, f.path + " points outside the .nup file");
        out.push_back(std::move(f));
    }
    return true;
}

}  // namespace

bool is_nup(const std::string& path) {
    std::ifstream f(fs_path(path), std::ios::binary);
    char m[4] = {};
    return f.read(m, 4) && std::memcmp(m, "NUP\x01", 4) == 0;
}

bool open(const std::string& path, Content& out, std::string* err) {
    out = Content{};
    std::ifstream f(fs_path(path), std::ios::binary | std::ios::ate);
    if (!f) return fail(err, "cannot open " + path);
    Toc t;
    const uint64_t real = static_cast<uint64_t>(f.tellg());
    std::vector<uint8_t> sb(0x80);
    f.seekg(0);
    if (!f.read(reinterpret_cast<char*>(sb.data()), 0x80) || std::memcmp(sb.data(), "NUP\x01", 4) != 0) return fail(err, path + " is not a .nup package");
    const uint64_t toc_off = u64(sb, 0x08), toc_size = u64(sb, 0x10);
    t.file_size = u64(sb, 0x40);
    if (u32(sb, 0x4C) != 0) return fail(err, path + " uses .nup features this build does not know");
    if (t.file_size != real || toc_size < 0x30 || toc_size > kMaxToc || toc_off > real || toc_size > real - toc_off)
        return fail(err, path + " is damaged or incomplete (size mismatch)");
    t.b.resize(toc_size);
    f.seekg(static_cast<std::streamoff>(toc_off));
    if (!f.read(reinterpret_cast<char*>(t.b.data()), static_cast<std::streamsize>(toc_size)) || std::memcmp(t.b.data(), "TOC\x01", 4) != 0)
        return fail(err, path + ": the table of contents is invalid");

    const uint64_t pkgs = u32(t.b, 4);
    t.dirs = u64(t.b, 8);
    t.files = u64(t.b, 0x10);
    t.pool_at = u64(t.b, 0x18);
    t.pool_n = u64(t.b, 0x20);
    if (pkgs > toc_size / 0x40 || t.dirs > toc_size / 0x18 || t.files > toc_size / 0x50) return fail(err, path + ": the table of contents is invalid");
    t.dir_at = 0x30 + 0x40 * pkgs;
    t.file_at = t.dir_at + 0x18 * t.dirs;
    t.child_at = t.file_at + 0x50 * t.files;
    if (t.child_at > t.pool_at || (t.pool_at - t.child_at) % 4 || t.pool_at > toc_size || t.pool_n > toc_size - t.pool_at)
        return fail(err, path + ": the table of contents is invalid");
    t.child_n = (t.pool_at - t.child_at) / 4;

    // The package to run: the highest update, else the base. ExeFS and RomFS may come from different packages (an update without one).
    int64_t best_exe = -1, best_rom = -1;
    uint32_t exe_dir = kNoDir, rom_dir = kNoDir;
    for (uint64_t i = 0; i < pkgs; i++) {
        const uint64_t r = 0x30 + 0x40 * i;
        const uint8_t kind = t.b[r + 0x18];
        if (kind != kKindBase && kind != kKindUpdate) continue;
        const int64_t rank = kind == kKindBase ? 0 : 1 + static_cast<int64_t>(u32(t.b, r + 0x14));
        if (u32(t.b, r + 0x0C) != kNoDir && rank > best_exe) best_exe = rank, exe_dir = u32(t.b, r + 0x0C);
        if (u32(t.b, r + 0x10) != kNoDir && rank > best_rom) best_rom = rank, rom_dir = u32(t.b, r + 0x10), out.version = u32(t.b, r + 0x14);
    }
    if (exe_dir == kNoDir && rom_dir == kNoDir) return fail(err, path + " holds no base game or update");
    if (exe_dir != kNoDir && !walk(t, exe_dir, "", 0, out.exefs, err)) return false;
    if (rom_dir != kNoDir && !walk(t, rom_dir, "", 0, out.romfs, err)) return false;
    return true;
}

bool read_file(const std::string& path, const File& file, std::vector<uint8_t>& out, std::string* err) {
    std::ifstream f(fs_path(path), std::ios::binary);
    out.resize(static_cast<size_t>(file.size));
    f.seekg(static_cast<std::streamoff>(file.offset));
    if (!f || !f.read(reinterpret_cast<char*>(out.data()), static_cast<std::streamsize>(file.size)))
        return fail(err, "cannot read " + file.path + " from " + path);
    return true;
}

}  // namespace nxrt::nup
