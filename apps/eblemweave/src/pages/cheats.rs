//! Cheats: Atmosphere cheat codes to switch on and off, and Cheat Engine
//! tables with live, editable and freezable values.

use crate::cheats::{self, Code, Entry, Memory, Table};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct CodeFile {
    name: String,
    path: PathBuf,
    codes: Vec<Code>,
    error: Option<String>,
}

struct TableState {
    table: Table,
    path: PathBuf,
    symbols: HashMap<String, u64>,
    /// Script entries switched on, by id.
    active: HashSet<String>,
    edits: HashMap<String, String>,
    frozen: HashMap<String, (u64, Vec<u8>)>,
    notes: HashMap<String, String>,
    /// Pattern searches still running, by entry id.
    pending: HashMap<String, Arc<Mutex<Option<Vec<(String, Option<u64>)>>>>>,
}

#[derive(Default)]
pub struct Ui {
    loaded: bool,
    files: Vec<CodeFile>,
    enabled: HashMap<String, Vec<String>>,
    new_name: String,
    new_codes: String,
    tables: Vec<TableState>,
    runner: Option<cheats::Handle>,
    /// Unit records found in the game's memory, and the search under way.
    units: Vec<u64>,
    searching: Option<Arc<Mutex<Option<Vec<u64>>>>>,
    /// Per unit: infinite HP, quick level-up, max stats.
    unit_cheats: HashMap<u64, [bool; 3]>,
    /// The values those cheats hold, refreshed while the menu is open.
    unit_freezes: Vec<(u64, Vec<u8>)>,
}

fn load(ui_state: &mut Ui, dir: &std::path::Path) {
    ui_state.files.clear();
    ui_state.tables.clear();
    ui_state.enabled = cheats::load_enabled(dir);
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    entries.sort();
    for path in entries {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        match ext.as_str() {
            "txt" => {
                let (codes, error) = match cheats::parse_codes(&text) {
                    Ok(c) => (c, None),
                    Err(e) => (Vec::new(), Some(e)),
                };
                ui_state.files.push(CodeFile { name, path, codes, error });
            }
            "ct" => {
                if let Ok(table) = cheats::parse_table(&name, &text) {
                    ui_state.tables.push(TableState {
                        table,
                        path,
                        symbols: HashMap::new(),
                        active: HashSet::new(),
                        edits: HashMap::new(),
                        frozen: HashMap::new(),
                        notes: HashMap::new(),
                        pending: HashMap::new(),
                    });
                }
            }
            _ => {}
        }
    }
    ui_state.loaded = true;
}

fn copy_in(dir: &std::path::Path, from: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let to = dir.join(from.file_name().ok_or("no file name")?);
    std::fs::copy(from, &to).map_err(|e| format!("copying {}: {e}", from.display()))?;
    Ok(())
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Cheats", "Atmosphere cheat codes and Cheat Engine tables, applied to the game while it runs.");
    let Some(dir) = cheats::dir() else { return };
    let mut message: Option<(String, Tone)> = None;
    if !app.cheats_ui.loaded {
        load(&mut app.cheats_ui, &dir);
    }
    #[cfg(windows)]
    if app.cheats_ui.runner.is_none() {
        app.cheats_ui.runner = Some(cheats::start());
    }
    let game = cheats::game();
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));

    match &game {
        Some(g) => widgets::status_line(ui, &p, Tone::Ok, "Game running", &format!("Attached to {} (process {}). Switched-on codes are applied about 60 times a second.", cheats::GAME_EXE, g.pid)),
        None => widgets::status_line(ui, &p, Tone::Neutral, "Game not running", "Switch codes on here; they apply as soon as the game starts. Table values show while it runs."),
    }
    ui.add_space(theme::SPACE_S);

    // ------------------------------------------------------------ codes
    let mut enabled_changed = false;
    widgets::section(ui, &p, "Cheat codes (Atmosphere format)", |ui| {
        if app.cheats_ui.files.is_empty() {
            ui.label(RichText::new("No cheat files yet. Add codes below or import a .txt from a cheats folder.").color(p.muted));
        }
        for f in &app.cheats_ui.files {
            ui.label(RichText::new(&f.name).font(theme::semibold(14.0)).color(p.ink));
            if let Some(e) = &f.error {
                widgets::status_line(ui, &p, Tone::Bad, "Could not read it", e);
            }
            let on = app.cheats_ui.enabled.entry(f.name.clone()).or_default();
            for c in &f.codes {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    if c.master {
                        widgets::pill(ui, &p, "master", Tone::Accent);
                        ui.label(RichText::new(&c.name).color(p.body)).on_hover_text("Runs whenever any code of this file is on");
                        return;
                    }
                    let mut is_on = on.contains(&c.name);
                    if widgets::toggle(ui, &p, &mut is_on).changed() {
                        if is_on {
                            on.push(c.name.clone());
                        } else {
                            on.retain(|n| n != &c.name);
                        }
                        enabled_changed = true;
                    }
                    ui.label(RichText::new(&c.name).color(p.ink));
                    ui.label(RichText::new(format!("{} words", c.words.len())).size(12.0).color(p.muted));
                });
            }
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                if widgets::button(ui, &p, "Edit file", true).clicked() {
                    let _ = std::process::Command::new("notepad").arg(&f.path).spawn();
                }
            });
            ui.add_space(6.0);
        }
        ui.separator();
        ui.label(RichText::new("Add codes").font(theme::semibold(14.0)).color(p.ink));
        ui.label(
            RichText::new("Paste codes as they are shared for Atmosphere: a [Name] line, then the code's hex words. Several codes can go in one paste.")
                .size(12.5)
                .color(p.muted),
        );
        ui.horizontal(|ui| {
            ui.label("File name");
            ui.add(egui::TextEdit::singleline(&mut app.cheats_ui.new_name).hint_text("my-codes").desired_width(200.0));
        });
        ui.add(egui::TextEdit::multiline(&mut app.cheats_ui.new_codes).font(egui::TextStyle::Monospace).desired_rows(6).desired_width(f32::INFINITY).hint_text("[Max gold]\n04100000 01234567 0098967F"));
        ui.horizontal(|ui| {
            let parsed = cheats::parse_codes(&app.cheats_ui.new_codes);
            let ok = matches!(&parsed, Ok(c) if !c.is_empty());
            if widgets::button(ui, &p, "Add", ok).clicked() {
                let name = crate::mods::clean_name(if app.cheats_ui.new_name.trim().is_empty() { "codes" } else { app.cheats_ui.new_name.trim() });
                let path = dir.join(format!("{name}.txt"));
                let mut text = std::fs::read_to_string(&path).unwrap_or_default();
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(app.cheats_ui.new_codes.trim());
                text.push('\n');
                let _ = std::fs::create_dir_all(&dir);
                message = Some(match std::fs::write(&path, text) {
                    Ok(()) => (format!("Added to {}", path.display()), Tone::Ok),
                    Err(e) => (format!("Could not write {}: {e}", path.display()), Tone::Bad),
                });
                app.cheats_ui.new_codes.clear();
                app.cheats_ui.loaded = false;
            }
            if let Err(e) = &parsed {
                if !app.cheats_ui.new_codes.trim().is_empty() {
                    ui.label(RichText::new(e).color(p.bad));
                }
            }
            if widgets::button(ui, &p, "Import .txt…", true).clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter("Atmosphere cheats", &["txt"]).pick_file() {
                    message = Some(match copy_in(&dir, &f) {
                        Ok(()) => ("Imported the cheat file".into(), Tone::Ok),
                        Err(e) => (e, Tone::Bad),
                    });
                    app.cheats_ui.loaded = false;
                }
            }
            if widgets::button(ui, &p, "Open cheats folder", true).clicked() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
            if widgets::button(ui, &p, "Reload", true).clicked() {
                app.cheats_ui.loaded = false;
            }
        });
        ui.label(
            RichText::new("Codes that patch the game's instructions do nothing here (the code is compiled ahead of time); codes that change data, follow pointers or test buttons work. Button tests read the first controller.")
                .size(12.5)
                .color(p.muted),
        );
    });
    if enabled_changed {
        cheats::save_enabled(&dir, &app.cheats_ui.enabled);
    }

    // ------------------------------------------------------------ tables
    widgets::section(ui, &p, "Cheat Engine tables", |ui| {
        ui.horizontal(|ui| {
            if widgets::button(ui, &p, "Load table…", true).clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter("Cheat Engine table", &["CT", "ct"]).pick_file() {
                    message = Some(match std::fs::read_to_string(&f).map_err(|e| e.to_string()).and_then(|t| cheats::parse_table("", &t)).and_then(|_| copy_in(&dir, &f)) {
                        Ok(()) => ("Loaded the table".into(), Tone::Ok),
                        Err(e) => (e, Tone::Bad),
                    });
                    app.cheats_ui.loaded = false;
                }
            }
        });
        ui.label(
            RichText::new("Switch on a script entry to find its pattern in the game's memory; the values below it then show live. Type a value and press Enter to write it, or tick Freeze to hold it.")
                .size(12.5)
                .color(p.muted),
        );
        for (ti, t) in app.cheats_ui.tables.iter_mut().enumerate() {
            egui::CollapsingHeader::new(RichText::new(&t.table.name).font(theme::semibold(14.0)).color(p.ink)).id_salt(("table", ti)).default_open(true).show(ui, |ui| {
                let entries = t.table.entries.clone();
                for e in &entries {
                    entry(ui, &p, t, e, None, game.as_ref(), &mut message);
                }
                if widgets::button(ui, &p, "Remove table", true).clicked() {
                    let _ = std::fs::rename(&t.path, t.path.with_extension("CT.removed"));
                    app.cheats_ui.loaded = false;
                }
            });
        }
    });

    // Hand the switched-on codes and frozen values to the cheat thread.
    if let Some(r) = &app.cheats_ui.runner {
        let mut codes = Vec::new();
        for f in &app.cheats_ui.files {
            let on = app.cheats_ui.enabled.get(&f.name).cloned().unwrap_or_default();
            if on.is_empty() {
                continue;
            }
            codes.extend(f.codes.iter().filter(|c| c.master).cloned());
            codes.extend(f.codes.iter().filter(|c| !c.master && on.contains(&c.name)).cloned());
        }
        let mut freezes: Vec<(u64, Vec<u8>)> = app.cheats_ui.tables.iter().flat_map(|t| t.frozen.values().cloned()).collect();
        freezes.extend(app.cheats_ui.unit_freezes.iter().cloned());
        let mut s = r.lock().unwrap();
        s.codes = codes;
        s.freezes = freezes;
    }
    if let Some((text, tone)) = message {
        app.notify(text, tone);
    }
}

type Game = crate::cheats::process::Process;

fn entry(ui: &mut egui::Ui, p: &crate::theme::Palette, t: &mut TableState, e: &Entry, parent: Option<u64>, game: Option<&Arc<Game>>, message: &mut Option<(String, Tone)>) {
    let mut addr = None;
    let script = !e.scans.is_empty() || e.unsupported_script;
    let done = t.pending.get(&e.id).and_then(|s| s.lock().unwrap().take());
    if let Some(found) = done {
        t.pending.remove(&e.id);
        if !found.is_empty() && found.iter().all(|(_, a)| a.is_some()) {
            for (sym, a) in found {
                t.symbols.insert(sym, a.unwrap());
            }
            t.active.insert(e.id.clone());
            t.notes.insert(e.id.clone(), "found".into());
        } else {
            t.notes.insert(e.id.clone(), "pattern not found (the data may not be loaded yet: try again in game)".into());
        }
    }
    if script {
        ui.horizontal(|ui| {
            let mut on = t.active.contains(&e.id) || t.pending.contains_key(&e.id);
            let r = widgets::toggle(ui, p, &mut on);
            ui.label(RichText::new(&e.description).font(theme::semibold(13.5)).color(p.ink));
            if e.unsupported_script {
                ui.label(RichText::new("uses script commands EmblemWeave cannot run").size(12.0).color(p.warn));
            }
            if let Some(n) = t.notes.get(&e.id) {
                ui.label(RichText::new(n).size(12.0).color(p.muted));
            }
            if r.changed() {
                if on {
                    match game {
                        None => *message = Some(("Start the game first: the pattern is searched in its memory".into(), Tone::Warn)),
                        Some(g) => {
                            let (g, scans) = (g.clone(), e.scans.clone());
                            let slot: Arc<Mutex<Option<Vec<(String, Option<u64>)>>>> = Default::default();
                            let out = slot.clone();
                            std::thread::spawn(move || {
                                let r = scans
                                    .iter()
                                    .map(|s| (s.symbol.to_ascii_lowercase(), cheats::aob_scan_parallel(g.as_ref(), cheats::GUEST_RANGE.0, cheats::GUEST_RANGE.1, &s.pattern)))
                                    .collect();
                                *out.lock().unwrap() = Some(r);
                            });
                            t.pending.insert(e.id.clone(), slot);
                            t.notes.insert(e.id.clone(), "searching the game's memory…".into());
                        }
                    }
                } else {
                    t.active.remove(&e.id);
                    t.pending.remove(&e.id);
                    for s in &e.scans {
                        t.symbols.remove(&s.symbol.to_ascii_lowercase());
                    }
                    t.notes.remove(&e.id);
                    let ids: Vec<String> = all_ids(e);
                    t.frozen.retain(|k, _| !ids.contains(k));
                }
            }
        });
    }
    if let Some(var) = e.var {
        addr = cheats::resolve(&e.address, parent, &t.symbols);
        ui.horizontal(|ui| {
            ui.add_space(if parent.is_some() { 24.0 } else { 0.0 });
            ui.label(RichText::new(&e.description).color(p.body));
            match (addr, game) {
                (Some(a), Some(g)) => {
                    let mut buf = vec![0u8; var.size()];
                    let value = if g.read(a, &mut buf) { var.format(&buf, e.signed) } else { "??".into() };
                    ui.label(RichText::new(&value).font(theme::mono(13.0)).color(p.ink)).on_hover_text(format!("{a:#x}"));
                    let text = t.edits.entry(e.id.clone()).or_default();
                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(90.0).hint_text("new value"));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        match var.encode(text) {
                            Some(b) => {
                                if !g.write(a, &b) {
                                    *message = Some(("Could not write to the game's memory".into(), Tone::Bad));
                                }
                                if let Some(f) = t.frozen.get_mut(&e.id) {
                                    f.1 = b;
                                }
                            }
                            None => *message = Some((format!("{text:?} is not a valid value"), Tone::Bad)),
                        }
                        text.clear();
                    }
                    let mut frozen = t.frozen.contains_key(&e.id);
                    if ui.checkbox(&mut frozen, "Freeze").changed() {
                        if frozen {
                            t.frozen.insert(e.id.clone(), (a, buf));
                        } else {
                            t.frozen.remove(&e.id);
                        }
                    }
                }
                (None, _) => {
                    ui.label(RichText::new("switch on the entry above").size(12.0).color(p.muted));
                }
                (Some(_), None) => {
                    ui.label(RichText::new("game not running").size(12.0).color(p.muted));
                }
            }
        });
    } else if !script && !e.group && !e.address.is_empty() {
        addr = cheats::resolve(&e.address, parent, &t.symbols);
    }
    if e.children.is_empty() {
        return;
    }
    let child_parent = addr.or(parent);
    if script || e.group {
        egui::CollapsingHeader::new(if script { "Values".to_string() } else { e.description.clone() }).id_salt(("ce", &t.table.name, &e.id)).default_open(script && t.active.contains(&e.id)).show(ui, |ui| {
            for c in &e.children {
                entry(ui, p, t, c, child_parent, game, message);
            }
        });
    } else {
        ui.indent(("ce", &e.id), |ui| {
            for c in &e.children {
                entry(ui, p, t, c, child_parent, game, message);
            }
        });
    }
}

fn all_ids(e: &Entry) -> Vec<String> {
    let mut v = vec![e.id.clone()];
    for c in &e.children {
        v.extend(all_ids(c));
    }
    v
}

const QUICK: [(&str, &str); 3] = [
    ("Infinite HP", "HP is held at max HP"),
    ("Quick level-up", "experience is held at 99, so the next experience gained levels the unit up"),
    ("Max stats", "max HP and every stat set to 99 (the game may cap them by class)"),
];

/// Quick cheats: per-unit switches for the units found in the game's memory.
pub fn quick(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    if app.cheats_ui.runner.is_none() {
        app.cheats_ui.runner = Some(cheats::start());
    }
    let game = cheats::game();
    widgets::section(ui, &p, "Units on the map", |ui| {
        ui.label(
            RichText::new("Finds the units in the game's memory by the layout their records share (experimental: results may include units not on the map). Search once a map is loaded, and again on the next map.")
                .size(12.5)
                .color(p.muted),
        );
        ui.horizontal(|ui| {
            let busy = app.cheats_ui.searching.is_some();
            if widgets::button(ui, &p, if busy { "Searching…" } else { "Find units" }, game.is_some() && !busy).clicked() {
                let g = game.clone().unwrap();
                let slot: Arc<Mutex<Option<Vec<u64>>>> = Default::default();
                let out = slot.clone();
                std::thread::spawn(move || {
                    let v = cheats::scan_units(g.as_ref());
                    *out.lock().unwrap() = Some(v);
                });
                app.cheats_ui.searching = Some(slot);
            }
            if game.is_none() {
                ui.label(RichText::new("Start the game first.").color(p.muted));
            }
        });
        if let Some(done) = app.cheats_ui.searching.as_ref().and_then(|s| s.lock().unwrap().take()) {
            app.cheats_ui.searching = None;
            app.cheats_ui.units = done.into_iter().take(40).collect();
            app.cheats_ui.unit_cheats.retain(|k, _| app.cheats_ui.units.contains(k));
        }
        let Some(g) = game.as_deref() else { return };
        let mut freezes = Vec::new();
        egui::Grid::new("quick-units").num_columns(6).spacing([12.0, 6.0]).striped(true).show(ui, |ui| {
            if !app.cheats_ui.units.is_empty() {
                for h in ["Slot", "Lv", "HP", QUICK[0].0, QUICK[1].0, QUICK[2].0] {
                    ui.label(RichText::new(h).font(theme::semibold(12.0)).color(p.muted));
                }
                ui.end_row();
            }
            for (n, &a) in app.cheats_ui.units.iter().enumerate() {
                let mut b = [0u8; cheats::unit::LEN];
                if !g.read(a, &mut b) || !cheats::looks_like_unit(&b) {
                    continue;
                }
                let max_hp = u16::from_le_bytes([b[8], b[9]]);
                let hp = u16::from_le_bytes([b[0x20], b[0x21]]);
                ui.label(RichText::new(format!("{}", n + 1)).color(p.ink)).on_hover_text(format!("record at {a:#x}"));
                ui.label(format!("{}", b[0x26]));
                ui.label(format!("{hp} / {max_hp}"));
                let on = app.cheats_ui.unit_cheats.entry(a).or_default();
                for (i, (_, help)) in QUICK.iter().enumerate() {
                    widgets::toggle(ui, &p, &mut on[i]).on_hover_text(*help);
                }
                ui.end_row();
                if on[2] {
                    let mut stats = Vec::new();
                    for _ in 0..9 {
                        stats.extend(99u16.to_le_bytes());
                    }
                    freezes.push((a + cheats::unit::STATS, stats));
                }
                let target_hp = if on[2] { 99 } else { max_hp };
                if on[0] {
                    freezes.push((a + cheats::unit::HP, target_hp.to_le_bytes().to_vec()));
                }
                if on[1] {
                    freezes.push((a + cheats::unit::EXP, 99u32.to_le_bytes().to_vec()));
                }
            }
        });
        app.cheats_ui.unit_freezes = freezes;
        ui.label(RichText::new("\"Unit can move again\" is not available yet: where the game keeps a unit's moved flag has not been found.").size(12.0).color(p.muted));
    });
    // The cheat thread picks the held values up from the page's hand-off.
    if let Some(r) = &app.cheats_ui.runner {
        let mut s = r.lock().unwrap();
        s.freezes.retain(|(a, _)| !app.cheats_ui.units.iter().any(|u| *a >= *u && *a < *u + cheats::unit::LEN as u64));
        s.freezes.extend(app.cheats_ui.unit_freezes.iter().cloned());
    }
}
