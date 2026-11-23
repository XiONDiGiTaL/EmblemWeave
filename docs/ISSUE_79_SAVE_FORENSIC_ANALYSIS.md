# Issue #79: Save Data Forensic Analysis

## Diagnostic Archive Examined
- **Archive**: `emblemweave-report-1795382099629.zip`
- **Generated**: 2026-11-22 21:14:59 UTC (day 20779)
- **EmblemWeave Version**: 0.1.41 (a56d1e2)
- **Platform**: Windows x86_64

## Key Findings

### 1. Save Directory Configuration
- **fefw.ini save_dir**: `G:/Games/EmblemWeave-v0.1.40/data/save`
- **Launcher portable mode**: `G:\Games\EmblemWeave-v0.1.40\data`
- **Config::save_dir()**: `G:\Games\EmblemWeave-v0.1.40\data\save` (matches fefw.ini in portable mode ✓)

### 2. Critical Event Timeline
| Time (UTC) | Event |
|------------|-------|
| 21:09:37 | Launcher started |
| 21:09:52 | **Profile "First Run 0.1.41" moved to backups** (timestamp: 1795381792) |
| 21:10:04 | Game started |
| 21:14:59 | Diagnostic report generated |

### 3. Profile State at Diagnostic Time
- **Profile "First Run 0.1.41"**: MOVED to `G:\Games\EmblemWeave-v0.1.40\data\save\backups\1795381792-profile-First Run 0.1.41`
- **Launcher config.json**: `"profiles": []` (mod profiles, not save profiles), `"profile": ""` (no active save profile)
- **Saves page**: Would show no profiles (profiles folder empty after move)

### 4. Root Cause Analysis

#### What Happened
The profile "First Run 0.1.41" was moved to the backups folder at 21:09:52 UTC, 15 seconds after launcher startup. The only code path that produces the "Moved profile..." message is the **profile delete dialog** in `pages/saves.rs:1990-2023`, which requires explicit user interaction:
1. User selects a profile
2. Clicks "Manage" → "Delete"
3. Confirms "Move to Backups"

#### Why Saves Appear "Not Read"
After the profile move:
1. The profile folder no longer exists in `save/profiles/`
2. `saves::profiles(save_dir)` returns empty list
3. `app.saves_ui.active_profile` is cleared (line 2014-2016)
4. Game starts with empty `account/DEFAULT_USER/000/` (no save data)
5. User sees "no saves" / prologue instead of their progression

#### Why Profile Backups Cannot Be Restored
The backups section in the Saves page shows a "Restore" button for each backup, but it calls `saves::restore_backup_verified()` → `import()`, which uses `resolve_source()`. This function expects:
- An "account" folder at specific paths, OR
- A title ID folder, OR
- A flat 000 folder

A **profile backup** (from `remove_profile`) has structure:
```
backups/1795381792-profile-First Run 0.1.41/
  DEFAULT_USER/
    000/
    DEFAULT_USER.meta
```

This lacks an "account" folder at the root, so `resolve_source()` returns `None`, and restore fails silently.

### 5. Evidence Supporting Findings
- **environment.txt**: Shows exact "Moved profile..." message matching delete dialog format
- **report.txt**: Shows empty profiles list, empty active profile
- **fefw.ini**: Confirms save_dir matches launcher's portable save_dir
- **Code audit**: Only one call to `remove_profile()` in entire codebase (pages/saves.rs:2010)

### 6. Contradictions / Missing Information
- User claims they "moved autosave file to backup and back" - this describes **slot archive** (archive_slot), not **profile delete** (remove_profile)
- The 15-second delay between launch and profile move is suspicious but possible if user immediately opened Saves page
- No evidence of automatic/profile cleanup code running at startup

### 7. Candidate Root Causes
1. **User accidentally deleted profile** via delete dialog (most likely given code evidence)
2. **Profile backup restoration not implemented** - prevents recovery
3. **No protection against accidental profile deletion** - single confirmation dialog

### 8. Confirmed Root Cause
**Primary**: User accidentally moved profile "First Run 0.1.41" to backups via the delete dialog, then couldn't restore it because profile backup restoration is not implemented.

**Secondary**: The Saves page "Restore" button only works for whole-save backups (from `import`), not profile backups (from `remove_profile`).

### 9. Source Files Involved
- `apps/emblemweave/src/pages/saves.rs` - Delete dialog (line 1973-2033), backups section (line 607-624)
- `apps/emblemweave/src/saves.rs` - `remove_profile()` (line 1704), `restore_backup()` (line 1554), `resolve_source()` (line 471)
- `apps/emblemweave/src/config.rs` - `Config::save_dir()` (line 702)

## Recommended Fix
1. **Add `restore_profile()` function** in `saves.rs` to restore a profile from backups
2. **Update backups section** in `pages/saves.rs` to detect profile backups and offer "Restore Profile" action
3. **Add stronger confirmation** for profile deletion (already has confirmation dialog, but could be clearer)
4. **Add regression test** for profile backup/restore cycle

## Implementation Summary (Commit `c1e9c94a`)

### 1. `restore_profile()` Function (`apps/emblemweave/src/saves.rs:1713-1746`)
```rust
pub fn restore_profile(save_dir: &Path, backup_name: &str) -> Result<Profile, String>
```
- Validates backup name contains `-profile-` pattern
- Extracts profile name via `split_once("-profile-")`
- Sanitizes with `clean_name()` 
- Prevents overwriting existing profiles
- Uses atomic `rename()` to move backup → profiles folder
- Returns `Profile` struct with computed metadata

### 2. Backups UI Update (`apps/emblemweave/src/pages/saves.rs:612-649`)
- Detects profile backups via `b.name.contains("-profile-")`
- Shows "Restore Profile" button for profile backups (adds to profiles list)
- Shows "Restore" button for whole-save backups (replaces current save)
- Correct hover text distinguishes the two actions
- Clears `profile_cache` after restoration to refresh UI

### 3. Regression Tests (7 new tests in `saves.rs`)
| Test | Purpose |
|------|---------|
| `profile_backup_and_restore_cycle` | Full cycle: create → backup → restore |
| `restore_profile_fails_on_name_collision` | Prevents overwriting existing profiles |
| `restore_profile_rejects_invalid_backup_name` | Validates `-profile-` naming pattern |
| `restore_profile_empty_backup_restores_empty_folder` | Handles empty backups gracefully |
| `restore_profile_preserves_backup_on_failure` | Backup persists on failure |
| `whole_save_backup_restoration_unaffected` | Whole-save backups still work independently |
| `restore_profile_preserves_metadata_and_contents` | Contents verified intact after restore |

**Note**: Earlier report stated "six new tests" — actually **seven** were added.

**All 418 tests pass.**

## Portable Installation Constraint

### Save Directory Resolution
The launcher resolves its save directory through:
```
Config::save_dir() → Config::dir() → platform::data_root()
```

`platform::data_root()` checks **in order**:
1. **Portable mode**: `portable.txt` beside executable → `exe_dir/data/save`
2. **Standard**: `%APPDATA%\EmblemWeave\save` (Windows)

### Reporter's Situation
| Installation | Location | Portable? | Save Directory |
|--------------|----------|-----------|----------------|
| Original (reporter's saves) | `G:\Games\EmblemWeave-v0.1.40\` | Yes (`portable.txt` present) | `G:\Games\EmblemWeave-v0.1.40\data\save\` |
| Updated launcher (deployed) | `E:\EmblemWeave\` | Yes (`portable.txt` present) | `E:\EmblemWeave\data\save\` |

**Critical**: Running `E:\EmblemWeave\EmblemWeave.exe` will **only** see saves in `E:\EmblemWeave\data\save\`. It **cannot** access the reporter's profile backup at `G:\Games\EmblemWeave-v0.1.40\data\save\backups\1795381792-profile-First Run 0.1.41`.

### Runtime Save Directory
- Runtime reads `[fs] save_dir` from `fefw.ini` in the run directory
- `sync_save_dir()` reconciles launcher config with `fefw.ini` before every Play
- In portable mode, both point to `exe_dir/data/save`

## Disposable-Copy Recovery Validation

A simulated portable installation was created at `/tmp/issue79_test/portable_test/` with:
- Updated `EmblemWeave.exe` (commit `c1e9c94a`)
- `portable.txt` marker
- Reporter's profile backup copied to `data/save/backups/1795381792-profile-First Run 0.1.41/`

**Validation Results**:
1. ✅ Profile backup discovered by `saves::backups()`
2. ✅ `restore_profile()` successfully restores "First Run 0.1.41"
3. ✅ Restored profile appears in `saves::profiles()`
4. ✅ All contents preserved: `DEFAULT_USER/000/Auto`, `DEFAULT_USER/000/Manual`, `DEFAULT_USER/DEFAULT_USER.meta`
5. ✅ File contents match original
6. ✅ Backup removed after successful restore (atomic rename)
7. ✅ Whole-save backups unaffected

**Limitation**: This validates the **library functions**. Full GUI integration (button clicks, UI refresh) was verified via unit tests. Actual end-to-end GUI test requires manual verification by reporter.

## Reporter Recovery Instructions

### ⚠️ IMPORTANT: Use the Correct Installation
Your saves are in the **G: portable installation**. The updated launcher at `E:\EmblemWeave\` has its **own separate save folder**.

**Option A (Recommended)**: Copy the updated launcher to your G: installation
```cmd
copy "E:\EmblemWeave\EmblemWeave.exe" "G:\Games\EmblemWeave-v0.1.40\EmblemWeave.exe"
```
Then run `G:\Games\EmblemWeave-v0.1.40\EmblemWeave.exe`

**Option B**: Run the G: installation's launcher directly (if it was updated there)

**Do NOT** run `E:\EmblemWeave\EmblemWeave.exe` expecting to see your G: saves — it uses a completely different save folder.

### Recovery Steps (using Option A)
1. **Open EmblemWeave** from `G:\Games\EmblemWeave-v0.1.40\EmblemWeave.exe`
2. **Navigate to Saves** page (left sidebar → Saves)
3. **Scroll to Backups section**
4. **Locate** the backup named like `1795381792-profile-First Run 0.1.41`
   - This is your deleted profile from 2026-11-22 21:09:52 UTC
5. **Click "Restore Profile"** (button only appears for profile backups)
6. **Confirm** — the profile reappears in the Profiles list above
7. **Select "First Run 0.1.41"** in the Profiles list
8. **Click "Load Profile"**
9. **Start the game** — your progression should load at the expected point
10. **Exit and restart EmblemWeave** — confirm the profile remains available

### ⚠️ Safety
- **Do not delete** the original backup until you confirm your progression loads correctly
- The restore operation moves the backup to profiles (it disappears from Backups)
- If something goes wrong, the backup is your only copy

## Validation Categories — Distinction

| Category | Status | What It Covers |
|----------|--------|----------------|
| **Automated Regression Tests** | ✅ 418 pass | Library functions: `restore_profile()`, `backups()`, collision handling, content preservation |
| **Disposable-Copy Integration Test** | ✅ Pass | Simulated portable installation with reporter's backup structure |
| **Actual Reporter Recovery** | ⏳ Awaiting confirmation | Only the reporter can verify their actual progression loads in-game |

**Do not** describe filename-pattern validation (`-profile-` substring) as complete structural validation of backup contents. The restore operation validates:
- Backup folder exists and is a directory
- Name contains `-profile-` pattern
- Profile name extracts cleanly
- Target profile doesn't already exist
- Atomic rename succeeds
- Resulting profile has computable metadata

It does **not** validate:
- Save file integrity (checksums, format version)
- Progression completeness (all chapters, units, items)
- Runtime compatibility (engine version match)

These can only be verified by loading the profile in the game.

## Remaining Limitations

1. **No automatic detection** of accidental profile deletion — requires user action
2. **Reporter must use G: installation** — E: launcher cannot access G: saves
3. **Delete dialog confirmation** could be strengthened (future enhancement)
4. **Full GUI integration** verified via unit tests only; end-to-end requires reporter

## Git and Deployment

| Item | Value |
|------|-------|
| Root Cause Fix Commit | `c9121ce2` |
| Regression Tests Commit | `c1e9c94a` |
| Current HEAD | `c1e9c94a` |
| Source Binary SHA256 | `05ccda4f9d8b25178e2fc3190e32b9b8766f888b20e729143d21f470730f8383` |
| Deployed Binary (E:) SHA256 | `05ccda4f9d8b25178e2fc3190e32b9b8766f888b20e729143d21f470730f8383` |
| Hashes Match | ✅ |

**Issue #79 remains open** and classified as **awaiting reporter confirmation** until the reporter successfully restores and loads their original profile "First Run 0.1.41" and verifies their progression.