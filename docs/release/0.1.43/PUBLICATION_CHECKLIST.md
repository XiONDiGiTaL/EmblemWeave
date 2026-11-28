# 0.1.43 publication checklist

> **Do not start this list until the release is explicitly authorized.** It turns the prepared material into a public release. Nothing in this repository has been tagged, published or deployed by preparing it.

## 1. Freeze and verify the candidate

- [ ] Confirm the intended commit (`b9eaf1a5` at the time this material was prepared) is the release commit.
- [ ] Confirm the tracked tree is clean apart from documented untracked work. Do not delete the user's worktrees/build folders.
- [ ] Re-run the launcher, runtime and release-tool suites; record the counts in [the changelog](../../changelog/0.1.43.md).
- [ ] Rebuild the full game with the supported clang-cl configuration; record the `fefw.exe` SHA-256.
- [ ] Build the Windows package with `tools/package_release.ps1`; record the zip, `release-manifest.json` and `SHA256SUMS` SHA-256 values.
- [ ] Extract the zip to a fresh folder and run `--import-save` and `--recover-saves` against a disposable save copy; confirm the source is unchanged.
- [ ] Run the local documentation link audit; resolve every missing local link and bad anchor.
- [ ] Decide and record the disposition of **#79 Report C**. Do not mark it fixed without a route-completed save → restart → Continue test.

## 2. Publish the GitHub release

- [ ] Create the `v0.1.43` tag with the project's normal signing policy.
- [ ] Create the GitHub release titled **EmblemWeave 0.1.43** with the body from [the changelog](../../changelog/0.1.43.md).
- [ ] Upload the Windows zip, `release-manifest.json`, `SHA256SUMS` and any Linux translator asset that was built and verified. Do not invent an asset that was not built.
- [ ] Verify the public asset URLs and run `sha256sum -c SHA256SUMS` against the downloaded files.
- [ ] Confirm the release is the latest public release and that the download is not a private/draft asset.

## 3. Switch the status in one pass

- [ ] `README.md`: change **Current release** from the 0.1.43 release-candidate wording to the published 0.1.43 wording, with the publication date and the release link.
- [ ] `docs/STATUS.md`: change **Current status** to 0.1.43 published; keep the field limitations and the #79 Report C note.
- [ ] `docs/changelog/README.md`: move 0.1.43 from “prepared” to the published row with its date.
- [ ] `docs/changelog/0.1.43.md`: change the status line from release candidate/not published to published, and keep the verification table and known issues.
- [ ] `docs/release.json`: set `version` to `0.1.43`, `tag` to `v0.1.43`, and `download` to the exact published Windows zip URL. The release workflow stamps this file; if it did not, set it by hand and re-check the site.
- [ ] `docs/index.html`: static fallback version/badge and download link must be the published release (or the always-valid releases page); `site.js` then replaces them from `release.json`/the GitHub API.
- [ ] `docs/release/0.1.43/README.md`: replace “prepared, not published” with the publication date and asset link.
- [ ] Remove any “0.1.42 is current” or “release candidate” wording that remains in the public pages.

## 4. Site, fallback and mirrors

- [ ] Deploy the updated `docs/` site through the normal host (GitHub Pages or the configured host). This is a separate, authorized deployment; a git push alone does not prove the fallback was redeployed.
- [ ] Open the live site and the fallback URL; check the hero download, the badge, the Guides/Docs links, the screenshots and the legal links.
- [ ] Confirm `release.json` is served with the published version and the download returns the package.
- [ ] Push the release commit to GitHub, GitLab and GitGud. Do not push to the `metrist` upstream.
- [ ] Verify each mirror's `main` resolves to the release commit; a mirror's static site, if it has one, is redeployed separately.
- [ ] Confirm the fallback page is either redeployed or clearly marked as a static copy of the current site.

## 5. Issue replies (manual, after the release is live)

- [ ] Use only the private reply templates prepared for this release; do not post them early.
- [ ] **#115:** the confirmed code/build fix may be announced, with the SDK build evidence and a request for the reporter's confirmation. Do not close the issue on the maintainer's behalf unless the project's normal policy says so.
- [ ] **#117/#76:** ask the reporter to retest; describe the change as a candidate/verification request, not a confirmed fix.
- [ ] **#79:** do not claim Report C is fixed. Ask for the route-completed save and the diagnostic ZIP; keep the issue open.
- [ ] **#116/#37/#41/#29/#70/#75/#59/#7:** do not attach a “fixed” reply. Point to the known limitation or the specific evidence request.
- [ ] Record the issue numbers and dates in the release handoff. No automated issue edits.

## 6. Post-publication verification

- [ ] Download the zip from the public URL and verify its SHA-256 against `SHA256SUMS`.
- [ ] Extract it to a clean folder; run `EmblemWeave.exe --verify-tools` and confirm the fresh-package message, then `--fetch-tools` if testing the toolchain path.
- [ ] Run a first launch, choose a dump (or the test fixture), build, Play, save, restart and Continue on a disposable install. Do not use an existing user's live save folder.
- [ ] Re-check the site, fallback and both mirrors after DNS/CDN caches settle.
- [ ] If any check fails, stop and use the rollback below rather than editing the published asset in place.

## 7. Rollback

- [ ] Unpublish the GitHub release or mark it as a pre-release/draft; keep the tag only if the project intends to keep the commit reference.
- [ ] Restore `docs/release.json`, `docs/index.html` and the README/STATUS status lines to the previous public release.
- [ ] Redeploy the site/fallback from the restored source.
- [ ] Tell the mirrors and the issue thread that the release was withdrawn; do not delete user reports.
- [ ] Record the reason in the release handoff.

## Evidence to attach to the release handoff

- Release commit and tag.
- Test counts and build configuration.
- `fefw.exe` SHA-256.
- Zip, release-manifest and `SHA256SUMS` SHA-256 values.
- Clean-extraction import/recovery result.
- Link-audit result.
- Site/fallback/mirror verification timestamps.
- Issue replies posted (numbers and dates).
