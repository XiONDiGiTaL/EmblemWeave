//! Main-character portraits for save profiles.
//!
//! The five renders in `assets/portraits` are decoded once and uploaded as
//! textures the first time a card needs them. A profile can be assigned a
//! portrait explicitly, or left on Automatic: the route lord stored in the
//! newest readable slot is used, and the card falls back to the generic
//! silhouette only when neither source names one.

use crate::theme;
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Copy)]
pub struct Portrait {
    pub key: &'static str,
    pub label: &'static str,
    png: &'static [u8],
}

pub const ALL: [Portrait; 5] = [
    Portrait { key: "cai", label: "Cai", png: include_bytes!("../assets/portraits/Portrait_cai_child_of_ribeira_feh.png") },
    Portrait { key: "dietrich", label: "Dietrich", png: include_bytes!("../assets/portraits/Portrait_dietrich_alluring_killer_feh.png") },
    Portrait { key: "eshmel", label: "Eshmel", png: include_bytes!("../assets/portraits/Portrait_eshmel_pale_raven_herald_feh.png") },
    Portrait { key: "leda", label: "Leda", png: include_bytes!("../assets/portraits/Portrait_leda_rose_of_caelus_feh.png") },
    Portrait { key: "theodora", label: "Theodora", png: include_bytes!("../assets/portraits/Portrait_theodora_mighty_queen_feh.png") },
];

static TEXTURES: Mutex<Option<HashMap<&'static str, egui::TextureHandle>>> = Mutex::new(None);

impl Portrait {
    pub fn by_key(key: &str) -> Option<Portrait> {
        ALL.iter().copied().find(|p| p.key.eq_ignore_ascii_case(key) || p.label.eq_ignore_ascii_case(key))
    }

    fn texture(&self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        let mut guard = TEXTURES.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        if let Some(tex) = map.get(self.key) {
            return Some(tex.clone());
        }
        let image = image::load_from_memory_with_format(self.png, image::ImageFormat::Png).ok()?.to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let tex = ctx.load_texture(format!("portrait-{}", self.key), pixels, egui::TextureOptions::LINEAR);
        map.insert(self.key, tex.clone());
        Some(tex)
    }
}

/// The protagonist a piece of save/unit text names. `Ids` keeps the game's own
/// id spelling after `PID_`, so both the English and Japanese names are matched
/// without translating anything.
pub fn key_for_name(text: &str) -> Option<&'static str> {
    let low = text.to_lowercase();
    for (key, names) in [
        ("cai", &["cai", "カイ"][..]),
        ("dietrich", &["dietrich", "ディートリヒ"][..]),
        ("eshmel", &["eshmel", "エシュメル"][..]),
        ("leda", &["leda", "レダ"][..]),
        ("theodora", &["theodora", "テオドラ"][..]),
    ] {
        if names.iter().any(|n| low.contains(n)) {
            return Some(key);
        }
    }
    None
}

/// The portrait to draw for a profile: an explicit choice wins, then the route
/// lord in its newest readable slot, then the profile name (which is useful when
/// someone names a run after a character before saving).
pub fn for_profile(explicit: Option<&str>, leader: Option<&str>, profile: &str) -> Option<&'static str> {
    explicit
        .and_then(Portrait::by_key)
        .map(|p| p.key)
        .or_else(|| leader.and_then(key_for_name))
        .or_else(|| key_for_name(profile))
}

/// Draw `key` into `rect`, cropping to fill instead of stretching the face.
pub fn paint(painter: &egui::Painter, rect: Rect, p: &theme::Palette, key: &str) -> bool {
    let Some(portrait) = Portrait::by_key(key) else { return false };
    let Some(tex) = portrait.texture(painter.ctx()) else { return false };
    let [w, h] = tex.size();
    let (iw, ih) = (w as f32, h as f32);
    let target = rect.width() / rect.height().max(1.0);
    let source = iw / ih.max(1.0);
    let uv = if source > target {
        let keep = target / source;
        Rect::from_min_max(Pos2::new((1.0 - keep) * 0.5, 0.0), Pos2::new((1.0 + keep) * 0.5, 1.0))
    } else {
        let keep = source / target;
        Rect::from_min_max(Pos2::new(0.0, (1.0 - keep) * 0.5), Pos2::new(1.0, (1.0 + keep) * 0.5))
    };
    painter.image(tex.id(), rect, uv, Color32::WHITE);
    // The artwork is bright; a foot of shade keeps the name and badges legible
    // without dimming the part of the portrait people recognise.
    let shaded = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + rect.height() * 0.64), rect.max);
    let [r, g, b, _] = p.bg_deep.to_array();
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(shaded.left_top(), Color32::TRANSPARENT);
    mesh.colored_vertex(shaded.right_top(), Color32::TRANSPARENT);
    mesh.colored_vertex(shaded.left_bottom(), Color32::from_rgba_unmultiplied(r, g, b, 225));
    mesh.colored_vertex(shaded.right_bottom(), Color32::from_rgba_unmultiplied(r, g, b, 225));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(egui::Shape::mesh(mesh));
    true
}

/// A row of portrait choices for Manage Saves. Returns the chosen key, or
/// `Some(None)` when Automatic was pressed. `current` is the explicit key, so
/// Automatic can be highlighted when it is absent.
pub fn picker(ui: &mut egui::Ui, p: &theme::Palette, current: Option<&str>) -> Option<Option<String>> {
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
        let auto = draw_choice(ui, p, None, current.is_none(), "Automatic");
        if auto.clicked() {
            chosen = Some(None);
        }
        for portrait in ALL {
            let active = current.is_some_and(|k| k.eq_ignore_ascii_case(portrait.key));
            if draw_choice(ui, p, Some(portrait), active, portrait.label).clicked() {
                chosen = Some(Some(portrait.key.to_string()));
            }
        }
    });
    chosen
}

fn draw_choice(ui: &mut egui::Ui, p: &theme::Palette, portrait: Option<Portrait>, active: bool, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(58.0, 68.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hover = ui.ctx().animate_bool(response.id.with("hover"), response.hovered());
    let face = Rect::from_min_size(rect.min, Vec2::new(58.0, 48.0));
    let painter = ui.painter();
    painter.rect_filled(face, CornerRadius::same(8), p.panel_hi);
    if let Some(portrait) = portrait {
        paint(painter, face.shrink(1.0), p, portrait.key);
    } else {
        painter.text(face.center(), egui::Align2::CENTER_CENTER, "A", theme::display(20.0), p.muted);
    }
    painter.rect_stroke(
        face,
        CornerRadius::same(8),
        Stroke::new(if active { 1.6_f32 } else { 1.0_f32 }, if active { p.accent } else { theme::alpha(p.line_strong, 190) }),
        StrokeKind::Inside,
    );
    if hover > 0.0 && !active {
        painter.rect_stroke(face, CornerRadius::same(8), Stroke::new(1.0_f32, p.accent_hi), StrokeKind::Inside);
    }
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 6.0),
        egui::Align2::CENTER_BOTTOM,
        label,
        theme::semibold(8.5),
        if active { p.accent_hi } else { p.muted },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_portrait_decodes() {
        for p in ALL {
            let image = image::load_from_memory(p.png).unwrap();
            assert!(image.width() >= 64 && image.height() >= 64, "{}", p.key);
        }
    }

    #[test]
    fn names_match_the_five_protagonists() {
        assert_eq!(key_for_name("PID_カイ"), Some("cai"));
        assert_eq!(key_for_name("Dietrich"), Some("dietrich"));
        assert_eq!(key_for_name("a normal soldier"), None);
    }

    #[test]
    fn an_explicit_choice_beats_the_save() {
        assert_eq!(for_profile(Some("leda"), Some("cai"), "Run A"), Some("leda"));
        assert_eq!(for_profile(None, Some("eshmel"), "Run A"), Some("eshmel"));
        assert_eq!(for_profile(None, None, "Theodora route"), Some("theodora"));
    }
}
