//! Mods: the installed RomFS mods, importing archives and folders, and
//! browsing and installing from GameBanana.

use crate::mods::{self, GbFile, GbMod, Installed, Mod, Prepared};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

type Slot<T> = Arc<Mutex<Option<Result<T, String>>>>;

/// What a background download ends with.
enum Step {
    Done(Installed),
    /// The download offers several variants: the user picks.
    Choose(Prepared),
}

/// A download or install running in the background.
struct Job {
    name: String,
    /// The GameBanana mod it is for, so that mod's buttons wait for it.
    gb: Option<u64>,
    slot: Slot<Step>,
    /// Files of the same mod (one family) ticked with this one, started
    /// one after another once it has ended (`start_next`).
    then: Later,
}

/// Files waiting for an install of the same mod to end: the name each
/// install goes by, its mod and file.
type Later = Vec<(String, GbMod, GbFile)>;

/// A question for the user. They come one at a time, oldest first, so
/// answering one never throws another away.
enum Ask {
    /// Which of a GameBanana mod's files to download: any number, each
    /// installed by its own job into its own folder. `installed` marks the
    /// files whose family is installed already.
    File { m: GbMod, files: Vec<GbFile>, all: bool, picked: Vec<bool>, installed: Vec<bool> },
    /// Which variants of a download to install; `then` waits for the install.
    Variant { name: String, p: Prepared, picked: Vec<bool>, then: Later },
}

impl Ask {
    fn gb(&self) -> Option<u64> {
        match self {
            Ask::File { m, .. } => Some(m.id),
            Ask::Variant { p, .. } => p.info.gamebanana_id,
        }
    }
}

#[derive(Default)]
pub struct Ui {
    link: String,
    browse: Vec<GbMod>,
    /// The GameBanana mod whose Install again was pressed once: the next
    /// press installs a second copy.
    again: Option<Again>,
    /// The egui pass the page was last drawn in, to tell when it was left.
    drawn: u64,
    more: bool,
    page: u32,
    listing: Option<Slot<(Vec<GbMod>, bool)>>,
    /// GameBanana mods whose file lists are being read.
    fetching: Vec<(u64, Slot<(GbMod, Vec<GbFile>)>)>,
    /// The GameBanana pages read so far, by mod id: a row whose page has
    /// several current files offers Install another file. The list itself
    /// carries no files.
    pages: HashMap<u64, (GbMod, Vec<GbFile>)>,
    jobs: Vec<Job>,
    asks: VecDeque<Ask>,
    /// The name typed into the save-as-profile box.
    profile_name: String,
    /// The page has been shown once: old downloads cleaned.
    started: bool,
    /// The installed list and its conflict scan, kept briefly. Both walk every
    /// mod tree; recomputing them on every scroll frame cost more than drawing
    /// the page.
    scan: Option<Scan>,
}

/// A pending Install again: which mod, and how much background work had
/// started when it was pressed (`STARTED`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Again {
    id: u64,
    started: u64,
}

/// Background work started so far (`spawn`): reading a page or the list,
/// downloading, installing.
static STARTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct Scan {
    at: std::time::Instant,
    installed: Vec<mods::Mod>,
    conflicts: Vec<mods::Conflict>,
    /// Code mods switched on together that ship a DLL of the same name.
    code_clashes: Vec<mods::Conflict>,
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
        !self.asks.is_empty()
    }

    /// Something is running or waiting for this GameBanana mod.
    fn working_on(&self, id: u64) -> bool {
        self.fetching.iter().any(|(i, _)| *i == id) || self.jobs.iter().any(|j| j.gb == Some(id)) || self.asks.iter().any(|a| a.gb() == Some(id))
    }

    /// Forget a pending Install again once the rows may have moved under it:
    /// the page was left (`pass` does not follow the last one drawn), work
    /// started since (`started`: a fetch, an install, a list reload), or that
    /// mod is no longer installed.
    fn forget_again(&mut self, pass: u64, started: u64, installed: &[Mod]) {
        let left = pass > self.drawn + 1;
        self.drawn = pass;
        if self.again.is_some_and(|a| left || a.started != started || !installed.iter().any(|m| m.info.gamebanana_id == Some(a.id))) {
            self.again = None;
        }
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
        let code_clashes = mods::code_clashes(&installed);
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
            code_clashes,
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
    STARTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let slot: Slot<T> = Arc::new(Mutex::new(None));
    let s = slot.clone();
    std::thread::spawn(move || {
        // A panic still ends the job, so its mod's buttons do not wait forever.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err("internal error; a crash report was saved, and the launcher keeps running".into()));
        *s.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
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
        Ok(Installed { off_for: Some(on), ref mods, needs_build, .. }) => {
            let build = if needs_build { " It adds hook sites to the game's code: press Build the game before playing it." } else { "" };
            (
                format!("Installed {} switched off: {on} is already on, and two copies switched on would both load.{build}", mods.join(", ")),
                Tone::Warn,
            )
        }
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

fn start_install(ui: &mut Ui, name: String, m: GbMod, f: GbFile, dir: PathBuf, then: Later) {
    let id = m.id;
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
    ui.jobs.push(Job { name, gb: Some(id), slot, then });
}

/// Start the first of the files that waited for an install of the same mod,
/// the rest waiting for it in turn: each comes in after the one before, so
/// the one that fits this PC build is on and the others, second copies, off
/// (`mods::finish`), whichever download is quicker.
fn start_next(ui: &mut Ui, mut then: Later, dir: &std::path::Path) {
    if !then.is_empty() {
        let (name, m, f) = then.remove(0);
        start_install(ui, name, m, f, dir.to_path_buf(), then);
    }
}

/// The families (`mods::family`) of a GameBanana page's files installed
/// already. A copy whose file is not recorded tells nothing.
fn installed_families(installed: &[Mod], id: u64) -> Vec<String> {
    let mut fams: Vec<String> = installed.iter().filter(|m| m.info.gamebanana_id == Some(id)).map(|m| mods::family(&m.info.file_name)).filter(|f| !f.is_empty()).collect();
    fams.sort();
    fams.dedup();
    fams
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

/// The file question for a mod's page: the recommended file ticked, unless
/// its family is installed already; files of an installed family are marked
/// and left unticked (picking one installs a second copy).
fn file_ask(m: GbMod, files: Vec<GbFile>, installed: &[Mod]) -> Ask {
    let fams = installed_families(installed, m.id);
    let marks: Vec<bool> = files.iter().map(|f| fams.contains(&mods::family(&f.name))).collect();
    let rec = mods::pick(&files, "");
    let picked = (0..files.len()).map(|i| Some(i) == rec && !marks[i]).collect();
    let all = !files.iter().any(|f| f.playable());
    Ask::File { m, files, all, picked, installed: marks }
}

/// The ticked files in `install_order`, by family: each family is one job
/// started at once, its first file (the one that fits this PC build) first
/// and the others waiting for it (`Job::then`), so that one comes in
/// switched on and the others as second copies, switched off. Each comes
/// with the name its install goes by (with several, the file's name follows
/// the mod's) and the mod it installs as (`titled`, by the families ticked
/// and `installed`).
fn picked_files(m: &GbMod, files: &[GbFile], picked: &[bool], installed: &[String]) -> Vec<Later> {
    let order = install_order(files, picked);
    let mut fams: Vec<String> = installed.to_vec();
    fams.extend(order.iter().map(|&i| mods::family(&files[i].name)));
    let several = order.len() > 1;
    let mut groups: Vec<Later> = Vec::new();
    let mut last: Option<String> = None;
    for i in order {
        let f = &files[i];
        let fam = mods::family(&f.name);
        let name = if several { format!("{} ({})", m.name, f.name) } else { m.name.clone() };
        let item = (name, titled(m, f, &fams), f.clone());
        match groups.last_mut() {
            Some(g) if last.as_deref() == Some(fam.as_str()) => g.push(item),
            _ => groups.push(vec![item]),
        }
        last = Some(fam);
    }
    groups
}

/// The mod a file installs as: when the page's files installed or ticked
/// span more than one family (a page that holds two mods), the title names
/// the file's family too (`Two Mods - manualstacking`), so their folders
/// tell them apart; one family keeps the plain title.
fn titled(m: &GbMod, f: &GbFile, families: &[String]) -> GbMod {
    let fam = mods::family(&f.name);
    let mut all: Vec<&str> = families.iter().map(String::as_str).filter(|x| !x.is_empty()).collect();
    all.push(&fam);
    all.sort();
    all.dedup();
    let mut m = m.clone();
    if all.len() > 1 && !fam.is_empty() {
        let short = fam.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
        m.name = format!("{} - {short}", m.name);
    }
    m
}

/// Ticked files of one family, by name: only the first comes in switched on,
/// or none when that family is installed (`installed` marks it).
fn same_mod_warning(files: &[GbFile], picked: &[bool], installed: &[bool]) -> Vec<String> {
    let order = install_order(files, picked);
    let mut out = Vec::new();
    let mut at = 0;
    while at < order.len() {
        let fam = mods::family(&files[order[at]].name);
        let end = (at..order.len()).find(|&k| mods::family(&files[order[k]].name) != fam).unwrap_or(order.len());
        if end - at > 1 {
            let names: Vec<&str> = order[at..end].iter().map(|&i| files[i].name.as_str()).collect();
            let (last, head) = names.split_last().expect("two or more");
            if order[at..end].iter().any(|&i| installed.get(i).copied().unwrap_or(false)) {
                out.push(format!("{} and {last} are the same mod as one you have: each comes in as a second copy, switched off while that one is on.", head.join(", ")));
            } else {
                let rest = if names.len() > 2 { "the others come in as second copies" } else { "the other comes in as a second copy" };
                out.push(format!("{} and {last} are the same mod: only one comes in switched on, {}; {rest}, switched off.", head.join(", "), names[0]));
            }
        }
        at = end;
    }
    out
}

/// A GameBanana row's button for a mod already installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    /// Its page (once read) has several current files: the file picker asks.
    Another,
    /// Install again, which asks first, as it installs a second copy.
    Again,
}

impl Row {
    fn of(files: Option<&[GbFile]>) -> Row {
        if files.is_some_and(|f| f.iter().filter(|f| f.playable()).count() > 1) {
            Row::Another
        } else {
            Row::Again
        }
    }

    fn label(self) -> &'static str {
        match self {
            Row::Another => "Install another file",
            Row::Again => "Install again",
        }
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

/// The question at the front of the queue, as a dialog.
fn ask_modal(app: &mut App, ctx: &egui::Context, dir: &std::path::Path) -> Option<(String, Tone)> {
    let p = app.config.theme.palette();
    let waiting = app.mods_ui.asks.len().saturating_sub(1);
    let more = if waiting == 0 { String::new() } else { format!("{waiting} more question{} after this one.", if waiting == 1 { "" } else { "s" }) };
    let mut message = None;
    let mut done = false;
    let mut after: Later = Vec::new();
    match app.mods_ui.asks.front_mut()? {
        Ask::File { m, files, all, picked, installed } => {
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
                                    if installed[i] {
                                        ui.label(RichText::new("(installed)").size(12.5).color(p.muted));
                                    }
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
                    for w in same_mod_warning(files, picked, installed) {
                        ui.label(RichText::new(w).size(12.0).color(p.warn));
                    }
                    if (0..files.len()).any(|i| picked[i] && installed[i]) {
                        ui.label(
                            RichText::new("A file marked (installed) is of a mod you have: it comes in as a second copy, switched off while the other is on.")
                                .size(12.0)
                                .color(p.warn),
                        );
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
                        if !more.is_empty() {
                            ui.label(RichText::new(&more).size(12.0).color(p.muted));
                        }
                    });
                },
            );
            if go && n_picked > 0 {
                done = true;
                // Each family is its own job, side by side; the front is still this question, it goes below.
                let have = app.mods_ui.scan.as_ref().map(|s| installed_families(&s.installed, m.id)).unwrap_or_default();
                for group in picked_files(m, files, picked, &have) {
                    start_next(&mut app.mods_ui, group, dir);
                }
            } else if modal.close {
                done = true;
            }
        }
        Ask::Variant { name, p: prep, picked, then } => {
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
                        if !more.is_empty() {
                            ui.label(RichText::new(&more).size(12.0).color(p.muted));
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
                let gb = prep.info.gamebanana_id;
                let then = std::mem::take(then);
                let slot = spawn(move || {
                    let r = mods::finish(&prep, &picks, &d);
                    prep.discard();
                    r.map(Step::Done)
                });
                app.mods_ui.jobs.push(Job { name, gb, slot, then });
                // The job owns the unpacked copy now: drop the question without discarding it.
                app.mods_ui.asks.pop_front();
                return None;
            }
            // A stray click beside the dialog does not throw the download away.
            if (modal.close && !modal.veil) || done {
                prep.discard();
                done = true;
                // Nothing of it installed: the files of the same mod waiting for it go ahead.
                after = std::mem::take(then);
                message = Some((format!("Nothing installed from {name}"), Tone::Neutral));
            }
        }
    }
    if done {
        app.mods_ui.asks.pop_front();
    }
    start_next(&mut app.mods_ui, after, dir);
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
    let clashes = app
        .mods_ui
        .scan
        .as_ref()
        .map(|scan| scan.code_clashes.clone())
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
    app.mods_ui.forget_again(ui.ctx().cumulative_pass_nr(), STARTED.load(std::sync::atomic::Ordering::Relaxed), &installed);
    if !app.mods_ui.started {
        app.mods_ui.started = true;
        // It walks folders: never on the UI thread.
        std::thread::spawn(mods::clean_incoming);
    }
    let busy = !app.mods_ui.jobs.is_empty() || !app.mods_ui.fetching.is_empty() || app.mods_ui.listing.is_some();
    if busy {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }

    // Results from background work.
    if let Some(r) = take(&app.mods_ui.listing) {
        app.mods_ui.listing = None;
        // The rows change: a pending Install again may point at another one now.
        app.mods_ui.again = None;
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
    let mut fetched = Vec::new();
    app.mods_ui.fetching.retain(|(_, slot)| match slot.lock().unwrap().take() {
        Some(r) => {
            fetched.push(r);
            false
        }
        None => true,
    });
    for r in fetched {
        if let Ok((m, files)) = &r {
            app.mods_ui.pages.insert(m.id, (m.clone(), files.clone()));
        }
        match r {
            Ok((m, files)) if files.is_empty() => messages.push((format!("{} has no files to download", m.name), Tone::Warn)),
            Ok((m, files)) => {
                let playable: Vec<&GbFile> = files.iter().filter(|f| f.playable()).collect();
                if playable.len() == 1 && playable[0].fit() >= 0 {
                    let f = playable[0].clone();
                    let mm = titled(&m, &f, &installed_families(&installed, m.id));
                    start_install(&mut app.mods_ui, m.name.clone(), mm, f, dir.clone(), Vec::new());
                } else {
                    app.mods_ui.asks.push_back(file_ask(m, files, &installed));
                }
            }
            Err(e) => messages.push((format!("GameBanana: {e}"), Tone::Bad)),
        }
    }
    let mut finished = Vec::new();
    app.mods_ui.jobs.retain_mut(|j| match j.slot.lock().unwrap().take() {
        Some(r) => {
            finished.push((j.name.clone(), r, std::mem::take(&mut j.then)));
            false
        }
        None => true,
    });
    if !finished.is_empty() {
        // An install changed the mods folder, even one that failed partway.
        app.mods_ui.invalidate_scan();
    }
    for (name, r, then) in finished {
        match r {
            Ok(Step::Done(i)) => {
                messages.push(installed_message(&name, Ok(i)));
                start_next(&mut app.mods_ui, then, &dir);
            }
            Ok(Step::Choose(prep)) => {
                let rec = prep.recommended();
                let picked = (0..prep.choices.len()).map(|i| i == rec).collect();
                mods::keep_stage(&prep);
                // The files waiting for it wait for the variant question too.
                app.mods_ui.asks.push_back(Ask::Variant { name, p: prep, picked, then });
            }
            Err(e) => {
                messages.push(installed_message(&name, Err(e)));
                start_next(&mut app.mods_ui, then, &dir);
            }
        }
    }
    let ctx = ui.ctx().clone();
    if let Some(m) = ask_modal(app, &ctx, &dir) {
        messages.push(m);
    }

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
        if !clashes.is_empty() {
            widgets::status_line(
                ui,
                &p,
                Tone::Warn,
                &format!("{} code DLL{} loaded twice", clashes.len(), if clashes.len() == 1 { "" } else { "s" }),
                "These switched-on code mods ship a DLL of the same name, most likely two copies of one mod, and both load. Switch one off.",
            );
            egui::Grid::new("code-clash-grid").num_columns(2).spacing([20.0, 6.0]).striped(true).show(ui, |ui| {
                for c in &clashes {
                    ui.label(RichText::new(&c.file).font(theme::mono(11.5)).color(p.body));
                    ui.label(RichText::new(c.mods.join("  ·  ")).size(12.0).color(p.warn));
                    ui.end_row();
                }
            });
            ui.add_space(theme::SPACE_S);
        }
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
            if widgets::button(ui, &p, "From archive…", true).clicked() {
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
                    app.mods_ui.jobs.push(Job { name, gb: None, slot, then: Vec::new() });
                }
            }
            if widgets::button(ui, &p, "From folder…", true).clicked() {
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
                    app.mods_ui.jobs.push(Job { name, gb: None, slot, then: Vec::new() });
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
            if widgets::button(ui, &p, "Install from GameBanana", id.is_some_and(|i| !app.mods_ui.working_on(i))).clicked() {
                let id = id.unwrap();
                app.mods_ui.fetching.push((id, spawn(move || mods::gb_mod(id))));
                app.mods_ui.link.clear();
            }
        });
        let fetching = app.mods_ui.fetching.len();
        if fetching > 0 {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Reading {fetching} mod page{} on GameBanana…", if fetching == 1 { "" } else { "s" }));
            });
        }
        for j in &app.mods_ui.jobs {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Installing {}…", j.name));
            });
        }
        if !app.mods_ui.asks.is_empty() {
            ui.label(RichText::new(format!("{} waiting for your choice.", app.mods_ui.asks.len())).size(12.5).color(p.warn));
        }
    });

    widgets::section(ui, &p, "GameBanana", |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Fire Emblem: Fortune's Weave mods, newest first.").color(p.body));
            if widgets::button(ui, &p, if app.mods_ui.browse.is_empty() { "Load list" } else { "Refresh" }, app.mods_ui.listing.is_none()).clicked() {
                app.mods_ui.page = 1;
                // The file lists read before may be out of date: read them again when needed.
                app.mods_ui.pages.clear();
                app.mods_ui.listing = Some(spawn(|| mods::gb_list(1)));
            }
            ui.hyperlink_to("Open on gamebanana.com", format!("https://gamebanana.com/mods/games/{}", mods::GAMEBANANA_GAME));
        });
        if app.mods_ui.listing.is_some() {
            ui.spinner();
        }
        let mut fetch: Option<u64> = None;
        let mut another: Option<u64> = None;
        let mut again = app.mods_ui.again;
        egui::Grid::new("gb").num_columns(4).spacing([14.0, 6.0]).striped(true).show(ui, |ui| {
            for m in &app.mods_ui.browse {
                ui.vertical(|ui| {
                    ui.set_min_width(460.0);
                    ui.hyperlink_to(RichText::new(&m.name).font(theme::semibold(13.5)), &m.url);
                    ui.label(RichText::new(format!("{} by {}{}", m.kind, m.author, if m.version.is_empty() { String::new() } else { format!(", {}", m.version) })).size(12.0).color(p.muted));
                });
                ui.label(RichText::new(format!("♥ {}", m.likes)).color(p.muted));
                let mine: Vec<&Mod> = installed.iter().filter(|x| x.info.gamebanana_id == Some(m.id)).collect();
                let working = app.mods_ui.working_on(m.id);
                if !mine.is_empty() {
                    widgets::pill(ui, &p, "Installed", Tone::Ok);
                } else {
                    ui.label("");
                }
                if working {
                    ui.spinner();
                } else if m.kind == "Mod" {
                    let row = Row::of(app.mods_ui.pages.get(&m.id).map(|(_, f)| f.as_slice()));
                    if mine.is_empty() {
                        if widgets::button(ui, &p, "Install", true).clicked() {
                            fetch = Some(m.id);
                        }
                    } else if row == Row::Another {
                        // Its page has several current files: the file picker asks, marking the installed ones.
                        if widgets::button(ui, &p, row.label(), true)
                            .on_hover_text("Choose more files of this mod's page; the ones you have are marked (installed)")
                            .clicked()
                        {
                            another = Some(m.id);
                            if again.is_some_and(|a| a.id == m.id) {
                                again = None;
                            }
                        }
                    } else if again.is_some_and(|a| a.id == m.id) {
                        // Asked first: the rows move when an install ends, and one click
                        // meant for another row must not install a second copy.
                        ui.horizontal(|ui| {
                            if widgets::primary_button(ui, &p, "Install a second copy?", true)
                                .on_hover_text("Installs another copy or another file of this mod beside the one you have. A copy of a mod that is on comes in switched off.")
                                .clicked()
                            {
                                fetch = Some(m.id);
                                again = None;
                            }
                            if widgets::button(ui, &p, "No", true).clicked() {
                                again = None;
                            }
                        });
                    } else if widgets::button(ui, &p, row.label(), true)
                        .on_hover_text("Installs another copy or another file of this mod beside the one you have; asks first")
                        .clicked()
                    {
                        again = Some(Again { id: m.id, started: STARTED.load(std::sync::atomic::Ordering::Relaxed) });
                    }
                } else {
                    ui.label(RichText::new("a tool, not a mod").size(12.0).color(p.muted));
                }
                ui.end_row();
            }
        });
        app.mods_ui.again = again;
        if let Some((m, files)) = another.and_then(|id| app.mods_ui.pages.get(&id).cloned()) {
            app.mods_ui.asks.push_back(file_ask(m, files, &installed));
        }
        if let Some(id) = fetch {
            app.mods_ui.fetching.push((id, spawn(move || mods::gb_mod(id))));
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

    fn installed(id: u64) -> Mod {
        Mod {
            path: PathBuf::from("x"),
            name: "x".into(),
            enabled: true,
            info: mods::Info { title: "x".into(), gamebanana_id: Some(id), ..mods::Info::default() },
            files: 1,
            has_romfs: true,
            has_exefs: false,
            has_code: false,
            has_patches: false,
        }
    }

    /// A pending Install again lasts only while nothing moved under it.
    #[test]
    fn install_again_is_forgotten_when_things_change() {
        let mine = vec![installed(7)];
        let mut ui = Ui { drawn: 10, ..Ui::default() };
        ui.again = Some(Again { id: 7, started: 3 });
        ui.forget_again(11, 3, &mine);
        assert_eq!(ui.again, Some(Again { id: 7, started: 3 }), "drawn again, nothing changed");
        // Something started in the background.
        ui.forget_again(12, 4, &mine);
        assert_eq!(ui.again, None);
        // The page was left and came back.
        ui.again = Some(Again { id: 7, started: 4 });
        ui.forget_again(20, 4, &mine);
        assert_eq!(ui.again, None);
        // That mod was removed.
        ui.again = Some(Again { id: 7, started: 4 });
        ui.forget_again(21, 4, &[installed(8)]);
        assert_eq!(ui.again, None);
    }

    fn gfile(id: u64, name: &str, desc: &str, date: u64) -> GbFile {
        GbFile { id, name: name.into(), description: desc.into(), date, ..GbFile::default() }
    }

    fn gbmod(id: u64, name: &str) -> GbMod {
        GbMod { id, kind: "Mod".into(), name: name.into(), author: String::new(), version: String::new(), likes: 0, url: String::new() }
    }

    fn with_file(id: u64, file: &str) -> Mod {
        let mut m = installed(id);
        m.info.file_name = file.into();
        m
    }

    /// The file picker ticks the recommended file; a file whose family is
    /// installed is marked and left unticked. Each ticked file is its own
    /// install, named by its file when there are several.
    #[test]
    fn the_file_picker_defaults_and_its_picks() {
        let m = gbmod(9, "Two Mods");
        let files = vec![gfile(1, "pleasenomoreclicking-v210-main-mod.zip", "MAIN MOD", 500), gfile(2, "manualstacking-v120-main-mod.zip", "MAIN MOD", 600), gfile(3, "manualstacking-v100.zip", "old", 100)];
        let Ask::File { picked, installed: marks, .. } = file_ask(m.clone(), files.clone(), &[]) else { panic!() };
        assert_eq!(picked, vec![false, true, false], "the recommended (newest) file is ticked");
        assert_eq!(marks, vec![false, false, false]);
        // Its family is installed: marked, not ticked. Another page's copy changes nothing.
        let have = [with_file(9, "manualstacking-v110-main-mod.zip"), with_file(4, "pleasenomoreclicking-v200-main-mod.zip")];
        let Ask::File { picked, installed: marks, .. } = file_ask(m.clone(), files.clone(), &have) else { panic!() };
        assert_eq!(marks, vec![false, true, true]);
        assert_eq!(picked, vec![false, false, false]);
        // A copy whose file is not recorded marks nothing.
        let Ask::File { picked, installed: marks, .. } = file_ask(m.clone(), files.clone(), &[installed(9)]) else { panic!() };
        assert_eq!((picked, marks), (vec![false, true, false], vec![false; 3]));
        let groups = picked_files(&m, &files, &[true, true, false], &[]);
        let names: Vec<(String, String, u64)> = groups.iter().flatten().map(|(n, mm, f)| (n.clone(), mm.name.clone(), f.id)).collect();
        assert_eq!(
            names,
            vec![
                ("Two Mods (pleasenomoreclicking-v210-main-mod.zip)".to_string(), "Two Mods - pleasenomoreclicking".to_string(), 1),
                ("Two Mods (manualstacking-v120-main-mod.zip)".to_string(), "Two Mods - manualstacking".to_string(), 2)
            ],
            "two families: two jobs, each folder named by its family"
        );
        assert_eq!(groups.len(), 2);
        assert!(same_mod_warning(&files, &[true, true, false], &[false; 3]).is_empty(), "different mods: no warning");
        let one = picked_files(&m, &files, &[false, true, false], &[]);
        assert_eq!((one[0][0].0.as_str(), one[0][0].1.name.as_str()), ("Two Mods", "Two Mods"), "one family: the plain title");
        // Another family of the page installed already: the folder names this one's.
        let one = picked_files(&m, &files, &[false, true, false], &["pleasenomoreclicking".to_string()]);
        assert_eq!(one[0][0].1.name, "Two Mods - manualstacking");
        assert_eq!(titled(&m, &files[1], &["manualstacking".to_string()]).name, "Two Mods", "its own family only");
    }

    /// Two ticked files of one mod (one family): the one that fits this PC
    /// build starts, the other waits for it whatever their page order, so the
    /// recommended one comes in on and the other, a second copy, off; the
    /// picker says so.
    #[test]
    fn files_of_one_mod_wait_for_the_recommended_one() {
        let m = gbmod(742827, "Turn Control");
        // The emulator build first on the page: started side by side it could finish first.
        let files = vec![gfile(2, "turncontrol-030-emulator.zip", "atmosphere for emus", 200), gfile(1, "turncontrol-030-emblemweave.zip", "for recomps", 200)];
        let warn = same_mod_warning(&files, &[true, true], &[false, false]);
        assert_eq!(warn, vec!["turncontrol-030-emblemweave.zip and turncontrol-030-emulator.zip are the same mod: only one comes in switched on, turncontrol-030-emblemweave.zip; the other comes in as a second copy, switched off.".to_string()]);
        assert!(same_mod_warning(&files, &[true, false], &[false, false]).is_empty());
        // That mod installed already: both come in off.
        let warn = same_mod_warning(&files, &[true, true], &[true, true]);
        assert_eq!(warn, vec!["turncontrol-030-emblemweave.zip and turncontrol-030-emulator.zip are the same mod as one you have: each comes in as a second copy, switched off while that one is on.".to_string()]);
        let groups = picked_files(&m, &files, &[true, true], &[]);
        assert_eq!(groups.len(), 1, "one family: one job, the other file waits for it");
        assert_eq!(groups[0].iter().map(|(_, _, f)| f.id).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(groups[0][1].1.name, "Turn Control", "one family: the plain title");
        // Started: one job, the other file waiting in it.
        let root = std::env::temp_dir().join(format!("ew-pages-wait-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mods_dir = root.join("mods");
        let mut ui = Ui::default();
        start_next(&mut ui, groups[0].clone(), &mods_dir);
        assert_eq!(ui.jobs.len(), 1);
        assert_eq!(ui.jobs[0].then.iter().map(|(_, _, f)| f.id).collect::<Vec<_>>(), vec![2]);
        // Installed in that order: the recommended one on, the other off.
        for (n, (_, mm, f)) in groups[0].iter().enumerate() {
            let d = root.join(format!("dl{n}"));
            let dll = d.join("TurnControl/exefs").join(format!("turn_control_{n}.dll"));
            std::fs::create_dir_all(dll.parent().unwrap()).unwrap();
            std::fs::write(&dll, b"x").unwrap();
            let info = mods::Info { title: mm.name.clone(), gamebanana_id: Some(m.id), file_name: f.name.clone(), ..mods::Info::default() };
            mods::finish(&mods::prepare(&d, info).unwrap(), &[0], &mods_dir).unwrap();
        }
        let mut l: Vec<(String, bool, String)> = mods::list(&mods_dir).into_iter().map(|x| (x.name, x.enabled, x.info.file_name)).collect();
        l.sort();
        assert_eq!(
            l,
            vec![
                ("Turn Control".to_string(), true, "turncontrol-030-emblemweave.zip".to_string()),
                ("Turn Control (2)".to_string(), false, "turncontrol-030-emulator.zip".to_string())
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Install another file only once the page is known to hold several
    /// current files; old versions and source code do not count.
    #[test]
    fn install_another_file_needs_several_current_files() {
        assert_eq!(Row::of(None), Row::Again, "the list carries no files: decided in the picker");
        assert_eq!((Row::Again.label(), Row::Another.label()), ("Install again", "Install another file"));
        let mut one = vec![gfile(1, "turncontrol-030.zip", "", 2), gfile(2, "turncontrol-010.zip", "", 1)];
        one[1].archived = true;
        one.push(gfile(3, "turncontrol-030-source-only.zip", "", 3));
        assert_eq!(Row::of(Some(&one)), Row::Again);
        let two = vec![gfile(1, "turncontrol-030-emblemweave.zip", "", 2), gfile(2, "turncontrol-030-emulator.zip", "", 2)];
        assert_eq!(Row::of(Some(&two)), Row::Another);
    }
}
