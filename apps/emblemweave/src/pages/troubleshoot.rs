//! Troubleshooter: the checks a support reply would ask for, in the order they
//! are useful, plus one place to produce a report that can be attached to an
//! issue. It deliberately reuses the same detections as the rest of the
//! launcher instead of maintaining a second, contradictory list.

use crate::deps::{self, Status};
use crate::jobs::JobKind;
use crate::pages::Page;
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui;

/// The Troubleshooter's report workspace, kept on the app so collapsing the
/// Report An Issue drawer never discards a draft or a loaded attachment list.
#[derive(Default)]
pub struct Ui {
    pub report: crate::pages::report_issue::State,
}

fn tool_status() -> (Tone, String) {
    let Some(rows) = deps::cached() else {
        return (Tone::Neutral, "Still checking the build tools on this PC.".into());
    };
    let blocking = deps::blocking(&rows);
    if blocking.is_empty() {
        let optional = rows.iter().filter(|r| matches!(r.status, Status::NotNeeded { .. })).count();
        (Tone::Ok, if optional == 0 { "Everything needed to build is installed.".into() } else { format!("Everything needed is installed; {optional} optional item(s) are not needed here.") })
    } else {
        let names: Vec<&str> = blocking.iter().map(|r| r.dep.name).collect();
        (Tone::Bad, format!("Missing or too old: {}. Open Requirements for the one-line installs.", names.join(", ")))
    }
}

fn data_status(app: &App) -> (Tone, String) {
    match app.game.worst() {
        crate::gamedata::Severity::Ok | crate::gamedata::Severity::Info => (Tone::Ok, "The game files needed for a build were found.".into()),
        crate::gamedata::Severity::Warn => (Tone::Warn, app.game.attention_line().unwrap_or_else(|| "The game files need attention.".into())),
        crate::gamedata::Severity::Error => (Tone::Bad, app.game.attention_line().unwrap_or_else(|| "The game files are incomplete.".into())),
    }
}

fn row(ui: &mut egui::Ui, p: &theme::Palette, tone: Tone, title: &str, detail: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    ui.horizontal_top(|ui| {
        widgets::status_dot(ui, p, tone);
        ui.vertical(|ui| {
            ui.label(crate::i18n::text(title).font(theme::semibold(13.5)).color(p.ink));
            ui.label(crate::i18n::text(detail).size(12.5).color(p.body));
        });
        if let Some(label) = action {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if widgets::button(ui, p, label, true).clicked() {
                    clicked = true;
                }
            });
        }
    });
    ui.add_space(theme::SPACE_XS);
    clicked
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(
        ui,
        &p,
        "Troubleshooter",
        "The usual causes, the checks this launcher already runs, and one report to attach when none of it matches.",
    );

    let (data_tone, data_text) = data_status(app);
    let (tools_tone, tools_text) = tool_status();
    let built = crate::engine::ready();
    let built_text = if built {
        "A game build is ready in this folder.".to_string()
    } else {
        "No game build was found yet.".to_string()
    };
    let logs = crate::jobs::newest_log();
    let log_text = logs
        .as_ref()
        .map(|p| format!("Newest log: {}", p.display()))
        .unwrap_or_else(|| "No task logs have been written yet.".into());
    let save_dir = crate::config::Config::save_dir();
    let save_text = save_dir
        .as_ref()
        .map(|p| format!("Save folder: {}", p.display()))
        .unwrap_or_else(|| "The launcher's data folder is not set.".into());

    widgets::section(ui, &p, "Start here", |ui| {
        if row(ui, &p, data_tone, "Game files", &data_text, Some("Game Setup")) {
            app.open(Page::Setup);
        }
        if row(ui, &p, tools_tone, "Build tools", &tools_text, Some("Requirements")) {
            app.open(Page::Requirements);
        }
        if row(
            ui,
            &p,
            if built { Tone::Ok } else { Tone::Neutral },
            "Game build",
            &built_text,
            if built { Some("Overview") } else { Some("Build") },
        ) {
            if built {
                app.open(Page::Overview);
            } else if app.game.exefs.is_some() && (app.game.romfs.is_some() || app.game.nup.is_some()) {
                app.start(JobKind::Engine);
            } else {
                app.open(Page::Setup);
                app.notify("Choose your game data first; the build uses your own dump.", Tone::Warn);
            }
        }
        if row(ui, &p, Tone::Neutral, "Task logs", &log_text, Some("Open")) {
            if let Some(dir) = crate::jobs::log_dir() {
                let _ = std::process::Command::new("explorer").arg(dir).spawn();
            }
        }
        if row(ui, &p, Tone::Neutral, "Saves", &save_text, Some("Open")) {
            if let Some(dir) = save_dir {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(dir).spawn();
            }
        }
    });

    let mut report_state = std::mem::take(&mut app.trouble_ui.report);
    report_state.ensure_loaded();
    let report_summary = report_state.summary();
    // A fresh navigation to this page gets a fresh drawer id, so Report An Issue
    // starts collapsed every time the Troubleshooter opens while the draft and
    // attachment list live on independently.
    let drawer_id = format!("report-an-issue-{}", app.page_epoch);
    widgets::section_drawer(
        ui,
        &p,
        &drawer_id,
        "Report An Issue",
        &report_summary,
        false,
        |ui| crate::pages::report_issue::workspace(app, ui, &p, &mut report_state),
    );
    app.trouble_ui.report = report_state;
    // The drawer's body does not draw when collapsed; persist the last keystroke here.
    app.trouble_ui.report.flush();
    if let Some((message, tone)) = app.trouble_ui.report.take_notice() {
        app.notify(message, tone);
    }

    widgets::section(ui, &p, "Common fixes", |ui| {
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, &p, "Check the build tools again", true)
                .on_hover_text(crate::i18n::tr("Runs the same checks as Requirements; a newly installed tool is found without reopening the launcher."))
                .clicked()
            {
                let rows = deps::refresh();
                let text = deps::summary(&rows);
                app.notify(text, if deps::blocking(&rows).is_empty() { Tone::Ok } else { Tone::Warn });
                ui.ctx().request_repaint();
            }
            let can_repair = app.game.exefs.is_some()
                && (app.game.romfs.is_some() || app.game.nup.is_some())
                && app.running(JobKind::Engine).is_none();
            if widgets::button(ui, &p, "Repair the game build", can_repair)
                .on_hover_text(crate::i18n::tr("Clears the compiled game folder and builds it again from your dump. Saves, mods and game files are not touched."))
                .clicked()
            {
                match crate::engine::clean_runtime_build() {
                    Ok(()) => app.start(JobKind::Engine),
                    Err(e) => app.notify(format!("Could not start the repair: {e}"), Tone::Bad),
                }
            }
            if widgets::button(ui, &p, "Reset graphics to safe defaults", true)
                .on_hover_text(crate::i18n::tr("Returns resolution, upscaler, frame limit and shader settings to the defaults. Your saves and game files are not touched. This is the first thing to try for a black screen."))
                .clicked()
            {
                app.confirm = Some(crate::Confirm::SafeGraphics);
            }
            if widgets::button(ui, &p, "Open the game log", true).clicked() {
                if let Some(dir) = crate::engine::run_dir() {
                    let _ = std::process::Command::new("explorer").arg(dir).spawn();
                } else {
                    app.notify("The run folder is not available yet. Build or start the game once.", Tone::Warn);
                }
            }
            if widgets::button(ui, &p, "Open the data folder", true).clicked() {
                if let Some(dir) = crate::config::Config::dir() {
                    let _ = std::process::Command::new("explorer").arg(dir).spawn();
                }
            }
        });
        ui.add_space(theme::SPACE_S);
        ui.label(
            crate::i18n::text(
                "A black screen with working sound is almost always a shader cache or graphics setting problem. A build error is almost always a missing or mixed toolchain. The Requirements and Diagnostics pages name both.",
            )
            .size(12.5)
            .color(p.muted),
        );
    });

    widgets::section(ui, &p, "The graphics device was lost at startup (#72)", |ui| {
        ui.label(
            crate::i18n::text(
                "If the game stops in its first seconds and the report says GPU DEVICE LOST with present mode immediate, and the GPU is an Intel one (Iris Xe, UHD, Core Ultra or Arc), Intel's Windows driver reset the device on the immediate present path. It is not a missing game file: the engine avoids that path on Intel graphics that share system memory. Install the newest Intel graphics driver, and press Rebuild once after a launcher update so the built game carries the fix.",
            )
            .size(12.5)
            .color(p.body),
        );
        ui.add_space(theme::SPACE_S);
        let safe = app.config.graphics.present_guard == "Always on";
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, &p, if safe { "Safe present mode is on" } else { "Use the safe present mode" }, !safe)
                .on_hover_text(crate::i18n::tr("Uses mailbox instead of immediate presentation on every GPU. The same switch is on Graphics > Display > Present mode safeguard; Off puts immediate back."))
                .clicked()
            {
                app.config.graphics.present_guard = "Always on".into();
                app.note("Safe present mode is on: the next start avoids the immediate present path", Tone::Ok);
            }
            if widgets::button(ui, &p, "Open Graphics", true).clicked() {
                app.open(Page::Graphics);
            }
        });
        ui.add_space(theme::SPACE_S);
        ui.label(
            crate::i18n::text(
                "By hand, `[gfx] present_guard = on` in fefw.ini (or present_mode = mailbox) does the same. The game log line `the present-mode guard replaced immediate with ...` says when the engine acted.",
            )
            .size(12.0)
            .color(p.muted),
        );
    });

    widgets::section(ui, &p, "Long sessions and memory", |ui| {
        ui.label(
            crate::i18n::text(
                "The engine reserves a large guest heap (12 GiB on this title) and stops itself at its host-memory limit (16 GiB by default). On a 16 GB PC, a long session followed by a scene that opens many voice banks can reach Windows' commit limit before the guard can write its own report. That is one plausible reading of the 0-frames freeze reported after about two hours of play; the attached freeze log did not contain a memory figure, so it is not proven yet.",
            )
            .size(12.5)
            .color(p.body),
        );
        ui.add_space(theme::SPACE_S);
        ui.label(
            crate::i18n::text(
                "This release makes the next report answer the question: the engine's freeze file now records private commit, working set, the host memory guard's footprint and limit, system RAM/commit, and presented frames. If it happens again, create a report below. Do not delete fefw_freeze.txt: it is copied into the bundle automatically.",
            )
            .size(12.5)
            .color(p.body),
        );
        ui.add_space(theme::SPACE_S);
        ui.horizontal_wrapped(|ui| {
            widgets::pill(ui, &p, "Freeze report: run/fefw_freeze.txt", Tone::Neutral);
            widgets::pill(ui, &p, "Game log: run/fefw.log", Tone::Neutral);
            if widgets::button(ui, &p, "Open the run folder", true).clicked() {
                if let Some(dir) = crate::engine::run_dir() {
                    let _ = std::process::Command::new("explorer").arg(dir).spawn();
                }
            }
        });
    });

    widgets::section(ui, &p, "Still stuck?", |ui| {
        ui.label(
            crate::i18n::text(
                "The issue tracker is the right place for a reproducible bug. Include the report bundle and the exact step that fails. Do not open an issue for game data you do not own, and do not upload game files, keys or saves that contain personal information.",
            )
            .size(12.5)
            .color(p.body),
        );
        ui.add_space(theme::SPACE_S);
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, &p, "Open Troubleshooting", true).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab("https://github.com/XiONDiGiTaL/EmblemWeave/blob/main/docs/TROUBLESHOOTING.md"));
            }
            if widgets::button(ui, &p, "Open the issue tracker", true).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab("https://github.com/XiONDiGiTaL/EmblemWeave/issues"));
            }
        });
    });
}
