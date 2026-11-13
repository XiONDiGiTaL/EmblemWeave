//! The design system: colour schemes, type, spacing and the shared widget
//! style. Every screen takes its colours from [`Palette`] and its sizes from
//! the constants here, so a scheme is data, never a branch inside a screen.
//!
//! * **XiØN** (default): the xiondigital.net tokens exactly, as its stylesheet
//!   defines them: `--bg #050607`, `--bg-raised #0a0b0d`, `--panel #0c0e11`,
//!   `--panel-hi #101318`, hairlines of 7% and 14% white, `--ink #ecedef`,
//!   `--body-copy #bdc3cc`, `--muted #8b9099`, `--blue #3f80ff`,
//!   `--blue-hi #79a8ff`, and `#4d8aff` for a hovered primary button.
//! * **Arcade**: a retro video-game palette: deep violet surfaces, hot pink,
//!   arcade yellow and cyan over a neon grid.
//! * **FE**: tactical fantasy, metallic gold and white silver over blackened
//!   steel, outlined in royal blue: the colours of the original logo render.
//! * **Ember**: forge fire, ember orange and molten gold over oxblood steel.
//! * **Verdant**: deep forest, emerald and pale gold over moss-black.
//! * **Frost**: cold steel, glacier blue and ice silver over midnight.
//!
//! Each scheme carries its own colourway of the logo artwork.

use eframe::egui::{self, Color32, FontFamily, FontId, TextStyle};
use serde::{Deserialize, Serialize};

/// Spacing scale, in points.
pub const SPACE_XS: f32 = 4.0;
pub const SPACE_S: f32 = 8.0;
pub const SPACE_M: f32 = 14.0;
pub const SPACE_L: f32 = 20.0;
pub const SPACE_XL: f32 = 30.0;
/// Corner radii: the site's 11 pixel buttons and `--radius` (18) cards.
pub const RADIUS_S: u8 = 12;
pub const RADIUS_M: u8 = 20;
/// Width of the navigation rail.
pub const NAV_WIDTH: f32 = 216.0;
/// The brand and status strips above and below the pages.
pub const TOPBAR_HEIGHT: f32 = 46.0;
pub const STATUSBAR_HEIGHT: f32 = 28.0;
/// The column of status panels beside the Overview page, and the window width
/// below which it steps aside so the page keeps its room.
pub const SIDE_WIDTH: f32 = 300.0;
pub const SIDE_MIN_WINDOW: f32 = 1240.0;
/// The widest the page column grows; wider windows centre it instead of
/// stretching the text across the screen.
pub const CONTENT_MAX: f32 = 1180.0;
/// Fully round controls: buttons and chips are pills.
pub const RADIUS_PILL: u8 = 200;
/// A card: soft on three corners, squared where it meets the page's spine.
pub const CARD: egui::CornerRadius = egui::CornerRadius { nw: 18, ne: 18, sw: 18, se: 6 };
/// Letter spacing for the small-caps labels, as the reference launcher sets it.
pub const TRACK_WIDE: f32 = 2.4;
pub const TRACK_MID: f32 = 1.6;
pub const TRACK_TIGHT: f32 = 0.9;

/// Every colour a screen may use.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Window background.
    pub bg: Color32,
    /// Deepest fill: inputs, code blocks.
    pub bg_deep: Color32,
    /// Secondary background: navigation rail, status bar.
    pub panel: Color32,
    /// Raised fill for hover.
    pub panel_hi: Color32,
    /// Elevated surface: sections and cards.
    pub surface: Color32,
    /// Hairlines, and the stronger hairline controls use.
    pub line: Color32,
    pub line_strong: Color32,
    /// Primary text.
    pub ink: Color32,
    /// Secondary text.
    pub body: Color32,
    /// Muted text: labels, hints.
    pub muted: Color32,
    /// Signature accent.
    pub accent: Color32,
    /// The accent a primary button takes on hover.
    pub accent_hover: Color32,
    /// Brighter accent for links and focus.
    pub accent_hi: Color32,
    /// Highlight for glows and the active indicator.
    pub electric: Color32,
    /// Deep accent for selection fills and pressed buttons.
    pub accent_deep: Color32,
    /// Text on a primary fill: white, or near-black where the accent is light.
    pub on_accent: Color32,
    /// Ambient glow, already translucent.
    pub glow: Color32,
    /// Restrained gold for brand details.
    pub gold: Color32,
    /// The logo artwork in this scheme's colourway (a transparent PNG).
    pub logo: &'static [u8],
    pub ok: Color32,
    pub warn: Color32,
    pub bad: Color32,
    /// Draw the arcade page's 32 pixel grid behind the content.
    pub grid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Scheme {
    #[default]
    Xion,
    Arcade,
    /// Saved by earlier versions as `emblem`.
    #[serde(alias = "emblem")]
    Fe,
    Ember,
    Verdant,
    Frost,
}

impl Scheme {
    pub const ALL: [Scheme; 6] = [Scheme::Xion, Scheme::Arcade, Scheme::Fe, Scheme::Ember, Scheme::Verdant, Scheme::Frost];

    pub fn label(self) -> &'static str {
        match self {
            Scheme::Xion => "XiØN",
            Scheme::Arcade => "Arcade",
            Scheme::Fe => "FE",
            Scheme::Ember => "Ember",
            Scheme::Verdant => "Verdant",
            Scheme::Frost => "Frost",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Scheme::Xion => "xiondigital.net: graphite surfaces, signature blue and a silver wordmark",
            Scheme::Arcade => "Retro arcade: hot pink, arcade yellow and cyan on a violet cabinet, over a neon grid drawn behind every page",
            Scheme::Fe => "Tactical fantasy: metallic gold, white silver and royal blue",
            Scheme::Ember => "Forge fire: ember orange and molten gold over oxblood steel",
            Scheme::Verdant => "Deep forest: emerald and pale gold over moss-black",
            Scheme::Frost => "Cold steel: glacier blue and ice silver over midnight",
        }
    }

    pub fn palette(self) -> Palette {
        // xiondigital.net, from its stylesheet.
        let xion = Palette {
            bg: rgb(0x05, 0x06, 0x07),
            bg_deep: rgb(0x07, 0x08, 0x0A),
            panel: rgb(0x0A, 0x0B, 0x0D),
            panel_hi: rgb(0x10, 0x13, 0x18),
            surface: rgb(0x0C, 0x0E, 0x11),
            line: white(0.07),
            line_strong: white(0.14),
            ink: rgb(0xEC, 0xED, 0xEF),
            body: rgb(0xBD, 0xC3, 0xCC),
            muted: rgb(0x8B, 0x90, 0x99),
            accent: rgb(0x3F, 0x80, 0xFF),
            accent_hover: rgb(0x4D, 0x8A, 0xFF),
            accent_hi: rgb(0x79, 0xA8, 0xFF),
            electric: rgb(0x79, 0xA8, 0xFF),
            accent_deep: rgb(0x1A, 0x3D, 0x82),
            on_accent: Color32::WHITE,
            glow: rgba(0x3F, 0x80, 0xFF, 40),
            gold: rgb(0xCF, 0xD4, 0xDC),
            logo: crate::logo::LOGO_XION,
            ok: rgb(0x4F, 0xC9, 0x8C),
            warn: rgb(0xE6, 0xB0, 0x4F),
            bad: rgb(0xE5, 0x5F, 0x6A),
            grid: false,
        };
        match self {
            Scheme::Xion => xion,
            // Retro arcade: a deep violet cabinet, hot pink, arcade yellow
            // and cyan, over a neon grid.
            Scheme::Arcade => Palette {
                bg: rgb(0x0B, 0x07, 0x16),
                bg_deep: rgb(0x07, 0x04, 0x0F),
                panel: rgb(0x11, 0x0A, 0x22),
                panel_hi: rgb(0x1D, 0x12, 0x38),
                surface: rgb(0x15, 0x0D, 0x2B),
                line: rgb(0x2A, 0x1C, 0x4A),
                line_strong: rgb(0x3D, 0x29, 0x6B),
                ink: rgb(0xFF, 0xF6, 0xE0),
                body: rgb(0xD3, 0xC8, 0xF0),
                muted: rgb(0x8F, 0x82, 0xB3),
                accent: rgb(0xFF, 0x2E, 0x97),
                accent_hover: rgb(0xFF, 0x52, 0xAA),
                accent_hi: rgb(0x29, 0xE3, 0xFF),
                electric: rgb(0x29, 0xE3, 0xFF),
                accent_deep: rgb(0x8A, 0x14, 0x56),
                on_accent: Color32::WHITE,
                glow: rgba(0xFF, 0x2E, 0x97, 48),
                gold: rgb(0xFF, 0xD2, 0x3F),
                logo: crate::logo::LOGO_ARCADE,
                ok: rgb(0x4D, 0xFF, 0x9A),
                warn: rgb(0xFF, 0xD2, 0x3F),
                bad: rgb(0xFF, 0x4D, 0x6D),
                grid: true,
            },
            Scheme::Fe => Palette {
                bg: rgb(0x07, 0x07, 0x09),
                bg_deep: rgb(0x04, 0x04, 0x05),
                panel: rgb(0x0F, 0x10, 0x14),
                panel_hi: rgb(0x1C, 0x1D, 0x24),
                surface: rgb(0x15, 0x16, 0x1C),
                line: rgb(0x2C, 0x2D, 0x38),
                line_strong: rgb(0x3A, 0x3C, 0x4A),
                ink: rgb(0xEE, 0xEC, 0xE6),
                body: rgb(0xB4, 0xB1, 0xAA),
                muted: rgb(0x7A, 0x77, 0x72),
                accent: rgb(0x2A, 0x5B, 0xD0),
                accent_hover: rgb(0x34, 0x68, 0xE0),
                accent_hi: rgb(0xE9, 0xCD, 0x6B),
                electric: rgb(0x5C, 0x8C, 0xFF),
                accent_deep: rgb(0x1D, 0x48, 0xAF),
                on_accent: Color32::WHITE,
                glow: rgba(0x1D, 0x48, 0xAF, 64),
                gold: rgb(0xC9, 0xA2, 0x27),
                logo: crate::logo::LOGO_FE,
                ok: rgb(0x7C, 0xB3, 0x7C),
                warn: rgb(0xD4, 0xA0, 0x4C),
                bad: rgb(0xC4, 0x5E, 0x5E),
                grid: false,
            },
            Scheme::Ember => Palette {
                bg: rgb(0x12, 0x07, 0x06),
                bg_deep: rgb(0x0B, 0x04, 0x03),
                panel: rgb(0x1A, 0x0A, 0x08),
                panel_hi: rgb(0x2A, 0x12, 0x10),
                surface: rgb(0x22, 0x0F, 0x0C),
                line: rgb(0x3A, 0x21, 0x1D),
                line_strong: rgb(0x55, 0x32, 0x2B),
                ink: rgb(0xFF, 0xED, 0xE8),
                body: rgb(0xD9, 0xB3, 0xA8),
                muted: rgb(0x9C, 0x74, 0x69),
                accent: rgb(0xF9, 0x73, 0x16),
                accent_hover: rgb(0xFB, 0x8A, 0x33),
                accent_hi: rgb(0xFD, 0xBA, 0x74),
                electric: rgb(0xFB, 0x92, 0x3C),
                accent_deep: rgb(0x9A, 0x3D, 0x0C),
                on_accent: Color32::WHITE,
                glow: rgba(0xF9, 0x73, 0x16, 48),
                gold: rgb(0xF4, 0xC1, 0x5A),
                logo: crate::logo::LOGO_EMBER,
                ok: rgb(0x6E, 0xCF, 0x8E),
                warn: rgb(0xF2, 0xB4, 0x4C),
                bad: rgb(0xE8, 0x62, 0x6D),
                grid: false,
            },
            Scheme::Verdant => Palette {
                bg: rgb(0x04, 0x12, 0x0C),
                bg_deep: rgb(0x02, 0x0B, 0x07),
                panel: rgb(0x08, 0x1A, 0x12),
                panel_hi: rgb(0x12, 0x29, 0x1F),
                surface: rgb(0x0C, 0x22, 0x17),
                line: rgb(0x21, 0x40, 0x2F),
                line_strong: rgb(0x35, 0x5C, 0x46),
                ink: rgb(0xE8, 0xFF, 0xF4),
                body: rgb(0xB4, 0xD9, 0xC6),
                muted: rgb(0x7C, 0x99, 0x89),
                accent: rgb(0x10, 0xB9, 0x81),
                accent_hover: rgb(0x22, 0xC7, 0x93),
                accent_hi: rgb(0x6E, 0xE7, 0xB7),
                electric: rgb(0x34, 0xD3, 0x99),
                accent_deep: rgb(0x0A, 0x6B, 0x4B),
                on_accent: Color32::WHITE,
                glow: rgba(0x10, 0xB9, 0x81, 40),
                gold: rgb(0xD9, 0xC5, 0x6A),
                logo: crate::logo::LOGO_VERDANT,
                ok: rgb(0x5F, 0xD3, 0x9A),
                warn: rgb(0xE0, 0xB4, 0x5C),
                bad: rgb(0xD9, 0x6A, 0x6A),
                grid: false,
            },
            Scheme::Frost => Palette {
                bg: rgb(0x06, 0x10, 0x1A),
                bg_deep: rgb(0x03, 0x0A, 0x12),
                panel: rgb(0x0A, 0x17, 0x24),
                panel_hi: rgb(0x14, 0x26, 0x38),
                surface: rgb(0x0E, 0x1E, 0x2E),
                line: rgb(0x22, 0x3A, 0x50),
                line_strong: rgb(0x34, 0x53, 0x6F),
                ink: rgb(0xEA, 0xF6, 0xFF),
                body: rgb(0xB5, 0xCC, 0xDD),
                muted: rgb(0x7E, 0x97, 0xAC),
                accent: rgb(0x38, 0xBD, 0xF8),
                accent_hover: rgb(0x56, 0xCB, 0xFB),
                accent_hi: rgb(0xA5, 0xE4, 0xFF),
                electric: rgb(0x7D, 0xD3, 0xFC),
                accent_deep: rgb(0x14, 0x62, 0x8F),
                // Glacier blue is light: dark ink keeps a white-free button legible.
                on_accent: rgb(0x04, 0x16, 0x24),
                glow: rgba(0x38, 0xBD, 0xF8, 40),
                gold: rgb(0xCF, 0xE3, 0xF2),
                logo: crate::logo::LOGO_FROST,
                ok: rgb(0x5B, 0xD3, 0xA6),
                warn: rgb(0xE6, 0xC0, 0x5A),
                bad: rgb(0xE4, 0x77, 0x7F),
                grid: false,
            },
        }
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_premultiplied(
        (r as u32 * a as u32 / 255) as u8,
        (g as u32 * a as u32 / 255) as u8,
        (b as u32 * a as u32 / 255) as u8,
        a,
    )
}

/// White at an opacity, as the site's `rgba(255, 255, 255, a)` hairlines.
const fn white(a: f32) -> Color32 {
    let v = (a * 255.0) as u8;
    Color32::from_rgba_premultiplied(v, v, v, v)
}

/// Translucent version of an opaque colour.
pub fn alpha(c: Color32, a: u8) -> Color32 {
    let [r, g, b, _] = c.to_array();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

// ---- typography ------------------------------------------------------------

pub fn display_family() -> FontFamily {
    FontFamily::Name("display".into())
}
pub fn display_bold_family() -> FontFamily {
    FontFamily::Name("display_bold".into())
}
pub fn semibold_family() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Serif display type for branding and page titles.
pub fn display(size: f32) -> FontId {
    FontId::new(size, display_family())
}
/// Semibold sans for labels and buttons.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, semibold_family())
}
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Install the launcher's typefaces from the fonts the system already has.
/// Each family falls back to egui's bundled faces, so a missing system font
/// changes the look, never the ability to render.
pub fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let base_prop = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let base_mono = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();

    let mut load = |key: &str, candidates: &[&str]| -> Option<String> {
        let bytes = candidates.iter().find_map(|p| std::fs::read(p).ok())?;
        fonts.font_data.insert(key.to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        Some(key.to_owned())
    };
    let win = |f: &str| format!(r"C:\Windows\Fonts\{f}");
    let serif = load("serif", &[&win("constan.ttf"), &win("georgia.ttf"), &win("pala.ttf"), "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf"]);
    let serif_bold = load("serif_bold", &[&win("constanb.ttf"), &win("georgiab.ttf"), &win("palab.ttf"), "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf"]);
    let sans = load("sans", &[&win("segoeui.ttf"), "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"]);
    let sans_semi = load("sans_semi", &[&win("seguisb.ttf"), &win("segoeuib.ttf")]);
    let mono = load("mono", &[&win("CascadiaMono.ttf"), &win("consola.ttf"), "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"]);

    // Japanese, last in every family: the game's own names (units, items,
    // classes) come from its data in Japanese.
    let cjk = load("cjk", &[&win("NotoSansJP-Regular.otf"), &win("YuGothM.ttc"), &win("meiryo.ttc"), &win("msgothic.ttc"), "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"]);
    let base_prop: Vec<String> = base_prop.into_iter().chain(cjk.clone()).collect();
    let base_mono: Vec<String> = base_mono.into_iter().chain(cjk).collect();
    let chain = |first: Option<String>, rest: &[String]| -> Vec<String> {
        first.into_iter().chain(rest.iter().cloned()).collect()
    };
    let prop = chain(sans.clone(), &base_prop);
    fonts.families.insert(FontFamily::Proportional, prop.clone());
    fonts.families.insert(FontFamily::Monospace, chain(mono, &base_mono));
    fonts.families.insert(display_family(), chain(serif.clone(), &prop));
    fonts.families.insert(display_bold_family(), chain(serif_bold.or(serif), &prop));
    fonts.families.insert(semibold_family(), chain(sans_semi.or(sans), &prop));
    ctx.set_fonts(fonts);
}

/// Apply a scheme to the whole interface.
pub fn apply(ctx: &egui::Context, scheme: Scheme) {
    let p = scheme.palette();
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = egui::Stroke::new(1.0_f32, p.line_strong);
    v.faint_bg_color = p.panel;
    v.extreme_bg_color = p.bg_deep;
    v.code_bg_color = p.bg_deep;
    v.override_text_color = Some(p.body);
    v.hyperlink_color = p.accent_hi;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.bad;
    v.window_corner_radius = RADIUS_M.into();
    v.menu_corner_radius = RADIUS_S.into();
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 12], blur: 30, spread: 0, color: Color32::from_black_alpha(190) };
    v.window_shadow = v.popup_shadow;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, p.line);
    w.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, p.body);
    w.inactive.bg_fill = p.panel_hi;
    w.inactive.weak_bg_fill = alpha(Color32::WHITE, 5);
    w.inactive.bg_stroke = egui::Stroke::new(1.0_f32, p.line_strong);
    w.inactive.fg_stroke = egui::Stroke::new(1.0_f32, p.ink);
    w.hovered.bg_fill = alpha(p.accent, 40);
    w.hovered.weak_bg_fill = alpha(p.accent, 18);
    w.hovered.bg_stroke = egui::Stroke::new(1.0_f32, alpha(p.accent, 115));
    w.hovered.fg_stroke = egui::Stroke::new(1.5_f32, p.ink);
    w.active.bg_fill = alpha(p.accent, 80);
    w.active.weak_bg_fill = alpha(p.accent, 60);
    w.active.bg_stroke = egui::Stroke::new(1.0_f32, p.accent_hi);
    w.active.fg_stroke = egui::Stroke::new(1.5_f32, p.ink);
    w.open.bg_fill = p.panel_hi;
    w.open.weak_bg_fill = p.panel_hi;
    w.open.bg_stroke = egui::Stroke::new(1.0_f32, p.accent);
    for state in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.corner_radius = RADIUS_S.into();
    }
    v.selection.bg_fill = alpha(p.accent, 110);
    v.selection.stroke = egui::Stroke::new(1.0_f32, p.ink);

    style.spacing.item_spacing = egui::vec2(SPACE_S + 2.0, SPACE_S);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    // Every control on a row — a button, a combo box, a text field — is the
    // same height, so a toolbar reads as one line.
    style.spacing.interact_size.y = 32.0;
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(RADIUS_PILL);
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(RADIUS_PILL);
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(RADIUS_PILL);
    style.spacing.slider_width = 220.0;
    style.text_styles.insert(TextStyle::Heading, display(26.0));
    style.text_styles.insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Button, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Small, FontId::new(12.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Monospace, mono(12.5));
    style.animation_time = 0.18;
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scheme_is_xion() {
        assert_eq!(Scheme::default(), Scheme::Xion);
    }

    #[test]
    fn the_xion_scheme_uses_the_site_tokens() {
        let p = Scheme::Xion.palette();
        assert_eq!(p.bg, Color32::from_rgb(0x05, 0x06, 0x07));
        assert_eq!(p.panel, Color32::from_rgb(0x0A, 0x0B, 0x0D));
        assert_eq!(p.surface, Color32::from_rgb(0x0C, 0x0E, 0x11));
        assert_eq!(p.panel_hi, Color32::from_rgb(0x10, 0x13, 0x18));
        assert_eq!(p.ink, Color32::from_rgb(0xEC, 0xED, 0xEF));
        assert_eq!(p.body, Color32::from_rgb(0xBD, 0xC3, 0xCC));
        assert_eq!(p.muted, Color32::from_rgb(0x8B, 0x90, 0x99));
        assert_eq!(p.accent, Color32::from_rgb(0x3F, 0x80, 0xFF));
        assert_eq!(p.accent_hover, Color32::from_rgb(0x4D, 0x8A, 0xFF));
        assert_eq!(p.accent_hi, Color32::from_rgb(0x79, 0xA8, 0xFF));
        assert_eq!(p.line, white(0.07));
        assert_eq!(p.line_strong, white(0.14));
    }

    #[test]
    fn arcade_is_its_own_palette_with_the_grid() {
        let (x, a) = (Scheme::Xion.palette(), Scheme::Arcade.palette());
        assert_ne!(x.accent, a.accent);
        assert_ne!(x.bg, a.bg);
        assert!(a.grid && !x.grid);
    }

    #[test]
    fn an_old_emblem_setting_loads_as_fe() {
        let s: Scheme = serde_json::from_str("\"emblem\"").unwrap();
        assert_eq!(s, Scheme::Fe);
    }

    #[test]
    fn every_scheme_has_its_own_colourway() {
        let mut seen = std::collections::HashSet::new();
        for s in Scheme::ALL {
            assert!(seen.insert(s.palette().logo.as_ptr()), "{} reuses another scheme's artwork", s.label());
        }
    }

    #[test]
    fn every_scheme_separates_its_surfaces_and_text() {
        for s in Scheme::ALL {
            let p = s.palette();
            assert_ne!(p.bg, p.surface, "{}", s.label());
            assert_ne!(p.ink, p.body, "{}", s.label());
            assert_ne!(p.accent, p.accent_hi, "{}", s.label());
        }
    }

    #[test]
    fn schemes_round_trip_through_the_config() {
        for s in Scheme::ALL {
            let back: Scheme = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
            assert_eq!(back, s);
        }
    }
}
