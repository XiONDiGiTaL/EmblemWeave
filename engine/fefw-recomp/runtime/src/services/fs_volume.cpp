// SPDX-License-Identifier: GPL-3.0-or-later
// Host-folder volumes for fsp-srv; see fs_volume.h for the model. The journal idea (committed folder, copy-on-write work folder,
// tombstones, commit decided by one rename) is from smo-recomp runtime/fs/nnfs.c (GPL-3.0); the code is new.
#define _SILENCE_CXX20_U8PATH_DEPRECATION_WARNING
#include "fs_volume.h"

#include <algorithm>
#include <atomic>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <map>
#include <sstream>

#include "nxrt/log.h"
#include "nxrt/platform.h"
#include "nxrt/write_watch.h"

namespace stdfs = std::filesystem;

namespace nxrt::services::fs {

namespace {

std::atomic<int> g_crash_point{0};

stdfs::path u8(const std::string& s) { return stdfs::u8path(s); }
std::string to_u8(const stdfs::path& p) {
    auto s = p.u8string();
    return std::string(s.begin(), s.end());
}

std::string rel_of(const std::string& path) { return path.size() > 1 ? path.substr(1) : std::string(); }

std::string parent_of(const std::string& path) {
    const size_t i = path.rfind('/');
    return i == 0 || i == std::string::npos ? std::string("/") : path.substr(0, i);
}

// Positioned I/O on a platform file (several threads may call it at once: no shared file position is moved).
class HostFile final : public VFile {
public:
    HostFile(nxrt::platform::File* f, uint32_t mode) : f_(f), mode_(mode) {}
    ~HostFile() override { nxrt::platform::file_close(f_); }
    uint32_t read(int64_t offset, void* dst, uint64_t size, uint64_t* got) override {
        *got = 0;
        if (offset < 0) return rc::kOutOfRange;
        if (!(mode_ & kModeRead)) return rc::kReadNotPermitted;
        uint8_t* d = static_cast<uint8_t*>(dst);
        const mem::ww::OsWriteScope ws(d, size);   // reading into a page the write tracker made read-only
        uint64_t at = 0;
        while (at < size) {
            std::size_t n = 0;
            if (!nxrt::platform::file_read_at(f_, offset + static_cast<int64_t>(at), d + at, static_cast<std::size_t>(size - at), &n))
                return rc::kGenericFailure;
            if (n == 0) break;   // the end of the file
            *got += n;
            at += n;
        }
        return 0;
    }
    uint32_t write(int64_t offset, const void* src, uint64_t size) override {
        if (offset < 0) return rc::kOutOfRange;
        if (!(mode_ & kModeWrite)) return rc::kWriteNotPermitted;
        int64_t cur = 0;
        get_size(&cur);
        if (static_cast<uint64_t>(offset) + size > static_cast<uint64_t>(cur) && !(mode_ & kModeAppend)) return rc::kNoAppend;
        const auto* s = static_cast<const uint8_t*>(src);
        const mem::ww::OsReadScope rs(s, size);   // writing from a page the write tracker guarded (a deferred GPU result)
        return nxrt::platform::file_write_at(f_, offset, s, static_cast<std::size_t>(size)) ? 0 : rc::kGenericFailure;
    }
    uint32_t flush() override { return (mode_ & kModeWrite) ? (nxrt::platform::file_flush(f_) ? 0 : rc::kGenericFailure) : 0; }
    uint32_t set_size(int64_t size) override {
        if (!(mode_ & kModeWrite)) return rc::kWriteNotPermitted;
        if (size < 0) return rc::kOutOfRange;
        std::lock_guard<std::mutex> lk(size_mu_);
        return nxrt::platform::file_truncate(f_, size) ? 0 : rc::kGenericFailure;
    }
    uint32_t get_size(int64_t* size) override {
        const int64_t n = nxrt::platform::file_size(f_);
        if (n < 0) return rc::kGenericFailure;
        *size = n;
        return 0;
    }

private:
    nxrt::platform::File* f_;
    uint32_t mode_;
    std::mutex size_mu_;
};

// Applies a decided commit. Marker lines: "W <work folder>", "R <committed folder>", then "D rel" (delete), "M rel" (make folder),
// "F rel" (move file work -> committed). Every step can be repeated, so a commit interrupted here is finished by running it again.
// Returns the number of steps, -1 when the marker cannot be read. `stop_after` (> 0) stops after that many steps (crash simulation).
int apply_marker(const stdfs::path& marker, int stop_after = 0) {
    std::ifstream in(marker, std::ios::binary);
    if (!in) return -1;
    std::string line;
    stdfs::path work, root;
    int n = 0;
    std::error_code ec;
    while (std::getline(in, line)) {
        while (!line.empty() && (line.back() == '\r' || line.back() == '\n')) line.pop_back();
        if (line.size() < 3) continue;
        const std::string arg = line.substr(2);
        if (line[0] == 'W') work = u8(arg);
        else if (line[0] == 'R') root = u8(arg);
        else if (work.empty() || root.empty()) continue;
        else if (line[0] == 'D') {
            std::error_code step_ec;
            stdfs::remove_all(root / u8(arg), step_ec);
            if (step_ec) {
                NXLOG_E("fs", "commit: deleting %s failed: %s", arg.c_str(), step_ec.message().c_str());
                return -1;
            }
            n++;
        } else if (line[0] == 'M') {
            std::error_code step_ec;
            stdfs::create_directories(root / u8(arg), step_ec);
            if (step_ec) {
                NXLOG_E("fs", "commit: creating folder %s failed: %s", arg.c_str(), step_ec.message().c_str());
                return -1;
            }
            n++;
        } else if (line[0] == 'F') {
            const stdfs::path a = work / u8(arg), b = root / u8(arg);
            if (stdfs::exists(a, ec)) {
                stdfs::create_directories(b.parent_path(), ec);
                stdfs::rename(a, b, ec);
                if (ec) {
                    NXLOG_E("fs", "commit: moving %s failed: %s", arg.c_str(), ec.message().c_str());
                    return -1;
                }
            }
            n++;
        } else {
            continue;
        }
        if (stop_after > 0 && n >= stop_after) return n;
    }
    in.close();
    stdfs::remove(marker, ec);
    return n;
}

}  // namespace

std::string save_folder_lock_name(const std::string& root) {
    char name[64];
    std::snprintf(name, sizeof name, "nxrt_save_%016llx", static_cast<unsigned long long>(std::hash<std::string>{}(root)));
    return name;
}

SaveFolderLock::SaveFolderLock(const std::string& root) {
    const std::string name = save_folder_lock_name(root);
    handle_ = nxrt::platform::named_mutex_create(name.c_str(), nullptr);
    if (handle_ && !nxrt::platform::named_mutex_lock(handle_)) {
        nxrt::platform::named_mutex_destroy(handle_);
        handle_ = nullptr;
    }
    if (!handle_) NXLOG_W("fs", "could not lock the save folder %s: committing without the cross-process lock", root.c_str());
}

SaveFolderLock::~SaveFolderLock() {
    if (!handle_) return;
    nxrt::platform::named_mutex_unlock(handle_);
    nxrt::platform::named_mutex_destroy(handle_);
}

// ---- path hygiene ----------------------------------------------------------------------------------------------------------------

bool normalize_path(const char* raw, size_t max_len, std::string* out, uint32_t* err) {
    out->clear();
    if (!raw) {
        *err = rc::kInvalidArgument;
        return false;
    }
    size_t n = 0;
    while (n < max_len && raw[n]) n++;
    if (n == max_len) {   // no terminator inside the buffer
        *err = rc::kTooLongPath;
        return false;
    }
    if (n > 0x300) {
        *err = rc::kTooLongPath;
        return false;
    }
    if (n == 0 || raw[0] != '/') {
        *err = rc::kInvalidArgument;
        return false;
    }
    std::string cur;
    std::vector<std::string> parts;
    for (size_t i = 0; i <= n; ++i) {
        const char c = i < n ? raw[i] : '/';
        if (c == '/') {
            if (cur == "." || cur == "..") {
                *err = rc::kInvalidArgument;
                return false;
            }
            if (!cur.empty()) parts.push_back(cur);
            cur.clear();
        } else if (c == '\\' || static_cast<unsigned char>(c) < 0x20 || c == ':' || c == '*' || c == '?' || c == '"' || c == '<' || c == '>' || c == '|') {
            *err = rc::kInvalidArgument;
            return false;
        } else {
            cur += c;
        }
    }
    for (const std::string& p : parts) {
        *out += "/";
        *out += p;
    }
    if (out->empty()) *out = "/";
    return true;
}

// ---- Volume ----------------------------------------------------------------------------------------------------------------------

struct Volume::Impl {
    stdfs::path root, work, marker, journal_file;
    std::mutex mu;
    std::set<std::string> tomb;   // committed paths deleted since the last commit (journalled volumes)
    bool journal = false, ro = false, ok = false;

    bool direct() const { return !journal; }
    stdfs::path cpath(const std::string& path) const { return root / u8(rel_of(path)); }
    stdfs::path wpath(const std::string& path) const { return (journal ? work : root) / u8(rel_of(path)); }

    // The committed layer shows `path` unless it or one of its parents was deleted since the last commit.
    bool committed_visible(const std::string& path) const {
        if (!journal) return false;   // direct volumes have one layer (wpath == cpath)
        for (size_t i = 1; i <= path.size(); ++i)
            if (i == path.size() || path[i] == '/')
                if (tomb.count(path.substr(0, i))) return false;
        std::error_code ec;
        return stdfs::exists(cpath(path), ec);
    }

    struct Found {
        bool exists = false, is_dir = false, in_work = false;
        stdfs::path host;
    };
    Found lookup(const std::string& path) const {
        Found f;
        std::error_code ec;
        if (path == "/") {
            f.exists = f.is_dir = f.in_work = true;
            f.host = wpath(path);
            return f;
        }
        const stdfs::path w = wpath(path);
        if (stdfs::exists(w, ec)) {
            f.exists = f.in_work = true;
            f.is_dir = stdfs::is_directory(w, ec);
            f.host = w;
            return f;
        }
        if (committed_visible(path)) {
            const stdfs::path c = cpath(path);
            f.exists = true;
            f.is_dir = stdfs::is_directory(c, ec);
            f.host = c;
        }
        return f;
    }

    // The parents of `path` exist as directories (in some layer); creates them in the work layer.
    uint32_t ensure_parents(const std::string& path) {
        const std::string par = parent_of(path);
        const Found f = lookup(par);
        if (!f.exists || !f.is_dir) return rc::kPathNotFound;
        std::error_code ec;
        stdfs::create_directories(wpath(par), ec);
        return ec ? rc::kGenericFailure : 0;
    }

    // Makes sure a committed file also exists in the work layer (copy on write).
    uint32_t materialize_file(const std::string& path) {
        std::error_code ec;
        if (!journal || stdfs::exists(wpath(path), ec)) return 0;
        if (!committed_visible(path)) return rc::kPathNotFound;
        if (const uint32_t r = ensure_parents(path)) return r;
        stdfs::copy_file(cpath(path), wpath(path), stdfs::copy_options::overwrite_existing, ec);
        return ec ? rc::kGenericFailure : 0;
    }

    // Copies the visible committed subtree under `path` into the work layer (before a directory rename).
    uint32_t materialize_tree(const std::string& path) {
        std::vector<DirItem> items;
        if (const uint32_t r = list(path, &items)) return r;
        std::error_code ec;
        stdfs::create_directories(wpath(path), ec);
        for (const DirItem& it : items) {
            const std::string child = (path == "/" ? "" : path) + "/" + it.name;
            if (it.is_dir) {
                if (const uint32_t r = materialize_tree(child)) return r;
            } else if (const uint32_t r = materialize_file(child)) {
                return r;
            }
        }
        return 0;
    }

    uint32_t list(const std::string& path, std::vector<DirItem>* out) {
        const Found f = lookup(path);
        if (!f.exists || !f.is_dir) return rc::kPathNotFound;
        std::map<std::string, DirItem> m;
        std::error_code ec;
        auto scan = [&](const stdfs::path& dir, bool committed) {
            for (stdfs::directory_iterator it(dir, ec), end; !ec && it != end; it.increment(ec)) {
                DirItem d;
                d.name = to_u8(it->path().filename());
                const std::string child = (path == "/" ? "" : path) + "/" + d.name;
                if (committed && tomb.count(child)) continue;
                std::error_code e2;
                d.is_dir = it->is_directory(e2);
                d.size = d.is_dir ? 0 : static_cast<int64_t>(it->file_size(e2));
                if (committed && m.count(d.name)) continue;
                m[d.name] = d;
            }
        };
        if (stdfs::exists(wpath(path), ec)) scan(wpath(path), false);
        if (journal && committed_visible(path)) scan(cpath(path), true);
        out->clear();
        for (auto& kv : m) out->push_back(kv.second);
        std::sort(out->begin(), out->end(), [](const DirItem& a, const DirItem& b) { return a.name < b.name; });
        return 0;
    }

    uint64_t used(const std::string& path) {
        std::vector<DirItem> items;
        if (list(path, &items)) return 0;
        uint64_t total = 0;
        for (const DirItem& it : items) {
            const std::string child = (path == "/" ? "" : path) + "/" + it.name;
            total += it.is_dir ? used(child) : static_cast<uint64_t>(it.size);
        }
        return total;
    }
};

Volume::Volume(const std::string& root, bool journal, bool read_only, uint64_t capacity)
    : p_(std::make_unique<Impl>()), capacity_(capacity), journal_(journal), read_only_(read_only) {
    Impl& s = *p_;
    s.root = u8(root);
    s.journal = journal;
    s.ro = read_only;
    std::error_code ec;
    if (journal) {
        stdfs::create_directories(s.root, ec);
        recover(root);
        s.marker = stdfs::path(s.root.string() + ".commit");
        s.work = stdfs::path(s.root.string() + ".work." + std::to_string(nxrt::platform::process_id()));
        s.journal_file = stdfs::path(s.root.string() + ".journal." + std::to_string(nxrt::platform::process_id()));
        stdfs::remove_all(s.work, ec);   // a second open in this process starts from the committed state
        stdfs::create_directories(s.work, ec);
        s.ok = stdfs::is_directory(s.root, ec) && stdfs::is_directory(s.work, ec);
    } else {
        s.ok = stdfs::is_directory(s.root, ec);
        if (!s.ok && !read_only) {
            stdfs::create_directories(s.root, ec);
            s.ok = stdfs::is_directory(s.root, ec);
        }
    }
}

Volume::~Volume() {
    if (!p_ || !p_->journal) return;
    std::error_code ec;
    // A decided commit that was not finished keeps its work folder: the next open rolls it forward. Otherwise the uncommitted
    // changes are dropped, as they are when a save is unmounted on the console without Commit.
    if (!stdfs::exists(p_->marker, ec)) stdfs::remove_all(p_->work, ec);
}

bool Volume::ok() const { return p_->ok; }

void Volume::set_crash_point_for_tests(int point) { g_crash_point = point; }

void Volume::recover(const std::string& root_s) {
    const stdfs::path root = u8(root_s);
    SaveFolderLock lock(root_s);
    std::error_code ec;
    const stdfs::path marker(root.string() + ".commit");
    if (stdfs::exists(marker, ec)) {
        const int n = apply_marker(marker);
        if (n < 0) {
            NXLOG_E("fs", "a commit of %s was interrupted after it was decided and could not be applied; it stays decided for the next attempt", root_s.c_str());
        } else {
            NXLOG_W("fs", "a commit of %s was interrupted after it was decided: finished now (%d changes)", root_s.c_str(), n);
        }
    }
    // Work folders and journals of processes that no longer run.
    const stdfs::path dir = root.parent_path();
    const std::string stem = root.filename().string();
    for (stdfs::directory_iterator it(dir, ec), end; !ec && it != end; it.increment(ec)) {
        const std::string name = it->path().filename().string();
        if (name.compare(0, stem.size(), stem) != 0) continue;
        const std::string tail = name.substr(stem.size());
        const bool is_work = tail.rfind(".work.", 0) == 0, is_journal = tail.rfind(".journal.", 0) == 0;
        if (!is_work && !is_journal) continue;
        const uint32_t pid = static_cast<uint32_t>(std::strtoul(tail.c_str() + (is_work ? 6 : 9), nullptr, 10));
        if (!pid || pid == nxrt::platform::process_id() || nxrt::platform::process_alive(pid)) continue;
        std::error_code e2;
        stdfs::remove_all(it->path(), e2);
        NXLOG_I("fs", "uncommitted %s of process %lu discarded (%s)", is_work ? "work folder" : "journal", static_cast<unsigned long>(pid), name.c_str());
    }
}

uint32_t Volume::create_file(const std::string& path, int64_t size) {
    if (read_only_) return rc::kReadOnlyFileSystem;
    if (size < 0) return rc::kOutOfRange;
    std::lock_guard<std::mutex> lk(p_->mu);
    if (path == "/") return rc::kPathAlreadyExists;
    if (p_->lookup(path).exists) return rc::kPathAlreadyExists;
    if (const uint32_t r = p_->ensure_parents(path)) return r;
    nxrt::platform::File* f = nxrt::platform::file_create_new(to_u8(p_->wpath(path)).c_str());
    if (!f) return rc::kGenericFailure;
    uint32_t r = 0;
    if (size && !nxrt::platform::file_truncate(f, size)) r = rc::kGenericFailure;
    nxrt::platform::file_close(f);
    p_->tomb.erase(path);   // harmless when absent; a re-created path is served from the work layer
    return r;
}

uint32_t Volume::delete_file(const std::string& path) {
    if (read_only_) return rc::kReadOnlyFileSystem;
    std::lock_guard<std::mutex> lk(p_->mu);
    const Impl::Found f = p_->lookup(path);
    if (!f.exists || f.is_dir) return rc::kPathNotFound;
    std::error_code ec;
    stdfs::remove(p_->wpath(path), ec);
    if (p_->committed_visible(path)) p_->tomb.insert(path);
    return 0;
}

uint32_t Volume::create_dir(const std::string& path) {
    if (read_only_) return rc::kReadOnlyFileSystem;
    std::lock_guard<std::mutex> lk(p_->mu);
    if (path == "/" || p_->lookup(path).exists) return rc::kPathAlreadyExists;
    if (const uint32_t r = p_->ensure_parents(path)) return r;
    std::error_code ec;
    stdfs::create_directory(p_->wpath(path), ec);
    return ec ? rc::kGenericFailure : 0;
}

uint32_t Volume::delete_dir(const std::string& path) {
    if (read_only_) return rc::kReadOnlyFileSystem;
    std::lock_guard<std::mutex> lk(p_->mu);
    const Impl::Found f = p_->lookup(path);
    if (path == "/" || !f.exists || !f.is_dir) return rc::kPathNotFound;
    std::vector<DirItem> items;
    p_->list(path, &items);
    if (!items.empty()) return rc::kDirectoryNotEmpty;
    std::error_code ec;
    stdfs::remove(p_->wpath(path), ec);
    if (p_->committed_visible(path)) p_->tomb.insert(path);
    return 0;
}

uint32_t Volume::delete_dir_recursive(const std::string& path) {
    if (read_only_) return rc::kReadOnlyFileSystem;
    std::lock_guard<std::mutex> lk(p_->mu);
    const Impl::Found f = p_->lookup(path);
    if (path == "/" || !f.exists || !f.is_dir) return rc::kPathNotFound;
    std::error_code ec;
    stdfs::remove_all(p_->wpath(path), ec);
    if (p_->committed_visible(path)) p_->tomb.insert(path);
    // work-layer tombstones below a re-created directory no longer apply
    for (auto it = p_->tomb.begin(); it != p_->tomb.end();)
        it = (it->size() > path.size() && it->compare(0, path.size(), path) == 0 && (*it)[path.size()] == '/') ? p_->tomb.erase(it) : std::next(it);
    return 0;
}

uint32_t Volume::clean_dir_recursive(const std::string& path) {
    if (read_only_) return rc::kReadOnlyFileSystem;
    std::vector<DirItem> items;
    {
        std::lock_guard<std::mutex> lk(p_->mu);
        if (const uint32_t r = p_->list(path, &items)) return r;
    }
    for (const DirItem& it : items) {
        const std::string child = (path == "/" ? "" : path) + "/" + it.name;
        const uint32_t r = it.is_dir ? delete_dir_recursive(child) : delete_file(child);
        if (r) return r;
    }
    return 0;
}

uint32_t Volume::rename_file(const std::string& from, const std::string& to) {
    if (read_only_) return rc::kReadOnlyFileSystem;
    std::lock_guard<std::mutex> lk(p_->mu);
    const Impl::Found a = p_->lookup(from);
    if (!a.exists || a.is_dir) return rc::kPathNotFound;
    if (from == to) return 0;
    if (p_->lookup(to).exists) return rc::kPathAlreadyExists;
    if (const uint32_t r = p_->ensure_parents(to)) return r;
    if (const uint32_t r = p_->materialize_file(from)) return r;
    std::error_code ec;
    stdfs::rename(p_->wpath(from), p_->wpath(to), ec);
    if (ec) return rc::kGenericFailure;
    if (p_->committed_visible(from)) p_->tomb.insert(from);
    return 0;
}

uint32_t Volume::rename_dir(const std::string& from, const std::string& to) {
    if (read_only_) return rc::kReadOnlyFileSystem;
    std::lock_guard<std::mutex> lk(p_->mu);
    const Impl::Found a = p_->lookup(from);
    if (from == "/" || !a.exists || !a.is_dir) return rc::kPathNotFound;
    if (from == to) return 0;
    if (to.size() > from.size() && to.compare(0, from.size(), from) == 0 && to[from.size()] == '/') return rc::kInvalidArgument;   // into itself
    if (p_->lookup(to).exists) return rc::kPathAlreadyExists;
    if (const uint32_t r = p_->ensure_parents(to)) return r;
    if (const uint32_t r = p_->materialize_tree(from)) return r;
    std::error_code ec;
    stdfs::rename(p_->wpath(from), p_->wpath(to), ec);
    if (ec) return rc::kGenericFailure;
    if (p_->committed_visible(from)) p_->tomb.insert(from);
    return 0;
}

uint32_t Volume::entry_type(const std::string& path, uint32_t* type) {
    std::lock_guard<std::mutex> lk(p_->mu);
    const Impl::Found f = p_->lookup(path);
    if (!f.exists) return rc::kPathNotFound;
    *type = f.is_dir ? 0 : 1;
    return 0;
}

uint32_t Volume::open_file(const std::string& path, uint32_t mode, std::shared_ptr<VFile>* out) {
    std::lock_guard<std::mutex> lk(p_->mu);
    const Impl::Found f = p_->lookup(path);
    if (!f.exists || f.is_dir) return rc::kPathNotFound;
    const bool write = (mode & (kModeWrite | kModeAppend)) != 0;
    if (write && read_only_) return rc::kReadOnlyFileSystem;
    stdfs::path host = f.host;
    if (write) {
        if (const uint32_t r = p_->materialize_file(path)) return r;
        host = p_->wpath(path);
    }
    nxrt::platform::File* opened = write ? nxrt::platform::file_open_update(to_u8(host).c_str()) : nxrt::platform::file_open(to_u8(host).c_str(), false);
    if (!opened) return rc::kPathNotFound;
    *out = std::make_shared<HostFile>(opened, mode);
    return 0;
}

uint32_t Volume::list_dir(const std::string& path, std::vector<DirItem>* out) {
    std::lock_guard<std::mutex> lk(p_->mu);
    return p_->list(path, out);
}

uint32_t Volume::file_times(const std::string& path, FileTimes* out) {
    std::lock_guard<std::mutex> lk(p_->mu);
    const Impl::Found f = p_->lookup(path);
    if (!f.exists) return rc::kPathNotFound;
    nxrt::platform::FileTimes t;
    if (!nxrt::platform::file_times(to_u8(f.host).c_str(), &t)) return rc::kGenericFailure;
    out->created = t.created > 0 ? t.created : t.modified;   // POSIX keeps no creation time: the write time stands in
    out->modified = t.modified;
    out->accessed = t.accessed;
    out->valid = true;
    return 0;
}

uint64_t Volume::used_bytes() {
    std::lock_guard<std::mutex> lk(p_->mu);
    return p_->used("/");
}

uint32_t Volume::commit() {
    if (!p_->journal) return 0;
    if (read_only_) return 0;
    Impl& s = *p_;
    std::lock_guard<std::mutex> lk(s.mu);
    SaveFolderLock lock(to_u8(s.root));
    std::error_code ec;
    if (s.tomb.empty() && !stdfs::exists(s.work, ec)) return 0;

    // Phase 1: flush the changed files, write the change list, make it the decided marker with one rename.
    std::string body = "W " + to_u8(s.work) + "\nR " + to_u8(s.root) + "\n";
    for (const std::string& t : s.tomb) body += "D " + rel_of(t) + "\n";
    int changes = static_cast<int>(s.tomb.size());
    std::vector<std::string> files, dirs;
    for (stdfs::recursive_directory_iterator it(s.work, ec), end; !ec && it != end; it.increment(ec)) {
        const std::string rel = to_u8(stdfs::relative(it->path(), s.work, ec));
        std::string g = rel;
        std::replace(g.begin(), g.end(), '\\', '/');
        if (it->is_directory(ec)) {
            dirs.push_back(g);
        } else {
            files.push_back(g);
            nxrt::platform::file_sync(to_u8(it->path()).c_str());
        }
    }
    for (const std::string& d : dirs) body += "M " + d + "\n";
    for (const std::string& f : files) body += "F " + f + "\n";
    changes += static_cast<int>(dirs.size() + files.size());
    {
        std::ofstream j(s.journal_file, std::ios::binary | std::ios::trunc);
        j << body;
        j.flush();
        if (!j) {
            NXLOG_E("fs", "save commit %s: writing the journal failed", to_u8(s.root).c_str());
            return rc::kGenericFailure;
        }
    }
    nxrt::platform::file_sync(to_u8(s.journal_file).c_str());
    if (!nxrt::platform::file_rename(to_u8(s.journal_file).c_str(), to_u8(s.marker).c_str(), true)) {
        NXLOG_E("fs", "save commit %s: making the decided marker failed", to_u8(s.root).c_str());
        return rc::kGenericFailure;
    }
    if (g_crash_point == 1) return rc::kGenericFailure;   // decided, nothing applied

    // Phase 2: apply. A crash in here is repaired by the next recover().
    const int applied = apply_marker(s.marker, g_crash_point == 2 ? 1 : 0);
    if (g_crash_point == 2) return rc::kGenericFailure;
    if (applied < 0) {
        NXLOG_E("fs", "save commit %s: applying the decided marker failed", to_u8(s.root).c_str());
        return rc::kGenericFailure;
    }
    s.tomb.clear();
    stdfs::remove_all(s.work, ec);
    stdfs::create_directories(s.work, ec);
    // The names, not just the count: a save that rewrites Auto, Global and the route data is what a chapter or route
    // completion writes, and a report with those names can tell that apart from a commit of one file. Names only; no
    // save contents are logged.
    std::string names;
    for (const std::string& f : files) {
        if (names.size() + f.size() > 240) {
            names += ", ...";
            break;
        }
        if (!names.empty()) names += ", ";
        names += f;
    }
    NXLOG_I("fs", "save commit %s: %d changes applied%s%s", to_u8(s.root).c_str(), changes, names.empty() ? "" : ": ",
            names.c_str());
    return 0;
}

}  // namespace nxrt::services::fs
