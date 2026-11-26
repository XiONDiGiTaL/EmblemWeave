// SPDX-License-Identifier: GPL-3.0-or-later
// Whether a newer release is published, for the overlay's note and the F1 menu: a game started from Steam may never see the launcher,
// which has the same check (launcher/src-tauri/src/update.rs). One request to GitHub's "latest release" per run, on its own thread; a
// failure (offline, rate limit) only shows in the log. Nothing is downloaded or installed here: updating stays the launcher's job.
//
// Only a game that knows its release checks: EmblemWeave's launcher writes [update] version and repo into fefw.ini at every Play (it
// builds the runtime on the player's PC), and a release package may name them at build time (NXRT_RELEASE_VERSION / NXRT_UPDATE_REPO);
// the ini wins. [update] check = 0 turns it off.
#pragma once
#include <string>
#include <string_view>

namespace nxrt::update {

// The release this game is part of ("0.1.43", or a fork's tag): [update] version, else the build's; "" when neither names one.
std::string current_version();
// Starts the request, once per run, when the build names its release and [update] check allows it. Later calls do nothing, so the
// menu may call it again after the setting is turned on. Any thread.
void start();
// The newer release's version once the check has found one and [update] check still allows it, else "". Any thread.
std::string available();

// ---- the parts without the network (tests) ----
// The "tag_name" of GitHub's release JSON without a leading v, "" when there is none or it holds anything but [A-Za-z0-9._+-].
std::string release_version(std::string_view json);
// As the launcher decides it (update.rs check): with by_tag (a fork tagging its releases by commit, FEFW_BUILD_ID) any other release
// is newer, else the first three version numbers decide.
bool is_newer(std::string_view latest, std::string_view current, bool by_tag);

}  // namespace nxrt::update
