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

/// A flat surface behind the navigation rail and the top bar. The old glass
/// rim and accent outline made every strip glow; a dashboard's chrome should
/// recede, so this is one quiet panel with a single hairline.
pub fn glass_panel(ui: &egui::Ui, rect: Rect, p: &Palette, radius: CornerRadius) {
    let painter = ui.painter();
    painter.rect_filled(rect, radius, p.panel);
    gradient_fill(painter, rect.shrink(1.0), radius.nw as f32 - 1.0, alpha(p.accent, 12), Color32::TRANSPARENT);
    painter.rect_stroke(rect, radius, Stroke::new(1.0_f32, p.line), StrokeKind::Inside);
}

/// Page title in the semibold display sans with a one-line description, over
/// a quiet eyebrow that names the section of the app.
pub fn page_header(ui: &mut egui::Ui, p: &Palette, title: &str, subtitle: &str) {
    ui.add_space(theme::SPACE_M);
    ui.label(RichText::new(title).font(theme::display(24.0)).color(p.ink));
    ui.add_space(2.0);
    ui.label(RichText::new(subtitle).size(12.5).color(p.body));
    ui.add_space(theme::SPACE_S);
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0_f32, p.line));
    ui.add_space(theme::SPACE_M - 2.0);
}

/// An elevated section with a small caps title.
pub fn section<R>(ui: &mut egui::Ui, p: &Palette, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let face = ui.painter().add(egui::Shape::Noop);
    let r = card_frame(p, Margin::symmetric(18, 14))
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

/// An elevated section whose body folds away behind its title. The open state
/// is kept per `id`, so a drawer stays as the player left it while the page is
/// rebuilt every frame. `summary` is the one-line value shown on the right
/// when the body is closed, after the pattern of a settings drawer.
pub fn section_drawer(
    ui: &mut egui::Ui,
    p: &Palette,
    id: &str,
    title: &str,
    summary: &str,
    default_open: bool,
    add: impl FnOnce(&mut egui::Ui),
) {
    let state = egui::Id::new(("section-drawer", id));
    let mut open = ui.ctx().data(|d| d.get_temp::<bool>(state).unwrap_or(default_open));
    let face = ui.painter().add(egui::Shape::Noop);
    let r = card_frame(p, Margin::symmetric(18, 14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        // 34 px is a full-height header, not just the text: the whole row opens
        // the drawer, which matters on a handheld where a thumb is the pointer.
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            open = !open;
        }
        if ui.is_rect_visible(rect) {
            let hover = ui.ctx().animate_bool(response.id.with("hover"), response.hovered());
            if hover > 0.0 {
                ui.painter().rect_filled(rect.expand2(Vec2::new(5.0, 2.0)), CornerRadius::same(theme::RADIUS_S), alpha(Color32::WHITE, (5.0 * hover) as u8));
            }
            if response.has_focus() {
                ui.painter().rect_stroke(rect.expand(2.0), CornerRadius::same(theme::RADIUS_S), Stroke::new(1.0_f32, p.electric), StrokeKind::Outside);
            }
            let painter = ui.painter();
            // A drawn chevron, so it follows the accent and the type instead of
            // borrowing a glyph from whichever fallback font is installed.
            let c = Pos2::new(rect.left() + 8.0, rect.center().y);
            let ink = lerp(p.muted, p.accent_hi, hover);
            let stroke = Stroke::new(1.6_f32, ink);
            if open {
                painter.line_segment([Pos2::new(c.x - 4.0, c.y - 2.0), Pos2::new(c.x, c.y + 2.0)], stroke);
                painter.line_segment([Pos2::new(c.x, c.y + 2.0), Pos2::new(c.x + 4.0, c.y - 2.0)], stroke);
            } else {
                painter.line_segment([Pos2::new(c.x - 2.0, c.y - 4.0), Pos2::new(c.x + 2.0, c.y)], stroke);
                painter.line_segment([Pos2::new(c.x + 2.0, c.y), Pos2::new(c.x - 2.0, c.y + 4.0)], stroke);
            }
            painter.text(Pos2::new(rect.left() + 20.0, rect.center().y), egui::Align2::LEFT_CENTER, title, theme::semibold(13.5), p.ink);
            if !summary.is_empty() {
                let font = theme::mono(11.5);
                let galley = painter.layout_no_wrap(summary.to_owned(), font, p.muted);
                painter.galley_with_override_text_color(Pos2::new(rect.right() - galley.size().x, rect.center().y - galley.size().y / 2.0), galley, p.muted);
            }
        }
        if open {
            ui.add_space(theme::SPACE_S);
            let line = ui.available_rect_before_wrap();
            ui.painter().hline(line.x_range(), line.top(), Stroke::new(1.0_f32, alpha(p.line, 150)));
            ui.add_space(theme::SPACE_S);
            add(ui);
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(state, open));
    ui.painter().set(face, card_face(r.response.rect, p));
    ui.add_space(theme::SPACE_M);
}

/// A compact status capsule for the application header. Three of these answer
/// "is my game ready?" without opening a page, and each one is a shortcut to
/// the page that can fix it.
pub fn header_chip(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str, tone: Tone) -> egui::Response {
    let lf = theme::semibold(8.5);
    let vf = theme::semibold(10.5);
    let lw = ui.painter().layout_no_wrap(label.to_uppercase(), lf.clone(), p.muted).size().x;
    let vw = ui.painter().layout_no_wrap(value.to_owned(), vf.clone(), p.ink).size().x;
    let w = (lw + vw + 40.0).ceil();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool(response.id.with("h"), response.hovered());
        let c = tone.colour(p);
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(13), lerp(p.panel_hi, p.surface, 0.35));
        painter.rect_stroke(
            rect,
            CornerRadius::same(13),
            Stroke::new(1.0_f32, lerp(p.line, alpha(p.accent_hi, 100), hover)),
            StrokeKind::Inside,
        );
        painter.circle_filled(Pos2::new(rect.left() + 11.0, rect.center().y), 2.8, c);
        painter.circle_filled(Pos2::new(rect.left() + 11.0, rect.center().y), 5.0, alpha(c, 24));
        painter.text(Pos2::new(rect.left() + 19.0, rect.center().y), egui::Align2::LEFT_CENTER, label.to_uppercase(), lf, p.muted);
        painter.text(Pos2::new(rect.right() - 10.0, rect.center().y), egui::Align2::RIGHT_CENTER, value, vf, p.ink);
        if response.has_focus() {
            painter.rect_stroke(rect.expand(1.0), CornerRadius::same(14), Stroke::new(1.0_f32, p.electric), StrokeKind::Outside);
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The primary game action: a wide, softly rounded key that reads from across
/// the room without a halo or a sheen loop. The sub-label always says what the
/// press will do.
pub fn large_action(ui: &mut egui::Ui, p: &Palette, title: &str, subtitle: &str, enabled: bool) -> egui::Response {
    let size = Vec2::new(216.0, 58.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool(response.id.with("h"), enabled && response.hovered());
        let pressed = enabled && response.is_pointer_button_down_on();
        let painter = ui.painter();
        let base = if !enabled {
            p.panel_hi
        } else if pressed {
            p.accent_deep
        } else {
            lerp(p.accent, p.accent_hover, hover)
        };
        let lift = if enabled { 1.5 * hover } else { 0.0 };
        let face = rect.translate(Vec2::new(0.0, -lift));
        painter.rect_filled(face, CornerRadius::same(16), base);
        if enabled {
            gradient_fill(painter, Rect::from_min_max(face.min, Pos2::new(face.max.x, face.center().y)), 16.0, alpha(Color32::WHITE, 34), Color32::TRANSPARENT);
            painter.rect_stroke(face, CornerRadius::same(16), Stroke::new(1.0_f32, alpha(Color32::WHITE, 48)), StrokeKind::Inside);
        } else {
            painter.rect_stroke(face, CornerRadius::same(16), Stroke::new(1.0_f32, p.line_strong), StrokeKind::Inside);
        }
        let ink = if enabled { p.on_accent } else { p.muted };
        painter.text(Pos2::new(face.center().x, face.center().y - 8.0), egui::Align2::CENTER_CENTER, title, theme::display(20.0), ink);
        painter.text(Pos2::new(face.center().x, face.center().y + 14.0), egui::Align2::CENTER_CENTER, subtitle, theme::semibold(8.5), alpha(ink, 210));
        if response.has_focus() {
            painter.rect_stroke(face.expand(2.0), CornerRadius::same(18), Stroke::new(1.5_f32, p.accent_hi), StrokeKind::Outside);
        }
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A quick-access tile. The same component is used for every shortcut, so the
/// Overview's grid stays one visual rhythm instead of a row of mini pages.
pub fn quick_tile(ui: &mut egui::Ui, p: &Palette, page: crate::pages::Page, detail: &str, tone: Tone) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 68.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool(response.id.with("h"), response.hovered());
        let painter = ui.painter();
        let face = rect.shrink2(Vec2::new(0.0, 1.0));
        painter.rect_filled(face, CornerRadius::same(12), lerp(p.surface, p.panel_hi, hover * 0.75));
        painter.rect_stroke(face, CornerRadius::same(12), Stroke::new(1.0_f32, lerp(p.line, alpha(p.accent_hi, 110), hover)), StrokeKind::Inside);
        let icon = Rect::from_min_size(Pos2::new(face.left() + 13.0, face.center().y - 11.0), Vec2::splat(22.0));
        painter.rect_filled(icon, CornerRadius::same(6), alpha(p.accent, 20));
        page.paint_icon(painter, icon.shrink(3.0), lerp(p.accent_hi, Color32::WHITE, hover * 0.4));
        painter.text(Pos2::new(face.left() + 44.0, face.center().y - 9.0), egui::Align2::LEFT_CENTER, page.label(), theme::semibold(12.5), p.ink);
        painter.text(Pos2::new(face.left() + 44.0, face.center().y + 10.0), egui::Align2::LEFT_CENTER, detail, theme::semibold(9.5), tone.colour(p));
        let arrow = Pos2::new(face.right() - 14.0, face.center().y);
        let ink = lerp(p.muted, p.accent_hi, hover);
        painter.line_segment([arrow + Vec2::new(-3.0, -3.0), arrow + Vec2::new(0.0, 0.0)], Stroke::new(1.2_f32, ink));
        painter.line_segment([arrow + Vec2::new(0.0, 0.0), arrow + Vec2::new(-3.0, 3.0)], Stroke::new(1.2_f32, ink));
        if response.has_focus() {
            painter.rect_stroke(face.expand(1.0), CornerRadius::same(13), Stroke::new(1.0_f32, p.electric), StrokeKind::Outside);
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("Open {}", page.label()))

}

/// A clickable parent section in the navigation rail.
pub fn nav_group_header(ui: &mut egui::Ui, p: &Palette, label: &str, open: bool, compact: bool, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), if compact { 14.0 } else { 28.0 }), Sense::click());
    if compact {
        if ui.is_rect_visible(rect) {
            ui.painter().hline((rect.left() + 8.0)..=(rect.right() - 8.0), rect.center().y, Stroke::new(1.0_f32, alpha(p.line, 180)));
        }
        return response;
    }
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool(response.id.with("h"), response.hovered());
        let painter = ui.painter();
        if active {
            let mark = egui::Rect::from_center_size(Pos2::new(rect.left() + 8.0, rect.center().y), Vec2::new(2.0, 13.0));
            painter.rect_filled(mark, CornerRadius::same(1), p.accent_hi);
        }
        if hover > 0.0 {
            painter.rect_filled(rect.shrink2(Vec2::new(6.0, 1.0)), CornerRadius::same(6), alpha(Color32::WHITE, (4.0 * hover) as u8));
        }
        painter.text(Pos2::new(rect.left() + 14.0, rect.center().y), egui::Align2::LEFT_CENTER, label.to_uppercase(), theme::semibold(9.0), if active { p.accent_hi } else { lerp(p.muted, p.ink, hover) });
        let c = Pos2::new(rect.right() - 14.0, rect.center().y);
        let ink = lerp(p.muted, p.accent_hi, hover);
        let stroke = Stroke::new(1.2_f32, ink);
        if open {
            painter.line_segment([c + Vec2::new(-3.0, -1.5), c + Vec2::new(0.0, 1.5)], stroke);
            painter.line_segment([c + Vec2::new(0.0, 1.5), c + Vec2::new(3.0, -1.5)], stroke);
        } else {
            painter.line_segment([c + Vec2::new(-1.5, -3.0), c + Vec2::new(1.5, 0.0)], stroke);
            painter.line_segment([c + Vec2::new(1.5, 0.0), c + Vec2::new(-1.5, 3.0)], stroke);
        }
        if response.has_focus() {
            painter.rect_stroke(rect.shrink2(Vec2::new(4.0, 1.0)), CornerRadius::same(6), Stroke::new(1.0_f32, p.electric), StrokeKind::Inside);
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The frame of a raised card: a quiet surface, one hairline, and a shadow
/// that lifts it off the page without glowing.
pub fn card_frame(p: &Palette, margin: Margin) -> egui::Frame {
    egui::Frame::new()
        .fill(lerp(p.surface, p.bg_deep, 0.10))
        .stroke(Stroke::new(1.0_f32, p.line))
        .corner_radius(theme::CARD)
        .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 26, spread: 0, color: Color32::from_black_alpha(105) })
        .inner_margin(margin)
}

/// The face of a card: a low accent wash from the top edge and a hairline of
/// light, subtle enough that the content is the bright thing.
pub fn card_face(r: Rect, p: &Palette) -> egui::Shape {
    let radius = theme::CARD;
    let top = Rect::from_min_max(r.min + Vec2::new(26.0, 1.0), Pos2::new(r.max.x - 26.0, r.min.y + 2.0));
    egui::Shape::Vec(vec![
        gradient_shape(r.shrink(1.0), radius.nw as f32 - 1.0, alpha(p.accent, 7), Color32::TRANSPARENT),
        egui::Shape::rect_filled(top, CornerRadius::same(1), alpha(Color32::WHITE, 14)),
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

/// Every button in the launcher: one height, one type size, one soft radius.
/// Primary is a solid accent with a gentle top light; secondary is a quiet
/// surface that brightens on hover. No outer glow, no specular bands: the
/// dashboard's one bright element should be the action itself.
fn button_core(ui: &mut egui::Ui, p: &Palette, text: &str, enabled: bool, primary: bool) -> egui::Response {
    const HEIGHT: f32 = 34.0;
    let font = theme::semibold(13.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, p.ink);
    let size = Vec2::new(galley.size().x + if primary { 38.0 } else { 28.0 }, HEIGHT);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool(response.id.with("hover"), enabled && response.hovered());
        let pressed = enabled && response.is_pointer_button_down_on();
        let radius = theme::RADIUS_PILL;
        let painter = ui.painter();
        if !enabled {
            painter.rect_filled(rect, CornerRadius::same(radius), alpha(Color32::WHITE, 4));
            painter.rect_stroke(rect, CornerRadius::same(radius), Stroke::new(1.0_f32, p.line), StrokeKind::Inside);
            painter.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, alpha(p.muted, 200));
            return response;
        }
        if primary {
            let base = if pressed {
                p.accent_deep
            } else if hover > 0.0 {
                lerp(p.accent, p.accent_hover, hover)
            } else {
                p.accent
            };
            painter.rect_filled(rect, CornerRadius::same(radius), base);
            gradient_fill(
                painter,
                Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.center().y)),
                radius as f32,
                alpha(Color32::WHITE, (18.0 + 12.0 * hover) as u8),
                Color32::TRANSPARENT,
            );
            painter.rect_stroke(rect, CornerRadius::same(radius), Stroke::new(1.0_f32, alpha(Color32::WHITE, 36)), StrokeKind::Inside);
        } else {
            let base = lerp(p.panel_hi, lerp(p.panel_hi, p.accent, 0.22), hover);
            painter.rect_filled(rect, CornerRadius::same(radius), base);
            painter.rect_stroke(
                rect,
                CornerRadius::same(radius),
                Stroke::new(1.0_f32, lerp(p.line_strong, p.accent, hover * 0.8)),
                StrokeKind::Inside,
            );
        }
        if response.has_focus() {
            painter.rect_stroke(rect.expand(2.0), CornerRadius::same(radius + 2), Stroke::new(1.5_f32, p.accent_hi), StrokeKind::Outside);
        }
        let ink = if primary { p.on_accent } else { p.ink };
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

/// A modal result. `close` is true for the veil, Escape, or the header's close
/// control; the body decides whether its own primary action also closes.
pub struct Modal {
    pub close: bool,
}

/// One dialog frame for every floating panel in the launcher. It draws a
/// dimmed veil, a fixed readable width, a consistent header with a real close
/// target, and a scrollable body that never leaves the window. Content owns its
/// footer, so destructive actions can keep their own confirmation wording.
pub fn modal(
    ctx: &egui::Context,
    p: &Palette,
    id: &str,
    title: &str,
    subtitle: &str,
    width: f32,
    add: impl FnOnce(&mut egui::Ui),
    footer: impl FnOnce(&mut egui::Ui),
) -> Modal {
    let screen = ctx.screen_rect();
    let width = width.clamp(300.0, (screen.width() - 40.0).max(300.0));
    let mut close = false;
    // The veil is its own layer, created first so the dialog is above it. It
    // catches a click anywhere outside the panel, including on the page behind.
    egui::Area::new(egui::Id::new(("modal-veil", id)))
        .fixed_pos(screen.min)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(rect, 0, Color32::from_black_alpha(150));
            if response.clicked() {
                close = true;
            }
        });
    let frame = egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0_f32, p.line_strong))
        .corner_radius(theme::CARD)
        .shadow(egui::epaint::Shadow { offset: [0, 14], blur: 34, spread: 0, color: Color32::from_black_alpha(185) })
        .inner_margin(Margin::symmetric(20, 16));
    // A Window, rather than a bare Area, gives the content the whole screen as
    // its available height. An anchored Area only offered the half below the
    // anchor, which pushed footers into a scroll viewport.
    egui::Window::new(title)
        .id(egui::Id::new(("modal-panel", id)))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .order(egui::Order::Foreground)
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(width - 40.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    eyebrow(ui, p, "EmblemWeave");
                    ui.add_space(1.0);
                    ui.label(RichText::new(title).font(theme::display(20.0)).color(p.ink));
                    if !subtitle.is_empty() {
                        ui.label(RichText::new(subtitle).size(11.5).color(p.body));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if modal_close(ui, p) {
                        close = true;
                    }
                });
            });
            ui.add_space(9.0);
            let line = ui.available_rect_before_wrap();
            ui.painter().hline(line.x_range(), line.top(), Stroke::new(1.0_f32, p.line));
            ui.add_space(9.0);
            egui::ScrollArea::vertical()
                .id_salt(("modal-body", id))
                .max_height((screen.height() - 300.0).max(150.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    add(ui);
                });
            ui.add_space(10.0);
            let line = ui.available_rect_before_wrap();
            ui.painter().hline(line.x_range(), line.top(), Stroke::new(1.0_f32, p.line));
            ui.add_space(8.0);
            footer(ui);
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    Modal { close }
}

fn modal_close(ui: &mut egui::Ui, p: &Palette) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool(response.id.with("close"), response.hovered());
        if hover > 0.0 {
            ui.painter().rect_filled(rect, CornerRadius::same(7), alpha(Color32::WHITE, (8.0 * hover) as u8));
        }
        let c = rect.center();
        let ink = lerp(p.muted, p.ink, hover);
        ui.painter().line_segment([c + Vec2::new(-4.0, -4.0), c + Vec2::new(4.0, 4.0)], Stroke::new(1.35_f32, ink));
        ui.painter().line_segment([c + Vec2::new(4.0, -4.0), c + Vec2::new(-4.0, 4.0)], Stroke::new(1.35_f32, ink));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Close").clicked()
}

/// A consistent inline error for fields and failed dialog actions.
pub fn error_note(ui: &mut egui::Ui, p: &Palette, text: &str) {
    egui::Frame::new()
        .fill(alpha(p.bad, 16))
        .stroke(Stroke::new(1.0_f32, alpha(p.bad, 80)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(9, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(11.5).color(p.bad));
        });
}

/// A consistent empty state, used when a list has nothing to show yet.
pub fn empty_note(ui: &mut egui::Ui, p: &Palette, title: &str, body: &str) {
    egui::Frame::new()
        .fill(alpha(p.bg_deep, 90))
        .stroke(Stroke::new(1.0_f32, p.line))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).font(theme::semibold(12.5)).color(p.ink));
            ui.label(RichText::new(body).size(11.5).color(p.muted));
        });
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

/// A navigation row with one of the launcher's own line icons. In compact mode
/// the row becomes a 38-pixel square in the icon rail; the label is preserved as
/// the tooltip, so nothing becomes undiscoverable.
pub fn nav_item_page(ui: &mut egui::Ui, p: &Palette, page: crate::pages::Page, selected: bool, compact: bool) -> egui::Response {
    let size = Vec2::new(ui.available_width(), if compact { 38.0 } else { 32.0 });
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool(response.id.with("h"), response.hovered());
        let sel = ui.ctx().animate_bool(response.id.with("s"), selected);
        let bg = if compact { rect.shrink(5.0) } else { rect.shrink2(Vec2::new(6.0, 1.0)) };
        let painter = ui.painter();
        if sel > 0.0 {
            painter.rect_filled(bg, CornerRadius::same(8), alpha(p.accent, (190.0 * sel) as u8));
            painter.rect_stroke(bg, CornerRadius::same(8), Stroke::new(1.0_f32, alpha(p.accent_hi, (75.0 * sel) as u8)), StrokeKind::Inside);
        } else if hover > 0.0 {
            painter.rect_filled(bg, CornerRadius::same(8), alpha(Color32::WHITE, (7.0 * hover) as u8));
        }
        if response.has_focus() {
            painter.rect_stroke(bg, CornerRadius::same(8), Stroke::new(1.0_f32, p.electric), StrokeKind::Inside);
        }
        let ink = if selected { Color32::WHITE } else { lerp(p.body, p.ink, hover) };
        let icon_colour = if selected { Color32::WHITE } else { lerp(p.muted, p.accent_hi, hover) };
        let icon = Rect::from_center_size(Pos2::new(bg.left() + 16.0, bg.center().y), Vec2::splat(16.0));
        page.paint_icon(painter, icon, icon_colour);
        if !compact {
            let font = theme::semibold(11.5);
            painter.text(Pos2::new(bg.left() + 32.0, bg.center().y), egui::Align2::LEFT_CENTER, page.label(), font, ink);
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if compact { page.label().to_string() } else { format!("{}  ·  {}", page.label(), page.group().label()) })
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
