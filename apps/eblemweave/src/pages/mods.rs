//! Mods: the installed RomFS mods, importing archives and folders, and
//! browsing and installing from GameBanana.

use crate::mods::{self, GbFile, GbMod, Installed};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText};
use std::sync::{Arc, Mutex};

type Slot<T> = Arc<Mutex<Option<Result<T, String>>>>;

#[derive(Default)]
pub struct Ui {
    link: String,
    browse: Vec<GbMod>,
    more: bool,
    page: u32,
    listing: Option<Slot<(Vec<GbMod>, bool)>>,
    /// The mod whose files are listed, and its files.
    files: Option<(GbMod, Vec<GbFile>)>,
    fetching: Option<Slot<(GbMod, Vec<GbFile>)>>,
    installing: Option<(String, Slot<Installed>)>,
    /// The name typed into the save-as-profile box.
    profile_name: String,
}

fn spawn<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Slot<T> {
    let slot: Slot<T> = Arc::new(Mutex::new(None));
    let s = slot.clone();
    std::thread::spawn(move || {
        let r = f();
        *s.lock().unwrap() = Some(r);
    });
    slot
}

fn take<T>(slot: &Option<Slot<T>>) -> Option<Result<T, String>> {
    slot.as_ref().and_then(|s| s.lock().unwrap().take())
}

fn installed_message(what: &str, r: Result<Installed, String>) -> (String, Tone) {
    match r {
        Ok(i) if i.exefs_only => (
            format!("{what} only patches the game's code (exefs), which cannot apply to a recompiled game. Nothing was installed."),
            Tone::Warn,
        ),
        Ok(i) if i.mods.len() > 1 => (format!("Installed {what} as {} variants; only the first is switched on: {}", i.mods.len(), i.mods.join(", ")), Tone::Ok),
        Ok(i) => (format!("Installed {}", i.mods.join(", ")), Tone::Ok),
        Err(e) => (format!("Could not install {what}: {e}"), Tone::Bad),
    }
}

/// Start loading the game's GameBanana list (the tour uses it too).
pub fn load_gamebanana(app: &mut App) {
    app.mods_ui.page = 1;
    app.mods_ui.listing = Some(spawn(|| mods::gb_list(1)));
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Mods", "RomFS mods laid over the game's files when it starts, from an archive, a folder or GameBanana.");
    let Some(dir) = mods::dir() else { return };
    let mut message: Option<(String, Tone)> = None;
    let busy = app.mods_ui.installing.is_some() || app.mods_ui.fetching.is_some() || app.mods_ui.listing.is_some();
    if busy {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }

    // Results from background work.
    if let Some(r) = take(&app.mods_ui.listing) {
        app.mods_ui.listing = None;
        match r {
            Ok((list, more)) => {
                if app.mods_ui.page <= 1 {
                    app.mods_ui.browse = list;
                } else {
                    app.mods_ui.browse.extend(list);
                }
                app.mods_ui.more = more;
            }
            Err(e) => message = Some((format!("GameBanana: {e}"), Tone::Bad)),
        }
    }
    if let Some(r) = take(&app.mods_ui.fetching) {
        app.mods_ui.fetching = None;
        match r {
            Ok((m, files)) if files.is_empty() => message = Some((format!("{} has no files to download", m.name), Tone::Warn)),
            Ok((m, files)) if files.len() == 1 => {
                let d = dir.clone();
                let (mm, f) = (m.clone(), files[0].clone());
                app.mods_ui.installing = Some((m.name.clone(), spawn(move || mods::gb_install(&mm, &f, &d))));
            }
            Ok(x) => app.mods_ui.files = Some(x),
            Err(e) => message = Some((format!("GameBanana: {e}"), Tone::Bad)),
        }
    }
    if let Some((name, slot)) = &app.mods_ui.installing {
        let done = slot.lock().unwrap().take();
        if let Some(r) = done {
            message = Some(installed_message(name, r));
            app.mods_ui.installing = None;
        }
    }

    widgets::section(ui, &p, "Installed", |ui| {
        let list = mods::list(&dir);
        if list.is_empty() {
            ui.label(RichText::new("No mods installed.").color(p.muted));
        }
        for m in &list {
            ui.horizontal(|ui| {
                let mut on = m.enabled;
                if widgets::toggle(ui, &p, &mut on).changed() {
                    if let Err(e) = mods::set_enabled(m, on) {
                        message = Some((e, Tone::Bad));
                    }
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new(&m.info.title).font(theme::semibold(14.0)).color(if m.enabled { p.ink } else { p.muted }));
                    let mut bits = vec![format!("{} files", m.files)];
                    if !m.info.author.is_empty() {
                        bits.push(format!("by {}", m.info.author));
                    }
                    if !m.info.version.is_empty() {
                        bits.push(m.info.version.clone());
                    }
                    if !m.has_romfs {
                        bits.push("no romfs folder: the game will not see it".into());
                    }
                    if m.has_exefs {
                        bits.push("its exefs patches cannot apply".into());
                    }
                    ui.label(RichText::new(bits.join("  ·  ")).size(12.5).color(p.muted));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, &p, "Remove", true).on_hover_text("Moves the mod to the mods-removed folder").clicked() {
                        message = Some(match mods::remove(m) {
                            Ok(to) => (format!("Moved {} to {}", m.info.title, to.display()), Tone::Ok),
                            Err(e) => (e, Tone::Bad),
                        });
                    }
                    if widgets::button(ui, &p, "Folder", true).clicked() {
                        let _ = std::process::Command::new("explorer").arg(&m.path).spawn();
                    }
                    if !m.info.source.is_empty() {
                        ui.hyperlink_to("Page", &m.info.source);
                    }
                });
            });
            ui.add_space(4.0);
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Changes apply the next time the game starts. When two mods change the same file, the one later in the list wins.").size(12.5).color(p.muted));
    });

    // The rest of the page reads the folder once, after the toggles above.
    let installed = mods::list(&dir);
    let shared = mods::conflicts(&installed);

    widgets::section(ui, &p, "Conflicts", |ui| {
        let on = installed.iter().filter(|m| m.enabled).count();
        if shared.is_empty() {
            widgets::status_line(ui, &p, Tone::Ok, "No file is replaced twice", &format!("{on} mods are on and none of them ships a file another one ships."));
            return;
        }
        widgets::status_line(
            ui,
            &p,
            Tone::Warn,
            &format!("{} shared file{}", shared.len(), if shared.len() == 1 { "" } else { "s" }),
            "The engine lays mods down in name order, so for these files the last mod wins. Switch one off to decide it yourself.",
        );
        ui.add_space(theme::SPACE_S);
        egui::ScrollArea::vertical().id_salt("conflicts").max_height(240.0).show(ui, |ui| {
            egui::Grid::new("conflict-grid").num_columns(2).spacing([20.0, 6.0]).striped(true).show(ui, |ui| {
                for c in &shared {
                    ui.label(RichText::new(&c.file).font(theme::mono(11.5)).color(p.body));
                    ui.label(RichText::new(c.mods.join("  ›  ")).size(12.0).color(p.warn));
                    ui.end_row();
                }
            });
        });
    });

    widgets::section(ui, &p, "Profiles", |ui| {
        ui.label(
            RichText::new("A profile is a set of mods to switch on together, so a whole setup can be swapped without touching each mod.")
                .size(12.5)
                .color(p.muted),
        );
        ui.add_space(theme::SPACE_S);
        let mut apply = None;
        let mut overwrite = None;
        let mut delete = None;
        for (i, prof) in app.config.profiles.iter().enumerate() {
            let on = prof.enabled.iter().filter(|n| installed.iter().any(|m| m.name.eq_ignore_ascii_case(n) && m.enabled)).count();
            ui.horizontal(|ui| {
                ui.label(RichText::new(&prof.name).font(theme::semibold(13.5)).color(p.ink));
                ui.label(RichText::new(format!("{on} of {} mods on", installed.len())).size(12.0).color(p.muted));
                if app.config.profile.eq_ignore_ascii_case(&prof.name) {
                    widgets::pill(ui, &p, "applied", Tone::Accent);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, &p, "Delete", true).clicked() {
                        delete = Some(i);
                    }
                    if widgets::button(ui, &p, "Update", true)
                        .on_hover_text("Save the mods that are on now into this profile")
                        .clicked()
                    {
                        overwrite = Some(i);
                    }
                    if widgets::button(ui, &p, "Apply", true).clicked() {
                        apply = Some(i);
                    }
                });
            });
            ui.add_space(2.0);
        }
        if app.config.profiles.is_empty() {
            ui.label(RichText::new("No profiles yet.").size(12.5).color(p.muted));
        }
        ui.add_space(theme::SPACE_S);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut app.mods_ui.profile_name).hint_text("Profile name").desired_width(220.0));
            let named = !app.mods_ui.profile_name.trim().is_empty();
            if widgets::button(ui, &p, "Save the mods that are on", named).clicked() {
                let name = app.mods_ui.profile_name.trim().to_string();
                let enabled: Vec<String> = installed.iter().filter(|m| m.enabled).map(|m| m.name.clone()).collect();
                match app.config.profiles.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&name)) {
                    Some(existing) => existing.enabled = enabled,
                    None => app.config.profiles.push(crate::config::ModProfile { name: name.clone(), enabled }),
                }
                app.config.profile = name.clone();
                app.mods_ui.profile_name.clear();
                message = Some((format!("Saved the mods that are on as {name}"), Tone::Ok));
            }
        });

        if let Some(i) = apply {
            let want = app.config.profiles[i].enabled.clone();
            match mods::apply_profile(&installed, &want) {
                Ok(n) => {
                    app.config.profile = app.config.profiles[i].name.clone();
                    message = Some((format!("Applied {}: {n} mods changed", app.config.profiles[i].name), Tone::Ok));
                }
                Err(e) => message = Some((e, Tone::Bad)),
            }
        }
        if let Some(i) = overwrite {
            let enabled: Vec<String> = installed.iter().filter(|m| m.enabled).map(|m| m.name.clone()).collect();
            app.config.profiles[i].enabled = enabled;
            message = Some((format!("Updated {}", app.config.profiles[i].name), Tone::Ok));
        }
        if let Some(i) = delete {
            let gone = app.config.profiles.remove(i);
            if app.config.profile.eq_ignore_ascii_case(&gone.name) {
                app.config.profile.clear();
            }
            message = Some((format!("Removed {}", gone.name), Tone::Ok));
        }
    });

    widgets::section(ui, &p, "Add a mod", |ui| {
        ui.horizontal(|ui| {
            if widgets::button(ui, &p, "From archive…", !busy).clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter("Mod archive", &["zip", "7z", "rar", "tar", "gz"]).pick_file() {
                    let d = dir.clone();
                    let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    app.mods_ui.installing = Some((name, spawn(move || mods::install(&f, &d, mods::Info::default()))));
                }
            }
            if widgets::button(ui, &p, "From folder…", !busy).clicked() {
                if let Some(f) = rfd::FileDialog::new().pick_folder() {
                    message = Some(installed_message(&f.display().to_string(), mods::install(&f, &dir, mods::Info::default())));
                }
            }
            if widgets::button(ui, &p, "Open mods folder", true).clicked() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut app.mods_ui.link).hint_text("https://gamebanana.com/mods/…").desired_width(320.0));
            let id = mods::gb_id(&app.mods_ui.link);
            if widgets::button(ui, &p, "Install from GameBanana", id.is_some() && !busy).clicked() {
                let id = id.unwrap();
                app.mods_ui.fetching = Some(spawn(move || mods::gb_mod(id)));
            }
        });
        if let Some((name, _)) = &app.mods_ui.installing {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Installing {name}…"));
            });
        }
    });

    let mut chosen: Option<(GbMod, GbFile)> = None;
    if let Some((m, files)) = &app.mods_ui.files {
        let mut close = false;
        widgets::section(ui, &p, &format!("Files of {}", m.name), |ui| {
            for f in files {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&f.name).font(theme::mono(13.0)).color(p.ink));
                    ui.label(RichText::new(format!("{:.1} MB", f.size as f64 / 1e6)).color(p.muted));
                    if !f.description.is_empty() {
                        ui.label(RichText::new(&f.description).color(p.body));
                    }
                    if widgets::button(ui, &p, "Install", !busy).clicked() {
                        chosen = Some((m.clone(), f.clone()));
                    }
                });
            }
            if widgets::button(ui, &p, "Close", true).clicked() {
                close = true;
            }
        });
        if close {
            app.mods_ui.files = None;
        }
    }
    if let Some((m, f)) = chosen {
        let d = dir.clone();
        app.mods_ui.files = None;
        app.mods_ui.installing = Some((m.name.clone(), spawn(move || mods::gb_install(&m, &f, &d))));
    }

    widgets::section(ui, &p, "GameBanana", |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Fire Emblem: Fortune's Weave mods, newest first.").color(p.body));
            if widgets::button(ui, &p, if app.mods_ui.browse.is_empty() { "Load list" } else { "Refresh" }, app.mods_ui.listing.is_none()).clicked() {
                app.mods_ui.page = 1;
                app.mods_ui.listing = Some(spawn(|| mods::gb_list(1)));
            }
            ui.hyperlink_to("Open on gamebanana.com", format!("https://gamebanana.com/mods/games/{}", mods::GAMEBANANA_GAME));
        });
        if app.mods_ui.listing.is_some() {
            ui.spinner();
        }
        let installed: Vec<u64> = mods::list(&dir).iter().filter_map(|m| m.info.gamebanana_id).collect();
        let mut fetch: Option<u64> = None;
        egui::Grid::new("gb").num_columns(4).spacing([14.0, 6.0]).striped(true).show(ui, |ui| {
            for m in &app.mods_ui.browse {
                ui.vertical(|ui| {
                    ui.set_min_width(460.0);
                    ui.hyperlink_to(RichText::new(&m.name).font(theme::semibold(13.5)), &m.url);
                    ui.label(RichText::new(format!("{} by {}{}", m.kind, m.author, if m.version.is_empty() { String::new() } else { format!(", {}", m.version) })).size(12.0).color(p.muted));
                });
                ui.label(RichText::new(format!("♥ {}", m.likes)).color(p.muted));
                if installed.contains(&m.id) {
                    widgets::pill(ui, &p, "Installed", Tone::Ok);
                } else {
                    ui.label("");
                }
                if m.kind == "Mod" {
                    if widgets::button(ui, &p, "Install", !busy).clicked() {
                        fetch = Some(m.id);
                    }
                } else {
                    ui.label(RichText::new("a tool, not a mod").size(12.0).color(p.muted));
                }
                ui.end_row();
            }
        });
        if let Some(id) = fetch {
            app.mods_ui.fetching = Some(spawn(move || mods::gb_mod(id)));
        }
        if app.mods_ui.more && widgets::button(ui, &p, "More", app.mods_ui.listing.is_none()).clicked() {
            app.mods_ui.page += 1;
            let n = app.mods_ui.page;
            app.mods_ui.listing = Some(spawn(move || mods::gb_list(n)));
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Mods are made by their authors and installed as they are; mods that patch the game's code (exefs) cannot apply.").size(12.5).color(p.muted));
    });

    if let Some((text, tone)) = message {
        app.notify(text, tone);
    }
}
