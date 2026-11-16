// SPDX-License-Identifier: GPL-3.0-or-later
// The native .nx reader used by the RomFS facade. The fixtures are NXFormat's synthetic conformance archives (MIT), shared with the
// launcher's Rust reader under crates/ew-format/tests/nx-fixtures rather than copied into the engine.
#include <windows.h>

#include <algorithm>
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
