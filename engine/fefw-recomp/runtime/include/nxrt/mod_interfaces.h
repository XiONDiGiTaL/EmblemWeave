// SPDX-License-Identifier: GPL-3.0-or-later
// Named interface tables for code mods (NxModApi version 6, nxrt/mod_api.h).
//
// NxModApi::get_interface answers from an InterfaceRegistry: a mod asks for a feature by name and version instead of
// inferring it from the API version. The rules (docs/MOD_INTERFACES.md):
//
//   - a name is canonical, dotted and lower-case: <owner>.<local-name>, segments of [a-z0-9-] starting with a letter or
//     digit ("nxp.ui", "emblemweave.ui.panel", "obeliskchamber.list"). Names are ASCII and case-sensitive;
//   - every interface table starts with uint32_t size and uint32_t version and grows only at the end. A request for
//     version N is answered by a table at version N or a later, compatible one; 0 asks for any version;
//   - one name, one table. Registering a name twice is refused rather than replacing the first: a collision is a bug in
//     the host, not a feature negotiation;
//   - names are owned by the project that defines them. A host answers for its own prefix and for the shared names it
//     implements; it does not synthesize another host's ("obeliskchamber.list" is NULL outside ObeliskChamber).
//
// The shared name nxp.nxmod (version 1) is the bridge to the legacy table: it answers the caller's own NxModApi, not a
// copy, so a plugin built on a shared layer reaches hooks, guest calls, memory and file edits wherever version 6 exists.
#pragma once
#include <cstddef>
#include <cstdint>
#include <mutex>
#include <string>
#include <vector>

struct NxModApi;   // nxrt/mod_api.h

namespace nxrt::nxmod {

// The name of the bridge to the host's NxModApi table (version 1).
inline constexpr const char* kNativeApi = "nxp.nxmod";
inline constexpr uint32_t kNativeApiVersion = 1;

class InterfaceRegistry {
public:
    // A table that depends on the caller: `context` is the NxModApi passed to get_interface. Returns the table or NULL.
    using Resolver = const void* (*)(const void* context, void* user);

    // Registration. False when the name is not canonical, the version is 0, the table is null, or the name is taken.
    bool add(const char* name, uint32_t version, const void* table);
    bool add_resolver(const char* name, uint32_t version, Resolver resolve, void* user = nullptr);

    // Lookup. `requested` 0 = any version; a request newer than the registered table answers NULL. Names are compared
    // exactly (case-sensitive) and validated first, so an invalid name is the same as an unknown one.
    const void* find(const char* name, uint32_t requested, const void* context) const;

    std::size_t count() const;
    bool empty() const;

private:
    struct Entry {
        std::string name;
        uint32_t version = 0;
        const void* table = nullptr;
        Resolver resolve = nullptr;
        void* user = nullptr;
    };
    bool add_entry(const char* name, uint32_t version, const void* table, Resolver resolve, void* user);

    mutable std::mutex mu_;
    std::vector<Entry> entries_;
};

// The process's registry. get_interface answers from it; nxp.nxmod v1 is registered on first use. A host adds its own
// interfaces here before it loads mods.
InterfaceRegistry& interfaces();

// A canonical interface name: at least two dot-separated lower-case segments of [a-z0-9-], each starting with a letter
// or digit. Null, empty, upper-case, a leading or trailing dot, an empty segment and anything over 127 bytes are invalid.
bool valid_name(const char* name);

// NxModApi::get_interface (version 6): interfaces().find(name, version, api).
const void* get_interface(const NxModApi* api, const char* name, uint32_t version);

}  // namespace nxrt::nxmod
