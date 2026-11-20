//! About: the project, its author, its build, its updates and its legal
//! position.

use crate::theme;
use crate::widgets::{self, Tone};
use crate::{logo, updater, App, UpdateState};
use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self};
use std::time::Duration;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    ui.add_space(theme::SPACE_L);
    ui.vertical_centered(|ui| {
        logo::show_lockup(ui, &p, 460.0_f32.min(ui.available_width()));
        ui.label(
            crate::i18n::text("A native PC recompilation of Fire Emblem: Fortune's Weave")
                .size(15.0)
                .color(p.body),
        );
        ui.horizontal(|ui| {
            ui.label(crate::i18n::text("by").color(p.muted));
            ui.hyperlink_to(
                crate::i18n::text("XiØN")
                    .font(theme::semibold(15.0))
                    .color(p.accent_hi),
                "https://github.com/XiONDiGiTaL",
            );
        });
    });
    ui.add_space(theme::SPACE_L);

    links(ui, &p);
    credits(ui, &p);
    updates(app, ui, &p);
    discord(app, ui, &p);
    storage(app, ui, &p);

    ui.columns(2, |cols| {
        widgets::section(&mut cols[0], &p, "Build", |ui| {
            widgets::kv(ui, &p, "Version", env!("CARGO_PKG_VERSION"));
            widgets::kv(ui, &p, "Revision", env!("EW_GIT_REV"));
            widgets::kv(ui, &p, "Target", &format!("{} {}", std::env::consts::OS, std::env::consts::ARCH));
            widgets::kv(ui, &p, "Profile", if cfg!(debug_assertions) { "debug" } else { "release" });
            widgets::kv(ui, &p, "Runtime", "EmblemWeave (in development)");
            widgets::kv(ui, &p, "Baseline", "fefw-recomp, for play today");
        });
        widgets::section(&mut cols[1], &p, "Project", |ui| {
            ui.label(
                crate::i18n::text(
                    "EmblemWeave is an independent project by XiØN. The source and every release live on GitHub; the other \
                     links above are mirrors and a fallback for when the main site or GitHub is unavailable.",
                )
                .size(13.0)
                .color(p.body),
            );
            ui.add_space(theme::SPACE_S);
            widgets::kv(ui, &p, "Site", "xiondigital.net/EmblemWeave");
            widgets::kv(ui, &p, "Mirrors", "GitHub, GitLab, GitGud");
            widgets::kv(ui, &p, "Fallback", "emblemweave.xiondigital.workers.dev");
            ui.add_space(theme::SPACE_S);
            ui.label(
                crate::i18n::text("The tiles at the top of this page open each one in your browser. No account is needed to read the mirror or the fallback.")
                    .size(12.5)
                    .color(p.muted),
            );
        });
    });

    widgets::section(ui, &p, "Legal", |ui| {
        ui.label(
            crate::i18n::text(
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

/// The canonical site and its mirrors, as one row of brand-coloured tiles. The
/// old page linked only GitHub and the arcade site; the mirror and fallback are
/// first-class links now, because the main host can be blocked.
fn links(ui: &mut egui::Ui, p: &theme::Palette) {
    widgets::section(ui, p, "Links", |ui| {
        ui.label(
            crate::i18n::text("The project site, the source, its mirrors, and a fallback if the domain cannot be reached.")
                .size(13.0)
                .color(p.body),
        );
        ui.add_space(theme::SPACE_S);
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        ui.horizontal_wrapped(|ui| {
            link_tile(
                ui,
                p,
                "◆",
                "Website",
                "project site",
                "https://xiondigital.net/EmblemWeave/",
                p.accent_hi,
            );
            link_tile(
                ui,
                p,
                "GH",
                "GitHub",
                "source + releases",
                "https://github.com/XiONDiGiTaL/EmblemWeave",
                egui::Color32::from_rgb(226, 232, 240),
            );
            link_tile(
                ui,
                p,
                "GL",
                "GitLab",
                "mirror",
                "https://gitlab.com/XiONDiGiTaL/emblemweave",
                egui::Color32::from_rgb(252, 109, 38),
            );
            link_tile(
                ui,
                p,
                "GG",
                "GitGud",
                "mirror",
                "https://gitgud.io/xiondigital/EmblemWeave",
                egui::Color32::from_rgb(132, 96, 255),
            );
            link_tile(
                ui,
                p,
                "↗",
                "Fallback",
                "if the domain is blocked",
                "https://emblemweave.xiondigital.workers.dev",
                p.warn,
            );
        });
        ui.add_space(theme::SPACE_XS);
        ui.label(
            crate::i18n::text("Each tile opens in your browser. The mirrors carry the same source and releases, and none of them need an account.")
                .size(12.0)
                .color(p.muted),
        );
    });
}

fn link_tile(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    glyph: &str,
    name: &str,
    detail: &str,
    url: &str,
    tint: egui::Color32,
) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(112.0, 88.0), egui::Sense::click());
    let hover = ui
        .ctx()
        .animate_bool(response.id.with("hover"), response.hovered());
    let tile = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + 25.0),
        egui::vec2(50.0, 50.0),
    );
    let radius = egui::CornerRadius::same(13);
    let painter = ui.painter();

    if hover > 0.04 {
        painter.rect_filled(
            tile.expand(4.0),
            egui::CornerRadius::same(17),
            theme::alpha(tint, (22.0 * hover) as u8),
        );
    }
    painter.rect_filled(tile, radius, p.bg_deep);
    widgets::gradient_fill(
        painter,
        tile.shrink(1.0),
        12.0,
        theme::alpha(tint, (42.0 + 20.0 * hover) as u8),
        egui::Color32::TRANSPARENT,
    );
    painter.rect_stroke(
        tile,
        radius,
        egui::Stroke::new(1.0_f32, theme::alpha(tint, (92.0 + 90.0 * hover) as u8)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        tile.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        theme::semibold(if glyph.len() > 1 { 13.0 } else { 18.0 }),
        tint,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.top() + 57.0),
        egui::Align2::CENTER_TOP,
        name,
        theme::semibold(11.5),
        if hover > 0.5 { p.ink } else { p.body },
    );
    painter.text(
        egui::pos2(rect.center().x, rect.top() + 72.0),
        egui::Align2::CENTER_TOP,
        detail,
        egui::FontId::proportional(9.5),
        p.muted,
    );

    if response.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
    response.on_hover_text(url);
}

/// A slow, continuous credits strip. It is deliberately one laid-out line
/// rather than a list: the page should feel like the project has a pulse, not
/// like a legal appendix. Hovering pauses it so a name can be read.
fn credits(ui: &mut egui::Ui, p: &theme::Palette) {
    widgets::section(ui, p, "Credits", |ui| {
        ui.label(
            crate::i18n::text(
                "A project is never one person. This is everyone whose work EmblemWeave stands on.",
            )
            .size(13.0)
            .color(p.body),
        );
        ui.add_space(theme::SPACE_S);

        let entries = [
            ("XiØN", "design, engineering and the EmblemWeave identity"),
            ("Goldoire", "fefw-recomp, the baseline engine, and everyone who contributed to that project"),
            ("Obersteinn", "preserving the final fefw-recomp 0.2.0 release after its repository was removed"),
            ("moistman7", "the GT tone-curve HDR patch and the 0.2.3/0.2.4 engine improvements (PRs #40, #52-#55)"),
            ("metrist38", "the fefw-recomp fork that supplies the 0.2.3 and 0.2.4 engine updates"),
            ("xinthefreefallxai / ObeliskChamber", "another fefw-recomp fork with updates beyond 0.2.0; we support others extending the original's life"),
            ("Shader translator", "SPIR-V shader translation (MIT)"),
            ("egui / eframe", "the launcher interface (MIT / Apache-2.0)"),
            ("roxmltree, windows-sys", "Cheat Engine tables, game memory and controller access (MIT / Apache-2.0)"),
            ("GameBanana", "hosts the community mods the Mods page installs; each belongs to its author"),
            ("libm", "musl's math library, ported to Rust (MIT)"),
            ("lz4_flex, ruzstd", "module and asset decompression (MIT / Apache-2.0)"),
            ("Unicorn", "independent AArch64 oracle used in development"),
            ("zbic", "the ZBIC development bridge"),
        ];
        let mut job = LayoutJob::default();
        job.wrap.max_width = f32::INFINITY;
        for (i, (who, what)) in entries.iter().enumerate() {
            if i > 0 {
                job.append(
                    "     ◆     ",
                    0.0,
                    TextFormat {
                        font_id: theme::semibold(10.0),
                        color: p.muted,
                        ..Default::default()
                    },
                );
            }
            job.append(
                who,
                0.0,
                TextFormat {
                    font_id: theme::semibold(13.5),
                    color: p.ink,
                    ..Default::default()
                },
            );
            job.append(
                "  ",
                0.0,
                TextFormat {
                    font_id: egui::FontId::proportional(13.0),
                    color: p.body,
                    ..Default::default()
                },
            );
            job.append(
                what,
                0.0,
                TextFormat {
                    font_id: egui::FontId::proportional(13.0),
                    color: p.body,
                    ..Default::default()
                },
            );
        }
        let galley = ui.fonts(|f| f.layout_job(job));
        let span = galley.size().x.max(1.0) + 48.0;
        let width = ui.available_width().max(240.0);
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, 54.0), egui::Sense::hover());
        let id = ui.id().with("credits-scroll");
        let mut offset = ui.ctx().data(|d| d.get_temp::<f32>(id).unwrap_or(0.0));
        if !response.hovered() {
            offset += ui.input(|i| i.unstable_dt.min(0.05)) * 18.0;
        }
        offset %= span;
        ui.ctx().data_mut(|d| d.insert_temp(id, offset));
        ui.ctx().request_repaint_after(Duration::from_millis(33));

        let painter = ui.painter();
        let radius = egui::CornerRadius::same(10);
        painter.rect_filled(rect, radius, p.bg_deep);
        painter.rect_stroke(
            rect,
            radius,
            egui::Stroke::new(1.0_f32, p.line),
            egui::StrokeKind::Inside,
        );
        let strip = painter.with_clip_rect(rect);
        let y = rect.center().y - galley.size().y * 0.5;
        for k in -1..=2 {
            let x = rect.left() - offset + k as f32 * span;
            if x < rect.right() && x + galley.size().x > rect.left() {
                strip.galley(egui::pos2(x, y), galley.clone(), p.ink);
            }
        }
        for i in 0..26 {
            let t = i as f32 / 26.0;
            let a = (232.0 * (1.0 - t)) as u8;
            let left = rect.left() + i as f32;
            let right = rect.right() - i as f32 - 1.0;
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(left, rect.top() + 1.0),
                    egui::pos2(left + 1.0, rect.bottom() - 1.0),
                ),
                0.0,
                theme::alpha(p.bg_deep, a),
            );
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(right, rect.top() + 1.0),
                    egui::pos2(right + 1.0, rect.bottom() - 1.0),
                ),
                0.0,
                theme::alpha(p.bg_deep, a),
            );
        }
        ui.add_space(theme::SPACE_XS);
        ui.label(
            crate::i18n::text("Hover over the credit line to pause it.")
                .size(11.5)
                .color(p.muted),
        );
        response.on_hover_text(crate::i18n::tr("Pause the credits"));
    });
}

/// Where the launcher keeps its data, and the button that moves it beside the program. A fresh copy starts here already, so this page is
/// for the player who has been using a build in their profile and wants the saves, cache and game in one folder they can carry.
fn storage(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    widgets::section(ui, p, "Storage", |ui| {
        let exe = crate::platform::exe_dir();
        let portable = exe
            .as_ref()
            .and_then(|d| crate::platform::portable_root(d))
            .is_some();
        let folder = crate::config::Config::dir();
        widgets::kv(
            ui,
            p,
            "Folder",
            &folder
                .as_ref()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| crate::platform::NO_DATA_DIR.to_string()),
        );
        widgets::kv(
            ui,
            p,
            "Portable",
            if portable {
                "yes, everything is in this folder"
            } else {
                "no, in your profile folder"
            },
        );
        if portable {
            ui.add_space(theme::SPACE_S);
            ui.label(
                crate::i18n::text(
                    "Move or copy this whole folder to take the saves and the game with it.",
                )
                .size(13.0)
                .color(p.muted),
            );
            return;
        }
        ui.add_space(theme::SPACE_S);
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, p, "Keep everything in this folder", true).clicked() {
                match exe.as_deref().map(|d| crate::platform::set_portable(d, true)) {
                    Some(Ok(copied)) => {
                        let saves = if copied.is_some() { " Your saves, settings and installed mods were copied across." } else { " Your settings and installed mods were copied across." };
                        app.notify(
                            format!("Portable mode is on. Restart EmblemWeave to use it; the game is built into this folder on the next build.{saves}"),
                            Tone::Ok,
                        );
                    }
                    Some(Err(e)) => app.notify(format!("Portable mode could not be turned on: {e}"), Tone::Bad),
                    None => app.notify("The folder this copy runs from could not be found", Tone::Bad),
                }
            }
        });
        ui.label(
            crate::i18n::text(
                "Everything then lives in a `data` folder beside the program, so the whole thing can be moved, copied or kept on a stick, \
                 and no other copy on this PC sees its saves. The game folder that is already built stays where it was, and is built again \
                 inside this folder on the next build; nothing in the old profile is deleted. An unpacked release zip is portable from the \
                 start, with no button to press.",
            )
            .size(13.0)
            .color(p.muted),
        );
    });
}

/// Discord Rich Presence while the launcher or game is running. The
/// application is built in; the player only chooses whether to show it.
fn discord(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    widgets::section(ui, p, "Discord Rich Presence", |ui| {
        let mut enabled = app.config.discord.enabled;
        ui.horizontal(|ui| {
            if widgets::toggle(ui, p, &mut enabled).changed() {
                app.config.discord.enabled = enabled;
                if enabled {
                    app.discord.start_launcher();
                } else {
                    app.discord.stop();
                }
            }
            ui.label(
                crate::i18n::text(
                    "Show EmblemWeave in Discord while the launcher or game runs",
                )
                .color(p.body),
            );
        });
        ui.add_space(theme::SPACE_S);
        ui.label(
            crate::i18n::text(
                "Built in: no application ID, token, or Discord setup is needed. Discord must be running on this PC. It shows \"In the launcher\" here and \"Playing the campaign\" while the game runs.",
            )
            .size(11.5)
            .color(p.muted),
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
            ui.label(crate::i18n::text("Check for updates when the launcher starts").color(p.body));
        });
        let busy = matches!(
            app.update.state,
            UpdateState::Checking | UpdateState::Downloading
        );
        if !app.config.updates.skipped.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    crate::i18n::text(format!("{} is skipped.", app.config.updates.skipped))
                        .size(12.0)
                        .color(p.muted),
                );
                if widgets::button(ui, p, "Stop skipping it", !busy).clicked() {
                    app.config.updates.skipped.clear();
                    app.check_updates();
                }
            });
        }
        ui.add_space(theme::SPACE_S);
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, p, "Check for updates", !busy).clicked() {
                app.check_updates();
            }
            if updater::has_backup()
                && widgets::button(ui, p, "Restore the previous launcher", !busy)
                    .on_hover_text(crate::i18n::tr("Puts back the launcher file the last update replaced, and restarts. Engine sources are not rolled back; the built game and your data are unchanged"))
                    .clicked()
            {
                match updater::restore_previous() {
                    Ok(()) => app.update.restart = true,
                    Err(e) => app.notify(e, Tone::Bad),
                }
            }
            ui.hyperlink_to(crate::i18n::text("All releases"), format!("https://github.com/{}/releases", updater::REPO));
        });
        ui.add_space(theme::SPACE_S);

        match app.update.state.clone() {
            UpdateState::Idle => {
                ui.label(
                    crate::i18n::text(
                        "The launcher checks GitHub for a newer release and installs it in place. It replaces the launcher, the docs and the \
                         engine sources; your data, saves, built game, fetched tools and portable-storage choice are kept.",
                    )
                    .size(12.5)
                    .color(p.muted),
                );
            }
            UpdateState::Checking => {
                widgets::status_line(
                    ui,
                    p,
                    Tone::Neutral,
                    "Checking GitHub…",
                    "The newest published release is looked up; nothing is downloaded yet.",
                );
            }
            UpdateState::UpToDate(tag) => {
                widgets::status_line(
                    ui,
                    p,
                    Tone::Ok,
                    "You are on the newest release",
                    &format!("{tag} is the newest published release."),
                );
            }
            UpdateState::Failed(e) => {
                widgets::status_line(
                    ui,
                    p,
                    Tone::Bad,
                    "The update check failed",
                    &format!("{e}. Nothing was changed."),
                );
            }
            UpdateState::InstallFailed(e) => {
                widgets::status_line(
                    ui,
                    p,
                    Tone::Bad,
                    "The update did not finish",
                    &format!("{e}. This launcher is still running; your data and built game were not touched. If the swap itself failed, logs\\update.log has the details."),
                );
            }
            UpdateState::Downloading => {
                widgets::status_line(
                    ui,
                    p,
                    Tone::Accent,
                    "Downloading the new version…",
                    "It is written over this install; the launcher then closes, starts the new build and keeps your data and built game.",
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
                        "This build is {}. Updating replaces the launcher, docs and engine sources, then restarts. Your data, saves, \
                         built game, fetched tools and portable-storage choice are kept.",
                        env!("CARGO_PKG_VERSION")
                    ),
                );
                ui.add_space(theme::SPACE_S);
                ui.horizontal_wrapped(|ui| {
                    if widgets::primary_button(ui, p, "Download and install", rel.asset.is_some())
                        .clicked()
                    {
                        app.install_update();
                    }
                    if !rel.page.is_empty() {
                        ui.hyperlink_to(crate::i18n::text("Release notes"), &rel.page);
                    }
                    if widgets::button(ui, p, "Skip this version", true).clicked() {
                        app.config.updates.skipped = rel.tag.clone();
                        app.update.state = UpdateState::Idle;
                        app.notify(format!("Skipped {}; undo it above", rel.tag), Tone::Neutral);
                    }
                });
                if rel.asset.is_none() {
                    ui.label(
                        crate::i18n::text(
                            "That release has no Windows download; use the release page.",
                        )
                        .size(12.0)
                        .color(p.warn),
                    );
                }
                if !rel.notes.is_empty() {
                    ui.add_space(theme::SPACE_S);
                    egui::ScrollArea::vertical()
                        .id_salt("release-notes")
                        .max_height(crate::widgets::list_height(ui, 0.22))
                        .show(ui, |ui| {
                            ui.label(crate::i18n::text(&rel.notes).size(12.5).color(p.body));
                        });
                }
            }
        }
    });
}
