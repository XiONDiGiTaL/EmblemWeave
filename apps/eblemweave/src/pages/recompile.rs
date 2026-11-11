//! Recompilation: run and inspect the static recompilation pipeline.

use crate::jobs::{JobKind, Stage, StageStatus, PIPELINE_STAGES};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(
        ui,
        &p,
        "Recompilation",
        "Decode the game's AArch64 code, recover its functions, emit C, build it natively and verify it against the reference interpreter.",
    );

    let running = app.running(JobKind::Pipeline).is_some();
    widgets::section(ui, &p, "Pipeline", |ui| {
        ui.horizontal(|ui| {
            let ready = app.game.decoded.is_some() && app.game.segments.iter().all(|s| s.ok());
            if widgets::primary_button(ui, &p, if running { "Running…" } else { "Run pipeline" }, ready && !running)
                .on_disabled_hover_text("Decode the main module on Game Setup first")
                .clicked()
            {
                app.start(JobKind::Pipeline);
            }
            ui.add_space(theme::SPACE_M);
            ui.vertical(|ui| {
                ui.label(RichText::new("Functions to recover").font(theme::semibold(13.0)).color(p.ink));
                ui.add_enabled(!running, egui::Slider::new(&mut app.lift_limit, 500..=40_000).logarithmic(true));
            });
            if running {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, &p, "Cancel", true).clicked() {
                        app.confirm = Some(crate::Confirm::CancelJob(JobKind::Pipeline));
                    }
                });
            }
        });
        ui.add_space(theme::SPACE_M);

        let stages: Vec<Stage> = match app.running(JobKind::Pipeline) {
            Some(job) => job.snapshot(|pr| pr.stages.clone()),
            None => PIPELINE_STAGES
                .iter()
                .map(|&name| Stage { name, status: StageStatus::Pending, detail: String::new(), started: None, elapsed: None })
                .collect(),
        };
        if running {
            let done = stages.iter().filter(|s| matches!(s.status, StageStatus::Done | StageStatus::Skipped | StageStatus::Failed)).count();
            widgets::progress_bar(ui, &p, Some(done as f32 / stages.len() as f32));
            ui.add_space(theme::SPACE_S);
        }
        widgets::stage_list(ui, &p, &stages);
        if !running && app.last_pipeline.is_some() {
            ui.label(RichText::new("The last completed run is summarised below.").size(12.5).color(p.muted));
        }
    });

    if let Some(s) = &app.last_pipeline {
        widgets::section(ui, &p, "Last run", |ui| {
            if let Some(r) = &s.report {
                ui.columns(4, |c| {
                    metric(&mut c[0], &p, "Decode coverage", &format!("{:.2}%", r.decode_coverage * 100.0));
                    metric(&mut c[1], &p, "Functions", &crate::fmt_count(r.lifted_functions as u64));
                    metric(&mut c[2], &p, "Basic blocks", &crate::fmt_count(r.blocks_lifted as u64));
                    metric(&mut c[3], &p, "Jump tables", &crate::fmt_count(r.jump_tables as u64));
                });
                ui.add_space(theme::SPACE_S);
                widgets::kv(ui, &p, "Unsupported words", &crate::fmt_count(r.unsupported_words as u64));
                widgets::kv(ui, &p, "Indirect edges", &format!("{} (resolved at run time)", r.unresolved_edges));
            }
            match s.compiled {
                Some(true) => widgets::status_line(ui, &p, Tone::Ok, "Native build", "the emitted C compiled to a native object"),
                Some(false) => widgets::status_line(ui, &p, Tone::Bad, "Native build", "the C compiler rejected the output"),
                None => widgets::status_line(ui, &p, Tone::Warn, "Native build", "skipped: no C compiler available"),
            }
            if let Some((ok, n)) = s.verified {
                let tone = if ok == n { Tone::Ok } else { Tone::Bad };
                widgets::status_line(ui, &p, tone, "Verification", &format!("{ok} of {n} functions identical to the reference interpreter"));
            }
            if let Some(out) = &s.output {
                ui.horizontal(|ui| {
                    widgets::kv(ui, &p, "Output", &out.display().to_string());
                    if ui.link("Open folder").clicked() {
                        let _ = std::process::Command::new("explorer").arg(out).spawn();
                    }
                });
            }
        });
    }

    if !app.last_job_log.is_empty() {
        widgets::section(ui, &p, "Log", |ui| {
            egui::ScrollArea::vertical().max_height(220.0).stick_to_bottom(true).show(ui, |ui| {
                for line in &app.last_job_log {
                    ui.label(RichText::new(line).font(theme::mono(12.0)).color(p.body));
                }
            });
        });
    }
}

fn metric(ui: &mut egui::Ui, p: &theme::Palette, label: &str, value: &str) {
    ui.label(RichText::new(value).font(theme::display(22.0)).color(p.ink));
    ui.label(RichText::new(label).size(12.0).color(p.muted));
}
