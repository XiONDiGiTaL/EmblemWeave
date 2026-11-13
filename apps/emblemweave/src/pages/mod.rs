//! The launcher's pages. Each module draws one page from the shared widgets.

pub mod about;
pub mod appearance;
pub mod cheats;
pub mod controls;
pub mod diagnostics;
pub mod log;
pub mod media;
pub mod mods;
pub mod overview;
pub mod recompile;
pub mod saves;
pub mod setup;

use crate::App;
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    Setup,
    Saves,
    Mods,
    Cheats,
    Recompile,
    Graphics,
    Audio,
    Controls,
    Appearance,
    Diagnostics,
    Log,
    About,
}

impl Page {
    pub const ALL: [Page; 13] = [
        Page::Overview,
        Page::Setup,
        Page::Saves,
        Page::Mods,
        Page::Cheats,
        Page::Recompile,
        Page::Graphics,
        Page::Audio,
        Page::Controls,
        Page::Appearance,
        Page::Diagnostics,
        Page::Log,
        Page::About,
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
            Page::Audio => "Audio",
            Page::Controls => "Controls",
            Page::Appearance => "Appearance",
            Page::Diagnostics => "Diagnostics",
            Page::Log => "Game Log",
            Page::About => "About",
        }
    }

    /// Glyphs from egui's bundled icon font, so they render on every system.
    pub fn icon(self) -> &'static str {
        match self {
            Page::Overview => "🏠",
            Page::Setup => "📁",
            Page::Saves => "💾",
            Page::Mods => "📦",
            Page::Cheats => "⚡",
            Page::Recompile => "⚙",
            Page::Graphics => "🖥",
            Page::Audio => "🔊",
            Page::Controls => "🎮",
            Page::Appearance => "🎨",
            Page::Diagnostics => "🔍",
            Page::Log => "📄",
            Page::About => "ℹ",
        }
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
        Page::Audio => media::audio(app, ui),
        Page::Controls => controls::show(app, ui),
        Page::Appearance => appearance::show(app, ui),
        Page::Diagnostics => diagnostics::show(app, ui),
        Page::Log => log::show(app, ui),
        Page::About => about::show(app, ui),
    }
}
