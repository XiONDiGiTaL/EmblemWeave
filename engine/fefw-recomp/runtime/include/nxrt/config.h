// SPDX-License-Identifier: GPL-3.0-or-later
// Configuration: an ini file plus environment-variable switches.
//   [section]
//   key = value        ; comment    # comment
// Lookup order for config_get_*("diag", "watchdog_seconds", def):
//   1. environment variable NXRT_DIAG_WATCHDOG_SECONDS   (NXRT_ + upper-case section + '_' + upper-case key)
//   2. the ini value
//   3. the default
// Keys outside any section use the section name "" and the env name NXRT_<KEY>.
#pragma once
#include <cstdint>
#include <string>
#include <vector>

namespace nxrt {

// Loads (replaces) the ini from a file. Returns false when the file cannot be read (the config stays empty).
bool config_load_file(const char* path);
// Loads from a string (tests). Replaces the current ini content.
void config_load_string(const char* text);
void config_clear();
// Programmatic override, as if it were in the ini (tests, command-line options). Env still wins over it.
void config_set(const char* section, const char* key, const char* value);

std::string config_get_str(const char* section, const char* key, const char* def = "");
int64_t config_get_int(const char* section, const char* key, int64_t def = 0);      // decimal, 0x hex, optional k/m/g suffix not supported
double config_get_double(const char* section, const char* key, double def = 0.0);
bool config_get_bool(const char* section, const char* key, bool def = false);       // 1/0 true/false yes/no on/off, case-insensitive
bool config_has(const char* section, const char* key);                              // set by env or ini

// Where a value came from: "env", "ini" or "default". For the startup dump.
const char* config_source(const char* section, const char* key);
// Logs (Info, tag "config") every key read so far with its effective value and source.
void config_log_effective();
// The environment variable name used for a key, e.g. NXRT_DIAG_WATCHDOG_SECONDS.
std::string config_env_name(const char* section, const char* key);

// ---- writing the player's ini back (the F1 menu; diag/ini_merge.cpp) ----
// The ini file config_load_file() was last given, read or not (empty: none; config_load_string() leaves it as it was).
std::string config_file_path();
// Merges ini text without losing what a person wrote: `updates` is ini text whose keys replace the values of the same keys in `base` in
// place (every occurrence; the line's indent and trailing comment stay), keys `base` lacks go after the last key of their section, sections
// it lacks at the end. Comments, blank lines, key order and keys `updates` does not name are kept. A section listed in `replace_sections`
// (lower case, e.g. "bindings.play") is replaced whole by the update's lines instead (sections whose keys repeat). An empty base gives
// `updates` as it is. Section and key names compare case-insensitively.
std::string ini_merge(const std::string& base, const std::string& updates, const std::vector<std::string>& replace_sections = {});
// Writes `key = value` into config_file_path() (merged as ini_merge does; a temp file renamed over it) and config_set()s it so later reads
// see it (an environment variable still wins). False, with *err, when there is no ini path or the file cannot be written.
bool config_write(const char* section, const char* key, const std::string& value, std::string* err = nullptr);

}  // namespace nxrt
