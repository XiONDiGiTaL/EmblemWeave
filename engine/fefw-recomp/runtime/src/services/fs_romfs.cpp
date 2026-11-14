// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from yuzu src/core/file_sys/fsmitm_romfsbuild.cpp (GPL-2.0-or-later; the table layout, entry sizes, the name hash and the
// bucket count rule) and ScarletNX src/nx/hle/filesystem/fs.c (GPL-3.0-or-later; the same rules, with unsigned name bytes).
// The image is laid out in memory: the header, the four tables and a map of segments. File data is read from the host files on demand.
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include "fs_romfs.h"

#include <windows.h>

#include <algorithm>
#include <array>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <map>
#include <memory>
#include <mutex>
#include <string>
#include <system_error>
#include <vector>

#include "nxrt/diag.h"
#include "nxrt/log.h"
#include "nxrt/nup.h"
#include "nxrt/nx.h"
#include "nxrt/write_watch.h"

namespace nxrt::services::fs {
namespace {

constexpr uint32_t kNone = 0xFFFFFFFFu;     // "no entry" in every link field
constexpr uint64_t kHeaderSize = 0x50;      // ten u64 fields
constexpr uint64_t kPartitionOfs = 0x200;   // file data starts here; the header is padded to it
constexpr uint64_t kDataAlign = 0x10;       // each file starts at a multiple of this, counted from the partition start
constexpr uint64_t kDirFixed = 0x18;        // directory entry without its name
constexpr uint64_t kFileFixed = 0x20;       // file entry without its name
constexpr size_t kMaxName = 255;            // bytes of UTF-8 in one name
constexpr size_t kMaxPath = 0x300;          // SDK FS_MAX_PATH (0x301 with the terminator); the path counts its leading '/'
constexpr size_t kOpenFiles = 16;           // host files kept open by the cache
constexpr size_t kLongPath = 240;           // host paths this long or longer get the \\?\ prefix

uint64_t align_up(uint64_t v, uint64_t a) { return (v + a - 1) / a * a; }
uint64_t pad4(size_t n) { return align_up(n, 4); }

void put32(uint8_t* p, uint32_t v) {
    for (int i = 0; i < 4; i++) p[i] = static_cast<uint8_t>(v >> (8 * i));
}
void put64(uint8_t* p, uint64_t v) {
    for (int i = 0; i < 8; i++) p[i] = static_cast<uint8_t>(v >> (8 * i));
}

// Bucket hash of an entry named `name` under the directory entry at table offset `parent`. The name bytes count unsigned, as the
// console's char does (yuzu's signed char on x86 differs for non-ASCII names only).
uint32_t name_hash(uint32_t parent, const std::string& name) {
    uint32_t h = parent ^ 123456789u;
    for (unsigned char c : name) {
        h = (h >> 5) | (h << 27);
        h ^= c;
    }
    return h;
}

// Bucket count for n entries: 3 for fewer than 3, n|1 below 19, else the first number from n that no prime up to 17 divides.
uint64_t bucket_count(uint64_t n) {
    if (n < 3) return 3;
    if (n < 19) return n | 1;
    uint64_t c = n;
    while (c % 2 == 0 || c % 3 == 0 || c % 5 == 0 || c % 7 == 0 || c % 11 == 0 || c % 13 == 0 || c % 17 == 0) c++;
    return c;
}

bool to_utf8(const std::wstring& w, std::string& out) {
    out.clear();
    if (w.empty()) return true;
    const int n = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, w.data(), static_cast<int>(w.size()), nullptr, 0, nullptr, nullptr);
    if (n <= 0) return false;
    out.resize(static_cast<size_t>(n));
    return WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, w.data(), static_cast<int>(w.size()), out.data(), n, nullptr, nullptr) == n;
}

bool to_wide(const std::string& s, std::wstring& out) {
    out.clear();
    if (s.empty()) return false;
    const int n = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, s.data(), static_cast<int>(s.size()), nullptr, 0);
    if (n <= 0) return false;
    out.resize(static_cast<size_t>(n));
    return MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, s.data(), static_cast<int>(s.size()), out.data(), n) == n;
}

// Host paths that may exceed MAX_PATH get the extended-length prefix. The input must already use backslashes.
std::wstring long_path(std::wstring p) {
    std::replace(p.begin(), p.end(), L'/', L'\\');
    if (p.size() < kLongPath || p.compare(0, 4, L"\\\\?\\") == 0) return p;
    if (p.compare(0, 2, L"\\\\") == 0) return L"\\\\?\\UNC\\" + p.substr(2);
    return L"\\\\?\\" + p;
}

// One folder or file of the host tree. Layout fields are filled in once the tree is complete.
struct Node {
    std::string name;            // UTF-8; empty for the root
    std::string path;            // image path: "" for the root, "/a/b" otherwise
    std::wstring host;           // host path to open
    bool is_dir = false;
    uint64_t size = 0;           // files only
    uint64_t src = 0;            // files: offset of the data in the host file
    bool packed = false;         // files: a zstd frame in the .nup, served from the cache once decompressed
    uint64_t stored = 0;         // files: bytes the frame covers in the package
    uint64_t pkg_off = 0;        // files: where the frame is in the package
    bool archived = false;       // files: read from the blocks of an .nx archive
    nx::File nx_src;             // files: the archive record (path, size, block range)
    std::vector<Node> kids;      // sorted by name bytes
    uint32_t entry = 0;          // offset of this entry in its table
    uint32_t parent = 0;         // entry offset of the parent directory (the root's is 0)
    uint32_t sibling = kNone;    // next entry in the parent's chain of the same kind
    uint32_t first_child = kNone;  // directories: first subdirectory entry
    uint32_t first_file = kNone;   // directories: first file entry
    uint64_t data = 0;           // files: offset of the data from the partition start
};

std::string shown(const Node& n) { return n.path.empty() ? std::string("/") : n.path; }

// Fills dir.kids from the host folder dir.host, recursively. False with msg set at the first entry that cannot be described.
bool scan(Node& dir, std::string& msg) {
    std::error_code ec;
    std::filesystem::directory_iterator it(std::filesystem::path(dir.host), ec);
    if (ec) {
        msg = "cannot read folder " + shown(dir);
        return false;
    }
    const std::filesystem::directory_iterator end;
    for (;;) {
        if (ec) {
            msg = "cannot read folder " + shown(dir);
            return false;
        }
        if (it == end) break;
        const std::filesystem::directory_entry& e = *it;
        std::string name;
        if (!to_utf8(e.path().filename().wstring(), name) || name.empty() || name.size() > kMaxName) {
            msg = "name cannot be stored in a RomFS (1 to 255 bytes of UTF-8): " + shown(dir) + "/" + name;
            return false;
        }
        Node k;
        k.name = name;
        k.path = dir.path + "/" + name;
        if (k.path.size() > kMaxPath) {
            msg = "path too long for a RomFS (0x300 bytes): " + k.path;
            return false;
        }
        k.host = long_path(e.path().wstring());
        std::error_code sec;
        const std::filesystem::file_status st = e.symlink_status(sec);
        if (sec) {
            msg = "cannot stat " + k.path;
            return false;
        }
        if (std::filesystem::is_symlink(st)) {
            msg = "symbolic link cannot be described: " + k.path;
            return false;
        }
        if (std::filesystem::is_directory(st)) {
            k.is_dir = true;
            if (!scan(k, msg)) return false;
        } else if (std::filesystem::is_regular_file(st)) {
            k.size = e.file_size(sec);
            if (sec) {
                msg = "cannot read the size of " + k.path;
                return false;
            }
        } else {
            msg = "not a file or folder: " + k.path;
            return false;
        }
        dir.kids.push_back(std::move(k));
        it.increment(ec);
    }
    // std::string's operator< compares unsigned char values, which is the byte order the RomFS hash tables expect.
    std::sort(dir.kids.begin(), dir.kids.end(), [](const Node& a, const Node& b) { return a.name < b.name; });
    return true;
}

void sort_tree(Node& d) {
    std::sort(d.kids.begin(), d.kids.end(), [](const Node& a, const Node& b) { return a.name < b.name; });
    for (Node& k : d.kids)
        if (k.is_dir) sort_tree(k);
}

// Where decompressed .nup entries are kept: %LOCALAPPDATA%\EmblemWeave\nup-cache\<package name>-<bytes>\files.
std::string nup_cache_dir(const std::string& nup_path) {
    std::filesystem::path base;
    if (const char* local = std::getenv("LOCALAPPDATA"); local && *local) {
        std::wstring wide;
        if (to_wide(std::string(local), wide)) base = std::filesystem::path(wide);
    }
    std::error_code ec;
    if (base.empty()) base = std::filesystem::temp_directory_path(ec);
    std::wstring wpkg;
    if (!to_wide(nup_path, wpkg)) return std::string();
    const std::filesystem::path pkg(wpkg);
    const auto bytes = std::filesystem::file_size(pkg, ec);
    std::string name = nup_path;
    if (const auto slash = name.find_last_of("/\\"); slash != std::string::npos) name = name.substr(slash + 1);
    if (const auto dot = name.find_last_of('.'); dot != std::string::npos) name = name.substr(0, dot);
    name += "-" + std::to_string(ec ? 0 : static_cast<uint64_t>(bytes));
    std::wstring wname;
    if (!to_wide(name, wname)) return std::string();
    std::string out;
    if (!to_utf8((base / L"EmblemWeave" / L"nup-cache" / wname / L"files").wstring(), out)) return std::string();
    return out;
}

// Fills root from the RomFS of an .nx archive. The archive is read through its own reader: a file is a range of blocks, decoded
// and integrity checked on demand, so nothing is copied out before the game asks for it.
bool scan_nx(const std::string& path, Node& root, std::string& msg, std::shared_ptr<nx::Reader>& out) {
    auto reader = std::make_shared<nx::Reader>(path);
    if (!reader->ok()) {
        msg = reader->error();
        return false;
    }
    const nx::Content& c = reader->content();
    if (c.romfs.empty()) {
        msg = path + " has no RomFS";
        return false;
    }
    for (const nx::File& f : c.romfs) {
        Node* d = &root;
        size_t at = 0;
        for (size_t slash; (slash = f.path.find('/', at)) != std::string::npos; at = slash + 1) {
            const std::string name = f.path.substr(at, slash - at);
            auto it = std::find_if(d->kids.begin(), d->kids.end(), [&](const Node& k) { return k.is_dir && k.name == name; });
            if (it == d->kids.end()) {
                Node k;
                k.is_dir = true;
                k.name = name;
                k.path = d->path + "/" + name;
                d->kids.push_back(std::move(k));
                it = d->kids.end() - 1;
            }
            d = &*it;
        }
        Node k;
        k.name = f.path.substr(at);
        k.path = "/" + f.path;
        k.size = f.size;
        k.archived = true;
        k.nx_src = f;
        if (k.name.size() > kMaxName || k.path.size() > kMaxPath) {
            msg = "name or path too long for a RomFS: " + k.path;
            return false;
        }
        d->kids.push_back(std::move(k));
    }
    sort_tree(root);
    out = std::move(reader);
    return true;
}

// Fills root from the RomFS of a .nup package: a stored file is a range of the package file, a zstd one is decompressed into the
// cache the first time the game reads it, and the image then reads that file.
bool scan_nup(const std::string& nup_path, Node& root, std::string& msg, const std::string& cache) {
    nup::Content c;
    if (!nup::open(nup_path, c, &msg)) return false;
    if (c.romfs.empty()) {
        msg = nup_path + " has no RomFS";
        return false;
    }
    for (const nup::File& f : c.romfs) {
        Node* d = &root;
        size_t at = 0;
        for (size_t slash; (slash = f.path.find('/', at)) != std::string::npos; at = slash + 1) {
            const std::string name = f.path.substr(at, slash - at);
            auto it = std::find_if(d->kids.begin(), d->kids.end(), [&](const Node& k) { return k.is_dir && k.name == name; });
            if (it == d->kids.end()) {
                Node k;
                k.is_dir = true;
                k.name = name;
                k.path = d->path + "/" + name;
                d->kids.push_back(std::move(k));
                it = d->kids.end() - 1;
            }
            d = &*it;
        }
        Node k;
        k.name = f.path.substr(at);
        k.path = "/" + f.path;
        k.size = f.size;
        if (f.compressed) {
            const std::string cached = cache + "/" + std::to_string(f.offset) + ".bin";
            if (!to_wide(cached, k.host)) {
                msg = "the cache path of " + f.path + " cannot be represented";
                return false;
            }
            k.packed = true;
            k.stored = f.stored;
            k.pkg_off = f.offset;
        } else {
            k.host = root.host;
            k.src = f.offset;
        }
        if (k.name.size() > kMaxName || k.path.size() > kMaxPath) {
            msg = "name or path too long for a RomFS: " + k.path;
            return false;
        }
        d->kids.push_back(std::move(k));
    }
    sort_tree(root);
    return true;
}

// ---- overlays (RomFS mods) --------------------------------------------------------------------------------------------------------
bool same_name_nocase(const std::string& a, const std::string& b) {
    if (a.size() != b.size()) return false;
    for (size_t i = 0; i < a.size(); i++) {
        unsigned char x = static_cast<unsigned char>(a[i]), y = static_cast<unsigned char>(b[i]);
        if (x >= 'A' && x <= 'Z') x = static_cast<unsigned char>(x + 32);
        if (y >= 'A' && y <= 'Z') y = static_cast<unsigned char>(y + 32);
        if (x != y) return false;
    }
    return true;
}

uint32_t count_files(const Node& n) {
    if (!n.is_dir) return 1;
    uint32_t c = 0;
    for (const Node& k : n.kids) c += count_files(k);
    return c;
}

// Gives n (and everything under it) its image path under the directory `parent`. False when a path gets too long.
bool repath(Node& n, const std::string& parent, std::string& msg) {
    n.path = parent + "/" + n.name;
    if (n.path.size() > kMaxPath) {
        msg = "path too long for a RomFS (0x300 bytes): " + n.path;
        return false;
    }
    for (Node& k : n.kids)
        if (!repath(k, n.path, msg)) return false;
    return true;
}

// Lays the overlay directory `add` over `base`. `taken` counts the files taken from the overlay.
bool merge(Node& base, Node& add, uint32_t& taken, std::string& msg) {
    for (Node& k : add.kids) {
        auto it = std::find_if(base.kids.begin(), base.kids.end(), [&](const Node& b) { return b.name == k.name; });
        if (it == base.kids.end())
            it = std::find_if(base.kids.begin(), base.kids.end(), [&](const Node& b) { return same_name_nocase(b.name, k.name); });
        if (it == base.kids.end()) {
            if (!repath(k, base.path, msg)) return false;
            taken += count_files(k);
            base.kids.push_back(std::move(k));
        } else if (it->is_dir && k.is_dir) {
            if (!merge(*it, k, taken, msg)) return false;
        } else if (!it->is_dir && !k.is_dir) {
            it->host = k.host;
            it->size = k.size;
            it->src = 0;
            taken++;
        } else {
            // a file where the game has a folder, or the other way round: the overlay's entry replaces the game's
            const std::string name = it->name;
            *it = std::move(k);
            it->name = name;
            if (!repath(*it, base.path, msg)) return false;
            taken += count_files(*it);
        }
    }
    return true;
}

void flatten(Node& n, std::vector<Node*>& dirs, std::vector<Node*>& files) {
    for (Node& k : n.kids) {
        if (k.is_dir) {
            dirs.push_back(&k);
            flatten(k, dirs, files);
        } else {
            files.push_back(&k);
        }
    }
}

struct Segment {
    enum class Kind : uint8_t { kHeader, kTables, kZero, kFile };
    uint64_t off = 0;            // image offset
    uint64_t len = 0;
    Kind kind = Kind::kZero;
    uint64_t src = 0;            // kHeader/kTables: offset in that buffer; kFile: offset in the host file
    uint32_t file = 0;           // kFile: index into Layout::files
};

struct FileRec {
    std::wstring host;
    bool archived = false;       // read from the .nx archive rather than the host
    nx::File nx_src;             // the archive record, when archived
    uint64_t size = 0;
    uint64_t image_off = 0;
    uint64_t src = 0;            // offset of the data in the host file
    std::string path;            // path inside the image ("/a/b.bin"), for the first-read note
    bool packed = false;         // still a zstd frame in the package until it is extracted
    uint64_t stored = 0;
    uint64_t pkg_off = 0;
};

// Everything build() produces. Replaced as a whole.
struct Layout {
    bool built = false;
    uint64_t size = 0;
    RomfsImage::Stats stats;
    std::array<uint8_t, kHeaderSize> header{};
    std::vector<uint8_t> tables;        // dir hash, dir table, file hash, file table, in that order from dir_hash_table_ofs
    std::vector<Segment> segs;          // contiguous, covering [0, size)
    std::vector<FileRec> files;         // in data order
    std::map<std::string, uint32_t> by_path;
    std::string package;                // the .nup or .nx, when the files come from one
    std::shared_ptr<nx::Reader> archive;  // the .nx reader the files are read through
};

// Lays out the host folder: entries sorted by full path in each table, file data in that order, 16-byte aligned.
bool make_layout(const std::string& host_dir, const std::vector<std::string>& overlays, Layout& L, std::string* err) {
    std::string msg;
    std::wstring wdir;
    const bool packaged = nup::is_nup(host_dir);
    const bool archived = !packaged && nx::is_nx(host_dir);
    if (!to_wide(host_dir, wdir)) {
        if (err) *err = "host folder name is empty or not valid UTF-8";
        return false;
    }
    std::error_code ec;
    const std::filesystem::path abs = std::filesystem::absolute(std::filesystem::path(wdir), ec);
    // A package is a single file and is served through its own reader, so only a
    // path that is neither a package nor a folder is rejected here.
    if (ec || (!packaged && !archived && !std::filesystem::is_directory(abs, ec))) {
        if (err) *err = "not a folder: " + host_dir;
        return false;
    }
    Node root;
    root.is_dir = true;
    root.host = long_path(abs.wstring());
    const std::string cache = packaged ? nup_cache_dir(host_dir) : std::string();
    if (packaged || archived) L.package = host_dir;
    if (!cache.empty()) {
        std::wstring wcache;
        if (to_wide(cache, wcache)) {
            std::error_code cec;
            std::filesystem::create_directories(std::filesystem::path(wcache), cec);
            if (cec) NXLOG_W("fs", "nup: cannot make the cache folder '%s' (%s); a compressed package will not run", cache.c_str(), cec.message().c_str());
        }
    }
    std::shared_ptr<nx::Reader> archive;
    const bool scanned = archived ? scan_nx(host_dir, root, msg, archive) : packaged ? scan_nup(host_dir, root, msg, cache) : scan(root, msg);
    if (!scanned) {
        if (err) *err = msg;
        return false;
    }
    if (archived) {
        L.archive = std::move(archive);
        NXLOG_I("fs", "nx: %s, %zu top-level RomFS entries", host_dir.c_str(), root.kids.size());
    }
    uint32_t taken = 0;
    for (const std::string& o : overlays) {
        std::wstring wo;
        std::error_code oec;
        const std::filesystem::path oabs = to_wide(o, wo) ? std::filesystem::absolute(std::filesystem::path(wo), oec) : std::filesystem::path();
        if (oabs.empty() || oec || !std::filesystem::is_directory(oabs, oec)) {
            NXLOG_W("fs", "romfs overlay '%s' is not a folder: left out", o.c_str());
            continue;
        }
        Node ov;
        ov.is_dir = true;
        ov.host = long_path(oabs.wstring());
        std::string omsg;
        if (!scan(ov, omsg)) {
            NXLOG_W("fs", "romfs overlay '%s' left out: %s", o.c_str(), omsg.c_str());
            continue;
        }
        // merge into a copy, so an overlay that fails halfway (a path too long) leaves the tree as it was
        Node merged = root;
        uint32_t n = 0;
        if (!merge(merged, ov, n, omsg)) {
            NXLOG_W("fs", "romfs overlay '%s' left out: %s", o.c_str(), omsg.c_str());
            continue;
        }
        root = std::move(merged);
        taken += n;
        NXLOG_I("fs", "romfs overlay '%s': %u files", o.c_str(), n);
    }
    if (!overlays.empty()) sort_tree(root);

    std::vector<Node*> dirs{&root}, files;
    flatten(root, dirs, files);
    auto by_path = [](const Node* a, const Node* b) { return a->path < b->path; };
    std::sort(dirs.begin(), dirs.end(), by_path);
    std::sort(files.begin(), files.end(), by_path);

    // Entry offsets in the two tables (root first: its path is empty).
    uint64_t dir_bytes = 0, file_bytes = 0;
    for (Node* d : dirs) {
        d->entry = static_cast<uint32_t>(dir_bytes);
        dir_bytes += kDirFixed + pad4(d->name.size());
    }
    for (Node* f : files) {
        f->entry = static_cast<uint32_t>(file_bytes);
        file_bytes += kFileFixed + pad4(f->name.size());
    }
    if (dir_bytes >= 0xFFFFFFF0ull || file_bytes >= 0xFFFFFFF0ull) {
        if (err) *err = "too many entries for a RomFS";
        return false;
    }

    // File data: each file at a multiple of 16 from the partition start.
    uint64_t part = 0;
    for (Node* f : files) {
        part = align_up(part, kDataAlign);
        f->data = part;
        part += f->size;
    }

    // Links: each directory's children in name order, as chains of sibling entries.
    for (Node* d : dirs) {
        std::vector<Node*> sub, fil;
        for (Node& k : d->kids) (k.is_dir ? sub : fil).push_back(&k);
        d->first_child = sub.empty() ? kNone : sub.front()->entry;
        d->first_file = fil.empty() ? kNone : fil.front()->entry;
        for (size_t i = 0; i < sub.size(); i++) {
            sub[i]->parent = d->entry;
            sub[i]->sibling = i + 1 < sub.size() ? sub[i + 1]->entry : kNone;
        }
        for (size_t i = 0; i < fil.size(); i++) {
            fil[i]->parent = d->entry;
            fil[i]->sibling = i + 1 < fil.size() ? fil[i + 1]->entry : kNone;
        }
    }

    // Table placement: the header is 0x50 bytes, the partition starts at 0x200, then the four tables, each 4-byte aligned.
    const uint64_t dbuckets = bucket_count(dirs.size());
    const uint64_t fbuckets = bucket_count(files.size());
    const uint64_t dhash_size = 4 * dbuckets, fhash_size = 4 * fbuckets;
    const uint64_t dhash_ofs = align_up(kPartitionOfs + part, 4);
    const uint64_t dtab_ofs = dhash_ofs + dhash_size;
    const uint64_t fhash_ofs = dtab_ofs + dir_bytes;
    const uint64_t ftab_ofs = fhash_ofs + fhash_size;
    const uint64_t image_size = ftab_ofs + file_bytes;

    L.tables.assign(dhash_size + dir_bytes + fhash_size + file_bytes, 0);
    uint8_t* dh = L.tables.data();
    uint8_t* dt = dh + dhash_size;
    uint8_t* fh = dt + dir_bytes;
    uint8_t* ft = fh + fhash_size;
    std::vector<uint32_t> dhead(dbuckets, kNone), fhead(fbuckets, kNone);

    for (Node* d : dirs) {
        const uint32_t b = static_cast<uint32_t>(name_hash(d->parent, d->name) % dbuckets);
        uint8_t* e = dt + d->entry;
        put32(e + 0, d->parent);
        put32(e + 4, d->sibling);
        put32(e + 8, d->first_child);
        put32(e + 12, d->first_file);
        put32(e + 16, dhead[b]);
        put32(e + 20, static_cast<uint32_t>(d->name.size()));
        std::memcpy(e + kDirFixed, d->name.data(), d->name.size());
        dhead[b] = d->entry;
    }
    for (Node* f : files) {
        const uint32_t b = static_cast<uint32_t>(name_hash(f->parent, f->name) % fbuckets);
        uint8_t* e = ft + f->entry;
        put32(e + 0, f->parent);
        put32(e + 4, f->sibling);
        put64(e + 8, f->data);
        put64(e + 16, f->size);
        put32(e + 24, fhead[b]);
        put32(e + 28, static_cast<uint32_t>(f->name.size()));
        std::memcpy(e + kFileFixed, f->name.data(), f->name.size());
        fhead[b] = f->entry;
    }
    for (size_t i = 0; i < dhead.size(); i++) put32(dh + 4 * i, dhead[i]);
    for (size_t i = 0; i < fhead.size(); i++) put32(fh + 4 * i, fhead[i]);

    std::memset(L.header.data(), 0, L.header.size());
    put64(L.header.data() + 0, kHeaderSize);
    put64(L.header.data() + 8, dhash_ofs);
    put64(L.header.data() + 16, dhash_size);
    put64(L.header.data() + 24, dtab_ofs);
    put64(L.header.data() + 32, dir_bytes);
    put64(L.header.data() + 40, fhash_ofs);
    put64(L.header.data() + 48, fhash_size);
    put64(L.header.data() + 56, ftab_ofs);
    put64(L.header.data() + 64, file_bytes);
    put64(L.header.data() + 72, kPartitionOfs);

    // Segments: header, padding, then each file's data with the zero gaps between, the zero gap up to the tables, the tables.
    L.segs.clear();
    L.files.clear();
    L.by_path.clear();
    L.segs.push_back({0, kHeaderSize, Segment::Kind::kHeader, 0, 0});
    L.segs.push_back({kHeaderSize, kPartitionOfs - kHeaderSize, Segment::Kind::kZero, 0, 0});
    uint64_t cursor = kPartitionOfs;
    uint64_t data_bytes = 0;
    for (Node* f : files) {
        const uint64_t img = kPartitionOfs + f->data;
        if (img > cursor) L.segs.push_back({cursor, img - cursor, Segment::Kind::kZero, 0, 0});
        const uint32_t idx = static_cast<uint32_t>(L.files.size());
        L.files.push_back({f->host, f->archived, f->nx_src, f->size, img, f->src, f->path, f->packed, f->stored, f->pkg_off});
        if (f->size > 0) L.segs.push_back({img, f->size, Segment::Kind::kFile, f->src, idx});
        L.by_path[f->path] = idx;
        cursor = img + f->size;
        data_bytes += f->size;
    }
    if (dhash_ofs > cursor) L.segs.push_back({cursor, dhash_ofs - cursor, Segment::Kind::kZero, 0, 0});
    L.segs.push_back({dhash_ofs, image_size - dhash_ofs, Segment::Kind::kTables, 0, 0});

    L.size = image_size;
    L.stats.files = static_cast<uint32_t>(files.size());
    L.stats.dirs = static_cast<uint32_t>(dirs.size());
    L.stats.data_bytes = data_bytes;
    L.stats.data_offset = kPartitionOfs;
    L.stats.overlay_files = taken;
    L.built = true;
    return true;
}

// One host file kept open. The box closes its handle when the last reader and the cache have let go of it.
struct HandleBox {
    HANDLE h = INVALID_HANDLE_VALUE;
    HandleBox() = default;
    HandleBox(const HandleBox&) = delete;
    HandleBox& operator=(const HandleBox&) = delete;
    ~HandleBox() {
        if (h != INVALID_HANDLE_VALUE) CloseHandle(h);
    }
};

struct OpenSlot {
    uint32_t file = kNone;
    uint64_t used = 0;
    std::shared_ptr<HandleBox> box;
};

}  // namespace

struct RomfsImage::Impl {
    Layout l;
    mutable std::mutex mu;                  // guards cache, tick and read_seen
    mutable std::vector<uint8_t> read_seen; // per file: read at least once (the game's loads, in the log and the crash report)
    void note_read(uint32_t idx) const {
        {
            std::lock_guard<std::mutex> lk(mu);
            if (idx >= read_seen.size() || read_seen[idx]) return;
            read_seen[idx] = 1;
        }
        const std::string& path = l.files[idx].path;
        NXLOG_D("fs", "romfs: first read of %s", path.c_str());
        diag_note_file("rom", path.c_str());
    }
    mutable std::vector<OpenSlot> cache;    // at most kOpenFiles entries
    mutable uint64_t tick = 0;
    mutable std::vector<uint8_t> ready;     // per file: its bytes are readable from host (always, unless packed)

    // A zstd entry becomes a file of the cache the first time it is read. The
    // package holds one frame per file, so it is decompressed whole.
    bool ensure(uint32_t idx) const {
        const FileRec& f = l.files[idx];
        if (!f.packed) return true;
        {
            std::lock_guard<std::mutex> lk(mu);
            if (idx < ready.size() && ready[idx]) return true;
        }
        nup::File src;
        src.path = f.path;
        src.size = f.size;
        src.offset = f.pkg_off;
        src.stored = f.stored;
        src.compressed = true;
        std::vector<uint8_t> data;
        std::string err;
        if (!nup::read_file(l.package, src, data, &err)) {
            NXLOG_E("fs", "nup: %s", err.c_str());
            return false;
        }
        std::filesystem::path out(f.host);
        std::error_code ec;
        std::filesystem::create_directories(out.parent_path(), ec);
        std::filesystem::path tmp = out;
        tmp += std::wstring(L".part") + std::to_wstring(GetCurrentThreadId());
        {
            std::ofstream o(tmp, std::ios::binary | std::ios::trunc);
            if (!o) {
                NXLOG_E("fs", "nup: cannot write the cache file %ls", tmp.c_str());
                return false;
            }
            o.write(reinterpret_cast<const char*>(data.data()), static_cast<std::streamsize>(data.size()));
            if (!o) {
                NXLOG_E("fs", "nup: cannot write the cache file %ls (disk full?)", tmp.c_str());
                return false;
            }
        }
        std::filesystem::rename(tmp, out, ec);
        if (ec) {
            std::error_code rec;
            std::filesystem::remove(out, rec);
            std::filesystem::rename(tmp, out, ec);
            if (ec) {
                NXLOG_E("fs", "nup: cannot move the cache file into place: %ls", out.c_str());
                return false;
            }
        }
        {
            std::lock_guard<std::mutex> lk(mu);
            if (idx < ready.size()) ready[idx] = 1;
        }
        NXLOG_I("fs", "nup: %s decompressed to the cache (%llu bytes)", f.path.c_str(), static_cast<unsigned long long>(f.size));
        return true;
    }

    // The handle for file idx, opened if it is not cached. The least recently used entry is dropped when the cache is full.
    std::shared_ptr<HandleBox> acquire(uint32_t idx) const {
        std::lock_guard<std::mutex> lk(mu);
        for (OpenSlot& s : cache) {
            if (s.file == idx) {
                s.used = ++tick;
                return s.box;
            }
        }
        const FileRec& f = l.files[idx];
        HANDLE h = CreateFileW(f.host.c_str(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr,
                               OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
        if (h == INVALID_HANDLE_VALUE) return nullptr;
        auto box = std::make_shared<HandleBox>();
        box->h = h;
        if (cache.size() < kOpenFiles) {
            cache.push_back(OpenSlot{idx, ++tick, box});
        } else {
            auto victim = std::min_element(cache.begin(), cache.end(),
                                           [](const OpenSlot& a, const OpenSlot& b) { return a.used < b.used; });
            *victim = OpenSlot{idx, ++tick, box};
        }
        return box;
    }

    // Positioned reads (OVERLAPPED offset), so concurrent readers of one handle need no seek lock.
    bool read_host(uint32_t idx, uint64_t ofs, uint8_t* out, uint64_t n) const {
        if (!ensure(idx)) return false;
        const mem::ww::OsWriteScope ws(out, n);   // ReadFile fails on a page the write tracker made read-only
        const FileRec& rec = l.files[idx];
        if (rec.archived) {
            if (!l.archive || !l.archive->read(rec.nx_src, ofs, out, n)) {
                NXLOG_E("fs", "nx: cannot read %s at %llu", rec.path.c_str(), static_cast<unsigned long long>(ofs));
                return false;
            }
            return true;
        }
        std::shared_ptr<HandleBox> box = acquire(idx);
        if (!box) return false;
        while (n > 0) {
            const DWORD chunk = static_cast<DWORD>(std::min<uint64_t>(n, 1ull << 30));
            OVERLAPPED ov{};
            ov.Offset = static_cast<DWORD>(ofs & 0xFFFFFFFFull);
            ov.OffsetHigh = static_cast<DWORD>(ofs >> 32);
            DWORD got = 0;
            // A short read means the file shrank since build(): an error, not zeros.
            if (!ReadFile(box->h, out, chunk, &got, &ov) || got != chunk) return false;
            out += chunk;
            ofs += chunk;
            n -= chunk;
        }
        return true;
    }
};

RomfsImage::RomfsImage() : p_(std::make_unique<Impl>()) {}
RomfsImage::~RomfsImage() = default;

bool RomfsImage::build(const std::string& host_dir, std::string* err, const std::vector<std::string>& overlays) {
    Layout fresh;
    const bool ok = make_layout(host_dir, overlays, fresh, err);
    std::lock_guard<std::mutex> lk(p_->mu);
    p_->cache.clear();
    p_->l = ok ? std::move(fresh) : Layout{};
    p_->read_seen.assign(p_->l.files.size(), 0);
    p_->ready.assign(p_->l.files.size(), 1);
    for (size_t i = 0; ok && i < p_->l.files.size(); i++) {
        const FileRec& f = p_->l.files[i];
        if (!f.packed) continue;
        // An entry decompressed by an earlier run is used as it is.
        std::error_code ec;
        const auto have = std::filesystem::file_size(std::filesystem::path(f.host), ec);
        p_->ready[i] = (!ec && have == f.size) ? 1 : 0;
    }
    return ok;
}

bool RomfsImage::built() const { return p_->l.built; }
uint64_t RomfsImage::size() const { return p_->l.size; }
RomfsImage::Stats RomfsImage::stats() const { return p_->l.stats; }

bool RomfsImage::read(uint64_t offset, void* dst, uint64_t len) const {
    const Layout& L = p_->l;
    if (!L.built || offset > L.size || len > L.size - offset) return false;
    if (len == 0) return true;
    uint8_t* out = static_cast<uint8_t*>(dst);
    auto it = std::upper_bound(L.segs.begin(), L.segs.end(), offset,
                               [](uint64_t v, const Segment& g) { return v < g.off; });
    size_t i = static_cast<size_t>(it - L.segs.begin()) - 1;
    while (len > 0) {
        const Segment& g = L.segs[i];
        const uint64_t within = offset - g.off;
        const uint64_t n = std::min(len, g.len - within);
        switch (g.kind) {
            case Segment::Kind::kHeader: std::memcpy(out, L.header.data() + g.src + within, n); break;
            case Segment::Kind::kTables: std::memcpy(out, L.tables.data() + g.src + within, n); break;
            case Segment::Kind::kZero: std::memset(out, 0, n); break;
            case Segment::Kind::kFile:
                p_->note_read(g.file);
                if (!p_->read_host(g.file, g.src + within, out, n)) return false;
                break;
        }
        out += n;
        offset += n;
        len -= n;
        i++;
    }
    return true;
}

bool RomfsImage::file_extent(const std::string& path, uint64_t* offset, uint64_t* size) const {
    const Layout& L = p_->l;
    auto it = L.by_path.find(path);
    if (it == L.by_path.end()) return false;
    const FileRec& f = L.files[it->second];
    if (offset) *offset = f.image_off;
    if (size) *size = f.size;
    return true;
}

}  // namespace nxrt::services::fs
