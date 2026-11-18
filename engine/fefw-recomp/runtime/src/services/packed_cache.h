// SPDX-License-Identifier: GPL-3.0-or-later
// Bounded LRU for decompressed zstd entries of a .nup package. A zstd entry is one frame for the whole file, so the RomFS reader decodes
// it whole and serves ranges from memory. Keeping every entry until a fixed budget is exceeded makes a large audio bank pin the budget
// and then drops every entry at once; this cache evicts the least recently used entry individually, and a file larger than the per-file
// cap is not retained at all.
#pragma once

#include <algorithm>
#include <cstdint>
#include <map>
#include <memory>
#include <mutex>
#include <vector>

namespace nxrt::services::fs {

class PackedCache {
public:
    struct Stats {
        uint64_t entries = 0;
        uint64_t bytes = 0;
        uint64_t peak = 0;
        uint64_t evictions = 0;
        uint64_t skipped = 0;
    };

    PackedCache(uint64_t budget, uint64_t entry_cap) : budget_(budget), entry_cap_(entry_cap) {}

    // Returns the cached copy and marks it most recently used, or null.
    std::shared_ptr<const std::vector<uint8_t>> get(uint32_t idx) {
        std::lock_guard<std::mutex> lk(mu_);
        auto it = entries_.find(idx);
        if (it == entries_.end()) return nullptr;
        it->second.used = ++tick_;
        return it->second.data;
    }

    // Drops the least recently used entries until `size` fits. An entry that cannot be retained also drops every cached entry, so
    // decoding a large audio bank does not hold the old cache alongside its output; readers that still hold a buffer keep only their
    // own reference.
    void make_room(uint64_t size) {
        std::lock_guard<std::mutex> lk(mu_);
        if (size > entry_cap_ || size > budget_) {
            drop_all();
            return;
        }
        while (bytes_ + size > budget_) drop_lru();
    }

    // Retains `data` when it fits the per-file cap and makes room by evicting the least recently used entries. Returns whether it was
    // retained; the caller still serves `data` when it was not.
    bool put(uint32_t idx, std::shared_ptr<const std::vector<uint8_t>> data) {
        if (!data) return false;
        const uint64_t size = data->size();
        std::lock_guard<std::mutex> lk(mu_);
        auto old = entries_.find(idx);
        if (old != entries_.end()) {
            bytes_ -= old->second.data->size();
            entries_.erase(old);
        }
        if (size > entry_cap_ || size > budget_) {
            ++skipped_;
            return false;
        }
        while (bytes_ + size > budget_) drop_lru();
        entries_.emplace(idx, Entry{std::move(data), ++tick_});
        bytes_ += size;
        peak_ = std::max(peak_, bytes_);
        return true;
    }

    void clear() {
        std::lock_guard<std::mutex> lk(mu_);
        entries_.clear();
        bytes_ = 0;
        peak_ = 0;
        tick_ = 0;
        evictions_ = 0;
        skipped_ = 0;
    }

    Stats stats() const {
        std::lock_guard<std::mutex> lk(mu_);
        return Stats{entries_.size(), bytes_, peak_, evictions_, skipped_};
    }

private:
    struct Entry {
        std::shared_ptr<const std::vector<uint8_t>> data;
        uint64_t used = 0;
    };

    void drop_lru() {
        if (entries_.empty()) return;
        auto victim = std::min_element(entries_.begin(), entries_.end(),
                                       [](const auto& a, const auto& b) { return a.second.used < b.second.used; });
        bytes_ -= victim->second.data->size();
        entries_.erase(victim);
        ++evictions_;
    }

    void drop_all() {
        while (!entries_.empty()) drop_lru();
    }

    uint64_t budget_;
    uint64_t entry_cap_;
    mutable std::mutex mu_;
    std::map<uint32_t, Entry> entries_;
    uint64_t bytes_ = 0;
    uint64_t peak_ = 0;
    uint64_t tick_ = 0;
    uint64_t evictions_ = 0;
    uint64_t skipped_ = 0;
};

}  // namespace nxrt::services::fs
