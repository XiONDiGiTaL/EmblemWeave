// SPDX-License-Identifier: GPL-3.0-or-later
// Reader for .nup game packages (https://github.com/a55uka/nup, SPEC.md format 1): one file holding a title's ExeFS and RomFS trees.
// The game is served straight from it: RomFS reads and ExeFS module loads become positioned reads of the package file. Only stored
// (codec 0, "raw") files can be read at an offset; a zstd file is one frame for the whole file, so it is decompressed once and kept in
// the runtime's cache, and later reads are positioned reads of that.
#pragma once
#include <cstdint>
#include <string>
#include <vector>

namespace nxrt::nup {

struct File {
    std::string path;           // inside its subtree, '/'-separated, no leading slash ("data/a.bin")
    uint64_t size = 0;          // of the contents, after decompression
    uint64_t offset = 0;        // absolute offset of the stored bytes in the package file
    uint64_t stored = 0;        // bytes in the package: `size` for a stored file, the frame's length for a zstd one
    bool compressed = false;    // codec 1: one zstd frame covering the whole file
};

struct Content {
    uint32_t version = 0;       // of the package used (0 = base game)
    std::vector<File> exefs;    // empty when the package has no ExeFS
    std::vector<File> romfs;    // empty when the package has no RomFS
};

// True when `path` names a regular file whose first bytes are the .nup magic.
bool is_nup(const std::string& path);

// Reads the table of contents and lists the ExeFS and RomFS of the package to run: the highest update, else the base game (add-ons
// are ignored). False with *err set when the file is not a valid package or holds a compressed file.
bool open(const std::string& path, Content& out, std::string* err = nullptr);

// Reads a whole file of the package, decompressing it when it is a zstd frame.
bool read_file(const std::string& path, const File& f, std::vector<uint8_t>& out, std::string* err = nullptr);

// Reads `len` stored bytes at `offset` (a frame, or a range of a stored file).
bool read_stored(const std::string& path, uint64_t offset, uint64_t len, std::vector<uint8_t>& out, std::string* err = nullptr);

}  // namespace nxrt::nup
