/* SPDX-License-Identifier: GPL-3.0-or-later
 * Code mods as DLLs: the interface between the runtime and a mod (services/mods_dll.cpp loads them).
 *
 * A mod is a folder under [fs] mods in the layout emulators use. Its exefs/ (where an emulator mod keeps IPS patches, which are not
 * applied here because the game code is compiled ahead of time) may also hold DLLs: every *.dll there is loaded when play mode starts,
 * in the order of the mods (sorted by name, as for RomFS), and its export
 *
 *     extern "C" __declspec(dllexport) int nxmod_init(const NxModApi* api, const char* mod_dir);
 *
 * is called once at the game's first frame boundary, on the main thread, with mod_dir = the mod's folder (UTF-8). By then the game's
 * modules are mapped and relocated, so nxmod_init can check the game's code and data (e.g. a signature check of the build it targets).
 * Nonzero = the mod declines (wrong game build, ...): its hooks are removed and it gets no frame calls. So one mod folder can carry both an emulator's IPS and this runtime's DLL.
 *
 * Plain C, so a mod can be built with any compiler. The table only grows: a mod checks api->version >= the version it was written for, and
 * api->size before reading a field added later. Guest state is the lifted code's Ctx (recomp/include/nxrecomp/ctx.h).
 *
 * What a mod can do:
 *   - attach to the hook sites the game was lifted with (recomp/games/<game>/main.sites): inline, midhook and entry hooks, the same as
 *     the runtime's own features (nxrt/hooks.h). A site that is not in that list is never reached: a new site needs a change to
 *     main.sites (and a Set up run) like any other hook.
 *   - call any lifted guest function (call_guest) from a hook or the frame callback, on the thread and Ctx it was given.
 *   - read and write guest memory directly: guest addresses are host addresses (main is at main_base); is_mapped checks a range first.
 *   - run once per presented frame (on_frame), on the main thread with its Ctx.
 *   - (version 2) give the game a function of its own to call (make_function): an address that works wherever the game keeps a
 *     function pointer, such as a slot of a vtable copy or a callback. With make_writable, which lifts the write protection of
 *     the game's read-only data, a mod can add entries to the game's own menus without a hook site.
 *
 *   - (version 3) change the game's data files as the game's file system is set up (edit_file): the mod is handed a file's bytes,
 *     the game's or an installed RomFS mod's, and may replace them; several mods can edit the same file in turn, so two mods adding
 *     rows to one table no longer overwrite each other. Because the file system is set up before nxmod_init runs, a mod that edits
 *     files also exports
 *
 *         extern "C" __declspec(dllexport) int nxmod_early(const NxModApi* api, const char* mod_dir);
 *
 *     which is called as soon as the DLL is loaded. The game is not in memory yet: only log, edit_file and the version 3 helpers
 *     (mod folders, host files, zstd, memory) may be used there. Nonzero = the mod declines, as for nxmod_init.
 *
 * Loading can be turned off with [fs] dll_mods = false. A DLL runs with the same rights as the game: install only mods you trust.
 */
#ifndef NXRT_MOD_API_H
#define NXRT_MOD_API_H

#include <stdint.h>

#include <nxrecomp/ctx.h>

#ifdef __cplusplus
extern "C" {
#endif

#define NXMOD_API_VERSION 3u

typedef void (*NxModInlineFn)(Ctx* c, uint64_t addr, void* user);
typedef int (*NxModSiteFn)(Ctx* c, uint64_t addr, void* user);   /* midhook / entry hook: nonzero = handled (nxrt/hooks.h) */
typedef void (*NxModFrameFn)(Ctx* c, void* user);
typedef void (*NxModFunctionFn)(Ctx* c, void* user);   /* a function the game calls: arguments in c->x[0..7], the result goes in c->x[0] */


/* A game data file being set up (version 3, edit_file). data/size is the content as it stands: the game's file, or the file an
 * installed RomFS mod put in its place, after the editors registered earlier. replace() hands back new content (copied before it
 * returns; calling it again replaces the earlier call's). Not calling it leaves the file as it is. */
typedef struct NxModFile {
    const char* path;            /* as registered: "GameData/Job.bin" */
    const uint8_t* data;
    uint64_t size;
    void (*replace)(const struct NxModFile* file, const void* data, uint64_t size);
} NxModFile;
typedef void (*NxModFileFn)(const NxModFile* file, void* user);

typedef struct NxModApi {
    uint32_t version;            /* NXMOD_API_VERSION of the runtime */
    uint32_t size;               /* sizeof(NxModApi) of the runtime */
    const char* game_id;         /* the game module's id, e.g. "fefw" */
    uint64_t main_base;          /* guest address of main's image */

    /* a log line under the tag "mod", naming the mod: pass the api pointer the mod was given (each mod gets its own table).
     * level 0 error, 1 warning, 2 info, 3 debug */
    void (*log)(const struct NxModApi* api, int level, const char* text);

    /* hooks at lifted sites (absolute guest addresses); each returns an id, 0 on failure. All of a mod's hooks are removed if it declines. */
    uint32_t (*hook_inline)(uint64_t addr, NxModInlineFn fn, void* user);
    uint32_t (*hook_mid)(uint64_t addr, NxModSiteFn fn, void* user);
    uint32_t (*hook_entry)(uint64_t addr, NxModSiteFn fn, void* user);
    int (*hook_remove)(uint32_t id);

    /* Calls the lifted function at addr with x0..x7 = args[0..nargs-1] and returns x0. Every register of c is as it was afterwards (the
     * call runs below c's stack pointer). Only from a hook or on_frame, with the Ctx they were given. An address that is not a lifted
     * function (is_function) is not called: a warning is logged and 0 returned. The function must return normally: a guest C++
     * exception that escapes it is not caught here. */
    uint64_t (*call_guest)(Ctx* c, uint64_t addr, const uint64_t* args, int nargs);

    /* 1 when every byte of [addr, addr + size) is mapped */
    int (*is_mapped)(uint64_t addr, uint64_t size);

    /* fn(c, user) once per presented frame on the main thread (the game's frame boundary), in the order registered */
    void (*on_frame)(NxModFrameFn fn, void* user);

    /* 1 when addr is the entry of a lifted guest function (or a host import thunk), i.e. something call_guest can call */
    int (*is_function)(uint64_t addr);

    /* ---- version 2 (check api->version >= 2 before use) ---- */

    /* Makes fn callable by the game and returns its guest address, 0 on failure (the runtime has a fixed number of these). When the
     * game calls that address, fn(c, user) runs on the calling thread with the guest registers in c: the arguments are c->x[0..7],
     * the function answers by writing c->x[0] and returning. It may use call_guest. The address stays valid for the whole run; if the
     * mod declines in nxmod_init, calls to it do nothing and answer 0. */
    uint64_t (*make_function)(NxModFunctionFn fn, void* user);

    /* Lifts the write protection of the pages holding [addr, addr + size) of guest memory (the game's vtables and other constant
     * data are read-only). 1 on success, 0 when the range is not mapped or the protection cannot be changed. */
    int (*make_writable)(uint64_t addr, uint64_t size);

    /* ---- version 3 (check api->version >= 3 before use). All of these may be called from nxmod_early. ---- */

    /* fn(file, user) is called for the game data file at `path` when the game's file system is set up: once per start, before the
     * game reads anything, on the thread that sets it up. path is the file's path in the game's data ("GameData/Job.bin", '/'
     * separated, ASCII case ignored). A file that does not exist is not created, and files over 256 MiB are not offered. Editors run
     * in the order registered (mods in name order). Returns 1, or 0 when it is too late (the file system is already set up: call
     * this from nxmod_early) or an argument is missing. Compressed files (.zst) arrive compressed: see zstd_*. */
    int (*edit_file)(const char* path, NxModFileFn fn, void* user);

    /* The installed mods, in the order they are applied: mod_count() folders, mod_folder(i) = the folder of mod i (UTF-8, no trailing
     * separator; NULL past the end). For a mod that reads files other mods ship beside their romfs/ and exefs/. */
    uint32_t (*mod_count)(void);
    const char* (*mod_folder)(uint32_t index);

    /* Host files by UTF-8 path. read_file copies up to cap bytes into buf and returns the file's whole size (so a result over cap
     * means the buffer was too small; buf may be NULL with cap 0 to ask for the size), -1 when the file cannot be read.
     * write_file replaces the file (through a temporary file beside it) and returns 1, 0 on failure. */
    int64_t (*read_file)(const char* path, void* buf, uint64_t cap);
    int (*write_file)(const char* path, const void* data, uint64_t size);

    /* zstd, for the game's .zst files. zstd_decompress returns the decompressed size, or -1 (bad data, or dst too small); with dst
     * NULL it returns the size the frame declares (-1 when it declares none). zstd_compress returns the compressed size, or -1; with
     * dst NULL it returns the largest size the result can have. level 0 = the library's default. */
    int64_t (*zstd_decompress)(const void* src, uint64_t src_size, void* dst, uint64_t cap);
    int64_t (*zstd_compress)(const void* src, uint64_t src_size, void* dst, uint64_t cap, int level);

    /* Memory for mods built without a C runtime: alloc returns size zeroed bytes (NULL on failure), free_mem releases them. */
    void* (*alloc)(uint64_t size);
    void (*free_mem)(void* p);
} NxModApi;

typedef int (*NxModInitFn)(const NxModApi* api, const char* mod_dir);
typedef int (*NxModEarlyFn)(const NxModApi* api, const char* mod_dir);   /* optional export nxmod_early (version 3) */

#ifdef __cplusplus
}
#endif

#endif /* NXRT_MOD_API_H */
