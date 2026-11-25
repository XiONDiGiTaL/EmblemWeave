# Funding and intellectual-property audit

**Date:** 2026-11-22
**Scope:** the EmblemWeave repository and the public repositories on the
`XiONDiGiTaL` GitHub account.
**Status:** documentation changes implemented; items marked for review still
need a human or legal decision.

This report records what was inspected, what was changed, and what could not be
established. It is not legal advice and does not claim that any project is
compliant with Nintendo's rights or with any other rights holder's rights.

## Method

1. Read `README.md`, `LICENSE`, `docs/TERMS.md`, `docs/terms.html`,
   `docs/LEGAL.md`, `docs/DMCA.md`, `docs/TROUBLESHOOTING.md`, the About page
   source, the engine's `LEGAL.md`, and the changelog index in EmblemWeave.
2. Searched all Markdown and HTML documentation for donation, monetisation,
   commercial-use, profit and non-commercial language.
3. Enumerated the account's public repositories with the GitHub API and read
   their descriptions, licences and fork status.
4. Ran a GitHub code search for the exact first-party phrase
   `"does not accept donations"` across `user:XiONDiGiTaL`.
5. Checked the tracked file list for ROM, NSP, XCI, NCA, key, ticket and
   firmware files.
6. Built the launcher and its tests after the changes.

Release descriptions, GitHub topics and the repository homepage were read from
metadata but release bodies were not retrieved one by one. No release, tag,
issue, branch or repository metadata was modified.

## Repository inventory

The authenticated account listing returned 33 repositories at the time of
inspection: 23 public and 10 private. Purpose is taken from each repository's
own description. Private repositories were inventoried from their metadata;
their contents were not otherwise audited.

| Repository | Kind | Licence | Purpose |
| --- | --- | --- | --- |
| EmblemWeave | original | GPL-3.0 | Recompilation launcher for *Fire Emblem: Fortune's Weave*, with mod management and enhancements |
| Shizuku | fork | Apache-2.0 | Rebuild of Shizuku (Android privileged API access) |
| MorphePatches | original | none declared | Android patches for Morphe |
| Kryvex | original | MIT | Multi-service desktop connection hub with a browser and workspaces |
| fccGUI | original | AGPL-3.0 | Electron frontend for Free Claude Code (not a Fire Emblem project) |
| XiONDiGiTaL | profile | none | GitHub profile repository |
| Wand-Enhancer | fork | Apache-2.0 | UX and interoperability extension for Wand |
| visualvm | original | none declared | Android visual-voicemail project (unrelated to the Java profiler of the same name) |
| MvC2Online | original | none declared | Browser-based *Marvel vs. Capcom 2* recompilation |
| ProjectTwilight | original | Apache-2.0 | *Ragnarok Redefined* game project |
| mofunctional | original | none declared | Browser game, *MO: FUNCTIONAL* |
| arcade | original | none declared | Game hub for xiondigital.net |
| veylattice | original | NOASSERTION | Original tactical card battler |
| LYNX-SHARE | fork | AGPL-3.0 | C++/JUCE DAW |
| anlasoz | original | none declared | No description provided; not inspected further |
| grease-and-gold-world-tour | original | none declared | 16-bit-style beat-'em-up concept |
| meldra | original | none declared | Browser multiplayer falling-block game |
| xiondigital.github.io | original | NOASSERTION | Artist EPK site |
| Vexium-Core | original | Apache-2.0 | Local model weights, checksums and licence |
| voxelrage | fork | none declared | Voxel beat-'em-up, forked for web hosting |
| kankakombat | original | none declared | Browser co-op beat-'em-up |
| leapbuddies | original | none declared | Browser co-op platformer |
| ux-session | original | none declared | Redirect for Leap Buddies |
| Sohbet | private | none declared | Chat application ("People. Conversation. Nothing else.") |
| AlmaBot | private | MIT | Discord bot modelled on Alma from *Monster Hunter Wilds*, with lobby listings and profiles |
| tandem-reference | private | none declared | APK fixtures for MorphePatches CI; proprietary upstream binaries, not redistributed |
| shattered-marches | private | none declared | 3D voxel tactical RPG in Godot |
| shipwright | private | none declared | Release-preparation assistant |
| clientforge | private | none declared | Project and delivery portal |
| localpilot | private | none declared | Setup and diagnostics for local AI runtimes |
| releaseready | private | none declared | Preflight checklist for independent music releases |
| versevault | private | none declared | Offline workspace for songwriters |
| yenilume | private | none declared | Hosted site for a creative AI assistant (upstream: smnrbck) |

Only **EmblemWeave** is a Nintendo-related project. Two others involve
Capcom-owned intellectual property: `MvC2Online` (a *Marvel vs. Capcom 2*
recompilation) and `AlmaBot` (a bot named after a *Monster Hunter Wilds*
character). `tandem-reference` holds proprietary upstream APK fixtures for CI;
its description says they are never redistributed publicly. The rest are
original works, unrelated tools, or forks of non-Nintendo projects.

### Cross-repository findings

- `"does not accept donations"` appears only in EmblemWeave
  (`docs/TERMS.md` and `docs/terms.html`). No other first-party repository
  contains that claim, so no blanket removal was made elsewhere.
- EmblemWeave has no `.github/FUNDING.yml` and no GitHub Sponsors
  configuration. Its documentation contained no Buy Me a Coffee link before
  this pass; it now has one link in the in-app About page only. The other
  repositories were not checked for a funding file individually.
- Forked repositories carry upstream licences and authorship. They were not
  edited and should not receive a creator-support link without the upstream
  project's situation being reviewed.
- Several repositories have no licence metadata. That is not a funding issue,
  but reuse rights in those repositories are unclear and should be fixed
  separately.

## Documentation changes

| File | Change | Reason |
| --- | --- | --- |
| `docs/TERMS.md` | Replaced "Free software, no monetisation" and the blanket "does not accept donations" with "Free software and independent funding"; added the voluntary-support distinction and the statement that the GPL rights are unchanged | The old wording is no longer accurate once a support link exists, and the new wording separates creator support from the software and the game |
| `docs/terms.html` | Same change in the HTML copy of the terms | The site must not contradict `TERMS.md` |
| `docs/LEGAL.md` | Added "Funding and independence" | States the same facts in the legal overview without adding a licence restriction |
| `docs/TROUBLESHOOTING.md` | Added "An emulator save is not offered" | Documents the yuzu/Eden layout, empty folders, Ryujinx's unsupported index and the folder/ZIP import path (issue #79) |
| `apps/emblemweave/src/pages/about.rs` | Added the text link "Support independent development" next to the XiØN attribution | The requested, non-intrusive support entry point |
| `apps/emblemweave/src/saves.rs` | `survey`/`survey_here` now return importable saves and empty title folders separately | An empty emulator folder is explained instead of being silently ignored |
| `apps/emblemweave/src/pages/setup.rs` | Scans through `survey_here`; lists empty folders with no Import button | Visibility only; nothing is imported from an empty folder |
| `apps/emblemweave/src/pages/saves.rs` | Added `empty_sources` to the save UI state | Carries the empty-folder list to the page |

Not changed:

- `LICENSE` (GPL-3.0-only) and copyright ownership.
- `README.md`: it has a disclaimer and credits but no false funding claim, so
  no donation language needed removal.
- `engine/fefw-recomp/LEGAL.md`: its Nintendo and clean-data statements remain
  accurate and were not weakened.
- `docs/changelog/0.1.34.md`: the historical "no donations" note is a record of
  what was true at that release. It was left as history; the current terms are
  authoritative.
- Every other repository.

## Nintendo intellectual-property findings

1. **Original code.** The Rust launcher and the C++ runtime are maintained in
   this repository as project source. The repository does not vendor a
   Nintendo binary, ROM, NSP, XCI, NCA, key file, ticket or firmware image; the
   tracked-file check found none. Per-file provenance of the runtime is not
   documented, so this audit does not claim that every line is a clean-room
   work.
2. **Original assets.** `assets/branding/logo-*.png` and the launcher's UI art
   are XiØN's own branding. The repository's GPL notice covers its
   code; it does not make Nintendo's material part of the project.
3. **Third-party assets.** The launcher uses third-party Rust crates under
   their own licences; credits are in `README.md`. The project does not
   redistribute game assets, fonts, music or voice data from the game.
4. **Nintendo names and references.** "Fire Emblem", "Fortune's Weave" and
   character names are Nintendo/Intelligent Systems property. They are used to
   identify the game the tool interoperates with. `README.md` and `docs/LEGAL.md`
   state that the project is independent and not affiliated with, endorsed by or
   sponsored by Nintendo. That is a nominative use, not a claim of ownership.
5. **Screenshots.** `docs/screenshots/*.png` show the launcher's own windows;
   some may include game imagery or character art rendered by the game. Game
   imagery remains Nintendo's. The GPL does not license it. This is the clearest
   remaining asset risk in the repository and is flagged for review; replacing
   any screenshot that shows game art with a UI-only capture would remove it.
6. **Reverse engineering and derivative status.** The runtime implements Switch
   services and game behaviour so that code generated from the user's own copy
   can run. Whether that implementation is a derivative work of Nintendo's
   software is a legal question this audit cannot answer. The documentation
   should not assert that it is independently created in every part.
7. **Game data and keys.** `docs/TERMS.md`, `docs/LEGAL.md` and
   `engine/fefw-recomp/LEGAL.md` state that the user must supply their own copy
   and that the project does not provide, decrypt, download or link to ROMs,
   keys or tickets. No tracked key, ticket or ROM file was found and no
   documentation contradicts that claim. The source was not audited line by
   line for decryption routines, so this report does not certify their
   absence.
8. **Mods and cheats.** These features operate on the user's files. The project
   does not distribute mods containing Nintendo assets, but it cannot control
   what users load. That boundary is stated in the terms and is not a licence to
   redistribute game content.

## Monetization assessment

| Project | Assessment |
| --- | --- |
| EmblemWeave | Independently developed and not sold. A general creator-support link can be described accurately only with the distinction now in `TERMS.md` and `LEGAL.md`. Because the app is built around a Nintendo game, a reviewer could still associate a link inside it with the project. Flagged for review; the safer alternative is to keep support links on xiondigital.net and not inside the app. |
| Original, non-Nintendo projects | Can describe themselves as independently supported. No links were added in this pass. |
| Forks (Shizuku, Wand-Enhancer, LYNX-SHARE, voxelrage) | Do not add a creator-support link. Upstream authorship and licences govern; a link would imply support for upstream work. |
| MvC2Online | Do not add a donation link until the Capcom IP position is reviewed. It is a recompilation of a third-party fighting game, the same category of concern as EmblemWeave but with a different rights holder. |
| AlmaBot | Do not add a donation link. It is named after a Capcom character; the same association concern applies. |
| tandem-reference | No funding link. It exists only to hold proprietary third-party fixtures for CI and is private. |
| All other repositories | No funding statement existed and none was added. |

No project is described as tax-exempt, charitable, "DMCA-proof", "legally
protected" or "Nintendo-approved". No paid feature, reward, subscription or
early access was introduced.

## Outstanding risks

1. **Association risk (EmblemWeave).** The link is a plain text link in About,
   opened only on click, with wording and a tooltip that separate creator
   support from the software and the game. It is still inside a Nintendo-related
   application. A human or legal review should decide whether to keep it there
   or move it to the website.
2. **Screenshots.** Any Nintendo game imagery in `docs/screenshots` is not
   licensed by this project. Review and replace if necessary.
3. **Derivative-work question.** Documentation cannot settle whether the
   runtime reimplementation is a derivative work. It now avoids claiming more
   independence than the project can support.
4. **Issue #79 is not confirmed fixed.** The reporter's diagnostic ZIP was never
   attached. The UI now shows empty emulator folders, but the reported save may
   still be under a different Windows account, a different profile, or a
   different game build. The test matrix remains mostly untested.
5. **Ryujinx.** Its numbered save folders are resolved through an index this
   project does not read. Import from an exported folder is the documented
   workaround; native Ryujinx scanning is not implemented.
6. **Licensing gaps in other repositories.** Repositories with no licence file
   have unclear reuse terms. This is outside the funding change but should be
   fixed independently.
7. **No `FUNDING.yml`.** None was added. A GitHub funding file would put a
   sponsor button on the repository itself, which is a stronger association
   than the About link and was not requested. The other repositories were not
   checked for one.

## Verification

- `cargo check -p emblemweave --all-targets` (separate
  `CARGO_TARGET_DIR=target/audit-check`): succeeded. The warnings are
  pre-existing dead-code and style warnings; the two dead-code warnings caused
  by the new survey wrappers were removed with the wrappers.
- `cargo test -p emblemweave emulator`: 3 passed, 0 failed. This covers the
  existing emulator-layout test, the fork-layout test and the new empty-folder
  test.
- `https://buymeacoffee.com/xiondigital` returned HTTP 200 through a browser
  user agent; `https://github.com/XiONDiGiTaL` returned HTTP 200.
- Markdown fence and trailing-whitespace check passed for the changed Markdown
  files.
- No dependency was added; `Cargo.toml` and the lockfile are unchanged.
- The application GUI was not launched in this pass because another build was
  active. The link uses the same `ui.hyperlink_to` path as the existing About
  links, and its URL is the verified one above. It was not clicked in a running
  window.

## Final recommendation

- Keep the EmblemWeave support link only if the distinction wording stays with
  it and a human review accepts the residual association. Otherwise move the
  link to xiondigital.net and remove it from About; the documentation changes
  remain correct either way.
- Do not add a support link to `MvC2Online`, `AlmaBot` or any fork until the
  relevant rights holder and upstream project have been reviewed.
- Review the screenshots and the runtime's provenance before describing the
  project as free of Nintendo material beyond the user's own game copy.
- Treat this report as an inventory and a record of changes, not as a legal
  opinion.
