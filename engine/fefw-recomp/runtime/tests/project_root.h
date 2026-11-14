// Where the optional local test data lives (work/shader_cache, work/nvn_refs, game/romfs, refs/ ...) when the checkout sits inside the
// author's project folder. NXRT_ROOT names that folder; without it the nearest parent of the working directory that holds both a
// runtime/ and a work/ folder is used. Outside such a folder the paths stay relative and the tests that need the data skip.
#pragma once

#include <cstdlib>
#include <filesystem>
#include <string>
#include <system_error>

namespace nxtest {

inline const std::filesystem::path& project_root() {
    static const std::filesystem::path root = [] {
        namespace fs = std::filesystem;
        if (const char* e = std::getenv("NXRT_ROOT"); e && *e) return fs::path(e);
        std::error_code ec;
        for (fs::path p = fs::current_path(ec); !p.empty(); p = p.parent_path()) {
            if (fs::is_directory(p / "runtime", ec) && fs::is_directory(p / "work", ec)) return p;
            if (p == p.parent_path()) break;
        }
        return fs::path();
    }();
    return root;
}

// `rel` below the project root, with '/' separators (a trailing '/' is kept).
inline std::string project_path(const std::string& rel) {
    return (project_root() / rel).generic_string();
}

}  // namespace nxtest
