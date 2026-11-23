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