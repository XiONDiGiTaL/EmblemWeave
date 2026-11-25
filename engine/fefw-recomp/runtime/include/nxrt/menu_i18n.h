// SPDX-License-Identifier: GPL-3.0-or-later
// Runtime menu translation (F1 overlay). Load a key = value table once and translate strings through nxrt::gfx::tr().
// The table lives next to fefw_rt.dll in locales\<name>.ini, chosen by [gfx] ui_language ("auto"/empty: the table for the
// Windows display language when there is one; "en": the built-in English).
// Missing keys fall back to the input string, so a partial table is always safe.
#pragma once

#include <algorithm>
#include <cctype>
#include <string>
#include <vector>

namespace nxrt::gfx {

// The table names that suit a Windows locale name ("pt-BR": "pt-BR", then "pt"), best first. Chinese goes by the script: "zh" is
// the Simplified table, so a Traditional locale never gets it. The launcher follows the same rules (launcher/src-tauri/src/lang.rs).
inline std::vector<std::string> menu_language_candidates(const std::string& tag) {
    std::vector<std::string> parts(1);
    for (const char c : tag) {
        if (c == '-' || c == '_') parts.emplace_back();
        else parts.back().push_back(static_cast<char>(std::toupper(static_cast<unsigned char>(c))));
    }
    std::string primary = parts[0];
    for (char& c : primary) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    if (primary.empty()) return {};
    std::vector<std::string> out = {tag};
    if (primary != "zh") {
        if (primary != tag) out.push_back(primary);
        return out;
    }
    const auto has = [&parts](const char* name) { return std::find(parts.begin() + 1, parts.end(), name) != parts.end(); };
    const bool traditional = has("HANT") || (!has("HANS") && (has("TW") || has("HK") || has("MO")));
    for (const char* name : traditional ? std::vector<const char*>{"zh-Hant", "zh-TW"} : std::vector<const char*>{"zh-Hans", "zh-CN", "zh"})
        out.emplace_back(name);
    return out;
}

// The table [gfx] ui_language asks for, "" for the built-in English: the named one, or for "auto" the first table present that
// suits the Windows display language.
std::string menu_language();

// Translate a UI string. Returns the input unchanged when no table is loaded or the key is missing.
const char* tr(const char* s);

// Why a table's line must not be used (its printf conversions are not the key's), or nullptr when it is sound.
const char* menu_translation_problem(const std::string& key, const std::string& value);

// Load the table for [gfx] ui_language. Called once from the overlay's font setup; safe to call again (reloads).
void menu_i18n_init();

} // namespace nxrt::gfx
