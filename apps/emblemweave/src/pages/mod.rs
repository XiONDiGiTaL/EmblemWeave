//! The launcher's pages, their navigation groups and their compact line-art
//! icons. Each page draws from the shared widgets and design system.

pub mod about;
pub mod appearance;
pub mod cheats;
pub mod controls;
pub mod diagnostics;
pub mod labb;
pub mod log;
pub mod media;
pub mod mods;
pub mod overview;
pub mod performance;
pub mod recompile;
pub mod requirements;
pub mod saves;
pub mod setup;
pub mod troubleshoot;
pub mod trinity;

use crate::config::InterfaceMode;
use crate::App;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    Setup,
    Saves,
    Mods,
    Cheats,
    Recompile,
    Graphics,
    Performance,
    Lab,
    Audio,
    Controls,
    Appearance,
    Diagnostics,
    Troubleshoot,
    Log,
    About,
    Requirements,
    Trinity,
}

impl Page {
    /// Shortcut order. Kept stable so Ctrl+1..9 keep opening the same pages
    /// even though the sidebar now groups them differently.
    pub const ALL: [Page; 18] = [
        Page::Overview,
        Page::Setup,
        Page::Saves,
        Page::Mods,
        Page::Cheats,
        Page::Recompile,
        Page::Graphics,
        Page::Performance,
        Page::Lab,
        Page::Audio,
        Page::Controls,
        Page::Appearance,
        Page::Diagnostics,
        Page::Troubleshoot,
        Page::Log,
        Page::About,
        Page::Requirements,
        Page::Trinity,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Setup => "Game Setup",
            Page::Saves => "Saves",
            Page::Mods => "Mods",
            Page::Cheats => "Cheats",
            Page::Recompile => "Recompilation",
            Page::Graphics => "Graphics",
            Page::Performance => "Performance",
            Page::Lab => "Performance Lab",
            Page::Audio => "Audio",
            Page::Controls => "Controls",
            Page::Appearance => "Appearance",
            Page::Diagnostics => "Diagnostics",
            Page::Troubleshoot => "Troubleshooter",
            Page::Log => "Game Log",
            Page::About => "About",
            Page::Requirements => "Requirements",
            Page::Trinity => "Trinity",
        }
    }

    /// Pages that expose build internals, logs or diagnostics. Standard mode
    /// keeps them available through Advanced mode rather than removing them.
    pub fn advanced(self) -> bool {
        matches!(self, Page::Recompile | Page::Diagnostics | Page::Log)
    }

    /// Whether the page belongs in this interface mode. This is the single
    /// source of truth used by the rail, keyboard navigation and the page
    /// dispatcher, so a hidden page cannot be opened by accident.
    pub fn available(self, mode: InterfaceMode) -> bool {
        mode == InterfaceMode::Advanced || !self.advanced()
    }

    /// Where this page belongs in the sidebar.
    pub fn group(self) -> NavGroup {
        NavGroup::ALL
            .into_iter()
            .find(|g| g.pages().contains(&self))
            .unwrap_or(NavGroup::Home)
    }

    /// A 16-pixel line icon. The shapes share one stroke weight and optical box,
    /// so the rail reads as one set rather than a row of borrowed emoji.
    pub fn paint_icon(self, painter: &egui::Painter, rect: Rect, colour: Color32) {
        let c = rect.center();
        let s = Stroke::new(1.35_f32, colour);
        let p = |x: f32, y: f32| Pos2::new(c.x + x, c.y + y);
        match self {
            Page::Overview => {
                painter.rect_stroke(Rect::from_center_size(c, egui::vec2(12.0, 10.0)), egui::CornerRadius::same(2), s, StrokeKind::Middle);
                painter.line_segment([p(-4.0, 1.0), p(-1.0, -2.0)], s);
                painter.line_segment([p(-1.0, -2.0), p(4.0, 3.0)], s);
            }
            Page::Setup => {
                painter.line_segment([p(-6.0, -3.5), p(-2.0, -3.5)], s);
                painter.line_segment([p(-2.0, -3.5), p(-0.5, -5.0)], s);
                painter.line_segment([p(-0.5, -5.0), p(2.0, -5.0)], s);
                painter.rect_stroke(Rect::from_min_max(p(-6.0, -3.5), p(6.0, 5.0)), egui::CornerRadius::same(1), s, StrokeKind::Middle);
            }
            Page::Saves => {
                painter.rect_stroke(Rect::from_min_max(p(-5.0, -6.0), p(5.0, 6.0)), egui::CornerRadius::same(1), s, StrokeKind::Middle);
                painter.line_segment([p(-2.0, -6.0), p(-2.0, -0.5)], s);
                painter.line_segment([p(-2.0, -0.5), p(0.0, 1.0)], s);
                painter.line_segment([p(0.0, 1.0), p(2.0, -0.5)], s);
                painter.line_segment([p(2.0, -0.5), p(2.0, -6.0)], s);
            }
            Page::Mods => {
                painter.rect_stroke(Rect::from_min_max(p(-5.0, -5.0), p(1.5, 1.5)), egui::CornerRadius::same(1), s, StrokeKind::Middle);
                painter.rect_stroke(Rect::from_min_max(p(-1.5, -1.5), p(5.0, 5.0)), egui::CornerRadius::same(1), s, StrokeKind::Middle);
            }
            Page::Cheats => {
                painter.add(egui::Shape::line(vec![p(-1.0, -6.0), p(-5.0, 0.0), p(-0.5, 0.0), p(1.0, 6.0), p(5.0, -0.5), p(0.5, -0.5), p(-1.0, -6.0)], s));
            }
            Page::Recompile => {
                painter.circle_stroke(c, 5.5, s);
                painter.circle_filled(c, 1.4, colour);
                for i in 0..6 {
                    let a = i as f32 * std::f32::consts::TAU / 6.0;
                    painter.line_segment([c + egui::vec2(a.cos() * 5.5, a.sin() * 5.5), c + egui::vec2(a.cos() * 7.0, a.sin() * 7.0)], s);
                }
            }
            Page::Graphics => {
                painter.rect_stroke(Rect::from_min_max(p(-6.0, -5.0), p(6.0, 3.0)), egui::CornerRadius::same(1), s, StrokeKind::Middle);
                painter.line_segment([p(0.0, 3.0), p(0.0, 5.5)], s);
                painter.line_segment([p(-3.5, 5.5), p(3.5, 5.5)], s);
            }
            Page::Performance => {
                painter.circle_stroke(p(0.0, 1.0), 5.8, s);
                painter.line_segment([p(0.0, 1.0), p(3.4, -2.4)], Stroke::new(1.6_f32, colour));
                painter.circle_filled(p(0.0, 1.0), 1.2, colour);
                painter.line_segment([p(-4.2, 5.0), p(4.2, 5.0)], s);
            }
            Page::Lab => {
                painter.add(egui::Shape::line(vec![p(-6.0, 3.5), p(-6.0, -3.0), p(-2.5, -3.0), p(-2.5, 5.5), p(1.5, 5.5), p(1.5, -5.0), p(5.6, -5.0)], s));
                painter.circle_filled(p(3.7, 1.3), 1.1, colour);
            }
            Page::Audio => {
                painter.add(egui::Shape::line(vec![p(-5.5, -2.0), p(-2.5, -2.0), p(1.5, -5.0), p(1.5, 5.0), p(-2.5, 2.0), p(-5.5, 2.0), p(-5.5, -2.0)], s));
                painter.circle_stroke(p(2.5, 0.0), 4.2, Stroke::new(1.2_f32, colour));
                painter.circle_stroke(p(2.5, 0.0), 7.1, Stroke::new(1.0_f32, crate::theme::alpha(colour, 120)));
            }
            Page::Controls => {
                painter.rect_stroke(Rect::from_center_size(c, egui::vec2(14.0, 8.0)), egui::CornerRadius::same(4), s, StrokeKind::Middle);
                painter.line_segment([p(-4.5, 0.0), p(-1.5, 0.0)], s);
                painter.line_segment([p(-3.0, -1.5), p(-3.0, 1.5)], s);
                painter.circle_filled(p(3.0, -1.2), 1.0, colour);
                painter.circle_filled(p(4.4, 0.6), 1.0, colour);
            }
            Page::Appearance => {
                painter.circle_stroke(p(0.0, 0.0), 6.0, s);
                painter.circle_filled(p(-2.4, -2.0), 1.2, colour);
                painter.circle_filled(p(2.2, -2.8), 1.2, colour);
                painter.circle_filled(p(3.0, 2.2), 1.2, colour);
            }
            Page::Diagnostics => {
                painter.add(egui::Shape::line(vec![p(-6.0, 1.0), p(-3.5, 1.0), p(-2.0, -4.0), p(0.0, 5.0), p(2.3, -2.0), p(3.8, 1.0), p(6.0, 1.0)], s));
            }
            Page::Troubleshoot => {
                painter.circle_stroke(p(-2.5, -2.5), 3.5, s);
                painter.line_segment([p(0.0, 0.0), p(5.5, 5.5)], Stroke::new(2.0_f32, colour));
                painter.circle_stroke(p(3.0, 3.0), 2.0, s);
            }
            Page::Log => {
                painter.rect_stroke(Rect::from_min_max(p(-4.5, -6.0), p(4.5, 6.0)), egui::CornerRadius::same(1), s, StrokeKind::Middle);
                for y in [-3.0, 0.0, 3.0] {
                    painter.line_segment([p(-2.0, y), p(2.0, y)], Stroke::new(1.2_f32, colour));
                }
            }
            Page::About => {
                painter.circle_stroke(c, 6.0, s);
                painter.circle_filled(p(0.0, -3.2), 1.0, colour);
                painter.line_segment([p(0.0, -0.8), p(0.0, 3.6)], s);
            }
            Page::Requirements => {
                painter.rect_stroke(Rect::from_min_max(p(-5.5, -6.0), p(5.5, 6.0)), egui::CornerRadius::same(1), s, StrokeKind::Middle);
                for y in [-2.5, 1.5, 4.0] {
                    painter.line_segment([p(-2.8, y), p(-1.4, y + 1.2)], Stroke::new(1.2_f32, colour));
                    painter.line_segment([p(-1.4, y + 1.2), p(0.4, y - 1.2)], Stroke::new(1.2_f32, colour));
                    painter.line_segment([p(2.0, y), p(3.6, y)], Stroke::new(1.0_f32, colour));
                }
            }
            Page::Trinity => {
                painter.circle_stroke(c, 6.0, s);
                painter.circle_stroke(p(0.0, 0.0), 3.0, Stroke::new(1.1_f32, colour));
                painter.line_segment([p(-5.0, -5.0), p(5.0, 5.0)], Stroke::new(1.0_f32, colour));
                painter.line_segment([p(5.0, -5.0), p(-5.0, 5.0)], Stroke::new(1.0_f32, colour));
            }
        }
    }
}

/// The sidebar's parent sections. They are presentation groups over existing
/// pages: no functionality is duplicated or removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavGroup {
    Home,
    Game,
    Library,
    Runtime,
    System,
    Settings,
    Trinity,
}

impl NavGroup {
    pub const ALL: [NavGroup; 7] = [NavGroup::Home, NavGroup::Game, NavGroup::Library, NavGroup::Runtime, NavGroup::System, NavGroup::Settings, NavGroup::Trinity];

    pub fn label(self) -> &'static str {
        match self {
            NavGroup::Home => "Home",
            NavGroup::Game => "Game",
            NavGroup::Library => "Library",
            NavGroup::Runtime => "Runtime",
            NavGroup::System => "System",
            NavGroup::Settings => "Settings",
            NavGroup::Trinity => "Trinity",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            NavGroup::Home => "home",
            NavGroup::Game => "game",
            NavGroup::Library => "library",
            NavGroup::Runtime => "runtime",
            NavGroup::System => "system",
            NavGroup::Settings => "settings",
            NavGroup::Trinity => "trinity",
        }
    }

    pub fn pages(self) -> &'static [Page] {
        match self {
            NavGroup::Home => &[Page::Overview],
            NavGroup::Game => &[Page::Setup, Page::Requirements],
            NavGroup::Library => &[Page::Saves, Page::Mods, Page::Cheats],
            NavGroup::Runtime => &[
                Page::Recompile,
                Page::Graphics,
                Page::Performance,
                Page::Lab,
                Page::Audio,
                Page::Controls,
            ],
            NavGroup::System => &[Page::Diagnostics, Page::Troubleshoot, Page::Log],
            NavGroup::Settings => &[Page::Appearance, Page::About],
            NavGroup::Trinity => &[Page::Trinity],
        }
    }

    /// The pages in this group that the selected interface mode shows. Groups
    /// with no visible pages are omitted by the rail, so Standard mode does not
    /// leave empty headings behind.
    pub fn pages_for(self, mode: InterfaceMode) -> Vec<Page> {
        self.pages().iter().copied().filter(|page| page.available(mode)).collect()
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    match app.page {
        Page::Overview => overview::show(app, ui),
        Page::Setup => setup::show(app, ui),
        Page::Saves => saves::show(app, ui),
        Page::Mods => mods::show(app, ui),
        Page::Cheats => cheats::show(app, ui),
        Page::Recompile => recompile::show(app, ui),
        Page::Graphics => media::graphics(app, ui),
        Page::Performance => performance::show(app, ui),
        Page::Lab => labb::show(app, ui),
        Page::Audio => media::audio(app, ui),
        Page::Controls => controls::show(app, ui),
        Page::Appearance => appearance::show(app, ui),
        Page::Diagnostics => diagnostics::show(app, ui),
        Page::Troubleshoot => troubleshoot::show(app, ui),
        Page::Log => log::show(app, ui),
        Page::About => about::show(app, ui),
        Page::Requirements => requirements::show(app, ui),
        Page::Trinity => trinity::show(app, ui),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_groups_cover_every_page_once() {
        let mut seen = Vec::new();
        for group in NavGroup::ALL {
            seen.extend_from_slice(group.pages());
        }
        assert_eq!(seen.len(), Page::ALL.len(), "a page belongs to more or fewer than one navigation group");
        for page in Page::ALL {
            assert_eq!(seen.iter().filter(|q| **q == page).count(), 1, "{} must appear exactly once", page.label());
            assert_eq!(page.group().pages().contains(&page), true, "{} is in the wrong group", page.label());
        }
    }

    #[test]
    fn navigation_labels_are_unique() {
        for (i, page) in Page::ALL.iter().enumerate() {
            assert!(!Page::ALL[i + 1..].iter().any(|q| q.label() == page.label()), "duplicate page label {}", page.label());
        }
    }

    #[test]
    fn standard_mode_hides_only_advanced_pages() {
        let hidden: Vec<Page> = Page::ALL
            .into_iter()
            .filter(|page| !page.available(InterfaceMode::Standard))
            .collect();
        assert_eq!(
            hidden,
            vec![Page::Recompile, Page::Diagnostics, Page::Log]
        );
        for page in Page::ALL {
            assert!(page.available(InterfaceMode::Advanced), "{} must remain reachable in Advanced mode", page.label());
        }
    }

    #[test]
    fn each_navigation_group_filters_its_own_pages_by_mode() {
        for group in NavGroup::ALL {
            let all = group.pages().to_vec();
            assert_eq!(
                group.pages_for(InterfaceMode::Advanced),
                all,
                "{} must keep every page in Advanced mode",
                group.label()
            );
            let expected: Vec<Page> = all.iter().copied().filter(|page| !page.advanced()).collect();
            assert_eq!(
                group.pages_for(InterfaceMode::Standard),
                expected,
                "{} must hide exactly its advanced pages in Standard mode",
                group.label()
            );
        }
    }

    #[test]
    fn standard_navigation_is_advanced_navigation_without_the_three_internal_pages() {
        let advanced: Vec<Page> = NavGroup::ALL
            .into_iter()
            .flat_map(|group| group.pages_for(InterfaceMode::Advanced))
            .collect();
        let standard: Vec<Page> = NavGroup::ALL
            .into_iter()
            .flat_map(|group| group.pages_for(InterfaceMode::Standard))
            .collect();

        assert_eq!(advanced.len(), Page::ALL.len(), "Advanced mode must show every page");
        for page in Page::ALL {
            assert!(advanced.contains(&page), "{} is missing from Advanced navigation", page.label());
        }
        // Standard keeps the rail's order and drops only what is advanced; nothing is regrouped or reordered.
        let expected: Vec<Page> = advanced.iter().copied().filter(|page| !page.advanced()).collect();
        assert_eq!(standard, expected);
        assert!(
            !standard.iter().any(|page| page.advanced()),
            "an advanced page must never reach Standard navigation"
        );
    }

    #[test]
    fn no_navigation_group_is_emptied_in_either_mode() {
        for mode in [InterfaceMode::Standard, InterfaceMode::Advanced] {
            for group in NavGroup::ALL {
                assert!(!group.pages_for(mode).is_empty(), "{} is empty in {:?}", group.label(), mode);
            }
        }
    }
}
