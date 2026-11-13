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

/// How a button is drawn: the pad's own round buttons, its square, and the
/// shoulder buttons, which the pad outline has no shape for.
#[derive(Clone, Copy, PartialEq)]
enum Face {
    Circle,
    Square,
    Keycap,
}

/// One binding's place on the pad: the action it belongs to, the caption drawn
/// beside it, the glyph in the button, where it sits in 0..1 of the map, its
/// radius in 0..1 of the map's width, where its caption goes, the pad button
/// that lights it, and how it is drawn.
struct Spot {
    action: &'static str,
    caption: &'static str,
    glyph: &'static str,
    at: (f32, f32),
    r: f32,
    cap: (f32, f32),
    pad: u16,
    face: Face,
}

/// The game's pad, button by button, on the reference layout: the left stick
/// and D-pad on the left, the face buttons and right stick on the right, the
/// square and the small round button between them, and the shoulders on top.
/// Bindings are matched by action name, so the map follows whatever the table
/// holds rather than a copy of it.
const SPOTS: [Spot; 15] = [
    Spot { action: "Confirm", caption: "Confirm", glyph: "A", at: (0.765, 0.435), r: 0.034, cap: (0.765, 0.508), pad: button::EAST, face: Face::Circle },
    Spot { action: "Cancel", caption: "Cancel", glyph: "B", at: (0.846, 0.332), r: 0.030, cap: (0.846, 0.400), pad: button::SOUTH, face: Face::Circle },
    Spot { action: "Menu", caption: "Menu", glyph: "X", at: (0.762, 0.243), r: 0.034, cap: (0.762, 0.176), pad: button::NORTH, face: Face::Circle },
    Spot { action: "Unit details", caption: "Details", glyph: "Y", at: (0.668, 0.355), r: 0.036, cap: (0.645, 0.428), pad: button::WEST, face: Face::Circle },
    Spot { action: "Previous unit", caption: "Previous", glyph: "L", at: (0.175, 0.062), r: 0.0, cap: (0.175, 0.112), pad: button::LEFT_SHOULDER, face: Face::Keycap },
    Spot { action: "Next unit", caption: "Next", glyph: "R", at: (0.825, 0.062), r: 0.0, cap: (0.825, 0.112), pad: button::RIGHT_SHOULDER, face: Face::Keycap },
    Spot { action: "Danger zone", caption: "Danger", glyph: "ZL", at: (0.075, 0.120), r: 0.0, cap: (0.075, 0.170), pad: 0, face: Face::Keycap },
    Spot { action: "Fast forward", caption: "Fast fwd", glyph: "ZR", at: (0.925, 0.120), r: 0.0, cap: (0.925, 0.170), pad: 0, face: Face::Keycap },
    Spot { action: "End turn", caption: "End turn", glyph: "+", at: (0.431, 0.342), r: 0.022, cap: (0.431, 0.400), pad: button::START, face: Face::Square },
    Spot { action: "Map", caption: "Map", glyph: "\u{2212}", at: (0.377, 0.247), r: 0.019, cap: (0.377, 0.298), pad: button::BACK, face: Face::Circle },
    Spot { action: "Cursor up", caption: "Up", glyph: "", at: (0.343, 0.398), r: 0.0, cap: (0.343, 0.398), pad: button::DPAD_UP, face: Face::Circle },
    Spot { action: "Cursor down", caption: "Down", glyph: "", at: (0.343, 0.622), r: 0.0, cap: (0.343, 0.622), pad: button::DPAD_DOWN, face: Face::Circle },
    Spot { action: "Cursor left", caption: "Left", glyph: "", at: (0.232, 0.505), r: 0.0, cap: (0.232, 0.505), pad: button::DPAD_LEFT, face: Face::Circle },
    Spot { action: "Cursor right", caption: "Right", glyph: "", at: (0.458, 0.505), r: 0.0, cap: (0.458, 0.505), pad: button::DPAD_RIGHT, face: Face::Circle },
    Spot { action: "Camera", caption: "Camera", glyph: "", at: (0.635, 0.668), r: 0.0, cap: (0.635, 0.668), pad: 0, face: Face::Circle },
];

/// The pad, drawn as a path from the reference outline so every scheme tints
/// it without an asset: the outline takes the scheme's accent, and a connected
/// pad lights each control as it is used.
fn controller_map(ui: &mut egui::Ui, p: &theme::Palette, bindings: &[crate::config::Binding], live: Option<pad::Live>) {
    let width = ui.available_width().min(520.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, width * 0.716), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let at = |x: f32, y: f32| egui::pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height());
    let line = |on: bool, rest: u8, lit: u8| theme::alpha(p.accent, if on { lit } else { rest });

    // The reference silhouette, clockwise from the bottom of the notch under the
    // small round button: one smooth dip there, then the flat top, the shoulder
    // steps, the body, the grips and the flat bottom. The dip is two anchors at
    // the same height either side of its lowest point, so the curve rounds it
    // instead of kinking.
    let anchors = [
        (0.403, 0.095), (0.415, 0.095), (0.430, 0.075), (0.480, 0.068), (0.560, 0.066),
        (0.635, 0.066), (0.690, 0.076), (0.740, 0.094), (0.775, 0.120), (0.815, 0.148),
        (0.860, 0.185), (0.895, 0.215), (0.925, 0.300), (0.945, 0.400), (0.960, 0.520),
        (0.965, 0.640), (0.955, 0.760), (0.930, 0.860), (0.890, 0.920), (0.840, 0.940),
        (0.790, 0.920), (0.760, 0.860), (0.740, 0.790), (0.700, 0.745), (0.620, 0.735),
        (0.520, 0.735), (0.420, 0.735), (0.340, 0.745), (0.300, 0.790), (0.280, 0.860),
        (0.250, 0.920), (0.200, 0.940), (0.150, 0.920), (0.110, 0.860), (0.085, 0.760),
        (0.075, 0.640), (0.080, 0.520), (0.095, 0.400), (0.115, 0.300), (0.145, 0.215),
        (0.180, 0.185), (0.215, 0.145), (0.245, 0.118), (0.290, 0.092), (0.340, 0.078),
        (0.386, 0.072),
    ];
    let points: Vec<egui::Pos2> = smooth_closed(&anchors).into_iter().map(|(x, y)| at(x, y)).collect();
    for (w, a) in [(9.0_f32, 14u8), (4.5_f32, 40)] {
        painter.add(egui::Shape::closed_line(points.clone(), egui::epaint::PathStroke::new(w, theme::alpha(p.accent, a))));
    }
    painter.add(egui::Shape::closed_line(points, egui::epaint::PathStroke::new(2.0_f32, p.accent)));

    // The shapes the reference pad has but no action uses: two small buttons
    // beside the face buttons. (The reference's third, in the notch, sat on the
    // outline at this size, so the notch alone carries it.)
    for (x, y, r) in [(0.623, 0.241, 0.022), (0.563, 0.346, 0.021)] {
        painter.circle_stroke(at(x, y), r * rect.width(), egui::Stroke::new(1.5_f32, theme::alpha(p.accent, 90)));
    }

    // The two sticks: the reference's double rings, with the stick's position
    // shown inside while a pad is connected.
    for (x, y, stick) in [(0.229, 0.342, live.map(|l| l.left)), (0.635, 0.540, live.map(|l| l.right))] {
        let c = at(x, y);
        let r = rect.width() * 0.066;
        painter.circle_stroke(c, r, egui::Stroke::new(2.0_f32, theme::alpha(p.accent, 150)));
        painter.circle_stroke(c, rect.width() * 0.045, egui::Stroke::new(1.5_f32, theme::alpha(p.accent, 90)));
        if let Some((vx, vy)) = stick {
            let dot = c + egui::vec2(vx * r * 0.66, -vy * r * 0.66);
            let moved = vx.abs().max(vy.abs()) > 0.02;
            painter.circle_filled(dot, r * 0.26, if moved { p.accent } else { theme::alpha(p.accent, 90) });
        }
    }

    // The D-pad is one cross, as the reference draws it. Each arm lights on
    // its own.
    let cross = at(0.343, 0.505);
    let half = egui::vec2(rect.width() * 0.071, rect.height() * 0.085);
    let arm = rect.width() * 0.045;
    for (bit, dx, dy, size) in [
        (button::DPAD_UP, 0.0, -half.y * 0.5, egui::vec2(arm, half.y)),
        (button::DPAD_DOWN, 0.0, half.y * 0.5, egui::vec2(arm, half.y)),
        (button::DPAD_LEFT, -half.x * 0.5, 0.0, egui::vec2(half.x, arm)),
        (button::DPAD_RIGHT, half.x * 0.5, 0.0, egui::vec2(half.x, arm)),
    ] {
        let on = live.is_some_and(|l| l.held(bit));
        painter.rect_filled(egui::Rect::from_center_size(cross + egui::vec2(dx, dy), size), 3, line(on, 70, 210));
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
        let on = bound.is_some();
        let base = if on { 34.0 } else { 10.0 };
        let fill = if held { (base + travel * 120.0).max(150.0) } else { base + travel * 120.0 } as u8;
        let edge = if held { 220 } else if on { 170 } else { 55 };
        let c = at(s.at.0, s.at.1);

        if s.glyph.is_empty() {
            // The D-pad arms and the sticks carry no face of their own: their
            // captions are the control's label.
            let colour = if held { p.accent } else if bound.is_some() { p.body } else { theme::alpha(p.muted, 120) };
            painter.text(at(s.cap.0, s.cap.1), egui::Align2::CENTER_CENTER, s.caption, egui::FontId::new(10.0, egui::FontFamily::Proportional), colour);
            continue;
        }

        let hit = match s.face {
            Face::Circle => {
                let r = s.r * rect.width();
                painter.circle_filled(c, r, theme::alpha(p.accent, fill));
                painter.circle_stroke(c, r, egui::Stroke::new(1.5_f32, theme::alpha(p.accent, edge)));
                egui::Rect::from_center_size(c, egui::vec2(r, r) * 2.0)
            }
            Face::Square => {
                let side = s.r * rect.width() * 2.0;
                let sq = egui::Rect::from_center_size(c, egui::vec2(side, side));
                painter.rect_filled(sq, egui::CornerRadius::same(3), theme::alpha(p.accent, fill));
                painter.rect_stroke(sq, egui::CornerRadius::same(3), egui::Stroke::new(1.5_f32, theme::alpha(p.accent, edge)), egui::StrokeKind::Inside);
                sq
            }
            Face::Keycap => {
                let w = 22.0 + 6.0 * s.glyph.chars().count().saturating_sub(1) as f32;
                let chip = egui::Rect::from_center_size(c, egui::vec2(w, 22.0));
                painter.rect_filled(chip, egui::CornerRadius::same(6), theme::alpha(p.accent, fill));
                painter.rect_stroke(chip, egui::CornerRadius::same(6), egui::Stroke::new(1.0_f32, theme::alpha(p.accent, edge)), egui::StrokeKind::Inside);
                chip
            }
        };
        painter.text(c, egui::Align2::CENTER_CENTER, s.glyph, theme::semibold(11.0), if on { p.ink } else { theme::alpha(p.ink, 90) });
        let cap_colour = if held { p.accent } else if on { p.body } else { theme::alpha(p.muted, 120) };
        painter.text(at(s.cap.0, s.cap.1), egui::Align2::CENTER_CENTER, s.caption, egui::FontId::new(10.0, egui::FontFamily::Proportional), cap_colour);
        if let Some(b) = bound {
            let hover = ui.interact(hit.expand(9.0), ui.id().with(("pad-spot", i)), egui::Sense::hover());
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
