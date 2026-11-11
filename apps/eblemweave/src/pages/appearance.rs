//! Appearance: choose the launcher's colour scheme from live previews.

use crate::theme::{self, Scheme};
use crate::widgets::{self, Tone};
use crate::{logo, App};
use eframe::egui::{self, RichText};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Appearance", "Three coordinated schemes. Each preview is drawn in its own palette; the choice is saved.");
    let mut chosen = None;
    for scheme in Scheme::ALL {
        let sp = scheme.palette();
        let active = app.config.theme == scheme;
        let r = egui::Frame::new()
            .fill(sp.bg)
            .stroke(egui::Stroke::new(if active { 2.0_f32 } else { 1.0_f32 }, if active { sp.electric } else { sp.line }))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS_M + 2))
            .inner_margin(egui::Margin::same(18))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    logo::show_lockup(ui, &sp, 300.0);
                    ui.add_space(theme::SPACE_L);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(scheme.label()).font(theme::display(22.0)).color(sp.ink));
                            if active {
                                widgets::pill(ui, &sp, "Active", Tone::Accent);
                            }
                        });
                        ui.label(RichText::new(scheme.detail()).size(13.0).color(sp.body));
                        ui.add_space(theme::SPACE_S);
                        ui.horizontal(|ui| {
                            for (name, c) in [
                                ("Background", sp.bg),
                                ("Surface", sp.surface),
                                ("Border", sp.line),
                                ("Text", sp.ink),
                                ("Accent", sp.accent),
                                ("Highlight", sp.electric),
                                ("Gold", sp.gold),
                            ] {
                                let (rect, resp) = ui.allocate_exact_size(egui::vec2(30.0, 20.0), egui::Sense::hover());
                                ui.painter().rect_filled(rect, 4, c);
                                ui.painter().rect_stroke(rect, 4, egui::Stroke::new(1.0_f32, sp.line), egui::StrokeKind::Inside);
                                resp.on_hover_text(format!("{name}  #{:02X}{:02X}{:02X}", c.r(), c.g(), c.b()));
                            }
                        });
                        ui.add_space(theme::SPACE_S);
                        if !active && widgets::button(ui, &sp, "Use this scheme", true).clicked() {
                            chosen = Some(scheme);
                        }
                    });
                });
            });
        if r.response.interact(egui::Sense::click()).clicked() && !active {
            chosen = Some(scheme);
        }
        ui.add_space(theme::SPACE_M);
    }
    if let Some(s) = chosen {
        app.config.theme = s;
        app.note(format!("Theme set to {}", s.label()), Tone::Neutral);
    }
}
