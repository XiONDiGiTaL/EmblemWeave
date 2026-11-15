// SPDX-License-Identifier: GPL-3.0-or-later
// Reader for .nup game packages (https://github.com/a55uka/nup, SPEC.md format 1): one file holding a title's ExeFS and RomFS trees.
// The game is served straight from it: RomFS reads and ExeFS module loads become positioned reads of the package file, so nothing is
// unpacked. Only stored (codec 0, "raw") files can be served that way; a zstd file is one frame for the whole file, with no random
// access into it, so a package packed with compression is refused with a message saying how to repack it.
#pragma once
#include <cstdint>
#include <string>
#include <vector>

namespace nxrt::nup {

struct File {
    std::string path;           // inside its subtree, '/'-separated, no leading slash ("data/a.bin")
    uint64_t size = 0;
    uint64_t offset = 0;        // absolute offset of the data in the package file
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

// Reads a whole file of the package (an ExeFS module).
bool read_file(const std::string& path, const File& f, std::vector<uint8_t>& out, std::string* err = nullptr);

}  // namespace nxrt::nup
