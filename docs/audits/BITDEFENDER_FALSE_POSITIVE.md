# Bitdefender behavioral detection on Rebuild (issue #116)

**Reported:** 2026-11-27, EmblemWeave 0.1.42, after an in-place update and the Rebuild button.
**Detection:** Bitdefender Advanced Threat Defense, `SuspiciousBehavior.E2686D3D4EC398E0`.
**Reported chain:** `cmd.exe -> cmake.exe -> ninja.exe`.

This document records what was established, what was implemented, and what could not be tested. It is not a claim
that the detection is eliminated.

## 1. Evidence available

| Item | Status |
| --- | --- |
| Issue #116 report and the Advanced Threat Defense screenshot | Available |
| Bitdefender version and virus-definition build | Not in the report; unavailable |
| Quarantine entries with exact file paths | Not available |
| Bitdefender event log or Advanced Threat Defense score breakdown | Not available |
| Event reproduction on a Bitdefender-protected machine | Not performed; this workstation does not run Bitdefender |

The screenshot shows the detection name and the blocked process chain. It does not show which file was scored or
which files were removed, so the specific file and the exact score inputs are unknown.

## 2. What the reported chain actually is

The chain matches the launcher's documented local rebuild, not the updater:

1. Overview's **Build the game** starts `JobKind::Engine` (`pages/overview.rs`, `jobs.rs`).
2. `engine::build` validates the carried toolchain, resolves Visual Studio and the Windows SDK, and starts each stage
   with its own logged command (`engine.rs`).
3. The C++ stage runs `cmd /C recomp\scripts\build_game.cmd fefw` from the engine root.
4. The final executable stage runs `pwsh -NoProfile -ExecutionPolicy Bypass -File game-build\scripts\build_fefw.ps1`.
5. Those scripts configure with CMake, build with Ninja, compile with `clang-cl.exe`, and link with `lld-link.exe`.

That is exactly a `cmd.exe -> cmake.exe -> ninja.exe` tree followed by compiler and linker children. The launcher logs
the exact command and streams each child's stdout/stderr into the build log; console windows are suppressed with
`CREATE_NO_WINDOW`, which predates 0.1.41 and is not new in 0.1.42.

The **updater** runs a different chain: it downloads the release zip, verifies its SHA-256 against the release asset
digest or the release notes' `sha256 of the zip`, unpacks it, runs a short temporary `.cmd` that waits for the running
launcher to exit, replaces `EmblemWeave.exe`, and restarts it (`updater.rs`). No CMake or Ninja is involved.

## 3. Regression window

The portable toolchain (clang-cl, CMake, Ninja and Python downloaded into `engine/fefw-recomp/tools/bin`) was added in
`387ff603` on 2026-11-17, before v0.1.41 (`a56d1e2`, 2026-11-20) and v0.1.42 (`a3b1fa9d`, 2026-11-25). v0.1.41 already
set `CREATE_NO_WINDOW` on the build command in `run_env`; `1a827515` (in the 0.1.42 range) extended console suppression
to tool probes and the ReShade setup, not to the build tree.

No source change between v0.1.41 and v0.1.42 was found that turns the rebuild into a different kind of process tree.
A behavioral score can change with the antivirus definitions or the reputation of a newly downloaded, unsigned
archive without any change in the program. The available evidence cannot distinguish those cases.

## 4. Classification

- **Reproduced detection:** no. No Bitdefender endpoint was available, and the original quarantine evidence is gone.
- **Confirmed triggering behavior:** the launcher's Rebuild runs `cmd -> CMake -> Ninja -> clang-cl/lld-link` and
  produces a new unsigned `fefw.exe`. This is by design and is what the report describes.
- **Suspected triggering behavior:** a heuristic score on an unsigned, freshly downloaded toolchain compiling and
  linking an executable shortly after an in-place update. This is a hypothesis, not a confirmed cause.
- **Implemented mitigation:** carried-toolchain integrity and explicit recovery (section 5). This reduces silent
  execution of a damaged toolchain and makes repair visible; it does not change the process tree.
- **Independently verified antivirus result:** none. No claim is made that the false positive is gone.

## 5. Implemented integrity and recovery model

`apps/emblemweave/src/portable.rs` now keeps an explicit record of what was installed and checks it:

- Every archive URL must be `https://` and every archive has a pinned SHA-256; a download is refused unless it matches.
- After unpacking, every required file is enumerated: the piece's executable, its `keep` files, and every file inside
  a `keep` folder (clang's resource headers). A required file the archive did not produce fails the install.
- A `.carried-<piece>.manifest` records `sha256 size relative/path` for those files before the `.carried-<piece>` stamp
  is written. The stamp alone is no longer treated as proof that the files are intact.
- The Overview uses a shallow check (presence, non-empty, recorded size) so it stays cheap enough to run while drawing.
- The build gate and the new `EmblemWeave.exe --verify-tools` re-hash every required file, including files inside the
  resource tree. A same-size substitution that passes the shallow check is caught there.
- A missing, empty, truncated or substituted file is named with its path and the way back: **Repair build tools** on
  Overview, restore from quarantine, or `EmblemWeave.exe --fetch-tools`.
- `--fetch-tools` now re-fetches all four pieces even when their stamps are present, so an install from before the
  manifest existed can be replaced with a verified one. `--verify-tools` exits 0 only when every carried piece matches.
- Repair downloads only the damaged pieces; a whole toolchain returns without touching the network.

These changes do not rename binaries, hide processes differently, disable security software, or add exclusions. They
make legitimate installs verifiable and repairs explicit.

## 6. Why the build process was not rewritten

`build_game.cmd` and `build_fefw.ps1` set up the engine's environment and drive a 634-target CMake/Ninja build. Moving
that orchestration into Rust would remove the `cmd`/PowerShell layers but would replace tested, engine-owned scripts
with a new build driver, which is a larger correctness risk than the behavior it removes. The launcher already logs
the exact command and the full child output, which is the transparency this workflow can provide without changing it.

## 7. Residual risk and next steps

- The launcher and the carried tools are unsigned. Code signing is the strongest available mitigation for reputation-
  based behavioral scoring; it requires a certificate and is not implemented here.
- A Bitdefender false-positive submission should include the detection ID, the build log, and the exact files. Without
  a protected test machine the score cannot be reproduced or shown to be cleared.
- If the detection recurs, the useful evidence is the Advanced Threat Defense notification with the scored file path
  and the Bitdefender version/definitions, not only the detection name.
- Do not recommend disabling Bitdefender or adding exclusions as a fix. Recovery is restore-from-quarantine plus a
  verified `--fetch-tools`/Repair, and the false positive belongs with the vendor.
