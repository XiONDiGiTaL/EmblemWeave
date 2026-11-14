// SPDX-License-Identifier: GPL-3.0-or-later
// Host-folder volumes (fs_volume.h): plain file operations, the journalled save semantics (copy on write, tombstones, two-phase commit,
// recovery after a crash between the phases) and path hygiene.
#define _SILENCE_CXX20_U8PATH_DEPRECATION_WARNING
#include <windows.h>

#include <cstring>
#include <filesystem>
#include <fstream>

#include "../src/services/fs_volume.h"
#include "test_framework.h"

NXTEST_GROUP(svc_fs_volume)

namespace stdfs = std::filesystem;
using namespace nxrt::services::fs;

namespace {

struct TempDir {
    stdfs::path p;
    explicit TempDir(const char* tag) {
        p = stdfs::temp_directory_path() / (std::string("nxrt_fsv_") + tag + "_" + std::to_string(GetCurrentProcessId()));
        std::error_code ec;
        stdfs::remove_all(p, ec);
        stdfs::create_directories(p, ec);
    }
    ~TempDir() {
        std::error_code ec;
        stdfs::remove_all(p, ec);
    }
    std::string root(const char* leaf) const { return (p / leaf).string(); }
};

std::string read_all(Volume& v, const std::string& path) {
    std::shared_ptr<VFile> f;
    if (v.open_file(path, kModeRead, &f)) return "<open failed>";
    int64_t sz = 0;
    f->get_size(&sz);
    std::string s(static_cast<size_t>(sz), '\0');
    uint64_t got = 0;
    f->read(0, s.data(), s.size(), &got);
    s.resize(static_cast<size_t>(got));
    return s;
}

bool put(Volume& v, const std::string& path, const std::string& data, bool create = true) {
    if (create && v.create_file(path, 0) != 0) return false;
    std::shared_ptr<VFile> f;
    if (v.open_file(path, kModeWrite | kModeAppend, &f)) return false;
    return f->write(0, data.data(), data.size()) == 0 && f->set_size(static_cast<int64_t>(data.size())) == 0;
}

bool names_are(Volume& v, const std::string& dir, std::initializer_list<const char*> want) {
    std::vector<DirItem> items;
    if (v.list_dir(dir, &items)) return false;
    if (items.size() != want.size()) return false;
    size_t i = 0;
    for (const char* w : want)
        if (items[i++].name != w) return false;
    return true;
}

}  // namespace

NXTEST(normalize_paths) {
    std::string out;
    uint32_t err = 0;
    CHECK(normalize_path("/a/b", 0x301, &out, &err));
    CHECK_EQ(out, std::string("/a/b"));
    CHECK(normalize_path("//a///b//", 0x301, &out, &err));
    CHECK_EQ(out, std::string("/a/b"));
    CHECK(normalize_path("/", 0x301, &out, &err));
    CHECK_EQ(out, std::string("/"));
    CHECK(!normalize_path("a/b", 0x301, &out, &err));            // not rooted
    CHECK(!normalize_path("/a/../b", 0x301, &out, &err));
    CHECK(!normalize_path("/a/./b", 0x301, &out, &err));
    CHECK(!normalize_path("/a\\b", 0x301, &out, &err));
    CHECK(!normalize_path("/a:b", 0x301, &out, &err));
    CHECK_EQ(err, nxrt::services::fs::rc::kInvalidArgument);
    std::string longp = "/" + std::string(0x310, 'x');
    CHECK(!normalize_path(longp.c_str(), 0x301, &out, &err));
    CHECK_EQ(err, nxrt::services::fs::rc::kTooLongPath);
    char unterminated[4] = {'/', 'a', 'b', 'c'};
    CHECK(!normalize_path(unterminated, 4, &out, &err));
}

NXTEST(direct_volume_basic_operations) {
    TempDir t("direct");
    Volume v(t.root("d"), false, false, 0);
    REQUIRE(v.ok());
    CHECK_EQ(v.create_dir("/sub"), 0u);
    CHECK_EQ(v.create_dir("/sub"), rc::kPathAlreadyExists);
    CHECK_EQ(v.create_dir("/x/y"), rc::kPathNotFound);          // parent missing
    CHECK(put(v, "/sub/a.txt", "hello"));
    CHECK_EQ(v.create_file("/sub/a.txt", 0), rc::kPathAlreadyExists);
    CHECK_EQ(read_all(v, "/sub/a.txt"), std::string("hello"));
    uint32_t type = 9;
    CHECK_EQ(v.entry_type("/sub", &type), 0u);
    CHECK_EQ(type, 0u);
    CHECK_EQ(v.entry_type("/sub/a.txt", &type), 0u);
    CHECK_EQ(type, 1u);
    CHECK_EQ(v.entry_type("/nope", &type), rc::kPathNotFound);
    CHECK_EQ(v.delete_dir("/sub"), rc::kDirectoryNotEmpty);
    CHECK_EQ(v.rename_file("/sub/a.txt", "/sub/b.txt"), 0u);
    CHECK(names_are(v, "/sub", {"b.txt"}));
    CHECK_EQ(v.rename_dir("/sub", "/moved"), 0u);
    CHECK_EQ(read_all(v, "/moved/b.txt"), std::string("hello"));
    CHECK_EQ(v.delete_file("/moved/b.txt"), 0u);
    CHECK_EQ(v.delete_dir("/moved"), 0u);
    CHECK(names_are(v, "/", {}));
    // size, sparse growth
    CHECK_EQ(v.create_file("/big", 4096), 0u);
    std::shared_ptr<VFile> f;
    REQUIRE(v.open_file("/big", kModeRead, &f) == 0);
    int64_t sz = 0;
    f->get_size(&sz);
    CHECK_EQ(sz, 4096);
    // commit on a direct volume is a no-op
    CHECK_EQ(v.commit(), 0u);
}

NXTEST(file_open_modes) {
    TempDir t("modes");
    Volume v(t.root("d"), false, false, 0);
    REQUIRE(put(v, "/f", "abcd"));
    std::shared_ptr<VFile> f;
    REQUIRE(v.open_file("/f", kModeRead, &f) == 0);
    CHECK_EQ(f->write(0, "x", 1), rc::kWriteNotPermitted);
    uint64_t got = 99;
    char buf[8] = {};
    CHECK_EQ(f->read(2, buf, 8, &got), 0u);                   // short read at the end
    CHECK_EQ(got, 2u);
    CHECK_EQ(f->read(10, buf, 8, &got), 0u);                  // past the end: nothing, not an error
    CHECK_EQ(got, 0u);
    REQUIRE(v.open_file("/f", kModeWrite, &f) == 0);
    CHECK_EQ(f->write(2, "xyz", 3), rc::kNoAppend);           // would extend the file without the append mode
    CHECK_EQ(f->write(1, "XY", 2), 0u);
    CHECK_EQ(read_all(v, "/f"), std::string("aXYd"));
    REQUIRE(v.open_file("/f", kModeWrite | kModeAppend, &f) == 0);
    CHECK_EQ(f->write(4, "!!", 2), 0u);
    CHECK_EQ(read_all(v, "/f"), std::string("aXYd!!"));
    CHECK_EQ(v.open_file("/missing", kModeRead, &f), rc::kPathNotFound);
}

NXTEST(read_only_volume_refuses_writes) {
    TempDir t("ro");
    {
        Volume w(t.root("d"), false, false, 0);
        REQUIRE(put(w, "/f", "data"));
    }
    Volume v(t.root("d"), false, true, 0);
    REQUIRE(v.ok());
    CHECK_EQ(read_all(v, "/f"), std::string("data"));
    CHECK_EQ(v.create_file("/g", 0), rc::kReadOnlyFileSystem);
    CHECK_EQ(v.delete_file("/f"), rc::kReadOnlyFileSystem);
    CHECK_EQ(v.create_dir("/d"), rc::kReadOnlyFileSystem);
    std::shared_ptr<VFile> f;
    CHECK_EQ(v.open_file("/f", kModeWrite, &f), rc::kReadOnlyFileSystem);
}

NXTEST(journal_uncommitted_changes_are_dropped) {
    TempDir t("drop");
    {
        Volume v(t.root("save"), true, false, 0);
        REQUIRE(v.ok());
        CHECK(put(v, "/kept", "one"));
        CHECK_EQ(v.commit(), 0u);
        CHECK(put(v, "/lost", "two"));
        CHECK_EQ(read_all(v, "/lost"), std::string("two"));    // visible inside the session
        CHECK(put(v, "/kept", "changed", false));
        CHECK_EQ(read_all(v, "/kept"), std::string("changed"));
    }
    Volume v(t.root("save"), true, false, 0);
    CHECK_EQ(read_all(v, "/kept"), std::string("one"));         // the committed content
    uint32_t type;
    CHECK_EQ(v.entry_type("/lost", &type), rc::kPathNotFound);
    // The committed folder was never written outside commit: the file there still holds the old bytes.
    std::ifstream in(t.p / "save" / "kept", std::ios::binary);
    std::string s((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
    CHECK_EQ(s, std::string("one"));
}

NXTEST(journal_commit_applies_changes_and_deletions) {
    TempDir t("commit");
    Volume v(t.root("save"), true, false, 0);
    REQUIRE(v.ok());
    CHECK_EQ(v.create_dir("/d"), 0u);
    CHECK(put(v, "/d/a", "A"));
    CHECK(put(v, "/d/b", "B"));
    CHECK(put(v, "/top", "T"));
    CHECK_EQ(v.commit(), 0u);
    CHECK(stdfs::exists(t.p / "save" / "d" / "a"));
    // modify, delete, create, rename, all in one commit
    CHECK(put(v, "/d/a", "A2", false));
    CHECK_EQ(v.delete_file("/d/b"), 0u);
    CHECK(put(v, "/d/c", "C"));
    CHECK_EQ(v.rename_file("/top", "/d/top2"), 0u);
    CHECK(names_are(v, "/d", {"a", "c", "top2"}));              // the union view before the commit
    CHECK(names_are(v, "/", {"d"}));
    CHECK(stdfs::exists(t.p / "save" / "d" / "b"));             // still committed on disk
    CHECK_EQ(v.commit(), 0u);
    CHECK(!stdfs::exists(t.p / "save" / "d" / "b"));
    CHECK(!stdfs::exists(t.p / "save" / "top"));
    CHECK(stdfs::exists(t.p / "save" / "d" / "top2"));
    CHECK_EQ(read_all(v, "/d/a"), std::string("A2"));
    CHECK(names_are(v, "/d", {"a", "c", "top2"}));
    CHECK(!stdfs::exists(t.p / "save.commit"));                 // marker removed
    // delete a directory tree that is committed, re-create it empty, commit: children are gone
    CHECK_EQ(v.delete_dir_recursive("/d"), 0u);
    CHECK_EQ(v.create_dir("/d"), 0u);
    CHECK(names_are(v, "/d", {}));
    CHECK(put(v, "/d/new", "N"));
    CHECK_EQ(v.commit(), 0u);
    CHECK(!stdfs::exists(t.p / "save" / "d" / "a"));
    CHECK_EQ(read_all(v, "/d/new"), std::string("N"));
    CHECK(names_are(v, "/d", {"new"}));
}

NXTEST(journal_delete_then_recreate_same_path) {
    TempDir t("recreate");
    Volume v(t.root("save"), true, false, 0);
    CHECK(put(v, "/f", "old"));
    CHECK_EQ(v.commit(), 0u);
    CHECK_EQ(v.delete_file("/f"), 0u);
    uint32_t type;
    CHECK_EQ(v.entry_type("/f", &type), rc::kPathNotFound);
    CHECK(put(v, "/f", "new"));
    CHECK_EQ(read_all(v, "/f"), std::string("new"));
    CHECK_EQ(v.commit(), 0u);
    CHECK_EQ(read_all(v, "/f"), std::string("new"));
    std::ifstream in(t.p / "save" / "f", std::ios::binary);
    std::string s((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
    CHECK_EQ(s, std::string("new"));
}

NXTEST(journal_rename_directory_with_committed_children) {
    TempDir t("rendir");
    Volume v(t.root("save"), true, false, 0);
    CHECK_EQ(v.create_dir("/a"), 0u);
    CHECK_EQ(v.create_dir("/a/sub"), 0u);
    CHECK(put(v, "/a/x", "X"));
    CHECK(put(v, "/a/sub/y", "Y"));
    CHECK_EQ(v.commit(), 0u);
    CHECK_EQ(v.rename_dir("/a", "/b"), 0u);
    CHECK(names_are(v, "/", {"b"}));
    CHECK_EQ(read_all(v, "/b/sub/y"), std::string("Y"));
    CHECK_EQ(v.commit(), 0u);
    CHECK(!stdfs::exists(t.p / "save" / "a"));
    CHECK_EQ(read_all(v, "/b/x"), std::string("X"));
    CHECK_EQ(read_all(v, "/b/sub/y"), std::string("Y"));
}

NXTEST(journal_crash_after_decision_rolls_forward) {
    TempDir t("crash1");
    {
        Volume v(t.root("save"), true, false, 0);
        CHECK(put(v, "/f", "before"));
        CHECK_EQ(v.commit(), 0u);
        CHECK(put(v, "/f", "after", false));
        CHECK(put(v, "/g", "G"));
        Volume::set_crash_point_for_tests(1);       // decided (marker renamed), nothing applied
        CHECK(v.commit() != 0);
        Volume::set_crash_point_for_tests(0);
        CHECK(stdfs::exists(t.p / "save.commit"));
        // the committed folder is still the old state
        std::ifstream in(t.p / "save" / "f", std::ios::binary);
        std::string s((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
        CHECK_EQ(s, std::string("before"));
    }   // the "crashed" process goes away: work folder kept because the marker exists
    Volume v(t.root("save"), true, false, 0);       // next mount recovers
    CHECK_EQ(read_all(v, "/f"), std::string("after"));
    CHECK_EQ(read_all(v, "/g"), std::string("G"));
    CHECK(!stdfs::exists(t.p / "save.commit"));
}

NXTEST(journal_crash_inside_apply_rolls_forward) {
    TempDir t("crash2");
    {
        Volume v(t.root("save"), true, false, 0);
        CHECK(put(v, "/a", "1"));
        CHECK(put(v, "/b", "2"));
        CHECK(put(v, "/c", "3"));
        Volume::set_crash_point_for_tests(2);       // stop after the first applied step
        CHECK(v.commit() != 0);
        Volume::set_crash_point_for_tests(0);
    }
    Volume v(t.root("save"), true, false, 0);
    CHECK_EQ(read_all(v, "/a"), std::string("1"));
    CHECK_EQ(read_all(v, "/b"), std::string("2"));
    CHECK_EQ(read_all(v, "/c"), std::string("3"));
}

NXTEST(journal_crash_before_decision_changes_nothing) {
    TempDir t("crash0");
    {
        Volume v(t.root("save"), true, false, 0);
        CHECK(put(v, "/f", "base"));
        CHECK_EQ(v.commit(), 0u);
        CHECK(put(v, "/f", "wip", false));
        // process killed with only a half written journal and no marker
        std::ofstream(t.p / ("save.journal." + std::to_string(GetCurrentProcessId()))) << "W x\nR y\nD f\n";
    }
    stdfs::remove(t.p / ("save.journal." + std::to_string(GetCurrentProcessId())));
    Volume v(t.root("save"), true, false, 0);
    CHECK_EQ(read_all(v, "/f"), std::string("base"));
}

NXTEST(journal_dead_process_leftovers_are_discarded) {
    TempDir t("dead");
    stdfs::create_directories(t.p / "save");
    stdfs::create_directories(t.p / "save.work.4000000000");   // a pid that does not run
    std::ofstream(t.p / "save.journal.4000000000") << "junk";
    Volume v(t.root("save"), true, false, 0);
    CHECK(!stdfs::exists(t.p / "save.work.4000000000"));
    CHECK(!stdfs::exists(t.p / "save.journal.4000000000"));
}

NXTEST(journal_space_accounting) {
    TempDir t("space");
    Volume v(t.root("save"), true, false, 1000);
    CHECK_EQ(v.capacity(), 1000u);
    CHECK(put(v, "/f", std::string(300, 'x')));
    CHECK_EQ(v.used_bytes(), 300u);
    CHECK_EQ(v.commit(), 0u);
    CHECK_EQ(v.used_bytes(), 300u);
    CHECK_EQ(v.delete_file("/f"), 0u);
    CHECK_EQ(v.used_bytes(), 0u);
}
