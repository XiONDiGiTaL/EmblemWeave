//! Appearance: choose the launcher's colour scheme from compact live previews.

use crate::config::InterfaceMode;
use crate::pages::Page;
use crate::theme::{self, Scheme};
use crate::widgets::{self, Tone};
use crate::{logo, App};
use eframe::egui::{self, RichText, Vec2};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    let header = format!("{} coordinated schemes. Each preview uses its own palette and logo colourway; the choice is saved.", Scheme::ALL.len());
    widgets::page_header(ui, &p, "Appearance", &header);
    ui.add_space(2.0);
    widgets::section(ui, &p, "Interface", |ui| {
        ui.label(
            RichText::new("Choose how much of the launcher you want to see. Both modes use the same installation, settings, saves and build.")
                .size(12.0)
                .color(p.body),
        );
        ui.add_space(7.0);
        ui.horizontal_wrapped(|ui| {
            for mode in [InterfaceMode::Standard, InterfaceMode::Advanced] {
                if app.config.interface_mode == mode {
                    let _ = widgets::primary_button(ui, &p, mode.label(), true);
                    widgets::pill(ui, &p, "Current", Tone::Accent);
                } else if widgets::button(ui, &p, mode.label(), true).clicked() {
                    set_mode(app, mode);
                }
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new(app.config.interface_mode.detail()).size(11.0).color(p.muted));
        ui.label(
            RichText::new("Advanced pages are hidden in Standard mode, not removed. Switch back at any time; no setting or installation data changes.")
                .size(10.5)
                .color(p.muted),
        );
    });
    ui.add_space(6.0);
    let mut chosen = None;

    let available = ui.available_width();
    let cols = if available >= 880.0 { 3 } else if available >= 560.0 { 2 } else { 1 };
    let gap = 10.0;
    let cell = ((available - gap * (cols - 1) as f32) / cols as f32).max(240.0);
    for row_schemes in Scheme::ALL.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for scheme in row_schemes {
                let sp = scheme.palette();
                let active = app.config.theme == *scheme;
                let mut clicked = false;
                ui.allocate_ui_with_layout(Vec2::new(cell, 226.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(cell);
                    let response = egui::Frame::new()
                        .fill(sp.bg)
                        .stroke(egui::Stroke::new(if active { 1.5_f32 } else { 1.0_f32 }, if active { sp.electric } else { sp.line }))
                        .corner_radius(egui::CornerRadius::same(12))
                        .inner_margin(egui::Margin::same(14))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.set_min_height(196.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(scheme.label()).font(theme::display(16.0)).color(sp.ink));
                                if active {
                                    widgets::pill(ui, &sp, "Active", Tone::Accent);
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    for c in [sp.accent, sp.electric, sp.gold] {
                                        let (r, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), egui::Sense::hover());
                                        ui.painter().circle_filled(r.center(), 4.0, c);
                                    }
                                });
                            });
                            ui.add_space(3.0);
                            logo::show_lockup(ui, &sp, (cell - 28.0).min(154.0));
                            ui.add_space(5.0);
                            ui.add(egui::Label::new(RichText::new(scheme.detail()).size(10.5).color(sp.body)).wrap());
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                for (name, c) in [
                                    ("Background", sp.bg),
                                    ("Surface", sp.surface),
                                    ("Text", sp.ink),
                                    ("Accent", sp.accent),
                                    ("Highlight", sp.electric),
                                ] {
                                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(18.0, 12.0), egui::Sense::hover());
                                    ui.painter().rect_filled(rect, 3, c);
                                    ui.painter().rect_stroke(rect, 3, egui::Stroke::new(1.0_f32, sp.line), egui::StrokeKind::Inside);
                                    resp.on_hover_text(format!("{name}  #{:02X}{:02X}{:02X}", c.r(), c.g(), c.b()));
                                }
                            });
                            if !active && widgets::button(ui, &sp, "Use this scheme", true).clicked() {
                                chosen = Some(*scheme);
                            }
                        });
                    if response.response.interact(egui::Sense::click()).clicked() && !active {
                        clicked = true;
                    }
                    if clicked {
                        chosen = Some(*scheme);
                    }
                });
            }
        });
        ui.add_space(gap);
    }
    if let Some(s) = chosen {
        app.config.theme = s;
        app.note(format!("Theme set to {}", s.label()), Tone::Neutral);
    }
}

fn set_mode(app: &mut App, mode: InterfaceMode) {
    if app.config.interface_mode == mode {
        return;
    }
    app.config.interface_mode = mode;
    if !app.page.available(mode) {
        app.page = Page::Overview;
    }
    app.note(format!("{} mode selected", mode.label()), Tone::Neutral);
}
