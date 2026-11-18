// SPDX-License-Identifier: GPL-3.0-or-later
// A RomFS image synthesised from a host folder: the byte-addressed storage nn::fs::MountRom reads (fsp-srv OpenDataStorageByCurrentProcess
// returns it as an IStorage; the SDK parses the RomFS tables itself). Nothing is copied: the metadata tables are built in memory and
// file data is read from the host files on demand.
//
// Image layout (https://switchbrew.org/wiki/RomFS, libnx romfs_dev / yuzu fsmitm_romfsbuild): the 0x50-byte header at offset 0 (padded
// to 0x200), the file data partition from 0x200 (each file starts at a multiple of 0x10 from the partition start, in the order of the
// file table), then the directory hash table, directory table, file hash table and file table, each 4-byte aligned; the header holds
// the offsets and sizes. Entries are sorted by name (byte order) within a directory.
#pragma once
#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <utility>
#include <vector>

namespace nxrt::services::fs {

// One folder laid over the RomFS. `name` is the installed mod's name for a RomFS mod; it is empty for the launcher's package overlay.
// The distinction is used in the startup log and to warn when a later layer shadows a mod's files.
struct RomfsOverlay {
    std::string path;
    std::string name;
    bool installed_mod = false;
    RomfsOverlay(std::string path, std::string name, bool installed_mod)
        : path(std::move(path)), name(std::move(name)), installed_mod(installed_mod) {}
};

// Edits of files while the image is laid out (code mods, nxrt/mod_api.h edit_file). A file whose path `wants` accepts is read whole and
// handed to `edit`; when that returns true the image serves the changed bytes from memory, with their new size. Paths are image
// paths ("/GameData/Job.bin"). Files over max_size are left alone.
struct RomfsEdits {
    std::function<bool(const std::string& path)> wants;
    std::function<bool(const std::string& path, std::vector<uint8_t>& data)> edit;
    uint64_t max_size = 256ull << 20;
};

class RomfsImage {
public:
    RomfsImage();
    ~RomfsImage();
    // Scans `host_dir` (recursively; names are taken as UTF-8 from the host paths) and builds the tables. `host_dir` may also name a .nup
    // package (nxrt/nup.h): its RomFS is served from the package file. False with *err set when the
    // folder does not exist or an entry cannot be described. Replaces an earlier build.
    // `overlays` are host folders laid over the tree in order (RomFS mods, LayeredFS style; a later one wins): a file at the same path
    // replaces the game's, a new path is added. Names match the game's ignoring ASCII case and keep the game's spelling. An overlay that
    // cannot be read is skipped with a warning; it never fails the build.
    // `edits` (optional) is applied to the merged tree, so an edit sees a mod's replacement of a file, not the game's original.
    bool build(const std::string& host_dir, std::string* err = nullptr, const std::vector<std::string>& overlays = {},
               const RomfsEdits* edits = nullptr);
    // The same, with the origin of every layer for the log: installed mods are named, the package overlay is not.
    bool build(const std::string& host_dir, std::string* err, const std::vector<RomfsOverlay>& overlays,
               const RomfsEdits* edits = nullptr);
    bool built() const;
    uint64_t size() const;                                  // total image size in bytes
    // Copies [offset, offset + len) of the image into dst. False when the range is outside the image or a host file cannot be read
    // (a file that shrank since build() reads as an error, not as zeros). Thread safe; any number of readers may call it at once.
    bool read(uint64_t offset, void* dst, uint64_t len) const;
    struct Stats {
        uint32_t files = 0, dirs = 0;                       // dirs counts the root
        uint64_t data_bytes = 0;                            // sum of file sizes
        uint64_t data_offset = 0;                           // header.file_partition_ofs
        uint32_t overlay_files = 0;                         // files served from overlays (replaced or added)
        uint32_t installed_mod_files = 0;                   // of those, files taken from installed mods
        uint32_t package_overlay_files = 0;                 // of those, files taken from the launcher's package overlay
        uint32_t shadowed_mod_files = 0;                    // mod files a later overlay replaced
        uint32_t edited_files = 0;                          // files served from memory after an edit (RomfsEdits)
        uint64_t packed_cache_bytes = 0;                    // decompressed .nup entries retained now
        uint64_t packed_cache_peak = 0;                     // largest retained total since build()
        uint32_t packed_cache_files = 0;                    // entries retained now
        uint64_t packed_cache_evictions = 0;                // LRU entries dropped to make room
        uint64_t packed_cache_skipped = 0;                  // entries too large for the per-file cap
    };
    Stats stats() const;
    // Image offset and size of a file by its path inside the image ("/a/b.bin", case sensitive), for tests and logs. False when absent.
    bool file_extent(const std::string& path, uint64_t* offset, uint64_t* size) const;

private:
    struct Impl;
    std::unique_ptr<Impl> p_;
};

}  // namespace nxrt::services::fs
