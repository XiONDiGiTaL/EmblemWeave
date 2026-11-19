// SPDX-License-Identifier: GPL-3.0-or-later
// Code mods as DLLs (nxrt/mod_api.h): every *.dll in the exefs/ folder of an installed mod (installed_mods(), common.cpp) is loaded when
// play mode starts; its nxmod_init is called with a table of runtime functions at the game's first frame boundary (main thread), when
// the game's modules are mapped and relocated. [fs] dll_mods = false turns loading off.
// A DLL may also export nxmod_early, called straight after it is loaded: the place to register file edits (edit_file), which the RomFS
// image applies when it is built (fs.cpp, on the game's first file access, long before the first frame).
#define _SILENCE_CXX20_U8PATH_DEPRECATION_WARNING
#include <algorithm>
#include <cstring>
#include <cwctype>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <mutex>
#include <set>
#include <string>
#include <utility>
#include <unordered_map>
#include <vector>

#include "nxrt/abi.h"
#include "nxrt/config.h"
#include "nxrt/dispatch.h"
#include "nxrt/game.h"
#include "nxrt/hooks.h"
#include "nxrt/log.h"
#include "nxrt/memory.h"
#include "nxrt/mod_api.h"
#include "nxrt/services_api.h"
#include "services_internal.h"

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

namespace nxrt::services::modapi {
namespace {

constexpr const char* kTag = "mods";

struct Mod {
    std::string name;            // mod folder below [fs] mods, then the DLL's file name
    NxModApi api{};              // the mod's own copy (log() finds the mod by it)
    HMODULE dll = nullptr;
    NxModInitFn init = nullptr;
    std::string dir;             // the mod's folder, UTF-8
    std::vector<uint32_t> hooks; // added during nxmod_init, removed if the mod declines
    bool declined = false;       // nxmod_early returned nonzero: nxmod_init is not called
};
struct FrameFn {
    NxModFrameFn fn;
    void* user;
    const Mod* owner;            // the mod whose nxmod_init registered it (removed if that mod declines), or null
};

// A function a mod gave the game (make_function): the thunk "mod.fn.<n>" runs mod_function, which finds this by the thunk's name.
struct ModFn {
    NxModFunctionFn fn;
    void* user;
    const Mod* owner;            // the mod whose nxmod_init made it (switched off if that mod declines), or null
};

// A file edit a mod registered (edit_file)
struct Editor {
    std::string key;             // image path, lower case ASCII: "/gamedata/job.bin"
    std::string path;            // as the mod wrote it (handed back in NxModFile::path)
    NxModFileFn fn;
    void* user;
    const Mod* owner;            // the mod whose nxmod_early / nxmod_init registered it, or null
    bool (*match)(const std::string& key) = nullptr;   // a runtime edit (add_runtime_edit): matches by pattern instead of key
    std::string owner_name;      // a runtime edit's owner, for the log
};

struct State {
    std::mutex mu;
    std::vector<Editor> editors;
    bool edits_closed = false;   // the RomFS image has asked: later edit_file calls are too late
    std::vector<std::string> folders;   // the installed mods' folders (mod_folder), filled on first use
    bool folders_known = false;
    ModCodec codec;
    std::unordered_map<std::string, ModFn> fns;   // by thunk name
    std::vector<Mod*> mods;      // never freed: hooks and frame callbacks may still point into the DLLs
    std::vector<FrameFn> frame;
    Mod* in_init = nullptr;      // the mod whose nxmod_init is running (main thread)
    uint32_t frame_hook = 0;
    uint64_t main_base = 0;
    std::vector<std::string> notices;   // shown on screen at start (notice())
    bool loaded = false;
    bool started = false;        // nxmod_init has run (first frame boundary)
};
State& st() {
    static State s;
    return s;
}

std::string mod_name(const NxModApi* api) {
    std::lock_guard<std::mutex> lk(st().mu);
    for (const Mod* m : st().mods)
        if (&m->api == api) return m->name;
    return "?";
}

void api_log(const NxModApi* api, int level, const char* text) {
    const std::string name = mod_name(api);
    const char* t = text ? text : "";
    switch (level) {
    case 0: NXLOG_E(kTag, "%s: %s", name.c_str(), t); break;
    case 1: NXLOG_W(kTag, "%s: %s", name.c_str(), t); break;
    case 2: NXLOG_I(kTag, "%s: %s", name.c_str(), t); break;
    default: NXLOG_D(kTag, "%s: %s", name.c_str(), t); break;
    }
}

uint32_t track(uint32_t id) {
    if (id && st().in_init) st().in_init->hooks.push_back(id);
    return id;
}

// ---- the sites main was lifted with ---------------------------------------------------------------------------------------------------
// Whoever builds fefw.exe writes the site list main was lifted with next to it, as <exe>.sites: Set up (fefw_build.py
// write_lifted_sites, with an @nsobid line) or the developer build (game-build/fefw/CMakeLists.txt, a copy of the list the libraries
// were lifted with). Without the file the list is unknown: hooks are not checked (a midhook only by the "is a lifted function" test).
struct SiteList {
    bool known = false;
    std::string bid;                                 // upper-case hex, "" when the file has no @nsobid line
    std::set<std::pair<std::string, uint64_t>> sites;   // (kind, guest address)
};

std::string upper(std::string t) {
    for (char& ch : t) ch = static_cast<char>(std::toupper(static_cast<unsigned char>(ch)));
    return t;
}

// Reads a site list: `<kind> <address>` lines, `#` comments, `@nsobid <id>`. Every kind is kept (patch lines too).
bool read_site_list(const std::filesystem::path& file, SiteList* out) {
    std::ifstream f(file);
    if (!f) return false;
    std::string line;
    while (std::getline(f, line)) {
        if (const size_t h = line.find('#'); h != std::string::npos) line.resize(h);
        char kind[32] = {}, arg[80] = {};
        if (std::sscanf(line.c_str(), "%31s %79s", kind, arg) != 2) continue;
        if (_stricmp(kind, "@nsobid") == 0) out->bid = upper(arg);
        else out->sites.insert({kind, std::strtoull(arg, nullptr, 16)});
    }
    out->known = true;
    return true;
}

const SiteList& lifted_sites() {
    static const SiteList list = [] {
        SiteList l;
        wchar_t exe[MAX_PATH * 2];
        const DWORD n = GetModuleFileNameW(nullptr, exe, static_cast<DWORD>(std::size(exe)));
        if (n == 0 || n >= std::size(exe)) return l;
        const std::filesystem::path file = std::filesystem::path(exe).replace_extension(L".sites");
        const std::u8string name = file.filename().u8string();
        const char* shown = reinterpret_cast<const char*>(name.c_str());
        if (read_site_list(file, &l)) NXLOG_I(kTag, "this build's hook sites: %zu (%s)", l.sites.size(), shown);
        else NXLOG_I(kTag, "no %s next to the exe: hooks of code mods are not checked against this build's sites", shown);
        return l;
    }();
    return list;
}

// A main.sites build ID against the list's: a prefix of it, the rest zeros (as Set up's bid_matches).
bool same_build(const std::string& mod_bid, const std::string& list_bid) {
    if (list_bid.empty()) return true;               // a developer build's list: assume the same game build
    std::string full = list_bid;
    full.resize(64, '0');
    const std::string t = upper(mod_bid);
    if (t.size() < 16 || t.size() > 64 || full.compare(0, t.size(), t) != 0) return false;
    return full.find_first_not_of('0', t.size()) == std::string::npos;
}

// Whether a hook of this kind at addr can run in this build. Addresses outside main are not checked (sites are main's only).
bool site_ok(const char* kind, uint64_t addr) {
    const uint64_t base = st().main_base;
    if (!addr || !base || addr < base || addr >= base + 0x400000000ull) return true;
    const SiteList& l = lifted_sites();
    if (!l.known) return std::strcmp(kind, "midhook") != 0 || fn_lookup(addr) != nullptr;
    return l.sites.count({kind, addr}) != 0;
}

void check_hook(const char* api_fn, const char* kind, uint64_t addr) {
    if (site_ok(kind, addr)) return;
    const Mod* m = st().in_init;
    NXLOG_W(kTag, "%s: %s(0x%llx): this build has no %s site there, the hook will never run (a mod lists its sites in "
                  "exefs/*.sites; Set up builds them in)", m ? m->name.c_str() : "a code mod", api_fn,
            static_cast<unsigned long long>(addr), kind);
}

uint32_t api_hook_inline(uint64_t addr, NxModInlineFn fn, void* user) {
    check_hook("hook_inline", "inline", addr);
    return track(hook_add_inline(addr, fn, user));
}
uint32_t api_hook_mid(uint64_t addr, NxModSiteFn fn, void* user) {
    check_hook("hook_mid", "midhook", addr);
    return track(hook_add_mid(addr, fn, user));
}
uint32_t api_hook_entry(uint64_t addr, NxModSiteFn fn, void* user) {
    check_hook("hook_entry", "entryhook", addr);
    return track(hook_add_entry(addr, fn, user));
}
int api_hook_remove(uint32_t id) { return hook_remove(id) ? 1 : 0; }

int api_is_function(uint64_t addr) { return addr && (fn_lookup(addr) || hle_is_thunk(addr)) ? 1 : 0; }

uint64_t api_call_guest(::Ctx* c, uint64_t addr, const uint64_t* args, int nargs) {
    if (!c || !addr) return 0;
    if (!api_is_function(addr)) {   // the runtime would trap (and end the game) on a call to an address that is not a lifted function
        NXLOG_W(kTag, "call_guest(0x%llx): not a lifted function, not called (returns 0)", static_cast<unsigned long long>(addr));
        return 0;
    }
    const ::Ctx saved = *c;
    for (int i = 0; i < 8; ++i) c->x[i] = (args && i < nargs) ? args[i] : 0;
    // below the caller's stack, 16-byte aligned (the stack the hook was given stays as it was)
    sp(c) = (sp(c) - 0x100) & ~uint64_t{0xF};
    call_indirect(c, addr);
    const uint64_t result = c->x[0];
    *c = saved;
    return result;
}

int api_is_mapped(uint64_t addr, uint64_t size) { return addr && mem::is_mapped(addr, size) ? 1 : 0; }

void api_on_frame(NxModFrameFn fn, void* user) {
    if (!fn) return;
    std::lock_guard<std::mutex> lk(st().mu);
    st().frame.push_back({fn, user, st().in_init});
}

// The handler of every make_function thunk: the runtime names the thunk it is running (hle_current_name), the name finds the mod's
// function. A function of a mod that declined, or an unknown name, answers 0.
void mod_function(::Ctx* c) {
    ModFn f{};
    if (const char* name = hle_current_name()) {
        std::lock_guard<std::mutex> lk(st().mu);
        if (auto it = st().fns.find(name); it != st().fns.end()) f = it->second;
    }
    if (f.fn) f.fn(c, f.user);
    else c->x[0] = 0;
}

uint64_t api_make_function(NxModFunctionFn fn, void* user) {
    if (!fn) return 0;
    std::string name;
    {
        std::lock_guard<std::mutex> lk(st().mu);
        name = "mod.fn." + std::to_string(st().fns.size());
        st().fns.emplace(name, ModFn{fn, user, st().in_init});
    }
    const uint64_t addr = hle_register(name.c_str(), &mod_function);
    if (!addr) {   // out of thunks (logged by the runtime)
        std::lock_guard<std::mutex> lk(st().mu);
        st().fns[name].fn = nullptr;
    }
    return addr;
}

int api_make_writable(uint64_t addr, uint64_t size) {
    if (!addr || !size || !mem::is_mapped(addr, size)) return 0;
    const uint64_t first = addr & ~uint64_t{mem::kPage - 1};
    const uint64_t end = (addr + size + mem::kPage - 1) & ~uint64_t{mem::kPage - 1};
    return mem::protect(first, end - first, mem::Prot::RW) ? 1 : 0;
}

// ---- version 3: file edits and helpers ---------------------------------------------------------------------------------------------
std::string edit_key(const char* path) {
    std::string k = path ? path : "";
    std::replace(k.begin(), k.end(), '\\', '/');
    while (!k.empty() && k.front() == '/') k.erase(k.begin());
    for (char& ch : k)
        if (ch >= 'A' && ch <= 'Z') ch = static_cast<char>(ch + 32);
    return k.empty() ? k : "/" + k;
}

int api_edit_file(const char* path, NxModFileFn fn, void* user) {
    const std::string key = edit_key(path);
    if (key.empty() || !fn) return 0;
    std::lock_guard<std::mutex> lk(st().mu);
    if (st().edits_closed) {
        NXLOG_W(kTag, "edit_file(%s): too late, the game's file system is already set up (register it from nxmod_early)", path);
        return 0;
    }
    st().editors.push_back({key, path, fn, user, st().in_init});
    return 1;
}

uint32_t api_mod_count() {
    std::lock_guard<std::mutex> lk(st().mu);
    if (!st().folders_known) {
        st().folders_known = true;
        namespace stdfs = std::filesystem;
        const stdfs::path root = stdfs::u8path(mods_dir());
        for (const RomfsMod& m : installed_mods()) {
            const std::u8string s = (root / stdfs::u8path(m.name)).lexically_normal().u8string();
            st().folders.emplace_back(reinterpret_cast<const char*>(s.data()), s.size());
        }
    }
    return static_cast<uint32_t>(st().folders.size());
}
const char* api_mod_folder(uint32_t index) {
    const uint32_t n = api_mod_count();
    std::lock_guard<std::mutex> lk(st().mu);
    return index < n ? st().folders[index].c_str() : nullptr;   // the strings are never changed once filled
}

int64_t api_read_file(const char* path, void* buf, uint64_t cap) {
    if (!path || !*path) return -1;
    std::error_code ec;
    const std::filesystem::path p = std::filesystem::u8path(path);
    if (!std::filesystem::is_regular_file(p, ec)) return -1;
    std::ifstream f(p, std::ios::binary | std::ios::ate);
    if (!f) return -1;
    const int64_t size = static_cast<int64_t>(f.tellg());
    if (size < 0) return -1;
    const uint64_t n = std::min<uint64_t>(static_cast<uint64_t>(size), buf ? cap : 0);
    if (n) {
        f.seekg(0);
        f.read(static_cast<char*>(buf), static_cast<std::streamsize>(n));
        if (static_cast<uint64_t>(f.gcount()) != n) return -1;
    }
    return size;
}

int api_write_file(const char* path, const void* data, uint64_t size) {
    if (!path || !*path || (size && !data)) return 0;
    const std::filesystem::path p = std::filesystem::u8path(path);
    std::filesystem::path tmp = p;
    tmp += L".tmp";
    {
        std::ofstream f(tmp, std::ios::binary | std::ios::trunc);
        if (!f) return 0;
        if (size) f.write(static_cast<const char*>(data), static_cast<std::streamsize>(size));
        f.flush();
        if (!f) return 0;
    }
    if (!MoveFileExW(tmp.c_str(), p.c_str(), MOVEFILE_REPLACE_EXISTING)) {
        std::error_code ec;
        std::filesystem::remove(tmp, ec);
        return 0;
    }
    return 1;
}

int64_t api_zstd_decompress(const void* src, uint64_t src_size, void* dst, uint64_t cap) {
    return st().codec.zstd_decompress && src ? st().codec.zstd_decompress(src, src_size, dst, cap) : -1;
}
int64_t api_zstd_compress(const void* src, uint64_t src_size, void* dst, uint64_t cap, int level) {
    return st().codec.zstd_compress && (src || !src_size) ? st().codec.zstd_compress(src, src_size, dst, cap, level) : -1;
}

void* api_alloc(uint64_t size) { return std::calloc(1, static_cast<size_t>(size ? size : 1)); }
void api_free(void* p) { std::free(p); }

// What NxModFile::replace writes to: the public part first, so the mod's pointer is this object's.
struct FileCall {
    NxModFile pub;
    std::vector<uint8_t> next;
    bool replaced = false;
};
void file_replace(const NxModFile* file, const void* data, uint64_t size) {
    if (!file || (size && !data)) return;
    FileCall* call = reinterpret_cast<FileCall*>(const_cast<NxModFile*>(file));
    const uint8_t* b = static_cast<const uint8_t*>(data);
    std::vector<uint8_t> copy(b, b + size);   // data may point into the file's current content or into an earlier replace
    call->next = std::move(copy);
    call->replaced = true;
}

// Takes back what a declining mod registered: its hooks, frame callbacks, functions and file edits.
void withdraw(Mod* m) {
    for (uint32_t id : m->hooks) hook_remove(id);
    m->hooks.clear();
    std::lock_guard<std::mutex> lk(st().mu);
    std::erase_if(st().frame, [m](const FrameFn& f) { return f.owner == m; });
    std::erase_if(st().editors, [m](const Editor& e) { return e.owner == m; });
    for (auto& [name, f] : st().fns)   // the thunks stay (the game may hold their addresses): they answer 0 from now on
        if (f.owner == m) f.fn = nullptr;
}

// nxmod_init of every loaded mod, in load order: at the first frame boundary, once the game's modules are mapped and relocated
void start_all() {
    std::vector<Mod*> mods;
    {
        std::lock_guard<std::mutex> lk(st().mu);
        mods = st().mods;
    }
    for (Mod* m : mods) {
        if (m->declined) continue;
        st().in_init = m;
        const int rc = m->init(&m->api, m->dir.c_str());
        st().in_init = nullptr;
        if (rc != 0) {
            withdraw(m);
            NXLOG_W(kTag, "%s: declined to start (nxmod_init returned %d); its hooks are removed", m->name.c_str(), rc);
            continue;   // the DLL stays loaded: it may still own memory the runtime saw
        }
        NXLOG_I(kTag, "%s: code mod started (%zu hooks)", m->name.c_str(), m->hooks.size());
    }
}

// the game's frame boundary (GamePlay::frame_update, also play mode's own hook): never "handled", the game's function runs as usual
int frame_hook(::Ctx* c, uint64_t, void*) {
    if (!st().started) {
        st().started = true;
        start_all();
    }
    std::vector<FrameFn> fns;
    {
        std::lock_guard<std::mutex> lk(st().mu);
        fns = st().frame;
    }
    for (const FrameFn& f : fns) f.fn(c, f.user);
    return 0;
}

std::string u8(const std::filesystem::path& p) {
    const std::u8string s = p.u8string();
    return std::string(reinterpret_cast<const char*>(s.data()), s.size());
}

void load_one(const std::string& mod, const std::filesystem::path& mod_dir, const std::filesystem::path& file, uint64_t base) {
    const std::string name = mod + "/" + u8(file.filename());
    // LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR needs a full path (a relative one fails with ERROR_INVALID_PARAMETER): [fs] mods may be relative
    std::error_code ec;
    const std::filesystem::path full = std::filesystem::absolute(file, ec);
    HMODULE dll = LoadLibraryExW((ec ? file : full).wstring().c_str(), nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (!dll) {
        NXLOG_E(kTag, "%s: cannot be loaded (Windows error %lu)", name.c_str(), static_cast<unsigned long>(GetLastError()));
        return;
    }
    const auto init = reinterpret_cast<NxModInitFn>(GetProcAddress(dll, "nxmod_init"));
    if (!init) {
        NXLOG_W(kTag, "%s: no nxmod_init export, not a code mod: unloaded", name.c_str());
        FreeLibrary(dll);
        return;
    }
    Mod* m = new Mod;
    m->name = name;
    m->dll = dll;
    m->api.version = NXMOD_API_VERSION;
    m->api.size = sizeof(NxModApi);
    m->api.game_id = game::info().id;
    m->api.main_base = base;
    m->api.log = &api_log;
    m->api.hook_inline = &api_hook_inline;
    m->api.hook_mid = &api_hook_mid;
    m->api.hook_entry = &api_hook_entry;
    m->api.hook_remove = &api_hook_remove;
    m->api.call_guest = &api_call_guest;
    m->api.is_mapped = &api_is_mapped;
    m->api.on_frame = &api_on_frame;
    m->api.is_function = &api_is_function;
    m->api.make_function = &api_make_function;
    m->api.make_writable = &api_make_writable;
    m->api.edit_file = &api_edit_file;
    m->api.mod_count = &api_mod_count;
    m->api.mod_folder = &api_mod_folder;
    m->api.read_file = &api_read_file;
    m->api.write_file = &api_write_file;
    m->api.zstd_decompress = &api_zstd_decompress;
    m->api.zstd_compress = &api_zstd_compress;
    m->api.alloc = &api_alloc;
    m->api.free_mem = &api_free;
    m->init = init;
    m->dir = u8(mod_dir);
    {
        std::lock_guard<std::mutex> lk(st().mu);
        st().mods.push_back(m);
    }
    NXLOG_I(kTag, "%s: loaded, starts at the first frame", name.c_str());
    // nxmod_early (optional): before the game's file system is set up, so the mod can register file edits
    if (const auto early = reinterpret_cast<NxModEarlyFn>(GetProcAddress(dll, "nxmod_early"))) {
        st().in_init = m;
        const int rc = early(&m->api, m->dir.c_str());
        st().in_init = nullptr;
        if (rc != 0) {
            m->declined = true;
            withdraw(m);
            NXLOG_W(kTag, "%s: declined at load (nxmod_early returned %d): not started", name.c_str(), rc);
        }
    }
}

}  // namespace

int sites_missing(const std::string& mod_dir) {
    namespace stdfs = std::filesystem;
    const SiteList& l = lifted_sites();
    if (!l.known || mod_dir.empty()) return -1;
    std::error_code ec;
    int missing = 0;
    for (const stdfs::directory_entry& e : stdfs::directory_iterator(stdfs::u8path(mod_dir) / "exefs", ec)) {
        if (!e.is_regular_file(ec) || _wcsicmp(e.path().extension().c_str(), L".sites") != 0) continue;
        SiteList want;
        if (!read_site_list(e.path(), &want) || want.bid.empty() || !same_build(want.bid, l.bid)) continue;
        for (const auto& s : want.sites) missing += l.sites.count(s) == 0;
    }
    return missing;
}

std::string notice() {
    std::lock_guard<std::mutex> lk(st().mu);
    std::string out;
    for (const std::string& n : st().notices) out += (out.empty() ? "" : "\n") + n;
    return out;
}

void set_codec(const ModCodec& c) { st().codec = c; }

bool file_edits_close() {
    std::lock_guard<std::mutex> lk(st().mu);
    st().edits_closed = true;
    return !st().editors.empty();
}

bool editor_matches(const Editor& e, const std::string& key) { return e.match ? e.match(key) : e.key == key; }

bool file_edit_wanted(const std::string& image_path) {
    const std::string key = edit_key(image_path.c_str());
    std::lock_guard<std::mutex> lk(st().mu);
    st().edits_closed = true;
    return std::any_of(st().editors.begin(), st().editors.end(), [&](const Editor& e) { return editor_matches(e, key); });
}

bool add_runtime_edit(const char* owner, bool (*match)(const std::string& key), void (*fn)(const NxModFile* file, void* user), void* user) {
    if (!match || !fn) return false;
    std::lock_guard<std::mutex> lk(st().mu);
    if (st().edits_closed) {
        NXLOG_W(kTag, "%s: file edit registered too late, the game's file system is already set up", owner ? owner : "?");
        return false;
    }
    Editor e;
    e.fn = fn;
    e.user = user;
    e.owner = nullptr;
    e.match = match;
    e.owner_name = owner ? owner : "runtime";
    // runtime edits first: a code mod's edit of the same file sees the result
    const auto it = std::find_if(st().editors.begin(), st().editors.end(), [](const Editor& x) { return !x.match; });
    st().editors.insert(it, std::move(e));
    return true;
}

bool file_edit_run(const std::string& image_path, std::vector<uint8_t>& data) {
    const std::string key = edit_key(image_path.c_str());
    std::vector<Editor> run;
    {
        std::lock_guard<std::mutex> lk(st().mu);
        st().edits_closed = true;
        for (const Editor& e : st().editors)
            if (editor_matches(e, key)) run.push_back(e);
    }
    bool changed = false;
    for (const Editor& e : run) {   // each editor sees what the one before it left
        FileCall call;
        call.pub.path = e.match ? image_path.c_str() : e.path.c_str();
        call.pub.data = data.data();
        call.pub.size = data.size();
        call.pub.replace = &file_replace;
        e.fn(&call.pub, e.user);
        if (!call.replaced) continue;
        NXLOG_I(kTag, "%s: %s edited (%zu -> %zu bytes)", e.owner ? e.owner->name.c_str() : !e.owner_name.empty() ? e.owner_name.c_str() : "?",
                image_path.c_str(), data.size(),
                call.next.size());
        data = std::move(call.next);
        changed = true;
    }
    return changed;
}

void load_dll_mods(uint64_t base, uint64_t frame_update) {
    if (st().loaded) return;
    st().loaded = true;
    st().main_base = base;
    // a mod whose hook sites this build was not lifted with: say so on screen as well as in the log, since its hooks would never run
    for (const RomfsMod& mod : installed_mods()) {
        if (mod.exefs.empty()) continue;
        const int missing = sites_missing(u8(std::filesystem::u8path(mod.exefs).parent_path()));
        if (missing <= 0) continue;
        NXLOG_W(kTag, "%s: %d of its hook sites (exefs/*.sites) are not in this build: its hooks will not run. Run Set up.",
                mod.name.c_str(), missing);
        std::lock_guard<std::mutex> lk(st().mu);
        st().notices.push_back(mod.name + ": run Set up (its hook sites are not in this build)");
    }
    if (!config_get_bool("fs", "dll_mods", true)) {
        NXLOG_I(kTag, "code mods (exefs/*.dll) are off ([fs] dll_mods = false)");
        return;
    }
    namespace stdfs = std::filesystem;
    for (const RomfsMod& mod : installed_mods()) {
        if (mod.exefs.empty()) continue;
        std::error_code ec;
        std::vector<stdfs::path> dlls;
        for (const stdfs::directory_entry& e : stdfs::directory_iterator(stdfs::u8path(mod.exefs), ec)) {
            if (!e.is_regular_file(ec)) continue;
            std::wstring ext = e.path().extension().wstring();
            std::transform(ext.begin(), ext.end(), ext.begin(), ::towlower);
            if (ext == L".dll") dlls.push_back(e.path());
        }
        std::sort(dlls.begin(), dlls.end());
        for (const stdfs::path& p : dlls) load_one(mod.name, stdfs::u8path(mod.exefs).parent_path(), p, base);
    }
    std::lock_guard<std::mutex> lk(st().mu);
    if (st().mods.empty()) return;
    if (frame_update) st().frame_hook = hook_add_entry(base + frame_update, &frame_hook);
    else NXLOG_W(kTag, "the game names no frame boundary: code mods are loaded but never started");
}

}  // namespace nxrt::services::modapi
