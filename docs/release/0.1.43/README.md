# EmblemWeave 0.1.43 release material

> **Prepared, not published.** 0.1.42 is the current public release. No `v0.1.43` tag or GitHub release exists yet. Do not post issue replies or flip the public release status until the publication checklist has been completed.

This folder groups the release-facing material that is ready for the 0.1.43 publication step:

| Document | What it is |
| --- | --- |
| [0.1.43 changelog](../../changelog/0.1.43.md) | The user-facing release notes and the verification table. |
| [Issue map](ISSUE_MAP.md) | What 0.1.43 changes for each open issue, with commit references and an explicit status. |
| [Publication checklist](PUBLICATION_CHECKLIST.md) | The ordered steps for tag, release assets, site, mirrors, fallback and issue replies. |
| [Launcher guide](../../LAUNCHER_GUIDE.md) | First run, build, play, saves, mods and updates. |
| [F1 menu guide](../../F1_MENU_GUIDE.md) | The engine's settings overlay, tab by tab. |
| [Save recovery](../../SAVE_RECOVERY_GUIDE.md) | Adopting an orphan save, profiles and stamped slots. |

## What is and is not confirmed

- **#115** (Windows SDK build failure) is the only issue this material treats as a confirmed code/build fix: the 0.1.43 candidate builds with Windows SDK 10.0.19041.0 and 10.0.28000.0, 634/634 targets each, and links `fefw.exe`.
- **#117** (Obelisk Chamber freeze) has the single-lock fix integrated and passed an isolated live retest for the freeze, but the in-game character switch was not visually confirmed and the issue is not reporter-confirmed.
- **#79 Report C** (route completion) is **not fixed or confirmed**. The save-discovery, stamped-slot, locking and atomic-commit improvements are real and tested, but a route-completed save → restart → Continue cycle is still missing.
- **#76**, **#77**, **#59**, **#70/#75**, **#37/#41**, **#29** and **#7** have partial, unconfirmed or unsupported status as recorded in the issue map. Do not describe them as fixed.

The issue map is traceability, not a promise. No issue was commented on, closed or relabelled while preparing this material.

## Publication status

The release becomes public only after the tag and GitHub release exist and the asset checksums are verified. The README, `docs/STATUS.md`, `docs/changelog/README.md`, `docs/release.json` and the site's static fallback must be switched in the same publication step. Until then, the correct public statement is: **0.1.42 is the current public release; 0.1.43 is a prepared release candidate.**

See [PUBLICATION_CHECKLIST.md](PUBLICATION_CHECKLIST.md) for the exact order.
