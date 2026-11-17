// SPDX-License-Identifier: GPL-3.0-or-later
// Finds spirv-val without assuming a machine-specific Vulkan SDK location. The tests use it from the working directory, so a
// missing validator must be a skip with a message that says how to provide it, not a failure or a hardcoded SDK path.
#pragma once

#include <cstdlib>
#include <filesystem>
#include <initializer_list>
#include <string>
#include <vector>

namespace nxtest {

namespace spirv_val_detail {

namespace fs = std::filesystem;

inline bool executable_file(const fs::path& p) {
    std::error_code ec;
    if (!fs::is_regular_file(p, ec)) return false;
#ifndef _WIN32
    const fs::perms perms = fs::status(p, ec).permissions();
    if (ec) return false;
    return (perms & (fs::perms::owner_exec | fs::perms::group_exec | fs::perms::others_exec)) != fs::perms::none;
#else
    return true;
#endif
}

inline std::vector<fs::path> names() {
#ifdef _WIN32
    return {fs::path("spirv-val.exe"), fs::path("spirv-val")};
#else
    return {fs::path("spirv-val")};
#endif
}

inline std::string from_path(const std::string& name) {
    const char* raw = std::getenv("PATH");
    if (!raw || !*raw) return {};
#ifdef _WIN32
    constexpr char separator = ';';
#else
    constexpr char separator = ':';
#endif
    std::string path(raw);
    size_t begin = 0;
    while (begin <= path.size()) {
        const size_t end = path.find(separator, begin);
        std::string dir = path.substr(begin, end == std::string::npos ? std::string::npos : end - begin);
        if (dir.size() >= 2 && dir.front() == '"' && dir.back() == '"') dir = dir.substr(1, dir.size() - 2);
        if (dir.empty()) dir = ".";
        const fs::path candidate = fs::path(dir) / name;
        if (executable_file(candidate)) return candidate.string();
        if (end == std::string::npos) break;
        begin = end + 1;
    }
    return {};
}

}  // namespace spirv_val_detail

// SPIRV_VAL / NXRT_SPIRV_VAL may name the executable directly; VULKAN_SDK may name the SDK root; otherwise PATH is searched.
// An empty result means the test must skip.
inline std::string find_spirv_val() {
    namespace d = spirv_val_detail;
    for (const char* variable : {"SPIRV_VAL", "NXRT_SPIRV_VAL"}) {
        if (const char* value = std::getenv(variable); value && *value) {
            const std::filesystem::path direct(value);
            if (d::executable_file(direct)) return direct.string();
            for (const std::filesystem::path& name : d::names()) {
                if (direct.filename() == name) {
                    if (const std::string found = d::from_path(value); !found.empty()) return found;
                }
            }
        }
    }
    if (const char* sdk = std::getenv("VULKAN_SDK"); sdk && *sdk) {
        const std::filesystem::path root(sdk);
        for (const std::filesystem::path& sub : {std::filesystem::path("Bin") / "spirv-val.exe", std::filesystem::path("bin") / "spirv-val",
                                                 std::filesystem::path("Bin") / "spirv-val", std::filesystem::path("spirv-val.exe")}) {
            if (d::executable_file(root / sub)) return (root / sub).string();
        }
    }
    for (const std::filesystem::path& name : d::names()) {
        if (const std::string found = d::from_path(name.string()); !found.empty()) return found;
    }
    return {};
}

inline const char* spirv_val_missing() {
    return "spirv-val not found (set SPIRV_VAL or NXRT_SPIRV_VAL, set VULKAN_SDK, or add spirv-val to PATH)";
}

// Quotes a path for the shell command helpers. On Windows cmd.exe strips the first and last quote of a command that starts
// with one, so the caller wraps the whole line in another pair there.
inline std::string quote(const std::string& text) { return "\"" + text + "\""; }

}  // namespace nxtest
