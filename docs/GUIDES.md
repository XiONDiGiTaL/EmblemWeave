# EmblemWeave guides

These are the task-first guides. The [Using EmblemWeave](USAGE.md) page remains the full page-by-page reference; the guides here answer “how do I do this?” without making you read the whole reference first.

| Guide | Use it for |
| --- | --- |
| [Launcher guide](LAUNCHER_GUIDE.md) | First run, build, Play, saves, mods, cheats, graphics/audio, diagnostics and updates. |
| [F1 menu guide](F1_MENU_GUIDE.md) | The engine's in-game settings overlay, tab by tab, and how it differs from EmblemWeave's F10 menu. |
| [Install and play](INSTALL.md) | Download, portable storage, game-data formats and the build tools. |
| [Using EmblemWeave](USAGE.md) | The rail, every page, the slot editor, display settings and upscaling. |
| [Save recovery](SAVE_RECOVERY_GUIDE.md) | A save folder that is present but not read, profiles and stamped slots. |
| [Troubleshooting](TROUBLESHOOTING.md) | When the build, the game or a setting stops working. |
| [0.1.43 release notes](changelog/0.1.43.md) | What changed in the prepared 0.1.43 release candidate. |

## Two in-game menus

EmblemWeave has two overlays, and they do different jobs:

- **F1** opens the **engine's settings overlay**. It is the fefw-recomp runtime's own menu: graphics, performance, upscaling, pacing, audio, controls, diagnostics and system information. Settings can apply live or at the next start, depending on the setting.
- **F10**, or **Back + RB** on a controller, opens **EmblemWeave's in-game menu**. It pauses the game and offers a screenshot, restart at the title screen, stop and return to the launcher, quick cheats, cheat codes and tables, display settings and the controls in use.

The launcher and the F1 menu share the same `fefw.ini` and `controls.ini` keys. Change a setting in either place and the other reads it back where the engine supports it; see the [F1 menu guide](F1_MENU_GUIDE.md#how-f1-and-the-launcher-share-settings).

## Release material for 0.1.43

The 0.1.43 notes, issue-to-commit evidence and publication checklist are grouped in [release/0.1.43](release/0.1.43/README.md). They are prepared material: **0.1.42 is the current public download until v0.1.43 is published.** The guides do not claim that an open issue is fixed unless the evidence says so.
