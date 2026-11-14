//! Diagnostics: what the runtime and the pipeline measured, with search and
//! an exportable report.

use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Runtime,
    Recompilation,
    Log,
}

/// The launcher's own frame times, with the 1 % and 0.1 % lows.
fn frame_times(app: &mut App, ui: &mut egui::Ui, p: &crate::theme::Palette) {
    widgets::section(ui, p, "Frame times", |ui| {
        ui.horizontal(|ui| {
            let mut shown = app.config.hud.fps;
            if widgets::toggle(ui, p, &mut shown).changed() {
                app.config.hud.fps = shown;
            }
            ui.label(RichText::new("Show the frame rate in the top bar").color(p.body));
        });
        ui.add_space(theme::SPACE_S);
        let samples = app.frames.samples();
        let s = app.frames.stats();
        if samples.is_empty() {
            ui.label(RichText::new("No frames measured yet.").color(p.muted));
            return;
        }
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(860.0), 130.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, egui::CornerRadius::same(6), crate::theme::alpha(p.panel_hi, 90));
        // The scale follows the 99th percentile: one start-up hitch would
        // otherwise flatten every other frame against the floor.
        let ceil = (s.p99_ms * 1.5).clamp(20.0, 100.0);
        let span = 1000.0 / 60.0;
        let y = |ms: f32| rect.bottom() - (ms / ceil).clamp(0.0, 1.0) * rect.height();
        for (ms, label) in [(span, "60 fps"), (2.0 * span, "30 fps")] {
            if ms < ceil {
                painter.line_segment([egui::pos2(rect.left(), y(ms)), egui::pos2(rect.right(), y(ms))], egui::Stroke::new(1.0_f32, crate::theme::alpha(p.muted, 60)));
                painter.text(egui::pos2(rect.right() - 4.0, y(ms) - 7.0), egui::Align2::RIGHT_BOTTOM, label, egui::FontId::new(9.5, egui::FontFamily::Proportional), crate::theme::alpha(p.muted, 160));
            }
        }
        let step = rect.width() / samples.len().max(1) as f32;
        for (i, t) in samples.iter().enumerate() {
            let x = rect.left() + i as f32 * step;
            let h = (t * 1000.0 / ceil).clamp(0.0, 1.0) * rect.height();
            painter.rect_filled(egui::Rect::from_min_max(egui::pos2(x, rect.bottom() - h), egui::pos2(x + step.max(1.0), rect.bottom())), 0, crate::theme::alpha(p.accent, 150));
        }
        ui.add_space(theme::SPACE_S);
        ui.horizontal_wrapped(|ui| {
            for (label, value) in [
                ("Frames", format!("{}", s.frames)),
                ("Average", format!("{:.1} fps  ({:.1} ms)", s.avg_fps, s.avg_ms)),
                ("1 % low", format!("{:.0} fps", s.low1_fps)),
                ("0.1 % low", format!("{:.0} fps", s.low01_fps)),
                ("Worst frame", format!("{:.1} ms", s.max_ms)),
            ] {
                widgets::pill(ui, p, &format!("{label}  {value}"), Tone::Neutral);
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Measured on the launcher's window, not the game's: the game draws in its own process.").size(12.0).color(p.muted));
    });
}

/// Settings the launcher saved before it changed something.
fn snapshots(app: &mut App, ui: &mut egui::Ui, p: &crate::theme::Palette) {
    let list = crate::snapshots::list();
    widgets::section(ui, p, "Settings snapshots", |ui| {
        ui.horizontal(|ui| {
            if widgets::button(ui, p, "Snap settings now", true).clicked() {
                match crate::snapshots::take("manual") {
                    Some(dir) => app.notify(format!("Settings saved to {}", dir.display()), Tone::Ok),
                    None => app.notify("There are no settings files to copy yet", Tone::Warn),
                }
            }
            ui.label(RichText::new(format!("{}, newest first, taken before the launcher changes a game setting.", list.len())).size(12.5).color(p.muted));
        });
        if list.is_empty() {
            ui.label(RichText::new("None yet.").size(12.5).color(p.muted));
            return;
        }
        for s in list.iter().take(6) {
            ui.horizontal(|ui| {
                ui.label(RichText::new(crate::snapshots::when(&s.name)).font(theme::mono(12.0)).color(p.muted));
                ui.label(RichText::new(crate::snapshots::reason(&s.name)).color(p.body));
                ui.label(RichText::new(s.files.join(", ")).size(11.5).color(p.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, p, "Restore", true).clicked() {
                        match crate::snapshots::restore(s) {
                            Ok(files) => app.notify(format!("Restored {}", files.join(", ")), Tone::Ok),
                            Err(e) => app.notify(e, Tone::Bad),
                        }
                    }
                });
            });
        }
    });
}

/// Bundles left by a crash.
fn crashes(app: &mut App, ui: &mut egui::Ui, p: &crate::theme::Palette) {
    let list = crate::crash::reports();
    widgets::section(ui, p, "Crash reports", |ui| {
        ui.horizontal(|ui| {
            if widgets::button(ui, p, "Save a report now", true)
                .on_hover_text("Writes this build, its settings and the recent activity to the crashes folder")
                .clicked()
            {
                match crate::crash::write_bundle(&crate::crash::diagnostics_text(), "saved on request") {
                    Some(dir) => app.notify(format!("Report written to {}", dir.display()), Tone::Ok),
                    None => app.notify("The report could not be written", Tone::Bad),
                }
            }
            if !list.is_empty() && widgets::button(ui, p, "Open folder", true).clicked() {
                if let Some(dir) = crate::crash::reports_dir() {
                    let _ = std::process::Command::new("explorer").arg(dir).spawn();
                }
            }
            ui.label(RichText::new(if list.is_empty() { "No crashes recorded.".to_string() } else { format!("{} recorded; the newest is {}", list.len(), list[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()) }).size(12.5).color(p.muted));
        });
    });
}

fn matches(filter: &str, text: &str) -> bool {
    filter.is_empty() || text.to_lowercase().contains(&filter.to_lowercase())
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Diagnostics", "Measurements from the runtime and the recompiler, for troubleshooting and bug reports.");

    ui.horizontal(|ui| {
        for (tab, label) in [(Tab::Runtime, "Runtime"), (Tab::Recompilation, "Recompilation"), (Tab::Log, "Activity log")] {
            if ui.selectable_label(app.diag_tab == tab, RichText::new(label).font(theme::semibold(14.0))).clicked() {
                app.diag_tab = tab;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::button(ui, &p, "Export report…", true).clicked() {
                export(app);
            }
            ui.add(egui::TextEdit::singleline(&mut app.diag_filter).hint_text("🔍 Filter").desired_width(220.0));
        });
    });
    ui.add_space(theme::SPACE_M);
    let filter = app.diag_filter.clone();

    match app.diag_tab {
        Tab::Runtime => {
            frame_times(app, ui, &p);
            snapshots(app, ui, &p);
            crashes(app, ui, &p);
            let Some((r, ms)) = &app.last_boot else {
                widgets::section(ui, &p, "", |ui| {
                    ui.label(RichText::new("No boot test has run this session. Run one from the Overview.").color(p.body));
                });
                return;
            };
            widgets::section(ui, &p, "Stop", |ui| {
                widgets::mono_line(ui, &widgets::demangle_line(&r.stop.describe()), p.warn);
                ui.add_space(theme::SPACE_S);
                widgets::kv(ui, &p, "Phase", &r.phase);
                widgets::kv(ui, &p, "Completed phases", &r.phases.join(" → "));
                widgets::kv(ui, &p, "Instructions", &crate::fmt_count(r.steps));
                widgets::kv(ui, &p, "Distinct functions", &crate::fmt_count(r.distinct_functions as u64));
                widgets::kv(ui, &p, "Host calls", &format!("{} ({} services available)", crate::fmt_count(r.host_calls), r.host_implemented));
                widgets::kv(ui, &p, "Threads / switches", &format!("{} / {}", r.threads, crate::fmt_count(r.switches)));
                widgets::kv(ui, &p, "Wall time", &format!("{:.2}s", *ms as f32 / 1000.0));
            });
            widgets::section(ui, &p, "Guest threads", |ui| {
                for t in r.thread_states.iter().filter(|t| matches(&filter, t)) {
                    let tone = if t.contains("running") { Tone::Accent } else if t.contains("waiting") { Tone::Neutral } else { Tone::Warn };
                    ui.horizontal(|ui| {
                        widgets::pill(ui, &p, if t.contains("running") { "running" } else if t.contains("waiting") { "waiting" } else { "ready" }, tone);
                        widgets::mono_line(ui, &widgets::demangle_line(t), p.body);
                    });
                }
            });
            widgets::section(ui, &p, "Host events", |ui| {
                for l in r.host_log.iter().filter(|l| matches(&filter, l)) {
                    widgets::mono_line(ui, l, p.body);
                }
            });
            widgets::section(ui, &p, "Imports reached", |ui| {
                egui::ScrollArea::vertical().id_salt("imports").max_height(crate::widgets::list_height(ui, 0.26)).show(ui, |ui| {
                    for i in r.import_calls.iter().filter(|i| matches(&filter, &i.name) || matches(&filter, &widgets::demangle(&i.name))) {
                        ui.horizontal(|ui| {
                            ui.add_sized([70.0, 16.0], egui::Label::new(RichText::new(crate::fmt_count(i.calls)).font(theme::mono(12.0)).color(p.accent_hi)));
                            ui.label(RichText::new(widgets::demangle(&i.name)).font(theme::mono(12.0)).color(p.body))
                                .on_hover_text(&i.name);
                        });
                    }
                });
            });
            widgets::section(ui, &p, "Last events before the stop", |ui| {
                egui::ScrollArea::vertical().id_salt("recent").max_height(crate::widgets::list_height(ui, 0.30)).stick_to_bottom(true).show(ui, |ui| {
                    for e in r.recent.iter().filter(|e| matches(&filter, e)) {
                        widgets::mono_line(ui, &widgets::demangle_line(e), p.body);
                    }
                });
            });
            widgets::section(ui, &p, "Hottest functions", |ui| {
                for h in &r.hot {
                    widgets::kv(ui, &p, &format!("0x{:x}", h.addr), &format!("{} entries, {} instructions", crate::fmt_count(h.entries), crate::fmt_count(h.instructions)));
                }
            });
        }
        Tab::Recompilation => match app.last_pipeline.as_ref().and_then(|s| s.report.as_ref()) {
            None => {
                widgets::section(ui, &p, "", |ui| {
                    ui.label(RichText::new("The pipeline has not run this session.").color(p.body));
                });
            }
            Some(r) => {
                widgets::section(ui, &p, "Lift report", |ui| {
                    let json = serde_json::to_string_pretty(r).unwrap_or_default();
                    for line in json.lines().filter(|l| matches(&filter, l)) {
                        ui.label(RichText::new(line).font(theme::mono(12.0)).color(p.body));
                    }
                });
            }
        },
        Tab::Log => {
            widgets::section(ui, &p, "Log files", |ui| {
                let dir = crate::jobs::log_dir();
                let folder = dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "APPDATA is not set".into());
                widgets::kv(ui, &p, "Folder", &folder);
                if let Some(path) = &app.last_job_log_path {
                    widgets::kv(ui, &p, "Last task", &path.display().to_string());
                }
                if let Some(dir) = &dir {
                    if widgets::button(ui, &p, "Open the log folder", true).clicked() {
                        let _ = std::process::Command::new("explorer").arg(dir).spawn();
                    }
                }
                ui.label(
                    RichText::new("Every task writes its own file there: what it ran, the whole output of the tools it called, and why it stopped.")
                        .size(12.5)
                        .color(p.muted),
                );
            });
            widgets::section(ui, &p, "Activity", |ui| {
                for a in app.activity.iter().rev().filter(|a| matches(&filter, &a.text)) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&a.at).font(theme::mono(11.5)).color(p.muted));
                        ui.label(RichText::new(&a.text).color(a.tone.colour(&p)));
                    });
                }
            });
            widgets::section(ui, &p, "Last task log", |ui| {
                for l in app.last_job_log.iter().filter(|l| matches(&filter, l)) {
                    ui.label(RichText::new(l).font(theme::mono(12.0)).color(p.body));
                }
            });
        }
    }
}

/// Write everything this page shows to a text file the user chooses.
fn export(app: &mut App) {
    let Some(path) = rfd::FileDialog::new().set_file_name("emblemweave-diagnostics.txt").save_file() else {
        return;
    };
    let mut out = String::new();
    out.push_str(&format!("EmblemWeave {} ({})\n\n", env!("CARGO_PKG_VERSION"), env!("EW_GIT_REV")));
    out.push_str("== game data ==\n");
    for c in &app.game.checks {
        out.push_str(&format!("{:?} {}: {}\n", c.severity, c.label, c.detail));
    }
    if let Some((r, ms)) = &app.last_boot {
        out.push_str(&format!("\n== runtime ({ms} ms) ==\n{}\n", r.headline()));
        for t in &r.thread_states {
            out.push_str(&format!("{t}\n"));
        }
        for l in &r.host_log {
            out.push_str(&format!("host: {l}\n"));
        }
        for i in &r.import_calls {
            out.push_str(&format!("import {} x{}\n", i.name, i.calls));
        }
        for e in &r.recent {
            out.push_str(&format!("event {e}\n"));
        }
    }
    if let Some(r) = app.last_pipeline.as_ref().and_then(|s| s.report.as_ref()) {
        out.push_str(&format!("\n== lift ==\n{}\n", serde_json::to_string_pretty(r).unwrap_or_default()));
    }
    out.push_str("\n== activity ==\n");
    for a in &app.activity {
        out.push_str(&format!("{} {}\n", a.at, a.text));
    }
    // What the build tools look like on this PC and the tail of the newest log:
    // the report a user attaches has to carry the reason, not just the stage.
    out.push_str(&crate::crash::environment_text());
    match std::fs::write(&path, out) {
        Ok(()) => app.notify(format!("Diagnostics written to {}", path.display()), Tone::Ok),
        Err(e) => app.notify(format!("Could not write the report: {e}"), Tone::Bad),
    }
}
