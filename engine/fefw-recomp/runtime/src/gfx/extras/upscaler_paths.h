// SPDX-License-Identifier: GPL-3.0-or-later
// Common places a player leaves the vendor upscaler DLLs. The runtime's
// defaults used to look only beside fefw.exe, beside the launcher and in the
// run folder. A DLL copied into the engine checkout, as in issue #26, sat in
// an ancestor of the build folder and was never found. This walks those
// ancestors too, including the familiar `upscalers` and `upscaler` spellings
// and the bin/x64 folders an SDK keeps when it is unpacked whole.
#pragma once

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>

#include <string>
#include <vector>

namespace nxrt::extras::upscaler_paths {

inline std::wstring slash(std::wstring path) {
    if (!path.empty() && path.back() != L'\\' && path.back() != L'/') path += L'\\';
    return path;
}

inline std::wstring join(std::wstring base, const wchar_t* tail) {
    if (tail && *tail) {
        if (!base.empty() && base.back() != L'\\' && base.back() != L'/') base += L'\\';
        base += tail;
    }
    return slash(base);
}

inline bool is_dir(const std::wstring& path) {
    const DWORD a = GetFileAttributesW(path.c_str());
    return a != INVALID_FILE_ATTRIBUTES && (a & FILE_ATTRIBUTE_DIRECTORY);
}

inline bool is_file(const std::wstring& path) {
    const DWORD a = GetFileAttributesW(path.c_str());
    return a != INVALID_FILE_ATTRIBUTES && !(a & FILE_ATTRIBUTE_DIRECTORY);
}

// The folder that contains a file or folder. A drive root stays "C:\"; other
// trailing separators are dropped so the result can be joined again.
inline std::wstring parent_dir(std::wstring path) {
    while (!path.empty() && (path.back() == L'\\' || path.back() == L'/')) path.pop_back();
    if (path.empty()) return {};
    const size_t cut = path.find_last_of(L"\\/");
    if (cut == std::wstring::npos) return {};
    if (cut == 2 && path[1] == L':') return path.substr(0, 3);
    return path.substr(0, cut);
}

inline std::wstring module_path() {
    std::wstring path(32768, L'\0');
    const DWORD n = GetModuleFileNameW(nullptr, path.data(), static_cast<DWORD>(path.size()));
    path.resize(n);
    return path;
}

inline std::wstring working_dir() {
    std::wstring path(32768, L'\0');
    const DWORD n = GetCurrentDirectoryW(static_cast<DWORD>(path.size()), path.data());
    path.resize(n);
    return path;
}

inline std::wstring launcher_dir() {
    std::wstring path(32768, L'\0');
    const DWORD n = GetEnvironmentVariableW(L"NXRT_LAUNCHER_DIR", path.data(), static_cast<DWORD>(path.size()));
    path.resize(n);
    return path;
}

inline void add(std::vector<std::wstring>& out, const std::wstring& path) {
    if (path.empty()) return;
    for (const std::wstring& other : out)
        if (lstrcmpiW(other.c_str(), path.c_str()) == 0) return;
    out.push_back(path);
}

// Every `<upscalers|upscaler>\<vendor>` folder from the executable's folder up
// through its ancestors, the working directory, and the launcher's folder.
// The bin folders cover an SDK archive extracted as it came.
inline std::vector<std::wstring> vendor_dirs(const wchar_t* vendor) {
    std::vector<std::wstring> out;
    auto add_from = [&out, vendor](std::wstring base) {
        if (base.empty()) return;
        for (const wchar_t* root : {L"upscalers", L"upscaler"}) {
            for (const wchar_t* sub : {L"", L"bin", L"bin\\x64", L"x64\\bin", L"x64"}) {
                add(out, join(join(join(base, root), vendor), sub));
            }
        }
    };

    // Walk a folder, its ancestors, and the copy of the engine checkout that
    // may sit below each one. The last part covers a release package where the
    // launcher is at the root and the engine is nested in engine/fefw-recomp.
    auto walk = [&add_from](std::wstring base, int levels) {
        for (int i = 0; i < levels && !base.empty(); ++i) {
            add_from(base);
            add_from(join(join(base, L"engine"), L"fefw-recomp"));
            const std::wstring up = parent_dir(base);
            if (up == base) break;
            base = up;
        }
    };

    walk(parent_dir(module_path()), 7);
    walk(working_dir(), 4);
    walk(launcher_dir(), 4);
    return out;
}

}  // namespace nxrt::extras::upscaler_paths
