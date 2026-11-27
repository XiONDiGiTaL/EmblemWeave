//! Trinity: the shared plugin layer across EmblemWeave, fefw-recomp and
//! ObeliskChamber. It lives inside the Mods page now (a folded "Trinity
//! Installed" section) and reads the same catalog and prefix registry as the
//! two Tauri launchers, drawn with EmblemWeave's own widgets.

use crate::mods::{Api, Mod};
use crate::theme::{self, alpha, Palette};
use crate::widgets::{self, Tone};
use eframe::egui::{self, Color32, RichText};
use serde::Deserialize;
use std::sync::Mutex;

const CATALOG: &str = include_str!("trinity_catalog.json");
const PREFIXES: &str = include_str!("trinity_prefixes.json");
const LOGO: &[u8] = include_bytes!("../../assets/trinity.png");

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Catalog {
    name: String,
    version: String,
    houses: Vec<House>,
    listings: Vec<Listing>,
    capabilities: Vec<Capability>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct House {
    id: String,
    name: String,
    monogram: String,
    host_api: String,
    launcher: String,
    prefix: String,
    accent: String,
    status: String,
    tone: String,
    evidence: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Listing {
    id: String,
    name: String,
    kind: String,
    version: String,
    status: String,
    tone: String,
    #[serde(default)]
    hosts: Vec<String>,
    summary: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Capability {
    name: String,
    detail: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Prefixes {
    neutral: Vec<PrefixEntry>,
    hosts: Vec<PrefixEntry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PrefixEntry {
    id: String,
    owner: String,
    prefix: String,
}

/// The folded section the Mods page draws under its installed list. Collapsed
/// by default so the mods themselves stay the page's subject; the header states
/// what is installed without being opened.
///
/// `installed` is the same scan the Mods page already made: the section never
/// walks the mods folder again.
pub fn section(ui: &mut egui::Ui, p: &Palette, installed: &[Mod]) {
    ui.add_space(theme::SPACE_S);
    let core = installed.iter().find(|m| m.is_trinity_core());
    let state = match core {
        Some(m) if !m.has_code => format!("{} · the DLL is missing", m.version()),
        Some(m) if !m.enabled => format!("{} · switched off", m.version()),
        Some(m) => format!("Trinity Core {}", m.version()),
        None => "not installed".to_string(),
    };
    let tone = if core.is_some_and(|m| m.has_code && m.enabled) {
        Tone::Ok
    } else if core.is_some() {
        Tone::Warn
    } else {
        Tone::Neutral
    };
    widgets::card_frame(p, egui::Margin::symmetric(16, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::CollapsingHeader::new(
            RichText::new("Trinity Installed")
                .font(theme::semibold(15.0))
                .color(p.ink),
        )
        .id_salt("trinity-installed")
        .default_open(false)
        .show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                widgets::pill(ui, p, &state, tone);
                widgets::pill(ui, p, "NXP plugin layer", Tone::Neutral);
                let mods = installed.iter().filter(|m| m.has_code && !m.is_trinity_core()).count();
                widgets::pill(ui, p, &format!("{mods} code mod{}", if mods == 1 { "" } else { "s" }), Tone::Neutral);
            });
            ui.add_space(6.0);
            status(ui, p, core);
            content(ui, p, installed);
        });
    });
}

/// The installed copy's state, in the words a repair needs.
fn status(ui: &mut egui::Ui, p: &Palette, core: Option<&Mod>) {
    let Some(core) = core else {
        widgets::status_line(
            ui,
            p,
            Tone::Warn,
            "Trinity Core is not installed",
            "EmblemWeave installs it when it starts. If this stays, the mods folder is not writable.",
        );
        return;
    };
    if !core.has_code {
        widgets::status_line(
            ui,
            p,
            Tone::Bad,
            "Trinity Core's DLL is missing",
            "The folder and its manifest are there, but exefs/Trinity.dll is not: the game cannot load it.\
             EmblemWeave restores the bundled copy the next time it starts; restart the launcher if this stays.",
        );
    } else if !core.enabled {
        widgets::status_line(
            ui,
            p,
            Tone::Warn,
            "Trinity Core is switched off",
            "Code mods that need it will not start. Switch it back on in the installed list above.",
        );
    } else {
        widgets::status_line(
            ui,
            p,
            Tone::Ok,
            "Trinity Core is installed and loaded",
            "It ships with EmblemWeave and is managed by the launcher, not by the GameBanana browser.",
        );
    }
}

/// The old Trinity page's content, unchanged, now drawn inside the section.
fn content(ui: &mut egui::Ui, p: &Palette, installed: &[Mod]) {
    let catalog = match serde_json::from_str::<Catalog>(CATALOG) {
        Ok(c) if !c.houses.is_empty() && !c.listings.is_empty() => c,
        _ => {
            widgets::empty_note(
                ui,
                p,
                "The Trinity catalog could not be read",
                "The embedded catalog is missing or invalid. Rebuild the launcher from a Trinity release.",
            );
            return;
        }
    };
    let prefixes = serde_json::from_str::<Prefixes>(PREFIXES).unwrap_or_default();

    hero(ui, p, &catalog, prefixes.neutral.len() + prefixes.hosts.len());
    metrics(ui, p, &catalog, &prefixes);
    code_mods(ui, p, installed);
    houses(ui, p, &catalog);
    registry(ui, p, &catalog);
    capabilities(ui, p, &catalog);
    prefix_table(ui, p, &catalog, &prefixes);
}

/// The installed code mods that use Trinity or NxModApi, told apart from the
/// core itself and from DLLs that declare nothing.
fn code_mods(ui: &mut egui::Ui, p: &Palette, installed: &[Mod]) {
    let core_ok = installed.iter().any(|m| m.is_trinity_core() && m.has_code);
    let list: Vec<&Mod> = installed.iter().filter(|m| m.has_code && !m.is_trinity_core()).collect();
    widgets::section(ui, p, "Code mods", |ui| {
        if list.is_empty() {
            ui.label(
                RichText::new("No code mods are installed. Only Trinity Core is loaded.")
                    .size(12.0)
                    .color(p.muted),
            );
            return;
        }
        for m in list {
            let (compat, tone) = match (m.api, m.enabled) {
                (_, false) => ("switched off".to_string(), Tone::Neutral),
                (Api::Trinity, true) if !core_ok => (
                    "needs Trinity Core, which is not loaded".to_string(),
                    Tone::Bad,
                ),
                (Api::Trinity, true) => {
                    let abi = m
                        .plugin
                        .as_ref()
                        .and_then(|p| p.abi)
                        .map(|a| format!("declares NXP ABI {}–{}", a.min, a.max))
                        .unwrap_or_else(|| "uses the NXP layer".to_string());
                    if m.uses_trinity() {
                        (format!("{abi}; Trinity Core is loaded"), Tone::Ok)
                    } else {
                        (abi, Tone::Ok)
                    }
                }
                (Api::NxMod, true) => (
                    "NxModApi: the runtime loads it directly, without Trinity".to_string(),
                    Tone::Ok,
                ),
                (Api::Unknown, true) => (
                    "its files do not declare an API: EmblemWeave cannot tell whether it uses Trinity".to_string(),
                    Tone::Warn,
                ),
            };
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&m.info.title).font(theme::semibold(13.0)).color(p.ink));
                        if !m.version().is_empty() {
                            ui.label(RichText::new(m.version()).font(theme::mono(10.5)).color(p.muted));
                        }
                    });
                    ui.label(RichText::new(format!("{}  ·  {compat}", m.api.label())).size(11.0).color(p.body));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    widgets::pill(ui, p, if m.enabled { "on" } else { "off" }, if m.enabled { tone } else { Tone::Neutral });
                });
            });
            ui.separator();
        }
        ui.label(
            RichText::new(
                "Read from each mod's plugin.json and DLL exports. A DLL that declares neither is reported as unknown rather than guessed at.",
            )
            .size(11.0)
            .color(p.muted),
        );
    });
}

fn logo_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    static TEXTURE: Mutex<Option<egui::TextureHandle>> = Mutex::new(None);
    let mut guard = TEXTURE.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(texture) = guard.as_ref() {
        return Some(texture.clone());
    }
    let decoded = image::load_from_memory_with_format(LOGO, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
    let texture = ctx.load_texture("trinity-logo", pixels, egui::TextureOptions::LINEAR);
    *guard = Some(texture.clone());
    Some(texture)
}

fn show_logo(ui: &mut egui::Ui) {
    let Some(texture) = logo_texture(ui.ctx()) else {
        return;
    };
    let width = 168.0_f32.min(ui.available_width());
    let height = width * texture.size()[1] as f32 / texture.size()[0] as f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    ui.painter().image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn hero(ui: &mut egui::Ui, p: &Palette, catalog: &Catalog, prefix_count: usize) {
    widgets::card_frame(p, egui::Margin::symmetric(24, 20)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            show_logo(ui);
        });
        ui.add_space(4.0);
        widgets::eyebrow(ui, p, &format!("NXP LAYER {}", catalog.version));
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "A common NXP contract, a compatibility bridge for the existing loaders, and a catalog that keeps each launcher's own character.",
            )
            .size(13.5)
            .color(p.body),
        );
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            widgets::pill(ui, p, &format!("{} launchers", catalog.houses.len()), Tone::Neutral);
            widgets::pill(ui, p, &format!("{} listings", catalog.listings.len()), Tone::Neutral);
            widgets::pill(ui, p, &format!("{} prefixes", prefix_count), Tone::Neutral);
            widgets::pill(ui, p, &format!("NXP {}", catalog.version), Tone::Accent);
        });
    });
    ui.add_space(theme::SPACE_M);
}

fn metrics(ui: &mut egui::Ui, p: &Palette, catalog: &Catalog, prefixes: &Prefixes) {
    widgets::section(ui, p, "", |ui| {
        ui.columns(4, |cols| {
            metric(
                &mut cols[0],
                p,
                &catalog.houses.len().to_string(),
                "Launchers connected",
                "one shared contract",
            );
            metric(
                &mut cols[1],
                p,
                &catalog.listings.len().to_string(),
                "Listings",
                "runtimes, bridges, standards",
            );
            metric(
                &mut cols[2],
                p,
                &(prefixes.neutral.len() + prefixes.hosts.len()).to_string(),
                "Owned prefixes",
                "no silent collisions",
            );
            metric(
                &mut cols[3],
                p,
                &catalog.capabilities.len().to_string(),
                "Core capabilities",
                "UI, settings, files, frame",
            );
        });
    });
}

fn metric(ui: &mut egui::Ui, p: &Palette, value: &str, label: &str, note: &str) {
    ui.label(
        RichText::new(value)
            .font(theme::display_bold(25.0))
            .color(p.ink),
    );
    ui.label(
        RichText::new(label)
            .font(theme::semibold(12.0))
            .color(p.body),
    );
    ui.label(RichText::new(note).size(10.5).color(p.muted));
}

fn houses(ui: &mut egui::Ui, p: &Palette, catalog: &Catalog) {
    widgets::section(ui, p, "Browse launchers", |ui| {
        ui.columns(3, |cols| {
            for (i, house) in catalog.houses.iter().take(3).enumerate() {
                house_card(&mut cols[i], p, house);
            }
        });
    });
}

fn house_card(ui: &mut egui::Ui, p: &Palette, house: &House) {
    let accent = parse_accent(&house.accent);
    widgets::card_frame(p, egui::Margin::symmetric(15, 13)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            egui::Frame::new()
                .fill(alpha(accent, 28))
                .stroke(egui::Stroke::new(1.0_f32, alpha(accent, 110)))
                .corner_radius(egui::CornerRadius::same(7))
                .inner_margin(egui::Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(&house.monogram)
                            .font(theme::mono(10.0))
                            .color(accent),
                    );
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let tone = if house.tone == "ok" {
                    Tone::Ok
                } else {
                    Tone::Warn
                };
                widgets::pill(ui, p, &house.status, tone);
            });
        });
        ui.add_space(7.0);
        ui.label(
            RichText::new(&house.name)
                .font(theme::semibold(15.0))
                .color(p.ink),
        );
        ui.add_space(5.0);
        widgets::kv(ui, p, "Prefix", &house.prefix);
        widgets::kv(ui, p, "Host API", &house.host_api);
        widgets::kv(ui, p, "Surface", &house.launcher);
        ui.add_space(4.0);
        ui.label(RichText::new(&house.evidence).size(10.5).color(p.muted));
    });
}

fn registry(ui: &mut egui::Ui, p: &Palette, catalog: &Catalog) {
    widgets::section(ui, p, "Registry", |ui| {
        for listing in &catalog.listings {
            let tone = if listing.tone == "ok" {
                Tone::Ok
            } else {
                Tone::Warn
            };
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(&listing.id)
                            .font(theme::mono(9.5))
                            .color(p.muted),
                    );
                    ui.label(
                        RichText::new(&listing.name)
                            .font(theme::semibold(13.5))
                            .color(p.ink),
                    );
                    ui.label(RichText::new(&listing.summary).size(11.0).color(p.body));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The row is as tall as its description; without a height of
                    // its own the pill would be drawn down the whole row.
                    ui.set_max_height(22.0);
                    widgets::pill(ui, p, &listing.status, tone);
                    let marks = listing
                        .hosts
                        .iter()
                        .filter_map(|id| {
                            catalog
                                .houses
                                .iter()
                                .find(|h| h.id == *id || h.monogram == *id)
                        })
                        .map(|h| h.monogram.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !marks.is_empty() {
                        ui.label(RichText::new(marks).font(theme::mono(9.0)).color(p.muted));
                    }
                    ui.label(
                        RichText::new(format!(
                            "{}  {}",
                            listing.kind.to_uppercase(),
                            listing.version
                        ))
                        .font(theme::mono(9.5))
                        .color(p.muted),
                    );
                });
            });
            ui.add_space(4.0);
            ui.separator();
        }
    });
}

fn capabilities(ui: &mut egui::Ui, p: &Palette, catalog: &Catalog) {
    widgets::section(ui, p, "Core capabilities", |ui| {
        for capability in &catalog.capabilities {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [180.0, 20.0],
                    egui::Label::new(
                        RichText::new(&capability.name)
                            .font(theme::semibold(12.0))
                            .color(p.ink),
                    ),
                );
                ui.label(RichText::new(&capability.detail).size(11.0).color(p.body));
            });
        }
    });
}

fn prefix_table(ui: &mut egui::Ui, p: &Palette, catalog: &Catalog, prefixes: &Prefixes) {
    widgets::section(ui, p, "Prefix registry", |ui| {
        ui.label(
            RichText::new(
                "A host claims one namespace. Portable services use trinity., and the compatibility ABI keeps nxp. Existing symbols stay valid.",
            )
            .size(12.0)
            .color(p.body),
        );
        ui.add_space(5.0);
        for entry in prefixes
            .neutral
            .iter()
            .map(|e| (e, "portable"))
            .chain(prefixes.hosts.iter().map(|e| (e, "host")))
        {
            let accent = prefix_accent(entry.0, entry.1 == "host", catalog, p.gold);
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(132.0, 20.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let (dot, _) =
                            ui.allocate_exact_size(egui::vec2(6.0, 6.0), egui::Sense::hover());
                        ui.painter().circle_filled(dot.center(), 2.5, accent);
                        ui.add_space(3.0);
                        ui.label(
                            RichText::new(&entry.0.prefix)
                                .font(theme::mono(10.5))
                                .color(accent),
                        );
                    },
                );
                let name = if entry.0.owner.is_empty() {
                    &entry.0.id
                } else {
                    &entry.0.owner
                };
                ui.label(RichText::new(name).size(11.0).color(p.body));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(entry.1).size(9.5).color(p.muted));
                });
            });
            ui.separator();
        }
    });
}

fn prefix_accent(
    entry: &PrefixEntry,
    is_host: bool,
    catalog: &Catalog,
    fallback: Color32,
) -> Color32 {
    if !is_host {
        return fallback;
    }
    catalog
        .houses
        .iter()
        .find(|house| house.id == entry.id)
        .map(|house| parse_accent(&house.accent))
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn trinity_root() -> PathBuf {
        std::env::var_os("TRINITY_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../Trinity")
                    .to_path_buf()
            })
    }

    fn assert_embedded_matches_shared(name: &str, embedded: &str) {
        let shared = trinity_root().join("shared").join(name);
        let expected = std::fs::read_to_string(&shared).unwrap_or_else(|error| {
            panic!(
                "cannot check the embedded Trinity {name} for drift: {} could not be read ({error}).\nSet TRINITY_ROOT to the Trinity checkout.",
                shared.display()
            )
        });
        let embedded = embedded.replace("\r\n", "\n");
        let expected = expected.replace("\r\n", "\n");
        if embedded == expected {
            return;
        }
        let first_difference = embedded
            .lines()
            .zip(expected.lines())
            .position(|(a, b)| a != b)
            .map(|line| line + 1)
            .unwrap_or_else(|| embedded.lines().count().min(expected.lines().count()) + 1);
        panic!(
            "EmblemWeave's embedded Trinity {name} has drifted from {} (first difference at line {first_difference}).\nRe-sync it from Trinity/shared before building the host page.",
            shared.display()
        );
    }

    #[test]
    fn the_embedded_catalog_has_the_three_houses() {
        let catalog: Catalog = serde_json::from_str(CATALOG).unwrap();
        assert_eq!(catalog.houses.len(), 3);
        assert_eq!(catalog.listings.len(), 5);
        assert!(catalog.houses.iter().all(|h| h.prefix.ends_with('.')));
    }

    #[test]
    fn the_embedded_prefix_registry_has_each_owner_once() {
        let prefixes: Prefixes = serde_json::from_str(PREFIXES).unwrap();
        assert_eq!(prefixes.neutral.len(), 2);
        assert_eq!(prefixes.hosts.len(), 3);
    }

    #[test]
    fn the_embedded_catalog_matches_trinity_shared() {
        assert_embedded_matches_shared("catalog.json", CATALOG);
    }

    #[test]
    fn the_embedded_prefix_registry_matches_trinity_shared() {
        assert_embedded_matches_shared("prefixes.json", PREFIXES);
    }

    #[test]
    fn house_prefixes_take_their_catalog_accent() {
        let catalog: Catalog = serde_json::from_str(CATALOG).unwrap();
        let prefixes: Prefixes = serde_json::from_str(PREFIXES).unwrap();
        let expected = [
            ("emblemweave", Color32::from_rgb(0x3F, 0x80, 0xFF)),
            ("fefw-recomp", Color32::from_rgb(0xA7, 0x8B, 0xFA)),
            ("obelisk-chamber", Color32::from_rgb(0x6F, 0xCF, 0x97)),
        ];
        for (id, want) in expected {
            let entry = prefixes.hosts.iter().find(|e| e.id == id).unwrap();
            assert_eq!(prefix_accent(entry, true, &catalog, Color32::GOLD), want);
        }
        assert_eq!(
            prefix_accent(&prefixes.neutral[0], false, &catalog, Color32::GOLD),
            Color32::GOLD
        );
    }
}

fn parse_accent(hex: &str) -> Color32 {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return Color32::from_rgb(0x3F, 0x80, 0xFF);
    }
    match u32::from_str_radix(h, 16) {
        Ok(v) => Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8),
        Err(_) => Color32::from_rgb(0x3F, 0x80, 0xFF),
    }
}
