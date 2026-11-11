//! Controls: keyboard and controller bindings for the game's actions.

use crate::theme;
use crate::widgets;
use crate::App;
use eframe::egui::{self, RichText};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Controls", "Bind the game's actions to your keyboard and controller.");
    widgets::pending_note(
        ui,
        &p,
        "The game reads player 1 from the first XInput gamepad, with buttons mapped by position to the Nintendo layout. Keyboard bindings are saved and apply once the game has its own window.",
    );

    // Key capture: the next key press becomes the binding.
    if let Some(i) = app.capture {
        let pressed = ui.input(|inp| {
            inp.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, .. } => Some(*key),
                _ => None,
            })
        });
        if let Some(key) = pressed {
            if key != egui::Key::Escape {
                if let Some(b) = app.config.input.bindings.get_mut(i) {
                    b.key = key.name().to_string();
                }
            }
            app.capture = None;
        }
    }

    widgets::section(ui, &p, "Profile", |ui| {
        widgets::setting_row(ui, &p, "Stick dead zone", "Movement below this is ignored.", |ui| {
            let mut pct = (app.config.input.deadzone * 100.0).round() as u32;
            if ui.add(egui::Slider::new(&mut pct, 0..=50).suffix(" %")).changed() {
                app.config.input.deadzone = pct as f32 / 100.0;
            }
        });
        widgets::setting_row(ui, &p, "Vibration", "", |ui| {
            let mut pct = (app.config.input.vibration * 100.0).round() as u32;
            if ui.add(egui::Slider::new(&mut pct, 0..=100).suffix(" %")).changed() {
                app.config.input.vibration = pct as f32 / 100.0;
            }
        });
    });

    widgets::section(ui, &p, "Bindings", |ui| {
        egui::Grid::new("bindings").num_columns(3).spacing([24.0, 8.0]).striped(true).show(ui, |ui| {
            for h in ["Action", "Keyboard", "Controller"] {
                ui.label(RichText::new(h).font(theme::semibold(12.0)).color(p.muted));
            }
            ui.end_row();
            for (i, b) in app.config.input.bindings.iter_mut().enumerate() {
                ui.label(RichText::new(&b.action).color(p.ink));
                let capturing = app.capture == Some(i);
                let text = if capturing { "Press a key… (Esc cancels)".to_string() } else { b.key.clone() };
                let r = ui.add_sized([210.0, 26.0], egui::Button::new(RichText::new(text).font(theme::mono(12.5))).selected(capturing));
                if r.on_hover_text("Click, then press the key to bind").clicked() {
                    app.capture = Some(i);
                }
                ui.add(egui::TextEdit::singleline(&mut b.pad).desired_width(220.0));
                ui.end_row();
            }
        });
        ui.add_space(theme::SPACE_M);
        if widgets::button(ui, &p, "Reset to defaults", true).clicked() {
            app.confirm = Some(crate::Confirm::ResetBindings);
        }
    });
}
