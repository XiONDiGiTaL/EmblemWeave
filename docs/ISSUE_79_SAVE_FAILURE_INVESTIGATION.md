# Issue #79: Saves not being read, an independent investigation

**Issue:** [XiONDiGiTaL/EmblemWeave#79](https://github.com/XiONDiGiTaL/EmblemWeave/issues/79), titled `[Bug] Saves files not being read`
**Reporter:** KalHawke
**Reported build:** EmblemWeave 0.1.41 (`a56d1e2`), Windows x86-64, game data **Warn**, built game **yes**
**Audit branch:** `main` @ `22a380c`, compared against tags `v0.1.40`, `v0.1.41`
**Scope:** research and diagnosis only. No production code was changed, no build or gameplay run was made, no save file was copied, moved or written.

**Classification: MULTIPLE PLAUSIBLE CAUSES.**
Two independent mechanisms fit the report, one of them is a confirmed defect in the current code (the
launcher and the runtime can resolve different save roots, and the runtime then silently creates an empty
save). The reporter's own evidence bundle was never attached to the issue, so the two leading causes
cannot be separated for *this* installation without it. The fixed account UID and title ID are **not**
involved: that is proven below, not assumed.

---

## 1. Issue summary

The reporter states that the game "boots into the prologue directly, as if there's no saves files", and that
moving saves around and "setting up a profile" did not help. They read the troubleshooting guide, rebuilt the
game, and did not check the toolchain.

Three facts in that text matter for diagnosis:

1. **The game runs.** "Built game: yes" and a prologue mean the executable, RomFS and shaders are good enough
   to reach gameplay. This is not a boot failure, a shader failure or a missing-game-data failure.
2. **The user intervened in the save folder by hand** ("moving saves around"). That makes layout errors a live
   possibility, not a theoretical one.
3. **"Game data: Warn"** is not about saves. In `apps/emblemweave/src/gamedata.rs:341`, `Warn` on the
   **Game build** check means the ExeFS module hashes differ from the dump the engine was tested with
   ("a different build of the game … this may work but is not the tested build"). A different game build is a
   separate compatibility risk (see H4).

No comment on the issue claims a fix, and there are no comments at all.

---

## 2. Diagnostic bundle: availability and inspection status

**The bundle was not available and was not inspected.**

Checked through the GitHub API on 2026-11-22 (issue record plus the comment list):

| Check | Result |
| --- | --- |
| Issue body attachment URL (`github.com/user-attachments/...`) | **None** |
| Issue comments | **0** |
| Any other upload on the issue | **None** |

The body does contain the filename, but in the launcher's own template wording:

> `- Diagnostic report: emblemweave-report-1795382099629.zip (created by Troubleshooter; attach the file to this issue)`

That sentence is generated locally by `apps/emblemweave/src/report.rs:278-305`. `issue_url()` deliberately
never uploads anything: it writes the ZIP beside the report and puts only its **name** in the prefilled issue.
The player has to drag the file into the page afterwards. The reporter did not, so the archive exists only on
their PC.

**Consequence:** none of the following could be read for this report: `report.txt`, `config.json`, launcher
logs, `game/fefw.ini`, `game/fefw.log`, `environment.txt`, the crash folders, or any save-directory listing.
Every statement below about *this* installation's paths is therefore an inference from code and from the
reporter's prose. Statements about the code, and about a real installation on the audit machine, are marked as
such.

**Process finding:** a report that is worth generating but is not attached leaves the maintainer with a
symptom and no paths. The bundle already contains the single file that would settle this issue
(`game/fefw.ini`, copied at `report.rs:223`); see the remediation spec, R6.

---

## 3. Evidence base

| Evidence | What it establishes |
| --- | --- |
| `apps/emblemweave/src/saves.rs` (2,154 lines) | Launcher-side save model, import, profiles, backups, account folder choice |
| `apps/emblemweave/src/pages/saves.rs`, `pages/setup.rs` | What the UI installs and what it only displays |
| `apps/emblemweave/src/config.rs`, `platform.rs`, `portable.rs` | Storage-root resolution and portable mode |
| `apps/emblemweave/src/engine.rs` | `fefw.ini` generation, Play launch, absence of any save-root reconciliation |
| `engine/fefw-recomp/runtime/src/services/fs.cpp` | Save key construction, `.meta` contract, mount, **silent empty-save creation** |
| `runtime/src/services/common.cpp`, `acc.cpp`, `games/fefw/fefw_core.cpp`, `fefw_data.cpp` | Account UID defaults, UID serialization, runtime default save root |
| `git diff v0.1.40 v0.1.41` on the save/storage files | Whether 0.1.41 regressed the save path (it did not) |
| Local `fefw.log`, `nxrt.log.prev` (audit machine) | The runtime's effective save root and the auto-create log lines |
| `%APPDATA%\EmblemWeave\save` and `%APPDATA%\FortunesWeaveRecompPC\save` (read-only listing) | Two save roots on one real machine; the second holds an empty auto-created account save |

No save file on the audit machine was opened, hashed, copied or modified; only directory and file **names**
were listed, plus a `[fs]` section read from a *diagnostic copy* of `fefw.ini` under
`%APPDATA%\EmblemWeave\crashes\`.

---

## 4. The save lifecycle, stage by stage

The launcher and the runtime are two programs that agree only through files. The chain is:

```
emulator NAND / user folder / old install
        │  (1) discovery
        ▼
launcher: saves::find / resolve_source
        │  (2) save-directory resolution        Config::save_dir() = <data root>\save
        ▼
launcher: account_dir() = <save>\account\0100000000000000706d635257464546
        │  (3) profile storage                  <save>\profiles\<name>
        │  (4) create / select                  pages/saves.rs carousel
        │  (5) load                             load_profile_verified()
        │  (6) backup current                    rename <account dir> -> <save>\backups\<stamp>
        │  (7) copy profile                       copy_tree / copy_flat into <account dir>
        │      write <account dir>.meta if absent  meta_text()
        ▼
launcher: engine::ini_text() → <data root>\play\fefw.ini   [fs] save_dir = <save>
        │  (8) runtime save-directory configuration (written at BUILD time, passed with --ini at Play)
        ▼
runtime: config_get_str("fs","save_dir")  else %APPDATA%\FortunesWeaveRecompPC\save
        │  (9) account/user selection           profile().user_uid_hi/lo
        ▼
runtime: save_key() = "account/" + hex(uid bytes)   (10) mount
        │  save_exists(key) = <save_dir>\account\<uid>.meta is a file AND account\<uid> is a folder
        │  if false → ensure_account_save() creates an EMPTY account save  ← silent fallback
        ▼
game: opens 000/ inside that volume, lists slots, shows Continue or starts the prologue   (11)
```

### Stage details and the exact code

| # | Stage | Code | Failure mode at this stage |
| --- | --- | --- | --- |
| 1 | Discovery | `saves::find`, `find_here`, `resolve_source` (`saves.rs:59`, `:112`, `:336`) | Finds nothing (emulator folder elsewhere), or resolves the wrong level |
| 2 | Save root | `Config::save_dir()` (`config.rs:698`) → `platform::data_root()` (`platform.rs:202-250`) | Portable vs `%APPDATA%`; a *different* root than the engine's ini |
| 3 | Profile storage | `profiles()` (`saves.rs:1033`), `profile_slots` (`:703`) | Stale cache; a profile folder that is not the installed save |
| 4 | Create/select | `create_empty_profile` (`:856`), carousel `selected_profile` (`pages/saves.rs`) | Selection is **view-only**; the badge is not the install state |
| 5 | Load | `load_profile_verified` (`:955`) | Refuses unreadable slots; on copy mismatch restores the old save |
| 6 | Backup | `import` (`:417`), `load_empty_profile` (`:986`) | The previous save is *moved* to `backups/<stamp>`, not deleted |
| 7 | Copy | `copy_tree` / `copy_flat` (`:160`, `:183`), `meta_text` (`:211`) | Wrong nesting; `.meta` kept from a foreign save; copy into the wrong account folder |
| 8 | Runtime config | `engine::ini_text` (`engine.rs:2072-2102`), `play()` (`:4960`) | **Absolute `save_dir` baked at build time; Play never re-reads or repairs it** |
| 9 | User selection | `load_profile` (`common.cpp:328-332`), `acc.cpp:49,63` | Only if `[account] uid` were set to a differently ordered value (it is not, by default) |
| 10 | Mount | `save_key` (`fs.cpp:99-118`), `save_exists` (`:165`), `open_save` (`:644-680`) | Key names an account folder that does not exist → **empty save created** |
| 11 | Slot discovery | `fs::Volume` over `<account>\000`, game reads slot files and `Global` | The volume is empty or has no slot files → the game offers a new game |

### The single most important line in the chain

`fs.cpp:662-670`:

```cpp
if (a.type == 1 && (a.application_id == p.program_id || a.application_id == 0)
    && p.account_save_size > 0 && !save_exists(key)) {
    ...
    NXLOG_I("fs", "OpenSaveDataFileSystem %s: creating the account save data of the starting user", key.c_str());
    fsapi::ensure_account_save(hi, lo, nullptr);
}
```

If the expected account save is not at the exact path the runtime computed, the runtime **creates an empty
one** and the game proceeds normally. There is no dialog, no refusal and no launcher notification. From the
player's seat the game simply "has no saves".

---

## 5. Expected versus actual directory layouts

### Expected (what the launcher installs and the runtime opens)

```
<data root>\save\
├── account\
│   ├── 0100000000000000706d635257464546\        <- DEFAULT_USER, the folder the game opens
│   │   └── 000\
│   │       ├── Auto
│   │       ├── Chapter_..._N
│   │       ├── Route_...
│   │       └── Global
│   └── 0100000000000000706d635257464546.meta    <- size / journal / owner / attr
├── profiles\<name>\000\...                       <- copies, never read by the game
└── backups\<stamp>\...                           <- whole saves moved aside
```

### Observed on the audit machine (read-only)

`%APPDATA%\EmblemWeave\save\account\0100000000000000706d635257464546\000\` contains
`Auto`, `Auto.bak`, `Chapter_Ism_0_2`, `Chapter_Ism_0_2.bak`, `Chapter_Ism_0_3`, `Chapter_Seo_1_1`,
`Chapter_Seo_1_2`, `Chapter_Seo_1_3`, `Chapter_Seo_1_3.orig`, `Chapter_Seo_1_4`, `Global`, `Route_Ism`,
with the sibling `.meta`. That is a healthy, game-written tree and it matches the expected layout exactly.

`%APPDATA%\FortunesWeaveRecompPC\save\account\0100000000000000706d635257464546\` exists **but contains no
`000`**: it is the empty account save that `open_save()` auto-creates when the runtime is pointed at its
built-in default root. `nxrt.log.prev` records the matching lines:

```
fs: fsp-srv: RomFS from 'game/romfs', save data in 'C:\Users\...\AppData\Roaming\FortunesWeaveRecompPC\save'
fs: OpenSaveDataFileSystem account/11110000000000002222000000000000: no such save data
fs: save data created: account/11110000000000002222000000000000 (4194304 bytes, journal 1048576)
```

Two roots therefore exist on a real machine, and the non-launcher root holds a synthetic empty save. That is
the failure *shape* of #79, reproduced at the filesystem level without running the game.

### The other observed shape: a save under a foreign account id

A yuzu/Eden title folder copied by hand into `<data root>\save\` usually retains the **console user id** as
its account folder name, for example:

```
save\account\11110000000000002222000000000000\000\Auto     <- real save, wrong folder name
save\account\0100000000000000706d635257464546\             <- absent, or an empty save the runtime made
```

The runtime opens only the second name. The files are present, the game sees nothing, and the launcher's
"current save" list (`saves::slots`, `saves.rs:1421`) also reads only `account_dir()` and therefore shows
nothing. This is the layout class behind issue #10; the launcher's **Import** normalizes it, a manual copy
does not.

---

## 6. Account identity: the fixed UID and title ID are correct

This was checked byte by byte, because "the launcher and the runtime disagree about the user" is the most
tempting explanation and it is **false** for a default installation.

| Source | Value |
| --- | --- |
| Launcher `DEFAULT_USER` (`saves.rs:193`) | `"0100000000000000706d635257464546"` |
| Runtime `title_defaults` (`fefw_core.cpp:32-33`) | `hi = 0x0000000000000001`, `lo = 0x4645465752636d70` ("FEFWRcmp") |
| UID serialization (`acc.cpp:63`, `put_uid`) | `hi` little-endian, then `lo` little-endian |
| Bytes produced | `01 00 00 00 00 00 00 00  70 6d 63 52 57 46 45 46` |
| Folder name (`fs.cpp:108`, `hex_of(a.uid,16)`) | `0100000000000000706d635257464546` |

The folder name is the **memory byte order**, not the display order of `hi`/`lo`. The launcher's constant is
that same memory order, so it matches. The runtime's own `ensure_account_save` writes the uid the same way
(`fs.cpp:914-925`), and `save_key` builds `account/<hex>` from the mount attribute. The installed save on the
audit machine is in exactly that folder, written by the game itself, which is an independent confirmation.

Two adjacent details are also correct:

* `meta_text()` (`saves.rs:211-225`) writes `size=67108864`, `journal=67108864`, `owner=<title>`,
  `attr=<0x40 bytes>` with the uid at `attr[8..24]`, `type=1` at `attr[32]`. `title_defaults` asks for
  `0x4000000` for both size and journal. They agree.
* `export()` (`saves.rs:550`) writes `<dest>\<TITLE>.` and `import()` reads the title folder; the title id is
  taken from the dump/NACP with `FALLBACK_TITLE_ID = 0x0400_7EF0_1F8A_E000` as the fallback
  (`pages/setup.rs:499-507`). The `.meta` owner therefore names the same title the runtime mounts.

**Latent trap, not this bug:** `parse_uid()` (`common.cpp:269`) reads `[account] uid` in *display* order and
would produce a folder name one byte-swap away from `DEFAULT_USER` if someone wrote the launcher's constant
into that key. No launcher code writes `[account] uid`, so a default install is unaffected. Any future
"manual UID override" feature must not copy the folder-name spelling into that key.

**Conclusion:** the fixed account UID, the `.meta` attribute and the title ID are consistent, and a save in
the expected folder is readable. This removes H5 as a cause (see §10).

---

## 7. Where the expected data can become invisible

Ordered by how directly the code produces "game starts a new game":

| ID | Choke point | Mechanism | Code |
| --- | --- | --- | --- |
| C1 | **Save root mismatch** | `[fs] save_dir` is written once at build with an **absolute** path. If the data root later moves (portable install moved/copied, `data\` moved to a new folder, marker added/removed), the ini keeps the old path. Play passes `--ini` and never checks the value. The runtime reads the old path, finds no save there, and creates an empty one. The launcher's Saves page reads the *new* root. | `engine.rs:2072-2102`, `:4960-5030`; `config.rs:698`; `platform.rs:108-136`; `fs.cpp:662-670` |
| C2 | **Foreign account folder** | Save copied under `account/<console uid>` instead of `account/DEFAULT_USER`. Runtime opens DEFAULT_USER only and creates an empty save; launcher "current save" list is empty too. | `saves.rs:203`; `fs.cpp:108,163,662` |
| C3 | **Wrong nesting under the account dir** | `account/DEFAULT_USER/000/000/...` or `account/DEFAULT_USER/<uid>/000/...`. The volume opens but holds no slot files at its root, so the game sees an empty save. Hand-copying a title folder one level too deep produces this. The launcher's **Import** un-nests; a manual copy does not. | `saves.rs:336-410`; `fs::Volume` over `000` |
| C4 | **Empty profile was loaded** | `load_empty_profile` moves the current account dir to `backups/<stamp>` and installs an empty one. The game then correctly starts a new game. The real save still exists under `save\backups\`. | `saves.rs:986-1040` |
| C5 | **UI truth gap** | The carousel shows a *selected* profile; `active_profile` is only a hint and the "installed" badge compares files. A profile can be visible and healthy while the game reads a different folder or root. Selection is documented as view-only. | `pages/saves.rs:1-10`, `saves.rs:820-854` |
| C6 | **Runtime default root** | If `[fs] save_dir` is missing or the ini is not the one passed (`fefw.exe` started directly, or with another cwd/ini), the runtime falls back to `%APPDATA%\FortunesWeaveRecompPC\save`. This root is **not** the launcher's root. | `common.cpp:376-382`; `fefw_data.cpp:33` |
| C7 | **Slot validation refuses a load** | `check_profile` requires every slot file to parse; a save from a different game version can be refused before install. The error is shown, so this is *not* silent, but it is a different symptom from the report. | `saves.rs:1104-1160` |
| C8 | **Meta contract** | `save_exists` requires the sibling `.meta`. A folder with `000` but no `.meta` is "no such save data"; the runtime then writes a meta and opens the existing tree, so the files are not lost, but the log says "created", which misleads diagnosis. | `fs.cpp:138-180`, `:662` |

C1 and C2 are the only two that produce the reported symptom **without any launcher error**, survive a
rebuild, and leave the files on disk. C4 is recoverable and leaves a backup. C7 would show a message. C8
self-heals the tree.

---

## 8. Historical fixes and whether one regressed

| Issue | Fix | Current state | Verdict |
| --- | --- | --- | --- |
| #10 account-directory selection | `account_dir()` pinned to `DEFAULT_USER`; `account_save()` still resolves other ids for *import* (`saves.rs:203-270`) | Present | **Correct and still needed.** It fixed *loading*; it does not migrate a save that was copied by hand under another id (C2) |
| #43 incorrectly nested imports | `resolve_source()` resolves title/account/install/NAND/flat shapes; `import()` refuses if nothing landed in `000` and restores the backup (`saves.rs:336-540`) | Present | **Correct.** A manual copy bypasses it entirely (C3) |
| #48 `Global` treated as an invalid slot | `is_global_save_file()` excluded from slot candidates (`saves.rs:657`) | Present | **Correct.** `Global` is copied, not validated as a slot |
| 0.1.39 profile/import work | Profile resolve-before-validate, backup-first load, verified restore | Present | **Correct** |
| 0.1.34 portable installs | marker `portable.txt`, `data\` beside the exe, adoption only when `%APPDATA%\EmblemWeave` is absent | Present | **Correct**, but it is also what makes the root movable, which is the precondition for C1 |
| 0.1.40 → 0.1.41 | `saves.rs` diff is 21 lines, all `route_lord` display; `savefile.rs` adds convoy/clock/route parsing; `fs.cpp` changes one mods-overlay argument; `config.rs::save_dir()` and `platform.rs::data_root_from()` are **identical** between the tags | Present | **No save-path regression in 0.1.41** |

The 0.1.41 comparison is worth stating plainly: the launcher's storage code did not change in the release the
reporter runs. If this is a regression, it predates 0.1.41 (the 0.1.34 portable switch is the last change that
can move the root), or it is not a code regression at all but a layout/installation condition.

**The current implementation is internally correct but not self-verifying.** It is correct for a save that
sits at `<Config::save_dir()>\account\DEFAULT_USER\000` and for an ini whose `[fs] save_dir` equals
`Config::save_dir()`. It has no mechanism to notice when either premise is false, and its failure mode is a
silently created empty save.

---

## 9. Ranked hypotheses

### H1: The save is present but under a different account folder than the runtime opens (C2/C3)

**Fit:** high. Explains "as if there's no save files", survives a rebuild, and is unaffected by the
troubleshooting guide. The most common way this happens is exactly what the reporter describes: copying a
yuzu/Eden title folder by hand into EmblemWeave's `save` instead of using **Import**, which keeps the
emulator's console user id (`11110000…`) as the account folder. A second variant is one level too many
(`account/DEFAULT_USER/000/000` or `account/DEFAULT_USER/<uid>/000`).

**Supporting code:** `account_dir()` is fixed to `DEFAULT_USER`; `fs::save_key()` derives the folder from the
mount UID, which is also fixed; nothing scans for or adopts a save found under another account folder at
startup; the launcher's "current save" view reads only the canonical folder.

**Would a rebuild fix it?** No. **Would "moving saves around" fix it?** Only by coincidence.

**What would confirm it:** the bundle's `game/fefw.ini` (`[fs] save_dir`) plus a directory listing of that
root, or the reporter running **Game Setup → Save data → Import from folder** on the folder they copied.

### H2: Launcher and runtime resolve different save roots (C1/C6)

**Fit:** medium-high. The launcher resolves `<data root>\save`; the runtime uses whatever absolute path is in
`<data root>\play\fefw.ini`, written at build time. Moving or copying `data\` to a new install folder (the
documented way to move an installation), moving a portable copy, or toggling portable mode leaves the ini
pointing at the old absolute path. Play validates the RomFS and overlay, never `save_dir`
(`engine.rs:2250-2284`). The runtime then creates an empty save at the stale path.

**Supporting evidence:** the audit machine has an empty auto-created account save at the runtime's **default**
root, which proves the silent-create path is exercised in practice; the launcher's own crash copy of `fefw.ini`
shows an absolute `save_dir = C:/Users/.../AppData/Roaming/EmblemWeave/save`.

**Would a rebuild fix it?** Usually yes, because a full build rewrites `fefw.ini`. The reporter rebuilt, which
weakens this hypothesis *unless* the rebuild was done before the root moved, reused a carried build, or the
root moved again afterwards.

**What would confirm it:** `game/fefw.ini`'s `[fs] save_dir` differing from the `Save folder` line on
Game Setup, or `fefw.log`'s `save data in '...'` naming a different folder than the launcher shows.

### H3: An empty profile was loaded and the real save is in `backups\`

**Fit:** medium. "Setting up a profile with no luck" is consistent with creating an **empty** profile and
loading it: `load_empty_profile` moves the current account save to `save\backups\<stamp>` and installs an empty
one, which is exactly a game that starts at the prologue. Nothing is lost, but the Saves page now shows a
fresh save and the carousel may show the empty profile as the installed one.

**What would confirm it:** a `save\backups\` entry whose file list matches the missing playthrough; the
bundle would show it only if the reporter listed it (the bundle does not include the save tree today).

### H4: The game ignores the save because the game build differs from the save's build

**Fit:** medium, and it is the only hypothesis that uses the reporter's own **Game data: Warn**. The build
check warns when `main`/module hashes differ from the tested dump (`gamedata.rs:320-360`). A save written by a
different title version can be rejected or ignored by the game itself; the runtime would still mount it and the
launcher would still show it, so the launcher side would look healthy while the game starts fresh.

**What would confirm it:** `report.txt`'s Game build line, `game/fefw.ini`'s romfs/title version, and whether
the save's slots parse (`savefile.rs`), plus a test with the exact tested dump.

### H5: Fixed UID or title ID mismatch, **ruled out**

Proven in §6: the launcher constant, the runtime defaults, the UID serialization and the on-disk folder name
all agree, and the real installation's game-written folder uses that exact name. No code path writes a
conflicting `[account] uid` by default.

### H6: 0.1.41 save-path regression, **ruled out**

`config.rs::save_dir()`, `platform.rs::data_root_from()`, `saves.rs` (except `route_lord` display) and the
runtime's save code are unchanged between `v0.1.40` and `v0.1.41`. The only `saves.rs` behavioural change is
which portrait a slot shows.

---

## 10. Reproduction procedure (safe, non-destructive)

The following is written so it can be run **without** the reporter's data and without touching any live
installation. It uses synthetic fixtures only. It assumes a separate checkout and a separate `data` root
(`EMBLEMWEAVE_DATA` is not a real variable; use a portable copy or a copied `data` folder).

**Fixture protocol (mandatory)**

1. Copy the whole `data\save` tree to a scratch folder. Never operate on the live tree.
2. Write a SHA-256 manifest of every file before the test and after it; the two must match for the source.
3. Use a distinct `--ini` file per run so `[fs] save_dir` points at the fixture.
4. Record the runtime's own line `fsp-srv: … save data in '<path>'` and the `OpenSaveDataFileSystem` result.
5. Do not run while the personal game is running, and do not use the personal build folder.

**R1: Canonical install (control)**
Copy a known-good `account/DEFAULT_USER/000` tree into `<fixture>\save`. Write a minimal ini with
`[fs] save_dir = <fixture>\save`. Start the game with `--ini <fixture>\fefw.ini`. Expected: log names the
fixture root, no "creating the account save data" line, Continue is offered.

**R2: Foreign account folder (H1)**
Same fixture, but rename `account/DEFAULT_USER` to `account/11110000000000002222000000000000`. Expected: log
`account/0100…: no such save data`, then `save data created: account/0100…`; game starts a new game. This
reproduces the report.

**R3: Nested account folder (H1 variant)**
Move `000` to `account/DEFAULT_USER/000/000`. Expected: the volume opens (the `.meta` exists) but the slot
listing is empty; game starts a new game with no "no such save data" line. This distinguishes nesting from a
missing account folder.

**R4: Root mismatch (H2)**
Build/point the ini at `<fixture A>\save`, then copy the whole `data` tree to `<fixture B>` without rewriting
the ini. Launch from B. Expected: the launcher's UI reads `B\save`; the runtime log names `A\save`; a new empty
save appears at A. This is the C1 reproduction.

**R5: Empty-profile backup (H3)**
In a fixture, load an empty profile through the Saves page. Expected: the old tree moves to
`save\backups\<stamp>`; the new account dir has only a `.meta`; the game starts a new game. Restoring the
backup restores the old playthrough.

**R6: `.meta` absent (C8)**
Delete only the sibling `.meta` from a fixture. Expected: "no such save data" then "created"; the `000` files
survive and the game reads them (self-heal). If the game instead starts fresh, C8 is a stronger cause than
this report suggests.

**R7: Game-version mismatch (H4)**. Needs a save from a different title version; out of scope without the
bundle.

---

## 11. Recommended remediation (summary)

The full specification is in `docs/ISSUE_79_REMEDIATION_SPEC.md`. In one sentence: **make the save root a
single, verified value at launch, and never let the runtime create an empty save without saying so.**

Priority order:

1. **Reconcile `[fs] save_dir` at Play** with `Config::save_dir()` using the existing key-preserving ini
   editor (`engine::set_ini_key`, `engine.rs:3797`), and log the resolved value. This removes C1 for every
   future run and costs one file read/write.
2. **Show the effective path, not the intended one.** The Saves page should display the `[fs] save_dir` the
   engine will actually use, and warn when it differs.
3. **Adopt orphaned roots and account folders, backup-first.** Reuse `saves::account_save()` (already resolves
   other account ids) to offer "a save for this game was found in X; install it?". Then copy into
   `account/DEFAULT_USER` via the existing `import()` path, never move or delete the source.
4. **Make the runtime's auto-create loud.** When `ensure_account_save` runs, log at WARN with the full path,
   and add the resolved root to the "no such save data" line. Optionally, when the account folder contains
   `000` with files but no `.meta`, write the meta and log a repair instead of "created".
5. **Put the save tree in the bundle.** Add a names/sizes/hashes listing of the effective save root (not the
   file contents) to `report.zip`, and record `[fs] save_dir` in `report.txt`.

No change to `DEFAULT_USER`, the title id, the `.meta` format, or any existing save is required or permitted.

---

## 12. Regression tests

Launcher-side, all against a temporary directory (no game, no build):

1. `import` from `account/DEFAULT_USER` (canonical) → files at `account/DEFAULT_USER/000`, source unchanged.
2. `import` from a title folder with `000` → no extra nesting.
3. `import` from `account/<foreign-uid>/000` → installed under `DEFAULT_USER`, foreign tree untouched.
4. `import` from the contents of `000` (flat) → installed at `account/DEFAULT_USER/000`.
5. `import` from a folder with `000/Global` and slots → `Global` copied, not validated as a slot.
6. `load_profile_verified` with a deliberately corrupted slot → error, previous save restored byte-for-byte.
7. `load_empty_profile` → previous tree present under `backups/<stamp>`, new tree has `.meta` only.
8. `installed_profile` after a load and after a restart → same answer, and `None` when the account folder is
   replaced by a different save.
9. **New:** `ini_save_dir` reconciliation: given an ini with `save_dir = A` and `Config::save_dir() = B`,
   Play (or the helper) rewrites only that key, preserves comments/order, and the next parse reports B.
10. **New:** root detection: with a populated `A\save` and an empty `B\save`, the launcher offers adoption
    and creates no empty profile/account save before the answer.

Runtime-side (needs a build, so FCC's call): open with a valid tree → no create line; open with a foreign
account id → "no such save data" + create + WARN naming the root; open with `.meta` missing → repair, not
data loss.

Documentation: the troubleshooting guide should state the exact folder the game reads, how to find it from
`Save folder` on Game Setup, and that **Import** must be used instead of copying folders by hand.

---

## 13. What is confirmed, and what is not

**Confirmed by code and by a real installation**

* The launcher and runtime agree on the account UID, its byte order, the `.meta` contract and the title id.
* The runtime creates an empty account save, silently, whenever the computed path has no save.
* `fefw.ini` carries an **absolute** `[fs] save_dir`, written at build time; `play()` passes it with `--ini`
  and never verifies or refreshes it.
* The runtime has a **different** default root (`%APPDATA%\FortunesWeaveRecompPC\save`) from the launcher
  (`%APPDATA%\EmblemWeave\save`), and an empty auto-created save exists at that default root on a real machine.
* 0.1.41 did not change the save path code relative to 0.1.40.

**Not confirmed**

* Which of H1–H4 applies to KalHawke's installation. The bundle that would answer it was never attached.
* Whether the reporter's save is under a foreign account id, under a stale root, in `backups\`, or rejected by
  the game because of the warned game-build mismatch.
* Any reproduction with gameplay: no build, no launch and no save copy was performed, by design and by the
  data-safety rules of this assignment.

**Next action that changes the answer:** ask for `emblemweave-report-1795382099629.zip` (or just
`game/fefw.ini`, `game/fefw.log` and a folder listing of the `Save folder` shown on Game Setup), then run R2,
R3 or R4 against a copy of that tree.
