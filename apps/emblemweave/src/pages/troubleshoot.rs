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
use eframe::egui::{self};
use std::path::{Path, PathBuf};

/// The attachment handoff state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttachmentState {
    /// No report generated yet.
    #[default]
    None,
    /// Report generated and validated, awaiting user attachment.
    AwaitingAttachment,
    /// User opened GitHub issue page (browser handoff in progress).
    BrowserHandoff,
    /// Attachment confirmed by user (manual confirmation).
    Confirmed,
    /// User dismissed the handoff without confirming.
    Dismissed,
}

#[derive(Default)]
pub struct Ui {
    pub note: String,
    pub report: Option<PathBuf>,
    pub report_validation: Option<(bool, String, Option<String>)>, // (valid, message, sha256)
    pub attachment_state: AttachmentState,
    pub show_handoff: bool,
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

    widgets::section(ui, &p, "Report an issue", |ui| {
        // Step 1: Write what happened
        ui.label(
            crate::i18n::text(
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

        // Step 2: Create report bundle
        ui.horizontal_wrapped(|ui| {
            if widgets::primary_button(ui, &p, "Create report bundle", true)
                .on_hover_text(crate::i18n::tr("Writes a timestamped .zip under the launcher's data/reports folder. Nothing is sent anywhere."))
                .clicked()
            {
                match report::bundle(app, &app.trouble_ui.note) {
                    Ok(path) => {
                        // Validate the generated report immediately
                        let validation = report::validate_report(&path);
                        let valid = validation.0;
                        let msg = validation.1.clone();
                        app.trouble_ui.report = Some(path.clone());
                        app.trouble_ui.report_validation = Some(validation);
                        app.trouble_ui.attachment_state = AttachmentState::AwaitingAttachment;
                        app.trouble_ui.show_handoff = true;
                        if valid {
                            app.notify(format!("Report created and validated: {}", msg), Tone::Ok);
                        } else {
                            app.notify(format!("Report created but validation failed: {}", msg), Tone::Warn);
                        }
                    }
                    Err(e) => app.notify(format!("Could not create the report: {e}"), Tone::Bad),
                }
            }
        });

        // Step 3: Handoff interface - shown after report is created
        let report_path = app.trouble_ui.report.clone();
        if let Some(path) = report_path {
            if app.trouble_ui.show_handoff {
                ui.add_space(theme::SPACE_S);
                render_handoff_interface(app, ui, &p, &path);
            } else {
                // Compact view when handoff is collapsed
                ui.add_space(theme::SPACE_S);
                ui.horizontal_wrapped(|ui| {
                    widgets::status_dot(ui, &p, report::tone(path.as_path()));
                    ui.label(crate::i18n::text(report::describe(path.as_path())).size(12.5).color(p.body));
                    if widgets::button(ui, &p, "Show attachment steps", true).clicked() {
                        app.trouble_ui.show_handoff = true;
                    }
                });
                // Show attachment reminder if not confirmed
                if app.trouble_ui.attachment_state == AttachmentState::AwaitingAttachment {
                    ui.add_space(theme::SPACE_XS);
                    ui.label(
                        crate::i18n::text("⚠ Report created but not yet attached to an issue. Click \"Show attachment steps\" for instructions.")
                            .size(11.5)
                            .color(p.warn),
                    );
                }
            }
        } else {
            ui.add_space(theme::SPACE_S);
            ui.label(crate::i18n::text("No report has been created in this session.").size(12.0).color(p.muted));
        }
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

/// Renders the diagnostic report attachment handoff interface.
fn render_handoff_interface(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette, path: &Path) {
    let default_validation = (false, "Not validated".to_string(), None::<String>);
    let validation_ref = app.trouble_ui.report_validation.as_ref().unwrap_or(&default_validation);
    let valid = validation_ref.0;
    let validation_msg = validation_ref.1.clone();
    let sha256 = validation_ref.2.clone();
    let is_zip = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    // Header with attachment state
    let (state_text, state_tone) = match app.trouble_ui.attachment_state {
        AttachmentState::AwaitingAttachment => ("⏳ Awaiting attachment", Tone::Warn),
        AttachmentState::BrowserHandoff => ("🌐 Browser opened — attach the file in GitHub", Tone::Neutral),
        AttachmentState::Confirmed => ("✅ Attachment confirmed", Tone::Ok),
        AttachmentState::Dismissed => ("⊘ Handoff dismissed", Tone::Neutral),
        AttachmentState::None => ("—", Tone::Neutral),
    };

    // Collapsible handoff panel
    egui::Frame::none()
        .fill(p.panel_hi)
        .stroke(egui::Stroke::new(1.0, p.line))
        .corner_radius(egui::CornerRadius::same(8))
        .show(ui, |ui| {
            ui.add_space(theme::SPACE_S);
            ui.horizontal(|ui| {
                ui.strong(crate::i18n::text("Attachment Handoff").font(theme::semibold(14.0)).color(p.ink));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, p, "✕", true).clicked() {
                        app.trouble_ui.show_handoff = false;
                    }
                });
            });
            ui.add_space(theme::SPACE_S);

            // Report file info
            ui.horizontal(|ui| {
                ui.label(crate::i18n::text("File:").font(theme::semibold(12.0)).color(p.ink));
                ui.label(crate::i18n::text(path.file_name().unwrap_or_default().to_string_lossy()).font(theme::mono(12.0)).color(p.body));
            });
            ui.horizontal(|ui| {
                ui.label(crate::i18n::text("Location:").font(theme::semibold(12.0)).color(p.ink));
                ui.label(crate::i18n::text(path.parent().unwrap_or(path).display().to_string()).font(theme::mono(11.0)).color(p.muted));
            });
            ui.horizontal(|ui| {
                ui.label(crate::i18n::text("Size:").font(theme::semibold(12.0)).color(p.ink));
                ui.label(crate::i18n::text(crate::fmt_size(size)).font(theme::mono(12.0)).color(p.body));
            });
            if let Some(hash) = sha256.as_ref() {
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::text("SHA-256:").font(theme::semibold(12.0)).color(p.ink));
                    ui.label(crate::i18n::text(hash).font(theme::mono(10.0)).color(p.muted));
                });
            }

            ui.add_space(theme::SPACE_S);

            // Validation status
            ui.horizontal(|ui| {
                widgets::status_dot(ui, p, if valid { Tone::Ok } else { Tone::Bad });
                ui.label(crate::i18n::text(if valid { "Validation: PASSED" } else { "Validation: FAILED" }).font(theme::semibold(12.0)).color(if valid { p.accent } else { p.bad }));
            });
            ui.label(crate::i18n::text(validation_msg).size(11.5).color(p.muted));

            ui.add_space(theme::SPACE_S);

            // Attachment state
            ui.horizontal(|ui| {
                ui.label(crate::i18n::text("Status:").font(theme::semibold(12.0)).color(p.ink));
                ui.label(crate::i18n::text(state_text).size(12.0).color(state_tone.colour(p)));
            });

            ui.add_space(theme::SPACE_S);

            // Action buttons
            ui.horizontal_wrapped(|ui| {
                // Open containing folder
                if widgets::button(ui, p, "📁 Open report folder", true).clicked() {
                    let dir = if path.is_dir() { path.to_path_buf() } else { path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| path.to_path_buf()) };
                    let _ = std::process::Command::new("explorer").arg(dir).spawn();
                }

                // Copy path
                if widgets::button(ui, p, "📋 Copy path", true).clicked() {
                    ui.ctx().copy_text(path.display().to_string());
                    app.notify("Report path copied to clipboard", Tone::Ok);
                }

                // Copy SHA-256
                if let Some(hash) = sha256 {
                    if widgets::button(ui, p, "🔐 Copy SHA-256", true).clicked() {
                        ui.ctx().copy_text(hash.clone());
                        app.notify("SHA-256 copied to clipboard", Tone::Ok);
                    }
                }
            });

            ui.add_space(theme::SPACE_M);

            // GitHub issue attachment instructions
            ui.label(
                crate::i18n::text("Attach the report to a GitHub issue:")
                    .font(theme::semibold(12.5))
                    .color(p.ink),
            );
            ui.add_space(theme::SPACE_XS);

            // Instructions
            egui::Frame::none()
                .fill(p.panel)
                .corner_radius(egui::CornerRadius::same(6))
                .show(ui, |ui| {
                    ui.add_space(theme::SPACE_S);
                    ui.horizontal(|ui| {
                        ui.add_space(theme::SPACE_S);
                        ui.vertical(|ui| {
                            ui.label(
                                crate::i18n::text("1. Click \"Open GitHub issue\" below (or go to github.com/XiONDiGiTaL/EmblemWeave/issues/new)")
                                    .size(12.0)
                                    .color(p.body),
                            );
                            ui.add_space(theme::SPACE_XS);
                            ui.label(
                                crate::i18n::text("2. In the issue editor, drag the .zip file into the description/comment box,")
                                    .size(12.0)
                                    .color(p.body),
                            );
                            ui.label(
                                crate::i18n::text("   or click the paperclip (Attach files) button and select the .zip")
                                    .size(12.0)
                                    .color(p.body),
                            );
                            ui.add_space(theme::SPACE_XS);
                            ui.label(
                                crate::i18n::text("3. Submit the issue — GitHub will show the attachment in the issue")
                                    .size(12.0)
                                    .color(p.body),
                            );
                            ui.add_space(theme::SPACE_XS);
                            ui.label(
                                crate::i18n::text("4. Click \"I attached the file\" below to confirm")
                                    .size(12.0)
                                    .color(p.body),
                            );
                        });
                        ui.add_space(theme::SPACE_S);
                    });
                    ui.add_space(theme::SPACE_S);
                });

            ui.add_space(theme::SPACE_S);

            // Action row
            ui.horizontal_wrapped(|ui| {
                let can_open = !app.trouble_ui.note.trim().is_empty();
                if widgets::primary_button(ui, p, "🌐 Open GitHub issue", can_open)
                    .on_hover_text(crate::i18n::tr("Opens the repository's new-issue page with your description and build details prefilled. The report is NOT attached automatically; you must drag the .zip into the issue."))
                    .clicked()
                {
                    if app.trouble_ui.report.is_none() {
                        match report::bundle(app, &app.trouble_ui.note) {
                            Ok(path) => {
                                let validation = report::validate_report(&path);
                                app.trouble_ui.report = Some(path);
                                app.trouble_ui.report_validation = Some(validation);
                            }
                            Err(e) => {
                                app.notify(format!("Could not create the report: {e}"), Tone::Bad);
                                return;
                            }
                        }
                    }
                    let url = report::issue_url(app, &app.trouble_ui.note, app.trouble_ui.report.as_deref());
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                    app.trouble_ui.attachment_state = AttachmentState::BrowserHandoff;
                }

                if app.trouble_ui.attachment_state == AttachmentState::AwaitingAttachment
                    || app.trouble_ui.attachment_state == AttachmentState::BrowserHandoff
                {
                    if widgets::button(ui, p, "✅ I attached the file", true)
                        .on_hover_text(crate::i18n::tr("Confirm that you have attached the report file to the GitHub issue."))
                        .clicked()
                    {
                        app.trouble_ui.attachment_state = AttachmentState::Confirmed;
                        app.notify("Attachment confirmed. Thank you!", Tone::Ok);
                    }
                }

                if widgets::button(ui, p, "⊘ Dismiss", true)
                    .on_hover_text(crate::i18n::tr("Hide this handoff. You can reopen it from the report status above."))
                    .clicked()
                {
                    app.trouble_ui.attachment_state = AttachmentState::Dismissed;
                    app.trouble_ui.show_handoff = false;
                }
            });

            ui.add_space(theme::SPACE_S);

            // Reminder if not confirmed
            if app.trouble_ui.attachment_state != AttachmentState::Confirmed {
                ui.label(
                    crate::i18n::text("⚠ The report is NOT attached until you drag it into the GitHub issue and confirm above.")
                        .size(11.5)
                        .color(p.warn),
                );
            }

            // Privacy note
            ui.add_space(theme::SPACE_XS);
            ui.label(
                crate::i18n::text("Privacy: The report contains system info, paths, and logs. Review it before sharing. No data is uploaded automatically.")
                    .size(11.0)
                    .color(p.muted),
            );

            ui.add_space(theme::SPACE_S);
        });
}
