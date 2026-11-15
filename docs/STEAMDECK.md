# Running on a Steam Deck

EmblemWeave is a Windows program. The Steam Deck runs it under Proton, the same way it runs any Windows game — but one step
Proton cannot do for you is **the build**. Building needs `winget`, PowerShell 7 and Visual Studio's C++ compiler, and the
launcher looks for them in their Windows folders, so a Deck cannot build the game. Build it once on a Windows PC, then copy
the result over.

This page has not been run on a Deck by the maintainer. The engine's own configuration carries a SteamOS path — `[memory]
heap_section = auto` switches to private memory under Wine because with a shared section "no memory pool reached the GPU
(every draw read the dummy buffer: a black picture)", per `runtime/docs/CONFIG.md` — and a deck report in
[issue #7](https://github.com/XiONDiGiTaL/EmblemWeave/issues/7) says a recompiled game of this kind runs under Proton. Treat
the steps as a route that should work, not as a tested recipe, and see [What to send back](#what-to-send-back).

## 1. Build on a Windows PC and copy the whole folder

On the Windows machine: **Game Setup** → choose your dump → **Build the game** → **Play** once to confirm the game runs.

Then copy, as they are:

| What | Why it is needed |
| --- | --- |
| The whole folder holding `EmblemWeave.exe` and `engine\` | `fefw.exe` is at `engine\fefw-recomp\game-build\fefw\build\release\`, and the translated shaders from the build are under `engine\fefw-recomp\work\` (`shaders\spirv`, `bc7_cache`). |
| `%APPDATA%\EmblemWeave\play\` | The `fefw.ini` the build wrote, plus the log. |
| `%APPDATA%\EmblemWeave\save\` | Your saves, if you want them on the Deck. |

The dump goes too, unless it is already on the Deck. exFAT on a USB stick is the easiest carrier.

## 2. Put it on the Deck

Desktop Mode, and either copy the folder straight across, or — if you are starting from the release zip — extract it and
then move your Windows-built `engine\` into place:

```sh
mkdir -p ~/EmblemWeave && unzip -q ~/Downloads/EmblemWeave-v*-windows-x64.zip -d ~/EmblemWeave
sha256sum ~/Downloads/EmblemWeave-v*-windows-x64.zip   # compare with the release notes
```

Every entry in the zip uses `/`, so the tree comes out intact and the launcher lands at
`~/EmblemWeave/EmblemWeave-v<version>/EmblemWeave.exe`. The build output must sit inside that same tree, because the
launcher resolves the engine relative to its own folder (`EW_ENGINE` overrides that).

Put the dump somewhere stable, for example `~/Games/fefw-dump/` with `exefs/`, `romfs/` and `control.nacp` inside.

## 3. Add the launcher to Steam

1. Steam → **Games** → **Add a Non-Steam Game** → **Browse** → `EmblemWeave.exe`.
2. Library → right-click it → **Properties** → **Compatibility** → tick **Force the use of a specific Steam Play
   compatibility tool** → **Proton Experimental** (a recent GE-Proton works too).
3. Launch it once and close it. That creates the Proton prefix — the fake `C:` drive both programs see.

Do the setup below in **Desktop Mode**: Game Setup's file picker and the diagnostics export use Windows dialogs, which are
awkward on a touchscreen. Playing itself is fine from Game Mode.

## 4. Find the prefix and put `fefw.ini` in it

```sh
ls -t ~/.steam/steam/steamapps/compatdata/ | head -1        # newest prefix = the app you just added
PREFIX=~/.steam/steam/steamapps/compatdata/<that-id>/pfx
ls "$PREFIX/drive_c/users/steamuser/AppData/Roaming/"
```

Inside the prefix, `%APPDATA%` is `$PREFIX/drive_c/users/steamuser/AppData/Roaming`, so the launcher's settings and the
game's run folder live at `…/Roaming/EmblemWeave/play/`. Copy the `play` and `save` folders from Windows into
`…/Roaming/EmblemWeave/`. Without `play/fefw.ini` the launcher says *the game is not built yet* — it checks for both
`fefw.exe` and `fefw.ini`.

## 5. Re-point `fefw.ini`

`fefw.ini` carries the Windows paths of the machine that built it. Proton maps `Z:` to the real root, and `C:` to the
prefix itself, so a dump at `~/Games/fefw-dump` and a tree at `~/EmblemWeave/EmblemWeave-v0.1.11` become:

```ini
[log]
path = fefw.log

[fs]
exefs = Z:/home/deck/Games/fefw-dump/exefs
romfs = Z:/home/deck/Games/fefw-dump/romfs
save_dir = C:/users/steamuser/AppData/Roaming/EmblemWeave/save
mods = mods

[nvn]
shader_cache = Z:/home/deck/EmblemWeave/EmblemWeave-v0.1.11/engine/fefw-recomp/work/shaders/spirv
bc7_cache = Z:/home/deck/EmblemWeave/EmblemWeave-v0.1.11/engine/fefw-recomp/work/bc7_cache
```

The launcher itself writes these paths with forward slashes, and the runtime accepts them that way, so keep `/` rather than
`\`. The About page in the F1 menu prints the paths the runtime actually resolved — the quickest way to spot a typo.

Any key can also be given as an environment variable instead of editing the file: `NXRT_` plus the upper-case section, an
underscore, and the upper-case key. Steam → Properties → **Launch Options**:

```
NXRT_FS_EXEFS=Z:/home/deck/Games/fefw-dump/exefs NXRT_FS_ROMFS=Z:/home/deck/Games/fefw-dump/romfs %command%
```

Environment variables win over the ini; the rule and the full key list are in `runtime/docs/CONFIG.md`.

## 6. Play

Press **Play**. The launcher writes the Graphics and Audio settings into `play/fefw.ini`, starts `fefw.exe` in the prefix and
watches the first frames. The log is `…/EmblemWeave/play/fefw.log`.

## What does not work on the Deck

- **Build the game** and **Install the missing build tools.** Both drive `winget` and the Visual Studio installer. The
  message you get is a missing program, not a broken download.
- **Recompilation page edits.** They need the dump and a C++ compiler; do them on Windows.

## If it does not start

| What you see | What to do |
| --- | --- |
| *the game is not built yet* | `play/fefw.ini` is missing from the prefix, or `engine/fefw-recomp/game-build/fefw/build/release/fefw.exe` is missing from the copied tree. |
| The window opens and stays black | Read `play/fefw.log`: it names the GPU it chose and the pipelines it built. Diagnostics → Export collects it too. |
| Complaints about a missing shader or a missing `spirv_harness` | The tree came from a launcher older than 0.1.11, or `engine/fefw-recomp/work/shaders/spirv` did not come across. Copy it from the Windows machine, or start from a current release zip and move the build over. |
| The launcher's window is tiny or blurry | Game Mode scaling; open it in Desktop Mode. |
| A Vulkan error or a crash on start | Note the Proton version — the Deck uses RADV — and include `play/fefw.log`. |

## What to send back

Proton version, whether `fefw.exe` was built on Windows, the contents of `play/fefw.log` (or the Diagnostics export), and
where it stopped. That is enough to turn this page into a tested recipe, or to fix what it gets wrong.
