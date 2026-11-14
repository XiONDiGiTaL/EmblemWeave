// SPDX-License-Identifier: GPL-3.0-or-later
// Reader for .nx archives (NXFormat, https://github.com/fab918/nxformat): one file holding a title's ExeFS and RomFS trees as
// independently compressed blocks. The launcher copies the ExeFS out of the archive; the RomFS is served from it directly, so a
// RomFS read becomes a block read of the archive file, decompressed and integrity checked once and then kept in a small cache.
#pragma once
#include <cstdint>
#include <memory>
#include <string>
#include <vector>

namespace nxrt::nx {

constexpr uint32_t kNoFile = 0xFFFFFFFFu;

struct File {
    std::string path;            // inside the RomFS, '/'-separated, no leading slash ("data/a.bin")
    uint64_t size = 0;           // logical bytes
    uint32_t first_block = 0;    // into the archive's block reference table
    uint32_t block_count = 0;
};

struct Content {
    uint64_t title = 0;          // application id the tree belongs to
    uint32_t version = 0;        // snapshot version used (0 for a base-only archive)
    uint32_t snapshot_kind = 0;  // 0 base, 1 update, 3 standalone update
    std::vector<File> romfs;     // empty when the archive holds no RomFS
};

// True when `path` names a regular file whose first bytes are the .nx magic.
bool is_nx(const std::string& path);

// An open archive. The four tables are read once; file data is read on demand.
class Reader {
  public:
    explicit Reader(const std::string& path);
    ~Reader();
    Reader(const Reader&) = delete;
    Reader& operator=(const Reader&) = delete;

    // False with error() set when the file is not a usable archive. The tables are checked against the digests in the header, so
    // a truncated or edited archive is reported instead of misread.
    bool ok() const { return ok_; }
    const std::string& error() const { return err_; }

    const Content& content() const { return content_; }

    // Reads `len` bytes of a RomFS file at `offset`. Blocks are verified against the per-slice CRCs in the archive as they are
    // decoded, so a damaged block is an error rather than wrong game data. Safe to call from several threads.
    bool read(const File& f, uint64_t offset, void* dst, uint64_t len);

    uint64_t archive_size() const;

  private:
    struct Impl;
    std::unique_ptr<Impl> p_;
    bool ok_ = false;
    std::string err_;
    Content content_;
};

}  // namespace nxrt::nx
