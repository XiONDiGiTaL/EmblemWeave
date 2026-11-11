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
                egui::ScrollArea::vertical().id_salt("imports").max_height(260.0).show(ui, |ui| {
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
                egui::ScrollArea::vertical().id_salt("recent").max_height(300.0).stick_to_bottom(true).show(ui, |ui| {
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
    match std::fs::write(&path, out) {
        Ok(()) => app.notify(format!("Diagnostics written to {}", path.display()), Tone::Ok),
        Err(e) => app.notify(format!("Could not write the report: {e}"), Tone::Bad),
    }
}
