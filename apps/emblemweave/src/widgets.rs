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

/// Letter-spaced small caps, as the reference launcher sets its labels.
pub fn tracked(ui: &mut egui::Ui, text: &str, size: f32, colour: Color32, tracking: f32) -> egui::Response {
    let font = theme::semibold(size);
    let chars: Vec<char> = text.chars().collect();
    let widths: Vec<f32> = chars
        .iter()
        .map(|c| ui.fonts(|f| f.layout_no_wrap(c.to_string(), font.clone(), colour).size().x))
        .collect();
    let total = widths.iter().sum::<f32>() + tracking * chars.len().saturating_sub(1) as f32;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(total.max(1.0), size * 1.7), Sense::hover());
    let mut x = rect.left();
    let painter = ui.painter();
    for (c, w) in chars.iter().zip(&widths) {
        painter.text(Pos2::new(x, rect.center().y), egui::Align2::LEFT_CENTER, c, font.clone(), colour);
        x += w + tracking;
    }
    response
}

/// A section's small caps label in the accent colour.
pub fn eyebrow(ui: &mut egui::Ui, p: &Palette, text: &str) {
    tracked(ui, &text.to_uppercase(), 10.5, p.accent, theme::TRACK_WIDE);
}

/// The height a list inside a page takes: a share of the window, floored so it
/// never collapses on a small screen and capped so it cannot swallow the page.
pub fn list_height(ui: &egui::Ui, share: f32) -> f32 {
    (ui.ctx().screen_rect().height() * share).clamp(180.0, 420.0)
}

/// A glowing status dot.
pub fn status_dot(ui: &mut egui::Ui, p: &Palette, tone: Tone) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    let c = tone.colour(p);
    let painter = ui.painter();
    painter.circle_filled(rect.center(), 5.0, alpha(c, 40));
    painter.circle_filled(rect.center(), 2.6, c);
}

/// A glass strip painted behind a panel's content, with a lit rim.
pub fn glass_panel(ui: &egui::Ui, rect: Rect, p: &Palette, radius: CornerRadius) {
    let painter = ui.painter();
    painter.rect_filled(rect, radius, lerp(p.surface, p.bg_deep, 0.15));
    gradient_fill(painter, rect.shrink(1.0), radius.nw as f32 - 1.0, alpha(p.accent, 20), Color32::TRANSPARENT);
    // Light catching the upper rim, as on the reference's liquid glass.
    let rim = Rect::from_min_max(rect.min + Vec2::new(radius.nw as f32, 1.0), Pos2::new(rect.max.x - radius.ne as f32, rect.max.y));
    painter.line_segment([rim.left_top(), rim.right_top()], Stroke::new(1.0_f32, alpha(Color32::WHITE, 34)));
    painter.rect_stroke(rect, radius, Stroke::new(1.0_f32, alpha(p.accent, 46)), StrokeKind::Inside);
}

/// Page title in the display serif with a one-line description, over an
/// accent eyebrow that names the section of the app.
pub fn page_header(ui: &mut egui::Ui, p: &Palette, title: &str, subtitle: &str) {
    ui.add_space(theme::SPACE_L);
    eyebrow(ui, p, title);
    ui.add_space(2.0);
    ui.label(RichText::new(title).font(theme::display(27.0)).color(p.ink));
    ui.add_space(3.0);
    ui.label(RichText::new(subtitle).size(13.0).color(p.body));
    ui.add_space(theme::SPACE_M - 2.0);
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0_f32, alpha(p.accent, 34)));
    ui.add_space(theme::SPACE_M);
}

/// An elevated section with a small caps title.
pub fn section<R>(ui: &mut egui::Ui, p: &Palette, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let face = ui.painter().add(egui::Shape::Noop);
    let r = card_frame(p, Margin::symmetric(20, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                eyebrow(ui, p, title);
                ui.add_space(theme::SPACE_S);
            }
            add(ui)
        });
    // The face is painted under the content once the card's size is known.
    ui.painter().set(face, card_face(r.response.rect, p));
    ui.add_space(theme::SPACE_M);
    r.inner
}

/// The frame of a raised card: tinted glass, an accent hairline, and the
/// squared bottom-right corner the reference panels carry.
pub fn card_frame(p: &Palette, margin: Margin) -> egui::Frame {
    egui::Frame::new()
        .fill(lerp(p.surface, p.bg_deep, 0.18))
        .stroke(Stroke::new(1.0_f32, alpha(p.accent, 34)))
        .corner_radius(theme::CARD)
        .shadow(egui::epaint::Shadow { offset: [0, 12], blur: 30, spread: 0, color: Color32::from_black_alpha(120) })
        .inner_margin(margin)
}

/// The face of a card: an accent wash from the top left, a hairline of light
/// along the top edge, and a faint bloom along the bottom.
pub fn card_face(r: Rect, p: &Palette) -> egui::Shape {
    let radius = theme::CARD;
    let top = Rect::from_min_max(r.min + Vec2::new(24.0, 1.0), Pos2::new(r.max.x - 24.0, r.min.y + 2.0));
    let bloom = Rect::from_min_max(Pos2::new(r.left() + 12.0, r.bottom() - 22.0), Pos2::new(r.max.x - 12.0, r.bottom() - 1.0));
    egui::Shape::Vec(vec![
        gradient_shape(r.shrink(1.0), radius.nw as f32 - 1.0, alpha(p.accent, 14), alpha(p.bg_deep, 55)),
        egui::Shape::rect_filled(bloom, CornerRadius::same(10), alpha(p.accent, 8)),
        egui::Shape::rect_filled(top, CornerRadius::same(1), alpha(Color32::WHITE, 24)),
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

/// Fill a rounded rectangle with a vertical gradient, as one mesh.
pub fn gradient_fill(painter: &egui::Painter, rect: Rect, radius: f32, top: Color32, bottom: Color32) {
    painter.add(gradient_shape(rect, radius, top, bottom));
}

/// [`gradient_fill`] as a shape, for painting behind content laid out later.
/// [`gradient_fill`] as a shape, for painting behind content laid out later.
///
/// The mesh is built column by column: each column runs from the rectangle's top
/// edge to its bottom edge at that x, shortened by the corner arc, with the
/// colour going from `top` at the top edge to `bottom` at the bottom. The
/// silhouette follows the corners exactly and the colour is interpolated across
/// the whole height, so there is no banding and no seam between rows, and —
/// unlike a triangle fan around the outline — no geometry that can leave the
/// rectangle when a corner radius is large next to a short side.
pub fn gradient_shape(rect: Rect, radius: f32, top: Color32, bottom: Color32) -> egui::Shape {
    let (w, h) = (rect.width().max(0.0), rect.height().max(0.0));
    if w < 0.5 || h < 0.5 {
        return egui::Shape::Noop;
    }
    let r = radius.min(w * 0.5).min(h * 0.5).max(0.0);
    let cols = ((w / 3.0).ceil() as usize).clamp(8, 160);
    let shade = |y: f32| lerp(top, bottom, ((y - rect.top()) / h).clamp(0.0, 1.0));
    // Where the rectangle's edge sits at this x, after the corner arc.
    let edge = |x: f32| -> (f32, f32) {
        let dx = (x - rect.left()).min(rect.right() - x).max(0.0);
        if r <= 0.5 || dx >= r {
            (rect.top(), rect.bottom())
        } else {
            let dy = (r * r - (r - dx) * (r - dx)).max(0.0).sqrt();
            (rect.top() + r - dy, rect.bottom() - r + dy)
        }
    };
    let mut mesh = egui::Mesh::default();
    for i in 0..=cols {
        let x = rect.left() + w * i as f32 / cols as f32;
        let (y0, y1) = edge(x);
        mesh.colored_vertex(Pos2::new(x, y0), shade(y0));
        mesh.colored_vertex(Pos2::new(x, y1), shade(y1));
    }
    for i in 0..cols as u32 {
        let a = i * 2;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    egui::Shape::mesh(mesh)
}

/// Every button in the launcher: one height, one type size, one pill radius.
/// The finish is liquid glass — a tinted body with a specular band across the
/// top, a lit rim and a soft glow in the scheme's accent — so a button reads as
/// the same physical material in every scheme, in that scheme's colour.
fn button_core(ui: &mut egui::Ui, p: &Palette, text: &str, enabled: bool, primary: bool) -> egui::Response {
    const HEIGHT: f32 = 32.0;
    let font = theme::semibold(13.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, p.ink);
    let size = Vec2::new(galley.size().x + if primary { 38.0 } else { 28.0 }, HEIGHT);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (slot, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(slot) {
        let hover = ui.ctx().animate_bool(response.id.with("hover"), enabled && response.hovered());
        let pressed = enabled && response.is_pointer_button_down_on();
        let rect = slot.translate(Vec2::new(0.0, -hover + if pressed { 1.0 } else { 0.0 }));
        let radius = theme::RADIUS_PILL;
        let painter = ui.painter();
        if !enabled {
            painter.rect_filled(rect, CornerRadius::same(radius), alpha(Color32::WHITE, 5));
            painter.rect_stroke(rect, CornerRadius::same(radius), Stroke::new(1.0_f32, p.line), StrokeKind::Inside);
            painter.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, alpha(p.muted, 210));
            return response;
        }
        // The glow: two faint rings of the accent under the body.
        for (i, a) in [(1.0_f32, 14u8), (2.2, 8)] {
            painter.rect_filled(rect.expand(i * (1.0 + hover)), CornerRadius::same(radius), alpha(p.accent, a));
        }
        if primary {
            let base = if pressed { p.accent_deep } else { lerp(lerp(p.accent, p.accent_deep, 0.22), p.accent_hi, hover * 0.3) };
            painter.rect_filled(rect, CornerRadius::same(radius), base);
            // The body's own shading: light from the top, weight at the bottom.
            gradient_fill(painter, Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.center().y)), radius as f32, alpha(Color32::WHITE, (34.0 + 26.0 * hover) as u8), Color32::TRANSPARENT);
            gradient_fill(painter, Rect::from_min_max(Pos2::new(rect.min.x, rect.center().y), rect.max), radius as f32, Color32::TRANSPARENT, alpha(Color32::BLACK, 46));
            // A specular hot spot across the upper third, as glass catches light:
            // thin pills of falling opacity, which keep their edges clean at
            // this size where a gradient mesh does not.
            let inner = rect.shrink2(Vec2::new(radius as f32 * 0.9, 1.0));
            for (dy, h, a) in [(0.5_f32, 2.5_f32, 54u8), (3.6, 3.0, 26), (7.0, 3.0, 10)] {
                let strip = Rect::from_min_size(inner.min + Vec2::new(0.0, dy), Vec2::new(inner.width(), h));
                painter.rect_filled(strip, CornerRadius::same((h * 0.5) as u8), alpha(Color32::WHITE, (a as f32 * (0.75 + 0.5 * hover)) as u8));
            }
            painter.rect_stroke(rect, CornerRadius::same(radius), Stroke::new(1.0_f32, alpha(Color32::WHITE, 46)), StrokeKind::Inside);
            painter.rect_stroke(rect.shrink(1.0), CornerRadius::same(radius - 1), Stroke::new(1.0_f32, alpha(p.accent_hi, 90)), StrokeKind::Inside);
        } else {
            let base = lerp(alpha(Color32::WHITE, 4), alpha(p.accent, 20), hover);
            painter.rect_filled(rect, CornerRadius::same(radius), base);
            gradient_fill(painter, Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.center().y + 2.0)), radius as f32, alpha(Color32::WHITE, (16.0 + 14.0 * hover) as u8), Color32::TRANSPARENT);
            let inner = rect.shrink2(Vec2::new(radius as f32 * 0.9, 1.0));
            for (dy, h, a) in [(0.5_f32, 2.0_f32, 26u8), (3.2, 2.5, 10)] {
                let strip = Rect::from_min_size(inner.min + Vec2::new(0.0, dy), Vec2::new(inner.width(), h));
                painter.rect_filled(strip, CornerRadius::same((h * 0.5) as u8), alpha(Color32::WHITE, a));
            }
            let rim = lerp(alpha(Color32::WHITE, 26), alpha(p.accent, 130), hover);
            painter.rect_stroke(rect, CornerRadius::same(radius), Stroke::new(1.0_f32, rim), StrokeKind::Inside);
        }
        if response.has_focus() {
            painter.rect_stroke(rect.expand(2.5), CornerRadius::same(radius), Stroke::new(1.5_f32, p.accent_hi), StrokeKind::Outside);
        }
        let ink = if primary { p.on_accent } else { lerp(p.ink, Color32::WHITE, hover) };
        painter.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, ink);
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// The one prominent action on a page.
pub fn primary_button(ui: &mut egui::Ui, p: &Palette, text: &str, enabled: bool) -> egui::Response {
    button_core(ui, p, text, enabled, true)
}

pub fn button(ui: &mut egui::Ui, p: &Palette, text: &str, enabled: bool) -> egui::Response {
    button_core(ui, p, text, enabled, false)
}

/// A slider in the launcher's own material: a glass track carrying a glowing
/// fill in the scheme's accent, with the value in monospace beside it. Track,
/// fill, handle and glow all come from the palette, so the control changes with
/// the theme like every other one.
pub struct Slider<'a> {
    value: &'a mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &'static str,
    decimals: usize,
    log: bool,
    on: bool,
}

impl<'a> Slider<'a> {
    pub fn new(value: &'a mut f32, range: std::ops::RangeInclusive<f32>) -> Slider<'a> {
        Slider { value, range, suffix: "", decimals: 0, log: false, on: true }
    }
    pub fn suffix(mut self, suffix: &'static str) -> Self {
        self.suffix = suffix;
        self
    }
    pub fn fixed_decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }
    pub fn logarithmic(mut self, on: bool) -> Self {
        self.log = on;
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    pub fn show(self, ui: &mut egui::Ui, p: &Palette) -> egui::Response {
        let (lo, hi) = (*self.range.start(), *self.range.end());
        let span = (hi - lo).abs().max(f32::EPSILON);
        // A logarithm needs a positive floor; anything else is linear.
        let log = self.log && lo > 0.0;
        let to_t = |v: f32| -> f32 {
            let t = if log { (v.max(lo) / lo).ln() / (hi / lo).ln() } else { (v - lo) / span };
            t.clamp(0.0, 1.0)
        };
        let from_t = |t: f32| -> f32 {
            let t = t.clamp(0.0, 1.0);
            if log { lo * (hi / lo).powf(t) } else { lo + span * t }
        };
        let text = format!("{:.*}{}", self.decimals, *self.value, self.suffix);
        let font = theme::mono(11.5);
        let galley = ui.painter().layout_no_wrap(text, font, p.ink);
        let text_w = galley.size().x;
        let width = ui.available_width().min(ui.spacing().slider_width + text_w + 12.0).max(150.0);
        let sense = if self.on { Sense::click_and_drag() } else { Sense::hover() };
        let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width, 22.0), sense);
        let track = Rect::from_min_max(
            Pos2::new(rect.left() + 2.0, rect.center().y - 3.5),
            Pos2::new(rect.right() - text_w - 12.0, rect.center().y + 3.5),
        );
        let t = to_t(*self.value);
        if self.on && (response.dragged() || response.clicked()) {
            if let Some(pos) = response.interact_pointer_pos() {
                let nt = ((pos.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);
                let v = from_t(nt);
                if v != *self.value {
                    *self.value = v;
                    response.mark_changed();
                }
            }
        }
        let hover = ui.ctx().animate_bool(response.id.with("h"), self.on && (response.hovered() || response.dragged()));
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            // The glass track.
            painter.rect_filled(track, CornerRadius::same(4), lerp(p.panel_hi, p.bg_deep, 0.35));
            painter.rect_stroke(track, CornerRadius::same(4), Stroke::new(1.0_f32, alpha(p.accent, 50)), StrokeKind::Inside);
            // The glowing fill, in the accent, with a halo either side of it.
            let fill = Rect::from_min_max(track.min, Pos2::new(track.left() + track.width() * t, track.max.y));
            if fill.width() > 1.0 {
                for (i, a) in [(2.0_f32, 18u8), (4.0, 10)] {
                    painter.rect_filled(fill.expand2(Vec2::new(0.0, i * (0.5 + hover))), CornerRadius::same(6), alpha(p.accent, a));
                }
                painter.rect_filled(fill, CornerRadius::same(4), p.accent);
                gradient_fill(painter, Rect::from_min_max(fill.min, Pos2::new(fill.max.x, fill.center().y)), 4.0, alpha(Color32::WHITE, 70), Color32::TRANSPARENT);
            }
            // The handle: a glass bead with a lit rim and its own glow.
            let hx = track.left() + track.width() * t;
            let hr = 7.0 + hover;
            if self.on {
                painter.circle_filled(Pos2::new(hx, track.center().y), hr + 4.0, alpha(p.accent, (14.0 + 20.0 * hover) as u8));
            }
            painter.circle_filled(Pos2::new(hx, track.center().y), hr, if self.on { lerp(p.accent_hi, Color32::WHITE, 0.25) } else { p.muted });
            painter.circle_stroke(Pos2::new(hx, track.center().y), hr, Stroke::new(1.0_f32, alpha(Color32::WHITE, 150)));
            painter.circle_filled(Pos2::new(hx - hr * 0.3, track.center().y - hr * 0.3), hr * 0.32, alpha(Color32::WHITE, 130));
            painter.galley_with_override_text_color(Pos2::new(rect.right() - text_w, rect.center().y - galley.size().y / 2.0), galley, if self.on { p.ink } else { p.muted });
        }
        if self.on {
            response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        response
    }
}

/// The page backdrop: a radial accent glow from the top, and for the arcade
/// scheme a 32 pixel neon grid, fading out away from the glow.
pub fn backdrop(painter: &egui::Painter, rect: Rect, p: &Palette) {
    let focus = Pos2::new(rect.left() + rect.width() * 0.40, rect.top());
    let reach = rect.width().max(rect.height()) * 0.9;
    if p.grid {
        // Wide enough, and faint enough, that text over the grid still reads.
        let step = 44.0;
        let mut x = rect.left() - (rect.left() % step);
        while x < rect.right() {
            let mut y = rect.top();
            while y < rect.bottom() {
                let d = ((Pos2::new(x, y) - focus).length() / reach).min(1.0);
                // Neon grid lines in the scheme's highlight colour.
                let a = (13.0 * (1.0 - d).powi(2)) as u8;
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
    let peak = if p.grid { 30u8 } else { 26 };
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
        // The same glass as the buttons: a lit strip across the track and a
        // glint on the knob.
        let strip = Rect::from_min_max(rect.min + Vec2::new(9.0, 2.0), Pos2::new(rect.max.x - 9.0, rect.min.y + 7.0));
        painter.rect_filled(strip, CornerRadius::same(3), alpha(Color32::WHITE, 26));
        let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
        let knob = Pos2::new(x, rect.center().y);
        painter.circle_filled(knob, 8.0, if *on { Color32::WHITE } else { p.body });
        painter.circle_filled(knob + Vec2::new(-2.2, -2.4), 2.6, alpha(Color32::WHITE, if *on { 90 } else { 40 }));
        painter.circle_stroke(knob, 8.0, Stroke::new(1.0_f32, alpha(Color32::BLACK, 26)));
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

/// A navigation entry: a pill that fills with the accent when it is the
/// current page, with the label in tracked small caps.
pub fn nav_item(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, selected: bool) -> egui::Response {
    let size = Vec2::new(ui.available_width(), 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool(response.id.with("h"), response.hovered());
        let sel = ui.ctx().animate_bool(response.id.with("s"), selected);
        let bg = rect.shrink2(Vec2::new(4.0, 2.0));
        let painter = ui.painter();
        if sel > 0.0 {
            // A glowing tinted pill, brightest at the left edge.
            for i in (1..=4).rev() {
                let g = bg.expand(i as f32 * 2.0).translate(Vec2::new(0.0, 2.0));
                painter.rect_filled(g, CornerRadius::same(theme::RADIUS_PILL), alpha(p.accent, ((22.0 / i as f32) * sel) as u8));
            }
            painter.rect_filled(bg, CornerRadius::same(theme::RADIUS_PILL), alpha(p.accent, (44.0 * sel) as u8));
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(bg.left() + 3.0, bg.center().y - 7.0), Pos2::new(bg.left() + 5.5, bg.center().y + 7.0)),
                CornerRadius::same(2),
                alpha(p.accent_hi, 235),
            );
        } else if hover > 0.0 {
            painter.rect_filled(bg, CornerRadius::same(theme::RADIUS_PILL), alpha(Color32::WHITE, (9.0 * hover) as u8));
        }
        if response.has_focus() {
            painter.rect_stroke(bg, CornerRadius::same(theme::RADIUS_PILL), Stroke::new(1.0_f32, p.electric), StrokeKind::Inside);
        }
        let ink = if selected { Color32::WHITE } else { lerp(p.body, p.ink, hover) };
        let icon_colour = if selected { Color32::WHITE } else { lerp(p.muted, p.accent_hi, hover) };
        painter.text(Pos2::new(bg.left() + 15.0, bg.center().y), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(13.0), icon_colour);
        let font = theme::semibold(11.5);
        let mut x = bg.left() + 31.0;
        for c in label.chars() {
            let w = ui.fonts(|f| f.layout_no_wrap(c.to_string(), font.clone(), ink).size().x);
            ui.painter().text(Pos2::new(x, bg.center().y), egui::Align2::LEFT_CENTER, c, font.clone(), ink);
            x += w + 0.6;
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A thin progress bar, slanted as the reference launcher's resource bars are.
/// The fill runs on a 8% skew, with a bright leading edge.
pub fn progress_bar(ui: &mut egui::Ui, p: &Palette, fraction: Option<f32>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 7.0), Sense::hover());
    let slant = rect.height() * 0.55;
    let bar = |left: f32, right: f32| {
        vec![
            Pos2::new(left + slant, rect.top()),
            Pos2::new(right + slant, rect.top()),
            Pos2::new(right, rect.bottom()),
            Pos2::new(left, rect.bottom()),
        ]
    };
    let painter = ui.painter();
    painter.add(egui::Shape::convex_polygon(bar(rect.left() + 1.0, rect.right() - slant - 1.0), p.panel_hi, Stroke::new(1.0_f32, alpha(p.accent, 40))));
    let (from, to) = match fraction {
        Some(f) => (0.0, rect.width() * f.clamp(0.0, 1.0)),
        None => {
            let t = (ui.input(|i| i.time) as f32 * 0.7).fract();
            let w = rect.width() * 0.28;
            let x = (rect.width() + w) * t - w;
            (x.max(0.0), (x + w).min(rect.width()))
        }
    };
    if to > from {
        let poly = bar(rect.left() + from + 1.0, rect.left() + to - slant);
        painter.add(egui::Shape::convex_polygon(poly.clone(), p.accent, Stroke::NONE));
        // A lit film over the upper half of the fill, as the reference bars have.
        if let (Some(tl), Some(tr), Some(br), Some(bl)) = (poly.first(), poly.get(1), poly.get(2), poly.get(3)) {
            let mid = |a: Pos2, b: Pos2| Pos2::new(a.x + (b.x - a.x) * 0.45, a.y + (b.y - a.y) * 0.45);
            painter.add(egui::Shape::convex_polygon(vec![*tl, *tr, mid(*tr, *br), mid(*tl, *bl)], alpha(Color32::WHITE, 45), Stroke::NONE));
        }
        if let (Some(a), Some(b)) = (poly.first(), poly.get(3)) {
            painter.line_segment([*b, *a], Stroke::new(1.2_f32, alpha(Color32::WHITE, 120)));
        }
    }
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
