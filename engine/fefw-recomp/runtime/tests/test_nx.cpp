// SPDX-License-Identifier: GPL-3.0-or-later
// The native .nx reader used by the RomFS facade. The fixtures are NXFormat's synthetic conformance archives (MIT), shared with the
// launcher's Rust reader under crates/ew-format/tests/nx-fixtures rather than copied into the engine.
#include <windows.h>

#include <algorithm>
#include <array>
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <string>
#include <vector>

#include "nxrt/nx.h"
#include "test_framework.h"

NXTEST_GROUP(nx)

namespace {

uint16_t le16(const std::vector<uint8_t>& b, size_t at) {
    return static_cast<uint16_t>(b[at] | (b[at + 1] << 8));
}
uint32_t le32(const std::vector<uint8_t>& b, size_t at) {
    return static_cast<uint32_t>(b[at]) | (static_cast<uint32_t>(b[at + 1]) << 8) |
           (static_cast<uint32_t>(b[at + 2]) << 16) | (static_cast<uint32_t>(b[at + 3]) << 24);
}
uint64_t le64(const std::vector<uint8_t>& b, size_t at) {
    return static_cast<uint64_t>(le32(b, at)) | (static_cast<uint64_t>(le32(b, at + 4)) << 32);
}
void put32(std::vector<uint8_t>& b, size_t at, uint32_t value) {
    for (int i = 0; i < 4; i++) b[at + i] = static_cast<uint8_t>(value >> (8 * i));
}
uint32_t crc32c(const uint8_t* p, size_t n) {
    uint32_t c = 0xFFFFFFFFu;
    for (size_t i = 0; i < n; i++) {
        c ^= p[i];
        for (int k = 0; k < 8; k++) c = (c & 1) ? (0x82F63B78u ^ (c >> 1)) : (c >> 1);
    }
    return c ^ 0xFFFFFFFFu;
}

void fix_header_crc(std::vector<uint8_t>& b, size_t at) {
    put32(b, at + 252, crc32c(b.data() + at, 252));
}

// The test changes a catalog row, so the header digest and both header CRCs
// must be updated before the reader can reach the catalog check.
struct Sha256 {
    uint32_t h[8] = {0x6A09E667u, 0xBB67AE85u, 0x3C6EF372u, 0xA54FF53Au,
                     0x510E527Fu, 0x9B05688Cu, 0x1F83D9ABu, 0x5BE0CD19u};
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
        for (int i = 0; i < 16; i++)
            w[i] = (uint32_t(p[4 * i]) << 24) | (uint32_t(p[4 * i + 1]) << 16) |
                   (uint32_t(p[4 * i + 2]) << 8) | p[4 * i + 3];
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
            if (used == sizeof(buf)) block(buf), used = 0;
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

void fix_catalog_digest(std::vector<uint8_t>& bytes) {
    const uint64_t off = le64(bytes, 24), size = le64(bytes, 32);
    std::vector<uint8_t> catalog(bytes.begin() + static_cast<size_t>(off),
                                 bytes.begin() + static_cast<size_t>(off + size));
    const auto digest = sha256(catalog);
    for (size_t at : {size_t{0}, size_t{4096}}) {
        std::memcpy(bytes.data() + at + 120, digest.data(), digest.size());
        fix_header_crc(bytes, at);
    }
}

std::filesystem::path fixtures() {
#ifdef EW_NX_FIXTURE_DIR
    return std::filesystem::path(EW_NX_FIXTURE_DIR);
#else
    return {};
#endif
}

bool have_fixtures() {
    const std::filesystem::path dir = fixtures();
    return !dir.empty() && std::filesystem::is_regular_file(dir / "minimal_valid.nx");
}

std::vector<uint8_t> read_all(const std::filesystem::path& path) {
    std::ifstream f(path, std::ios::binary);
    return std::vector<uint8_t>(std::istreambuf_iterator<char>(f), std::istreambuf_iterator<char>());
}

struct TempFile {
    std::filesystem::path path;
    explicit TempFile(const std::vector<uint8_t>& bytes) {
        static unsigned seq = 0;
        path = std::filesystem::temp_directory_path() /
               ("nxrt_nx_" + std::to_string(GetCurrentProcessId()) + "_" + std::to_string(seq++) + ".nx");
        std::ofstream f(path, std::ios::binary | std::ios::trunc);
        f.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
    }
    ~TempFile() {
        std::error_code ec;
        std::filesystem::remove(path, ec);
    }
};

nxrt::nx::Reader open_fixture(const char* name) {
    return nxrt::nx::Reader((fixtures() / name).string());
}

void require_fixtures() {
    if (!have_fixtures()) {
        nxtest::skip("NXFormat fixtures are not beside this engine checkout");
    }
}

}  // namespace

NXTEST(minimal_archive_opens) {
    require_fixtures();
    if (!have_fixtures()) return;
    auto r = open_fixture("minimal_valid.nx");
    CHECK_MSG(r.ok(), r.error());
    CHECK_EQ(r.content().romfs.size(), size_t{0});
    CHECK_EQ(r.content().snapshot_kind, 0u);
}

NXTEST(raw_and_zstd_files_read) {
    require_fixtures();
    if (!have_fixtures()) return;
    {
        auto r = open_fixture("raw_only.nx");
        CHECK_MSG(r.ok(), r.error());
        REQUIRE(r.content().romfs.size() == 1);
        const auto& f = r.content().romfs[0];
        CHECK_EQ(f.size, uint64_t{4});
        std::vector<uint8_t> data(4);
        CHECK_MSG(r.read(f, 0, data.data(), data.size()), "the raw block passes its CRC");
        CHECK_EQ(data.size(), size_t{4});
    }
    {
        auto r = open_fixture("zstd_single.nx");
        CHECK_MSG(r.ok(), r.error());
        REQUIRE(r.content().romfs.size() == 1);
        const auto& f = r.content().romfs[0];
        CHECK_EQ(f.size, uint64_t{4096});
        std::vector<uint8_t> data(4096);
        CHECK_MSG(r.read(f, 0, data.data(), data.size()), "the zstd block decodes and passes its CRC");
        CHECK(std::all_of(data.begin(), data.end(), [&](uint8_t b) { return b == data[0]; }));
    }
}

NXTEST(nested_and_multiblock_files_read) {
    require_fixtures();
    if (!have_fixtures()) return;
    {
        auto r = open_fixture("nested_vfs.nx");
        CHECK_MSG(r.ok(), r.error());
        bool nested = false;
        for (const auto& f : r.content().romfs) nested = nested || f.path.find('/') != std::string::npos;
        CHECK_MSG(nested, "a nested fixture path keeps its slash");
    }
    {
        auto r = open_fixture("multi_block_file.nx");
        CHECK_MSG(r.ok(), r.error());
        REQUIRE(!r.content().romfs.empty());
        const auto& f = r.content().romfs[0];
        CHECK_EQ(f.size, uint64_t{65537});
        std::vector<uint8_t> data(static_cast<size_t>(f.size));
        CHECK_MSG(r.read(f, 0, data.data(), data.size()), "a file spanning two blocks reads in one piece");
    }
}

NXTEST(source_resource_archive_is_version_3) {
    require_fixtures();
    if (!have_fixtures()) return;
    auto r = open_fixture("source_resource_v103.nx");
    CHECK_MSG(r.ok(), r.error());
    CHECK_EQ(r.content().snapshot_kind, 0u);
    REQUIRE(!r.content().romfs.empty());
    const auto& f = r.content().romfs[0];
    std::vector<uint8_t> data(static_cast<size_t>(f.size));
    CHECK_MSG(r.read(f, 0, data.data(), data.size()), "the 0.3 fixture's RomFS file reads");
}

NXTEST(damaged_archives_are_rejected) {
    require_fixtures();
    if (!have_fixtures()) return;
    {
        auto r = open_fixture("corrupted_manifest.nx");
        CHECK(!r.ok());
    }
    {
        auto r = open_fixture("truncated_archive.nx");
        CHECK(!r.ok());
    }
}

NXTEST(a_corrupt_zstd_block_is_not_returned) {
    require_fixtures();
    if (!have_fixtures()) return;
    auto bytes = read_all(fixtures() / "zstd_single.nx");
    const uint64_t index_at = le64(bytes, 88);
    const uint64_t block_at = le64(bytes, static_cast<size_t>(index_at));
    REQUIRE(block_at + 8 < bytes.size());
    bytes[static_cast<size_t>(block_at) + 4] ^= 0xFF;   // inside the zstd frame, not in a header
    TempFile file(bytes);
    nxrt::nx::Reader r(file.path.string());
    REQUIRE(r.ok());
    const auto& f = r.content().romfs[0];
    std::vector<uint8_t> data(f.size);
    CHECK(!r.read(f, 0, data.data(), data.size()));
}

NXTEST(a_damaged_block_crc_is_not_returned) {
    require_fixtures();
    if (!have_fixtures()) return;
    auto bytes = read_all(fixtures() / "raw_only.nx");
    REQUIRE(bytes.size() > 8192);
    const uint64_t index_at = le64(bytes, 88);
    REQUIRE(index_at + 8 <= bytes.size());
    const uint64_t block_at = le64(bytes, static_cast<size_t>(index_at));
    REQUIRE(block_at + 4 <= bytes.size());
    bytes[static_cast<size_t>(block_at)] ^= 0x5A;
    TempFile file(bytes);
    nxrt::nx::Reader r(file.path.string());
    REQUIRE(r.ok());
    const auto& f = r.content().romfs[0];
    uint8_t data[4] = {};
    CHECK(!r.read(f, 0, data, sizeof data));
}

NXTEST(an_unknown_required_feature_or_reserved_header_byte_is_rejected) {
    require_fixtures();
    if (!have_fixtures()) return;
    {
        auto bytes = read_all(fixtures() / "minimal_valid.nx");
        for (size_t at : {size_t{0}, size_t{4096}}) {
            const uint16_t features = static_cast<uint16_t>(le16(bytes, at + 12) | 2);
            bytes[at + 12] = static_cast<uint8_t>(features);
            bytes[at + 13] = static_cast<uint8_t>(features >> 8);
            fix_header_crc(bytes, at);
        }
        TempFile file(bytes);
        nxrt::nx::Reader r(file.path.string());
        CHECK(!r.ok());
    }
    {
        auto bytes = read_all(fixtures() / "minimal_valid.nx");
        for (size_t at : {size_t{0}, size_t{4096}}) {
            bytes[at + 228] = 1;
            fix_header_crc(bytes, at);
        }
        TempFile file(bytes);
        nxrt::nx::Reader r(file.path.string());
        CHECK(!r.ok());
    }
}

NXTEST(a_catalog_application_program_range_is_rejected) {
    require_fixtures();
    if (!have_fixtures()) return;
    auto bytes = read_all(fixtures() / "minimal_valid.nx");
    const uint64_t catalog = le64(bytes, 24);
    const uint64_t apps = catalog + le64(bytes, static_cast<size_t>(catalog) + 48);
    REQUIRE(apps + 80 <= bytes.size());
    put32(bytes, static_cast<size_t>(apps) + 12, 1);   // first program
    put32(bytes, static_cast<size_t>(apps) + 16, 1);   // program count, with no program rows
    fix_catalog_digest(bytes);
    TempFile file(bytes);
    nxrt::nx::Reader r(file.path.string());
    CHECK(!r.ok());
    CHECK(r.error().find("program range") != std::string::npos);
}

NXTEST(a_catalog_application_reserved_bytes_are_rejected) {
    require_fixtures();
    if (!have_fixtures()) return;
    auto bytes = read_all(fixtures() / "minimal_valid.nx");
    const uint64_t catalog = le64(bytes, 24);
    const uint64_t apps = catalog + le64(bytes, static_cast<size_t>(catalog) + 48);
    REQUIRE(apps + 80 <= bytes.size());
    bytes[static_cast<size_t>(apps) + 10] = 1;   // NxApplication reserved0
    fix_catalog_digest(bytes);
    TempFile file(bytes);
    nxrt::nx::Reader r(file.path.string());
    CHECK(!r.ok());
    CHECK(r.error().find("reserved") != std::string::npos);
}

NXTEST(a_damaged_slot_leaves_the_other_usable) {
    require_fixtures();
    if (!have_fixtures()) return;
    auto bytes = read_all(fixtures() / "minimal_valid.nx");
    // Slot A becomes invalid; slot B is untouched and generation 0, so the archive still mounts.
    bytes[0] ^= 0xFF;
    TempFile file(bytes);
    nxrt::nx::Reader r(file.path.string());
    CHECK_MSG(r.ok(), r.error());
}

NXTEST(different_headers_with_the_same_generation_are_rejected) {
    require_fixtures();
    if (!have_fixtures()) return;
    auto bytes = read_all(fixtures() / "minimal_valid.nx");
    // Change a valid field, not a reserved one: the second slot must stay a valid header for the
    // same-generation conflict check to be the thing that rejects it.
    bytes[4096 + 16] ^= 1;
    fix_header_crc(bytes, 4096);
    TempFile file(bytes);
    nxrt::nx::Reader r(file.path.string());
    CHECK(!r.ok());
}
