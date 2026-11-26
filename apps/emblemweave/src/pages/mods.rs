//! Mods: the installed RomFS mods, importing archives and folders, and
//! browsing and installing from GameBanana.

use crate::mods::{self, GbFile, GbMod, Installed, Prepared};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

type Slot<T> = Arc<Mutex<Option<Result<T, String>>>>;

/// What a background download ends with.
enum Step {
    Done(Installed),
    /// The download offers several variants: the user picks.
    Choose(Prepared),
}

/// A question for the user. While it waits, nothing else is started.
enum Ask {
    /// Which of a GameBanana mod's files to download: any number, each
    /// installed into its own folder.
    File { m: GbMod, files: Vec<GbFile>, all: bool, picked: Vec<bool> },
    /// Which variants of a download to install.
    Variant { name: String, p: Prepared, picked: Vec<bool> },
}

#[derive(Default)]
pub struct Ui {
    link: String,
    browse: Vec<GbMod>,
    more: bool,
    page: u32,
    listing: Option<Slot<(Vec<GbMod>, bool)>>,
    fetching: Option<Slot<(GbMod, Vec<GbFile>)>>,
    /// The download or install running, and what it is.
    installing: Option<(String, Slot<Step>)>,
    /// Files picked together that wait for the install before them (one
    /// install or question at a time), oldest first.
    pending: VecDeque<(String, GbMod, GbFile)>,
    ask: Option<Ask>,
    /// The name typed into the save-as-profile box.
    profile_name: String,
    /// The installed list and its conflict scan, kept briefly. Both walk every
    /// mod tree; recomputing them on every scroll frame cost more than drawing
    /// the page.
    scan: Option<Scan>,
}

struct Scan {
    at: std::time::Instant,
    installed: Vec<mods::Mod>,
    conflicts: Vec<mods::Conflict>,
    /// The build's view of code mods, computed with the scan. These checks read
    /// the game's build ID and every mod's `.sites` files, so leaving them in
    /// the draw path was as expensive as the scan they depend on.
    site_problem: Option<String>,
    site_stale: bool,
    missing_code: Vec<String>,
}

/// Long enough that a scroll does not rescan, short enough that a folder
/// dropped in from Explorer appears on the next redraw. Actions on the page
/// invalidate the scan immediately.
const SCAN_TTL: std::time::Duration = std::time::Duration::from_secs(1);

impl Ui {
    /// A question is open: the page behind it takes no typing.
    fn asking(&self) -> bool {
        self.ask.is_some()
    }

    fn refresh_scan(&mut self, dir: &std::path::Path) {
        if self
            .scan
            .as_ref()
            .is_some_and(|scan| scan.at.elapsed() < SCAN_TTL)
        {
            return;
        }
        let installed = mods::list(dir);
        let conflicts = mods::conflicts(&installed);
        let engine = crate::engine::root();
        let code = installed.iter().any(|m| m.has_code);
        let site_problem = code
            .then(|| engine.as_deref().and_then(|root| crate::sites::problem_for(root, &installed)))
            .flatten();
        let site_stale = code
            && engine
                .as_deref()
                .and_then(|root| crate::sites::stale_for(root, &installed))
                == Some(true);
        let missing_code = installed
            .iter()
            .filter(|m| m.has_code)
            .filter(|m| {
                engine
                    .as_deref()
                    .is_some_and(|root| crate::sites::mod_missing(root, &m.path))
            })
            .map(|m| m.name.clone())
            .collect();
        self.scan = Some(Scan {
            at: std::time::Instant::now(),
            installed,
            conflicts,
            site_problem,
            site_stale,
            missing_code,
        });
    }

    fn invalidate_scan(&mut self) {
        self.scan = None;
    }
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
        Ok(i) if i.mods.is_empty() => (
            format!("{what} only patches the game's code (exefs), which cannot apply to a recompiled game. Nothing was installed."),
            Tone::Warn,
        ),
        Ok(i) if i.needs_build => (
            format!("Installed {}. It adds hook sites to the game's code: press Build the game before playing, or its code mod cannot work.", i.mods.join(", ")),
            Tone::Warn,
        ),
        Ok(i) if i.mods.len() > 1 && i.parts => (format!("Installed {what} as {} parts, all switched on: {}", i.mods.len(), i.mods.join(", ")), Tone::Ok),
        Ok(i) if i.mods.len() > 1 => (format!("Installed {what} as {} variants; only {} is switched on: {}", i.mods.len(), i.mods[0], i.mods.join(", ")), Tone::Ok),
        Ok(i) => (format!("Installed {}", i.mods.join(", ")), Tone::Ok),
        Err(e) => (format!("Could not install {what}: {e}"), Tone::Bad),
    }
}

fn day(ts: u64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Start loading the game's GameBanana list (the tour uses it too).
pub fn load_gamebanana(app: &mut App) {
    app.mods_ui.page = 1;
    app.mods_ui.listing = Some(spawn(|| mods::gb_list(1)));
}

fn start_install(ui: &mut Ui, name: String, m: GbMod, f: GbFile, dir: PathBuf) {
    let slot = spawn(move || {
        let p = mods::gb_prepare(&m, &f)?;
        if p.choices.len() > 1 {
            return Ok(Step::Choose(p));
        }
        let picks: Vec<usize> = (0..p.choices.len()).collect();
        let r = mods::finish(&p, &picks, &dir);
        p.discard();
        r.map(Step::Done)
    });
    ui.installing = Some((name, slot));
}

/// The files ticked when the file picker opens: the recommended one.
fn default_picks(files: &[GbFile], rec: Option<usize>) -> Vec<bool> {
    (0..files.len()).map(|i| Some(i) == rec).collect()
}

/// The ticked files in the order they are installed: page order, except that
/// the files of one family (`mods::family`, one mod in several builds) come
/// together, the one that fits this PC build best (`mods::pick` within the
/// family) first.
fn install_order(files: &[GbFile], picked: &[bool]) -> Vec<usize> {
    let ticked: Vec<usize> = (0..files.len()).filter(|&i| picked.get(i).copied().unwrap_or(false)).collect();
    let fams: Vec<String> = ticked.iter().map(|&i| mods::family(&files[i].name)).collect();
    let mut order: Vec<(usize, bool, usize)> = ticked
        .iter()
        .enumerate()
        .map(|(n, &i)| {
            let first = fams.iter().position(|f| *f == fams[n]).unwrap_or(n);
            (first, mods::pick(files, &files[i].name) != Some(i), i)
        })
        .collect();
    order.sort();
    order.into_iter().map(|(_, _, i)| i).collect()
}

/// Queue the ticked files of a mod's page (`install_order`): each is
/// installed into its own folder, one after another (`next_pending`). With
/// several, each install is named by its file too.
fn queue_files(ui: &mut Ui, m: &GbMod, files: &[GbFile], picked: &[bool]) {
    let chosen: Vec<&GbFile> = install_order(files, picked).into_iter().map(|i| &files[i]).collect();
    let several = chosen.len() > 1;
    for f in chosen {
        let name = if several { format!("{} ({})", m.name, f.name) } else { m.name.clone() };
        ui.pending.push_back((name, m.clone(), f.clone()));
    }
}

/// Start the next picked file once nothing else runs or asks.
fn next_pending(ui: &mut Ui, dir: &std::path::Path) {
    if ui.installing.is_some() || ui.ask.is_some() || ui.fetching.is_some() {
        return;
    }
    if let Some((name, m, f)) = ui.pending.pop_front() {
        start_install(ui, name, m, f, dir.to_path_buf());
    }
}

/// A file's description line: version, size, upload day and the author's note.
fn file_meta(f: &GbFile) -> String {
    let mut bits = Vec::new();
    if !f.version.is_empty() {
        bits.push(f.version.clone());
    }
    bits.push(if f.size >= 1_000_000 { format!("{:.1} MB", f.size as f64 / 1e6) } else { format!("{} KB", (f.size + 999) / 1000) });
    if f.date > 0 {
        bits.push(format!("uploaded {}", day(f.date)));
    }
    if !f.description.trim().is_empty() {
        bits.push(f.description.trim().to_string());
    }
    bits.join("  ·  ")
}

/// The question waiting for an answer, as a dialog.
fn ask_modal(app: &mut App, ctx: &egui::Context, dir: &std::path::Path) -> Option<(String, Tone)> {
    let p = app.config.theme.palette();
    let mut message = None;
    let mut done = false;
    match app.mods_ui.ask.as_mut()? {
        Ask::File { m, files, all, picked } => {
            let rec = mods::pick(files, "");
            let hidden = files.iter().filter(|f| !f.playable()).count();
            let n_picked = picked.iter().filter(|x| **x).count();
            let mut go = false;
            let modal = widgets::modal(
                ctx,
                &p,
                "mods-pick-file",
                &format!("Which files of {}?", m.name),
                "The mod's page offers several downloads. Old versions and source code are hidden; the recommended one fits this PC build. Each file you tick is installed into its own folder.",
                640.0,
                |ui| {
                    for (i, f) in files.iter().enumerate() {
                        if !*all && !f.playable() {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            let mut on = picked[i];
                            if widgets::toggle(ui, &p, &mut on).changed() {
                                picked[i] = on;
                            }
                            ui.vertical(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(&f.name).font(theme::mono(13.0)).color(if f.playable() { p.ink } else { p.muted }));
                                    if Some(i) == rec {
                                        widgets::pill(ui, &p, "Recommended", Tone::Accent);
                                    }
                                    if f.archived {
                                        widgets::pill(ui, &p, "Old version", Tone::Warn);
                                    }
                                    if f.source_only() {
                                        widgets::pill(ui, &p, "Source code", Tone::Neutral);
                                    } else if f.fit() < 0 {
                                        widgets::pill(ui, &p, "Console / emulator", Tone::Warn);
                                    }
                                });
                                ui.label(RichText::new(file_meta(f)).size(12.0).color(p.muted));
                            });
                        });
                        ui.add_space(6.0);
                    }
                    if hidden > 0 && hidden < files.len() {
                        let label = if *all { "Hide old versions and source code".to_string() } else { format!("Show {hidden} old version{} and source code", if hidden == 1 { "" } else { "s" }) };
                        if ui.link(RichText::new(label).size(12.5)).clicked() {
                            *all = !*all;
                        }
                    }
                    if n_picked > 1 {
                        ui.label(RichText::new("Several picked: they are installed one after another, each into its own folder.").size(12.0).color(p.muted));
                    }
                },
                |ui| {
                    ui.horizontal(|ui| {
                        let label = if n_picked > 1 { format!("Install {n_picked}") } else { "Install".to_string() };
                        if widgets::primary_button(ui, &p, &label, n_picked > 0).clicked() {
                            go = true;
                        }
                        if widgets::button(ui, &p, "Cancel", true).clicked() {
                            done = true;
                        }
                        ui.hyperlink_to("Open the mod's page", &m.url);
                    });
                },
            );
            if go && n_picked > 0 {
                let (m, files, picked) = (m.clone(), files.clone(), picked.clone());
                done = true;
                // The question is still set; it goes below, then the first file starts.
                queue_files(&mut app.mods_ui, &m, &files, &picked);
            } else if modal.close {
                done = true;
            }
        }
        Ask::Variant { name, p: prep, picked } => {
            let mut go = false;
            let title = format!("What to install from {name}?");
            let builds = prep.platform_builds();
            let sub = if builds {
                "The download holds several versions of the mod, one per platform. Only the ones you pick are installed; the recommended one fits this PC build."
            } else {
                "The download holds several parts of the mod. Only the ones you pick are installed."
            };
            let rec = prep.recommended();
            let n_picked = picked.iter().filter(|x| **x).count();
            let modal = widgets::modal(
                ctx,
                &p,
                "mods-pick-variant",
                &title,
                sub,
                640.0,
                |ui| {
                    for (i, c) in prep.choices.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let mut on = picked[i];
                            if widgets::toggle(ui, &p, &mut on).changed() {
                                picked[i] = on;
                            }
                            ui.vertical(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(&c.label).font(theme::semibold(13.5)).color(p.ink));
                                    if i == rec && c.fit > 0 {
                                        widgets::pill(ui, &p, "Recommended", Tone::Accent);
                                    }
                                    if c.fit < 0 {
                                        widgets::pill(ui, &p, "Console / emulator", Tone::Warn);
                                    }
                                });
                                let mut bits = vec![format!("{} files", c.files)];
                                if c.romfs.is_some() {
                                    bits.push("game files (romfs)".into());
                                }
                                if c.exefs.is_some() {
                                    bits.push("code (exefs)".into());
                                }
                                ui.label(RichText::new(bits.join("  ·  ")).size(12.0).color(p.muted));
                            });
                        });
                        ui.add_space(4.0);
                    }
                    let n = picked.iter().filter(|x| **x).count();
                    let ticked: Vec<usize> = (0..picked.len()).filter(|&i| picked[i]).collect();
                    if n > 1 && prep.exclusive(&ticked) {
                        // As `mods::finish` decides: the recommended one when picked, else the first picked.
                        let on = if picked[rec] { rec } else { picked.iter().position(|x| *x).unwrap_or(0) };
                        let why = if on != rec { ", the first picked," } else if prep.choices[rec].fit > 0 { ", the recommended one," } else { "" };
                        let what = if builds { "Several picked" } else { "Several picked replace the same files" };
                        let text = format!("{what}: {}{why} is switched on; the others are installed switched off.", prep.choices[on].label);
                        ui.label(RichText::new(text).size(12.0).color(p.warn));
                    } else if n > 1 {
                        ui.label(RichText::new("Several picked: each is installed into its own folder, all switched on.").size(12.0).color(p.muted));
                    }
                },
                |ui| {
                    ui.horizontal(|ui| {
                        let n = n_picked;
                        let label = if n > 1 { format!("Install {n}") } else { "Install".to_string() };
                        if widgets::primary_button(ui, &p, &label, n > 0).clicked() {
                            go = true;
                        }
                        if widgets::button(ui, &p, "Cancel", true).clicked() {
                            done = true;
                        }
                    });
                },
            );
            if go {
                let picks: Vec<usize> = (0..picked.len()).filter(|&i| picked[i]).collect();
                if picks.is_empty() {
                    return None;
                }
                let prep = prep.clone();
                let d = dir.to_path_buf();
                let name = name.clone();
                let slot = spawn(move || {
                    let r = mods::finish(&prep, &picks, &d);
                    prep.discard();
                    r.map(Step::Done)
                });
                app.mods_ui.installing = Some((name, slot));
                // The install owns the unpacked copy now: drop the question without discarding it.
                app.mods_ui.ask = None;
                return None;
            }
            // A stray click beside the dialog does not throw the download away.
            if (modal.close && !modal.veil) || done {
                prep.discard();
                done = true;
                message = Some((format!("Nothing installed from {name}"), Tone::Neutral));
            }
        }
    }
    if done {
        app.mods_ui.ask = None;
    }
    message
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Mods", "RomFS mods laid over the game's files when it starts, from an archive, a folder or GameBanana.");
    let Some(dir) = mods::dir() else { return };
    let mut messages: Vec<(String, Tone)> = Vec::new();
    app.mods_ui.refresh_scan(&dir);
    let installed = app
        .mods_ui
        .scan
        .as_ref()
        .map(|scan| scan.installed.clone())
        .unwrap_or_default();
    let shared = app
        .mods_ui
        .scan
        .as_ref()
        .map(|scan| scan.conflicts.clone())
        .unwrap_or_default();
    let site_problem = app
        .mods_ui
        .scan
        .as_ref()
        .and_then(|scan| scan.site_problem.clone());
    let site_stale = app
        .mods_ui
        .scan
        .as_ref()
        .is_some_and(|scan| scan.site_stale);
    let missing_code = app
        .mods_ui
        .scan
        .as_ref()
        .map(|scan| scan.missing_code.clone())
        .unwrap_or_default();
    let busy = app.mods_ui.installing.is_some() || !app.mods_ui.pending.is_empty() || app.mods_ui.fetching.is_some() || app.mods_ui.listing.is_some() || app.mods_ui.ask.is_some();
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
            Err(e) => messages.push((format!("GameBanana: {e}"), Tone::Bad)),
        }
    }
    if let Some(r) = take(&app.mods_ui.fetching) {
        app.mods_ui.fetching = None;
        match r {
            Ok((m, files)) if files.is_empty() => messages.push((format!("{} has no files to download", m.name), Tone::Warn)),
            Ok((m, files)) => {
                let playable: Vec<&GbFile> = files.iter().filter(|f| f.playable()).collect();
                if playable.len() == 1 && playable[0].fit() >= 0 {
                    let f = playable[0].clone();
                    start_install(&mut app.mods_ui, m.name.clone(), m, f, dir.clone());
                } else {
                    let all = playable.is_empty();
                    let picked = default_picks(&files, mods::pick(&files, ""));
                    app.mods_ui.ask = Some(Ask::File { m, files, all, picked });
                }
            }
            Err(e) => messages.push((format!("GameBanana: {e}"), Tone::Bad)),
        }
    }
    let finished = app.mods_ui.installing.as_ref().and_then(|(name, slot)| Some((name.clone(), slot.lock().unwrap().take()?)));
    if let Some((name, r)) = finished {
        app.mods_ui.installing = None;
        // An install changed the mods folder, even one that failed partway.
        app.mods_ui.invalidate_scan();
        match r {
            Ok(Step::Done(i)) => messages.push(installed_message(&name, Ok(i))),
            Ok(Step::Choose(prep)) => {
                let rec = prep.recommended();
                let picked = (0..prep.choices.len()).map(|i| i == rec).collect();
                app.mods_ui.ask = Some(Ask::Variant { name, p: prep, picked });
            }
            Err(e) => messages.push(installed_message(&name, Err(e))),
        }
    }
    let ctx = ui.ctx().clone();
    if let Some(m) = ask_modal(app, &ctx, &dir) {
        messages.push(m);
    }
    next_pending(&mut app.mods_ui, &dir);

    widgets::section(ui, &p, "Installed", |ui| {
        let list = &installed;
        if list.is_empty() {
            ui.label(RichText::new("No mods installed.").color(p.muted));
        }
        // Code mods whose hook sites the built game lacks (installed or updated since the last build).
        // A problem in a .sites file stops the build: say which file and line before it is pressed.
        if let Some(e) = site_problem.as_deref() {
            ui.label(RichText::new(format!("A code mod's hook sites cannot be used: {e}")).size(12.5).color(p.bad));
        }
        let stale = site_stale;
        if stale {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Code mods bring hook sites the built game does not have yet: build the game again, or their code will not run.")
                        .size(12.5)
                        .color(p.warn),
                );
                if widgets::button(ui, &p, "Build the game now", !app.busy()).clicked() {
                    app.start(crate::jobs::JobKind::Engine);
                }
            });
        }
        for m in list {
            ui.horizontal(|ui| {
                let mut on = m.enabled;
                if widgets::toggle(ui, &p, &mut on).changed() {
                    match mods::set_enabled(m, on) {
                        Ok(_) => app.mods_ui.invalidate_scan(),
                        Err(e) => messages.push((e, Tone::Bad)),
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
                    if !m.info.variant.is_empty() {
                        bits.push(m.info.variant.clone());
                    }
                    if !m.has_romfs && !m.has_exefs {
                        bits.push("no romfs folder: the game will not see it".into());
                    }
                    if m.has_code {
                        let missing = missing_code.iter().any(|name| name == &m.name);
                        bits.push(if missing { "code mod: needs Build the game".into() } else { "code mod".into() });
                    }
                    if m.has_patches {
                        bits.push("its .ips / .pchtxt patches cannot apply".into());
                    }
                    ui.label(RichText::new(bits.join("  ·  ")).size(12.5).color(p.muted));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, &p, "Remove", true).on_hover_text("Moves the mod to the mods-removed folder").clicked() {
                        messages.push(match mods::remove(m) {
                            Ok(to) => {
                                app.mods_ui.invalidate_scan();
                                (format!("Moved {} to {}", m.info.title, to.display()), Tone::Ok)
                            }
                            Err(e) => (e, Tone::Bad),
                        });
                    }
                    if widgets::button(ui, &p, "Folder", true).clicked() {
                        let _ = std::process::Command::new("explorer").arg(&m.path).spawn();
                    }
                    if !m.info.source.is_empty() && m.info.source.starts_with("http") {
                        ui.hyperlink_to("Page", &m.info.source);
                    }
                });
            });
            ui.add_space(4.0);
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Changes apply the next time the game starts. When two mods change the same file, the one later in the list wins. A code mod is a DLL that runs inside the game with the game's own rights: install only mods you trust.").size(12.5).color(p.muted));
    });

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
            "The game loads its mods in name order, so for these files the last mod wins. Switch one off to decide it yourself.",
        );
        ui.add_space(theme::SPACE_S);
        egui::ScrollArea::vertical().id_salt("conflicts").max_height(crate::widgets::list_height(ui, 0.24)).show(ui, |ui| {
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
                messages.push((format!("Saved the mods that are on as {name}"), Tone::Ok));
            }
        });

        if let Some(i) = apply {
            let want = app.config.profiles[i].enabled.clone();
            match mods::apply_profile(&installed, &want) {
                Ok(n) => {
                    app.config.profile = app.config.profiles[i].name.clone();
                    app.mods_ui.invalidate_scan();
                    messages.push((format!("Applied {}: {n} mods changed", app.config.profiles[i].name), Tone::Ok));
                }
                Err(e) => messages.push((e, Tone::Bad)),
            }
        }
        if let Some(i) = overwrite {
            let enabled: Vec<String> = installed.iter().filter(|m| m.enabled).map(|m| m.name.clone()).collect();
            app.config.profiles[i].enabled = enabled;
            messages.push((format!("Updated {}", app.config.profiles[i].name), Tone::Ok));
        }
        if let Some(i) = delete {
            let gone = app.config.profiles.remove(i);
            if app.config.profile.eq_ignore_ascii_case(&gone.name) {
                app.config.profile.clear();
            }
            messages.push((format!("Removed {}", gone.name), Tone::Ok));
        }
    });

    widgets::section(ui, &p, "Add a mod", |ui| {
        ui.horizontal(|ui| {
            if widgets::button(ui, &p, "From archive…", !busy).clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter("Mod archive", &["zip", "7z", "rar", "tar", "gz"]).pick_file() {
                    let d = dir.clone();
                    let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    let slot = spawn(move || {
                        let p = mods::prepare(&f, mods::Info::default())?;
                        if p.choices.len() > 1 {
                            return Ok(Step::Choose(p));
                        }
                        let picks: Vec<usize> = (0..p.choices.len()).collect();
                        let r = mods::finish(&p, &picks, &d);
                        p.discard();
                        r.map(Step::Done)
                    });
                    app.mods_ui.installing = Some((name, slot));
                }
            }
            if widgets::button(ui, &p, "From folder…", !busy).clicked() {
                if let Some(f) = rfd::FileDialog::new().pick_folder() {
                    let d = dir.clone();
                    let name = f.display().to_string();
                    // Looking at the folder and copying it walk its files: off the UI thread, like an archive.
                    let slot = spawn(move || {
                        let p = mods::prepare(&f, mods::Info::default())?;
                        if p.choices.len() > 1 {
                            return Ok(Step::Choose(p));
                        }
                        let picks: Vec<usize> = (0..p.choices.len()).collect();
                        mods::finish(&p, &picks, &d).map(Step::Done)
                    });
                    app.mods_ui.installing = Some((name, slot));
                }
            }
            if widgets::button(ui, &p, "Open mods folder", true).clicked() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            // While a question is open, keys go to the dialog, not to this field behind it.
            ui.add_enabled(!app.mods_ui.asking(), egui::TextEdit::singleline(&mut app.mods_ui.link).hint_text("https://gamebanana.com/mods/…").desired_width(320.0));
            let id = mods::gb_id(&app.mods_ui.link);
            if widgets::button(ui, &p, "Install from GameBanana", id.is_some() && !busy).clicked() {
                let id = id.unwrap();
                app.mods_ui.fetching = Some(spawn(move || mods::gb_mod(id)));
            }
        });
        if app.mods_ui.fetching.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading the mod's page on GameBanana…");
            });
        }
        if let Some((name, _)) = &app.mods_ui.installing {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Installing {name}…"));
            });
        }
        let waiting = app.mods_ui.pending.len();
        if waiting > 0 {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{waiting} more file{} to install after this one.", if waiting == 1 { "" } else { "s" })).size(12.5).color(p.muted));
                if ui.link(RichText::new("Cancel the rest").size(12.5)).clicked() {
                    app.mods_ui.pending.clear();
                }
            });
        }
    });

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
        let installed_ids: Vec<u64> = installed.iter().filter_map(|m| m.info.gamebanana_id).collect();
        let mut fetch: Option<u64> = None;
        egui::Grid::new("gb").num_columns(4).spacing([14.0, 6.0]).striped(true).show(ui, |ui| {
            for m in &app.mods_ui.browse {
                ui.vertical(|ui| {
                    ui.set_min_width(460.0);
                    ui.hyperlink_to(RichText::new(&m.name).font(theme::semibold(13.5)), &m.url);
                    ui.label(RichText::new(format!("{} by {}{}", m.kind, m.author, if m.version.is_empty() { String::new() } else { format!(", {}", m.version) })).size(12.0).color(p.muted));
                });
                ui.label(RichText::new(format!("♥ {}", m.likes)).color(p.muted));
                if installed_ids.contains(&m.id) {
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
        ui.label(RichText::new("Mods are made by their authors and installed as they are; code mods made for this engine (a DLL in exefs) load at start, but .ips / .pchtxt patches cannot apply.").size(12.5).color(p.muted));
    });

    for (text, tone) in messages {
        app.notify(text, tone);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gfile(id: u64, name: &str, desc: &str, date: u64) -> GbFile {
        GbFile { id, name: name.into(), description: desc.into(), date, ..GbFile::default() }
    }

    /// The file picker ticks the recommended file; every ticked file is
    /// queued, in page order, each to be installed into its own folder.
    #[test]
    fn the_file_picker_ticks_the_recommended_file_and_queues_each_ticked_one() {
        let m = GbMod { id: 742827, kind: "Mod".into(), name: "Turn Control".into(), author: String::new(), version: String::new(), likes: 0, url: String::new() };
        let files = vec![gfile(1, "turncontrol-030-emblemweave.zip", "for recomps", 200), gfile(2, "turncontrol-030-emulator.zip", "atmosphere for emus", 200)];
        let rec = mods::pick(&files, "");
        assert_eq!(rec, Some(0));
        assert_eq!(default_picks(&files, rec), vec![true, false]);
        let mut ui = Ui::default();
        queue_files(&mut ui, &m, &files, &[true, true]);
        let names: Vec<(String, u64)> = ui.pending.iter().map(|(n, _, f)| (n.clone(), f.id)).collect();
        assert_eq!(names, vec![("Turn Control (turncontrol-030-emblemweave.zip)".to_string(), 1), ("Turn Control (turncontrol-030-emulator.zip)".to_string(), 2)]);
        // The file that fits this PC build goes first within its family, wherever it is on the page.
        let flipped = vec![files[1].clone(), files[0].clone()];
        assert_eq!(install_order(&flipped, &[true, true]), vec![1, 0]);
        let mut ui = Ui::default();
        queue_files(&mut ui, &m, &flipped, &[true, true]);
        assert_eq!(ui.pending.iter().map(|(_, _, f)| f.id).collect::<Vec<_>>(), vec![1, 2]);
        // Other families keep page order, each family together.
        let three = vec![gfile(3, "other-100.zip", "", 100), files[1].clone(), files[0].clone()];
        assert_eq!(install_order(&three, &[true, true, true]), vec![0, 2, 1]);
        // One file keeps the mod's own name.
        let mut ui = Ui::default();
        queue_files(&mut ui, &m, &files, &[false, true]);
        assert_eq!(ui.pending.iter().map(|(n, _, _)| n.as_str()).collect::<Vec<_>>(), vec!["Turn Control"]);
    }
}
