//! Controls: the pad the game reads, its tuning in the engine's
//! `controls.ini`, and the bindings for EmblemWeave's own engine.

use crate::pad::{self, button};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText};
use std::time::Duration;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Controls", "Bind the game's actions to your keyboard and controller.");

    // The pad as it is this frame: the page follows it while it is open.
    let live = pad::poll();
    ui.ctx().request_repaint_after(Duration::from_millis(20));

    widgets::pending_note(
        ui,
        &p,
        "The game reads player 1 from the first XInput gamepad, with buttons mapped by position to the Nintendo layout. The pad settings below are written to the game's controls.ini, which is where it reads them.",
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

    widgets::section(ui, &p, "Controller", |ui| match live {
        Some(l) => {
            widgets::status_line(
                ui,
                &p,
                Tone::Ok,
                &format!("Controller {} connected — listening", l.slot + 1),
                "Press a button or move a stick and the map below follows it. The game uses this pad as player 1.",
            );
            ui.add_space(10.0);
            egui::Grid::new("live-pad").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                for (label, text) in [
                    ("Left stick", format!("{:+.2}, {:+.2}", l.left.0, l.left.1)),
                    ("Right stick", format!("{:+.2}, {:+.2}", l.right.0, l.right.1)),
                    ("ZL travel", format!("{:.0} %", l.lt * 100.0)),
                    ("ZR travel", format!("{:.0} %", l.rt * 100.0)),
                ] {
                    ui.label(RichText::new(label).size(12.5).color(p.muted));
                    ui.label(RichText::new(text).font(theme::mono(12.5)).color(p.ink));
                    ui.end_row();
                }
            });
        }
        None => widgets::status_line(
            ui,
            &p,
            Tone::Neutral,
            "No controller connected",
            "Plug one in and it is picked up on the next frame: the game reads player 1 from the first XInput pad. Keyboard bindings work meanwhile.",
        ),
    });

    // The map sits first, where it is fully in view: it is the page's picture
    // of the pad, and the table below is the same bindings in list form.
    widgets::section(ui, &p, "Controller map", |ui| {
        ui.label(
            RichText::new("Where each action sits on the pad. Hover a button for its binding; the table below edits it.")
                .size(12.5)
                .color(p.muted),
        );
        ui.vertical_centered(|ui| controller_map(ui, &p, &app.config.input.bindings, live));
    });

    widgets::section(ui, &p, "Pad tuning", |ui| pad_tuning(app, ui, &p));

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

/// A slider in whole percent, returning the new value when it moved.
fn percent(ui: &mut egui::Ui, value: f32, range: std::ops::RangeInclusive<u32>) -> Option<u32> {
    let mut pct = (value * 100.0).round() as u32;
    let changed = ui.add(egui::Slider::new(&mut pct, range).suffix(" %")).changed();
    changed.then_some(pct)
}

/// The four `[pad]` values the game reads, written into its `controls.ini` as
/// they change.
fn pad_tuning(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    let Some(ini) = crate::ingame::controls_ini() else {
        widgets::status_line(ui, p, Tone::Warn, "No settings folder", "APPDATA is not set, so the launcher cannot write the game's controls.ini.");
        return;
    };
    let seed = app.config.input.deadzone;
    let mut t = match app.tuning {
        Some(t) => t,
        None => {
            let t = pad::Tuning::load(&ini, seed);
            app.tuning = Some(t);
            t
        }
    };
    let mut save: Option<(&'static str, f32)> = None;

    widgets::setting_row(ui, p, "Left stick dead zone", "Movement below this is ignored.", |ui| {
        if let Some(pct) = percent(ui, t.left_deadzone, 0..=50) {
            t.left_deadzone = pct as f32 / 100.0;
            save = Some(("deadzone", t.left_deadzone));
        }
    });
    widgets::setting_row(ui, p, "Right stick dead zone", "Camera movement below this is ignored.", |ui| {
        if let Some(pct) = percent(ui, t.right_deadzone, 0..=50) {
            t.right_deadzone = pct as f32 / 100.0;
            save = Some(("right_deadzone", t.right_deadzone));
        }
    });
    widgets::setting_row(ui, p, "Full tilt", "How far the stick must travel to count as fully pushed. Lower it for a worn pad that tops out early.", |ui| {
        if let Some(pct) = percent(ui, t.stick_max, 50..=100) {
            t.stick_max = pct as f32 / 100.0;
            save = Some(("stick_max", t.stick_max));
        }
    });
    widgets::setting_row(ui, p, "Trigger threshold", "Trigger travel that counts as ZL or ZR pressed.", |ui| {
        if let Some(pct) = percent(ui, t.trigger, 5..=100) {
            t.trigger = pct as f32 / 100.0;
            save = Some(("trigger_threshold", t.trigger));
        }
    });
    widgets::setting_row(ui, p, "Vibration", "Kept by the launcher; the game's own rumble level is in its F1 menu.", |ui| {
        let mut pct = (app.config.input.vibration * 100.0).round() as u32;
        if ui.add(egui::Slider::new(&mut pct, 0..=100).suffix(" %")).changed() {
            app.config.input.vibration = pct as f32 / 100.0;
        }
    });

    if let Some((key, v)) = save {
        match pad::write(&ini, key, v) {
            Ok(()) => {
                if key == "deadzone" {
                    app.config.input.deadzone = v;
                }
                app.tuning = Some(t);
            }
            Err(e) => app.notify(e, Tone::Bad),
        }
    } else {
        app.tuning = Some(t);
    }

    ui.add_space(theme::SPACE_S);
    ui.horizontal(|ui| {
        if widgets::button(ui, p, "Re-read controls.ini", true).clicked() {
            app.tuning = Some(pad::Tuning::load(&ini, app.config.input.deadzone));
        }
        ui.label(RichText::new(ini.display().to_string()).font(theme::mono(11.5)).color(p.muted));
    });
    ui.label(
        RichText::new("Written where the game reads them: it takes them at its next start, and F1 > Controller applies them at once in-game.")
            .size(12.0)
            .color(p.muted),
    );
}

/// One binding's place on the pad: the action it belongs to, the caption drawn
/// beside it, the button face drawn in a chip (empty for the stick and D-pad
/// entries), where each sits in 0..1 of the map, and the pad button that
/// lights it.
struct Spot {
    action: &'static str,
    caption: &'static str,
    glyph: &'static str,
    at: (f32, f32),
    cap: (f32, f32),
    /// The XInput button for this spot; 0 for the sticks and triggers, which
    /// have no button bit.
    pad: u16,
}

/// The game's pad, button by button. Bindings are matched by action name, so
/// the map follows whatever the table holds rather than a copy of it.
const SPOTS: [Spot; 15] = [
    Spot { action: "Confirm", caption: "Confirm", glyph: "A", at: (0.705, 0.400), cap: (0.705, 0.442), pad: button::EAST },
    Spot { action: "Cancel", caption: "Cancel", glyph: "B", at: (0.780, 0.335), cap: (0.780, 0.377), pad: button::SOUTH },
    Spot { action: "Menu", caption: "Menu", glyph: "X", at: (0.630, 0.335), cap: (0.630, 0.377), pad: button::NORTH },
    Spot { action: "Unit details", caption: "Details", glyph: "Y", at: (0.705, 0.278), cap: (0.705, 0.232), pad: button::WEST },
    Spot { action: "Previous unit", caption: "Previous", glyph: "L", at: (0.190, 0.150), cap: (0.190, 0.192), pad: button::LEFT_SHOULDER },
    Spot { action: "Next unit", caption: "Next", glyph: "R", at: (0.810, 0.150), cap: (0.810, 0.192), pad: button::RIGHT_SHOULDER },
    Spot { action: "Danger zone", caption: "Danger", glyph: "ZL", at: (0.115, 0.062), cap: (0.115, 0.107), pad: 0 },
    Spot { action: "Fast forward", caption: "Fast fwd", glyph: "ZR", at: (0.885, 0.062), cap: (0.885, 0.107), pad: 0 },
    Spot { action: "End turn", caption: "End turn", glyph: "+", at: (0.565, 0.195), cap: (0.565, 0.237), pad: button::START },
    Spot { action: "Map", caption: "Map", glyph: "\u{2212}", at: (0.435, 0.195), cap: (0.435, 0.237), pad: button::BACK },
    Spot { action: "Cursor up", caption: "Up", glyph: "", at: (0.300, 0.215), cap: (0.300, 0.215), pad: button::DPAD_UP },
    Spot { action: "Cursor down", caption: "Down", glyph: "", at: (0.300, 0.478), cap: (0.300, 0.478), pad: button::DPAD_DOWN },
    Spot { action: "Cursor left", caption: "Left", glyph: "", at: (0.176, 0.350), cap: (0.176, 0.350), pad: button::DPAD_LEFT },
    Spot { action: "Cursor right", caption: "Right", glyph: "", at: (0.424, 0.350), cap: (0.424, 0.350), pad: button::DPAD_RIGHT },
    Spot { action: "Camera", caption: "Camera", glyph: "", at: (0.670, 0.655), cap: (0.670, 0.655), pad: 0 },
];

/// The pad, drawn as a path so every scheme tints it without an asset: the
/// outline takes the scheme's accent, the buttons its surfaces, and a pad that
/// is connected lights each control as it is used.
fn controller_map(ui: &mut egui::Ui, p: &theme::Palette, bindings: &[crate::config::Binding], live: Option<pad::Live>) {
    let width = ui.available_width().min(660.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, width * 0.55), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let at = |x: f32, y: f32| egui::pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height());

    // A pad's silhouette from a handful of anchors, smoothed so it scales
    // cleanly at any width.
    let anchors = [
        (0.500, 0.150), (0.600, 0.138), (0.700, 0.128), (0.780, 0.132), (0.850, 0.165),
        (0.900, 0.235), (0.930, 0.360), (0.930, 0.520), (0.910, 0.690), (0.875, 0.800),
        (0.815, 0.870), (0.745, 0.865), (0.695, 0.800), (0.660, 0.760), (0.590, 0.740),
        (0.500, 0.735), (0.410, 0.740), (0.340, 0.760), (0.305, 0.800), (0.255, 0.865),
        (0.185, 0.870), (0.125, 0.800), (0.090, 0.690), (0.070, 0.520), (0.070, 0.360),
        (0.100, 0.235), (0.150, 0.165), (0.220, 0.132), (0.300, 0.128), (0.400, 0.138),
    ];
    let points: Vec<egui::Pos2> = smooth_closed(&anchors).into_iter().map(|(x, y)| at(x, y)).collect();
    for (w, a) in [(9.0_f32, 14u8), (4.5_f32, 40)] {
        painter.add(egui::Shape::closed_line(points.clone(), egui::epaint::PathStroke::new(w, theme::alpha(p.accent, a))));
    }
    painter.add(egui::Shape::closed_line(points, egui::epaint::PathStroke::new(2.0_f32, p.accent)));

    // The D-pad is one cross rather than four chips, so up, down, left and
    // right read as a single control. Each arm lights on its own.
    let cross = at(0.300, 0.350);
    let (half, arm) = (rect.width() * 0.050, rect.width() * 0.015);
    for (bit, dx, dy, size) in [
        (button::DPAD_UP, 0.0, -half * 0.5, egui::vec2(arm, half)),
        (button::DPAD_DOWN, 0.0, half * 0.5, egui::vec2(arm, half)),
        (button::DPAD_LEFT, -half * 0.5, 0.0, egui::vec2(half, arm)),
        (button::DPAD_RIGHT, half * 0.5, 0.0, egui::vec2(half, arm)),
    ] {
        let on = live.is_some_and(|l| l.held(bit));
        let colour = theme::alpha(p.accent, if on { 210 } else { 70 });
        painter.rect_filled(egui::Rect::from_center_size(cross + egui::vec2(dx, dy), size), 3, colour);
    }

    // The two sticks: outlines only, as the pad has them, with the stick's
    // position shown inside while a pad is connected.
    for (x, stick) in [(0.330, live.map(|l| l.left)), (0.670, live.map(|l| l.right))] {
        let c = at(x, 0.575);
        let r = rect.width() * 0.030;
        painter.circle_stroke(c, r, egui::Stroke::new(1.5_f32, theme::alpha(p.accent, 130)));
        if let Some((vx, vy)) = stick {
            let dot = c + egui::vec2(vx * r * 0.72, -vy * r * 0.72);
            let moved = vx.abs().max(vy.abs()) > 0.02;
            painter.circle_filled(dot, r * 0.30, if moved { p.accent } else { theme::alpha(p.accent, 90) });
        }
    }

    for (i, s) in SPOTS.iter().enumerate() {
        let bound = bindings.iter().find(|b| b.action.eq_ignore_ascii_case(s.action));
        let held = live.is_some_and(|l| l.held(s.pad));
        // The triggers are analog: their chips fill with the travel.
        let travel = match s.action {
            "Danger zone" => live.map_or(0.0, |l| l.lt),
            "Fast forward" => live.map_or(0.0, |l| l.rt),
            _ => 0.0,
        };
        if s.glyph.is_empty() {
            let colour = if held || travel > 0.1 { p.accent } else if bound.is_some() { p.body } else { theme::alpha(p.muted, 120) };
            painter.text(at(s.cap.0, s.cap.1), egui::Align2::CENTER_CENTER, s.caption, egui::FontId::new(10.0, egui::FontFamily::Proportional), colour);
            continue;
        }
        let c = at(s.at.0, s.at.1);
        let w = 22.0 + 6.0 * s.glyph.chars().count().saturating_sub(1) as f32;
        let chip = egui::Rect::from_center_size(c, egui::vec2(w, 22.0));
        let on = bound.is_some();
        let base = if on { 34.0 } else { 10.0 };
        let fill = (base + travel * 120.0) as u8;
        let fill = if held { fill.max(150) } else { fill };
        let edge = if held { 220 } else if on { 170 } else { 55 };
        painter.rect_filled(chip, egui::CornerRadius::same(6), theme::alpha(p.accent, fill));
        painter.rect_stroke(chip, egui::CornerRadius::same(6), egui::Stroke::new(1.0_f32, theme::alpha(p.accent, edge)), egui::StrokeKind::Inside);
        painter.text(c, egui::Align2::CENTER_CENTER, s.glyph, theme::semibold(11.0), if on { p.ink } else { theme::alpha(p.ink, 90) });
        let cap_colour = if held { p.accent } else if on { p.body } else { theme::alpha(p.muted, 120) };
        painter.text(at(s.cap.0, s.cap.1), egui::Align2::CENTER_CENTER, s.caption, egui::FontId::new(10.0, egui::FontFamily::Proportional), cap_colour);
        if let Some(b) = bound {
            let hover = ui.interact(chip.expand(9.0), ui.id().with(("pad-spot", i)), egui::Sense::hover());
            hover.on_hover_text(format!("{}  ·  {}", b.action, b.pad));
        }
    }
}

/// A closed Catmull-Rom curve through `anchors`, ten points per span.
fn smooth_closed(anchors: &[(f32, f32)]) -> Vec<(f32, f32)> {
    const STEPS: usize = 10;
    let n = anchors.len();
    let mut out = Vec::with_capacity(n * STEPS);
    for i in 0..n {
        let (p0, p1) = (anchors[(i + n - 1) % n], anchors[i]);
        let (p2, p3) = (anchors[(i + 1) % n], anchors[(i + 2) % n]);
        for s in 0..STEPS {
            out.push(catmull_rom(p0, p1, p2, p3, s as f32 / STEPS as f32));
        }
    }
    out
}

fn catmull_rom(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32), t: f32) -> (f32, f32) {
    let f = |a: f32, b: f32, c: f32, d: f32| {
        let (t2, t3) = (t * t, t * t * t);
        0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3)
    };
    (f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1))
}

#[cfg(test)]
mod tests {
    use super::SPOTS;
    use crate::config::default_bindings;

    /// Every binding the game ships with has somewhere to sit on the map.
    #[test]
    fn every_default_binding_has_a_place_on_the_pad() {
        for b in default_bindings() {
            assert!(SPOTS.iter().any(|s| s.action == b.action), "{} has no place on the controller map", b.action);
        }
    }

    /// Every named button lights exactly one spot, and the four D-pad arms
    /// light the four cursor spots.
    #[test]
    fn each_pad_button_lights_one_spot() {
        for (glyph, bit) in [
            ("A", crate::pad::button::EAST),
            ("B", crate::pad::button::SOUTH),
            ("X", crate::pad::button::NORTH),
            ("Y", crate::pad::button::WEST),
            ("L", crate::pad::button::LEFT_SHOULDER),
            ("R", crate::pad::button::RIGHT_SHOULDER),
            ("+", crate::pad::button::START),
            ("\u{2212}", crate::pad::button::BACK),
        ] {
            let lit: Vec<_> = SPOTS.iter().filter(|s| s.pad == bit).collect();
            assert_eq!(lit.len(), 1, "{glyph} lights {} spots", lit.len());
        }
        let arms = [
            crate::pad::button::DPAD_UP,
            crate::pad::button::DPAD_DOWN,
            crate::pad::button::DPAD_LEFT,
            crate::pad::button::DPAD_RIGHT,
        ];
        let cursors: Vec<_> = SPOTS.iter().filter(|s| s.action.starts_with("Cursor")).collect();
        assert_eq!(cursors.len(), 4);
        for (s, bit) in cursors.iter().zip(arms) {
            assert_eq!(s.pad, bit, "{} has the wrong D-pad button", s.action);
        }
    }
}
