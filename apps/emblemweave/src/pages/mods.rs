//! Mods: the installed RomFS mods, importing archives and folders, and
//! browsing, installing and updating from GameBanana.

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

/// A download, install or update running in the background.
struct Job {
    name: String,
    /// The GameBanana mod it is for, so that mod's buttons wait for it.
    gb: Option<u64>,
    /// The installed mod it replaces, for an update.
    update: Option<Mod>,
    /// The GameBanana file an update is downloading: one download of a file at a time.
    file: Option<u64>,
    slot: Slot<Step>,
    /// Files of the same mod (one family) ticked with this one, started
    /// one after another once it has ended (`start_next`).
    then: Later,
}

/// Files waiting for an install of the same mod to end: the name each
/// install goes by, its mod and file.
type Later = Vec<(String, GbMod, GbFile)>;

/// Updates that download at once; the rest wait in `Ui::queued`.
const UPDATES_AT_ONCE: usize = 3;

/// A question for the user. They come one at a time, oldest first, so
/// answering one never throws another away.
enum Ask {
    /// Which of a GameBanana mod's files to download: any number, each
    /// installed by its own job into its own folder. `installed` marks the
    /// files whose family is installed already. For an update (`update`)
    /// exactly one, the upload that replaces the installed copy.
    File { m: GbMod, files: Vec<GbFile>, all: bool, picked: Vec<bool>, installed: Vec<bool>, update: Option<Mod> },
    /// Which variants of a download to install; `then` waits for the install.
    Variant { name: String, p: Prepared, picked: Vec<bool>, update: Option<Mod>, then: Later },
}

impl Ask {
    fn gb(&self) -> Option<u64> {
        match self {
            Ask::File { m, .. } => Some(m.id),
            Ask::Variant { p, .. } => p.info.gamebanana_id,
        }
    }
}

type Remote = HashMap<u64, (GbMod, Vec<GbFile>)>;

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
    /// Mods being moved to mods-removed (name, title, where to), off the UI
    /// thread: a held folder is retried for a moment.
    removing: Vec<(String, String, Slot<PathBuf>)>,
    /// Updates waiting for a free download slot (Update all), oldest first.
    queued: VecDeque<(Mod, GbMod, GbFile)>,
    asks: VecDeque<Ask>,
    /// The name typed into the save-as-profile box.
    profile_name: String,
    /// The installed mods' GameBanana pages, from the last update check.
    remote: Remote,
    /// The check running: the ids it reads, what it found, what failed.
    checking: Option<Slot<(Vec<u64>, Remote, Vec<String>)>>,
    checked: Option<std::time::SystemTime>,
    /// Mods the last check could not read.
    check_failed: Vec<String>,
    /// The page has been shown once: old downloads cleaned, updates checked.
    started: bool,
    /// The installed list and its conflict scan, kept briefly. Both walk every
    /// mod tree; recomputing them on every scroll frame cost more than drawing
    /// the page.
    scan: Option<Scan>,
    deep: Option<Deep>,
    /// The periodic rescan running off the UI thread, with the generation it
    /// was started in: an action on the page bumps `scan_gen`, so a rescan
    /// that began before it is thrown away rather than shown. `Err` is a
    /// rescan that panicked.
    rescan: Option<(u64, Slot<(Scan, Deep)>)>,
    scan_gen: u64,
    /// A periodic rescan panicked: no more until the next synchronous scan,
    /// so a scan that always panics does not write a crash report every second.
    rescan_failed: bool,
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
    /// Each mod's settings files (`mods::settings_of`), by folder: its
    /// Settings button opens them. Read with the scan, never per frame.
    settings: HashMap<PathBuf, Vec<String>>,
    /// The game is running (read with the scan, not on every frame): updates
    /// wait, and code mods, whose DLLs it holds, stay as they are.
    running: bool,
}

/// What the scan reads inside each mod's folder, kept while the mod folders
/// stay the same: the rescan every `SCAN_TTL` would otherwise walk every mod
/// for them again.
struct Deep {
    /// Each mod folder with its own and its `exefs` folder's modification
    /// times, sorted: a mod added, removed, switched, replaced or given a new
    /// file there changes it.
    key: Vec<(PathBuf, Option<std::time::SystemTime>, Option<std::time::SystemTime>)>,
    code_clashes: Vec<mods::Conflict>,
    settings: HashMap<PathBuf, Vec<String>>,
}

/// Long enough that scrolling does not trigger a rescan; a folder
/// dropped in from Explorer appears on the next redraw. Actions on the page
/// invalidate the scan immediately.
const SCAN_TTL: std::time::Duration = std::time::Duration::from_secs(30);

impl Ui {
    /// A question is open: the page behind it takes no typing.
    fn asking(&self) -> bool {
        !self.asks.is_empty()
    }

    /// Something is running or waiting for this GameBanana mod.
    fn working_on(&self, id: u64) -> bool {
        self.fetching.iter().any(|(i, _)| *i == id) || self.jobs.iter().any(|j| j.gb == Some(id)) || self.asks.iter().any(|a| a.gb() == Some(id))
    }

    /// The update check's result: what was checked is replaced, so a mod that
    /// failed this time shows no stale update, and the file lists it read
    /// replace the pages read before (`pages`), which may be older.
    fn take_check(&mut self, checked: &[u64], remote: Remote, failed: Vec<String>) {
        for id in checked {
            self.remote.remove(id);
        }
        for (id, page) in &remote {
            self.pages.insert(*id, page.clone());
        }
        self.remote.extend(remote);
        self.check_failed = failed;
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

    /// An update of this mod is running or waiting for an answer. Matched by
    /// name: switching the mod on or off renames its folder.
    fn updating(&self, m: &Mod) -> bool {
        self.jobs.iter().any(|j| j.update.as_ref().is_some_and(|u| u.name.eq_ignore_ascii_case(&m.name)))
            || self.queued.iter().any(|(u, _, _)| u.name.eq_ignore_ascii_case(&m.name))
            || self.asks.iter().any(|a| match a {
                Ask::Variant { update: Some(u), .. } | Ask::File { update: Some(u), .. } => u.name.eq_ignore_ascii_case(&m.name),
                _ => false,
            })
    }

    /// This mod is being moved to mods-removed.
    fn removing(&self, m: &Mod) -> bool {
        self.removing.iter().any(|(n, _, _)| n.eq_ignore_ascii_case(&m.name))
    }

    /// Keep the scan fresh. The first scan, the one after an action on the
    /// page (`invalidate_scan`) and the one on coming back to the page run
    /// here, so the page never draws an empty or out-of-date list; the
    /// periodic rescan every `SCAN_TTL` runs on a thread, because walking
    /// every mod on the UI thread showed as a hitch once a second while
    /// scrolling. `pass` is the frame number, as in `forget_again`.
    fn refresh_scan(&mut self, dir: &std::path::Path, ctx: &egui::Context, pass: u64) {
        // Back from another page: a build, the game starting or a folder
        // changed in Explorer may have changed what the scan says.
        if pass > self.drawn + 1 && self.scan.is_some() {
            self.invalidate_scan();
        }
        if let Some((gen, slot)) = &self.rescan {
            let done = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(r) = done {
                let current = *gen == self.scan_gen;
                self.rescan = None;
                match r {
                    // Started before an action on the page: out of date.
                    _ if !current => {}
                    Ok((scan, deep)) => {
                        self.scan = Some(scan);
                        self.deep = Some(deep);
                    }
                    // Panicked (the crash hook saved a report): keep showing the
                    // last scan, and rescan only after the next action on the
                    // page or the next visit to it.
                    Err(_) => self.rescan_failed = true,
                }
            }
        }
        match &self.scan {
            None => {
                let (scan, deep) = compute_scan(dir, self.deep.take());
                self.scan = Some(scan);
                self.deep = Some(deep);
                self.rescan_failed = false;
            }
            Some(s) if s.at.elapsed() >= SCAN_TTL && self.rescan.is_none() && !self.rescan_failed => {
                // Not `spawn`: that counts as work started (`STARTED`), which
                // would drop a pending Install again every second.
                let slot: Slot<(Scan, Deep)> = Arc::new(Mutex::new(None));
                let out = slot.clone();
                let (dir, deep, ctx) = (dir.to_path_buf(), self.deep.take(), ctx.clone());
                std::thread::spawn(move || {
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute_scan(&dir, deep))).map_err(|_| "the scan panicked".to_string());
                    *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
                    ctx.request_repaint();
                });
                self.rescan = Some((self.scan_gen, slot));
            }
            _ => {}
        }
    }

    fn invalidate_scan(&mut self) {
        self.scan = None;
        self.deep = None;
        self.scan_gen += 1;
    }
}

/// One scan of the mods folder: the installed list, its conflicts, the code
/// mods' hook-site state and each mod's settings files. `deep` is the last
/// scan's per-folder part, reused while the mod folders are unchanged.
fn compute_scan(dir: &std::path::Path, deep: Option<Deep>) -> (Scan, Deep) {
    let installed = mods::list(dir);
    let conflicts = mods::conflicts(&installed);
    let modified = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let mut key: Vec<_> = installed.iter().map(|m| (m.path.clone(), modified(&m.path), modified(&m.path.join("exefs")))).collect();
    key.sort();
    let deep = match deep {
        Some(d) if d.key == key => d,
        _ => {
            let code_clashes = mods::code_clashes(&installed);
            let settings = installed
                .iter()
                .map(|m| (m.path.clone(), mods::settings_of(&m.path)))
                .filter(|(_, files)| !files.is_empty())
                .collect();
            Deep { key, code_clashes, settings }
        }
    };
    let (code_clashes, settings) = (deep.code_clashes.clone(), deep.settings.clone());
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
    let scan = Scan {
        at: std::time::Instant::now(),
        installed,
        conflicts,
        code_clashes,
        site_problem,
        site_stale,
        missing_code,
        settings,
        running: crate::platform::game_running(),
    };
    (scan, deep)
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
        Ok(Installed { replaced: Some((old, kept)), ref mods, needs_build, ref merged, ref new_beside, ref dropped, .. }) => {
            let i = Installed { mods: mods.clone(), exefs_only: false, patch_only: false, needs_build, parts: false, replaced: None, merged: Vec::new(), new_beside: Vec::new(), dropped: Vec::new(), off_for: None };
            let mut kept = if kept == 0 { String::new() } else { format!(" Your settings were kept ({kept} file{}).", if kept == 1 { "" } else { "s" }) };
            for (file, added) in merged {
                kept.push_str(&format!(
                    " {file}: {} new entr{} added ({}).",
                    added.len(),
                    if added.len() == 1 { "y" } else { "ies" },
                    added.join(", ")
                ));
            }
            for file in new_beside {
                let name = file.rsplit('/').next().unwrap_or(file);
                kept.push_str(&format!(" {file}: the new version's default is beside it as {name}.new."));
            }
            if !dropped.is_empty() {
                kept.push_str(&format!(
                    " {} no longer ships {}; your cop{} kept in the mod folder.",
                    mods.join(", "),
                    dropped.join(", "),
                    if dropped.len() == 1 { "y was" } else { "ies were" }
                ));
            }
            let build = if i.needs_build { " It brings hook sites: press Build the game before playing." } else { "" };
            (
                format!("Updated {}.{kept}{build} The old copy is in {}.", i.mods.join(", "), old.display()),
                if i.needs_build { Tone::Warn } else { Tone::Ok },
            )
        }
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

/// A file inside a mod, from its `/`-separated path: Explorer needs backslashes.
fn mod_file(m: &Mod, rel: &str) -> PathBuf {
    rel.split('/').fold(m.path.clone(), |p, part| p.join(part))
}

/// Explorer opening `path`: a file in the app Windows uses for it, a folder in
/// a window. Explorer reads its own command line and splits a path at a comma
/// unless the whole path is quoted, which `arg` only does for one with a
/// space, so the path goes in quoted as it is.
fn explorer(path: &std::path::Path) -> std::process::Command {
    let mut c = std::process::Command::new("explorer");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.raw_arg(format!("\"{}\"", path.display()));
    }
    #[cfg(not(windows))]
    c.arg(path);
    c
}

/// The Settings button of a mod with settings files: one file opens in the
/// app Windows has for it; with several, or one with the new version's
/// default beside it (`<file>.new`), a menu lists them and the folder. The
/// launcher never edits them itself.
fn settings_button(ui: &mut egui::Ui, p: &theme::Palette, m: &Mod, files: &[String]) {
    if let [only] = files {
        if widgets::button(ui, p, "Settings", true).on_hover_text(format!("Opens {only} in the app Windows uses for it")).clicked() {
            let _ = explorer(&mod_file(m, only)).spawn();
        }
        return;
    }
    let id = ui.make_persistent_id(("mod-settings", &m.path));
    let b = widgets::button(ui, p, "Settings", true).on_hover_text("This mod's settings files");
    if b.clicked() {
        ui.memory_mut(|mem| mem.toggle_popup(id));
    }
    egui::popup_below_widget(ui, id, &b, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        ui.set_min_width(200.0);
        // One line per path, however long.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        for f in files {
            // `settings_of` puts a new default right under its file.
            let new_default = f.strip_suffix(".new").is_some_and(|base| files.iter().any(|x| x == base));
            let (label, tip) = if new_default {
                (format!("{f} (new default)"), "The new version's default, put beside your file by an update: opens it in the app Windows uses for it")
            } else {
                (f.clone(), "Opens it in the app Windows uses for it")
            };
            if ui.button(RichText::new(label).font(theme::mono(12.0))).on_hover_text(tip).clicked() {
                let _ = explorer(&mod_file(m, f)).spawn();
            }
        }
        ui.separator();
        if ui.button("Open mod folder").clicked() {
            let _ = explorer(&m.path).spawn();
        }
    });
}

/// A launcher button's hover text. `widgets::button` drawn disabled is still
/// enabled for egui, so `on_disabled_hover_text` never shows on it: the text
/// is picked here by the state the button is drawn in.
fn hover(b: egui::Response, enabled: bool, on: &str, off: &str) -> egui::Response {
    b.on_hover_text(if enabled { on } else { off })
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

/// Read the GameBanana page of every installed mod that came from there.
fn check_updates(ui: &mut Ui, installed: &[Mod]) {
    let mut ids: Vec<(u64, String)> = installed.iter().filter(|m| !m.info.no_updates).filter_map(|m| Some((m.info.gamebanana_id?, m.info.title.clone()))).collect();
    ids.sort();
    ids.dedup_by_key(|x| x.0);
    let checked: Vec<u64> = ids.iter().map(|x| x.0).collect();
    ui.checking = Some(spawn(move || {
        let found = Mutex::new((Remote::new(), Vec::new()));
        // A few at a time: one slow page should not hold up the rest.
        for chunk in ids.chunks(4) {
            std::thread::scope(|s| {
                for (id, title) in chunk {
                    let found = &found;
                    s.spawn(move || {
                        let r = std::panic::catch_unwind(|| mods::gb_mod(*id)).unwrap_or_else(|_| Err("internal error".into()));
                        let mut f = found.lock().unwrap_or_else(|e| e.into_inner());
                        match r {
                            Ok(x) => {
                                f.0.insert(*id, x);
                            }
                            Err(e) => f.1.push((title.clone(), e)),
                        }
                    });
                }
            });
        }
        let (remote, failed) = found.into_inner().unwrap_or_else(|e| e.into_inner());
        // Every mod failed (offline, or GameBanana down): say it once.
        let failed: Vec<String> = if !failed.is_empty() && remote.is_empty() {
            vec!["GameBanana could not be reached".to_string()]
        } else {
            failed.into_iter().map(|(t, e)| format!("{t}: {e}")).collect()
        };
        Ok((checked, remote, failed))
    }));
}

/// The newer upload for an installed mod, from the last check.
fn update_of<'a>(remote: &'a Remote, m: &Mod) -> Option<(&'a GbMod, GbFile)> {
    let (gm, files) = remote.get(&m.info.gamebanana_id?)?;
    mods::update_for(m, files).map(|f| (gm, f))
}

fn update_label(m: &Mod, gm: &GbMod, f: &GbFile) -> String {
    let to = if !f.version.is_empty() { f.version.clone() } else { gm.version.clone() };
    let from = &m.info.version;
    if !to.is_empty() && !from.is_empty() && !to.eq_ignore_ascii_case(from) {
        format!("Update: {from} → {to}")
    } else if !to.is_empty() && from.is_empty() {
        format!("Update: {to}")
    } else {
        format!("Update: new upload {}", day(f.date))
    }
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
    ui.jobs.push(Job { name, gb: Some(id), update: None, file: None, slot, then });
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

/// Update an installed mod: straight to the new file, or ask which file when
/// the page holds more than one mod and the installed file is unknown.
fn request_update(ui: &mut Ui, old: Mod, m: GbMod, f: GbFile) {
    let files = ui.remote.get(&m.id).map(|(_, f)| f.clone()).unwrap_or_default();
    if mods::update_needs_choice(&old, &files) {
        // One file: an update replaces one copy, so nothing is ticked and nothing marked.
        let n = files.len();
        ui.asks.push_back(Ask::File { m, files, all: false, picked: vec![false; n], installed: vec![false; n], update: Some(old) });
    } else {
        start_update(ui, old, m, f);
    }
}

/// Queue an update; `pump_updates` starts it when a download slot is free.
fn start_update(ui: &mut Ui, old: Mod, m: GbMod, f: GbFile) {
    ui.queued.push_back((old, m, f));
}

/// Start queued updates: at most `UPDATES_AT_ONCE` downloads at a time, and
/// one at a time per GameBanana file, so two variants installed from one
/// file do not download it twice at once.
fn pump_updates(ui: &mut Ui) {
    let mut i = 0;
    while i < ui.queued.len() {
        if ui.jobs.iter().filter(|j| j.file.is_some()).count() >= UPDATES_AT_ONCE {
            return;
        }
        let file = ui.queued[i].2.id;
        if ui.jobs.iter().any(|j| j.file == Some(file)) {
            i += 1;
            continue;
        }
        let (old, m, f) = ui.queued.remove(i).expect("in range");
        let (name, id) = (old.info.title.clone(), m.id);
        let target = old.clone();
        let slot = spawn(move || {
            // The game holds its code mods' DLLs: nothing can be replaced while it runs.
            if crate::platform::game_running() {
                return Err("Close the game to update".into());
            }
            let p = mods::gb_prepare(&m, &f)?;
            if p.choices.is_empty() {
                p.discard();
                return Err("the new download has nothing this engine can use".into());
            }
            match p.same_variant(&target.info) {
                Some(i) => {
                    let r = mods::finish_update(&p, i, &target);
                    p.discard();
                    r.map(Step::Done)
                }
                None => Ok(Step::Choose(p)),
            }
        });
        ui.jobs.push(Job { name, gb: Some(id), update: Some(old), file: Some(file), slot, then: Vec::new() });
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
    Ask::File { m, files, all, picked, installed: marks, update: None }
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
        Ask::File { m, files, all, picked, installed, update } => {
            // An update recommends the file that follows the installed one.
            let rec = mods::pick(files, update.as_ref().map(|u| u.info.file_name.as_str()).unwrap_or(""));
            let hidden = files.iter().filter(|f| !f.playable()).count();
            let n_picked = picked.iter().filter(|x| **x).count();
            let mut go = false;
            let mut chosen: Option<GbFile> = None;
            let modal = widgets::modal(
                ctx,
                &p,
                "mods-pick-file",
                &if update.is_some() { format!("Which file updates {}?", m.name) } else { format!("Which files of {}?", m.name) },
                if update.is_some() {
                    "The mod's page offers several downloads. Old versions and source code are hidden; the recommended one fits this PC build."
                } else {
                    "The mod's page offers several downloads. Old versions and source code are hidden; the recommended one fits this PC build. Each file you tick is installed into its own folder."
                },
                640.0,
                |ui| {
                    for (i, f) in files.iter().enumerate() {
                        if !*all && !f.playable() {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            let mut on = picked[i];
                            if update.is_none() && widgets::toggle(ui, &p, &mut on).changed() {
                                picked[i] = on;
                            }
                            ui.vertical(|ui| {
                                if update.is_some() {
                                    ui.set_width((ui.available_width() - 110.0).max(200.0));
                                }
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(&f.name).font(theme::mono(13.0)).color(if f.playable() { p.ink } else { p.muted }));
                                    if installed[i] && update.is_none() {
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
                            // An update replaces one copy: one file, by its own button.
                            if update.is_some() {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let b = if Some(i) == rec { widgets::primary_button(ui, &p, "Update", true) } else { widgets::button(ui, &p, "Update", true) };
                                    if b.clicked() {
                                        chosen = Some(f.clone());
                                    }
                                });
                            }
                        });
                        ui.add_space(6.0);
                    }
                    if hidden > 0 && hidden < files.len() {
                        let label = if *all { "Hide old versions and source code".to_string() } else { format!("Show {hidden} old version{} and source code", if hidden == 1 { "" } else { "s" }) };
                        if ui.link(RichText::new(label).size(12.5)).clicked() {
                            *all = !*all;
                        }
                    }
                    for w in if update.is_none() { same_mod_warning(files, picked, installed) } else { Vec::new() } {
                        ui.label(RichText::new(w).size(12.0).color(p.warn));
                    }
                    if update.is_none() && (0..files.len()).any(|i| picked[i] && installed[i]) {
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
                        if update.is_none() && widgets::primary_button(ui, &p, &label, n_picked > 0).clicked() {
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
            // Cancelled or closed: an update says it did not happen.
            if chosen.is_none() && (done || modal.close) {
                if let Some(u) = update.as_ref() {
                    message = Some((format!("{} was not updated", u.info.title), Tone::Neutral));
                }
            }
            if let (Some(f), Some(old)) = (chosen, update.clone()) {
                let m = m.clone();
                done = true;
                // The front is still this question; it goes below.
                start_update(&mut app.mods_ui, old, m, f);
            } else if go && n_picked > 0 && update.is_none() {
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
        Ask::Variant { name, p: prep, picked, update, then } => {
            let single = update.is_some();
            let mut go = false;
            let title = if single { format!("Which version of {name} to update to?") } else { format!("What to install from {name}?") };
            let builds = prep.platform_builds();
            let sub = if single {
                "The new download holds several versions; pick the one that replaces your copy."
            } else if builds {
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
                                // An update takes exactly one.
                                if single && on {
                                    picked.iter_mut().for_each(|x| *x = false);
                                }
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
                    if !single && n > 1 && prep.exclusive(&ticked) {
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
                        let label = if single { "Update".to_string() } else if n > 1 { format!("Install {n}") } else { "Install".to_string() };
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
                let target = update.clone();
                let name = name.clone();
                let gb = prep.info.gamebanana_id;
                let then = std::mem::take(then);
                let slot = spawn(move || {
                    let r = match &target {
                        Some(old) => mods::finish_update(&prep, picks[0], old),
                        None => mods::finish(&prep, &picks, &d),
                    };
                    prep.discard();
                    r.map(Step::Done)
                });
                app.mods_ui.jobs.push(Job { name, gb, update: update.clone(), file: None, slot, then });
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
                message = Some(if single { (format!("{name} was not updated"), Tone::Neutral) } else { (format!("Nothing installed from {name}"), Tone::Neutral) });
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
    widgets::page_header(ui, &p, "Mods", "RomFS and code mods for the game, from an archive, a folder or GameBanana, kept up to date from GameBanana.");
    let Some(dir) = mods::dir() else { return };
    let mut messages: Vec<(String, Tone)> = Vec::new();
    app.mods_ui.refresh_scan(&dir, ui.ctx(), ui.ctx().cumulative_pass_nr());
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
        if app.config.updates.mods && installed.iter().any(|m| m.info.gamebanana_id.is_some()) {
            check_updates(&mut app.mods_ui, &installed);
        }
    }
    let settings = app
        .mods_ui
        .scan
        .as_ref()
        .map(|scan| scan.settings.clone())
        .unwrap_or_default();
    let running = app.mods_ui.scan.as_ref().is_some_and(|scan| scan.running);
    let busy = !app.mods_ui.jobs.is_empty() || !app.mods_ui.removing.is_empty() || !app.mods_ui.queued.is_empty() || !app.mods_ui.fetching.is_empty() || app.mods_ui.listing.is_some() || app.mods_ui.checking.is_some();
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
            finished.push((j.name.clone(), j.update.clone(), r, std::mem::take(&mut j.then)));
            false
        }
        None => true,
    });
    if !finished.is_empty() {
        // An install or update changed the mods folder, even one that failed partway.
        app.mods_ui.invalidate_scan();
    }
    let mut removed = Vec::new();
    app.mods_ui.removing.retain(|(_, title, slot)| match slot.lock().unwrap().take() {
        Some(r) => {
            removed.push((title.clone(), r));
            false
        }
        None => true,
    });
    if !removed.is_empty() {
        app.mods_ui.invalidate_scan();
    }
    for (title, r) in removed {
        messages.push(match r {
            Ok(to) => (format!("Moved {title} to {}", to.display()), Tone::Ok),
            Err(e) => (e, Tone::Bad),
        });
    }
    for (name, update, r, then) in finished {
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
                app.mods_ui.asks.push_back(Ask::Variant { name, p: prep, picked, update, then });
            }
            Err(e) if update.is_some() => messages.push((format!("Could not update {name}: {e}"), Tone::Bad)),
            Err(e) => {
                messages.push(installed_message(&name, Err(e)));
                start_next(&mut app.mods_ui, then, &dir);
            }
        }
    }
    if let Some(r) = take(&app.mods_ui.checking) {
        app.mods_ui.checking = None;
        app.mods_ui.checked = Some(std::time::SystemTime::now());
        match r {
            Ok((checked, remote, failed)) => app.mods_ui.take_check(&checked, remote, failed),
            Err(e) => app.mods_ui.check_failed = vec![e],
        }
    }
    let ctx = ui.ctx().clone();
    if let Some(m) = ask_modal(app, &ctx, &dir) {
        messages.push(m);
    }
    pump_updates(&mut app.mods_ui);

    widgets::section(ui, &p, "Installed", |ui| {
        let list = &installed;
        if list.is_empty() {
            ui.label(RichText::new("No mods installed.").color(p.muted));
        }
        // Updates from GameBanana: the check, and the button that takes them all.
        let updates: Vec<(Mod, GbMod, GbFile)> = list
            .iter()
            .filter(|m| !app.mods_ui.updating(m))
            .filter_map(|m| update_of(&app.mods_ui.remote, m).map(|(g, f)| (m.clone(), g.clone(), f)))
            .collect();
        let from_gb = list.iter().filter(|m| m.info.gamebanana_id.is_some() && !m.info.no_updates).count();
        if from_gb > 0 {
            ui.horizontal(|ui| {
                if app.mods_ui.checking.is_some() {
                    ui.spinner();
                    ui.label(RichText::new("Checking GameBanana for updates…").size(12.5).color(p.muted));
                } else {
                    let (tone, label) = match updates.len() {
                        0 => (Tone::Ok, "Up to date".to_string()),
                        1 => (Tone::Accent, "1 update".to_string()),
                        n => (Tone::Accent, format!("{n} updates")),
                    };
                    if app.mods_ui.checked.is_some() {
                        widgets::status_line(
                            ui,
                            &p,
                            tone,
                            &label,
                            &format!("{from_gb} mods from GameBanana, checked {}.", crate::pages::saves::ago(app.mods_ui.checked)),
                        );
                    } else {
                        widgets::status_line(ui, &p, Tone::Neutral, "Not checked", &format!("{from_gb} mods from GameBanana. Press Check for updates to ask GameBanana."));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if updates.len() > 1
                        && hover(
                            widgets::primary_button(ui, &p, &format!("Update all ({})", updates.len()), !running),
                            !running,
                            "Updates every mod listed with an update, three downloads at a time",
                            "Close the game to update",
                        )
                        .clicked()
                    {
                        for (m, g, f) in updates.clone() {
                            request_update(&mut app.mods_ui, m, g, f);
                        }
                    }
                    if widgets::button(ui, &p, "Check for updates", app.mods_ui.checking.is_none()).clicked() {
                        check_updates(&mut app.mods_ui, list);
                    }
                });
            });
            ui.horizontal(|ui| {
                let mut auto = app.config.updates.mods;
                if widgets::toggle(ui, &p, &mut auto).changed() {
                    app.config.updates.mods = auto;
                }
                ui.label(RichText::new("Check automatically when this page opens (asks GameBanana once per launcher start)").size(12.5).color(p.muted));
            });
            if !app.mods_ui.check_failed.is_empty() && app.mods_ui.checking.is_none() {
                ui.label(RichText::new(format!("Could not read: {}", app.mods_ui.check_failed.join("; "))).size(12.0).color(p.warn));
            }
            ui.add_space(theme::SPACE_S);
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
        let mut do_update: Option<(Mod, GbMod, GbFile)> = None;
        let mut skip: Option<(Mod, u64)> = None;
        for m in list {
            let update = updates.iter().find(|(u, _, _)| u.path == m.path);
            let updating = app.mods_ui.updating(m);
            let removing = app.mods_ui.removing(m);
            ui.horizontal(|ui| {
                let mut on = m.enabled;
                // The running game holds a switched-on code mod's DLL: its folder cannot be renamed or
                // moved. One switched on now would only load when the game starts again.
                let held = running && m.has_code;
                let toggled = ui
                    .add_enabled_ui(!updating && !removing && !held, |ui| widgets::toggle(ui, &p, &mut on))
                    .inner
                    .on_disabled_hover_text(if updating {
                        "Wait for the update to finish"
                    } else if removing {
                        "It is being moved to mods-removed"
                    } else if m.enabled {
                        "Close the game first: it holds this code mod's DLL"
                    } else {
                        "Close the game first: code mods load when the game starts"
                    })
                    .changed();
                if toggled && !updating && !removing && !held {
                    match mods::set_enabled(m, on) {
                        Ok(_) => app.mods_ui.invalidate_scan(),
                        Err(e) => messages.push((e, Tone::Bad)),
                    }
                }
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&m.info.title).font(theme::semibold(14.0)).color(if m.enabled { p.ink } else { p.muted }));
                        if let Some((_, g, f)) = update {
                            widgets::pill(ui, &p, &update_label(m, g, f), Tone::Accent).on_hover_text(format!("{}\n{}", f.name, file_meta(f)));
                        }
                    });
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
                    if m.info.gamebanana_id.is_some() && m.installed_at > 0 {
                        bits.push(format!("installed {}", day(m.installed_at)));
                    }
                    if m.info.no_updates && m.info.gamebanana_id.is_some() {
                        bits.push("updates not checked".into());
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
                    // A switched-off code mod is not loaded, so it can go while the game runs.
                    let locked = held && m.enabled;
                    let can_remove = !updating && !removing && !locked;
                    if hover(
                        widgets::button(ui, &p, "Remove", can_remove),
                        can_remove,
                        "Moves the mod to the mods-removed folder",
                        if updating {
                            "Wait for the update to finish"
                        } else if removing {
                            "It is being moved to mods-removed"
                        } else {
                            "Close the game first: it holds this code mod's DLL"
                        },
                    )
                    .clicked()
                    {
                        // A held folder is retried for a moment: never on the UI thread.
                        let gone = m.clone();
                        let slot = spawn(move || mods::remove(&gone));
                        app.mods_ui.removing.push((m.name.clone(), m.info.title.clone(), slot));
                    }
                    if widgets::button(ui, &p, "Folder", true).clicked() {
                        let _ = explorer(&m.path).spawn();
                    }
                    if let Some(files) = settings.get(&m.path) {
                        settings_button(ui, &p, m, files);
                    }
                    if !m.info.source.is_empty() && m.info.source.starts_with("http") {
                        ui.hyperlink_to("Page", &m.info.source);
                    }
                    if removing {
                        ui.spinner();
                        ui.label(RichText::new("Removing…").size(12.5).color(p.muted));
                    } else if updating {
                        ui.spinner();
                        ui.label(RichText::new("Updating…").size(12.5).color(p.muted));
                    } else if let Some((_, g, f)) = update {
                        if widgets::button(ui, &p, "Skip", true)
                            .on_hover_text("Don't offer this upload again; a newer one will still be offered")
                            .clicked()
                        {
                            skip = Some((m.clone(), f.date));
                        }
                        if hover(
                            widgets::primary_button(ui, &p, "Update", !running),
                            !running,
                            "Downloads the new version and puts it in this mod's place. Your settings files are kept and the old copy goes to mods-removed.",
                            "Close the game to update",
                        )
                        .clicked()
                        {
                            do_update = Some((m.clone(), g.clone(), f.clone()));
                        }
                    }
                });
            });
            ui.add_space(4.0);
        }
        if let Some((m, g, f)) = do_update {
            request_update(&mut app.mods_ui, m, g, f);
        }
        if let Some((m, date)) = skip {
            let mut info = m.info.clone();
            info.skip_update = date;
            // Keep the install date: an old copy is dated by this file, which is about to change.
            if info.installed == 0 {
                info.installed = m.installed_at;
            }
            match mods::write_info(&m.path, &info) {
                // The skipped upload must stop showing as an update.
                Ok(_) => app.mods_ui.invalidate_scan(),
                Err(e) => messages.push((e, Tone::Bad)),
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Changes apply the next time the game starts. When two mods change the same file, the one later in the list wins. A code mod is a DLL that runs inside the game with the game's own rights: install only mods you trust. Close the game before updating a code mod: Windows keeps a running mod's DLL locked.").size(12.5).color(p.muted));
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
            let wanted = |m: &Mod| want.iter().any(|w| w.eq_ignore_ascii_case(&m.name));
            // A mod being updated or removed keeps its state until that is done, and while the
            // game runs no code mod is switched: it holds a switched-on one's DLL, and one
            // switched on would only load when the game starts again.
            let busy = |m: &Mod| app.mods_ui.updating(m) || app.mods_ui.removing(m);
            let updating = installed.iter().filter(|m| busy(m) && wanted(m) != m.enabled).count();
            let held = installed.iter().filter(|m| !busy(m) && running && m.has_code && wanted(m) != m.enabled).count();
            let free: Vec<Mod> = installed.iter().filter(|m| !busy(m) && !(running && m.has_code)).cloned().collect();
            let (n, failed) = mods::apply_profile(&free, &want);
            app.mods_ui.invalidate_scan();
            let mut left = Vec::new();
            if updating > 0 {
                left.push(format!("{updating} being updated or removed"));
            }
            if held > 0 {
                left.push(format!("{held} code mod{} (close the game first)", if held == 1 { "" } else { "s" }));
            }
            let partial = !left.is_empty() || !failed.is_empty();
            let left = if left.is_empty() { String::new() } else { format!(" ({} left as they are)", left.join(", ")) };
            let name = app.config.profiles[i].name.clone();
            if !partial {
                app.config.profile = name.clone();
                messages.push((format!("Applied {name}: {n} mods changed"), Tone::Ok));
            } else {
                // Only a profile that is wholly on is shown as applied.
                let failed = if failed.is_empty() { String::new() } else { format!("; could not switch: {}", failed.join("; ")) };
                messages.push((format!("{name} applied in part, so it is not marked as applied: {n} mods changed{left}{failed}"), Tone::Warn));
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
                    app.mods_ui.jobs.push(Job { name, gb: None, update: None, file: None, slot, then: Vec::new() });
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
                    app.mods_ui.jobs.push(Job { name, gb: None, update: None, file: None, slot, then: Vec::new() });
                }
            }
            if widgets::button(ui, &p, "Open mods folder", true).clicked() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = explorer(&dir).spawn();
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
                ui.label(if j.update.is_some() { format!("Updating {}…", j.name) } else { format!("Installing {}…", j.name) });
            });
        }
        for (m, _, _) in &app.mods_ui.queued {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(format!("Waiting to update {}…", m.info.title)).color(p.muted));
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
        let mut update: Option<(Mod, GbMod, GbFile)> = None;
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
                let newer = mine.iter().filter(|x| !app.mods_ui.updating(x)).find_map(|x| update_of(&app.mods_ui.remote, x).map(|(g, f)| ((*x).clone(), g.clone(), f)));
                let working = app.mods_ui.working_on(m.id);
                if newer.is_some() {
                    widgets::pill(ui, &p, "Update available", Tone::Accent);
                } else if !mine.is_empty() {
                    widgets::pill(ui, &p, "Installed", Tone::Ok);
                } else {
                    ui.label("");
                }
                if working {
                    ui.spinner();
                } else if let Some(u) = newer {
                    if hover(widgets::primary_button(ui, &p, "Update", !running), !running, "Downloads the new version and puts it in the installed copy's place", "Close the game to update").clicked() {
                        update = Some(u);
                    }
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
        if let Some((m, g, f)) = update {
            request_update(&mut app.mods_ui, m, g, f);
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
            installed_at: 0,
        }
    }

    /// A path with a comma reaches Explorer quoted as a whole.
    #[cfg(windows)]
    #[test]
    fn explorer_gets_the_path_quoted() {
        let c = explorer(std::path::Path::new(r"C:\Mods\Turn Control, v2\exefs\settings.ini"));
        let args: Vec<_> = c.get_args().collect();
        assert_eq!(args, vec![std::ffi::OsStr::new(r#""C:\Mods\Turn Control, v2\exefs\settings.ini""#)]);
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

    /// The update message names the entries a merge added and the defaults put beside the user's files.
    #[test]
    fn an_update_message_lists_merges_and_new_defaults() {
        let i = Installed {
            mods: vec!["Turn Control".into()],
            exefs_only: false,
            patch_only: false,
            needs_build: false,
            parts: false,
            replaced: Some((PathBuf::from("bin/Turn Control (before update)"), 2)),
            merged: vec![("exefs/settings.ini".into(), vec!["Controls/hold_key".into(), "Ui/scale".into()])],
            new_beside: vec!["exefs/config.json".into()],
            dropped: Vec::new(),
            off_for: None,
        };
        let (text, tone) = installed_message("Turn Control", Ok(i));
        assert!(matches!(tone, Tone::Ok));
        assert!(text.starts_with("Updated Turn Control. Your settings were kept (2 files)."), "{text}");
        assert!(text.contains(" exefs/settings.ini: 2 new entries added (Controls/hold_key, Ui/scale)."), "{text}");
        assert!(text.contains(" exefs/config.json: the new version's default is beside it as config.json.new."), "{text}");
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

    /// The file picker of an update stays a single choice: an update
    /// replaces one copy, so nothing is ticked, nothing is marked installed,
    /// and each file has its own Update button.
    #[test]
    fn the_update_file_picker_stays_single() {
        let mut old = with_file(9, "mymod-v1-main-mod.zip");
        old.info.file_date = 100;
        let files = vec![gfile(1, "mymod-v2-main-mod.zip", "", 200), gfile(2, "mymod-v2-optional-addon.zip", "", 200)];
        let gm = gbmod(9, "My Mod");
        let mut ui = Ui::default();
        ui.remote.insert(9, (gm.clone(), files.clone()));
        request_update(&mut ui, old, gm.clone(), files[0].clone());
        assert_eq!(ui.asks.len(), 1, "the launcher can't tell main mod from add-on: it asks");
        let Some(Ask::File { picked, installed: marks, update, .. }) = ui.asks.front() else { panic!() };
        assert!(update.is_some());
        assert!(!picked.contains(&true) && !marks.contains(&true));
        // The installed page's files are known from the check: the row offers Install another file.
        let mut ui = Ui::default();
        ui.pages.insert(9, (gm.clone(), vec![files[0].clone()]));
        ui.take_check(&[9], Remote::from([(9, (gm.clone(), files.clone()))]), Vec::new());
        assert_eq!(Row::of(ui.pages.get(&9).map(|(_, f)| f.as_slice())), Row::Another, "the check's newer list replaces the page read before");
        assert_eq!(ui.remote.get(&9).map(|(_, f)| f.len()), Some(2));
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
