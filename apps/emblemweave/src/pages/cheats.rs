//! Cheats: Atmosphere cheat codes to switch on and off, and Cheat Engine
//! tables with live, editable and freezable values.

use crate::cheats::{self, Code, Entry, Memory, Table, ScanCancelToken, ScanProgress, ScanUnitsConfig};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

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
    searching: Option<Arc<Mutex<Option<Result<Vec<u64>, String>>>>>,
    /// Cancellation token for the unit scan.
    scan_cancel: Option<Arc<ScanCancelToken>>,
    /// Progress of the current unit scan.
    scan_progress: Option<Arc<Mutex<ScanProgress>>>,
    /// When the scan started.
    scan_start: Option<Instant>,
    /// Per unit: infinite HP, quick level-up, max stats.
    unit_cheats: HashMap<u64, [bool; 3]>,
    /// Item pickup multiplier and its live convoy search.
    item_factor: u32,
    item_on: bool,
    item_status: String,
    item_search: Option<Arc<Mutex<Option<Result<cheats::ItemMultiplier, String>>>>>,
    /// Free-time clock hold (the game's turn currency) and its search.
    clock_on: bool,
    clock_status: String,
    clock_search: Option<Arc<Mutex<Option<Result<cheats::ClockFreeze, String>>>>>,
}

impl Ui {
    /// Forget the units of the game that just stopped: their records live at
    /// different addresses in the next run.
    pub fn reset_units(&mut self) {
        self.units.clear();
        self.unit_cheats.clear();
        self.searching = None;
        self.scan_cancel = None;
        self.scan_progress = None;
        self.scan_start = None;
        self.item_on = false;
        self.item_search = None;
        self.item_status = "Start the game and load a save, then switch the multiplier on again.".into();
        self.clock_on = false;
        self.clock_search = None;
        self.clock_status = "Start the game and load a save, then switch the clock hold on again.".into();
    }
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
    if ui_state.item_factor < 2 {
        ui_state.item_factor = 3;
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
    widgets::page_header(ui, &p, "Cheats", "Quick per-unit cheats, Atmosphere cheat codes and Cheat Engine tables, applied to the game while it runs.");
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
    // Live values only exist while the game runs; with no game attached the
    // page is a still list and needs no frames of its own.
    if game.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
    }

    match &game {
        Some(g) => widgets::status_line(ui, &p, Tone::Ok, "Game running", &format!("Attached to {} (process {}). Switched-on codes are applied about 60 times a second.", cheats::GAME_EXE, g.pid)),
        None => widgets::status_line(ui, &p, Tone::Neutral, "Game not running", "Switch codes on here; they apply as soon as the game starts. Table values show while it runs."),
    }
    ui.add_space(theme::SPACE_S);

    // ------------------------------------------------------- quick cheats
    // The same switches the in-game menu's Quick tab has (F10); they need the
    // game running to find its units, so they are also offered here rather
    // than only behind the in-game menu.
    quick(app, ui);

    // ------------------------------------------------------------ codes
    let mut enabled_changed = false;
    widgets::section(ui, &p, "Cheat codes (Atmosphere format)", |ui| {
        if app.cheats_ui.files.is_empty() {
            ui.label(crate::i18n::text("No cheat files yet. Add codes below or import a .txt from a cheats folder.").color(p.muted));
        }
        for f in &app.cheats_ui.files {
            ui.label(crate::i18n::text(&f.name).font(theme::semibold(14.0)).color(p.ink));
            if let Some(e) = &f.error {
                widgets::status_line(ui, &p, Tone::Bad, "Could not read it", e);
            }
            let on = app.cheats_ui.enabled.entry(f.name.clone()).or_default();
            for c in &f.codes {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    if c.master {
                        widgets::pill(ui, &p, "master", Tone::Accent);
                        ui.label(crate::i18n::text(&c.name).color(p.body)).on_hover_text(crate::i18n::tr("Runs whenever any code of this file is on"));
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
                    ui.label(crate::i18n::text(&c.name).color(p.ink));
                    ui.label(crate::i18n::text(format!("{} words", c.words.len())).size(12.0).color(p.muted));
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
        ui.label(crate::i18n::text("Add codes").font(theme::semibold(14.0)).color(p.ink));
        ui.label(
            crate::i18n::text("Paste codes as they are shared for Atmosphere: a [Name] line, then the code's hex words. Several codes can go in one paste.")
                .size(12.5)
                .color(p.muted),
        );
        ui.horizontal(|ui| {
            ui.label(crate::i18n::text("File name"));
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
                    ui.label(crate::i18n::text(e).color(p.bad));
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
            crate::i18n::text("Codes that patch the game's instructions do nothing here (the code is compiled ahead of time); codes that change data, follow pointers or test buttons work. Button tests read the first controller.")
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
            crate::i18n::text("Switch on a script entry to find its pattern in the game's memory; the values below it then show live. Type a value and press Enter to write it, or tick Freeze to hold it.")
                .size(12.5)
                .color(p.muted),
        );
        for (ti, t) in app.cheats_ui.tables.iter_mut().enumerate() {
            egui::CollapsingHeader::new(crate::i18n::text(&t.table.name).font(theme::semibold(14.0)).color(p.ink)).id_salt(("table", ti)).default_open(true).show(ui, |ui| {
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
        let freezes: Vec<(u64, Vec<u8>)> = app.cheats_ui.tables.iter().flat_map(|t| t.frozen.values().cloned()).collect();
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
            ui.label(crate::i18n::text(&e.description).font(theme::semibold(13.5)).color(p.ink));
            if e.unsupported_script {
                ui.label(crate::i18n::text("uses script commands EmblemWeave cannot run").size(12.0).color(p.warn));
            }
            if let Some(n) = t.notes.get(&e.id) {
                ui.label(crate::i18n::text(n).size(12.0).color(p.muted));
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
            ui.label(crate::i18n::text(&e.description).color(p.body));
            match (addr, game) {
                (Some(a), Some(g)) => {
                    let mut buf = vec![0u8; var.size()];
                    let value = if g.read(a, &mut buf) { var.format(&buf, e.signed) } else { "??".into() };
                    ui.label(crate::i18n::text(&value).font(theme::mono(13.0)).color(p.ink)).on_hover_text(format!("{a:#x}"));
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
                    if ui.checkbox(&mut frozen, crate::i18n::text("Freeze")).changed() {
                        if frozen {
                            t.frozen.insert(e.id.clone(), (a, buf));
                        } else {
                            t.frozen.remove(&e.id);
                        }
                    }
                }
                (None, _) => {
                    ui.label(crate::i18n::text("switch on the entry above").size(12.0).color(p.muted));
                }
                (Some(_), None) => {
                    ui.label(crate::i18n::text("game not running").size(12.0).color(p.muted));
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
    ("Infinite HP", "HP is held at the unit's max HP every tick, including from 0 HP"),
    ("Quick level-up", "the level rises one step every quarter second, up to 40"),
    ("Max stats", "max HP and every stat set to 200 (the game may cap them by class)"),
];

fn clock_text(value: &[u8]) -> String {
    if value.len() < 6 {
        return "the save's clock".into();
    }
    let year = u16::from_le_bytes([value[0], value[1]]);
    format!("{year:04}-{:02}-{:02} {:02}:{:02}", value[2], value[3], value[4], value[5])
}

/// Party-wide modifications derived from ReWeaver's save layout: convoy row
/// quantities and the free-time clock that gives each chapter its turns.
fn toolbox(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    if app.cheats_ui.runner.is_none() {
        app.cheats_ui.runner = Some(cheats::start());
    }
    if app.cheats_ui.item_factor < 2 {
        app.cheats_ui.item_factor = 3;
    }
    let game = cheats::game();
    if let Some(done) = app
        .cheats_ui
        .item_search
        .as_ref()
        .and_then(|s| s.lock().unwrap().take())
    {
        app.cheats_ui.item_search = None;
        match done {
            Ok(m) => {
                let (base, rows) = (m.base, m.slots.len());
                if let Some(r) = &app.cheats_ui.runner {
                    r.lock().unwrap().item_multiplier = Some(m);
                }
                app.cheats_ui.item_on = true;
                app.cheats_ui.item_status = format!("Watching {rows} convoy rows at {base:#x}.");
            }
            Err(e) => {
                app.cheats_ui.item_on = false;
                app.cheats_ui.item_status = e.clone();
                app.notify(e, Tone::Bad);
            }
        }
    }
    if let Some(done) = app
        .cheats_ui
        .clock_search
        .as_ref()
        .and_then(|s| s.lock().unwrap().take())
    {
        app.cheats_ui.clock_search = None;
        match done {
            Ok(c) => {
                let (n, text) = (c.addresses.len(), clock_text(&c.value));
                if let Some(r) = &app.cheats_ui.runner {
                    r.lock().unwrap().clock_freeze = Some(c);
                }
                app.cheats_ui.clock_on = true;
                app.cheats_ui.clock_status = format!("Holding {text} in {n} memory location(s).");
            }
            Err(e) => {
                app.cheats_ui.clock_on = false;
                app.cheats_ui.clock_status = e.clone();
                app.notify(e, Tone::Bad);
            }
        }
    }

    widgets::section(ui, &p, "Party & free time", |ui| {
        ui.label(
            crate::i18n::text(
                "These use the save layout ReWeaver and the community save editor document: convoy rows hold a quantity, and free time spends one turn each time the clock crosses a six-hour boundary.",
            )
            .size(12.0)
            .color(p.muted),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let mut on = app.cheats_ui.item_on;
            if widgets::toggle(ui, &p, &mut on).changed() {
                if on {
                    match (game.clone(), crate::config::Config::save_dir()) {
                        (Some(g), Some(dir)) => {
                            let factor = app.cheats_ui.item_factor.max(2);
                            let slot: Arc<Mutex<Option<Result<cheats::ItemMultiplier, String>>>> = Default::default();
                            let out = slot.clone();
                            std::thread::spawn(move || {
                                let r = cheats::find_item_multiplier(g.as_ref(), &dir, factor);
                                *out.lock().unwrap() = Some(r);
                            });
                            app.cheats_ui.item_search = Some(slot);
                            app.cheats_ui.item_on = true;
                            app.cheats_ui.item_status = "Searching the game's memory for the current convoy…".into();
                        }
                        (None, _) => {
                            app.cheats_ui.item_on = false;
                            app.notify("Start the game and load a save first", Tone::Warn);
                        }
                        (_, None) => {
                            app.cheats_ui.item_on = false;
                            app.notify("The launcher's save folder is not set up yet", Tone::Warn);
                        }
                    }
                } else {
                    app.cheats_ui.item_on = false;
                    app.cheats_ui.item_search = None;
                    app.cheats_ui.item_status = "Off.".into();
                    if let Some(r) = &app.cheats_ui.runner {
                        r.lock().unwrap().item_multiplier = None;
                    }
                }
            }
            ui.label(crate::i18n::text("Item pickup multiplier").font(theme::semibold(13.0)).color(p.ink))
                .on_hover_text("Multiplies the quantity the game adds to an existing stack of 2 or more. New rows and one-slot gear are left alone");
            let mut factor = app.cheats_ui.item_factor.max(2);
            if ui
                .add(egui::DragValue::new(&mut factor).range(2..=9).speed(0.2))
                .on_hover_text("How many copies a gain becomes (2-9)")
                .changed()
            {
                app.cheats_ui.item_factor = factor;
                if let Some(r) = &app.cheats_ui.runner {
                    if let Some(m) = r.lock().unwrap().item_multiplier.as_mut() {
                        m.factor = factor;
                    }
                }
            }
            ui.label(crate::i18n::text("×").color(p.muted));
        });
        if !app.cheats_ui.item_status.is_empty() {
            ui.label(crate::i18n::text(&app.cheats_ui.item_status).size(11.5).color(p.muted));
        }

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let mut on = app.cheats_ui.clock_on;
            if widgets::toggle(ui, &p, &mut on).changed() {
                if on {
                    match (game.clone(), crate::config::Config::save_dir()) {
                        (Some(g), Some(dir)) => {
                            let slot: Arc<Mutex<Option<Result<cheats::ClockFreeze, String>>>> = Default::default();
                            let out = slot.clone();
                            std::thread::spawn(move || {
                                let r = cheats::find_clock_freeze(g.as_ref(), &dir);
                                *out.lock().unwrap() = Some(r);
                            });
                            app.cheats_ui.clock_search = Some(slot);
                            app.cheats_ui.clock_on = true;
                            app.cheats_ui.clock_status = "Looking for the free-time clock in the game's memory…".into();
                        }
                        (None, _) => {
                            app.cheats_ui.clock_on = false;
                            app.notify("Start the game and load a save first", Tone::Warn);
                        }
                        (_, None) => {
                            app.cheats_ui.clock_on = false;
                            app.notify("The launcher's save folder is not set up yet", Tone::Warn);
                        }
                    }
                } else {
                    app.cheats_ui.clock_on = false;
                    app.cheats_ui.clock_search = None;
                    app.cheats_ui.clock_status = "Off.".into();
                    if let Some(r) = &app.cheats_ui.runner {
                        r.lock().unwrap().clock_freeze = None;
                    }
                }
            }
            ui.label(crate::i18n::text("Max action points (hold the free-time clock)")
                .font(theme::semibold(13.0))
                .color(p.ink))
                .on_hover_text("Freezes the in-game date and time at the save's current value, so the clock never crosses the six-hour boundaries that spend free-time turns");
        });
        if !app.cheats_ui.clock_status.is_empty() {
            ui.label(crate::i18n::text(&app.cheats_ui.clock_status).size(11.5).color(p.muted));
        }
        ui.label(
            crate::i18n::text(
                "Switch the clock hold on while free time is visible. It keeps the current date and time, so exploration and activities no longer cross a turn boundary; save before switching it off if you want the game's clock to continue normally.",
            )
            .size(11.0)
            .color(p.muted),
        );
    });
}

/// Quick cheats: per-unit switches for the units found in the game's memory.
pub fn quick(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    if app.cheats_ui.runner.is_none() {
        app.cheats_ui.runner = Some(cheats::start());
    }
    toolbox(app, ui);
    let game = cheats::game();
    widgets::section(ui, &p, "Units on the map", |ui| {
        ui.label(
            crate::i18n::text("Finds the units in the game's memory by the layout their records share (experimental: results may include units not on the map). Search once a map is loaded, and again on the next map.")
                .size(12.5)
                .color(p.muted),
        );
        ui.label(
            crate::i18n::text("Each unit has three switches: Infinite HP (HP held at max, even from 0), Quick level-up (one level every quarter second, to 40) and Max stats (max HP and every stat set to 200). The in-game menu's Quick tab (F10) has the same switches.")
                .size(12.5)
                .color(p.body),
        );

        let busy = app.cheats_ui.searching.is_some();
        ui.horizontal(|ui| {
            if busy {
                // Show progress and cancel button while scanning.
                let cancel = app.cheats_ui.scan_cancel.clone();
                let progress = app.cheats_ui.scan_progress.clone();
                let start = app.cheats_ui.scan_start;

                let (completed, total, found, elapsed, cancelled, timed_out, device_lost) = if let Some(ref p) = progress {
                    let pg = p.lock().unwrap_or_else(|e| e.into_inner());
                    (pg.completed_pieces, pg.total_pieces, pg.units_found, pg.elapsed_secs, pg.cancelled, pg.timed_out, pg.device_lost)
                } else {
                    (0, 0, 0, 0.0, false, false, false)
                };

                // Cancel and the deadline are observed when a read returns. The status says so rather than implying a
                // ReadProcessMemory call in flight can be interrupted.
                let status = if device_lost {
                    "The game closed; finishing the scan…".to_string()
                } else if timed_out {
                    "Timed out; waiting for the current memory read to return".to_string()
                } else if cancelled {
                    "Cancelling; waiting for the current memory read to return".to_string()
                } else if total > 0 {
                    format!("Scanning… {}/{} pieces, {} units found ({:.1}s)", completed, total, found, elapsed)
                } else {
                    "Searching…".to_string()
                };

                if widgets::button(ui, &p, "Cancel", !cancelled && !device_lost).clicked() {
                    if let Some(c) = cancel {
                        c.cancel();
                    }
                    // Show the request immediately even while a read is blocked. The worker keeps running until that
                    // read returns, so this is deliberately "Cancelling", not "Cancelled".
                    if let Some(pg) = &progress {
                        let mut pg = pg.lock().unwrap_or_else(|e| e.into_inner());
                        pg.cancelled = true;
                        pg.elapsed_secs = start.map_or(pg.elapsed_secs, |s| s.elapsed().as_secs_f64());
                    }
                }
                ui.label(crate::i18n::text(&status).color(p.muted));
            } else {
                if widgets::button(ui, &p, "Find units", game.is_some()).clicked() {
                    if let Some(g) = game.clone() {
                        let cancel = Arc::new(ScanCancelToken::new());
                        let progress = Arc::new(Mutex::new(ScanProgress {
                            total_pieces: 0,
                            completed_pieces: 0,
                            units_found: 0,
                            elapsed_secs: 0.0,
                            remaining_secs: 0.0,
                            cancelled: false,
                            timed_out: false,
                            device_lost: false,
                        }));
                        let start = Instant::now();
                        let out_cancel = cancel.clone();
                        let out_progress = progress.clone();
                        let slot: Arc<Mutex<Option<Result<Vec<u64>, String>>>> = Default::default();
                        let out_slot = slot.clone();

                        std::thread::spawn(move || {
                            let config = ScanUnitsConfig::default();
                            let (units, prog) = cheats::scan_units(g.as_ref(), config, Some(&out_cancel), Some(&out_progress));
                            let result = cheats::scan_outcome(units, &prog);
                            *out_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                        });

                        app.cheats_ui.searching = Some(slot);
                        app.cheats_ui.scan_cancel = Some(cancel);
                        app.cheats_ui.scan_progress = Some(progress);
                        app.cheats_ui.scan_start = Some(start);
                    }
                }
            }
            if game.is_none() {
                ui.label(crate::i18n::text("Start the game first.").color(p.muted));
            }
        });

        // Check for completed scan.
        if let Some(done) = app.cheats_ui.searching.as_ref().and_then(|s| s.lock().unwrap().take()) {
            app.cheats_ui.searching = None;
            app.cheats_ui.scan_cancel = None;
            app.cheats_ui.scan_progress = None;
            app.cheats_ui.scan_start = None;
            match done {
                Ok(units) => {
                    app.cheats_ui.units = units.into_iter().take(40).collect();
                    app.cheats_ui.unit_cheats.retain(|k, _| app.cheats_ui.units.contains(k));
                    app.notify(String::from("Unit scan complete"), Tone::Ok);
                }
                Err(e) => {
                    app.notify(format!("Unit scan failed: {e}"), Tone::Bad);
                }
            }
        }

        let Some(g) = game.as_deref() else { return };
        egui::Grid::new("quick-units").num_columns(6).spacing([12.0, 6.0]).striped(true).show(ui, |ui| {
            if !app.cheats_ui.units.is_empty() {
                for h in ["Slot", "Lv", "HP", QUICK[0].0, QUICK[1].0, QUICK[2].0] {
                    ui.label(crate::i18n::text(h).font(theme::semibold(12.0)).color(p.muted));
                }
                ui.end_row();
            }
            for (n, &a) in app.cheats_ui.units.iter().enumerate() {
                let mut b = [0u8; cheats::unit::LEN];
                if !g.read(a, &mut b) || !cheats::looks_like_unit(&b) {
                    continue;
                }
                let max_hp = b[cheats::unit::MAX_HP as usize];
                let hp = u16::from_le_bytes([b[0x20], b[0x21]]);
                let level = b[cheats::unit::LEVEL as usize];
                let on = app.cheats_ui.unit_cheats.entry(a).or_default();
                ui.label(crate::i18n::text(format!("{}", n + 1)).color(p.ink)).on_hover_text(format!("record at {a:#x}"));
                ui.label(format!("{level}"));
                ui.label(format!("{hp} / {max_hp}"));
                for (i, (_, help)) in QUICK.iter().enumerate() {
                    widgets::toggle(ui, &p, &mut on[i]).on_hover_text(*help);
                }
                ui.end_row();
            }
        });
        ui.label(crate::i18n::text("\"Unit can move again\" is not available yet: where the game keeps a unit's moved flag has not been found.").size(12.0).color(p.muted));
    });
    // The cheat thread holds each switched-on cheat every tick, so they keep
    // applying while the menu is closed and to a unit that has just died.
    if let Some(r) = &app.cheats_ui.runner {
        let mut s = r.lock().unwrap();
        s.units = app
            .cheats_ui
            .unit_cheats
            .iter()
            .filter(|(a, on)| app.cheats_ui.units.contains(a) && on.iter().any(|x| *x))
            .map(|(a, on)| (*a, cheats::UnitCheats { infinite_hp: on[0], quick_level: on[1], max_stats: on[2] }))
            .collect();
        s.freezes.retain(|(a, _)| !app.cheats_ui.units.iter().any(|u| *a >= *u && *a < *u + cheats::unit::LEN as u64));
        if s.rejected > 0 {
            let n = s.rejected;
            ui.label(crate::i18n::text(format!("{n} writes were refused by the game: the record may have moved (find units again).")).size(12.0).color(p.warn));
        }
    }
}
