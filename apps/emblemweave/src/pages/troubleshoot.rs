//! Troubleshooter: the checks a support reply would ask for, in the order they
//! are useful, plus one place to produce a report that can be attached to an
//! issue. It deliberately reuses the same detections as the rest of the
//! launcher instead of maintaining a second, contradictory list.

use crate::deps::{self, Status};
use crate::jobs::JobKind;
use crate::pages::Page;
use crate::report;
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText};
use std::path::PathBuf;

#[derive(Default)]
pub struct Ui {
    pub note: String,
    pub report: Option<PathBuf>,
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
            ui.label(RichText::new(title).font(theme::semibold(13.5)).color(p.ink));
            ui.label(RichText::new(detail).size(12.5).color(p.body));
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

    widgets::section(ui, &p, "Common fixes", |ui| {
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, &p, "Check the build tools again", true)
                .on_hover_text("Runs the same checks as Requirements; a newly installed tool is found without reopening the launcher.")
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
                .on_hover_text("Clears the compiled game folder and builds it again from your dump. Saves, mods and game files are not touched.")
                .clicked()
            {
                match crate::engine::clean_runtime_build() {
                    Ok(()) => app.start(JobKind::Engine),
                    Err(e) => app.notify(format!("Could not start the repair: {e}"), Tone::Bad),
                }
            }
            if widgets::button(ui, &p, "Reset graphics to safe defaults", true)
                .on_hover_text("Returns resolution, upscaler, frame limit and shader settings to the defaults. Your saves and game files are not touched. This is the first thing to try for a black screen.")
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
            RichText::new(
                "A black screen with working sound is almost always a shader cache or graphics setting problem. A build error is almost always a missing or mixed toolchain. The Requirements and Diagnostics pages name both.",
            )
            .size(12.5)
            .color(p.muted),
        );
    });

    widgets::section(ui, &p, "Long sessions and memory", |ui| {
        ui.label(
            RichText::new(
                "The engine reserves a large guest heap (12 GiB on this title) and stops itself at its host-memory limit (16 GiB by default). On a 16 GB PC, a long session followed by a scene that opens many voice banks can reach Windows' commit limit before the guard can write its own report. That is one plausible reading of the 0-frames freeze reported after about two hours of play; the attached freeze log did not contain a memory figure, so it is not proven yet.",
            )
            .size(12.5)
            .color(p.body),
        );
        ui.add_space(theme::SPACE_S);
        ui.label(
            RichText::new(
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

    widgets::section(ui, &p, "Report an issue", |ui| {
        ui.label(
            RichText::new(
                "Write what happened. Create the bundle, then attach the generated .zip to the GitHub issue. The bundle includes your launcher settings, the newest task logs, the game log and any freeze or crash report; it is not uploaded until you attach it yourself.",
            )
            .size(12.5)
            .color(p.body),
        );
        ui.add_space(theme::SPACE_S);
        let edit = egui::TextEdit::multiline(&mut app.trouble_ui.note)
            .desired_rows(5)
            .desired_width(f32::INFINITY)
            .hint_text("What were you doing, what did you expect, and what happened? Include the scene or menu if you can.");
        ui.add(edit);
        ui.add_space(theme::SPACE_S);
        ui.horizontal_wrapped(|ui| {
            if widgets::primary_button(ui, &p, "Create report bundle", true)
                .on_hover_text("Writes a timestamped .zip under the launcher's data/reports folder. Nothing is sent anywhere.")
                .clicked()
            {
                match report::bundle(app, &app.trouble_ui.note) {
                    Ok(path) => {
                        let text = report::describe(&path);
                        app.trouble_ui.report = Some(path);
                        app.notify(text, Tone::Ok);
                    }
                    Err(e) => app.notify(format!("Could not create the report: {e}"), Tone::Bad),
                }
            }
            let has_report = app.trouble_ui.report.is_some();
            if widgets::button(ui, &p, "Open the report folder", has_report).clicked() {
                if let Some(path) = &app.trouble_ui.report {
                    let dir = if path.is_dir() { path.clone() } else { path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| path.clone()) };
                    let _ = std::process::Command::new("explorer").arg(dir).spawn();
                }
            }
            if widgets::button(ui, &p, "Copy the report path", has_report).clicked() {
                if let Some(path) = &app.trouble_ui.report {
                    ui.ctx().copy_text(path.display().to_string());
                    app.notify("Report path copied; attach the file to the issue", Tone::Ok);
                }
            }
            let can_open = !app.trouble_ui.note.trim().is_empty();
            if widgets::button(ui, &p, "Open a GitHub issue", can_open)
                .on_hover_text("Opens the repository's new-issue page with your description and build details prefilled. The report is not attached automatically; drag the .zip into the issue after it opens.")
                .clicked()
            {
                if app.trouble_ui.report.is_none() {
                    match report::bundle(app, &app.trouble_ui.note) {
                        Ok(path) => app.trouble_ui.report = Some(path),
                        Err(e) => {
                            app.notify(format!("Could not create the report: {e}"), Tone::Bad);
                            return;
                        }
                    }
                }
                let url = report::issue_url(app, &app.trouble_ui.note, app.trouble_ui.report.as_deref());
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        });
        if let Some(path) = &app.trouble_ui.report {
            ui.add_space(theme::SPACE_S);
            ui.horizontal_wrapped(|ui| {
                widgets::status_dot(ui, &p, report::tone(path));
                ui.label(RichText::new(report::describe(path)).size(12.5).color(p.body));
            });
        } else {
            ui.add_space(theme::SPACE_S);
            ui.label(RichText::new("No report has been created in this session.").size(12.0).color(p.muted));
        }
    });

    widgets::section(ui, &p, "Still stuck?", |ui| {
        ui.label(
            RichText::new(
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
