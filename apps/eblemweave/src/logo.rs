//! The EmblemWeave logo: the rendered EMBLEM WEAVE artwork, cut out of its
//! background, in one colourway per scheme.
//!
//! The artwork lives in `assets/branding` as transparent PNGs:
//! `logo-fe.png` is the original render (gold, silver and royal blue),
//! `logo-xion.png` is recoloured from it by `tools/recolor_logo.py`, and
//! `logo-arcade.png` is separate 8-bit artwork with a neon glow, cut out of its
//! white background by `tools/unmatte_logo.py`. Each is decoded and uploaded
//! to the GPU once, the first time it is drawn, and drawn at its own aspect.

use eframe::egui::{self, Color32, Rect, Vec2};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::theme::Palette;

pub const LOGO_XION: &[u8] = include_bytes!("../../../assets/branding/logo-xion.png");
pub const LOGO_ARCADE: &[u8] = include_bytes!("../../../assets/branding/logo-arcade.png");
pub const LOGO_FE: &[u8] = include_bytes!("../../../assets/branding/logo-fe.png");

/// Height relative to width of the rendered artwork (FE and XiØN); the
/// fallback when a scheme's texture cannot be loaded.
pub const LOCKUP_ASPECT: f32 = 439.0 / 922.0;

static TEXTURES: Mutex<Option<HashMap<usize, egui::TextureHandle>>> = Mutex::new(None);

/// The scheme's artwork as a texture, created on first use.
fn texture(ctx: &egui::Context, png: &'static [u8]) -> Option<egui::TextureHandle> {
    let mut guard = TEXTURES.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    let key = png.as_ptr() as usize;
    if let Some(t) = map.get(&key) {
        return Some(t.clone());
    }
    let decoded = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.to_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
    let handle = ctx.load_texture(format!("logo-{key:x}"), pixels, egui::TextureOptions::LINEAR);
    map.insert(key, handle.clone());
    Some(handle)
}

/// Draw the logo into `rect`, fitted and centred, with a soft accent glow.
pub fn lockup(painter: &egui::Painter, rect: Rect, palette: &Palette) {
    let Some(tex) = texture(painter.ctx(), palette.logo) else { return };
    let [w, h] = tex.size();
    let scale = (rect.width() / w as f32).min(rect.height() / h as f32);
    let size = Vec2::new(w as f32, h as f32) * scale;
    let at = Rect::from_center_size(rect.center(), size);
    glow(painter, at.center(), Vec2::new(size.x * 0.52, size.y * 0.62), palette.electric);
    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.image(tex.id(), at, uv, Color32::WHITE);
}

/// An elliptical glow from the accent to clear, as one mesh.
fn glow(p: &egui::Painter, centre: egui::Pos2, radius: Vec2, colour: Color32) {
    let [r, g, b, _] = colour.to_array();
    let peak = 34u8;
    let mut mesh = egui::Mesh::default();
    let (rings, segments) = (10u32, 40u32);
    mesh.colored_vertex(centre, Color32::from_rgba_unmultiplied(r, g, b, peak));
    for ring in 1..=rings {
        let t = ring as f32 / rings as f32;
        let a = (peak as f32 * (1.0 - t * t).powi(2)) as u8;
        for s in 0..segments {
            let ang = s as f32 / segments as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(centre + Vec2::new(ang.cos() * radius.x * t, ang.sin() * radius.y * t), Color32::from_rgba_unmultiplied(r, g, b, a));
        }
    }
    let idx = |ring: u32, s: u32| 1 + (ring - 1) * segments + (s % segments);
    for s in 0..segments {
        mesh.add_triangle(0, idx(1, s), idx(1, s + 1));
    }
    for ring in 1..rings {
        for s in 0..segments {
            let (a, b, c, d) = (idx(ring, s), idx(ring, s + 1), idx(ring + 1, s), idx(ring + 1, s + 1));
            mesh.add_triangle(a, b, c);
            mesh.add_triangle(b, d, c);
        }
    }
    p.add(egui::Shape::mesh(mesh));
}

/// The logo inline in a layout, sized by width.
pub fn show_lockup(ui: &mut egui::Ui, palette: &Palette, width: f32) -> egui::Response {
    let aspect = texture(ui.ctx(), palette.logo).map(|t| t.size()[1] as f32 / t.size()[0] as f32).unwrap_or(LOCKUP_ASPECT);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, width * aspect), egui::Sense::hover());
    lockup(ui.painter(), rect, palette);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rendered_colourways_share_a_size() {
        for png in [LOGO_XION, LOGO_FE] {
            let img = image::load_from_memory(png).unwrap();
            assert_eq!((img.width(), img.height()), (922, 439));
            assert!(img.color().has_alpha());
        }
    }

    #[test]
    fn the_arcade_artwork_is_cut_out() {
        let img = image::load_from_memory(LOGO_ARCADE).unwrap().to_rgba8();
        assert!(img.width() > img.height() * 2);
        // Corners are transparent: no white background left.
        for (x, y) in [(0, 0), (img.width() - 1, 0), (0, img.height() - 1), (img.width() - 1, img.height() - 1)] {
            assert_eq!(img.get_pixel(x, y)[3], 0, "corner ({x}, {y})");
        }
        // No opaque near-white pixels remain anywhere.
        assert!(!img.pixels().any(|p| p[3] > 200 && p[0] > 240 && p[1] > 240 && p[2] > 240));
    }
}
