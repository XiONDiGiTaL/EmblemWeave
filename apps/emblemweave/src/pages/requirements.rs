//! Requirements: every dependency this release needs, what was found on this PC,
//! and the one line that installs what is missing. Everything here comes from
//! `crate::deps`, the same registry the build checks and the docs table is
//! generated from, so the page cannot claim a different requirement than the
//! build enforces.

use crate::deps::{self, Row, Stage, Status};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText};
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
        "What this release needs, what this PC has, and what to install. Checks use the same toolchain detection the build does.",
    );
    ui.add_space(theme::SPACE_M);

    // The report is computed on a background thread at start-up, so a frame
    // never waits for vswhere, dotnet and a vcvars run.
    let Some(rows) = deps::cached() else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("Checking the toolchain…").color(p.body));
        });
        ui.add_space(theme::SPACE_S);
        if widgets::button(ui, &p, "Check now", true).clicked() {
            let rows = deps::refresh();
            app.notify(deps::summary(&rows), if deps::blocking(&rows).is_empty() { Tone::Ok } else { Tone::Warn });
        }
        ui.ctx().request_repaint_after(Duration::from_millis(250));
        return;
    };

    let blocking = deps::blocking(&rows);
    let missing_lines = deps::install_lines(&rows);
    let manual = deps::manual_fixes(&rows);

    ui.horizontal(|ui| {
        if widgets::button(ui, &p, "↻  Check again", true)
            .on_hover_text("Runs every check again. Nothing needs a restart: the checks look where a fresh install lands.")
            .clicked()
        {
            let rows = deps::refresh();
            let text = deps::summary(&rows);
            app.notify(text, if deps::blocking(&rows).is_empty() { Tone::Ok } else { Tone::Warn });
        }
        let can_install = !missing_lines.is_empty() && crate::engine::have_winget();
        if widgets::primary_button(ui, &p, "Install what is missing", can_install)
            .on_hover_text(if crate::engine::have_winget() {
                format!("Runs these in a new window, one after another:\n{}", missing_lines.join("\n"))
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
    ui.add_space(theme::SPACE_S);
    let (text, colour) = if blocking.is_empty() {
        (deps::summary(&rows), p.ok)
    } else {
        (deps::summary(&rows), p.bad)
    };
    ui.label(RichText::new(text).size(13.0).color(colour));
    if !manual.is_empty() {
        for line in &manual {
            ui.label(RichText::new(format!("• {line}")).size(12.5).color(p.body));
        }
    }
    // The distinction people trip over: a release that bundles the translator
    // does not need the .NET SDK, only the runtime the translator ships with.
    for key in ["dotnet-sdk", "sdk"] {
        if let Some(r) = deps::row(&rows, key) {
            if matches!(r.status, Status::NotNeeded { .. }) {
                let _ = widgets::pending_note(ui, &p, &format!("{}: {}", r.dep.name, r.status.detail()));
            }
        }
    }
    ui.add_space(theme::SPACE_M);

    // One section per stage: the page reads like the build runs.
    for stage in [Stage::Decode, Stage::Recompile, Stage::Build, Stage::Translate] {
        let stage_rows: Vec<&Row> = rows.iter().filter(|r| r.dep.stages.contains(&stage)).collect();
        if stage_rows.is_empty() {
            continue;
        }
        widgets::section(ui, &p, stage.label(), |ui| {
            for row in stage_rows {
                row_view(app, ui, &p, row);
            }
        });
    }
}

fn row_view(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette, row: &Row) {
    let status = &row.status;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(row.dep.name).font(theme::semibold(13.5)).color(p.ink));
        widgets::pill(ui, p, status.label(), tone(status));
        if let Some(min) = row.dep.min {
            ui.label(RichText::new(format!("needs {min}")).size(12.0).color(p.muted));
        }
        if row.required {
            ui.label(RichText::new("required").size(11.5).color(p.muted));
        } else {
            ui.label(RichText::new("optional here").size(11.5).color(p.muted));
        }
    });
    ui.label(RichText::new(status.detail()).size(12.5).color(p.body));
    if !status.ok() {
        ui.label(RichText::new(row.dep.note).size(12.0).color(p.muted));
        if let Some(install) = row.dep.install {
            ui.horizontal(|ui| {
                ui.label(RichText::new(install.line()).monospace().size(12.0).color(p.accent_hi));
                if install.runnable() && widgets::button(ui, p, "Install", true).clicked() {
                    match crate::engine::start_install_lines(&[install.line().to_string()]) {
                        Ok(_) => app.notify("Installing in a new window. Accept the Windows prompt, then press Check again.", Tone::Ok),
                        Err(e) => app.notify(e, Tone::Bad),
                    }
                }
            });
        }
    } else {
        ui.label(RichText::new(row.dep.note).size(12.0).color(p.muted));
    }
    ui.add_space(theme::SPACE_XS);
}
