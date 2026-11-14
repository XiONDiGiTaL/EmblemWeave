// SPDX-License-Identifier: GPL-3.0-or-later
// .nx archive reader (nxrt/nx.h). The layout is NXFormat's (github.com/fab918/nxformat, docs/SPEC.md): a 256-byte header in each
// of the two 4 KiB header pages, a catalog (NXCT) and a manifest (NXMF) that are checked against SHA-256 values in the header, a
// block index of one record per stored block, and an integrity table of one CRC-32C per 16 KiB slice of a block. The archive is a
// file the player supplies, so every index, name and extent is bounds checked before it is used.
#include "nxrt/nx.h"

#include <zstd.h>

#include <algorithm>
#include <array>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <list>
#include <map>
#include <mutex>
#include <set>
#include <system_error>
#include <vector>

#include "nxrt/log.h"

namespace nxrt::nx {
namespace {

constexpr uint64_t kHeaderSize = 256;
constexpr uint64_t kHeaderPage = 4096;
constexpr uint64_t kMaxCatalog = 16ull << 20;
constexpr uint64_t kMaxManifest = 4ull << 30;
constexpr uint32_t kCheckSize = 16384;
constexpr size_t kCacheBlocks = 256;          // decoded blocks kept (4 MiB at 16 KiB a block)
constexpr int kMaxDepth = 64;

constexpr uint8_t kKindBase = 0, kKindUpdate = 1, kKindStandaloneUpdate = 3;
constexpr uint8_t kCodecRaw = 0, kCodecZstd = 1;

std::filesystem::path fs_path(const std::string& utf8) { return std::filesystem::path(std::u8string(utf8.begin(), utf8.end())); }

std::string fail(std::string* err, std::string msg) {
    if (err) *err = std::move(msg);
    return {};
}

uint16_t u16(const std::vector<uint8_t>& b, uint64_t o) {
    uint16_t v;
    std::memcpy(&v, b.data() + o, 2);
    return v;
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

// ---- CRC-32C (Castagnoli), the check the archive uses for headers and block slices ------------------------------------------
const std::array<uint32_t, 256>& crc_table() {
    static const std::array<uint32_t, 256> t = [] {
        std::array<uint32_t, 256> a{};
        for (uint32_t i = 0; i < 256; i++) {
            uint32_t c = i;
            for (int k = 0; k < 8; k++) c = (c & 1) ? (0x82F63B78u ^ (c >> 1)) : (c >> 1);
            a[i] = c;
        }
        return a;
    }();
    return t;
}

uint32_t crc32c(const uint8_t* p, size_t n) {
    const std::array<uint32_t, 256>& t = crc_table();
    uint32_t c = 0xFFFFFFFFu;
    for (size_t i = 0; i < n; i++) c = t[(c ^ p[i]) & 0xFF] ^ (c >> 8);
    return c ^ 0xFFFFFFFFu;
}

// ---- SHA-256, for the catalog, manifest and block index digests -------------------------------------------------------------
struct Sha256 {
    uint32_t h[8] = {0x6A09E667u, 0xBB67AE85u, 0x3C6EF372u, 0xA54FF53Au, 0x510E527Fu, 0x9B05688Cu, 0x1F83D9ABu, 0x5BE0CD19u};
    uint8_t buf[64] = {};
    size_t used = 0;
    uint64_t total = 0;

    static uint32_t ror(uint32_t x, int n) { return (x >> n) | (x << (32 - n)); }

    void block(const uint8_t* p) {
        static const uint32_t k[64] = {
            0x428A2F98u, 0x71374491u, 0xB5C0FBCFu, 0xE9B5DBA5u, 0x3956C25Bu, 0x59F111F1u, 0x923F82A4u, 0xAB1C5ED5u,
            0xD807AA98u, 0x12835B01u, 0x243185BEu, 0x550C7DC3u, 0x72BE5D74u, 0x80DEB1FEu, 0x9BDC06A7u, 0xC19BF174u,
            0xE49B69C1u, 0xEFBE4786u, 0x0FC19DC6u, 0x240CA1CCu, 0x2DE92C6Fu, 0x4A7484AAu, 0x5CB0A9DCu, 0x76F988DAu,
            0x983E5152u, 0xA831C66Du, 0xB00327C8u, 0xBF597FC7u, 0xC6E00BF3u, 0xD5A79147u, 0x06CA6351u, 0x14292967u,
            0x27B70A85u, 0x2E1B2138u, 0x4D2C6DFCu, 0x53380D13u, 0x650A7354u, 0x766A0ABBu, 0x81C2C92Eu, 0x92722C85u,
            0xA2BFE8A1u, 0xA81A664Bu, 0xC24B8B70u, 0xC76C51A3u, 0xD192E819u, 0xD6990624u, 0xF40E3585u, 0x106AA070u,
            0x19A4C116u, 0x1E376C08u, 0x2748774Cu, 0x34B0BCB5u, 0x391C0CB3u, 0x4ED8AA4Au, 0x5B9CCA4Fu, 0x682E6FF3u,
            0x748F82EEu, 0x78A5636Fu, 0x84C87814u, 0x8CC70208u, 0x90BEFFFAu, 0xA4506CEBu, 0xBEF9A3F7u, 0xC67178F2u};
        uint32_t w[64];
        for (int i = 0; i < 16; i++) w[i] = (uint32_t(p[4 * i]) << 24) | (uint32_t(p[4 * i + 1]) << 16) | (uint32_t(p[4 * i + 2]) << 8) | p[4 * i + 3];
        for (int i = 16; i < 64; i++) {
            const uint32_t s0 = ror(w[i - 15], 7) ^ ror(w[i - 15], 18) ^ (w[i - 15] >> 3);
            const uint32_t s1 = ror(w[i - 2], 17) ^ ror(w[i - 2], 19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16] + s0 + w[i - 7] + s1;
        }
        uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
        for (int i = 0; i < 64; i++) {
            const uint32_t s1 = ror(e, 6) ^ ror(e, 11) ^ ror(e, 25);
            const uint32_t ch = (e & f) ^ (~e & g);
            const uint32_t t1 = hh + s1 + ch + k[i] + w[i];
            const uint32_t s0 = ror(a, 2) ^ ror(a, 13) ^ ror(a, 22);
            const uint32_t mj = (a & b) ^ (a & c) ^ (b & c);
            const uint32_t t2 = s0 + mj;
            hh = g, g = f, f = e, e = d + t1, d = c, c = b, b = a, a = t1 + t2;
        }
        h[0] += a, h[1] += b, h[2] += c, h[3] += d, h[4] += e, h[5] += f, h[6] += g, h[7] += hh;
    }

    void add(const uint8_t* p, size_t n) {
        total += n;
        while (n > 0) {
            const size_t take = std::min(n, sizeof(buf) - used);
            std::memcpy(buf + used, p, take);
            used += take;
            p += take;
            n -= take;
            if (used == sizeof(buf)) {
                block(buf);
                used = 0;
            }
        }
    }

    std::array<uint8_t, 32> done() {
        const uint64_t bits = total * 8;
        uint8_t pad[72] = {0x80};
        const size_t pad_len = used < 56 ? 56 - used : 120 - used;
        add(pad, pad_len);
        uint8_t len[8];
        for (int i = 0; i < 8; i++) len[i] = static_cast<uint8_t>(bits >> (56 - 8 * i));
        add(len, 8);
        std::array<uint8_t, 32> out{};
        for (int i = 0; i < 8; i++)
            for (int k = 0; k < 4; k++) out[4 * i + k] = static_cast<uint8_t>(h[i] >> (24 - 8 * k));
        return out;
    }
};

std::array<uint8_t, 32> sha256(const std::vector<uint8_t>& b) {
    Sha256 s;
    if (!b.empty()) s.add(b.data(), b.size());
    return s.done();
}

// ---- header -------------------------------------------------------------------------------------------------------------------
struct Header {
    uint64_t generation = 0;
    uint64_t archive_size = 0;
    uint64_t catalog_offset = 0, catalog_size = 0;
    uint64_t manifest_offset = 0, manifest_size = 0;
    uint64_t block_index_offset = 0, block_count = 0;
    uint64_t integrity_offset = 0, integrity_size = 0;
    uint32_t check_size = 0;
    std::array<uint8_t, 32> catalog_sha{}, manifest_sha{}, index_sha{};
};

bool read_slot(const std::vector<uint8_t>& page, size_t at, Header& out) {
    if (page.size() < at + kHeaderSize) return false;
    const uint8_t* b = page.data() + at;
    if (std::memcmp(b, "NXAR", 4) != 0) return false;
    if (u32(page, at + 252) != crc32c(b, 252)) return false;
    if (u16(page, at + 4) != 0) return false;
    const uint16_t version = u16(page, at + 6);
    if (version < 2 || version > 3) return false;
    if (u32(page, at + 8) != kHeaderSize) return false;
    if ((u16(page, at + 12) & 1) != 1) return false;
    if (u16(page, at + 14) != 0) return false;
    out.archive_size = u64(page, at + 16);
    out.catalog_offset = u64(page, at + 24);
    out.catalog_size = u64(page, at + 32);
    out.manifest_offset = u64(page, at + 72);
    out.manifest_size = u64(page, at + 80);
    out.block_index_offset = u64(page, at + 88);
    out.block_count = u64(page, at + 96);
    out.integrity_offset = u64(page, at + 104);
    out.integrity_size = u64(page, at + 112);
    std::memcpy(out.catalog_sha.data(), b + 120, 32);
    std::memcpy(out.manifest_sha.data(), b + 152, 32);
    std::memcpy(out.index_sha.data(), b + 184, 32);
    out.generation = u64(page, at + 216);
    out.check_size = u32(page, at + 224);
    return true;
}

struct Block {
    uint64_t offset = 0;
    uint32_t stored = 0;
    uint32_t logical = 0;
    uint8_t codec = 0;
    uint32_t first_check = 0;
};

}  // namespace

struct Reader::Impl {
    std::string path;
    uint64_t archive_size = 0;
    uint32_t check_size = kCheckSize;

    // Manifest tables.
    std::vector<std::pair<uint32_t, uint32_t>> dirs;                 // (first entry, entry count)
    struct Entry {
        std::string name;
        uint32_t target = 0;
        bool dir = false;
    };
    std::vector<Entry> entries;
    struct Rec {
        uint64_t size = 0;
        uint32_t first_block = 0;
        uint32_t block_count = 0;
    };
    std::vector<Rec> files;
    std::vector<uint32_t> block_refs;
    std::vector<Block> blocks;
    std::vector<uint32_t> checks;

    // Catalog rows.
    struct App {
        uint64_t id = 0;
        uint32_t first = 0, count = 0;
    };
    std::vector<App> apps;
    struct Snap {
        uint32_t root = 0;
        uint8_t kind = 0;
        uint32_t version = 0;
    };
    std::vector<Snap> snaps;

    std::fstream file;
    std::mutex io;               // guards `file` seeks and reads

    mutable std::mutex cmu;      // guards the decoded block cache
    mutable std::map<uint32_t, std::shared_ptr<const std::vector<uint8_t>>> cache;
    mutable std::list<uint32_t> order;   // most recently used at the front

    bool read_at(uint64_t off, uint64_t len, std::vector<uint8_t>& out, std::string* err) {
        if (off > archive_size || len > archive_size - off) return fail(err, "a table lies outside the archive").empty();
        out.resize(static_cast<size_t>(len));
        std::lock_guard<std::mutex> lk(io);
        file.clear();
        file.seekg(static_cast<std::streamoff>(off));
        if (!file.read(reinterpret_cast<char*>(out.data()), static_cast<std::streamsize>(len)))
            return fail(err, "cannot read " + path + " at " + std::to_string(off)).empty();
        return true;
    }

    bool parse_catalog(const std::vector<uint8_t>& c, std::string* err);
    bool parse_manifest(const std::vector<uint8_t>& m, std::string* err);
    bool build_content(Content& out, std::string* err);
    bool list_dir(uint32_t dir, const std::string& prefix, bool romfs_root, std::set<uint32_t>& seen, int depth,
                  std::vector<File>& out, std::string* err);

    // The decoded bytes of one block, verified. False with err set when the block is damaged or keeps failing its check.
    std::shared_ptr<const std::vector<uint8_t>> block_bytes(uint32_t id, std::string* err);
    bool decode_block(uint32_t id, std::vector<uint8_t>& out, std::string* err);

    // Where each of a file's blocks starts in the file: block_count + 1 entries, the last being the file's size. A block can hold
    // several integrity slices, so this cannot be derived from the slice size and is built once per file and kept.
    mutable std::mutex span_mu;
    mutable std::map<uint32_t, std::shared_ptr<const std::vector<uint64_t>>> spans;   // by first_block
    std::shared_ptr<const std::vector<uint64_t>> spans_of(const File& f, std::string* err);
};

std::shared_ptr<const std::vector<uint64_t>> Reader::Impl::spans_of(const File& f, std::string* err) {
    {
        std::lock_guard<std::mutex> lk(span_mu);
        auto it = spans.find(f.first_block);
        if (it != spans.end()) return it->second;
    }
    auto starts = std::make_shared<std::vector<uint64_t>>(static_cast<size_t>(f.block_count) + 1);
    uint64_t at = 0;
    for (uint32_t i = 0; i < f.block_count; i++) {
        (*starts)[i] = at;
        const uint32_t ref = block_refs[static_cast<size_t>(f.first_block) + i];
        if (ref >= blocks.size()) return fail(err, "a file names a block that does not exist").empty(), nullptr;
        at += blocks[ref].logical;
        if (at > f.size) return fail(err, "a file's blocks hold more bytes than the file").empty(), nullptr;
    }
    (*starts)[f.block_count] = at;
    if (at != f.size) return fail(err, "a file's blocks hold fewer bytes than the file").empty(), nullptr;
    std::lock_guard<std::mutex> lk(span_mu);
    spans[f.first_block] = starts;
    return starts;
}

bool Reader::Impl::parse_catalog(const std::vector<uint8_t>& c, std::string* err) {
    if (c.size() < 128 || std::memcmp(c.data(), "NXCT", 4) != 0) return fail(err, "the catalog header is missing").empty();
    if (u16(c, 4) != 1 || u16(c, 6) != 128) return fail(err, "the catalog is a version this build does not know").empty();
    const uint32_t n_apps = u32(c, 20), n_progs = u32(c, 24), n_snaps = u32(c, 28);
    const uint64_t pool_off = u64(c, 96), pool_len = u64(c, 104);
    if (pool_off > c.size() || pool_len > c.size() - pool_off) return fail(err, "the catalog string pool is out of range").empty();
    auto string_at = [&](uint32_t off, uint32_t len, std::string& out) -> bool {
        if (off == 0xFFFFFFFFu || len == 0) return true;
        if (static_cast<uint64_t>(off) + len > pool_len) return false;
        out.assign(reinterpret_cast<const char*>(c.data() + pool_off + off), len);
        return true;
    };

    uint64_t at = u64(c, 48);
    for (uint32_t i = 0; i < n_apps; i++, at += 80) {
        if (at + 80 > c.size()) return fail(err, "the catalog application table is truncated").empty();
        App a;
        a.id = u64(c, at);
        a.first = u32(c, at + 20);
        a.count = u32(c, at + 24);
        std::string title;
        if (!string_at(u32(c, at + 28), u32(c, at + 32), title)) return fail(err, "a catalog title is out of range").empty();
        apps.push_back(a);
    }
    at = u64(c, 56);
    for (uint32_t i = 0; i < n_progs; i++, at += 64)
        if (at + 64 > c.size()) return fail(err, "the catalog program table is truncated").empty();
    at = u64(c, 64);
    for (uint32_t i = 0; i < n_snaps; i++, at += 120) {
        if (at + 120 > c.size()) return fail(err, "the catalog snapshot table is truncated").empty();
        Snap s;
        s.kind = c[at + 4];
        s.version = u32(c, at + 8);
        s.root = u32(c, at + 12);
        snaps.push_back(s);
    }
    if (snaps.empty()) return fail(err, "the catalog holds no snapshots").empty();
    return true;
}

bool Reader::Impl::parse_manifest(const std::vector<uint8_t>& m, std::string* err) {
    if (m.size() < 96 || std::memcmp(m.data(), "NXMF", 4) != 0) return fail(err, "the manifest header is missing").empty();
    if (u16(m, 4) != 1 || u16(m, 6) != 96) return fail(err, "the manifest is a version this build does not know").empty();
    const uint32_t n_dirs = u32(m, 16), n_files = u32(m, 20), n_entries = u32(m, 24), n_refs = u32(m, 28);

    uint64_t at = u64(m, 32);
    for (uint32_t i = 0; i < n_dirs; i++, at += 16) {
        if (at + 16 > m.size()) return fail(err, "the manifest directory table is truncated").empty();
        dirs.emplace_back(u32(m, at), u32(m, at + 4));
    }
    at = u64(m, 40);
    for (uint32_t i = 0; i < n_files; i++, at += 64) {
        if (at + 64 > m.size()) return fail(err, "the manifest file table is truncated").empty();
        Rec r;
        r.size = u64(m, at);
        r.first_block = u32(m, at + 8);
        r.block_count = u32(m, at + 12);
        files.push_back(r);
    }
    // Entry names are stored as (offset, length) from the start of the Manifest, not from the path pool inside it.
    at = u64(m, 48);
    for (uint32_t i = 0; i < n_entries; i++, at += 24) {
        if (at + 24 > m.size()) return fail(err, "the manifest entry table is truncated").empty();
        Entry e;
        const uint64_t off = u64(m, at), len = u32(m, at + 8);
        if (off > m.size() || len > m.size() - off) return fail(err, "a manifest entry name is out of range").empty();
        if (len > 255) return fail(err, "a manifest entry name is too long").empty();
        e.name.assign(reinterpret_cast<const char*>(m.data() + off), static_cast<size_t>(len));
        if (e.name.empty() || e.name == "." || e.name == ".." || e.name.find_first_of("/\\:") != std::string::npos ||
            e.name.find('\0') != std::string::npos)
            return fail(err, "the manifest holds an invalid name").empty();
        e.target = u32(m, at + 12);
        e.dir = m[at + 16] == 1;
        entries.push_back(std::move(e));
    }
    at = u64(m, 56);
    for (uint32_t i = 0; i < n_refs; i++, at += 4) {
        if (at + 4 > m.size()) return fail(err, "the manifest block reference table is truncated").empty();
        block_refs.push_back(u32(m, at));
    }
    // Check the file records once here, so no read has to.
    for (const Rec& r : files) {
        if (r.block_count == 0) {
            if (r.size != 0) return fail(err, "the manifest has a file with no blocks").empty();
            continue;
        }
        if (static_cast<uint64_t>(r.first_block) + r.block_count > block_refs.size())
            return fail(err, "the manifest has a file whose blocks are out of range").empty();
    }
    return true;
}

bool Reader::Impl::list_dir(uint32_t dir, const std::string& prefix, bool romfs_root, std::set<uint32_t>& seen, int depth,
                            std::vector<File>& out, std::string* err) {
    if (depth > kMaxDepth) return fail(err, "the manifest tree is deeper than 64 levels").empty();
    if (dir >= dirs.size()) return fail(err, "the manifest names a directory that does not exist").empty();
    if (!seen.insert(dir).second) return true;   // a cyclic manifest cannot loop here
    const uint64_t first = dirs[dir].first;
    const uint64_t count = dirs[dir].second;
    if (first + count > entries.size()) return fail(err, "a manifest directory range is out of bounds").empty();
    for (uint64_t i = first; i < first + count; i++) {
        const Entry& e = entries[i];
        const std::string path = prefix.empty() ? e.name : prefix + "/" + e.name;
        if (e.dir) {
            // The game's RomFS is the "romfs" folder of the snapshot tree; a tree without it is taken as the RomFS itself.
            const bool is_root = !romfs_root && prefix.empty() && e.name == "romfs";
            const std::string sub = is_root ? std::string() : path;
            if (!list_dir(e.target, sub, is_root, seen, depth + 1, out, err)) return false;
            continue;
        }
        if (e.target >= files.size()) return fail(err, "the manifest names a file that does not exist").empty();
        if (path.size() > 0x300) return fail(err, "a RomFS path is too long: " + path).empty();
        const Rec& r = files[e.target];
        File f;
        f.path = path;
        f.size = r.size;
        f.first_block = r.first_block;
        f.block_count = r.block_count;
        out.push_back(std::move(f));
    }
    return true;
}

bool Reader::Impl::build_content(Content& out, std::string* err) {
    if (apps.empty()) return fail(err, "the catalog holds no application").empty();
    size_t pick = 0;
    for (size_t i = 1; i < apps.size(); i++)
        if (apps[i].count > apps[pick].count) pick = i;
    const App& a = apps[pick];
    out.title = a.id;
    const size_t first = a.first, last = std::min<size_t>(static_cast<size_t>(a.first) + a.count, snaps.size());
    if (first >= last) return fail(err, "the application has no snapshot").empty();
    int64_t best = -1;
    for (size_t i = first; i < last; i++) {
        const Snap& s = snaps[i];
        if (s.kind == kKindUpdate || s.kind == kKindStandaloneUpdate) {
            if (best < 0 || s.version > snaps[best].version) best = static_cast<int64_t>(i);
        }
    }
    if (best < 0) {
        for (size_t i = first; i < last; i++)
            if (snaps[i].kind == kKindBase) best = static_cast<int64_t>(i);
    }
    if (best < 0) best = static_cast<int64_t>(first);
    out.version = snaps[best].version;
    out.snapshot_kind = snaps[best].kind;

    std::set<uint32_t> seen;
    if (!list_dir(snaps[best].root, "", false, seen, 0, out.romfs, err)) return false;
    if (out.romfs.empty()) {
        // Either the tree has no "romfs" folder (the root is the RomFS) or it is genuinely empty.
        std::set<uint32_t> again;
        if (!list_dir(snaps[best].root, "", true, again, 0, out.romfs, err)) return false;
    }
    return true;
}

bool Reader::Impl::decode_block(uint32_t id, std::vector<uint8_t>& out, std::string* err) {
    if (id >= blocks.size()) return fail(err, "a file names a block that does not exist").empty();
    const Block& b = blocks[id];
    std::vector<uint8_t> stored;
    if (!read_at(b.offset, b.stored, stored, err)) return false;
    if (b.codec == kCodecRaw) {
        if (b.stored != b.logical) return fail(err, "a raw block stores a different size than it declares").empty();
        out = std::move(stored);
    } else if (b.codec == kCodecZstd) {
        out.resize(b.logical);
        const size_t got = ZSTD_decompress(out.data(), out.size(), stored.data(), stored.size());
        if (ZSTD_isError(got)) return fail(err, std::string("a block is not a usable zstd frame: ") + ZSTD_getErrorName(got)).empty();
        if (got != b.logical) return fail(err, "a block decodes to the wrong size").empty();
    } else {
        return fail(err, "a block uses a codec this build does not know").empty();
    }
    if (out.size() != b.logical) return fail(err, "a block decodes to the wrong size").empty();
    // One CRC-32C per 16 KiB slice of the decoded block.
    for (size_t k = 0, at = 0; at < out.size(); k++, at += check_size) {
        const size_t n = std::min<size_t>(check_size, out.size() - at);
        const uint64_t idx = static_cast<uint64_t>(b.first_check) + k;
        if (idx >= checks.size()) return fail(err, "the integrity table does not cover a block").empty();
        if (checks[idx] != crc32c(out.data() + at, n)) return fail(err, "a block failed its integrity check").empty();
    }
    return true;
}

std::shared_ptr<const std::vector<uint8_t>> Reader::Impl::block_bytes(uint32_t id, std::string* err) {
    {
        std::lock_guard<std::mutex> lk(cmu);
        auto it = cache.find(id);
        if (it != cache.end()) {
            order.remove(id);
            order.push_front(id);
            return it->second;
        }
    }
    auto data = std::make_shared<std::vector<uint8_t>>();
    if (!decode_block(id, *data, err)) return nullptr;
    std::lock_guard<std::mutex> lk(cmu);
    cache[id] = data;
    order.push_front(id);
    while (order.size() > kCacheBlocks) {
        cache.erase(order.back());
        order.pop_back();
    }
    return data;
}

Reader::Reader(const std::string& path) : p_(std::make_unique<Impl>()) {
    p_->path = path;
    std::error_code ec;
    const uint64_t len = static_cast<uint64_t>(std::filesystem::file_size(fs_path(path), ec));
    if (ec) {
        err_ = "cannot open " + path;
        return;
    }
    if (len < kHeaderPage * 2) {
        err_ = path + " is too short to be a .nx archive";
        return;
    }
    p_->archive_size = len;
    p_->file.open(fs_path(path), std::ios::binary | std::ios::in);
    if (!p_->file) {
        err_ = "cannot open " + path;
        return;
    }

    std::vector<uint8_t> pages(kHeaderPage * 2);
    {
        std::lock_guard<std::mutex> lk(p_->io);
        p_->file.read(reinterpret_cast<char*>(pages.data()), static_cast<std::streamsize>(pages.size()));
    }
    Header a, b;
    const bool has_a = read_slot(pages, 0, a);
    const bool has_b = read_slot(pages, kHeaderPage, b);
    Header h;
    if (has_a && has_b) {
        const bool same = std::memcmp(pages.data(), pages.data() + kHeaderPage, kHeaderSize) == 0;
        if (a.generation == b.generation && !same) {
            err_ = path + " holds two different headers with the same generation";
            return;
        }
        h = a.generation >= b.generation ? a : b;
    } else if (has_a) {
        h = a;
    } else if (has_b) {
        h = b;
    } else {
        err_ = path + " has no intact .nx header";
        return;
    }
    if (h.archive_size > len) {
        err_ = path + " claims more bytes than the file has";
        return;
    }
    if (h.catalog_size < 128 || h.catalog_size > kMaxCatalog || h.manifest_size < 96 || h.manifest_size > kMaxManifest ||
        h.check_size != kCheckSize) {
        err_ = path + " has a table this build cannot use";
        return;
    }
    p_->check_size = h.check_size;

    std::vector<uint8_t> catalog, manifest;
    if (!p_->read_at(h.catalog_offset, h.catalog_size, catalog, &err_) ||
        !p_->read_at(h.manifest_offset, h.manifest_size, manifest, &err_))
        return;
    if (sha256(catalog) != h.catalog_sha) {
        err_ = "the catalog of " + path + " does not match its digest";
        return;
    }
    if (sha256(manifest) != h.manifest_sha) {
        err_ = "the manifest of " + path + " does not match its digest";
        return;
    }
    if (!p_->parse_catalog(catalog, &err_) || !p_->parse_manifest(manifest, &err_)) return;

    // Block index and integrity table, checked together against the digest the header holds.
    if (h.block_count > 0) {
        std::vector<uint8_t> index, integrity;
        if (!p_->read_at(h.block_index_offset, h.block_count * 24, index, &err_) ||
            !p_->read_at(h.integrity_offset, h.integrity_size, integrity, &err_))
            return;
        std::vector<uint8_t> both = index;
        both.insert(both.end(), integrity.begin(), integrity.end());
        if (sha256(both) != h.index_sha) {
            err_ = "the block index of " + path + " does not match its digest";
            return;
        }
        if (integrity.size() % 4 != 0) {
            err_ = "the integrity table of " + path + " is not whole";
            return;
        }
        p_->blocks.resize(static_cast<size_t>(h.block_count));
        for (size_t i = 0; i < p_->blocks.size(); i++) {
            const uint64_t at = i * 24;
            Block& blk = p_->blocks[i];
            blk.offset = u64(index, at);
            blk.stored = u32(index, at + 8);
            blk.logical = u32(index, at + 12);
            blk.codec = index[at + 16];
            blk.first_check = u32(index, at + 20);
            if (blk.offset > h.archive_size || blk.stored > h.archive_size - blk.offset) {
                err_ = "a block of " + path + " lies outside the file";
                return;
            }
            if (blk.codec != kCodecRaw && blk.codec != kCodecZstd) {
                err_ = "a block of " + path + " uses a codec this build does not know";
                return;
            }
            // A block is made of whole integrity slices, so its logical size is a multiple of the slice size at most; the
            // per-slice checks below are what actually validates it.
            if (blk.logical > 0 && static_cast<uint64_t>(blk.first_check) + (blk.logical + kCheckSize - 1) / kCheckSize >
                                       static_cast<uint64_t>(h.integrity_size / 4)) {
                err_ = "the integrity table of " + path + " does not cover a block";
                return;
            }
        }
        p_->checks.resize(integrity.size() / 4);
        for (size_t i = 0; i < p_->checks.size(); i++) p_->checks[i] = u32(integrity, i * 4);
    } else if (h.integrity_size != 0) {
        err_ = "an archive without blocks has an integrity table";
        return;
    }

    if (!p_->build_content(content_, &err_)) return;
    ok_ = true;
}

Reader::~Reader() = default;

uint64_t Reader::archive_size() const { return p_->archive_size; }

bool Reader::read(const File& f, uint64_t offset, void* dst, uint64_t len) {
    if (offset > f.size || len > f.size - offset) return false;
    if (len == 0) return true;
    if (f.block_count == 0) return false;
    std::string err;
    auto starts = p_->spans_of(f, &err);
    if (!starts) {
        NXLOG_E("fs", "nx: %s: %s", f.path.c_str(), err.c_str());
        return false;
    }
    uint8_t* out = static_cast<uint8_t*>(dst);
    // The block holding `offset`: the last one that starts at or before it.
    auto it = std::upper_bound(starts->begin(), starts->end(), offset);
    uint32_t i = static_cast<uint32_t>(std::max<ptrdiff_t>(0, (it - starts->begin()) - 1));
    if (i >= f.block_count) i = f.block_count - 1;
    while (len > 0) {
        if (i >= f.block_count) return false;
        const uint32_t id = p_->block_refs[static_cast<size_t>(f.first_block) + i];
        auto bytes = p_->block_bytes(id, &err);
        if (!bytes) {
            NXLOG_E("fs", "nx: %s: %s", f.path.c_str(), err.c_str());
            return false;
        }
        const uint64_t start = (*starts)[i];
        if (offset < start) return false;
        const uint64_t within = offset - start;
        if (within >= bytes->size()) {   // the index says the block is shorter than the decoded bytes: damaged
            if (within < (*starts)[i + 1] - start) return false;
            i++;
            continue;
        }
        const uint64_t n = std::min<uint64_t>({len, bytes->size() - within, (*starts)[i + 1] - offset});
        std::memcpy(out, bytes->data() + within, static_cast<size_t>(n));
        out += n;
        offset += n;
        len -= n;
        i++;
    }
    return true;
}

bool is_nx(const std::string& path) {
    std::ifstream f(fs_path(path), std::ios::binary);
    char m[4] = {};
    return f.read(m, 4) && std::memcmp(m, "NXAR", 4) == 0;
}

}  // namespace nxrt::nx
