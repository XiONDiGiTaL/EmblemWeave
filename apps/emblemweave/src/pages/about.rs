//! About: the project, its author, its build, its updates and its legal
//! position.

use crate::theme;
use crate::widgets::{self, Tone};
use crate::{logo, updater, App, UpdateState};
use eframe::egui::{self, RichText};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    ui.add_space(theme::SPACE_L);
    ui.vertical_centered(|ui| {
        logo::show_lockup(ui, &p, 460.0_f32.min(ui.available_width()));
        ui.label(RichText::new("A native PC recompilation of Fire Emblem: Fortune's Weave").size(15.0).color(p.body));
        ui.horizontal(|ui| {
            ui.label(RichText::new("by").color(p.muted));
            ui.hyperlink_to(RichText::new("XiØN").font(theme::semibold(15.0)).color(p.accent_hi), "https://github.com/XiONDiGiTaL");
        });
    });
    ui.add_space(theme::SPACE_L);

    updates(app, ui, &p);

    ui.columns(2, |cols| {
        widgets::section(&mut cols[0], &p, "Build", |ui| {
            widgets::kv(ui, &p, "Version", env!("CARGO_PKG_VERSION"));
            widgets::kv(ui, &p, "Revision", env!("EW_GIT_REV"));
            widgets::kv(ui, &p, "Target", &format!("{} {}", std::env::consts::OS, std::env::consts::ARCH));
            widgets::kv(ui, &p, "Profile", if cfg!(debug_assertions) { "debug" } else { "release" });
            widgets::kv(ui, &p, "Runtime", "EmblemWeave (in development)");
            widgets::kv(ui, &p, "Baseline", "fefw-recomp, for play today");
            ui.add_space(theme::SPACE_S);
            ui.hyperlink_to("github.com/XiONDiGiTaL/EmblemWeave", "https://github.com/XiONDiGiTaL/EmblemWeave");
            ui.hyperlink_to("xiondigital.net", "https://xiondigital.net/arcade/");
        });
        widgets::section(&mut cols[1], &p, "Credits", |ui| {
            for (who, what) in [
                ("XiØN", "design, engineering and the EmblemWeave identity"),
                ("Goldoire", "fefw-recomp, the baseline engine, and everyone who contributed to that project"),
                ("Obersteinn", "preserving the final fefw-recomp 0.2.0 release after its repository was removed"),
                ("Ryujinx", "shader translation, whose shader code the build uses (MIT)"),
                ("egui / eframe", "the launcher interface (MIT / Apache-2.0)"),
                ("roxmltree, windows-sys", "Cheat Engine tables, game memory and controller access (MIT / Apache-2.0)"),
                ("GameBanana", "hosts the community mods the Mods page installs; each belongs to its author"),
                ("libm", "musl's math library, ported to Rust (MIT)"),
                ("lz4_flex, ruzstd", "module and asset decompression (MIT / Apache-2.0)"),
                ("Unicorn", "independent AArch64 oracle used in development"),
                ("zbic", "the ZBIC development bridge"),
            ] {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(who).font(theme::semibold(13.5)).color(p.ink));
                    ui.label(RichText::new(what).size(13.0).color(p.body));
                });
            }
        });
    });

    widgets::section(ui, &p, "Legal", |ui| {
        ui.label(
            RichText::new(
                "EmblemWeave is an independent project by XiØN. It is not affiliated with, endorsed by, or supported by \
                 Nintendo or Intelligent Systems. It contains no game code, no game assets, no firmware and no keys, and it \
                 downloads none. You must supply data from your own legally obtained copy of the game; EmblemWeave only \
                 reads it. All trademarks belong to their respective owners.",
            )
            .size(13.0)
            .color(p.body),
        );
    });
}

/// Checking GitHub, installing a release in place, and going back a version.
fn updates(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    widgets::section(ui, p, "Updates", |ui| {
        widgets::kv(ui, p, "This build", env!("CARGO_PKG_VERSION"));
        let mut check = app.config.updates.check;
        ui.horizontal(|ui| {
            if widgets::toggle(ui, p, &mut check).changed() {
                app.config.updates.check = check;
            }
            ui.label(RichText::new("Check for updates when the launcher starts").color(p.body));
        });
        ui.add_space(theme::SPACE_S);

        let busy = matches!(app.update.state, UpdateState::Checking | UpdateState::Downloading);
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, p, "Check for updates", !busy).clicked() {
                app.check_updates();
            }
            if updater::has_backup()
                && widgets::button(ui, p, "Restore the previous version", !busy)
                    .on_hover_text("Puts back the build the last update replaced, and restarts")
                    .clicked()
            {
                match updater::restore_previous() {
                    Ok(()) => app.update.restart = true,
                    Err(e) => app.notify(e, Tone::Bad),
                }
            }
            ui.hyperlink_to("All releases", format!("https://github.com/{}/releases", updater::REPO));
        });
        ui.add_space(theme::SPACE_S);

        match app.update.state.clone() {
            UpdateState::Idle => {
                ui.label(RichText::new("The launcher checks GitHub for a newer release and installs it itself.").size(12.5).color(p.muted));
            }
            UpdateState::Checking => {
                widgets::status_line(ui, p, Tone::Neutral, "Checking GitHub…", "The newest published release is looked up; nothing is downloaded yet.");
            }
            UpdateState::UpToDate(tag) => {
                widgets::status_line(ui, p, Tone::Ok, "You are on the newest release", &format!("{tag} is the newest published release."));
            }
            UpdateState::Failed(e) => {
                widgets::status_line(ui, p, Tone::Bad, "The update check failed", &format!("{e}. Nothing was changed."));
            }
            UpdateState::Downloading => {
                widgets::status_line(
                    ui,
                    p,
                    Tone::Accent,
                    "Downloading the new version…",
                    "It is unpacked beside this one; the launcher then closes, replaces itself and starts again.",
                );
                ui.add_space(4.0);
                ui.spinner();
            }
            UpdateState::Ready(rel) => {
                widgets::status_line(
                    ui,
                    p,
                    Tone::Accent,
                    &format!("{} is available", rel.tag),
                    &format!(
                        "This build is {}. Downloading installs it in place: the launcher closes, replaces itself and starts again.",
                        env!("CARGO_PKG_VERSION")
                    ),
                );
                ui.add_space(theme::SPACE_S);
                ui.horizontal_wrapped(|ui| {
                    if widgets::primary_button(ui, p, "Download and install", rel.asset.is_some()).clicked() {
                        app.install_update();
                    }
                    if !rel.page.is_empty() {
                        ui.hyperlink_to("Release notes", &rel.page);
                    }
                    if widgets::button(ui, p, "Skip this version", true).clicked() {
                        app.config.updates.skipped = rel.tag.clone();
                        app.update.state = UpdateState::Idle;
                    }
                });
                if rel.asset.is_none() {
                    ui.label(RichText::new("That release has no Windows download; use the release page.").size(12.0).color(p.warn));
                }
                if !rel.notes.is_empty() {
                    ui.add_space(theme::SPACE_S);
                    egui::ScrollArea::vertical().id_salt("release-notes").max_height(crate::widgets::list_height(ui, 0.22)).show(ui, |ui| {
                        ui.label(RichText::new(&rel.notes).size(12.5).color(p.body));
                    });
                }
            }
        }
    });
}
