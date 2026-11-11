//! About: the project, its author, its build and its legal position.

use crate::theme;
use crate::widgets;
use crate::{logo, App};
use eframe::egui::{self, RichText};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    ui.add_space(theme::SPACE_L);
    ui.vertical_centered(|ui| {
        logo::show_lockup(ui, &p, 560.0_f32.min(ui.available_width()));
        ui.label(RichText::new("A native PC recompilation of Fire Emblem: Fortune's Weave").size(15.0).color(p.body));
        ui.horizontal(|ui| {
            ui.label(RichText::new("by").color(p.muted));
            ui.hyperlink_to(RichText::new("XiØN").font(theme::semibold(15.0)).color(p.accent_hi), "https://github.com/XiONDiGiTaL");
        });
    });
    ui.add_space(theme::SPACE_L);

    ui.columns(2, |cols| {
        widgets::section(&mut cols[0], &p, "Build", |ui| {
            widgets::kv(ui, &p, "Version", env!("CARGO_PKG_VERSION"));
            widgets::kv(ui, &p, "Revision", env!("EW_GIT_REV"));
            widgets::kv(ui, &p, "Target", &format!("{} {}", std::env::consts::OS, std::env::consts::ARCH));
            widgets::kv(ui, &p, "Profile", if cfg!(debug_assertions) { "debug" } else { "release" });
            ui.add_space(theme::SPACE_S);
            ui.hyperlink_to("github.com/XiONDiGiTaL/EmblemWeave", "https://github.com/XiONDiGiTaL/EmblemWeave");
            ui.hyperlink_to("xiondigital.net", "https://xiondigital.net/arcade/");
        });
        widgets::section(&mut cols[1], &p, "Credits", |ui| {
            for (who, what) in [
                ("XiØN", "design, engineering and the EmblemWeave identity"),
                ("Ryujinx", "the shader translator the bundled engine builds on (MIT)"),
                ("egui / eframe", "the launcher interface (MIT / Apache-2.0)"),
                ("roxmltree, windows-sys", "Cheat Engine tables, game memory and controller access (MIT / Apache-2.0)"),
                ("GameBanana", "hosts the community mods the Mods page installs; each belongs to its author"),
                ("libm", "musl's math library, ported to Rust (MIT)"),
                ("lz4_flex, ruzstd", "module and asset decompression (MIT / Apache-2.0)"),
                ("Unicorn", "independent AArch64 oracle used in development"),
                ("zbic", "the ZBIC development bridge"),
            ] {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(who).font(theme::semibold(13.5)).color(p.ink));
                    ui.label(RichText::new(what).size(13.0).color(p.body));
                });
            }
        });
    });

    widgets::section(ui, &p, "Legal", |ui| {
        ui.label(
            RichText::new(
                "EmblemWeave is an independent project by XiØN. It is not affiliated with, endorsed by, or supported by \
                 Nintendo or Intelligent Systems. It contains no game code, no game assets, no firmware and no keys, and it \
                 downloads none. You must supply data from your own legally obtained copy of the game; EmblemWeave only \
                 reads it. All trademarks belong to their respective owners.",
            )
            .size(13.0)
            .color(p.body),
        );
    });
}
