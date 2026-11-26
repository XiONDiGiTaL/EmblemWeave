// SPDX-License-Identifier: GPL-3.0-or-later
// Named interface tables for NxModApi version 6: contract in nxrt/mod_interfaces.h, rules in docs/MOD_INTERFACES.md.
//
// Registration happens before mods load (a host adds its own tables, then get_interface is served from this registry).
// Lookups may come from any thread and from nxmod_early; the registry takes its lock for the name lookup and calls a
// resolver after releasing it, so a resolver may not assume the lock is held.
#include "nxrt/mod_interfaces.h"

#include <cstring>

namespace nxrt::nxmod {
namespace {

bool segment_start(char c) { return (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9'); }
bool segment_char(char c) { return segment_start(c) || c == '-'; }

}  // namespace

bool valid_name(const char* name) {
    if (!name) return false;
    const std::size_t len = std::strlen(name);
    if (len == 0 || len > 127) return false;
    bool dot = false;
    bool at_segment_start = true;
    for (std::size_t i = 0; i < len; ++i) {
        const char c = name[i];
        if (c == '.') {
            if (at_segment_start) return false;   // a leading dot or an empty segment
            at_segment_start = true;
            dot = true;
            continue;
        }
        if (at_segment_start) {
            if (!segment_start(c)) return false;
            at_segment_start = false;
        } else if (!segment_char(c)) {
            return false;
        }
    }
    return dot && !at_segment_start;   // at least one dot, and no trailing one
}

bool InterfaceRegistry::add(const char* name, uint32_t version, const void* table) {
    return add_entry(name, version, table, nullptr, nullptr);
}

bool InterfaceRegistry::add_resolver(const char* name, uint32_t version, Resolver resolve, void* user) {
    return add_entry(name, version, nullptr, resolve, user);
}

bool InterfaceRegistry::add_entry(const char* name, uint32_t version, const void* table, Resolver resolve, void* user) {
    if (!valid_name(name) || version == 0 || (!table && !resolve)) return false;
    std::lock_guard<std::mutex> lk(mu_);
    for (const Entry& e : entries_)
        if (e.name == name) return false;   // one name, one table: never replace a registration
    entries_.push_back(Entry{name, version, table, resolve, user});
    return true;
}

const void* InterfaceRegistry::find(const char* name, uint32_t requested, const void* context) const {
    if (!valid_name(name)) return nullptr;
    Entry entry;
    bool found = false;
    {
        std::lock_guard<std::mutex> lk(mu_);
        for (const Entry& e : entries_) {
            if (e.name != name) continue;
            entry = e;
            found = true;
            break;
        }
    }
    if (!found) return nullptr;
    // Tables grow only at the end, so a newer table also answers a request for an older version. A request for a
    // version this runtime does not have is incompatible, not a reason to hand back an older table.
    if (requested != 0 && requested > entry.version) return nullptr;
    return entry.resolve ? entry.resolve(context, entry.user) : entry.table;
}

std::size_t InterfaceRegistry::count() const {
    std::lock_guard<std::mutex> lk(mu_);
    return entries_.size();
}

bool InterfaceRegistry::empty() const {
    std::lock_guard<std::mutex> lk(mu_);
    return entries_.empty();
}

InterfaceRegistry& interfaces() {
    // Process lifetime, like the runtime's other host tables: never destroyed, so a mod may hold a table it was given.
    static InterfaceRegistry* registry = [] {
        auto* r = new InterfaceRegistry;
        r->add_resolver(kNativeApi, kNativeApiVersion,
                        [](const void* context, void*) { return context; });
        return r;
    }();
    return *registry;
}

const void* get_interface(const NxModApi* api, const char* name, uint32_t version) {
    return interfaces().find(name, version, api);
}

}  // namespace nxrt::nxmod
