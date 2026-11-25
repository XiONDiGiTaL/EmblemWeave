// SPDX-License-Identifier: GPL-3.0-or-later
// fsp-srv at IPC level: RomFS storage, read-only data file system, save data (create / open / write / commit / drop), SD card, account
// save API. Request bytes follow Switchbrew "Filesystem services" / libnx fs_service.c layouts.
#define _SILENCE_CXX20_U8PATH_DEPRECATION_WARNING
#include <windows.h>

#include <cstdio>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <vector>

#include <zstd.h>

#include "nxrt/config.h"
#include "nxrt/game.h"
#include "nxrt/services_api.h"
#include "svc_test_util.h"

NXTEST_GROUP(svc_fs)

namespace stdfs = std::filesystem;
using nxsvc::Args;
using nxsvc::Reply;
using nxsvc::Session;

namespace {

constexpr uint32_t kPathNotFound = 0x202, kPathExists = 0x402, kTargetNotFound = 0x7D402, kSdNoDevice = 0xFA202, kOutOfRange = 0x177A02;
constexpr uint32_t kNoAppend = 0x307202;

struct Fixture {
    stdfs::path dir, rom, save;
    nxsvc::Env env;
    Fixture() {
        dir = stdfs::temp_directory_path() / ("nxrt_svcfs_" + std::to_string(GetCurrentProcessId()));
        std::error_code ec;
        stdfs::remove_all(dir, ec);
        rom = dir / "romfs";
        save = dir / "save";
        stdfs::create_directories(rom / "Worlds", ec);
        stdfs::create_directories(save, ec);
        write(rom / "a.bin", std::string(1000, 'a'));
        write(rom / "Worlds" / "b.pak", "bee");
        nxrt::config_set("fs", "romfs", rom.string().c_str());
        nxrt::config_set("fs", "save_dir", save.string().c_str());
        nxrt::services::reset_all();
    }
    ~Fixture() {
        nxrt::config_clear();
        nxrt::services::reset_all();
        std::error_code ec;
        stdfs::remove_all(dir, ec);
    }
    static void write(const stdfs::path& p, const std::string& s) {
        std::ofstream o(p, std::ios::binary);
        o << s;
    }
};

// A 0x301-byte path buffer for the A/X buffer of the IFileSystem commands.
struct PathBuf {
    char b[0x301] = {};
    explicit PathBuf(const char* s) { std::strncpy(b, s, 0x300); }
};

// SaveDataAttribute (0x40): application id, uid (hi, lo), system save data id, type, rank, index.
std::vector<uint8_t> attr(uint8_t type, uint64_t uid_hi, uint64_t uid_lo, uint16_t index = 0) {
    std::vector<uint8_t> a(0x40, 0);
    const uint64_t app = 0;   // 0 = the calling title
    std::memcpy(&a[0], &app, 8);
    std::memcpy(&a[8], &uid_hi, 8);
    std::memcpy(&a[16], &uid_lo, 8);
    a[32] = type;
    std::memcpy(&a[34], &index, 2);
    return a;
}

Session open_fsp() {
    Session s = nxsvc::connect("fsp-srv");
    CHECK(s.valid());
    CHECK(s.call(1, Args().with_pid().u64(0)).ok());   // SetCurrentProcess(pid)
    return s;
}

// CreateSaveDataFileSystem(attr, creation info, meta info): creation info = size, journal size, available, owner, flags, space id.
Reply create_save(Session& s, const std::vector<uint8_t>& a, int64_t size, int64_t journal) {
    Args args;
    args.bytes(a.data(), a.size());
    args.i64(size).i64(journal).u64(size).u64(0x010012101468C000ull).u32(0).u8(1).pad(0x1B);   // creation info, 0x40 bytes
    args.pad(0x10);                                                                          // meta info
    return s.call(22, args);
}

Reply open_save(Session& s, const std::vector<uint8_t>& a, uint8_t space = 1, uint32_t cmd = 51) {
    Args args;
    args.u8(space).pad(7).bytes(a.data(), a.size());
    return s.call(cmd, args);
}

Reply fs_call_path(Session& fsys, uint32_t cmd, const char* path, Args extra = {}) {
    auto pb = std::make_unique<PathBuf>(path);   // heap: the X descriptor holds 39-bit addresses
    extra.buf_x(pb->b, 0x301);
    return fsys.call(cmd, extra);
}

constexpr uint32_t kSlotMagic = 0xCDEFBCDD;

uint32_t fnv1a32(const uint8_t* p, size_t n) {
    uint32_t h = 0x811C9DC5u;
    for (size_t i = 0; i < n; ++i) h = (h ^ p[i]) * 0x01000193u;
    return h;
}

uint32_t fnv1_32(const uint8_t* p, size_t n) {
    uint32_t h = 0x811C9DC5u;
    for (size_t i = 0; i < n; ++i) h = h * 0x01000193u ^ p[i];
    return h;
}

void put_u32(std::vector<uint8_t>& b, size_t at, uint32_t v) { std::memcpy(b.data() + at, &v, sizeof v); }

// One valid FEFW account save slot: the 0x124-byte container and the zstd frame the game will read from 0x108.
std::vector<uint8_t> slot_with_payload(const std::string& payload) {
    std::vector<uint8_t> frame(ZSTD_compressBound(payload.size()));
    const size_t n = ZSTD_compress(frame.data(), frame.size(), payload.data(), payload.size(), 3);
    if (ZSTD_isError(n)) return {};
    frame.resize(n);
    std::vector<uint8_t> raw(0x124);
    put_u32(raw, 0, kSlotMagic);
    put_u32(raw, 4, 0xE0);
    raw.insert(raw.end(), frame.begin(), frame.end());
    put_u32(raw, 0x100, static_cast<uint32_t>(raw.size() - 0x104));
    put_u32(raw, 0x104, kSlotMagic);
    put_u32(raw, 0x108, static_cast<uint32_t>(n));
    put_u32(raw, 0x10C, fnv1_32(reinterpret_cast<const uint8_t*>(payload.data()), payload.size()));
    put_u32(raw, 0x110, 3);
    put_u32(raw, 8, fnv1a32(raw.data() + 0x1F, 0xE0));
    return raw;
}

void write_bytes(const stdfs::path& p, const std::vector<uint8_t>& data) {
    std::error_code ec;
    stdfs::create_directories(p.parent_path(), ec);
    std::ofstream o(p, std::ios::binary | std::ios::trunc);
    o.write(reinterpret_cast<const char*>(data.data()), static_cast<std::streamsize>(data.size()));
}

}  // namespace

NXTEST(fsp_set_current_process_and_log_mode) {
    Fixture f;
    Session s = open_fsp();
    Reply r = s.call(1005);
    CHECK(r.ok());
    CHECK_EQ(r.get<uint32_t>(0), 0u);
    r = s.call(1011);                        // GetProgramIndexForAccessLog
    CHECK(r.ok());
    r = s.call(27);                          // IsExFatSupported
    CHECK(r.ok());
    CHECK_EQ(r.get<uint8_t>(0), 0u);
}

NXTEST(fsp_unknown_command_is_stub) {
    Fixture f;
    Session s = open_fsp();
    CHECK_EQ(s.call(9999).result, nxrt::kernel::kStubService);
}

NXTEST(rom_storage_header_and_file_data) {
    Fixture f;
    Session s = open_fsp();
    Reply r = s.call(200);
    REQUIRE(r.ok());
    Session st = s.sub(r);
    REQUIRE(st.valid());
    r = st.call(4);                          // IStorage::GetSize
    REQUIRE(r.ok());
    const int64_t total = r.get<int64_t>(0);
    CHECK(total > 0x200 + 1003);
    uint8_t hdr[0x50] = {};
    r = st.call(0, Args().i64(0).i64(0x50).buf_b(hdr, 0x50));
    CHECK(r.ok());
    uint64_t f64[10];
    std::memcpy(f64, hdr, 0x50);
    CHECK_EQ(f64[0], 0x50u);                 // header size
    const uint64_t dir_hash = f64[1], dir_tab = f64[3], file_hash = f64[5], file_tab = f64[7], data = f64[9];
    CHECK_EQ(data, 0x200u);
    CHECK(dir_hash > data && dir_tab > dir_hash && file_hash > dir_tab && file_tab > file_hash);
    CHECK(file_tab + f64[8] <= static_cast<uint64_t>(total));
    // root directory entry: first entry of the directory table; its first file is a.bin (sorted), with the data we wrote
    uint32_t root[6] = {};
    r = st.call(0, Args().i64(static_cast<int64_t>(dir_tab)).i64(24).buf_b(root, 24));
    CHECK(r.ok());
    CHECK_EQ(root[1], 0xFFFFFFFFu);          // no sibling
    CHECK(root[2] != 0xFFFFFFFFu);           // child directory (Worlds)
    CHECK(root[3] != 0xFFFFFFFFu);           // child file (a.bin)
    uint8_t fe[0x20 + 8] = {};
    r = st.call(0, Args().i64(static_cast<int64_t>(file_tab + root[3])).i64(0x28).buf_b(fe, 0x28));
    CHECK(r.ok());
    // file entry (Switchbrew RomFS): parent u32, sibling u32, data offset u64, data size u64, hash u32, name length u32, name
    uint64_t off, size;
    uint32_t name_len;
    std::memcpy(&off, fe + 0x08, 8);
    std::memcpy(&size, fe + 0x10, 8);
    std::memcpy(&name_len, fe + 0x1C, 4);
    CHECK_EQ(size, 1000u);
    CHECK_EQ(name_len, 5u);
    char content[1000];
    r = st.call(0, Args().i64(static_cast<int64_t>(data + off)).i64(1000).buf_b(content, 1000));
    CHECK(r.ok());
    CHECK_EQ(std::string(content, 1000), std::string(1000, 'a'));
}

NXTEST(rom_storage_read_out_of_range) {
    Fixture f;
    Session s = open_fsp();
    Session st = s.sub(s.call(200));
    REQUIRE(st.valid());
    const int64_t total = st.call(4).get<int64_t>(0);
    char buf[16];
    CHECK_EQ(st.call(0, Args().i64(total - 8).i64(16).buf_b(buf, 16)).result, kOutOfRange);
    CHECK(st.call(0, Args().i64(total - 16).i64(16).buf_b(buf, 16)).ok());
    CHECK_EQ(st.call(1, Args().i64(0).i64(1).buf_a(buf, 1)).result, 0x313802u);   // writes to the RomFS are refused
}

NXTEST(rom_missing_folder_is_target_not_found) {
    Fixture f;
    nxrt::config_set("fs", "romfs", (f.dir / "does_not_exist").string().c_str());
    nxrt::services::reset_all();
    Session s = open_fsp();
    CHECK_EQ(s.call(200).result, kTargetNotFound);
}

NXTEST(rom_file_system_read_only) {
    Fixture f;
    Session s = open_fsp();
    Reply r = s.call(2);                     // OpenDataFileSystemByCurrentProcess
    REQUIRE(r.ok());
    Session fsys = s.sub(r);
    // GetEntryType
    r = fs_call_path(fsys, 7, "/a.bin");
    CHECK(r.ok());
    CHECK_EQ(r.get<uint32_t>(0), 1u);
    r = fs_call_path(fsys, 7, "/Worlds");
    CHECK_EQ(r.get<uint32_t>(0), 0u);
    CHECK_EQ(fs_call_path(fsys, 7, "/nope").result, kPathNotFound);
    // OpenFile + Read
    r = fs_call_path(fsys, 8, "/Worlds/b.pak", Args().u32(1));
    REQUIRE(r.ok());
    Session file = fsys.sub(r);
    char buf[16] = {};
    r = file.call(0, Args().u32(0).pad(4).i64(0).i64(16).buf_b(buf, 16));
    CHECK(r.ok());
    CHECK_EQ(r.get<uint64_t>(0), 3u);
    CHECK_EQ(std::string(buf, 3), std::string("bee"));
    CHECK_EQ(file.call(4).get<int64_t>(0), 3);
    // refuses writes
    CHECK_EQ(fs_call_path(fsys, 1, "/a.bin").result, 0x313802u);
    // OpenDirectory (files and directories), read entries
    r = fs_call_path(fsys, 9, "/", Args().u32(3));
    REQUIRE(r.ok());
    Session dir = fsys.sub(r);
    CHECK_EQ(dir.call(1).get<int64_t>(0), 2);
    std::vector<uint8_t> ents(0x310 * 4, 0xEE);
    r = dir.call(0, Args().buf_b(ents.data(), ents.size()));
    CHECK(r.ok());
    CHECK_EQ(r.get<int64_t>(0), 2);
    CHECK_EQ(std::string(reinterpret_cast<char*>(ents.data())), std::string("Worlds"));
    CHECK_EQ(ents[0x304], 0u);
    CHECK_EQ(std::string(reinterpret_cast<char*>(ents.data() + 0x310)), std::string("a.bin"));
    CHECK_EQ(ents[0x310 + 0x304], 1u);
    int64_t fsz;
    std::memcpy(&fsz, &ents[0x310 + 0x308], 8);
    CHECK_EQ(fsz, 1000);
}

// The data of the root's first file (a.bin in the fixture) read through the RomFS tables of IStorage `st`.
static std::string first_root_file(Session& st) {
    uint64_t f64[10] = {};
    if (!st.call(0, Args().i64(0).i64(0x50).buf_b(f64, 0x50)).ok()) return {};
    uint32_t root[6] = {};
    if (!st.call(0, Args().i64(static_cast<int64_t>(f64[3])).i64(24).buf_b(root, 24)).ok()) return {};
    uint8_t fe[0x28] = {};
    if (!st.call(0, Args().i64(static_cast<int64_t>(f64[7] + root[3])).i64(0x28).buf_b(fe, 0x28)).ok()) return {};
    uint64_t off, size;
    std::memcpy(&off, fe + 0x08, 8);
    std::memcpy(&size, fe + 0x10, 8);
    std::string data(static_cast<size_t>(size), '\0');
    if (size && !st.call(0, Args().i64(static_cast<int64_t>(f64[9] + off)).i64(static_cast<int64_t>(size)).buf_b(data.data(), size)).ok()) return {};
    return data;
}

NXTEST(rom_storage_with_mods) {
    Fixture f;
    nxrt::services::profile_reload();
    const stdfs::path mods = f.dir / "mods";
    std::error_code ec;
    char other[17];
    std::snprintf(other, sizeof other, "%016llX", static_cast<unsigned long long>(nxrt::services::profile().program_id + 0x10000));
    for (const char* m : {"01-first", "02-second", "_off", "zz-exefs-only"}) stdfs::create_directories(mods / m / "romfs", ec);
    stdfs::remove_all(mods / "zz-exefs-only" / "romfs", ec);
    stdfs::create_directories(mods / "zz-exefs-only" / "exefs", ec);
    stdfs::create_directories(mods / "contents" / other / "x" / "romfs", ec);
    Fixture::write(mods / "01-first" / "romfs" / "A.BIN", "first");      // case differs from the game's a.bin
    Fixture::write(mods / "02-second" / "romfs" / "a.bin", "second");    // sorted last: wins
    Fixture::write(mods / "_off" / "romfs" / "a.bin", "off");
    Fixture::write(mods / "contents" / other / "x" / "romfs" / "a.bin", "other title");
    nxrt::config_set("fs", "mods", mods.string().c_str());
    nxrt::services::reset_all();
    Session s = open_fsp();
    Session st = s.sub(s.call(200));
    REQUIRE(st.valid());
    CHECK_EQ(first_root_file(st), std::string("second"));
    // without mods the game's file is served
    nxrt::config_set("fs", "mods", (f.dir / "no_mods").string().c_str());
    nxrt::services::reset_all();
    Session s2 = open_fsp();
    Session st2 = s2.sub(s2.call(200));
    REQUIRE(st2.valid());
    CHECK_EQ(first_root_file(st2), std::string(1000, 'a'));
}

// #39: packed-package files are base game data, so their overlay sits below every installed mod. The order the facade builds is
// base RomFS, [fs] romfs_overlay, then installed mods in name order; this walks that order one layer at a time.
NXTEST(rom_storage_package_overlay_is_below_mods) {
    Fixture f;
    nxrt::services::profile_reload();
    const stdfs::path overlay = f.dir / "romfs-overlay";
    const stdfs::path mods = f.dir / "mods";
    const stdfs::path disabled = f.dir / "mods-disabled";
    std::error_code ec;
    stdfs::create_directories(overlay, ec);
    Fixture::write(overlay / "a.bin", "package-overlay");
    nxrt::config_set("fs", "romfs_overlay", overlay.string().c_str());

    auto root_file = [&]() -> std::string {
        nxrt::services::reset_all();
        Session s = open_fsp();
        const Reply r = s.call(200);
        if (!r.ok()) return {};
        Session st = s.sub(r);
        return st.valid() ? first_root_file(st) : std::string();
    };

    // 2. The package overlay replaces the base file.
    CHECK_EQ(root_file(), std::string("package-overlay"));

    for (const char* m : {"01-first", "02-second", "_off"}) stdfs::create_directories(mods / m / "romfs", ec);
    Fixture::write(mods / "01-first" / "romfs" / "a.bin", "mod-one");
    Fixture::write(mods / "02-second" / "romfs" / "a.bin", "mod-two");
    Fixture::write(mods / "_off" / "romfs" / "a.bin", "disabled");
    nxrt::config_set("fs", "mods", mods.string().c_str());
    // 3 and 4. A mod replaces the package overlay, and the last mod in name order wins.
    CHECK_EQ(root_file(), std::string("mod-two"));

    // 5. A mod disabled by its leading `_` is left out, restoring the package overlay underneath it.
    stdfs::create_directories(disabled / "_off" / "romfs", ec);
    Fixture::write(disabled / "_off" / "romfs" / "a.bin", "disabled");
    nxrt::config_set("fs", "mods", disabled.string().c_str());
    CHECK_EQ(root_file(), std::string("package-overlay"));

    // 1. With neither a package overlay nor a mod, the base RomFS file is visible.
    nxrt::config_set("fs", "romfs_overlay", "");
    nxrt::config_set("fs", "mods", (f.dir / "no_mods").string().c_str());
    CHECK_EQ(root_file(), std::string(1000, 'a'));
}

NXTEST(aoc_storage_by_data_id) {
    Fixture f;
    nxrt::services::profile_reload();
    const uint64_t id = nxrt::services::profile().program_id + 0x1001ull;
    char name[17];
    std::snprintf(name, sizeof name, "%016llX", static_cast<unsigned long long>(id));
    std::error_code ec;
    stdfs::create_directories(f.dir / "dlc" / name / "0" / "romfs", ec);
    Fixture::write(f.dir / "dlc" / name / "0" / "romfs" / "dlc1.txt", "x");
    nxrt::config_set("aoc", "dir", (f.dir / "dlc").string().c_str());
    Session s = open_fsp();
    // OpenDataStorageByDataId (cmd 202): u8 storage id, 7 bytes padding, u64 data id.
    Reply r = s.call(202, Args().u8(0).pad(7).u64(id));
    REQUIRE(r.ok());
    Session st = s.sub(r);
    REQUIRE(st.valid());
    CHECK(st.call(4).get<int64_t>(0) > 0x200);
    uint64_t hdr[10] = {};
    CHECK(st.call(0, Args().i64(0).i64(0x50).buf_b(hdr, 0x50)).ok());
    CHECK_EQ(hdr[0], 0x50u);
    CHECK_EQ(s.call(202, Args().u8(0).pad(7).u64(id + 1)).result, kTargetNotFound);   // not supplied
}

NXTEST(sd_card_is_absent) {
    Fixture f;
    Session s = open_fsp();
    CHECK_EQ(s.call(18).result, kSdNoDevice);
    Session op = s.sub(s.call(400));
    REQUIRE(op.valid());
    Reply r = op.call(0);
    CHECK(r.ok());
    CHECK_EQ(r.get<uint8_t>(0), 0u);
    r = s.call(631);
    CHECK_EQ(r.get<uint8_t>(0), 0u);
}

NXTEST(save_open_missing_then_create_open_write_commit) {
    Fixture f;
    Session s = open_fsp();
    const std::vector<uint8_t> a = attr(1, 0x1111, 0x2222);
    CHECK_EQ(open_save(s, a).result, kTargetNotFound);
    CHECK(create_save(s, a, 4 << 20, 1 << 20).ok());
    CHECK_EQ(create_save(s, a, 4 << 20, 1 << 20).result, kPathExists);
    Reply r = open_save(s, a);
    REQUIRE(r.ok());
    {
        Session fsys = s.sub(r);
        CHECK(fs_call_path(fsys, 0, "/slot0", Args().u32(0).pad(4).i64(16)).ok());      // CreateFile(option, size)
        r = fs_call_path(fsys, 8, "/slot0", Args().u32(6));                                // write | append
        REQUIRE(r.ok());
        Session file = fsys.sub(r);
        const char data[16] = {'M', 'P', 'R', 'S', 'A', 'V', 'E', '0', 1, 2, 3, 4, 5, 6, 7, 8};
        CHECK(file.call(1, Args().u32(1).pad(4).i64(0).i64(16).buf_a(data, 16)).ok());    // Write with the flush option
        CHECK_EQ(file.call(1, Args().u32(0).pad(4).i64(16).i64(8).buf_a(data, 8)).ok(), true);
        CHECK(fsys.call(10).ok());                                                          // Commit
        CHECK(fs_call_path(fsys, 0, "/slot1", Args().u32(0).pad(4).i64(4)).ok());          // not committed
    }
    // both sessions closed: slot1 is gone, slot0 stays
    r = open_save(s, a);
    REQUIRE(r.ok());
    Session fsys = s.sub(r);
    Reply t = fs_call_path(fsys, 7, "/slot0");
    CHECK(t.ok());
    CHECK_EQ(t.get<uint32_t>(0), 1u);
    CHECK_EQ(fs_call_path(fsys, 7, "/slot1").result, kPathNotFound);
    r = fs_call_path(fsys, 8, "/slot0", Args().u32(1));
    REQUIRE(r.ok());
    Session file = fsys.sub(r);
    CHECK_EQ(file.call(4).get<int64_t>(0), 24);
    char buf[24] = {};
    CHECK(file.call(0, Args().u32(0).pad(4).i64(0).i64(24).buf_b(buf, 24)).ok());
    CHECK_EQ(std::string(buf, 8), std::string("MPRSAVE0"));
    // write without the append mode past the end
    r = fs_call_path(fsys, 8, "/slot0", Args().u32(2));
    Session w = fsys.sub(r);
    CHECK_EQ(w.call(1, Args().u32(0).pad(4).i64(20).i64(8).buf_a(buf, 8)).result, kNoAppend);
}

NXTEST(save_disk_layout_is_a_folder_under_save_dir) {
    Fixture f;
    Session s = open_fsp();
    CHECK(create_save(s, attr(1, 5, 6), 4 << 20, 1 << 20).ok());
    CHECK(stdfs::is_directory(f.save / "account" / "05000000000000000600000000000000"));
    CHECK(stdfs::exists(f.save / "account" / "05000000000000000600000000000000.meta"));
    CHECK(create_save(s, attr(3, 0, 0), 1 << 20, 1 << 18).ok());       // device save
    CHECK(stdfs::is_directory(f.save / "device"));
    CHECK(create_save(s, attr(5, 0, 0, 2), 1 << 20, 1 << 18).ok());    // cache storage index 2
    CHECK(stdfs::is_directory(f.save / "cache" / "2"));
}

// #77: the game's zstd wrapper turns a decoder error into an absurd copy size. The account Auto slot must therefore reach
// the guest only after its container and frame decode; an absent slot is a state the game already handles. The guard is a
// read gate: writing the slot (to replace or repair it) and unrelated save files stay open.
NXTEST(save_guard_slot_validator_accepts_only_a_decodable_slot) {
    const auto cb = nxrt::game::info().save_file_readable;
    REQUIRE(cb != nullptr);
    const std::vector<uint8_t> good = slot_with_payload(std::string(0x4000, 'A'));
    REQUIRE(!good.empty());
    CHECK(cb("save:account/11110000000000002222000000000000", "/000/Auto", good.data(), good.size()));
    std::vector<uint8_t> damaged = good;
    for (size_t i = 0; i < 48 && 0x124 + i < damaged.size(); ++i) damaged[0x124 + i] ^= 0xFF;
    CHECK(!cb("save:account/11110000000000002222000000000000", "/000/Auto", damaged.data(), damaged.size()));
    std::vector<uint8_t> bad_fnv = good;
    uint32_t payload_fnv = 0;
    std::memcpy(&payload_fnv, good.data() + 0x10C, sizeof payload_fnv);
    put_u32(bad_fnv, 0x10C, payload_fnv ^ 1u);
    CHECK(!cb("save:account/11110000000000002222000000000000", "/000/Auto", bad_fnv.data(), bad_fnv.size()));
    std::vector<uint8_t> bad_header = good;
    bad_header[8] ^= 0xFF;
    CHECK(!cb("save:account/11110000000000002222000000000000", "/000/Auto", bad_header.data(), bad_header.size()));
}

NXTEST(save_guard_rejects_a_damaged_autosave_but_not_other_save_files) {
    Fixture f;
    Session s = open_fsp();
    const auto a = attr(1, 0x1111, 0x2222);
    REQUIRE(create_save(s, a, 4 << 20, 1 << 20).ok());
    const stdfs::path dir = f.save / "account" / "11110000000000002222000000000000" / "000";
    const stdfs::path auto_path = dir / "Auto";
    const std::vector<uint8_t> good = slot_with_payload(std::string(0x4000, 'A'));
    REQUIRE(!good.empty());
    write_bytes(auto_path, good);

    Reply r = open_save(s, a);
    REQUIRE(r.ok());
    Session fsys = s.sub(r);
    REQUIRE(fsys.valid());
    r = fs_call_path(fsys, 8, "/000/Auto", Args().u32(1));
    REQUIRE(r.ok());
    {
        Session file = fsys.sub(r);
        CHECK_EQ(file.call(4).get<int64_t>(0), static_cast<int64_t>(good.size()));
    }

    // The reported failure: a frame that native zstd rejects. The game sees the same "no autosave" path as a moved file.
    std::vector<uint8_t> damaged = good;
    for (size_t i = 0; i < 48 && 0x124 + i < damaged.size(); ++i) damaged[0x124 + i] ^= 0xFF;
    write_bytes(auto_path, damaged);
    Reply damaged_open = fs_call_path(fsys, 8, "/000/Auto", Args().u32(1));
    CHECK_EQ(damaged_open.result, kPathNotFound);
    Reply damaged_read_write = fs_call_path(fsys, 8, "/000/Auto", Args().u32(3));
    CHECK_EQ(damaged_read_write.result, kPathNotFound);

    // The guard is only for reads: replacing the damaged slot by opening it for write is still allowed.
    {
        Reply wr = fs_call_path(fsys, 8, "/000/Auto", Args().u32(2));
        CHECK(wr.ok());
        Session w = fsys.sub(wr);
    }

    // Another save file with the same bytes is not the Auto slot and must not be hidden.
    write_bytes(dir / "Global", damaged);
    {
        Reply global_open = fs_call_path(fsys, 8, "/000/Global", Args().u32(1));
        CHECK(global_open.ok());
        Session global_file = fsys.sub(global_open);
    }

    // A declared frame length that does not match the frame is rejected too.
    std::vector<uint8_t> bad_length = good;
    put_u32(bad_length, 0x108, static_cast<uint32_t>(bad_length.size() - 0x124 - 1));
    write_bytes(auto_path, bad_length);
    Reply bad_length_open = fs_call_path(fsys, 8, "/000/Auto", Args().u32(1));
    CHECK_EQ(bad_length_open.result, kPathNotFound);

    // Restoring the original bytes restores the open once the uncommitted write session is gone, so the guard did not
    // alter or consume the file.
    fsys.close();
    write_bytes(auto_path, good);
    r = open_save(s, a);
    REQUIRE(r.ok());
    Session reopened = s.sub(r);
    REQUIRE(reopened.valid());
    Reply restored_open = fs_call_path(reopened, 8, "/000/Auto", Args().u32(1));
    CHECK(restored_open.ok());
    Session restored_file = reopened.sub(restored_open);
}

NXTEST(save_info_reader_lists_saves) {
    Fixture f;
    Session s = open_fsp();
    CHECK(create_save(s, attr(1, 0xAA, 0xBB), 4 << 20, 1 << 20).ok());
    Reply r = s.call(61, Args().u8(1));      // OpenSaveDataInfoReaderBySaveDataSpaceId
    REQUIRE(r.ok());
    Session rd = s.sub(r);
    std::vector<uint8_t> infos(0x60 * 4, 0);
    r = rd.call(0, Args().buf_b(infos.data(), infos.size()));
    CHECK(r.ok());
    CHECK_EQ(r.get<int64_t>(0), 1);
    CHECK_EQ(infos[9], 1u);                   // type: account
    uint64_t hi, size;
    std::memcpy(&hi, &infos[0x10], 8);
    std::memcpy(&size, &infos[0x30], 8);
    CHECK_EQ(hi, 0xAAu);
    CHECK_EQ(size, static_cast<uint64_t>(4 << 20));
    r = rd.call(0, Args().buf_b(infos.data(), infos.size()));
    CHECK_EQ(r.get<int64_t>(0), 0);           // exhausted
}

NXTEST(save_delete_by_attribute) {
    Fixture f;
    Session s = open_fsp();
    const auto a = attr(1, 7, 8);
    CHECK(create_save(s, a, 1 << 20, 1 << 18).ok());
    Args args;
    args.u8(1).pad(7).bytes(a.data(), a.size());
    CHECK(s.call(28, args).ok());
    CHECK_EQ(open_save(s, a).result, kTargetNotFound);
    CHECK_EQ(s.call(28, args).result, kTargetNotFound);
}

NXTEST(save_on_sd_space_has_no_device) {
    Fixture f;
    Session s = open_fsp();
    CHECK_EQ(open_save(s, attr(1, 1, 1), 4).result, kSdNoDevice);
}

NXTEST(account_save_api) {
    Fixture f;
    bool created = false;
    int64_t size = 0, journal = 0;
    CHECK_EQ(nxrt::services::fsapi::account_save_size(1, 2, &size, &journal), kTargetNotFound);
    CHECK_EQ(nxrt::services::fsapi::ensure_account_save(1, 2, &created), 0u);
    CHECK(created);
    CHECK_EQ(nxrt::services::fsapi::ensure_account_save(1, 2, &created), 0u);
    CHECK(!created);
    CHECK_EQ(nxrt::services::fsapi::account_save_size(1, 2, &size, &journal), 0u);
    CHECK_EQ(size, static_cast<int64_t>(nxrt::services::profile().account_save_size));   // the registered game's control.nacp sizes
    CHECK_EQ(journal, static_cast<int64_t>(nxrt::services::profile().account_save_journal));
    // the same save is then openable through fsp-srv with the attribute the SDK builds for MountSaveData
    Session s = open_fsp();
    CHECK(open_save(s, attr(1, 1, 2)).ok());
}

NXTEST(multi_commit_manager_opens) {
    Fixture f;
    Session s = open_fsp();
    const auto a = attr(1, 3, 4);
    CHECK(create_save(s, a, 1 << 20, 1 << 18).ok());
    Session fsys = s.sub(open_save(s, a));
    REQUIRE(fsys.valid());
    CHECK(fs_call_path(fsys, 2, "/d").ok());
    // IMultiCommitManager::Add takes the file system as an object argument: the interface travels as a move handle
    Session mgr = s.sub(s.call(1200));
    REQUIRE(mgr.valid());
    // (Add / Commit are exercised through the file system's own Commit: the manager only forwards)
    CHECK(fsys.call(10).ok());
    CHECK(stdfs::is_directory(f.save / "account" / "03000000000000000400000000000000" / "d"));
}
