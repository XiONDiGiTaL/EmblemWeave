//! Shared interface components. Every one takes its colours from the active
//! [`Palette`] and its sizes from `theme`, so pages compose them instead of
//! styling widgets themselves.

use crate::jobs::{Stage, StageStatus};
use crate::theme::{self, alpha, Palette};
use eframe::egui::{self, Color32, CornerRadius, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

/// Status colours with their meaning, so a page never picks a raw colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Accent,
    Ok,
    Warn,
    Bad,
}

impl Tone {
    pub fn colour(self, p: &Palette) -> Color32 {
        match self {
            Tone::Neutral => p.body,
            Tone::Accent => p.accent_hi,
            Tone::Ok => p.ok,
            Tone::Warn => p.warn,
            Tone::Bad => p.bad,
        }
    }
}

/// Page title in the display serif with a one-line description.
pub fn page_header(ui: &mut egui::Ui, p: &Palette, title: &str, subtitle: &str) {
    ui.add_space(theme::SPACE_L);
    ui.label(RichText::new(title).font(theme::display(30.0)).color(p.ink));
    ui.add_space(2.0);
    ui.label(RichText::new(subtitle).size(14.0).color(p.body));
    ui.add_space(theme::SPACE_L);
}

/// An elevated section with a small caps title.
pub fn section<R>(ui: &mut egui::Ui, p: &Palette, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let face = ui.painter().add(egui::Shape::Noop);
    let r = card_frame(p, Margin::symmetric(24, 20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title.to_uppercase()).font(theme::semibold(11.5)).color(p.muted));
                ui.add_space(theme::SPACE_S);
            }
            add(ui)
        });
    // The face is painted under the content once the card's size is known.
    ui.painter().set(face, card_face(r.response.rect, p));
    ui.add_space(theme::SPACE_M);
    r.inner
}

/// The frame of a raised card: no hard outline, a deep soft shadow.
pub fn card_frame(p: &Palette, margin: Margin) -> egui::Frame {
    egui::Frame::new()
        .fill(lerp(p.surface, p.panel_hi, 0.35))
        .stroke(Stroke::new(1.0_f32, alpha(Color32::WHITE, 9)))
        .corner_radius(CornerRadius::same(theme::RADIUS_M))
        .shadow(egui::epaint::Shadow { offset: [0, 22], blur: 52, spread: 0, color: Color32::from_black_alpha(120) })
        .inner_margin(margin)
}

/// The face of a card: lighter at the top, with a thin highlight along the
/// top edge, as the soft-UI style has it.
pub fn card_face(r: Rect, p: &Palette) -> egui::Shape {
    let radius = theme::RADIUS_M as f32 - 1.0;
    let top = Rect::from_min_max(r.min + Vec2::new(radius * 0.6, 1.0), Pos2::new(r.max.x - radius * 0.6, r.min.y + 2.0));
    egui::Shape::Vec(vec![
        gradient_shape(r.shrink(1.0), radius, alpha(Color32::WHITE, 10), alpha(p.bg_deep, 40)),
        egui::Shape::rect_filled(top, CornerRadius::same(1), alpha(Color32::WHITE, 16)),
    ])
}

/// A small rounded label: status, counts, flags.
pub fn pill(ui: &mut egui::Ui, p: &Palette, text: &str, tone: Tone) -> egui::Response {
    let c = tone.colour(p);
    egui::Frame::new()
        .fill(alpha(c, 30))
        .stroke(Stroke::new(1.0_f32, alpha(c, 90)))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(11, 4))
        .show(ui, |ui| ui.label(RichText::new(text).font(theme::semibold(12.0)).color(c)))
        .response
}

/// A coloured dot followed by text, for check lists.
pub fn status_line(ui: &mut egui::Ui, p: &Palette, tone: Tone, label: &str, detail: &str) {
    ui.horizontal_wrapped(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 4.0, tone.colour(p));
        ui.label(RichText::new(label).font(theme::semibold(13.5)).color(p.ink));
        ui.label(RichText::new(detail).size(13.0).color(p.body));
    });
}

/// The outline of a rounded rectangle, clockwise from the top left.
fn rounded_outline(rect: Rect, radius: f32) -> Vec<Pos2> {
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (Pos2::new(rect.left() + r, rect.top() + r), std::f32::consts::PI),
        (Pos2::new(rect.right() - r, rect.top() + r), 1.5 * std::f32::consts::PI),
        (Pos2::new(rect.right() - r, rect.bottom() - r), 0.0),
        (Pos2::new(rect.left() + r, rect.bottom() - r), 0.5 * std::f32::consts::PI),
    ];
    let mut pts = Vec::with_capacity(4 * 9);
    for (c, start) in corners {
        for k in 0..=8 {
            let a = start + k as f32 / 8.0 * std::f32::consts::FRAC_PI_2;
            pts.push(c + Vec2::new(a.cos(), a.sin()) * r);
        }
    }
    pts
}

/// Fill a rounded rectangle with a vertical gradient, as one mesh.
pub fn gradient_fill(painter: &egui::Painter, rect: Rect, radius: f32, top: Color32, bottom: Color32) {
    painter.add(gradient_shape(rect, radius, top, bottom));
}

/// [`gradient_fill`] as a shape, for painting behind content laid out later.
pub fn gradient_shape(rect: Rect, radius: f32, top: Color32, bottom: Color32) -> egui::Shape {
    let outline = rounded_outline(rect, radius);
    let shade = |y: f32| lerp(top, bottom, ((y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0));
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), shade(rect.center().y));
    for q in &outline {
        mesh.colored_vertex(*q, shade(q.y));
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    egui::Shape::mesh(mesh)
}

/// A liquid surface: a deep-to-light body, a glossy meniscus across the top
/// half, a lit rim, and a sheen that drifts across while `flow` is above zero.
pub fn liquid(ui: &egui::Ui, rect: Rect, radius: f32, base: Color32, flow: f32) {
    let painter = ui.painter();
    let deep = lerp(base, Color32::BLACK, 0.28);
    let light = lerp(base, Color32::WHITE, 0.16);
    gradient_fill(painter, rect, radius, light, deep);
    // The meniscus: a bright film over the top half that fades to nothing.
    let film = Rect::from_min_max(rect.min + Vec2::new(1.5, 1.5), Pos2::new(rect.max.x - 1.5, rect.center().y + 1.0));
    gradient_fill(painter, film, (radius - 1.5).max(0.0), Color32::from_white_alpha(52), Color32::from_white_alpha(4));
    // A caustic sheen: a soft diagonal band moving left to right.
    if flow > 0.0 {
        let t = (ui.input(|i| i.time) as f32 * 0.45).fract();
        let band = rect.height() * 1.4;
        let x = rect.left() - band + (rect.width() + band * 2.0) * t;
        let clip = painter.with_clip_rect(rect.shrink(1.0).intersect(painter.clip_rect()));
        let a = (34.0 * flow) as u8;
        let mut mesh = egui::Mesh::default();
        let slant = rect.height() * 0.6;
        for (dx, alpha) in [(0.0, 0u8), (band * 0.5, a), (band, 0u8)] {
            mesh.colored_vertex(Pos2::new(x + dx + slant, rect.top()), Color32::from_white_alpha(alpha));
            mesh.colored_vertex(Pos2::new(x + dx - slant, rect.bottom()), Color32::from_white_alpha(alpha));
        }
        for i in 0..2u32 {
            mesh.add_triangle(i * 2, i * 2 + 1, i * 2 + 2);
            mesh.add_triangle(i * 2 + 1, i * 2 + 2, i * 2 + 3);
        }
        clip.add(egui::Shape::mesh(mesh));
        ui.ctx().request_repaint();
    }
    // Lit upper rim and a darker lower rim give the surface its thickness.
    let rim = CornerRadius::same(radius as u8);
    painter.rect_stroke(rect, rim, Stroke::new(1.0_f32, Color32::from_white_alpha(26)), StrokeKind::Inside);
    painter.line_segment(
        [Pos2::new(rect.left() + radius, rect.top() + 0.8), Pos2::new(rect.right() - radius, rect.top() + 0.8)],
        Stroke::new(1.0_f32, Color32::from_white_alpha(70)),
    );
}

fn button_core(ui: &mut egui::Ui, p: &Palette, text: &str, enabled: bool, primary: bool, height: f32) -> egui::Response {
    let font = theme::semibold(if primary { 15.5 } else { 14.0 });
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, p.ink);
    let size = Vec2::new(galley.size().x + if primary { 46.0 } else { 32.0 }, height);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (slot, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(slot) {
        let hover = ui.ctx().animate_bool(response.id.with("hover"), enabled && response.hovered());
        let pressed = enabled && response.is_pointer_button_down_on();
        // The site lifts a hovered button by two pixels.
        let rect = slot.translate(Vec2::new(0.0, -2.0 * hover + if pressed { 1.0 } else { 0.0 }));
        let radius = theme::RADIUS_S as f32;
        let painter = ui.painter();
        if !enabled {
            painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_S), alpha(Color32::WHITE, 6));
            painter.rect_stroke(rect, CornerRadius::same(theme::RADIUS_S), Stroke::new(1.0_f32, p.line), StrokeKind::Inside);
            painter.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, p.muted);
            return response;
        }
        if primary {
            // A blue shadow under the button, deeper on hover.
            for i in 1..=5 {
                let spread = i as f32 * 2.6;
                let r = Rect::from_min_max(
                    Pos2::new(rect.left() + 8.0 - spread * 0.4, rect.bottom() - 6.0),
                    Pos2::new(rect.right() - 8.0 + spread * 0.4, rect.bottom() + spread * (1.0 + hover)),
                );
                painter.rect_filled(r, CornerRadius::same(12), alpha(p.accent, ((26.0 + 22.0 * hover) / i as f32) as u8));
            }
            let base = if pressed { p.accent_deep } else { lerp(p.accent, p.accent_hover, hover) };
            liquid(ui, rect, radius, base, hover);
        } else {
            let base = lerp(alpha(Color32::WHITE, 5), alpha(p.accent, 22), hover);
            ui.painter().rect_filled(rect, CornerRadius::same(theme::RADIUS_S), base);
            gradient_fill(ui.painter(), Rect::from_min_max(rect.min + Vec2::splat(1.0), Pos2::new(rect.max.x - 1.0, rect.center().y)), radius - 1.0, Color32::from_white_alpha(14), Color32::from_white_alpha(0));
            let edge = lerp(p.line_strong, alpha(p.accent, 115), hover);
            ui.painter().rect_stroke(rect, CornerRadius::same(theme::RADIUS_S), Stroke::new(1.0_f32, edge), StrokeKind::Inside);
            if hover > 0.0 {
                liquid_sheen(ui, rect, hover);
            }
        }
        if response.has_focus() {
            ui.painter().rect_stroke(rect.expand(3.0), CornerRadius::same(theme::RADIUS_S + 3), Stroke::new(1.5_f32, p.accent_hi), StrokeKind::Outside);
        }
        let ink = if primary { Color32::WHITE } else { lerp(p.ink, Color32::WHITE, hover) };
        ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, ink);
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// Only the drifting sheen of [`liquid`], for glass controls.
fn liquid_sheen(ui: &egui::Ui, rect: Rect, flow: f32) {
    let t = (ui.input(|i| i.time) as f32 * 0.45).fract();
    let band = rect.height() * 1.4;
    let x = rect.left() - band + (rect.width() + band * 2.0) * t;
    let clip = ui.painter().with_clip_rect(rect.shrink(1.0).intersect(ui.clip_rect()));
    let a = (22.0 * flow) as u8;
    let slant = rect.height() * 0.6;
    let mut mesh = egui::Mesh::default();
    for (dx, alpha) in [(0.0, 0u8), (band * 0.5, a), (band, 0u8)] {
        mesh.colored_vertex(Pos2::new(x + dx + slant, rect.top()), Color32::from_white_alpha(alpha));
        mesh.colored_vertex(Pos2::new(x + dx - slant, rect.bottom()), Color32::from_white_alpha(alpha));
    }
    for i in 0..2u32 {
        mesh.add_triangle(i * 2, i * 2 + 1, i * 2 + 2);
        mesh.add_triangle(i * 2 + 1, i * 2 + 2, i * 2 + 3);
    }
    clip.add(egui::Shape::mesh(mesh));
    ui.ctx().request_repaint();
}

/// The page backdrop: a radial accent glow from the top, and for the arcade
/// scheme a 32 pixel neon grid, fading out away from the glow.
pub fn backdrop(painter: &egui::Painter, rect: Rect, p: &Palette) {
    let focus = Pos2::new(rect.left() + rect.width() * 0.40, rect.top());
    let reach = rect.width().max(rect.height()) * 0.9;
    if p.grid {
        let step = 32.0;
        let mut x = rect.left() - (rect.left() % step);
        while x < rect.right() {
            let mut y = rect.top();
            while y < rect.bottom() {
                let d = ((Pos2::new(x, y) - focus).length() / reach).min(1.0);
                // Neon grid lines in the scheme's highlight colour.
                let a = (26.0 * (1.0 - d).powi(2)) as u8;
                if a > 0 {
                    let line = Stroke::new(1.0_f32, alpha(p.electric, a));
                    painter.line_segment([Pos2::new(x, y), Pos2::new(x, y + step)], line);
                    painter.line_segment([Pos2::new(x, y), Pos2::new(x + step, y)], line);
                }
                y += step;
            }
            x += step;
        }
    }
    // One wide ellipse of the accent from the top edge.
    let [r, g, b, _] = p.accent.to_array();
    let peak = if p.grid { 40u8 } else { 26 };
    let mut mesh = egui::Mesh::default();
    let (rings, segs) = (10u32, 40u32);
    mesh.colored_vertex(focus, Color32::from_rgba_unmultiplied(r, g, b, peak));
    for ring in 1..=rings {
        let t = ring as f32 / rings as f32;
        let a = (peak as f32 * (1.0 - t).powi(2)) as u8;
        for s in 0..segs {
            let ang = s as f32 / segs as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(focus + Vec2::new(ang.cos() * reach * 0.75 * t, ang.sin() * reach * 0.45 * t), Color32::from_rgba_unmultiplied(r, g, b, a));
        }
    }
    let idx = |ring: u32, s: u32| 1 + (ring - 1) * segs + (s % segs);
    for s in 0..segs {
        mesh.add_triangle(0, idx(1, s), idx(1, s + 1));
    }
    for ring in 1..rings {
        for s in 0..segs {
            let (a, b, c, d) = (idx(ring, s), idx(ring, s + 1), idx(ring + 1, s), idx(ring + 1, s + 1));
            mesh.add_triangle(a, b, c);
            mesh.add_triangle(b, d, c);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let [ar, ag, ab, aa] = a.to_array();
    let [br, bg, bb, ba] = b.to_array();
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(m(ar, br), m(ag, bg), m(ab, bb), m(aa, ba))
}

/// The one prominent action on a page.
pub fn primary_button(ui: &mut egui::Ui, p: &Palette, text: &str, enabled: bool) -> egui::Response {
    button_core(ui, p, text, enabled, true, 44.0)
}

pub fn button(ui: &mut egui::Ui, p: &Palette, text: &str, enabled: bool) -> egui::Response {
    button_core(ui, p, text, enabled, false, 32.0)
}

/// An animated on/off switch.
pub fn toggle(ui: &mut egui::Ui, p: &Palette, on: &mut bool) -> egui::Response {
    let size = Vec2::new(40.0, 22.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool(response.id, *on);
        let painter = ui.painter();
        let track = lerp(p.panel_hi, p.accent, t);
        painter.rect_filled(rect, CornerRadius::same(11), track);
        painter.rect_stroke(
            rect,
            CornerRadius::same(11),
            Stroke::new(1.0_f32, if response.hovered() { p.accent_hi } else { p.line }),
            StrokeKind::Inside,
        );
        let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
        painter.circle_filled(Pos2::new(x, rect.center().y), 8.0, if *on { Color32::WHITE } else { p.body });
        if response.has_focus() {
            painter.rect_stroke(rect.expand(3.0), CornerRadius::same(13), Stroke::new(1.5_f32, p.electric), StrokeKind::Outside);
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A labelled settings row: label and help on the left, control on the right.
pub fn setting_row(ui: &mut egui::Ui, p: &Palette, label: &str, help: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() * 0.5).max(220.0));
            ui.label(RichText::new(label).font(theme::semibold(14.0)).color(p.ink));
            if !help.is_empty() {
                ui.label(RichText::new(help).size(12.5).color(p.muted));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
    ui.add_space(theme::SPACE_XS);
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0_f32, alpha(p.line, 120)));
    ui.add_space(theme::SPACE_S);
}

/// A key/value line in monospace, for addresses and counts.
pub fn kv(ui: &mut egui::Ui, p: &Palette, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(170.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(170.0);
            ui.label(RichText::new(key).size(13.0).color(p.muted));
        });
        ui.add(egui::Label::new(RichText::new(value).font(theme::mono(12.5)).color(p.ink)).truncate())
            .on_hover_text(value);
    });
}

/// The honest marker for settings that are stored but not yet consumed.
pub fn pending_note(ui: &mut egui::Ui, p: &Palette, text: &str) {
    egui::Frame::new()
        .fill(alpha(p.warn, 18))
        .stroke(Stroke::new(1.0_f32, alpha(p.warn, 90)))
        .corner_radius(CornerRadius::same(theme::RADIUS_S))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(13.0).color(p.warn));
        });
    ui.add_space(theme::SPACE_M);
}

/// A vertical list of pipeline stages with status, timing and detail.
pub fn stage_list(ui: &mut egui::Ui, p: &Palette, stages: &[Stage]) {
    for (i, s) in stages.iter().enumerate() {
        let (tone, mark) = match s.status {
            StageStatus::Pending => (Tone::Neutral, "○"),
            StageStatus::Active => (Tone::Accent, "▶"),
            StageStatus::Done => (Tone::Ok, "●"),
            StageStatus::Skipped => (Tone::Warn, "◌"),
            StageStatus::Failed => (Tone::Bad, "✕"),
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{:>2}", i + 1)).font(theme::mono(12.0)).color(p.muted));
            ui.label(RichText::new(mark).size(15.0).color(tone.colour(p)));
            let name_colour = if s.status == StageStatus::Pending { p.body } else { p.ink };
            ui.label(RichText::new(s.name).font(theme::semibold(14.0)).color(name_colour));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let time = match (s.elapsed, s.started, s.status) {
                    (Some(d), _, _) => format!("{:.2}s", d.as_secs_f32()),
                    (None, Some(t), StageStatus::Active) => format!("{:.1}s", t.elapsed().as_secs_f32()),
                    _ => String::new(),
                };
                ui.label(RichText::new(time).font(theme::mono(12.0)).color(p.muted));
            });
        });
        if !s.detail.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(46.0);
                ui.label(RichText::new(&s.detail).size(12.5).color(p.body));
            });
        }
        ui.add_space(2.0);
    }
}

/// A navigation entry with an icon, the active indicator and hover feedback.
pub fn nav_item(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, selected: bool) -> egui::Response {
    let size = Vec2::new(ui.available_width(), 37.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool(response.id.with("h"), response.hovered());
        let sel = ui.ctx().animate_bool(response.id.with("s"), selected);
        let bg = rect.shrink2(Vec2::new(12.0, 3.0));
        let painter = ui.painter();
        if sel > 0.0 {
            // A raised, glowing tile for the current page.
            for i in 1..=4 {
                let g = bg.expand(i as f32 * 2.5).translate(Vec2::new(0.0, 3.0));
                painter.rect_filled(g, CornerRadius::same(theme::RADIUS_S + i * 2), alpha(p.accent, ((30.0 / i as f32) * sel) as u8));
            }
            liquid(ui, bg, theme::RADIUS_S as f32, lerp(p.panel_hi, p.accent, sel * 0.85), 0.0);
        } else if hover > 0.0 {
            painter.rect_filled(bg, CornerRadius::same(theme::RADIUS_S), alpha(Color32::WHITE, (10.0 * hover) as u8));
        }
        let painter = ui.painter();
        // The icon sits in its own rounded tile.
        let tile = Rect::from_center_size(Pos2::new(bg.left() + 20.0, bg.center().y), Vec2::splat(28.0));
        if sel == 0.0 {
            painter.rect_filled(tile, CornerRadius::same(10), alpha(Color32::WHITE, (6.0 + 8.0 * hover) as u8));
        }
        if response.has_focus() {
            painter.rect_stroke(bg, CornerRadius::same(theme::RADIUS_S + 2), Stroke::new(1.0_f32, p.electric), StrokeKind::Inside);
        }
        let ink = if selected { Color32::WHITE } else { lerp(p.body, p.ink, hover) };
        let icon_colour = if selected { Color32::WHITE } else { lerp(p.muted, p.accent_hi, hover) };
        painter.text(tile.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(15.0), icon_colour);
        painter.text(Pos2::new(bg.left() + 44.0, bg.center().y), egui::Align2::LEFT_CENTER, label, theme::semibold(14.5), ink);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A thin determinate or indeterminate progress bar.
pub fn progress_bar(ui: &mut egui::Ui, p: &Palette, fraction: Option<f32>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(2), p.panel_hi);
    let fill = match fraction {
        Some(f) => Rect::from_min_size(rect.min, Vec2::new(rect.width() * f.clamp(0.0, 1.0), rect.height())),
        None => {
            let t = (ui.input(|i| i.time) as f32 * 0.6).fract();
            let w = rect.width() * 0.25;
            Rect::from_min_size(Pos2::new(rect.left() + (rect.width() + w) * t - w, rect.top()), Vec2::new(w, rect.height()))
                .intersect(rect)
        }
    };
    painter.rect_filled(fill, CornerRadius::same(2), p.accent_hi);
    if fraction.is_none() {
        ui.ctx().request_repaint();
    }
}

/// The qualified name of an Itanium-mangled C++ symbol, for display:
/// `_ZN2nn2os9LockMutexEPNS0_9MutexTypeE` becomes `nn::os::LockMutex`.
/// Anything that is not a mangled name is returned unchanged.
pub fn demangle(sym: &str) -> String {
    let Some(rest) = sym.strip_prefix("_Z") else { return sym.to_string() };
    let b = rest.as_bytes();
    let mut i = 0;
    let read = |i: &mut usize| -> Option<String> {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        let n: usize = rest[start..*i].parse().ok()?;
        let s = rest.get(*i..*i + n)?.to_string();
        *i += n;
        Some(s)
    };
    let mut parts = Vec::new();
    if b.first() == Some(&b'N') {
        i += 1;
        while i < b.len() && matches!(b[i], b'K' | b'V' | b'r') {
            i += 1;
        }
        if rest[i..].starts_with("St") {
            parts.push("std".to_string());
            i += 2;
        }
        while i < b.len() && b[i] != b'E' {
            if b[i].is_ascii_digit() {
                match read(&mut i) {
                    Some(s) => parts.push(s),
                    None => return sym.to_string(),
                }
            } else if rest[i..].starts_with("C2") || rest[i..].starts_with("C1") {
                let class = parts.last().cloned().unwrap_or_default();
                parts.push(class);
                i += 2;
            } else if rest[i..].starts_with("D2") || rest[i..].starts_with("D1") || rest[i..].starts_with("D0") {
                let class = parts.last().cloned().unwrap_or_default();
                parts.push(format!("~{class}"));
                i += 2;
            } else {
                // Operators, templates and substitutions: stop at what we have.
                break;
            }
        }
    } else if b.first().is_some_and(|c| c.is_ascii_digit()) {
        match read(&mut i) {
            Some(s) => parts.push(s),
            None => return sym.to_string(),
        }
    }
    if parts.is_empty() {
        sym.to_string()
    } else {
        parts.join("::")
    }
}

/// Demangle every mangled word in a line of text.
pub fn demangle_line(line: &str) -> String {
    line.split(' ')
        .map(|w| {
            let core = w.trim_end_matches([',', ')', ';']);
            if core.starts_with("_Z") {
                format!("{}{}", demangle(core), &w[core.len()..])
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A wrapped monospace line, for logs and reports.
pub fn mono_line(ui: &mut egui::Ui, text: &str, colour: Color32) {
    ui.add(egui::Label::new(RichText::new(text).font(theme::mono(12.0)).color(colour)).wrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demangles_the_names_the_runtime_reports() {
        assert_eq!(demangle("_ZN2nn2os9LockMutexEPNS0_9MutexTypeE"), "nn::os::LockMutex");
        assert_eq!(demangle("_ZN2nv18InitializeGraphicsEPvm"), "nv::InitializeGraphics");
        assert_eq!(demangle("_ZNSt3__113random_deviceclEv"), "std::__1::random_device");
        assert_eq!(demangle("_ZNSt3__113random_deviceC2ERKNS_12basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEEE"), "std::__1::random_device::random_device");
        assert_eq!(demangle("memset"), "memset");
        assert_eq!(
            demangle_line("thread 0x1 waiting in _ZN2nn2oe22PopNotificationMessageEv, pc 0x2"),
            "thread 0x1 waiting in nn::oe::PopNotificationMessage, pc 0x2"
        );
    }
}
