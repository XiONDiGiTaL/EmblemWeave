//! Saves: save profiles as a carousel of independent playthroughs, the slots
//! inside a profile, the slot editor, and import/export.
//!
//! The profiles themselves are still the same folders under
//! `<save folder>/profiles/<name>` used by earlier versions. Nothing here
//! changes the game's save layout: selecting a card is only a view operation;
//! loading is a separate, backup-first copy into the account folder the game
//! reads.

use crate::savefile::{grade, Ids, Player, Slot, Unit, GRADES, MASTERY_NAMES, STAT_NAMES};
use crate::saves::{self, Profile, ProfileMeta};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
};
use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Default)]
pub struct Ui {
    pub editor: Option<Editor>,
    /// The profile the page last loaded, so the carousel can say which save is
    /// installed without making the selected card the loaded one.
    pub active_profile: Option<String>,
    /// The selected card. Selection is view-only; `active_profile` is the save
    /// the game actually reads.
    pub selected_profile: Option<String>,
    /// The Create Profile card is selected instead of a profile.
    pub create_selected: bool,
    /// Profile summaries and their parsed slot metadata, rebuilt only when a
    /// profile directory or its newest file changes.
    profile_cache: Option<ProfileCache>,
    pub dialog: Option<CreateDialog>,
    pub rename: Option<RenameDialog>,
    pub manage: Option<ManageDialog>,
    pub delete: Option<String>,
    /// Scroll the page to the editor on the next frame (the tour asks for it).
    pub scroll_to_editor: bool,
    /// Emulator saves found for the title, refreshed at most every few seconds
    /// from the Saves page. Scanning every frame watched every yuzu fork's
    /// NAND and made opening the page slower than it needed to be.
    pub sources: Vec<crate::saves::Found>,
    pub sources_title: Option<u64>,
    pub sources_at: f64,
    ids: Option<Arc<Ids>>,
}

#[derive(Clone)]
struct CachedProfile {
    profile: Profile,
    meta: ProfileMeta,
}

struct ProfileCache {
    key: u64,
    items: Vec<CachedProfile>,
    active: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CreateMode {
    Empty,
    Duplicate,
    Import,
    Current,
}

pub struct CreateDialog {
    mode: CreateMode,
    name: String,
    source: Option<String>,
    import_dir: Option<PathBuf>,
    error: Option<String>,
}

impl CreateDialog {
    fn new(source: Option<String>) -> Self {
        Self { mode: CreateMode::Empty, name: String::new(), source, import_dir: None, error: None }
    }
}

pub struct RenameDialog {
    target: String,
    name: String,
    error: Option<String>,
}

pub struct ManageDialog {
    profile: String,
    pending_delete: Option<PathBuf>,
    notice: Option<(String, Tone)>,
    /// Display-only slot labels. The game's file names and contents are not
    /// changed; see `Config::slot_names`.
    names: BTreeMap<String, String>,
    renaming: Option<String>,
    rename_text: String,
    slots: Option<Vec<crate::saves::SlotInfo>>,
}

pub struct Editor {
    slot: Slot,
    player: Option<Player>,
    units: Vec<Unit>,
    original: (Option<Player>, Vec<Unit>),
}

#[derive(Clone)]
enum Action {
    Select(String),
    Move(i32),
    Create,
    Load,
    Duplicate,
    Manage,
    Export,
    OpenFolder,
    Rename,
    Delete,
    KeepCurrent,
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

pub(crate) fn ago(t: Option<std::time::SystemTime>) -> String {
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

fn count(n: usize) -> String {
    let raw = n.to_string();
    let mut out = String::new();
    for (i, c) in raw.chars().enumerate() {
        if i > 0 && (raw.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn fit(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
    if let Some(space) = s.rfind(' ') {
        if s.len() - space < 12 {
            s.truncate(space);
        }
    }
    s.push('…');
    s
}

/// A long path with its recognisable end kept, rather than a hard clip or the
/// repeated drive prefix. The full value stays in the hover text.
fn tail_path(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let tail: String = chars[chars.len() - max.saturating_sub(1)..].iter().collect();
    format!("…{tail}")
}

/// The game's RomFS as a folder, where the game data (and so the names of
/// units) can be read: the one the engine was configured with, else the dump's
/// when it is a folder.
pub fn romfs_dir(app: &App) -> Option<PathBuf> {
    let ini = crate::engine::run_dir().and_then(|d| std::fs::read_to_string(d.join("fefw.ini")).ok());
    let from_ini = ini.and_then(|t| {
        t.lines().find_map(|l| l.trim().strip_prefix("romfs").map(|r| r.trim_start().trim_start_matches('=').trim().to_string()))
    });
    from_ini.map(PathBuf::from).filter(|p| p.join("GameData").is_dir()).or_else(|| app.game.romfs.clone().filter(|p| p.join("GameData").is_dir()))
}

fn ids_for(app: &mut App) -> Arc<Ids> {
    if app.saves_ui.ids.is_none() {
        app.saves_ui.ids = Some(Arc::new(romfs_dir(app).map(|r| Ids::from_romfs(&r)).unwrap_or_default()));
    }
    app.saves_ui.ids.clone().unwrap_or_default()
}

/// Open a slot in the editor. Profile slots work too: the editor writes the
/// slot it was opened on, without assuming it is the current account save.
pub fn open_slot(app: &mut App, path: &std::path::Path) -> Result<(), String> {
    let slot = Slot::open(path)?;
    let ids = ids_for(app);
    let (player, units) = (slot.player(), slot.units(&ids));
    app.saves_ui.editor = Some(Editor { original: (player, units.clone()), slot, player, units });
    Ok(())
}

fn cache_key(profiles: &[Profile]) -> u64 {
    let mut h = DefaultHasher::new();
    for p in profiles {
        p.name.hash(&mut h);
        p.files.hash(&mut h);
        p.modified
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .hash(&mut h);
    }
    h.finish()
}

fn build_cache(save_dir: &Path, profiles: &[Profile], ids: &Ids) -> ProfileCache {
    let items = profiles
        .iter()
        .map(|profile| CachedProfile { profile: profile.clone(), meta: saves::profile_meta(profile, ids) })
        .collect();
    ProfileCache { key: cache_key(profiles), items, active: saves::installed_profile(save_dir, profiles) }
}

fn selected_index(items: &[CachedProfile], selected: Option<&str>, create: bool) -> usize {
    if create || items.is_empty() {
        items.len()
    } else {
        selected.and_then(|name| items.iter().position(|p| p.profile.name == name)).unwrap_or(0)
    }
}

fn slot_summary(meta: &ProfileMeta, empty: bool) -> (String, Tone) {
    if empty {
        return ("No saves yet".into(), Tone::Accent);
    }
    if meta.slots.is_empty() {
        return ("No save files".into(), Tone::Warn);
    }
    if meta.valid_slots == 0 {
        return (format!("{} unreadable", meta.slots.len()), Tone::Bad);
    }
    if meta.valid_slots < meta.slots.len() {
        return (format!("{} of {} readable", meta.valid_slots, meta.slots.len()), Tone::Warn);
    }
    (format!("{} slot{}", meta.slots.len(), if meta.slots.len() == 1 { "" } else { "s" }), Tone::Ok)
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Saves", "Browse playthroughs as profiles, inspect their saves, and load one only when you choose to.");
    let Some(save_dir) = crate::config::Config::save_dir() else { return };
    let title = app
        .game
        .title_id
        .or_else(|| app.game.nacp.as_deref().and_then(crate::saves::nacp_save_owner))
        .unwrap_or(crate::saves::FALLBACK_TITLE_ID);
    let running = crate::platform::game_running();
    let mut message: Option<(String, Tone)> = None;

    if running {
        widgets::status_line(ui, &p, Tone::Warn, "The game is running", "Close it before loading or editing saves: it writes its save when it closes.");
        ui.add_space(theme::SPACE_S);
    }

    let profiles = crate::saves::profiles(&save_dir);
    let ids = ids_for(app);
    let key = cache_key(&profiles);
    if app.saves_ui.profile_cache.as_ref().is_none_or(|c| c.key != key) {
        app.saves_ui.profile_cache = Some(build_cache(&save_dir, &profiles, &ids));
    }
    let mut inferred_active = app.saves_ui.profile_cache.as_ref().and_then(|c| c.active.clone());
    if inferred_active.as_deref().is_some_and(|name| !profiles.iter().any(|p| p.name == name)) {
        inferred_active = None;
    }
    let mut active = app.saves_ui.active_profile.clone().or(inferred_active);
    if active.as_deref().is_some_and(|name| !profiles.iter().any(|p| p.name == name)) {
        active = None;
    }
    app.saves_ui.active_profile = active.clone();

    if profiles.is_empty() {
        app.saves_ui.create_selected = true;
        app.saves_ui.selected_profile = None;
    } else if !app.saves_ui.create_selected {
        let selected_exists = app.saves_ui.selected_profile.as_deref().is_some_and(|name| profiles.iter().any(|p| p.name == name));
        if !selected_exists {
            app.saves_ui.selected_profile = active.clone().or_else(|| Some(profiles[0].name.clone()));
        }
    }
    let items = app.saves_ui.profile_cache.as_ref().map(|c| c.items.clone()).unwrap_or_default();

    widgets::eyebrow(ui, &p, "Save profiles");
    ui.add_space(2.0);
    ui.label(RichText::new("Independent playthroughs").font(theme::display(20.0)).color(p.ink));
    ui.label(
        RichText::new(if items.is_empty() {
            "Create your first profile below. It keeps its own slots, backups and edits apart from every other playthrough."
        } else {
            "The centre card is selected for viewing. Nothing is loaded until you press Load Profile."
        })
        .size(12.5)
        .color(p.body),
    );
    ui.add_space(theme::SPACE_S);

    let action = if items.is_empty() {
        empty_profiles(ui, &p, &save_dir, running)
    } else {
        carousel(app, ui, &p, &items, active.as_deref(), running)
    };
    if let Some(action) = action {
        match action {
            Action::Select(name) => {
                app.saves_ui.selected_profile = Some(name);
                app.saves_ui.create_selected = false;
            }
            Action::Move(delta) => {
                let n = items.len() + 1;
                let idx = selected_index(&items, app.saves_ui.selected_profile.as_deref(), app.saves_ui.create_selected);
                let next = (idx as i32 + delta).rem_euclid(n as i32) as usize;
                if next == items.len() {
                    app.saves_ui.create_selected = true;
                    app.saves_ui.selected_profile = None;
                } else {
                    app.saves_ui.create_selected = false;
                    app.saves_ui.selected_profile = items.get(next).map(|i| i.profile.name.clone());
                }
            }
            Action::Create => {
                app.saves_ui.dialog = Some(CreateDialog::new(app.saves_ui.selected_profile.clone()));
                app.saves_ui.create_selected = true;
            }
            Action::KeepCurrent => {
                let current = crate::saves::summary(&crate::saves::account_dir(&save_dir)).0 > 0;
                if current {
                    app.saves_ui.dialog = Some(CreateDialog { mode: CreateMode::Current, ..CreateDialog::new(None) });
                } else {
                    message = Some(("There is no current save to keep yet. Play once, or import a save first.".into(), Tone::Warn));
                }
            }
            Action::Load => {
                if let Some(item) = selected_item(&items, app.saves_ui.selected_profile.as_deref()) {
                    let before = app.saves_ui.active_profile.clone();
                    message = Some(load_profile(app, &save_dir, &item.profile, title, active.as_deref(), running));
                    // The carousel above was drawn with the old active profile.
                    // Use the one the load actually installed for the rest of
                    // this frame, and draw the next frame with the new badge
                    // instead of waiting for the mouse to move.
                    if app.saves_ui.active_profile != before {
                        active = app.saves_ui.active_profile.clone();
                        ui.ctx().request_repaint();
                    }
                }
            }
            Action::Duplicate => {
                if let Some(item) = selected_item(&items, app.saves_ui.selected_profile.as_deref()) {
                    app.saves_ui.dialog = Some(CreateDialog { mode: CreateMode::Duplicate, source: Some(item.profile.name.clone()), ..CreateDialog::new(None) });
                }
            }
            Action::Manage => {
                if let Some(item) = selected_item(&items, app.saves_ui.selected_profile.as_deref()) {
                    app.saves_ui.manage = Some(ManageDialog {
                        names: app.config.slot_names.get(&item.profile.name).cloned().unwrap_or_default(),
                        profile: item.profile.name.clone(),
                        pending_delete: None,
                        notice: None,
                        renaming: None,
                        rename_text: String::new(),
                        slots: None,
                    });
                }
            }
            Action::Export => {
                if let Some(item) = selected_item(&items, app.saves_ui.selected_profile.as_deref()) {
                    if let Some(dest) = rfd::FileDialog::new().set_title("Choose a folder for the profile backup").pick_folder() {
                        message = Some(match crate::saves::export_profile(&item.profile, &dest, title) {
                            Ok((to, n)) => (format!("Exported {} to {} ({n} files)", item.profile.name, to.display()), Tone::Ok),
                            Err(e) => (format!("Could not export the profile: {e}"), Tone::Bad),
                        });
                    }
                }
            }
            Action::OpenFolder => {
                if let Some(item) = selected_item(&items, app.saves_ui.selected_profile.as_deref()) {
                    let _ = std::fs::create_dir_all(&item.profile.path);
                    let _ = std::process::Command::new("explorer").arg(&item.profile.path).spawn();
                }
            }
            Action::Rename => {
                if let Some(item) = selected_item(&items, app.saves_ui.selected_profile.as_deref()) {
                    app.saves_ui.rename = Some(RenameDialog { target: item.profile.name.clone(), name: item.profile.name.clone(), error: None });
                }
            }
            Action::Delete => {
                if let Some(item) = selected_item(&items, app.saves_ui.selected_profile.as_deref()) {
                    app.saves_ui.delete = Some(item.profile.name.clone());
                }
            }
        }
    }

    current_save(ui, &p, &save_dir, active.as_deref());

    current_slots(app, ui, &p, &save_dir, &mut message, running);

    if let Some(m) = editor(app, ui, &p, running) {
        message = Some(m);
    }

    // Whole saves kept by an import or a removed profile. A player who loaded
    // the wrong branch wants the old one back without hunting through Explorer.
    let backups = crate::saves::backups(&save_dir);
    widgets::section(ui, &p, "Backups", |ui| {
        if backups.is_empty() {
            ui.label(RichText::new("No whole-save backups yet. Importing a save or loading a profile keeps the save it replaces here.").size(12.5).color(p.muted));
        }
        for b in &backups {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&b.name).font(theme::mono(12.5)).color(p.ink));
                ui.label(RichText::new(format!("{} files, {}", b.files, ago(b.modified))).size(12.0).color(p.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, &p, "Restore", !running)
                        .on_hover_text("Makes this backup the current save; the save it replaces is kept in the backups folder")
                        .clicked()
                    {
                        message = Some(match crate::saves::restore_backup_verified(&save_dir, b, title, &stamp()) {
                            Ok(i) => {
                                app.saves_ui.editor = None;
                                app.saves_ui.active_profile = None;
                                app.saves_ui.profile_cache = None;
                                (format!("Restored {} ({} files){}", b.name, i.files, i.backup.map(|p| format!("; the save it replaced is in {}", p.display())).unwrap_or_default()), Tone::Ok)
                            }
                            Err(e) => (format!("Could not restore {}: {e}", b.name), Tone::Bad),
                        });
                    }
                });
            });
        }
        ui.add_space(4.0);
        if widgets::button(ui, &p, "Open backups folder", true).clicked() {
            let _ = std::fs::create_dir_all(save_dir.join("backups"));
            let _ = std::process::Command::new("explorer").arg(save_dir.join("backups")).spawn();
        }
    });

    crate::pages::setup::saves_section(app, ui, &p);

    // The floating panels are drawn after the page so their backgrounds cover
    // the cards and the action row cleanly.
    create_dialog(app, ui.ctx(), &save_dir, &profiles, &p);
    rename_dialog(app, ui.ctx(), &save_dir, &profiles, &p);
    delete_dialog(app, ui.ctx(), &save_dir, &p);
    manage_dialog(app, ui.ctx(), &save_dir, &ids, &p, running, &mut message);

    if let Some((text, tone)) = message {
        app.notify(text, tone);
    }
}

fn selected_item<'a>(items: &'a [CachedProfile], selected: Option<&str>) -> Option<&'a CachedProfile> {
    selected.and_then(|name| items.iter().find(|i| i.profile.name == name))
}

/// The empty state replaces the profile carousel rather than showing one
/// oversized creation card in the middle of it. It stays compact and points at
/// the two ways a first profile is made.
fn empty_profiles(ui: &mut egui::Ui, p: &crate::theme::Palette, save_dir: &Path, running: bool) -> Option<Action> {
    let current_files = crate::saves::summary(&crate::saves::account_dir(save_dir)).0;
    let mut action = None;
    widgets::section(ui, p, "Start here", |ui| {
        ui.horizontal_top(|ui| {
            let (icon, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
            ui.painter().rect_filled(icon, CornerRadius::same(10), theme::alpha(p.accent, 20));
            crate::pages::Page::Saves.paint_icon(ui.painter(), icon.shrink(11.0), p.accent_hi);
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("No save profiles yet").font(theme::display(18.0)).color(p.ink));
                ui.label(
                    RichText::new(
                        "A profile keeps a whole playthrough separate: its slots, backups and edits. Make one from a fresh save, or keep the save that is already loaded.",
                    )
                    .size(12.0)
                    .color(p.body),
                );
            });
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            if widgets::primary_button(ui, p, "Create a profile", !running).clicked() {
                action = Some(Action::Create);
            }
            if widgets::button(ui, p, "Keep current save", !running && current_files > 0)
                .on_hover_text(if current_files > 0 { "Copies the save the game currently reads into a new, independent profile" } else { "There is no current save to keep yet. Play once, or import a save below." })
                .clicked()
            {
                action = Some(Action::KeepCurrent);
            }
            if widgets::button(ui, p, "Saves folder", true).clicked() {
                let dir = crate::saves::account_dir(save_dir);
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
        });
        if running {
            ui.add_space(4.0);
            ui.label(RichText::new("Close the game before creating or loading a profile; it writes its save when it closes.").size(11.0).color(p.warn));
        }
        ui.add_space(5.0);
        ui.label(RichText::new("Importing from an emulator or a folder is available in the Save data section below.").size(10.5).color(p.muted));
    });
    action
}

fn load_profile(app: &mut App, save_dir: &Path, profile: &Profile, title: u64, active: Option<&str>, running: bool) -> (String, Tone) {
    if running {
        return ("Close the game before loading a profile: it writes its save when it closes.".into(), Tone::Warn);
    }
    if active == Some(profile.name.as_str()) {
        return (format!("{} is already the loaded profile", profile.name), Tone::Neutral);
    }
    match crate::saves::load_profile_verified(save_dir, profile, title, &stamp()) {
        Ok(imported) => {
            app.saves_ui.editor = None;
            app.saves_ui.active_profile = Some(profile.name.clone());
            app.saves_ui.profile_cache = None;
            let backup = imported.backup.map(|b| format!("; the previous save is in {}", b.display())).unwrap_or_default();
            (format!("Loaded {} ({} files{})", profile.name, imported.files, backup), Tone::Ok)
        }
        Err(e) => (format!("Could not load the profile: {e}"), Tone::Bad),
    }
}

fn current_save(ui: &mut egui::Ui, p: &crate::theme::Palette, save_dir: &Path, active: Option<&str>) {
    let account = crate::saves::account_dir(save_dir);
    let current = crate::saves::slots_with_sizes(save_dir);
    widgets::section(ui, p, "Current save", |ui| {
        ui.label(RichText::new("The game reads this folder").size(12.0).color(p.muted));
        let path_text = account.display().to_string();
        ui.add(egui::Label::new(RichText::new(tail_path(&path_text, 96)).font(theme::mono(11.5)).color(p.ink)).truncate())
            .on_hover_text(format!("{}\naccount/<the title's fixed user id>", account.display()));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let newest = current.iter().filter_map(|s| s.1).max();
            widgets::pill(ui, p, &format!("{} slots", current.len()), if current.is_empty() { Tone::Neutral } else { Tone::Ok });
            if !current.is_empty() {
                widgets::pill(ui, p, &megabytes(current.iter().map(|s| s.2).sum()), Tone::Neutral);
            }
            if let Some(t) = newest {
                widgets::pill(ui, p, &ago(Some(t)), Tone::Neutral);
            }
            if let Some(name) = active {
                widgets::pill(ui, p, &format!("loaded: {name}"), Tone::Accent);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::button(ui, p, "Open folder", true).clicked() {
                    let _ = std::fs::create_dir_all(&account);
                    let _ = std::process::Command::new("explorer").arg(&account).spawn();
                }
            });
        });
        if current.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new("No save here yet. Play once, or import one below.").size(12.5).color(p.warn));
        }
        ui.add_space(5.0);
        if active.is_none() {
            ui.label(
                RichText::new("The current save is not one of the profiles. Keep it as a profile to make it part of the carousel.")
                    .size(12.0)
                    .color(p.muted),
            );
        }
    });
}

fn carousel(app: &mut App, ui: &mut egui::Ui, p: &crate::theme::Palette, items: &[CachedProfile], active: Option<&str>, running: bool) -> Option<Action> {
    let selected = app.saves_ui.selected_profile.as_deref();
    let idx = selected_index(items, selected, app.saves_ui.create_selected);
    let n = items.len() + 1;
    let available = ui.available_width().max(320.0);
    let gap = ui.spacing().item_spacing.x.max(8.0);
    let center_w = (available * 0.42).clamp(270.0, 340.0);
    let mut side_w = ((available - center_w - 2.0 * gap) / 2.0).clamp(104.0, 178.0);
    // The creation card stays visible beside the carousel, not only after the
    // player has walked all the way to the end. Keep the row below the available
    // width by giving the side cards a little room up when it is needed.
    let create_w = (side_w * 0.62).clamp(92.0, 118.0);
    let row_width = center_w + 2.0 * side_w + 2.0 * create_w + 4.0 * gap;
    if row_width > available {
        side_w = (side_w - (row_width - available) * 0.5).max(72.0);
    }
    let total = if idx == items.len() {
        center_w + 2.0 * (side_w + gap)
    } else {
        center_w + 2.0 * side_w + 2.0 * create_w + 4.0 * gap
    };
    let pad = ((available - total) / 2.0).max(0.0);
    let center_h = 352.0;
    let side_h = 258.0;
    let mut action = None;

    let row = ui.horizontal(|ui| {
        // The spacer after the previous card mirrors the creation card: it
        // keeps the selected profile on the centre line even though the
        // creation card sits at the far right.
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(pad);
        let left = idx.checked_sub(1).and_then(|i| items.get(i));
        match left {
            Some(item) => {
                if draw_profile_card(ui, p, item, portrait_key(app, item), false, active == Some(item.profile.name.as_str()), side_w, side_h).clicked() {
                    action = Some(Action::Select(item.profile.name.clone()));
                }
            }
            None => {
                ui.allocate_exact_size(Vec2::new(side_w, side_h), Sense::hover());
            }
        }
        ui.add_space(gap);
        if idx == items.len() {
            if draw_create_card(ui, p, true, center_w, center_h).clicked() {
                action = Some(Action::Create);
            }
        } else if let Some(item) = items.get(idx) {
            ui.add_space(create_w);
            ui.add_space(gap);
            let active_here = active == Some(item.profile.name.as_str());
            if draw_profile_card(ui, p, item, portrait_key(app, item), true, active_here, center_w, center_h).clicked() {
                action = Some(Action::Select(item.profile.name.clone()));
            }
        }
        ui.add_space(gap);
        let right = if idx < items.len() { items.get(idx + 1) } else { None };
        match right {
            Some(item) => {
                if draw_profile_card(ui, p, item, portrait_key(app, item), false, active == Some(item.profile.name.as_str()), side_w, side_h).clicked() {
                    action = Some(Action::Select(item.profile.name.clone()));
                }
            }
            None => {
                ui.allocate_exact_size(Vec2::new(side_w, side_h), Sense::hover());
            }
        }
        if idx != items.len() {
            ui.add_space(gap);
            if draw_create_card(ui, p, false, create_w, side_h).clicked() {
                action = Some(Action::Create);
            }
        }
    });
    if action.is_none() && row.response.hovered() && n > 1 {
        let wheel = ui.input(|i| i.raw_scroll_delta.y);
        if wheel.abs() > 4.0 {
            action = Some(Action::Move(if wheel > 0.0 { -1 } else { 1 }));
        }
    }

    ui.add_space(4.0);
    ui.vertical_centered(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(360.0, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(50.0);
            if widgets::button(ui, p, "‹ Previous", n > 1).clicked() {
                action = Some(Action::Move(-1));
            }
            ui.add_space(4.0);
            ui.allocate_ui_with_layout(Vec2::new(86.0, 24.0), egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                let label = if items.is_empty() {
                    "No profiles yet".to_string()
                } else {
                    format!("{} OF {}", idx + 1, n)
                };
                ui.label(RichText::new(label).font(theme::mono(11.5)).color(p.muted));
            });
            ui.add_space(4.0);
            if widgets::button(ui, p, "Next ›", n > 1).clicked() {
                action = Some(Action::Move(1));
            }
        });
        ui.add_space(2.0);
        ui.label(
            RichText::new("Click a side card, scroll over the carousel, or use the arrows and left/right keys. Selecting a card never loads it.")
                .size(12.0)
                .color(p.muted),
        );
    });

    if app.saves_ui.create_selected || items.is_empty() {
        ui.add_space(theme::SPACE_S);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Create a separate playthrough").font(theme::semibold(14.0)).color(p.ink));
            if widgets::primary_button(ui, p, "Create New Profile", true).clicked() {
                action = Some(Action::Create);
            }
            if widgets::button(ui, p, "Keep current save as profile", true).clicked() {
                action = Some(Action::KeepCurrent);
            }
        });
        return action;
    }

    let Some(item) = selected_item(items, app.saves_ui.selected_profile.as_deref()) else {
        return action;
    };
    let active_here = active == Some(item.profile.name.as_str());
    let (slot_text, slot_tone) = slot_summary(&item.meta, crate::saves::is_empty_profile(&item.profile));
    ui.add_space(theme::SPACE_S);
    ui.horizontal_wrapped(|ui| {
        widgets::status_dot(ui, p, if active_here { Tone::Ok } else { Tone::Accent });
        ui.label(RichText::new(&item.profile.name).font(theme::display(19.0)).color(p.ink));
        if active_here {
            widgets::pill(ui, p, "Currently loaded", Tone::Ok);
        } else {
            widgets::pill(ui, p, "Selected, not loaded", Tone::Neutral);
        }
        widgets::pill(ui, p, &slot_text, slot_tone);
        let _ = (running, active_here);
    });
    ui.add_space(8.0);
    let can_load = !running && !active_here && (item.meta.valid_slots > 0 || crate::saves::is_empty_profile(&item.profile));
    ui.horizontal_wrapped(|ui| {
        let load = widgets::primary_button(ui, p, if active_here { "Loaded" } else { "Load Profile" }, can_load);
        let load = if running {
            load.on_hover_text("Close the game first")
        } else if active_here {
            load.on_hover_text("This is already the save the game reads")
        } else if !can_load {
            load.on_hover_text("This profile has no readable save. Use Manage Saves to inspect it.")
        } else {
            load
        };
        if load.clicked() {
            action = Some(Action::Load);
        }
        if widgets::button(ui, p, "Manage Saves", true).clicked() {
            action = Some(Action::Manage);
        }
        if widgets::button(ui, p, "Duplicate", true).clicked() {
            action = Some(Action::Duplicate);
        }
        if widgets::button(ui, p, "Rename", true).clicked() {
            action = Some(Action::Rename);
        }
        if widgets::button(ui, p, "Export Backup", item.meta.valid_slots > 0).clicked() {
            action = Some(Action::Export);
        }
        if widgets::button(ui, p, "Open Folder", true).clicked() {
            action = Some(Action::OpenFolder);
        }
        if widgets::button(ui, p, "Delete…", true)
            .on_hover_text("Moves the profile folder to Backups; the current save is not changed")
            .clicked()
        {
            action = Some(Action::Delete);
        }
    });
    if crate::platform::game_running() {
        ui.label(RichText::new("Loading and deleting are disabled while the game is running.").size(11.5).color(p.warn));
    }

    if ui.ctx().memory(|m| m.focused().is_none()) {
        if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            action = Some(Action::Move(-1));
        } else if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            action = Some(Action::Move(1));
        } else if ui.input(|i| i.key_pressed(egui::Key::Enter)) && app.saves_ui.create_selected {
            action = Some(Action::Create);
        }
    }
    action
}

fn portrait_key(app: &App, item: &CachedProfile) -> Option<&'static str> {
    crate::portraits::for_profile(
        app.config.profile_portraits.get(&item.profile.name).map(String::as_str),
        item.meta.leader.as_deref(),
        &item.profile.name,
    )
}

fn draw_profile_card(ui: &mut egui::Ui, p: &crate::theme::Palette, item: &CachedProfile, portrait: Option<&'static str>, selected: bool, active: bool, width: f32, height: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, theme::CARD, p.surface);
    painter.rect_stroke(
        rect,
        theme::CARD,
        Stroke::new(if selected { 1.6_f32 } else { 1.0_f32 }, if selected { p.accent } else { p.line }),
        StrokeKind::Inside,
    );
    if selected {
        painter.rect_stroke(rect.shrink(3.0), CornerRadius::same(theme::RADIUS_M - 3), Stroke::new(1.0_f32, theme::alpha(p.accent_hi, 55)), StrokeKind::Inside);
    }

    let art = Rect::from_min_max(rect.min + Vec2::splat(1.0), Pos2::new(rect.max.x - 1.0, rect.top() + height * if selected { 0.43 } else { 0.47 }));
    draw_art(ui, painter, art, p, item, portrait, selected);

    let active_badge = active;
    if active_badge {
        draw_tag(painter, Pos2::new(art.left() + 12.0, art.top() + 12.0), p, "Active", Tone::Ok);
    } else if crate::saves::is_empty_profile(&item.profile) {
        draw_tag(painter, Pos2::new(art.left() + 12.0, art.top() + 12.0), p, "Empty", Tone::Accent);
    } else if item.meta.valid_slots == 0 {
        draw_tag(painter, Pos2::new(art.left() + 12.0, art.top() + 12.0), p, "Check save", Tone::Warn);
    }

    let x = rect.left() + if selected { 18.0 } else { 12.0 };
    let text_w = (rect.width() - 2.0 * (x - rect.left())).max(40.0);
    let name_y = art.bottom() + if selected { 17.0 } else { 12.0 };
    let name = painter.layout(fit(&item.profile.name, if selected { 34 } else { 18 }), theme::display(if selected { 21.0 } else { 14.0 }), p.ink, text_w);
    painter.galley(Pos2::new(x, name_y), name.clone(), p.ink);
    if selected {
        let stats = [
            ("Gold", item.meta.gold.map(|v| count(v as usize)).unwrap_or_else(|| "—".into())),
            ("Renown", item.meta.renown.map(|v| count(v as usize)).unwrap_or_else(|| "—".into())),
            ("Save slots", if item.meta.valid_slots == item.meta.slots.len() {
                item.meta.slots.len().to_string()
            } else {
                format!("{} / {}", item.meta.valid_slots, item.meta.slots.len())
            }),
            ("Units", if item.meta.units == 0 { "—".into() } else { item.meta.units.to_string() }),
            ("Modified", if item.profile.modified.is_some() { ago(item.profile.modified) } else { "—".into() }),
        ];
        let start = name_y + name.size().y + 7.0;
        for (i, (label, value)) in stats.iter().enumerate() {
            let y = start + i as f32 * 20.0;
            painter.text(Pos2::new(x, y), Align2::LEFT_TOP, *label, FontId::proportional(11.5), p.muted);
            let g = painter.layout_no_wrap(value.clone(), theme::semibold(12.0), p.body);
            painter.galley(Pos2::new(rect.right() - 18.0 - g.size().x, y - 1.0), g, p.body);
        }
        painter.text(Pos2::new(x, rect.bottom() - 18.0), Align2::LEFT_CENTER, "VIEW ONLY · LOAD FROM ACTIONS", theme::semibold(8.5), theme::alpha(p.muted, 190));
    } else {
        let (slot_text, tone) = slot_summary(&item.meta, crate::saves::is_empty_profile(&item.profile));
        painter.text(Pos2::new(x, rect.bottom() - 16.0), Align2::LEFT_CENTER, slot_text, theme::semibold(9.5), tone.colour(p));
        painter.text(Pos2::new(rect.right() - 12.0, rect.bottom() - 16.0), Align2::RIGHT_CENTER, "VIEW", theme::semibold(9.0), p.muted);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn draw_create_card(ui: &mut egui::Ui, p: &crate::theme::Palette, selected: bool, width: f32, height: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, theme::CARD, p.panel_hi);
    painter.rect_stroke(
        rect,
        theme::CARD,
        Stroke::new(if selected { 1.6_f32 } else { 1.0_f32 }, if selected { p.accent } else { p.line_strong }),
        StrokeKind::Inside,
    );
    widgets::gradient_fill(
        painter,
        rect.shrink(1.0),
        theme::RADIUS_M as f32 - 1.0,
        theme::alpha(p.accent, if selected { 42 } else { 16 }),
        Color32::TRANSPARENT,
    );
    let c = Pos2::new(rect.center().x, rect.top() + height * 0.32);
    painter.circle_filled(c, if selected { 31.0 } else { 24.0 }, theme::alpha(p.accent, 35));
    painter.circle_stroke(c, if selected { 31.0 } else { 24.0 }, Stroke::new(1.2_f32, theme::alpha(p.accent_hi, 120)));
    let arm = if selected { 11.0 } else { 9.0 };
    painter.line_segment([c - Vec2::new(arm, 0.0), c + Vec2::new(arm, 0.0)], Stroke::new(2.0_f32, p.accent_hi));
    painter.line_segment([c - Vec2::new(0.0, arm), c + Vec2::new(0.0, arm)], Stroke::new(2.0_f32, p.accent_hi));
    let title = painter.layout("Create New Profile".into(), theme::semibold(if selected { 17.0 } else { 12.5 }), p.ink, (width - 24.0).max(40.0));
    painter.galley(Pos2::new(rect.center().x - title.size().x / 2.0, c.y + 46.0), title, p.ink);
    if selected {
        let body = painter.layout("Start a separate playthrough with its own save slots.".into(), FontId::proportional(12.0), p.body, (width - 48.0).max(40.0));
        painter.galley(Pos2::new(rect.center().x - body.size().x / 2.0, c.y + 75.0), body, p.body);
        painter.text(Pos2::new(rect.center().x, rect.bottom() - 24.0), Align2::CENTER_CENTER, "OPEN CREATION OPTIONS", theme::semibold(9.0), p.accent_hi);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn draw_art(ui: &egui::Ui, painter: &egui::Painter, art: Rect, p: &crate::theme::Palette, item: &CachedProfile, portrait: Option<&str>, selected: bool) {
    let clipped = painter.with_clip_rect(art);
    widgets::gradient_fill(&clipped, art, theme::RADIUS_M as f32 - 1.0, theme::alpha(p.accent, if selected { 58 } else { 30 }), theme::alpha(p.bg_deep, 20));
    if let Some(key) = portrait {
        if crate::portraits::paint(&clipped, art, p, key) {
            clipped.line_segment(
                [Pos2::new(art.left() + 18.0, art.bottom() - 26.0), Pos2::new(art.right() - 18.0, art.bottom() - 26.0)],
                Stroke::new(1.0_f32, theme::alpha(p.accent_hi, 40)),
            );
            return;
        }
    }
    let c = Pos2::new(art.center().x, art.top() + art.height() * 0.56);
    let scale = if selected { 1.0 } else { 0.82 };
    // A deliberately generic portrait silhouette. It is used when a character
    // asset has not been matched, and never pretends to be a specific unit.
    clipped.circle_filled(Pos2::new(c.x, c.y - 24.0 * scale), 20.0 * scale, theme::alpha(p.ink, 26));
    let shoulders = vec![
        Pos2::new(c.x - 46.0 * scale, art.bottom() - 9.0),
        Pos2::new(c.x - 31.0 * scale, c.y + 7.0 * scale),
        Pos2::new(c.x + 31.0 * scale, c.y + 7.0 * scale),
        Pos2::new(c.x + 46.0 * scale, art.bottom() - 9.0),
    ];
    clipped.add(egui::Shape::convex_polygon(shoulders, theme::alpha(p.ink, 22), Stroke::NONE));
    let initial = item.profile.name.chars().next().unwrap_or('+').to_uppercase().to_string();
    clipped.text(Pos2::new(art.right() - 16.0, art.top() + 20.0), Align2::RIGHT_TOP, initial, theme::display(if selected { 44.0 } else { 30.0 }), theme::alpha(p.accent_hi, if selected { 60 } else { 36 }));
    clipped.line_segment(
        [Pos2::new(art.left() + 18.0, art.bottom() - 26.0), Pos2::new(art.right() - 18.0, art.bottom() - 26.0)],
        Stroke::new(1.0_f32, theme::alpha(p.accent_hi, 36)),
    );
    let _ = ui;
}

fn draw_tag(painter: &egui::Painter, at: Pos2, p: &crate::theme::Palette, text: &str, tone: Tone) {
    let font = theme::semibold(9.5);
    let g = painter.layout_no_wrap(text.into(), font, tone.colour(p));
    let rect = Rect::from_min_size(at, Vec2::new(g.size().x + 14.0, 21.0));
    painter.rect_filled(rect, CornerRadius::same(255), theme::alpha(tone.colour(p), 34));
    painter.galley(Pos2::new(rect.left() + 7.0, rect.center().y - g.size().y / 2.0), g, tone.colour(p));
}

fn current_slots(app: &mut App, ui: &mut egui::Ui, p: &crate::theme::Palette, save_dir: &Path, message: &mut Option<(String, Tone)>, running: bool) {
    widgets::section(ui, p, "Slots in the current save", |ui| {
        let account = crate::saves::account_dir(save_dir);
        let slots = crate::saves::slots_with_sizes(save_dir);
        let account_text = format!("The save the game reads is {}:", account.display());
        ui.add(egui::Label::new(RichText::new(account_text.clone()).size(12.5).color(p.muted)).truncate()).on_hover_text(account_text);
        ui.label(
            RichText::new(format!(
                "{} slot{}, {} in total.",
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
                if widgets::button(ui, p, if open { "Editing" } else { "Edit" }, !open).clicked() {
                    if let Err(e) = open_slot(app, path) {
                        *message = Some((format!("{name}: {e}"), Tone::Bad));
                    }
                }
                if widgets::button(ui, p, "Move to backups", !running)
                    .on_hover_text("The game offers a new game in this slot again; the file is kept in the backups folder")
                    .clicked()
                {
                    *message = Some(match crate::saves::archive_slot(save_dir, path, &stamp()) {
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
}

fn create_dialog(app: &mut App, ctx: &egui::Context, save_dir: &Path, profiles: &[Profile], p: &crate::theme::Palette) {
    let Some(mut d) = app.saves_ui.dialog.take() else { return };
    let mut close = false;
    let submitted = std::cell::Cell::new(None);
    let ready = std::cell::Cell::new(false);
    let submit_mode = std::cell::Cell::new(CreateMode::Empty);
    let mut choose_import = false;
    let shown = widgets::modal(ctx, p, "create-profile", "Create profile", "An independent copy of the whole save folder.", 540.0, |ui| {
            ui.radio_value(&mut d.mode, CreateMode::Empty, "Empty profile");
            ui.label(RichText::new("Starts with no save. Loading it gives the game a fresh save slot to create on its next play.").size(11.5).color(p.muted));
            ui.add_space(4.0);
            ui.radio_value(&mut d.mode, CreateMode::Duplicate, "Duplicate an existing profile");
            ui.add_space(4.0);
            ui.radio_value(&mut d.mode, CreateMode::Import, "Import a profile backup");
            ui.add_space(4.0);
            ui.radio_value(&mut d.mode, CreateMode::Current, "Save the current save as a profile");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Profile name").font(theme::semibold(13.0)).color(p.ink));
                ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("Run A, before the choice").desired_width(260.0));
            });
            if !d.name.trim().is_empty() {
                if let Err(e) = crate::saves::checked_profile_name(&d.name) {
                    ui.label(RichText::new(e).size(11.5).color(p.bad));
                } else if profiles.iter().any(|profile| profile.name == d.name.trim()) {
                    ui.label(RichText::new("That profile name is already in the carousel.").size(11.5).color(p.warn));
                }
            }
            if d.mode == CreateMode::Duplicate {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Source").color(p.muted));
                    egui::ComboBox::from_id_salt("duplicate-source")
                        .selected_text(d.source.clone().unwrap_or_else(|| "choose a profile".into()))
                        .width(260.0)
                        .show_ui(ui, |ui| {
                            for profile in profiles {
                                ui.selectable_value(&mut d.source, Some(profile.name.clone()), &profile.name);
                            }
                        });
                });
            }
            if d.mode == CreateMode::Import {
                ui.horizontal(|ui| {
                    if widgets::button(ui, p, "Choose backup folder", true).clicked() {
                        choose_import = true;
                    }
                    ui.add(egui::Label::new(RichText::new(d.import_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "No folder chosen".into())).size(12.0).color(p.muted)).truncate());
                });
                ui.label(RichText::new("The folder is validated before it is added. Its source is never changed.").size(11.0).color(p.muted));
            }
            if let Some(error) = &d.error {
                ui.add_space(6.0);
                widgets::error_note(ui, p, error);
            }
            let name = crate::saves::checked_profile_name(&d.name);
            let duplicate = name.as_ref().ok().is_some_and(|n| profiles.iter().any(|x| x.name == *n));
            let can_create = match d.mode {
                    CreateMode::Empty => name.is_ok() && !duplicate,
                    CreateMode::Duplicate => name.is_ok() && !duplicate && d.source.as_ref().is_some_and(|s| profiles.iter().any(|x| x.name == *s)),
                    CreateMode::Import => name.is_ok() && !duplicate && d.import_dir.is_some(),
                    CreateMode::Current => name.is_ok() && !duplicate && crate::saves::summary(&crate::saves::account_dir(save_dir)).0 > 0,
            };
            ready.set(can_create);
            submit_mode.set(d.mode);
        }, |ui| {
            ui.horizontal(|ui| {
                if widgets::primary_button(ui, p, "Create", ready.get()).clicked() {
                    submitted.set(Some(submit_mode.get()));
                }
                if widgets::button(ui, p, "Cancel", true).clicked() {
                    close = true;
                }
            });
        });
    if shown.close {
        close = true;
    }
    if choose_import {
        if let Some(dir) = rfd::FileDialog::new().set_title("Choose a profile backup folder").pick_folder() {
            d.import_dir = Some(dir);
            d.error = None;
        }
    }
    if let Some(mode) = submitted.get() {
        let name = match crate::saves::checked_profile_name(&d.name) {
            Ok(n) => n,
            Err(e) => {
                d.error = Some(e);
                app.saves_ui.dialog = Some(d);
                return;
            }
        };
        if profiles.iter().any(|p| p.name == name) {
            d.error = Some(format!("there is already a profile called {name}"));
            app.saves_ui.dialog = Some(d);
            return;
        }
        let result: Result<Profile, String> = match mode {
            CreateMode::Empty => crate::saves::create_empty_profile(save_dir, &name),
            CreateMode::Duplicate => match d.source.as_ref().and_then(|s| profiles.iter().find(|p| p.name == *s)) {
                Some(source) => crate::saves::duplicate_profile(save_dir, source, &name),
                None => Err("choose a profile to duplicate".into()),
            },
            CreateMode::Import => match d.import_dir.as_ref() {
                Some(source) => crate::saves::import_profile(save_dir, &name, source),
                None => Err("choose a backup folder to import".into()),
            },
            CreateMode::Current => {
                if crate::saves::summary(&crate::saves::account_dir(save_dir)).0 == 0 {
                    Err("there is no current save to keep yet".into())
                } else {
                    match crate::saves::store_profile(save_dir, &name, &stamp()) {
                        Ok(_) => crate::saves::profiles(save_dir).into_iter().find(|p| p.name == name).ok_or_else(|| "the profile was written but could not be listed".into()),
                        Err(e) => Err(e),
                    }
                }
            }
        };
        match result {
            Ok(profile) => {
                if d.mode == CreateMode::Duplicate {
                    if let Some(names) = d.source.as_ref().and_then(|source| app.config.slot_names.get(source)).cloned() {
                        app.config.slot_names.insert(profile.name.clone(), names);
                    }
                    if let Some(portrait) = d.source.as_ref().and_then(|source| app.config.profile_portraits.get(source)).cloned() {
                        app.config.profile_portraits.insert(profile.name.clone(), portrait);
                    }
                }
                app.saves_ui.selected_profile = Some(profile.name.clone());
                app.saves_ui.create_selected = false;
                app.saves_ui.profile_cache = None;
                app.saves_ui.dialog = None;
                app.notify(format!("Created profile {}", profile.name), Tone::Ok);
            }
            Err(e) => {
                d.error = Some(e);
                app.saves_ui.dialog = Some(d);
            }
        }
    } else if close {
        app.saves_ui.dialog = None;
    } else {
        app.saves_ui.dialog = Some(d);
    }
}

fn rename_dialog(app: &mut App, ctx: &egui::Context, save_dir: &Path, profiles: &[Profile], p: &crate::theme::Palette) {
    let Some(mut d) = app.saves_ui.rename.take() else { return };
    let mut close = false;
    let mut submit = false;
    let valid = std::cell::Cell::new(false);
    let subtitle = format!("Rename {} without touching its save files.", d.target);
    let shown = widgets::modal(ctx, p, "rename-profile", "Rename profile", &subtitle, 460.0, |ui| {
            ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("New name").desired_width(360.0));
            if let Some(e) = &d.error {
                ui.add_space(6.0);
                widgets::error_note(ui, p, e);
            }
            valid.set(!d.name.trim().is_empty() && d.name.trim() != d.target);
        }, |ui| {
            ui.horizontal(|ui| {
                if widgets::primary_button(ui, p, "Rename", valid.get()).clicked() {
                    submit = true;
                }
                if widgets::button(ui, p, "Cancel", true).clicked() {
                    close = true;
                }
            });
        });
    if shown.close {
        close = true;
    }
    if submit {
        let name = match crate::saves::checked_profile_name(&d.name) {
            Ok(n) => n,
            Err(e) => {
                d.error = Some(e);
                app.saves_ui.rename = Some(d);
                return;
            }
        };
        if profiles.iter().any(|p| p.name != d.target && p.name == name) {
            d.error = Some(format!("there is already a profile called {name}"));
            app.saves_ui.rename = Some(d);
            return;
        }
        let Some(old) = profiles.iter().find(|p| p.name == d.target) else {
            app.saves_ui.rename = None;
            return;
        };
        match crate::saves::rename_profile(save_dir, old, &name) {
            Ok(new) => {
                if let Some(names) = app.config.slot_names.remove(old.name.as_str()) {
                    app.config.slot_names.insert(new.name.clone(), names);
                }
                if let Some(portrait) = app.config.profile_portraits.remove(old.name.as_str()) {
                    app.config.profile_portraits.insert(new.name.clone(), portrait);
                }
                if app.saves_ui.active_profile.as_deref() == Some(old.name.as_str()) {
                    app.saves_ui.active_profile = Some(new.name.clone());
                }
                if app.saves_ui.selected_profile.as_deref() == Some(old.name.as_str()) {
                    app.saves_ui.selected_profile = Some(new.name.clone());
                }
                app.saves_ui.profile_cache = None;
                app.saves_ui.rename = None;
                app.notify(format!("Renamed {} to {}", old.name, new.name), Tone::Ok);
            }
            Err(e) => {
                d.error = Some(e);
                app.saves_ui.rename = Some(d);
            }
        }
    } else if close {
        app.saves_ui.rename = None;
    } else {
        app.saves_ui.rename = Some(d);
    }
}

fn delete_dialog(app: &mut App, ctx: &egui::Context, save_dir: &Path, p: &crate::theme::Palette) {
    let Some(target) = app.saves_ui.delete.clone() else { return };
    let mut close = false;
    let mut submit = false;
    let shown = widgets::modal(ctx, p, "delete-profile", "Move profile to Backups?", &format!("Remove {target} from the carousel."), 480.0, |ui| {
            ui.label(RichText::new("The profile folder is moved to Backups, not deleted. The current save is not changed, so this cannot remove the save the game is reading.").size(12.0).color(p.body));
        }, |ui| {
            ui.horizontal(|ui| {
                if widgets::primary_button(ui, p, "Move to Backups", true).clicked() {
                    submit = true;
                }
                if widgets::button(ui, p, "Cancel", true).clicked() {
                    close = true;
                }
            });
        });
    if shown.close {
        close = true;
    }
    if submit {
        let Some(profile) = crate::saves::profiles(save_dir).into_iter().find(|p| p.name == target) else {
            app.saves_ui.delete = None;
            return;
        };
        match crate::saves::remove_profile(save_dir, &profile, &stamp()) {
            Ok(to) => {
                app.config.slot_names.remove(target.as_str());
                app.config.profile_portraits.remove(target.as_str());
                if app.saves_ui.active_profile.as_deref() == Some(target.as_str()) {
                    app.saves_ui.active_profile = None;
                }
                app.saves_ui.selected_profile = None;
                app.saves_ui.profile_cache = None;
                app.saves_ui.delete = None;
                app.notify(format!("Moved profile {target} to {}", to.display()), Tone::Ok);
            }
            Err(e) => {
                app.saves_ui.delete = None;
                app.notify(format!("Could not remove the profile: {e}"), Tone::Bad);
            }
        }
    } else if close {
        app.saves_ui.delete = None;
    }
}

fn manage_dialog(app: &mut App, ctx: &egui::Context, save_dir: &Path, ids: &Ids, p: &crate::theme::Palette, running: bool, message: &mut Option<(String, Tone)>) {
    let Some(mut m) = app.saves_ui.manage.take() else { return };
    let Some(profile) = crate::saves::profiles(save_dir).into_iter().find(|p| p.name == m.profile) else {
        app.notify("That profile is no longer there", Tone::Warn);
        return;
    };
    let title_now = app
        .game
        .title_id
        .or_else(|| app.game.nacp.as_deref().and_then(crate::saves::nacp_save_owner))
        .unwrap_or(crate::saves::FALLBACK_TITLE_ID);
    let active_now = app.saves_ui.active_profile.clone();
    if m.slots.is_none() {
        m.slots = Some(crate::saves::profile_slots(&profile, ids));
    }
    let slots = m.slots.clone().unwrap_or_default();
    let newest = slots.iter().find(|slot| slot.valid).or_else(|| slots.first());
    let meta = saves::ProfileMeta {
        gold: newest.and_then(|slot| slot.gold),
        renown: newest.and_then(|slot| slot.renown),
        units: newest.map_or(0, |slot| slot.units),
        leader: newest.and_then(|slot| slot.leader.clone()),
        valid_slots: slots.iter().filter(|slot| slot.valid).count(),
        slots: slots.clone(),
    };
    let mut close = false;
    let mut open_editor: Option<PathBuf> = None;
    let mut backup_slot: Option<PathBuf> = None;
    let mut restore_slot: Option<PathBuf> = None;
    let mut export_slot: Option<PathBuf> = None;
    let mut archive_slot: Option<PathBuf> = None;
    let mut dismissed = false;
    let explicit_portrait = app.config.profile_portraits.get(&profile.name).cloned();
    let mut portrait_choice: Option<Option<String>> = None;
    let subtitle = format!("{} · {} files · portrait follows the newest save unless you choose one.", profile.path.display(), profile.files);
    let shown = widgets::modal(ctx, p, "manage-saves", &profile.name, &subtitle, 780.0, |ui| {
            ui.horizontal(|ui| {
                if app.saves_ui.active_profile.as_deref() == Some(profile.name.as_str()) {
                    widgets::pill(ui, p, "Currently loaded", Tone::Ok);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, p, "Open folder", true).clicked() {
                        let _ = std::fs::create_dir_all(&profile.path);
                        let _ = std::process::Command::new("explorer").arg(&profile.path).spawn();
                    }
                });
            });
            ui.add_space(4.0);
            ui.label(RichText::new("Carousel portrait").font(theme::semibold(11.0)).color(p.muted));
            portrait_choice = crate::portraits::picker(ui, p, explicit_portrait.as_deref());
            ui.add_space(4.0);
            let (summary, tone) = slot_summary(&meta, saves::is_empty_profile(&profile));
            widgets::status_line(ui, p, tone, &summary, "Actual files in this profile. Unknown files are kept, never tidied away.");
            if let Some((text, tone)) = &m.notice {
                ui.add_space(4.0);
                ui.label(RichText::new(text).size(12.0).color(tone.colour(p)));
            }
            ui.separator();
            if slots.is_empty() {
                ui.add_space(12.0);
                widgets::empty_note(ui, p, "This profile has no save files yet", "Load it as a fresh playthrough, then create a save in the game.");
                ui.add_space(6.0);
                if widgets::primary_button(ui, p, "Load as a fresh playthrough", !running && saves::is_empty_profile(&profile)).clicked() {
                    *message = Some(load_profile(app, save_dir, &profile, title_now, active_now.as_deref(), running));
                    close = true;
                }
            } else {
                for slot in &slots {
                        let display = m.names.get(&slot.relative).cloned().unwrap_or_else(|| slot.relative.clone());
                        // The read-only summary: what the slot itself says, with a gap wherever a field could not be read.
                        let s = &slot.summary;
                        let mut parts: Vec<String> = Vec::new();
                        if let Some(g) = slot.gold { parts.push(format!("Gold {}", count(g as usize))) }
                        if let Some(r) = slot.renown { parts.push(format!("Renown {}", count(r as usize))) }
                        if slot.units > 0 { parts.push(format!("{} units", slot.units)) }
                        if let Some(d) = s.difficulty { parts.push(format!("Difficulty {d}")) }
                        if let Some(t) = s.playthrough { parts.push(format!("Playthrough {t}")) }
                        if let Some(n) = s.flags { parts.push(format!("{} flags", count(n))) }
                        let meta = parts.join("  ");
                        ui.horizontal_wrapped(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(fit(&display, 48)).font(theme::semibold(13.0)).color(p.ink))
                                        .on_hover_text(if display == slot.relative { slot.relative.clone() } else { format!("File: {}", slot.relative) });
                                    widgets::pill(ui, p, slot.kind.label(), Tone::Neutral);
                                    if slot.valid {
                                        widgets::pill(ui, p, "Readable", Tone::Ok);
                                    } else {
                                        widgets::pill(ui, p, "Unreadable", Tone::Bad);
                                    }
                                });
                                ui.label(RichText::new(format!("{} · {} · {}", megabytes(slot.size), ago(slot.modified), meta.trim_end())).size(11.5).color(p.muted));
                                if !slot.problems.is_empty() {
                                    ui.label(RichText::new(slot.problems.join("; ")).size(11.0).color(p.warn));
                                }
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if widgets::button(ui, p, "Remove", true).clicked() {
                                    m.pending_delete = Some(slot.path.clone());
                                }
                                if widgets::button(ui, p, "Rename", true).clicked() {
                                    m.renaming = Some(slot.relative.clone());
                                    m.rename_text = m.names.get(&slot.relative).cloned().unwrap_or_else(|| slot.relative.clone());
                                }
                                if widgets::button(ui, p, "Export", true).clicked() {
                                    export_slot = Some(slot.path.clone());
                                }
                                if !saves::slot_backups(&slot.path).is_empty() && widgets::button(ui, p, "Restore", true).clicked() {
                                    restore_slot = Some(slot.path.clone());
                                }
                                if widgets::button(ui, p, "Backup", true).clicked() {
                                    backup_slot = Some(slot.path.clone());
                                }
                                if widgets::button(ui, p, "Inspect", slot.valid).clicked() {
                                    open_editor = Some(slot.path.clone());
                                }
                            });
                        });
                        if m.renaming.as_deref() == Some(slot.relative.as_str()) {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new("Display name").size(12.0).color(p.muted));
                                ui.add(egui::TextEdit::singleline(&mut m.rename_text).desired_width(260.0));
                                if widgets::primary_button(ui, p, "Save name", true).clicked() {
                                    let label = m.rename_text.trim().chars().take(48).collect::<String>();
                                    if label.is_empty() {
                                        m.names.remove(&slot.relative);
                                    } else {
                                        m.names.insert(slot.relative.clone(), label);
                                    }
                                    app.config.slot_names.insert(profile.name.clone(), m.names.clone());
                                    m.renaming = None;
                                }
                                if widgets::button(ui, p, "Cancel", true).clicked() {
                                    m.renaming = None;
                                }
                            });
                            ui.label(RichText::new("This label is not written into the game save; the file keeps its name.").size(11.0).color(p.muted));
                        }
                        if m.pending_delete.as_ref() == Some(&slot.path) {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new("Move this slot to Backups? The game will offer a new save there.").size(12.0).color(p.warn));
                                if widgets::primary_button(ui, p, "Confirm remove", true).clicked() {
                                    archive_slot = Some(slot.path.clone());
                                }
                                if widgets::button(ui, p, "Cancel", true).clicked() {
                                    m.pending_delete = None;
                                }
                            });
                        }
                    ui.separator();
                }
            }
        }, |_| {});
    if shown.close {
        close = true;
    }
    if let Some(choice) = portrait_choice {
        match choice {
            Some(key) => {
                app.config.profile_portraits.insert(profile.name.clone(), key);
            }
            None => {
                app.config.profile_portraits.remove(&profile.name);
            }
        }
    }
    if let Some(path) = open_editor {
        match open_slot(app, &path) {
            Ok(()) => {
                app.saves_ui.scroll_to_editor = true;
                app.saves_ui.manage = None;
                dismissed = true;
                app.notify("Opened the slot in the editor", Tone::Accent);
            }
            Err(e) => m.notice = Some((e, Tone::Bad)),
        }
    }
    if let Some(path) = backup_slot {
        match saves::backup_slot(save_dir, &profile, &path, &stamp()) {
            Ok(to) => m.notice = Some((format!("Backed up to {}", to.display()), Tone::Ok)),
            Err(e) => m.notice = Some((e, Tone::Bad)),
        }
    }
    if let Some(path) = restore_slot {
        match saves::restore_slot_backup(&path) {
            Ok((from, kept)) => {
                m.slots = None;
                let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
                m.notice = Some((format!("Restored {}; the copy it replaced is {}", name(&from), name(&kept)), Tone::Ok));
            }
            Err(e) => m.notice = Some((e, Tone::Bad)),
        }
    }
    if let Some(path) = export_slot {
        if let Some(dest) = rfd::FileDialog::new().set_title("Export save slot").set_file_name(path.file_name().unwrap_or_default().to_string_lossy()).save_file() {
            match std::fs::copy(&path, &dest) {
                Ok(_) => m.notice = Some((format!("Exported to {}", dest.display()), Tone::Ok)),
                Err(e) => m.notice = Some((format!("Could not export the slot: {e}"), Tone::Bad)),
            }
        }
    }
    if let Some(path) = archive_slot {
        match saves::archive_slot(save_dir, &path, &stamp()) {
            Ok(to) => {
                m.pending_delete = None;
                m.slots = None;
                app.saves_ui.profile_cache = None;
                m.notice = Some((format!("Moved slot to {}", to.display()), Tone::Ok));
            }
            Err(e) => m.notice = Some((e, Tone::Bad)),
        }
    }
    let dismissed = close || dismissed;
    app.config.slot_names.insert(profile.name.clone(), m.names.clone());
    if !dismissed {
        app.saves_ui.manage = Some(m);
    }
}

fn editor(app: &mut App, ui: &mut egui::Ui, p: &crate::theme::Palette, running: bool) -> Option<(String, Tone)> {
    let ids = app.saves_ui.ids.clone().unwrap_or_default();
    if app.saves_ui.editor.is_some() && app.saves_ui.scroll_to_editor {
        app.saves_ui.scroll_to_editor = false;
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
    let ed = app.saves_ui.editor.as_mut()?;
    let mut message = None;
    let mut close = false;
    let mut saved = false;
    let named = !ids.people.is_empty();
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
                        saved = true;
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
    if saved {
        app.saves_ui.profile_cache = None;
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
