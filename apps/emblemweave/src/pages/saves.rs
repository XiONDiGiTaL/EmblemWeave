//! Saves: save profiles (named copies to switch between), the slots in the
//! current save with an editor, and import and export.

use crate::savefile::{grade, Ids, Player, Slot, Unit, GRADES, MASTERY_NAMES, STAT_NAMES};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText};
use std::path::PathBuf;

#[derive(Default)]
pub struct Ui {
    pub profile_name: String,
    pub editor: Option<Editor>,
    /// Scroll the page to the editor on the next frame (the tour asks for it).
    pub scroll_to_editor: bool,
    ids: Option<std::sync::Arc<Ids>>,
}

pub struct Editor {
    slot: Slot,
    player: Option<Player>,
    units: Vec<Unit>,
    original: (Option<Player>, Vec<Unit>),
}

fn stamp() -> String {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string()
}

/// Bytes as the page shows them: KB under a megabyte, then one decimal.
fn megabytes(bytes: u64) -> String {
    if bytes < 1 << 20 {
        format!("{} KB", bytes.div_ceil(1024).max(1))
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn ago(t: Option<std::time::SystemTime>) -> String {
    let Some(secs) = t.and_then(|t| std::time::SystemTime::now().duration_since(t).ok()).map(|d| d.as_secs()) else {
        return String::new();
    };
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

/// The game's RomFS as a folder, where the game data (and so the names of
/// units) can be read: the one the engine was configured with, else the
/// dump's when it is a folder.
pub fn romfs_dir(app: &App) -> Option<PathBuf> {
    let ini = crate::engine::run_dir().and_then(|d| std::fs::read_to_string(d.join("fefw.ini")).ok());
    let from_ini = ini.and_then(|t| {
        t.lines().find_map(|l| l.trim().strip_prefix("romfs").map(|r| r.trim_start().trim_start_matches('=').trim().to_string()))
    });
    from_ini.map(PathBuf::from).filter(|p| p.join("GameData").is_dir()).or_else(|| app.game.romfs.clone().filter(|p| p.join("GameData").is_dir()))
}

/// Open a slot in the editor.
pub fn open_slot(app: &mut App, path: &std::path::Path) -> Result<(), String> {
    let slot = Slot::open(path)?;
    if app.saves_ui.ids.is_none() {
        app.saves_ui.ids = Some(std::sync::Arc::new(romfs_dir(app).map(|r| Ids::from_romfs(&r)).unwrap_or_default()));
    }
    let ids = app.saves_ui.ids.clone().unwrap_or_default();
    let (player, units) = (slot.player(), slot.units(&ids));
    app.saves_ui.editor = Some(Editor { original: (player, units.clone()), slot, player, units });
    Ok(())
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Saves", "Switch between saves, edit a slot, and bring saves in from an emulator.");
    let Some(save_dir) = crate::config::Config::save_dir() else { return };
    let title = app
        .game
        .title_id
        .or_else(|| app.game.nacp.as_deref().and_then(crate::saves::nacp_save_owner))
        .unwrap_or(crate::saves::FALLBACK_TITLE_ID);
    let running = crate::platform::game_running();
    let mut message: Option<(String, Tone)> = None;

    if running {
        widgets::status_line(ui, &p, Tone::Warn, "The game is running", "Close it before switching or editing saves: it writes its save when it closes.");
        ui.add_space(theme::SPACE_S);
    }

    widgets::section(ui, &p, "Save profiles", |ui| {
        ui.label("A profile is a named copy of the whole save. Loading one makes it the current save; the save it replaces is kept in the backups folder.");
        ui.add_space(6.0);
        let profiles = crate::saves::profiles(&save_dir);
        if profiles.is_empty() {
            ui.label(RichText::new("No profiles yet.").color(p.muted));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Profile name").font(theme::semibold(13.0)).color(p.ink));
            ui.add(egui::TextEdit::singleline(&mut app.saves_ui.profile_name).hint_text("Run A, before the choice").desired_width(220.0));
            let ok = !app.saves_ui.profile_name.trim().is_empty();
            if widgets::button(ui, &p, "Keep current save as profile", ok).clicked() {
                let name = app.saves_ui.profile_name.trim().to_string();
                message = Some(match crate::saves::store_profile(&save_dir, &name, &stamp()) {
                    Ok(n) => (format!("Kept the current save ({n} files) as profile {name}"), Tone::Ok),
                    Err(e) => (format!("Could not keep the save: {e}"), Tone::Bad),
                });
                app.saves_ui.profile_name.clear();
            }
        });
        ui.label(
            RichText::new("The same name is what Rename and Copy use on a profile below: type a new one, then press the button on that row.")
                .size(12.0)
                .color(p.muted),
        );
        let typed = app.saves_ui.profile_name.trim().to_string();
        for pr in &profiles {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&pr.name).font(theme::semibold(14.0)).color(p.ink));
                ui.label(RichText::new(format!("{} files, {}", pr.files, ago(pr.modified))).color(p.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, &p, "Remove", true).on_hover_text("Moves the profile to the backups folder").clicked() {
                        message = Some(match crate::saves::remove_profile(&save_dir, pr, &stamp()) {
                            Ok(b) => (format!("Moved profile {} to {}", pr.name, b.display()), Tone::Ok),
                            Err(e) => (format!("Could not remove the profile: {e}"), Tone::Bad),
                        });
                    }
                    // Rename and copy take the name in the field above, so one input serves the three
                    // things a player does with a profile: keep, rename, branch.
                    let named = !typed.is_empty() && typed != pr.name;
                    if widgets::button(ui, &p, "Copy", named)
                        .on_hover_text("Keeps a second copy under the name in the field above, for a branch of the story")
                        .clicked()
                    {
                        message = Some(match crate::saves::duplicate_profile(&save_dir, pr, &typed) {
                            Ok(c) => (format!("Copied {} to {}", pr.name, c.name), Tone::Ok),
                            Err(e) => (format!("Could not copy the profile: {e}"), Tone::Bad),
                        });
                    }
                    if widgets::button(ui, &p, "Rename", named).on_hover_text("Renames this profile to the name in the field above").clicked() {
                        message = Some(match crate::saves::rename_profile(&save_dir, pr, &typed) {
                            Ok(r) => (format!("Renamed {} to {}", pr.name, r.name), Tone::Ok),
                            Err(e) => (format!("Could not rename the profile: {e}"), Tone::Bad),
                        });
                    }
                    if widgets::button(ui, &p, "Load", !running).clicked() {
                        message = Some(match crate::saves::load_profile(&save_dir, pr, title, &stamp()) {
                            Ok(i) => {
                                app.saves_ui.editor = None;
                                (format!("Loaded profile {} ({} files){}", pr.name, i.files, i.backup.map(|b| format!("; the previous save is in {}", b.display())).unwrap_or_default()), Tone::Ok)
                            }
                            Err(e) => (format!("Could not load the profile: {e}"), Tone::Bad),
                        });
                    }
                });
            });
        }

    });

    widgets::section(ui, &p, "Slots in the current save", |ui| {
        let account = crate::saves::account_dir(&save_dir);
        let slots = crate::saves::slots_with_sizes(&save_dir);
        ui.label(
            RichText::new(format!(
                "The save the game reads is {}: {} slot{}, {} in total.",
                account.display(),
                slots.len(),
                if slots.len() == 1 { "" } else { "s" },
                megabytes(slots.iter().map(|s| s.2).sum())
            ))
            .size(12.5)
            .color(p.muted),
        );
        if slots.is_empty() {
            ui.label(RichText::new("No save yet. Play the game or import a save below.").color(p.muted));
        }
        egui::Grid::new("slots").num_columns(4).spacing([18.0, 6.0]).striped(true).show(ui, |ui| {
            for (path, when, size) in &slots {
                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                ui.label(RichText::new(&name).font(theme::mono(13.0)).color(p.ink));
                ui.label(RichText::new(format!("{}, {}", megabytes(*size), ago(*when))).color(p.muted));
                let open = app.saves_ui.editor.as_ref().is_some_and(|e| &e.slot.path == path);
                if widgets::button(ui, &p, if open { "Editing" } else { "Edit" }, !open).clicked() {
                    if let Err(e) = open_slot(app, path) {
                        message = Some((format!("{name}: {e}"), Tone::Bad));
                    }
                }
                if widgets::button(ui, &p, "Move to backups", !running)
                    .on_hover_text("The game offers a new game in this slot again; the file is kept in the backups folder")
                    .clicked()
                {
                    message = Some(match crate::saves::archive_slot(&save_dir, path, &stamp()) {
                        Ok(to) => {
                            app.saves_ui.editor = None;
                            (format!("Moved {name} to {}", to.display()), Tone::Ok)
                        }
                        Err(e) => (format!("Could not move {name}: {e}"), Tone::Bad),
                    });
                }
                ui.end_row();
            }
        });
    });

    if let Some(m) = editor(app, ui, &p, running) {
        message = Some(m);
    }

    crate::pages::setup::saves_section(app, ui, &p);
    if let Some((text, tone)) = message {
        app.notify(text, tone);
    }
}

fn editor(app: &mut App, ui: &mut egui::Ui, p: &crate::theme::Palette, running: bool) -> Option<(String, Tone)> {
    let named = app.saves_ui.ids.as_ref().is_some_and(|i| !i.people.is_empty());
    let ids = app.saves_ui.ids.clone().unwrap_or_default();
    if app.saves_ui.editor.is_some() && app.saves_ui.scroll_to_editor {
        app.saves_ui.scroll_to_editor = false;
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
    let ed = app.saves_ui.editor.as_mut()?;
    let mut message = None;
    let mut close = false;
    let title = format!("Editing {}", ed.slot.path.file_name().unwrap_or_default().to_string_lossy());
    widgets::section(ui, p, &title, |ui| {
        let problems = ed.slot.verify();
        if !problems.is_empty() {
            widgets::status_line(ui, p, Tone::Warn, "Checksums", &problems.join(", "));
        }
        if let Some(pl) = ed.player.as_mut() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Gold").font(theme::semibold(14.0)).color(p.ink));
                ui.add(egui::DragValue::new(&mut pl.gold).range(0..=9_999_999));
                ui.add_space(18.0);
                ui.label(RichText::new("Renown").font(theme::semibold(14.0)).color(p.ink));
                ui.add(egui::DragValue::new(&mut pl.renown).range(0..=999_999));
            });
            ui.add_space(6.0);
        } else {
            ui.label(RichText::new("This slot has no player data (gold, renown).").color(p.muted));
        }
        if !named {
            ui.label(RichText::new("Units are found by the ids in your game data; choose your game on Game Setup to list them.").color(p.muted));
        } else if ed.units.is_empty() {
            ui.label(RichText::new("No units were found in this slot.").color(p.muted));
        } else {
            ui.label(
                RichText::new("Stats are what each unit has gained from level-ups; the game adds class and personal bases on top. HP is current HP.")
                    .size(12.5)
                    .color(p.muted),
            );
            egui::ScrollArea::horizontal().id_salt("units").show(ui, |ui| {
                egui::Grid::new("units").num_columns(13).spacing([10.0, 6.0]).striped(true).show(ui, |ui| {
                    for h in ["Unit", "Lv", "Exp", "Max HP"].iter().chain(STAT_NAMES.iter()) {
                        ui.label(RichText::new(*h).font(theme::semibold(12.0)).color(p.muted));
                    }
                    ui.end_row();
                    for u in ed.units.iter_mut() {
                        ui.label(RichText::new(&u.name).color(p.ink)).on_hover_text(format!("record at {:#x}", u.at));
                        ui.add(egui::DragValue::new(&mut u.level).range(1..=99));
                        ui.add(egui::DragValue::new(&mut u.exp).range(0..=99));
                        ui.add(egui::DragValue::new(&mut u.max_hp).range(1..=if u.hp_width == 1 { 255 } else { 999 }));
                        for v in u.stats.iter_mut() {
                            ui.add(egui::DragValue::new(v).range(0..=255));
                        }
                        ui.end_row();
                    }
                });
            });
            ui.add_space(6.0);
            for (n, u) in ed.units.iter_mut().enumerate() {
                if u.masteries.is_empty() && u.items.is_empty() && u.spells.is_empty() {
                    continue;
                }
                egui::CollapsingHeader::new(RichText::new(format!("{}: masteries and items", u.name)).color(p.ink)).id_salt(("unit-more", n)).show(ui, |ui| {
                    if !u.masteries.is_empty() {
                        ui.label(RichText::new("Masteries (experience, grade)").font(theme::semibold(12.5)).color(p.muted));
                        egui::Grid::new(("mastery", n)).num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
                            for (_, k, v) in u.masteries.iter_mut() {
                                ui.label(MASTERY_NAMES.get(*k as usize).copied().unwrap_or("?"));
                                ui.add(egui::DragValue::new(v).range(0..=2560));
                                ui.label(RichText::new(grade(*v)).font(theme::semibold(13.0)).color(p.accent_hi));
                                egui::ComboBox::from_id_salt(("grade", n, *k)).selected_text("set grade").width(90.0).show_ui(ui, |ui| {
                                    for (g, t) in GRADES {
                                        if ui.selectable_label(false, g).clicked() {
                                            *v = t;
                                        }
                                    }
                                });
                                ui.end_row();
                            }
                        });
                    }
                    for (label, list) in [("Items", &mut u.items), ("Spells", &mut u.spells)] {
                        if list.is_empty() {
                            continue;
                        }
                        ui.label(RichText::new(label).font(theme::semibold(12.5)).color(p.muted));
                        for (i, (_, id)) in list.iter_mut().enumerate() {
                            item_picker(ui, (label, n, i), id, &ids.items);
                        }
                    }
                    ui.label(RichText::new("Items are swapped in place: the slot keeps its kind (a forged item stays forged). Adding or removing slots is not supported yet.").size(12.0).color(p.muted));
                });
            }
        }
        ui.add_space(8.0);
        let dirty = (ed.player, ed.units.clone()) != ed.original;
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, p, "Save changes", dirty && !running).clicked() {
                if let Some(pl) = ed.player {
                    ed.slot.set_player(&pl);
                }
                for u in &ed.units {
                    ed.slot.write_unit(u);
                }
                message = Some(match ed.slot.save() {
                    Ok(()) => {
                        ed.original = (ed.player, ed.units.clone());
                        (format!("Saved {}; the original is kept as {}.orig", ed.slot.path.display(), ed.slot.path.file_name().unwrap_or_default().to_string_lossy()), Tone::Ok)
                    }
                    Err(e) => (format!("Could not save: {e}"), Tone::Bad),
                });
            }
            if widgets::button(ui, p, "Revert", dirty).clicked() {
                ed.player = ed.original.0;
                ed.units = ed.original.1.clone();
            }
            if widgets::button(ui, p, "Close", true).clicked() {
                close = true;
            }
        });
    });
    if close {
        app.saves_ui.editor = None;
    }
    message
}

/// A searchable choice of item, by the id names in the player's game data.
fn item_picker(ui: &mut egui::Ui, salt: impl std::hash::Hash + Copy, id: &mut u32, names: &std::collections::HashMap<u32, String>) {
    let current = names.get(id).cloned().unwrap_or_else(|| format!("item {id:08x}"));
    let filter_id = ui.make_persistent_id(("item-filter", salt));
    egui::ComboBox::from_id_salt(salt).selected_text(current).width(260.0).height(320.0).show_ui(ui, |ui| {
        let mut filter: String = ui.data_mut(|d| d.get_temp(filter_id).unwrap_or_default());
        ui.add(egui::TextEdit::singleline(&mut filter).hint_text("search").desired_width(240.0));
        ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
        let f = filter.to_lowercase();
        let mut shown: Vec<(&u32, &String)> = names.iter().filter(|(_, n)| f.is_empty() || n.to_lowercase().contains(&f)).collect();
        shown.sort_by(|a, b| a.1.cmp(b.1));
        for (k, n) in shown.into_iter().take(300) {
            if ui.selectable_label(k == id, n).clicked() {
                *id = *k;
            }
        }
    });
}
