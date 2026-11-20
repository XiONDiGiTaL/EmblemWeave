//! Requirements: what this release needs, what this PC has, and what to install.
//! The default view is a plain-language readiness summary. The full dependency
//! matrix and install commands live in one collapsed technical drawer.

use crate::deps::{self, Row, Stage, Status};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self};
use std::time::Duration;

fn tone(status: &Status) -> Tone {
    match status {
        Status::Ready { .. } => Tone::Ok,
        Status::NotNeeded { .. } => Tone::Neutral,
        Status::TooOld { .. } => Tone::Warn,
        Status::Missing => Tone::Bad,
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(
        ui,
        &p,
        "Requirements",
        "What EmblemWeave needs to build and run your game, what this PC already has, and what to install next.",
    );

    let Some(rows) = deps::cached() else {
        widgets::section(ui, &p, "Checking this PC", |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(15.0).color(p.accent));
                ui.label(crate::i18n::text("Preparing the build-tool report…").font(theme::semibold(13.0)).color(p.ink));
            });
            ui.add_space(7.0);
            widgets::progress_bar(ui, &p, None);
            ui.add_space(7.0);
            ui.label(
                crate::i18n::text(
                    "This runs in the background: Visual Studio, Windows SDK, LLVM, CMake, Ninja, Python and the packaged shader tools are checked independently. It usually takes a few seconds on the first launch.",
                )
                .size(12.0)
                .color(p.body),
            );
            for (name, detail) in [
                ("Build tools", "The compiler, linker and build generator used for the native game."),
                ("Python environment", "The bridge that decodes the game's compressed main module."),
                ("Packaged helpers", "The shader translator shipped with this release, where present."),
            ] {
                ui.add_space(4.0);
                widgets::status_line(ui, &p, Tone::Neutral, name, detail);
            }
            if widgets::button(ui, &p, "Check now", true).clicked() {
                let rows = deps::refresh();
                app.notify(deps::summary(&rows), if deps::blocking(&rows).is_empty() { Tone::Ok } else { Tone::Warn });
            }
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        });
        return;
    };

    let blocking = deps::blocking(&rows);
    let missing_lines = deps::install_lines(&rows);
    let manual = deps::manual_fixes(&rows);
    let all_ready = blocking.is_empty();

    // Readiness, in one sentence and with the next action beside it.
    widgets::section(ui, &p, "Readiness", |ui| {
        ui.horizontal_top(|ui| {
            widgets::status_dot(ui, &p, if all_ready { Tone::Ok } else { Tone::Bad });
            ui.vertical(|ui| {
                ui.label(
                    crate::i18n::text(if all_ready { "This PC can build the game" } else { "A few things are needed first" })
                        .font(theme::display(19.0))
                        .color(if all_ready { p.ok } else { p.warn }),
                );
                ui.label(crate::i18n::text(deps::summary(&rows)).size(12.5).color(p.body));
            });
            // A fixed-height slot keeps this right-aligned action from taking the remaining page height and
            // pushing the missing-tool rows and manual instructions below the fold.
            let width = ui.available_width();
            ui.allocate_ui_with_layout(egui::vec2(width, 40.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_install = !missing_lines.is_empty() && crate::engine::have_winget();
                if !all_ready
                    && widgets::primary_button(ui, &p, "Install what is missing", can_install)
                        .on_hover_text(if crate::engine::have_winget() {
                            if missing_lines.is_empty() {
                                "The remaining tool needs the manual step below.".to_string()
                            } else {
                                format!("Runs these in a new window, one after another:\n{}", missing_lines.join("\n"))
                            }
                        } else {
                            "winget is not available on this PC. Install App Installer from the Microsoft Store, then check again.".to_string()
                        })
                        .clicked()
                {
                    match crate::engine::start_install_lines(&missing_lines) {
                        Ok(_) => app.notify("Installing in a new window. Accept the Windows prompt, then press Check again.", Tone::Ok),
                        Err(e) => app.notify(e, Tone::Bad),
                    }
                }
            });
        });
        ui.add_space(7.0);
        ui.horizontal(|ui| {
            if widgets::button(ui, &p, "Check again", true)
                .on_hover_text(crate::i18n::tr("Runs every check again. Nothing needs a restart: the checks look where a fresh install lands."))
                .clicked()
            {
                let rows = deps::refresh();
                let text = deps::summary(&rows);
                app.notify(text, if deps::blocking(&rows).is_empty() { Tone::Ok } else { Tone::Warn });
            }
            if !app.config.onboarding.completed && widgets::button(ui, &p, if app.config.onboarding.dismissed { "Resume setup guide" } else { "Open setup guide" }, true)
                .on_hover_text(crate::i18n::tr("Continue from the step you stopped at; the guide reads the same live tool checks."))
                .clicked()
            {
                app.config.onboarding.dismissed = false;
                app.config.onboarding.step = 2;
            }
            ui.label(crate::i18n::text("Checks happen in the background; this page updates itself.").size(10.5).color(p.muted));
        });
        for line in &manual {
            ui.add_space(3.0);
            ui.label(crate::i18n::text(format!("• {line}")).size(11.5).color(p.body));
        }
        if deps::row(&rows, "sdk").is_some_and(|r| r.blocking()) {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                let installer_available = crate::toolchain::visual_studio_installer().is_file();
                if widgets::button(ui, &p, "Open Visual Studio Installer", installer_available)
                    .on_hover_text(if installer_available {
                        "Opens the Visual Studio Installer. Choose Build Tools 2022 → Modify → Individual components → Windows 11 SDK."
                    } else {
                        "Visual Studio Installer was not found; use the Windows SDK download page."
                    })
                    .clicked()
                {
                    match crate::toolchain::open_visual_studio_installer() {
                        Ok(()) => app.notify(
                            "Visual Studio Installer opened. Add the Windows 11 SDK under Individual components, then press Check again.",
                            Tone::Ok,
                        ),
                        Err(e) => app.notify(e, Tone::Warn),
                    }
                }
                ui.hyperlink_to(crate::i18n::text("Open the Windows SDK download page"), deps::WINDOWS_SDK_URL);
            });
        }
    });

    widgets::section(ui, &p, "Updates and existing builds", |ui| {
        ui.label(
            crate::i18n::text(
                "An EmblemWeave update replaces the launcher, documentation and engine sources in place. It does not delete your settings, saves, mods, fetched tools or built game, and it does not force a rebuild. Build reuses what is already there and only regenerates when the engine revision, compiler/toolchain or build settings actually changed.",
            )
            .size(12.0)
            .color(p.body),
        );
        ui.add_space(5.0);
        for (name, path) in [
            ("Settings, saves, mods and logs", crate::config::Config::dir()),
            ("Engine sources and build", crate::engine::root()),
            ("The game's run folder", crate::engine::run_dir()),
        ] {
            ui.horizontal_wrapped(|ui| {
                ui.label(crate::i18n::text(format!("{name}:")).size(11.0).color(p.muted));
                match path {
                    Some(path) => ui.label(crate::i18n::text(path.display().to_string()).size(11.0).monospace().color(p.ink)),
                    None => ui.label(crate::i18n::text("not selected yet").size(11.0).color(p.muted)),
                };
            });
        }
        ui.add_space(4.0);
        widgets::pending_note(
            ui,
            &p,
            "Keep these folders when moving or backing up the launcher. Do not delete them to \"start fresh\" unless you intend to lose saves and rebuild the game.",
        );
    });

    if !blocking.is_empty() {
        widgets::section(ui, &p, "Needs attention", |ui| {
            for row in &blocking {
                row_view(app, ui, &p, row);
            }
        });
    }

    // Everything technical is still here, one deliberate step away.
    widgets::section_drawer(
        ui,
        &p,
        "dependencies",
        "All dependencies",
        if all_ready { "everything found" } else { "detailed matrix" },
        false,
        |ui| {
            // The distinction people trip over: a release that bundles the
            // translator does not need the .NET SDK, only the runtime it ships.
            for key in ["dotnet-sdk", "sdk"] {
                if let Some(r) = deps::row(&rows, key) {
                    if matches!(r.status, Status::NotNeeded { .. }) {
                        let _ = widgets::pending_note(ui, &p, &format!("{}: {}", r.dep.name, r.status.detail()));
                    }
                }
            }
            for stage in [Stage::Decode, Stage::Recompile, Stage::Build, Stage::Translate] {
                let stage_rows: Vec<&Row> = rows.iter().filter(|r| r.dep.stages.contains(&stage)).collect();
                if stage_rows.is_empty() {
                    continue;
                }
                ui.add_space(4.0);
                widgets::eyebrow(ui, &p, stage.label());
                ui.add_space(3.0);
                for row in stage_rows {
                    row_view(app, ui, &p, row);
                }
            }
            ui.add_space(5.0);
            ui.label(crate::i18n::text("Detection details and executable paths are included in the hover text and in Diagnostics → Export report.").size(10.5).color(p.muted));
        },
    );
}

fn row_view(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette, row: &Row) {
    let status = &row.status;
    let tone = tone(status);
    ui.horizontal_wrapped(|ui| {
        widgets::status_dot(ui, p, tone);
        ui.label(crate::i18n::text(row.dep.name).font(theme::semibold(12.5)).color(p.ink));
        widgets::pill(ui, p, status.label(), tone);
        if let Some(min) = row.dep.min {
            ui.label(crate::i18n::text(format!("needs {min}")).size(11.0).color(p.muted));
        }
        ui.label(crate::i18n::text(if row.required { "required" } else { "optional here" }).size(10.5).color(p.muted));
    });
    ui.horizontal_top(|ui| {
        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.add(egui::Label::new(crate::i18n::text(status.detail()).size(11.5).color(p.body)).wrap());
            ui.add(egui::Label::new(crate::i18n::text(row.dep.note).size(11.0).color(p.muted)).wrap());
        });
        if !status.ok() {
            if let Some(install) = row.dep.install {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if install.runnable() {
                        if widgets::button(ui, p, "Install", true)
                            .on_hover_text(format!("Runs in a new window:\n{}", install.line()))
                            .clicked()
                        {
                            match crate::engine::start_install_lines(&[install.line().to_string()]) {
                                Ok(_) => app.notify("Installing in a new window. Accept the Windows prompt, then press Check again.", Tone::Ok),
                                Err(e) => app.notify(e, Tone::Bad),
                            }
                        }
                    } else {
                        ui.vertical(|ui| {
                            ui.label(crate::i18n::text("Manual step").size(10.0).color(p.warn)).on_hover_text(install.line());
                            if row.dep.key == "sdk" {
                                ui.hyperlink_to(crate::i18n::text("Download page"), deps::WINDOWS_SDK_URL);
                            }
                        });
                    }
                });
            }
        }
    });
    ui.add_space(7.0);
    let line = ui.available_rect_before_wrap();
    ui.painter().hline(line.x_range(), line.top(), egui::Stroke::new(1.0_f32, theme::alpha(p.line, 100)));
    ui.add_space(6.0);
}
