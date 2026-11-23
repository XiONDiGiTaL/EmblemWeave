//! Controls: the pad the game reads, its tuning in the engine's
//! `controls.ini`, and the bindings for EmblemWeave's own engine.

use crate::pad::{self, button};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self};
use std::sync::Mutex;
use std::time::Duration;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Controls", "Bind the game's actions to your keyboard and controller.");

    // The pad as it is this frame: the page follows it while it is open.
    let live = pad::poll();
    // The pad readout follows the controller while a binding is being captured;
    // at rest a quarter second is enough to see a press, and the page no longer
    // redraws fifty times a second just to hold a still picture.
    let pace = if app.capture.is_some() { 20 } else { 250 };
    ui.ctx().request_repaint_after(Duration::from_millis(pace));

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
                ] {
                    ui.label(crate::i18n::text(label).size(12.5).color(p.muted));
                    ui.label(crate::i18n::text(text).font(theme::mono(12.5)).color(p.ink));
                    ui.end_row();
                }
            });
            ui.add_space(10.0);
            // Travel reads better as a filled bar than as a number on its own:
            // a trigger that only reaches 60 % is obvious at a glance.
            for (label, value) in [("ZL travel", l.lt), ("ZR travel", l.rt)] {
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::text(label).size(12.5).color(p.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(crate::i18n::text(format!("{:.0} %", value * 100.0)).font(theme::mono(12.0)).color(p.ink));
                    });
                });
                widgets::progress_bar(ui, &p, Some(value));
                ui.add_space(6.0);
            }
            ui.add_space(4.0);
            if widgets::button(ui, &p, "Test vibration", true)
                .on_hover_text(crate::i18n::tr("Runs both motors for about half a second through XInput, without starting the game. If this buzzes but play does not, set Vibration backend below to Standard."))
                .clicked()
            {
                match pad::rumble_test() {
                    Ok(()) => app.notify("Vibration test sent to the controller", Tone::Ok),
                    Err(e) => app.notify(e, Tone::Bad),
                }
            }
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
            crate::i18n::text("Where each action sits on the pad. Hover a button for its binding; the table below edits it.")
                .size(12.5)
                .color(p.muted),
        );
        ui.vertical_centered(|ui| controller_map(ui, &p, &app.config.input.bindings, live));
    });

    widgets::section(ui, &p, "Pad tuning", |ui| pad_tuning(app, ui, &p));

    widgets::section(ui, &p, "Bindings", |ui| {
        egui::Grid::new("bindings").num_columns(3).spacing([24.0, 8.0]).striped(true).show(ui, |ui| {
            for h in ["Action", "Keyboard", "Controller"] {
                ui.label(crate::i18n::text(h).font(theme::semibold(12.0)).color(p.muted));
            }
            ui.end_row();
            for (i, b) in app.config.input.bindings.iter_mut().enumerate() {
                ui.label(crate::i18n::text(&b.action).color(p.ink));
                let capturing = app.capture == Some(i);
                let text = if capturing { "Press a key… (Esc cancels)".to_string() } else { b.key.clone() };
                let r = ui.add_sized([210.0, 26.0], egui::Button::new(crate::i18n::text(text).font(theme::mono(12.5))).selected(capturing));
                if r.on_hover_text(crate::i18n::tr("Click, then press the key to bind")).clicked() {
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
fn percent(ui: &mut egui::Ui, p: &theme::Palette, value: f32, range: std::ops::RangeInclusive<u32>) -> Option<u32> {
    let mut pct = (value * 100.0).round();
    let changed = widgets::Slider::new(&mut pct, *range.start() as f32..=*range.end() as f32)
        .suffix(" %")
        .show(ui, p)
        .changed();
    changed.then_some(pct.round() as u32)
}

/// The four `[pad]` values the game reads, written into its `controls.ini` as
/// they change.
fn pad_tuning(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    let Some(ini) = crate::ingame::controls_ini() else {
        widgets::status_line(ui, p, Tone::Warn, "No settings folder", "The launcher's data folder is not set (APPDATA on Windows, XDG_CONFIG_HOME or HOME elsewhere), so it cannot write the game's controls.ini.");
        return;
    };
    let seed = app.config.input.deadzone;
    let mut t = match app.tuning {
        Some(t) => t,
        None => {
            let text = std::fs::read_to_string(&ini).unwrap_or_default();
            let t = pad::Tuning::from_ini_seeded(&text, seed, app.config.input.vibration);
            app.tuning = Some(t);
            t
        }
    };
    let mut save: Option<(&'static str, f32)> = None;
    let mut save_flag: Option<(&'static str, bool)> = None;

    widgets::setting_row(ui, p, "Left stick dead zone", "Movement below this is ignored.", |ui| {
        if let Some(pct) = percent(ui, p, t.left_deadzone, 0..=50) {
            t.left_deadzone = pct as f32 / 100.0;
            save = Some(("deadzone", t.left_deadzone));
        }
    });
    widgets::setting_row(ui, p, "Right stick dead zone", "Camera movement below this is ignored.", |ui| {
        if let Some(pct) = percent(ui, p, t.right_deadzone, 0..=50) {
            t.right_deadzone = pct as f32 / 100.0;
            save = Some(("right_deadzone", t.right_deadzone));
        }
    });
    widgets::setting_row(ui, p, "Full tilt", "How far the stick must travel to count as fully pushed. Lower it for a worn pad that tops out early.", |ui| {
        if let Some(pct) = percent(ui, p, t.stick_max, 50..=100) {
            t.stick_max = pct as f32 / 100.0;
            save = Some(("stick_max", t.stick_max));
        }
    });
    widgets::setting_row(ui, p, "Trigger threshold", "Trigger travel that counts as ZL or ZR pressed.", |ui| {
        if let Some(pct) = percent(ui, p, t.trigger, 5..=100) {
            t.trigger = pct as f32 / 100.0;
            save = Some(("trigger_threshold", t.trigger));
        }
    });
    widgets::setting_row(ui, p, "Rumble", "Whether the game may shake the pad. Written to controls.ini as [pad] haptic: automatic uses DualSense haptics or HD Rumble, off sends nothing.", |ui| {
        let mut on = t.rumble;
        if widgets::toggle(ui, p, &mut on).changed() {
            t.rumble = on;
            save_flag = Some(("haptic", on));
        }
    });
    widgets::setting_row(ui, p, "Rumble strength", "Multiplies the game's own amplitude, 0 to 100 %. [pad] haptic_strength.", |ui| {
        let mut pct = (t.rumble_scale * 100.0).round();
        if widgets::Slider::new(&mut pct, 0.0..=100.0).suffix(" %").show(ui, p).changed() {
            t.rumble_scale = pct / 100.0;
            app.config.input.vibration = pct / 100.0;
            save = Some(("haptic_strength", t.rumble_scale));
        }
    });
    // Auto picks DualSense audio haptics, HD Rumble packets for a wired Switch Pro Controller, and the standard
    // two motors for everything else. If a pad's richer path is not working, Standard is the same path the
    // Goldoire build used, so it is the one press that rules the engine's backend out (issue #10).
    let mut backend = pad::haptic_mode(&ini);
    widgets::setting_row(ui, p, "Vibration backend", "Automatic uses the best path the pad offers; Standard forces the two-motor rumble every pad supports. Written to controls.ini as [pad] haptic.", |ui| {
        let label = match backend.as_str() {
            "off" => "Off",
            "rumble" => "Standard (most compatible)",
            "dualsense" => "DualSense haptics",
            _ => "Automatic",
        };
        egui::ComboBox::from_id_salt("haptic-backend").selected_text(label).width(220.0).show_ui(ui, |ui| {
            for (word, text) in [
                ("auto", "Automatic"),
                ("rumble", "Standard (most compatible)"),
                ("dualsense", "DualSense haptics"),
            ] {
                if ui.selectable_label(backend == word, crate::i18n::text(text)).clicked() {
                    backend = word.to_string();
                }
            }
        });
    });
    if backend != pad::haptic_mode(&ini) {
        match pad::write_haptic_mode(&ini, &backend) {
            Ok(()) => {
                t.rumble = backend != "off";
                app.tuning = Some(t);
                app.notify(format!("Vibration backend set to {backend}"), Tone::Neutral);
            }
            Err(e) => app.notify(e, Tone::Bad),
        }
    }

    if let Some((key, on)) = save_flag {
        let written = if key == "haptic" { pad::write_haptic(&ini, on) } else { pad::write_flag(&ini, key, on) };
        match written {
            Ok(()) => app.tuning = Some(t),
            Err(e) => app.notify(e, Tone::Bad),
        }
    } else if let Some((key, v)) = save {
        let written = if key == "haptic_strength" { pad::write_haptic_strength(&ini, v) } else { pad::write(&ini, key, v) };
        match written {
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
        ui.label(crate::i18n::text(ini.display().to_string()).font(theme::mono(11.5)).color(p.muted));
    });
    ui.label(
        crate::i18n::text("Written where the game reads them: it takes them at its next start, and F1 > Controller applies them at once in-game.")
            .size(12.0)
            .color(p.muted),
    );
}

/// The controller artwork, as a white-on-alpha mask. The supplied image is
/// black ink over a transparent body; the mask keeps the ink's darkness as
/// alpha and makes its colour white, so one tint recolours every stroke and
/// button while the body lets the page show through.
const OUTLINE_PNG: &[u8] = include_bytes!("../../assets/controller-outline.png");

/// The artwork's pixel size and the map box's aspect ratio.
const OUTLINE_SIZE: (f32, f32) = (711.0, 496.0);
const MAP_ASPECT: f32 = OUTLINE_SIZE.0 / OUTLINE_SIZE.1;

static OUTLINE: Mutex<Option<egui::TextureHandle>> = Mutex::new(None);

/// The outline as a texture, decoded once on first use.
fn outline_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let mut cached = OUTLINE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(tex) = cached.as_ref() {
        return Some(tex.clone());
    }
    let decoded = image::load_from_memory_with_format(OUTLINE_PNG, image::ImageFormat::Png).ok()?.to_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
    let tex = ctx.load_texture("controller-outline", pixels, egui::TextureOptions::LINEAR);
    *cached = Some(tex.clone());
    Some(tex)
}

/// How a spot is drawn over the artwork.
#[derive(Clone, Copy)]
enum Face {
    /// A round face button: the artwork draws the circle and its letter, so the
    /// overlay is only a hover or held fill inside it.
    Circle,
    /// A shoulder button: the artwork has one unlabelled bumper per side, so
    /// the overlay is a small keycap carrying the button's name.
    Keycap,
    /// A control the artwork draws in full, such as a D-pad arm or a centre
    /// pill: the overlay only lights the area.
    Bare,
}

/// One binding's place on the pad: the action it belongs to, the caption drawn
/// beside it, the glyph in the button, where the artwork's control sits in 0..1
/// of the map, the hit box's half-extents, where its caption goes, the pad
/// button that lights it, and how it is drawn.
struct Spot {
    action: &'static str,
    caption: &'static str,
    glyph: &'static str,
    at: (f32, f32),
    half: (f32, f32),
    cap: (f32, f32),
    pad: u16,
    face: Face,
}

/// The game's pad, button by button, on the artwork's own layout: the left
/// stick and D-pad on the left, the face buttons and right stick on the right,
/// the centre pills below them, and the bumpers on top. Every position is a
/// fraction of the 711 × 496 artwork, measured from its pixels, so each hit
/// area sits exactly over the control it binds.
const SPOTS: [Spot; 15] = [
    // The face diamond: the artwork carries the X/Y/A/B letters, so only the
    // caption and the hit area are added.
    Spot { action: "Menu", caption: "Menu", glyph: "X", at: (0.761, 0.190), half: (0.0335, 0.0480), cap: (0.761, 0.118), pad: button::NORTH, face: Face::Circle },
    Spot { action: "Unit details", caption: "Details", glyph: "Y", at: (0.691, 0.290), half: (0.0335, 0.0480), cap: (0.594, 0.290), pad: button::WEST, face: Face::Circle },
    Spot { action: "Confirm", caption: "Confirm", glyph: "A", at: (0.829, 0.289), half: (0.0335, 0.0480), cap: (0.829, 0.222), pad: button::EAST, face: Face::Circle },
    Spot { action: "Cancel", caption: "Cancel", glyph: "B", at: (0.761, 0.387), half: (0.0335, 0.0480), cap: (0.761, 0.463), pad: button::SOUTH, face: Face::Circle },
    // The bumpers. The artwork has one per side, so each carries a trigger and
    // a shoulder label, triggers outboard.
    Spot { action: "Danger zone", caption: "Danger", glyph: "ZL", at: (0.230, 0.030), half: (0.027, 0.019), cap: (0.230, 0.078), pad: 0, face: Face::Keycap },
    Spot { action: "Previous unit", caption: "Prev", glyph: "L", at: (0.285, 0.030), half: (0.019, 0.019), cap: (0.285, 0.078), pad: button::LEFT_SHOULDER, face: Face::Keycap },
    Spot { action: "Next unit", caption: "Next", glyph: "R", at: (0.720, 0.030), half: (0.019, 0.019), cap: (0.700, 0.078), pad: button::RIGHT_SHOULDER, face: Face::Keycap },
    Spot { action: "Fast forward", caption: "Fast fwd", glyph: "ZR", at: (0.775, 0.030), half: (0.027, 0.019), cap: (0.760, 0.078), pad: 0, face: Face::Keycap },
    // The minus and plus pills in the strip below the sticks.
    Spot { action: "Map", caption: "Map", glyph: "", at: (0.358, 0.671), half: (0.026, 0.021), cap: (0.300, 0.671), pad: button::BACK, face: Face::Bare },
    Spot { action: "End turn", caption: "End turn", glyph: "", at: (0.640, 0.672), half: (0.026, 0.021), cap: (0.725, 0.672), pad: button::START, face: Face::Bare },
    // The D-pad, one hit area per arm of the artwork's cross.
    Spot { action: "Cursor up", caption: "Up", glyph: "", at: (0.361, 0.451), half: (0.025, 0.054), cap: (0.361, 0.365), pad: button::DPAD_UP, face: Face::Bare },
    Spot { action: "Cursor down", caption: "Down", glyph: "", at: (0.361, 0.559), half: (0.025, 0.054), cap: (0.361, 0.647), pad: button::DPAD_DOWN, face: Face::Bare },
    Spot { action: "Cursor left", caption: "Left", glyph: "", at: (0.324, 0.505), half: (0.038, 0.036), cap: (0.245, 0.505), pad: button::DPAD_LEFT, face: Face::Bare },
    Spot { action: "Cursor right", caption: "Right", glyph: "", at: (0.398, 0.505), half: (0.038, 0.036), cap: (0.478, 0.505), pad: button::DPAD_RIGHT, face: Face::Bare },
    // The right stick is the camera; the artwork's ring is the control.
    Spot { action: "Camera", caption: "Camera", glyph: "", at: (0.705, 0.615), half: (0.032, 0.012), cap: (0.705, 0.615), pad: 0, face: Face::Bare },
];

/// The pad, drawn from the outline artwork and tinted with the scheme accent.
/// A connected pad lights each control as it is used, and the sticks show their
/// live position inside the artwork's hubs.
fn controller_map(ui: &mut egui::Ui, p: &theme::Palette, bindings: &[crate::config::Binding], live: Option<pad::Live>) {
    let width = ui.available_width().min(520.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, width / MAP_ASPECT), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let at = |x: f32, y: f32| egui::pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height());
    let Some(outline) = outline_texture(ui.ctx()) else { return };

    // One mask, one tint: every stroke and filled button takes the accent, and
    // the transparent body lets the page through.
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.image(outline.id(), rect, uv, p.accent);

    // The sticks: a dot inside each of the artwork's filled hubs, following the
    // pad, so the map reads as live rather than as a picture.
    for (x, y, stick) in [(0.2283, 0.2944, live.map(|l| l.left)), (0.6297, 0.5050, live.map(|l| l.right))] {
        if let Some((vx, vy)) = stick {
            let c = at(x, y);
            let r = rect.width() * 0.045;
            painter.circle_filled(c + egui::vec2(vx * r * 0.6, -vy * r * 0.6), rect.width() * 0.011, p.ink);
        }
    }

    for (i, s) in SPOTS.iter().enumerate() {
        let bound = bindings.iter().find(|b| b.action.eq_ignore_ascii_case(s.action));
        let held = live.is_some_and(|l| l.held(s.pad));
        // The triggers are analog: their keycaps fill with the travel.
        let travel = match s.action {
            "Danger zone" => live.map_or(0.0, |l| l.lt),
            "Fast forward" => live.map_or(0.0, |l| l.rt),
            _ => 0.0,
        };
        let on = bound.is_some();
        let c = at(s.at.0, s.at.1);
        let hit = egui::Rect::from_center_size(c, egui::vec2(s.half.0 * rect.width(), s.half.1 * rect.height()) * 2.0);
        let hovered = ui.interact(hit.expand(4.0), ui.id().with(("pad-spot", i)), egui::Sense::hover());
        if let Some(b) = bound {
            hovered.clone().on_hover_text(format!("{}  ·  {}", b.action, b.pad));
        }

        match s.face {
            Face::Circle => {
                let r = s.half.0 * rect.width();
                if hovered.hovered() || held {
                    painter.circle_filled(c, r * 0.92, theme::alpha(p.accent_hi, if held { 120 } else { 45 }));
                    painter.circle_stroke(c, r, egui::Stroke::new(if held { 2.0_f32 } else { 1.0_f32 }, theme::alpha(p.accent_hi, if held { 230 } else { 150 })));
                }
            }
            Face::Keycap => {
                let mut fill = 34.0 + travel * 120.0;
                if hovered.hovered() {
                    fill = fill.max(60.0);
                }
                let fill = if held { 150 } else { fill as u8 };
                painter.rect_filled(hit, egui::CornerRadius::same(5), theme::alpha(p.accent, fill));
                painter.rect_stroke(hit, egui::CornerRadius::same(5), egui::Stroke::new(1.0_f32, theme::alpha(p.accent, if held { 230 } else { 170 })), egui::StrokeKind::Inside);
                painter.text(c, egui::Align2::CENTER_CENTER, s.glyph, theme::semibold(10.0), p.on_accent);
            }
            Face::Bare => {
                if hovered.hovered() || held {
                    painter.rect_filled(hit, egui::CornerRadius::same(5), theme::alpha(p.accent_hi, if held { 80 } else { 35 }));
                }
            }
        }

        let cap_colour = if held { p.accent } else if on { p.body } else { theme::alpha(p.muted, 120) };
        let cap_size = if matches!(s.face, Face::Keycap) { 9.0 } else { 10.0 };
        painter.text(at(s.cap.0, s.cap.1), egui::Align2::CENTER_CENTER, s.caption, egui::FontId::new(cap_size, egui::FontFamily::Proportional), cap_colour);
    }
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

    /// The artwork is the map: its pixel size fixes the box, and every spot in
    /// it has to stay inside that box.
    #[test]
    fn the_map_matches_the_artwork() {
        let art = image::load_from_memory_with_format(super::OUTLINE_PNG, image::ImageFormat::Png).expect("controller outline decodes");
        assert_eq!((art.width(), art.height()), (super::OUTLINE_SIZE.0 as u32, super::OUTLINE_SIZE.1 as u32));
        assert!((super::MAP_ASPECT - art.width() as f32 / art.height() as f32).abs() < 1e-4);
        for s in SPOTS.iter() {
            for (x, y) in [s.at, s.cap] {
                assert!((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y), "{} lies outside the artwork", s.action);
            }
        }
    }

}
