// SPDX-License-Identifier: GPL-3.0-or-later
// RomFS image builder (services/fs_romfs). The images are checked by a small parser written in this file from the RomFS layout
// (header, directory and file entries with their child and sibling chains, the two hash tables), not by the builder's own code.
// Only the public RomfsImage interface is used. Host trees are created under the temp directory and removed at the end.
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <winioctl.h>

#include <algorithm>
#include <atomic>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <map>
#include <system_error>
#include <set>
#include <string>
#include <thread>
#include <vector>

#include "../src/services/fs_romfs.h"
#include "test_framework.h"

NXTEST_GROUP(svc_fs_romfs)

namespace {

using nxrt::services::fs::RomfsImage;
constexpr uint32_t kNone = 0xFFFFFFFFu;

// One entry of a host test tree. `path` is an image path ("/dir/name").
struct Item {
    std::string path;
    bool is_dir = false;
    std::vector<uint8_t> data;
};

std::vector<uint8_t> pattern(size_t n, uint32_t seed) {
    std::vector<uint8_t> v(n);
    for (size_t i = 0; i < n; i++) v[i] = static_cast<uint8_t>(i * 131u + seed * 17u + (i >> 9) * 7u);
    return v;
}

std::vector<std::string> split(const std::string& p) {
    std::vector<std::string> out;
    std::string cur;
    for (char c : p) {
        if (c == '/') {
            if (!cur.empty()) out.push_back(cur);
            cur.clear();
        } else {
            cur.push_back(c);
        }
    }
    if (!cur.empty()) out.push_back(cur);
    return out;
}

std::filesystem::path u8path(const std::string& s) {
    return std::filesystem::path(std::u8string(reinterpret_cast<const char8_t*>(s.data()), s.size()));
}

std::string utf8(const std::filesystem::path& p) {
    const std::u8string u = p.u8string();
    return std::string(reinterpret_cast<const char*>(u.data()), u.size());
}

// Host paths of 240 characters or more get the extended-length prefix.
std::filesystem::path long_path(const std::filesystem::path& p) {
    const std::wstring w = p.wstring();
    if (w.size() < 240 || w.compare(0, 4, L"\\\\?\\") == 0) return p;
    if (w.compare(0, 2, L"\\\\") == 0) return std::filesystem::path(L"\\\\?\\UNC\\" + w.substr(2));
    return std::filesystem::path(L"\\\\?\\" + w);
}

// The temp folder for one test, always in extended-length form so that 255-character names fit.
struct TempTree {
    std::filesystem::path root;
    TempTree() {
        static std::atomic<unsigned> seq{0};
        std::error_code ec;
        std::wstring base = std::filesystem::temp_directory_path(ec).wstring();
        while (!base.empty() && base.back() == L'\\') base.pop_back();   // the temp path ends with a separator
        root = std::filesystem::path(L"\\\\?\\" + base + L"\\nxrt_romfs_" + std::to_wstring(GetCurrentProcessId()) + L"_" +
                                     std::to_wstring(seq++));
        std::filesystem::create_directories(root, ec);
    }
    ~TempTree() {
        std::error_code ec;
        std::filesystem::remove_all(root, ec);
    }
};

std::filesystem::path join(const std::filesystem::path& root, const std::string& image_path) {
    std::filesystem::path p = root;
    for (const std::string& c : split(image_path)) p /= u8path(c);
    return long_path(p);
}

void write_host(const std::filesystem::path& root, const std::vector<Item>& items) {
    for (const Item& it : items) {
        const std::filesystem::path p = join(root, it.path);
        if (it.is_dir) {
            std::error_code ec;
            std::filesystem::create_directories(p, ec);
            CHECK_MSG(!ec, it.path);
            continue;
        }
        std::error_code ec;
        std::filesystem::create_directories(p.parent_path(), ec);
        std::ofstream f(p, std::ios::binary | std::ios::trunc);
        f.write(reinterpret_cast<const char*>(it.data.data()), static_cast<std::streamsize>(it.data.size()));
        CHECK_MSG(f.good(), it.path);
    }
}

bool build_from(const std::filesystem::path& dir, RomfsImage& img) {
    std::string err;
    const bool ok = img.build(utf8(dir), &err);
    CHECK_MSG(ok, err);
    return ok;
}

// Names that exercise the tables: a 1-byte name, UTF-8, a 255-byte name, nested folders, an empty folder, and 200 names in one
// folder (more entries than buckets' worth of collisions).
std::vector<Item> nested_tree() {
    std::vector<Item> v;
    v.push_back({"/empty_dir", true, {}});
    v.push_back({"/readme.txt", false, pattern(0, 1)});
    v.push_back({"/caf\xC3\xA9" ".bin", false, pattern(300, 2)});
    v.push_back({"/x", false, pattern(1, 3)});
    v.push_back({"/" + std::string(251, 'q') + ".bin", false, pattern(5000, 4)});
    v.push_back({"/B.bin", false, pattern(17, 5)});
    v.push_back({"/a.bin", false, pattern(33, 6)});
    v.push_back({"/dir_a/sub/deep/leaf.dat", false, pattern(4100, 7)});
    v.push_back({"/dir_a/sub/0.bin", false, pattern(16, 8)});
    v.push_back({"/dir_a/sub/1.bin", false, pattern(1, 9)});
    v.push_back({"/dir_a/sub/emptydir", true, {}});
    for (int i = 0; i < 200; i++) {
        char b[40];
        std::snprintf(b, sizeof b, "/many/f%03d.bin", i);
        v.push_back({b, false, pattern(static_cast<size_t>((i * 37) % 700), 10u + static_cast<uint32_t>(i))});
    }
    return v;
}

// Hash of a name under a directory entry (Switchbrew RomFS: parent ^ 123456789, then per byte rotate right 5 and xor the byte).
uint32_t oracle_hash(uint32_t parent, const std::string& name) {
    uint32_t h = parent ^ 123456789u;
    for (unsigned char c : name) {
        h = (h >> 5) | (h << 27);
        h ^= c;
    }
    return h;
}

// Bucket count rule: 3 below 3 entries, n|1 below 19, else the first value from n not divisible by 2,3,5,7,11,13 or 17.
uint64_t oracle_buckets(uint64_t n) {
    if (n < 3) return 3;
    if (n < 19) return n | 1;
    uint64_t c = n;
    while (c % 2 == 0 || c % 3 == 0 || c % 5 == 0 || c % 7 == 0 || c % 11 == 0 || c % 13 == 0 || c % 17 == 0) c++;
    return c;
}

uint64_t pad4(uint64_t n) { return (n + 3) & ~uint64_t{3}; }

struct Extent {
    std::string path;
    uint64_t offset = 0;   // image offset of the data
    uint64_t size = 0;
};

// The parser. Header: ten u64 (header size 0x50, dir hash ofs/size, dir table ofs/size, file hash ofs/size, file table ofs/size,
// file partition ofs). Directory entry: parent, sibling, child, file, next-in-bucket (u32 each), name length (u32), name padded to 4.
// File entry: parent, sibling (u32), data offset and data size (u64), next-in-bucket (u32), name length (u32), name padded to 4.
// Offsets inside the tables are entry offsets from the table start. 0xFFFFFFFF means none. The root is the entry at offset 0.
class Oracle {
public:
    explicit Oracle(const RomfsImage& img) : img_(img) {}

    bool load() {
        size_ = img_.size();
        uint8_t h[0x50];
        if (size_ < 0x50 || !img_.read(0, h, sizeof h)) return false;
        for (int i = 0; i < 10; i++) hdr_[i] = le(h + 8 * i, 8);
        if (hdr_[0] != 0x50 || hdr_[9] != 0x200 || hdr_[1] > size_) return false;
        meta_.resize(static_cast<size_t>(size_ - hdr_[1]));
        return meta_.empty() || img_.read(hdr_[1], meta_.data(), meta_.size());
    }

    uint64_t hdr(int i) const { return hdr_[i]; }
    uint64_t image_size() const { return size_; }
    bool bad() const { return bad_; }

    // Layout facts from the header alone: alignment, contiguity, table sizes against the entry counts.
    void check_layout(uint64_t dirs, uint64_t files) {
        CHECK_EQ(hdr_[0], 0x50ull);
        CHECK_EQ(hdr_[9], 0x200ull);
        for (int i = 1; i <= 8; i++) CHECK_MSG(hdr_[i] % 4 == 0, "table field not 4-byte aligned");
        CHECK_EQ(hdr_[1] + hdr_[2], hdr_[3]);
        CHECK_EQ(hdr_[3] + hdr_[4], hdr_[5]);
        CHECK_EQ(hdr_[5] + hdr_[6], hdr_[7]);
        CHECK_EQ(hdr_[7] + hdr_[8], size_);
        CHECK_EQ(hdr_[2] / 4, oracle_buckets(dirs));
        CHECK_EQ(hdr_[6] / 4, oracle_buckets(files));
    }

    // Walks the child and sibling chains from the root: every reachable entry once, every link confirmed through the hash tables.
    bool walk(std::vector<Extent>& files, std::vector<std::string>& dirs) {
        WalkState w;
        w.dir_bytes = 0x18;
        walk_dir(0, "", w, 0);
        if (bad_) return false;
        if (w.dir_bytes != hdr_[4] || w.file_bytes != hdr_[8]) {
            CHECK_MSG(false, "entry bytes reached by the walk differ from the table sizes");
            return false;
        }
        files = std::move(w.files);
        dirs = std::move(w.dirs);
        return true;
    }

    // Entry of directory `name` under `parent`, found through the directory hash table. kNone when absent.
    uint32_t find_dir(uint32_t parent, const std::string& name) {
        const uint64_t buckets = hdr_[2] / 4;
        if (buckets == 0) {
            bad_ = true;
            return kNone;
        }
        uint32_t e = u32(hdr_[1] + 4 * (oracle_hash(parent, name) % buckets));
        for (uint64_t steps = 0; e != kNone && steps <= hdr_[4]; steps++) {
            if (dparent(e) == parent && dname(e) == name) return e;
            e = dnext(e);
        }
        return kNone;
    }

    // Entry of file `name` under directory entry `parent`, through the file hash table.
    uint32_t find_file(uint32_t parent, const std::string& name) {
        const uint64_t buckets = hdr_[6] / 4;
        if (buckets == 0) {
            bad_ = true;
            return kNone;
        }
        uint32_t e = u32(hdr_[5] + 4 * (oracle_hash(parent, name) % buckets));
        for (uint64_t steps = 0; e != kNone && steps <= hdr_[8]; steps++) {
            if (fparent(e) == parent && fname(e) == name) return e;
            e = fnext(e);
        }
        return kNone;
    }

    // Directory entry for an image path ("/" is the root). kNone when absent.
    uint32_t resolve_dir(const std::string& path) {
        uint32_t cur = 0;
        for (const std::string& c : split(path)) {
            cur = find_dir(cur, c);
            if (cur == kNone) return kNone;
        }
        return cur;
    }

    // Image path to (offset, size), through the hash tables. False when absent.
    bool resolve_file(const std::string& path, uint64_t* offset, uint64_t* size) {
        std::vector<std::string> parts = split(path);
        if (parts.empty()) return false;
        std::string leaf = parts.back();
        parts.pop_back();
        std::string dir_path;
        for (const std::string& p : parts) dir_path += "/" + p;
        const uint32_t d = resolve_dir(dir_path.empty() ? "/" : dir_path);
        if (d == kNone) return false;
        const uint32_t f = find_file(d, leaf);
        if (f == kNone) return false;
        *offset = hdr_[9] + fdata(f);
        *size = fsize(f);
        return true;
    }

    // Longest chain in the file hash table.
    uint64_t max_file_chain() {
        uint64_t best = 0;
        const uint64_t buckets = hdr_[6] / 4;
        for (uint64_t b = 0; b < buckets && !bad_; b++) {
            uint64_t n = 0;
            for (uint32_t e = u32(hdr_[5] + 4 * b); e != kNone && n <= hdr_[8]; e = fnext(e)) n++;
            best = std::max(best, n);
        }
        return best;
    }

private:
    struct WalkState {
        std::set<uint32_t> seen_dirs, seen_files;
        uint64_t dir_bytes = 0, file_bytes = 0;
        std::vector<Extent> files;
        std::vector<std::string> dirs;
    };

    void walk_dir(uint32_t dir, const std::string& prefix, WalkState& w, int depth) {
        if (bad_) return;
        if (depth > 200 || !w.seen_dirs.insert(dir).second) {
            bad_ = true;
            return;
        }
        w.dirs.push_back(prefix.empty() ? std::string("/") : prefix);
        for (uint32_t c = dchild(dir); c != kNone && !bad_; c = dsibling(c)) {
            const std::string name = dname(c);
            w.dir_bytes += 0x18 + pad4(name.size());
            if (dparent(c) != dir || find_dir(dir, name) != c) {
                bad_ = true;
                return;
            }
            walk_dir(c, prefix + "/" + name, w, depth + 1);
        }
        for (uint32_t f = dfile(dir); f != kNone && !bad_; f = fsibling(f)) {
            const std::string name = fname(f);
            w.file_bytes += 0x20 + pad4(name.size());
            if (fparent(f) != dir || find_file(dir, name) != f || !w.seen_files.insert(f).second) {
                bad_ = true;
                return;
            }
            w.files.push_back({prefix + "/" + name, hdr_[9] + fdata(f), fsize(f)});
        }
    }

    static uint64_t le(const uint8_t* p, int n) {
        uint64_t v = 0;
        for (int i = n - 1; i >= 0; i--) v = (v << 8) | p[i];
        return v;
    }
    uint64_t get(uint64_t at, int n) {
        if (at < hdr_[1] || at + static_cast<uint64_t>(n) > size_) {
            bad_ = true;
            return 0;
        }
        return le(meta_.data() + (at - hdr_[1]), n);
    }
    uint32_t u32(uint64_t at) { return static_cast<uint32_t>(get(at, 4)); }
    std::string name_at(uint64_t at, uint64_t n) {
        if (at < hdr_[1] || at + n > size_) {
            bad_ = true;
            return {};
        }
        return std::string(reinterpret_cast<const char*>(meta_.data() + (at - hdr_[1])), static_cast<size_t>(n));
    }

    uint32_t dparent(uint32_t e) { return u32(hdr_[3] + e); }
    uint32_t dsibling(uint32_t e) { return u32(hdr_[3] + e + 4); }
    uint32_t dchild(uint32_t e) { return u32(hdr_[3] + e + 8); }
    uint32_t dfile(uint32_t e) { return u32(hdr_[3] + e + 12); }
    uint32_t dnext(uint32_t e) { return u32(hdr_[3] + e + 16); }
    std::string dname(uint32_t e) { return name_at(hdr_[3] + e + 24, u32(hdr_[3] + e + 20)); }

    uint32_t fparent(uint32_t e) { return u32(hdr_[7] + e); }
    uint32_t fsibling(uint32_t e) { return u32(hdr_[7] + e + 4); }
    uint64_t fdata(uint32_t e) { return get(hdr_[7] + e + 8, 8); }
    uint64_t fsize(uint32_t e) { return get(hdr_[7] + e + 16, 8); }
    uint32_t fnext(uint32_t e) { return u32(hdr_[7] + e + 24); }
    std::string fname(uint32_t e) { return name_at(hdr_[7] + e + 32, u32(hdr_[7] + e + 28)); }

    const RomfsImage& img_;
    uint64_t size_ = 0;
    uint64_t hdr_[10] = {};
    std::vector<uint8_t> meta_;   // image bytes from dir_hash_table_ofs to the end
    bool bad_ = false;
};

// Full check of an image against the host tree it was built from: the walk, the hash lookups, the header, the data.
void check_tree(RomfsImage& img, const std::vector<Item>& items) {
    Oracle o(img);
    if (!o.load()) {
        CHECK_MSG(false, "image metadata unreadable");
        return;
    }
    std::vector<Extent> found;
    std::vector<std::string> dirs;
    if (!o.walk(found, dirs)) {
        CHECK_MSG(false, "tables inconsistent");
        return;
    }
    o.check_layout(dirs.size(), found.size());

    std::map<std::string, const Item*> want_files;
    std::set<std::string> want_dirs{"/"};
    for (const Item& it : items) {
        // every folder on the way to an entry is a directory entry; an explicit folder is one too
        const std::vector<std::string> parts = split(it.path);
        const size_t folders = it.is_dir ? parts.size() : parts.size() - 1;
        std::string acc;
        for (size_t i = 0; i < folders; i++) {
            acc += "/" + parts[i];
            want_dirs.insert(acc);
        }
        if (!it.is_dir) want_files[it.path] = &it;
    }
    CHECK_EQ(found.size(), want_files.size());
    CHECK_EQ(dirs.size(), want_dirs.size());
    for (const std::string& d : dirs) CHECK_MSG(want_dirs.count(d) == 1, d);

    for (const auto& [path, item] : want_files) {
        uint64_t off = 0, sz = 0;
        CHECK_MSG(o.resolve_file(path, &off, &sz), path);
        CHECK_EQ(sz, item->data.size());
        uint64_t iofs = 0, isz = 0;
        CHECK_MSG(img.file_extent(path, &iofs, &isz), path);
        CHECK_EQ(iofs, off);
        CHECK_EQ(isz, sz);
        CHECK_MSG(off >= 0x200 && (off - 0x200) % 16 == 0, path);
        CHECK_MSG(off + sz <= o.hdr(1), path);
        std::vector<uint8_t> got(static_cast<size_t>(sz));
        CHECK_MSG(sz == 0 || img.read(off, got.data(), got.size()), path);
        CHECK_MSG(got == item->data, path);
    }
    for (const Extent& e : found) CHECK_MSG(want_files.count(e.path) == 1, e.path);
}

std::vector<uint8_t> read_all(RomfsImage& img) {
    std::vector<uint8_t> v(static_cast<size_t>(img.size()));
    if (!v.empty()) CHECK(img.read(0, v.data(), v.size()));
    return v;
}

}  // namespace

NXTEST(romfs_empty_folder) {
    TempTree t;
    const auto dir = t.root / "empty";
    std::error_code ec;
    std::filesystem::create_directories(dir, ec);
    RomfsImage img;
    REQUIRE(build_from(dir, img));
    CHECK(img.built());
    // header 0x50 padded to 0x200, one bucket group of 3 for the directories and 3 for the files, one 0x18-byte root entry
    CHECK_EQ(img.size(), 0x230ull);
    CHECK_EQ(img.stats().files, 0u);
    CHECK_EQ(img.stats().dirs, 1u);
    CHECK_EQ(img.stats().data_bytes, 0ull);
    CHECK_EQ(img.stats().data_offset, 0x200ull);
    Oracle o(img);
    REQUIRE(o.load());
    std::vector<Extent> files;
    std::vector<std::string> dirs;
    REQUIRE(o.walk(files, dirs));
    CHECK_EQ(files.size(), 0u);
    CHECK_EQ(dirs.size(), 1u);
    CHECK_EQ(o.find_dir(0, ""), 0u);
    o.check_layout(1, 0);
    CHECK_EQ(o.resolve_dir("/"), 0u);
}

NXTEST(romfs_missing_folder_is_error) {
    TempTree t;
    RomfsImage img;
    std::string err;
    CHECK(!img.build(utf8(t.root / "does_not_exist"), &err));
    CHECK(!img.built());
    CHECK(!err.empty());
    // a file is not a folder
    std::vector<Item> one{{"/f.bin", false, pattern(4, 1)}};
    write_host(t.root, one);
    CHECK(!img.build(utf8(t.root / "f.bin"), &err));
    CHECK(!img.built());
}

NXTEST(romfs_nested_tree_every_file_found) {
    TempTree t;
    const std::vector<Item> items = nested_tree();
    write_host(t.root, items);
    RomfsImage img;
    REQUIRE(build_from(t.root, img));
    CHECK(img.built());
    CHECK_EQ(img.stats().files, 6u + 3u + 200u);
    CHECK_EQ(img.stats().dirs, 7u);   // root, empty_dir, dir_a, dir_a/sub, dir_a/sub/deep, dir_a/sub/emptydir, many
    check_tree(img, items);
    // the 200 names in /many share buckets of the file hash table
    Oracle o(img);
    REQUIRE(o.load());
    CHECK(o.max_file_chain() >= 2);
}

NXTEST(romfs_header_fields_consistent) {
    // three shapes: the empty root, one file of odd size, and a tree whose data ends before the padding to 4
    {
        TempTree t;
        std::error_code ec;
        std::filesystem::create_directories(t.root / "e", ec);
        RomfsImage img;
        REQUIRE(build_from(t.root / "e", img));
        Oracle o(img);
        REQUIRE(o.load());
        o.check_layout(1, 0);
    }
    {
        TempTree t;
        std::vector<Item> items{{"/one.bin", false, pattern(7, 3)}};
        write_host(t.root, items);
        RomfsImage img;
        REQUIRE(build_from(t.root, img));
        Oracle o(img);
        REQUIRE(o.load());
        o.check_layout(1, 1);
        uint64_t off = 0, sz = 0;
        CHECK(img.file_extent("/one.bin", &off, &sz));
        CHECK_EQ(off, 0x200ull);
        CHECK_EQ(sz, 7ull);
        check_tree(img, items);
    }
    {
        TempTree t;
        const std::vector<Item> items = nested_tree();
        write_host(t.root, items);
        RomfsImage img;
        REQUIRE(build_from(t.root, img));
        Oracle o(img);
        REQUIRE(o.load());
        std::vector<Extent> files;
        std::vector<std::string> dirs;
        REQUIRE(o.walk(files, dirs));
        o.check_layout(dirs.size(), files.size());
        check_tree(img, items);
    }
}

NXTEST(romfs_odd_piece_reads_equal_whole) {
    TempTree t;
    const std::vector<Item> items = nested_tree();
    write_host(t.root, items);
    RomfsImage img;
    REQUIRE(build_from(t.root, img));
    const uint64_t n = img.size();
    REQUIRE(n > 0);
    const std::vector<uint8_t> whole = read_all(img);
    static const uint64_t steps[] = {1, 3, 7, 16, 33, 511, 1000, 4097, 9};
    std::vector<uint8_t> pieces(static_cast<size_t>(n));
    uint64_t at = 0;
    size_t k = 0;
    while (at < n) {
        const uint64_t len = std::min<uint64_t>(steps[k++ % 9], n - at);
        REQUIRE(img.read(at, pieces.data() + at, len));
        at += len;
    }
    CHECK(pieces == whole);
    check_tree(img, items);
}

NXTEST(romfs_read_past_end_fails) {
    TempTree t;
    const std::vector<Item> items = nested_tree();
    write_host(t.root, items);
    RomfsImage img;
    REQUIRE(build_from(t.root, img));
    const uint64_t n = img.size();
    uint8_t b[16] = {};
    CHECK(img.read(n - 1, b, 1));
    CHECK(!img.read(n, b, 1));
    CHECK(!img.read(n - 1, b, 2));
    CHECK(!img.read(n + 5, b, 1));
    CHECK(!img.read(~uint64_t{0} - 2, b, 8));   // offset + len wraps
    RomfsImage never;
    CHECK(!never.read(0, b, 1));
}

NXTEST(romfs_read_spanning_areas) {
    TempTree t;
    const std::vector<Item> items = nested_tree();
    write_host(t.root, items);
    RomfsImage img;
    REQUIRE(build_from(t.root, img));
    const std::vector<uint8_t> whole = read_all(img);
    Oracle o(img);
    REQUIRE(o.load());
    std::vector<Extent> files;
    std::vector<std::string> dirs;
    REQUIRE(o.walk(files, dirs));
    std::sort(files.begin(), files.end(), [](const Extent& a, const Extent& b) { return a.offset < b.offset; });
    REQUIRE(files.size() >= 3);

    auto expect_span = [&](uint64_t at, uint64_t len) {
        std::vector<uint8_t> got(static_cast<size_t>(len), 0xEE);
        REQUIRE(img.read(at, got.data(), got.size()));
        CHECK(std::memcmp(got.data(), whole.data() + at, static_cast<size_t>(len)) == 0);
    };
    // header and the padding up to the first file
    expect_span(0x40, files.front().offset + 8 - 0x40);
    // end of one file, the alignment gap, start of the next: three bytes before the first end, five bytes into the next file
    bool gap_found = false;
    for (size_t i = 0; i + 1 < files.size() && !gap_found; i++) {
        const uint64_t end = files[i].offset + files[i].size;
        if (files[i].size >= 3 && files[i + 1].offset > end) {
            const uint64_t from = end - 3;
            expect_span(from, files[i + 1].offset + 5 - from);
            gap_found = true;
        }
    }
    CHECK(gap_found);
    // last file's data into the tables
    const Extent& last = files.back();
    expect_span(last.offset + last.size - 6, o.hdr(1) + 20 - (last.offset + last.size - 6));
    // the whole image in one read
    expect_span(0, img.size());
}

NXTEST(romfs_file_over_4gib) {
    TempTree t;
    const uint64_t kBig = 5ull << 30;
    const auto big = t.root / "a_big.bin";
    HANDLE h = CreateFileW(big.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (h == INVALID_HANDLE_VALUE) {
        nxtest::skip("cannot create the 5 GiB test file");
        return;
    }
    // sparse, so that the 5 GiB take no disk space; without sparse support the test is skipped rather than filling the volume
    DWORD ret = 0;
    const bool sparse = DeviceIoControl(h, FSCTL_SET_SPARSE, nullptr, 0, nullptr, 0, &ret, nullptr) != 0;
    LARGE_INTEGER len{};
    len.QuadPart = static_cast<LONGLONG>(kBig);
    const bool sized = sparse && SetFilePointerEx(h, len, nullptr, FILE_BEGIN) && SetEndOfFile(h);
    if (!sized) {
        CloseHandle(h);
        nxtest::skip("the volume does not take a sparse 5 GiB file");
        return;
    }
    // markers around the 4 GiB point of the file: image offset 0x100000000 is file offset 0xFFFFFE00
    const std::vector<uint8_t> marker = pattern(64, 99);
    LARGE_INTEGER at{};
    at.QuadPart = 0xFFFFFF00ll;
    DWORD wrote = 0;
    bool ok = SetFilePointerEx(h, at, nullptr, FILE_BEGIN) && WriteFile(h, marker.data(), 64, &wrote, nullptr) && wrote == 64;
    CloseHandle(h);
    CHECK(ok);
    const std::vector<uint8_t> after = pattern(4100, 5);
    {
        std::ofstream f(t.root / "z_after.bin", std::ios::binary);
        f.write(reinterpret_cast<const char*>(after.data()), static_cast<std::streamsize>(after.size()));
    }

    RomfsImage img;
    REQUIRE(build_from(t.root, img));
    CHECK_EQ(img.stats().files, 2u);
    uint64_t zoff = 0, zsz = 0;
    REQUIRE(img.file_extent("/z_after.bin", &zoff, &zsz));
    CHECK(zoff > 0xFFFFFFFFull);
    CHECK_EQ(zoff, 0x200ull + kBig);
    CHECK_EQ(zsz, 4100ull);
    uint64_t boff = 0, bsz = 0;
    REQUIRE(img.file_extent("/a_big.bin", &boff, &bsz));
    CHECK_EQ(boff, 0x200ull);
    CHECK_EQ(bsz, kBig);

    // a read that crosses image offset 4 GiB, compared with the host file bytes at the same place
    std::ifstream host(big, std::ios::binary);
    REQUIRE(host.good());
    const uint64_t win_img = 0x100000000ull - 0x2000;
    std::vector<uint8_t> got(0x4000), want(0x4000);
    REQUIRE(img.read(win_img, got.data(), got.size()));
    host.seekg(static_cast<std::streamoff>(win_img - 0x200));
    host.read(reinterpret_cast<char*>(want.data()), static_cast<std::streamsize>(want.size()));
    CHECK(host.gcount() == static_cast<std::streamsize>(want.size()));
    CHECK(got == want);
    // the marker written just above 4 GiB lies inside that window
    CHECK(std::memcmp(got.data() + (0x100000100ull - win_img), marker.data(), marker.size()) == 0);

    // the end of the big file runs straight into the next file: 8 zero bytes, then the next file's data
    std::vector<uint8_t> seam(8 + after.size());
    REQUIRE(img.read(zoff - 8, seam.data(), seam.size()));
    CHECK(std::memcmp(seam.data() + 8, after.data(), after.size()) == 0);
    for (int i = 0; i < 8; i++) CHECK_EQ(seam[i], 0);
}

NXTEST(romfs_concurrent_reads_identical) {
    TempTree t;
    std::vector<Item> items;
    for (int i = 0; i < 40; i++) {
        char b[32];
        std::snprintf(b, sizeof b, "/part_%02d.bin", i);
        items.push_back({b, false, pattern(static_cast<size_t>((i * 5003) % 200000 + i), static_cast<uint32_t>(i))});
    }
    write_host(t.root, items);
    RomfsImage img;
    REQUIRE(build_from(t.root, img));
    const uint64_t n = img.size();
    const std::vector<uint8_t> ref = read_all(img);
    // 40 files against 16 cached handles: the cache has to evict while four readers run
    std::atomic<int> bad{0};
    auto worker = [&](uint32_t seed) {
        uint32_t x = seed * 2654435761u + 1;
        std::vector<uint8_t> buf;
        for (int r = 0; r < 300; r++) {
            x = x * 1103515245u + 12345u;
            const uint64_t off = static_cast<uint64_t>(x >> 3) % n;
            x = x * 1103515245u + 12345u;
            const uint64_t len = std::min<uint64_t>(1 + (x >> 4) % 70000u, n - off);
            buf.assign(static_cast<size_t>(len), 0);
            if (!img.read(off, buf.data(), len) || std::memcmp(buf.data(), ref.data() + off, static_cast<size_t>(len)) != 0) bad++;
        }
        // one full pass in pieces
        for (uint64_t off = 0; off < n; off += 4097) {
            const uint64_t len = std::min<uint64_t>(4097, n - off);
            buf.assign(static_cast<size_t>(len), 0);
            if (!img.read(off, buf.data(), len) || std::memcmp(buf.data(), ref.data() + off, static_cast<size_t>(len)) != 0) bad++;
        }
    };
    std::vector<std::thread> threads;
    for (uint32_t i = 0; i < 4; i++) threads.emplace_back(worker, i + 1);
    for (std::thread& th : threads) th.join();
    CHECK_EQ(bad.load(), 0);
}

// A minimal .nup (SPEC.md v1, written here by hand): one update package whose romfs holds /a.bin and /d/b.bin, stored raw. The image
// built from the package must serve the same bytes as the files; a compressed file is refused.
namespace {
std::vector<uint8_t> tiny_nup(const std::vector<uint8_t>& a, const std::vector<uint8_t>& b, uint8_t codec) {
    auto put = [](std::vector<uint8_t>& v, size_t o, uint64_t x, int n) { for (int i = 0; i < n; i++) v[o + i] = static_cast<uint8_t>(x >> (8 * i)); };
    const std::string pool = "romfsda.binb.bin";                  // romfs@0 d@5 a.bin@6 b.bin@11
    const size_t dirs = 3, files = 2, slots = 4;
    const size_t dir_at = 0x30 + 0x40, file_at = dir_at + 0x18 * dirs, child_at = file_at + 0x50 * files, pool_at = child_at + 4 * slots;
    const size_t toc_size = pool_at + pool.size(), blob = (0x80 + toc_size + 15) & ~size_t(15);
    const size_t a_at = blob, b_at = (a_at + a.size() + 15) & ~size_t(15), total = b_at + b.size();
    std::vector<uint8_t> v(total, 0);
    std::memcpy(v.data(), "NUP\x01", 4);
    put(v, 0x04, 0x80, 2); put(v, 0x06, 1, 2); put(v, 0x08, 0x80, 8); put(v, 0x10, toc_size, 8); put(v, 0x38, blob, 8);
    put(v, 0x40, total, 8); put(v, 0x48, 1, 4);
    const size_t t = 0x80;
    std::memcpy(v.data() + t, "TOC\x01", 4);
    put(v, t + 4, 1, 4); put(v, t + 8, dirs, 8); put(v, t + 0x10, files, 8); put(v, t + 0x18, pool_at, 8); put(v, t + 0x20, pool.size(), 8);
    put(v, t + 0x30 + 0x08, 0, 4); put(v, t + 0x30 + 0x0C, 0xFFFFFFFFu, 4); put(v, t + 0x30 + 0x10, 1, 4);   // root 0, no exefs, romfs 1
    put(v, t + 0x30 + 0x14, 655360, 4); v[t + 0x30 + 0x18] = 1;                                            // update
    const uint32_t dir_rec[3][4] = {{0, 0, 0, 1}, {0, 5, 1, 2}, {5, 1, 3, 1}};                             // name off, len, first, count
    for (size_t i = 0; i < dirs; i++) for (int k = 0; k < 4; k++) put(v, t + dir_at + 0x18 * i + 4 * k, dir_rec[i][k], 4);
    const uint64_t fr[2][4] = {{6, 5, a.size(), a_at}, {11, 5, b.size(), b_at}};
    for (size_t i = 0; i < files; i++) {
        const size_t r = t + file_at + 0x50 * i;
        put(v, r, fr[i][0], 4); put(v, r + 4, fr[i][1], 4); put(v, r + 8, fr[i][2], 8); put(v, r + 0x10, fr[i][3], 8); put(v, r + 0x18, fr[i][2], 8);
        v[r + 0x40] = codec;
    }
    const uint32_t kids[4] = {1, 3, 2, 4};                       // root: romfs; romfs: a.bin, d; d: b.bin (ids >= 3 are files)
    for (size_t i = 0; i < slots; i++) put(v, t + child_at + 4 * i, kids[i], 4);
    std::memcpy(v.data() + t + pool_at, pool.data(), pool.size());
    std::memcpy(v.data() + a_at, a.data(), a.size());
    std::memcpy(v.data() + b_at, b.data(), b.size());
    return v;
}
}  // namespace

NXTEST(romfs_from_nup_package) {
    TempTree t;
    const std::vector<uint8_t> a = pattern(5000, 1), b = pattern(70000, 2);
    const auto path = t.root / "game.nup";
    for (uint8_t codec : {uint8_t(0), uint8_t(1)}) {
        const std::vector<uint8_t> bytes = tiny_nup(a, b, codec);
        { std::ofstream(path, std::ios::binary | std::ios::trunc).write(reinterpret_cast<const char*>(bytes.data()), bytes.size()); }
        RomfsImage img;
        std::string err;
        if (codec) {
            CHECK(!img.build(utf8(path), &err));
            CHECK(err.find("compressed") != std::string::npos);
            continue;
        }
        REQUIRE(img.build(utf8(path), &err));
        CHECK_EQ(img.stats().files, 2u);
        CHECK_EQ(img.stats().dirs, 2u);
        for (const auto& [p, want] : {std::pair{std::string("/a.bin"), &a}, std::pair{std::string("/d/b.bin"), &b}}) {
            uint64_t off = 0, size = 0;
            REQUIRE(img.file_extent(p, &off, &size));
            CHECK_EQ(size, want->size());
            std::vector<uint8_t> got(size);
            REQUIRE(img.read(off, got.data(), size));
            CHECK(got == *want);
        }
        Oracle o(img);
        REQUIRE(o.load());
        o.check_layout(2, 2);
    }
}

// RomFS mods: overlays laid over the game's tree. A file at the same path (ignoring ASCII case) takes the overlay's data and keeps the
// game's spelling, new paths are added, a later overlay wins over an earlier one, and a missing overlay is skipped.
NXTEST(romfs_overlays_replace_and_add) {
    TempTree base, mod1, mod2;
    write_host(base.root, {{"/Data/Unit.bin", false, pattern(100, 1)}, {"/Data/Keep.bin", false, pattern(50, 2)}, {"/top.txt", false, pattern(7, 3)}});
    write_host(mod1.root,
               {{"/data/unit.bin", false, pattern(300, 4)}, {"/Data/New/added.bin", false, pattern(20, 5)}, {"/top.txt", false, pattern(9, 6)}});
    write_host(mod2.root, {{"/top.txt", false, pattern(11, 7)}});
    RomfsImage img;
    std::string err;
    const std::vector<std::string> overlays{utf8(mod1.root), utf8(base.root / L"does_not_exist"), utf8(mod2.root)};
    REQUIRE(img.build(utf8(base.root), &err, overlays));
    CHECK_EQ(img.stats().overlay_files, 4u);   // unit.bin, added.bin and top.txt from mod1, top.txt again from mod2
    check_tree(img, {{"/Data/Unit.bin", false, pattern(300, 4)},
                     {"/Data/Keep.bin", false, pattern(50, 2)},
                     {"/Data/New/added.bin", false, pattern(20, 5)},
                     {"/top.txt", false, pattern(11, 7)}});
    uint64_t off = 0, sz = 0;
    CHECK(!img.file_extent("/data/unit.bin", &off, &sz));   // the game's spelling is kept
}

// An overlay file where the game has a folder replaces the folder.
NXTEST(romfs_overlay_file_replaces_folder) {
    TempTree base, mod;
    write_host(base.root, {{"/a/b.bin", false, pattern(10, 1)}, {"/c.bin", false, pattern(10, 2)}});
    write_host(mod.root, {{"/a", false, pattern(5, 3)}});
    RomfsImage img;
    std::string err;
    REQUIRE(img.build(utf8(base.root), &err, {utf8(mod.root)}));
    check_tree(img, {{"/a", false, pattern(5, 3)}, {"/c.bin", false, pattern(10, 2)}});
}
