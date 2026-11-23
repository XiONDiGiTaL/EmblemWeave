//! EmblemWeave: the native PC recompilation launcher for Fire Emblem:
//! Fortune's Weave, by XiØN.
//!
//! The launcher is the product interface to the recompilation system: it
//! locates and validates the user's own game data, decodes and recompiles the
//! game's code, runs the runtime, and reports exactly how far it gets. It never
//! claims more than the runtime delivers: Play stays unavailable until the game
//! can present a frame.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cheats;
mod config;
mod cpu;
mod crash;
mod deps;
mod discord;
mod engine;
mod gpu;
#[cfg(windows)]
mod exe_icon;
mod gamedata;
mod i18n;
mod ingame;
mod jobs;
mod logo;
mod manifest;
mod memory;
mod mods;
mod onboarding;
mod pad;
mod pages;
mod patches;
mod perf;
mod performance;
mod platform;
mod portable;
mod portraits;
mod report;
mod savefile;
mod saves;
mod shell;
mod sites;
mod startup_overlay;
mod snapshots;
mod theme;
mod build_versioning;
mod build_versioning_cli;
mod object_store;
mod object_store_cli;
mod pipeline_cache;
mod pipeline_cache_cli;
mod romfs_store;
mod storage_manager;
mod storage_manager_cli;
mod toolchain;
mod toolchain_manager;
mod updater;
mod widgets;

use config::Config;
use eframe::egui::{self};
use gamedata::GameData;
use jobs::{Job, JobKind, JobResult};
use pages::Page;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use widgets::Tone;

/// One line of the activity feed.
pub struct Activity {
    pub at: String,
    pub text: String,
    pub tone: Tone,
}

/// A slot a background thread leaves its result in.
type Slot<T> = std::sync::Arc<std::sync::Mutex<Option<Result<T, String>>>>;

/// What the About page says about updates.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    Ready(updater::Release),
    UpToDate(String),
    Failed(String),
    /// The check worked but writing or swapping the release did not.
    InstallFailed(String),
    Downloading,
}

#[derive(Default)]
pub struct Updating {
    pub state: UpdateState,
    slot: Option<Slot<updater::Release>>,
    install: Option<Slot<std::path::PathBuf>>,
    /// The replacement is unpacked: close so the script can swap the files.
    pub restart: bool,
}

pub struct Toast {
    pub text: String,
    pub tone: Tone,
    pub until: Instant,
    /// The one thing worth doing about what the toast says, when there is one (a failed build can be started again from here).
    pub action: Option<ToastAction>,
}

#[derive(Debug, Clone, Copy)]
pub enum ToastAction {
    BuildAgain,
    InstallUpdate,
}

impl ToastAction {
    fn label(self) -> &'static str {
        match self {
            ToastAction::BuildAgain => "Build again",
            ToastAction::InstallUpdate => "Update now",
        }
    }

    fn translated_label(self) -> String {
        crate::i18n::tr(self.label())
    }
}

/// A question the user must answer before an action runs.
pub enum Confirm {
    ResetBindings,
    SafeGraphics,
    RestoreGraphics,
    CancelJob(JobKind),
    /// The player moved the master volume above 100%. The percentage is applied only after
    /// "I know the risks"; cancelling restores the previous master volume.
    AudioBoost {
        pct: u32,
        previous: f32,
    },
}

/// The master volume to keep after an above-100% warning: the requested boost when accepted,
/// otherwise the value from before the warning opened.
fn audio_boost_master(pct: u32, previous: f32, accepted: bool) -> f32 {
    if accepted {
        (pct as f32 / 100.0).clamp(0.0, 2.0)
    } else {
        previous
    }
}

pub struct App {
    pub config: Config,
    saved: String,
    last_save_check: Instant,
    pub page: Page,
    /// The page the central scroll area was last laid out for, so a page opens
    /// at the top instead of inheriting the scroll position of the last one.
    page_seen: Option<Page>,
    pub game: GameData,
    pub jobs: Vec<Job>,
    pub last_boot: Option<(ew_boot::BootReport, u128)>,
    pub last_pipeline: Option<jobs::PipelineSummary>,
    pub last_job_log: Vec<String>,
    /// The file the last task's log was written to, for Diagnostics.
    pub last_job_log_path: Option<PathBuf>,
    pub activity: Vec<Activity>,
    pub toasts: Vec<Toast>,
    pub confirm: Option<Confirm>,
    pub capture: Option<usize>,
    pub diag_tab: pages::diagnostics::Tab,
    pub diag_filter: String,
    pub lift_limit: usize,
    /// `--boot-test`: start a boot test once game detection finishes.
    pub boot_on_start: bool,
    /// `--build-engine`: build the game with the engine once detection finishes.
    build_on_start: bool,
    /// Set when Play had to write a packed package's files first: the game starts when that finishes.
    play_after_repair: bool,
    /// `--screenshot PATH`: save the window's own rendered frame and close.
    screenshot: Option<(std::path::PathBuf, u32)>,
    applied: theme::Scheme,
    pub saves_ui: pages::saves::Ui,
    pub trouble_ui: pages::troubleshoot::Ui,
    pub mods_ui: pages::mods::Ui,
    pub cheats_ui: pages::cheats::Ui,
    /// The game's `[pad]` settings as the Controls page last read them.
    pub tuning: Option<pad::Tuning>,
    /// The launcher's own frame times.
    pub frames: perf::Frames,
    pub update: Updating,
    pub discord: discord::Presence,
    pub ingame: ingame::Ingame,
    pub log_ui: pages::log::Ui,
    /// A built engine found in an older copy of this launcher, offered on Overview until it is used or declined.
    pub reuse: Option<crate::engine::Existing>,
    /// Background channel for the async reuse offer detection.
    reuse_arc: std::sync::Arc<std::sync::Mutex<Option<crate::engine::Existing>>>,
    /// Cached stale build check result, updated periodically in background.
    stale_build_cache: Option<(Option<String>, &'static str)>,
    /// When the stale build cache was last refreshed.
    stale_build_cached_at: Option<Instant>,
    /// `--tour`: walk through the features by itself, for recordings.
    tour: Option<Instant>,
    tour_step: usize,
    /// The expandable background-task panel in the status strip.
    pub show_activity: bool,
    /// The one Overview panel open from the header or a status button.
    pub overview_popup: Option<crate::pages::overview::Popup>,
    /// Keep the hover preview open while the pointer crosses the gap between
    /// the status-bar hub and the panel. This is transient UI state, never
    /// written to config.
    pub activity_hover_until: Option<Instant>,
    /// Seen page, so opening a page from elsewhere reopens its navigation group.
    nav_last: Option<Page>,
    /// A Play succeeded and the launcher should minimize once this frame is done.
    minimize_after_play: bool,
    /// The always-on-top card shown while the game is booting.
    pub startup: startup_overlay::StartupOverlay,
    /// When the last redraw started, for the frame cap in `update`.
    last_frame: Instant,
}

fn clock() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = secs % 86_400;
    format!("{:02}:{:02}:{:02} UTC", s / 3600, s % 3600 / 60, s % 60)
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> App {
        let (mut config, warning) = Config::load();
        i18n::set_language(config.ui_language);
        theme::configure_fonts(&cc.egui_ctx, config.ui_language);
        // An older release carried the upstream Tauri launcher beside the engine.
        // The updated launcher removes that folder in the background; the engine
        // sources beside it are left untouched.
        std::thread::spawn(crate::engine::remove_legacy_launcher);
        if !config.onboarding.introduced {
            config.onboarding.introduced = true;
            // A configuration with game paths belongs to someone who used the
            // launcher before this guide existed. Do not greet them with
            // first-run setup; Overview can reopen it when they want it.
            if !config.paths.game_root.trim().is_empty() || !config.paths.exefs.trim().is_empty() {
                config.onboarding.completed = true;
            }
        }
        theme::apply(&cc.egui_ctx, config.theme);
        let saved = serde_json::to_string(&config).unwrap_or_default();
        let last_boot = Config::dir()
            .map(|d| d.join("last-boot.json"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .map(|_| ());
        // An older updater moved only the executable out of its download. Finish that release before any background
        // thread reads the engine; a release this version downloaded still has its executable and was already overlaid.
        let repaired = crate::updater::current_exe().and_then(|exe| {
            let install = exe.parent()?.to_path_buf();
            let engine =
                crate::engine::root().unwrap_or_else(|| install.join("engine").join("fefw-recomp"));
            Some(crate::updater::finish_interrupted_update(&install, &engine))
        });
        // The previous build is kept in `backups\`; one an older version wrote beside the launcher moves in here, so
        // the install root never collects `.old` files.
        crate::updater::migrate_legacy_backup();
        // The game's taskbar icon comes from fefw.exe, which a build or the
        // engine can replace: make it ours again at every start, off the render
        // thread because the executable is rewritten on disk.
        std::thread::spawn(|| {
            let _ = crate::engine::brand_game();
        });
        // The dependency report takes a second (vswhere, dotnet, one vcvars
        // run); warm it on demand when the Requirements page opens instead of
        // at startup to avoid blocking the UI thread on heavy detection work.
        // A failed or interrupted update used to leave its download behind in
        // %TEMP%. It is never needed by the running launcher, so clear it at
        // startup; saves, profiles, mods and the engine build live elsewhere.
        // If finishing it above failed, keep the release so the next start can retry.
        let keep_cache = matches!(&repaired, Some(Err(_)));
        std::thread::spawn(move || {
            if !keep_cache {
                let _ = crate::updater::clean_cache();
            }
        });
        // A fresh copy has no build of its own. Find one already on the PC — an older copy beside this one, or a checkout an
        // earlier copy's `engine.txt` names — and offer it before the player is asked to compile the engine again.
        // Run in background to avoid blocking startup on filesystem scans.
        let reuse_arc = std::sync::Arc::new(std::sync::Mutex::new(None::<crate::engine::Existing>));
        let reuse_clone = reuse_arc.clone();
        let config_clone = config.clone();
        std::thread::spawn(move || {
            if let Some(e) = crate::engine::existing_build() {
                if !config_clone.reuse_dismissed.contains(&e.root.display().to_string()) {
                    *reuse_clone.lock().unwrap() = Some(e);
                }
            }
        });
        // The build stamp refresh (checking if fefw.exe is newer than fefw.ini)
        // can run in the background; the Overview page will read the cache.
        std::thread::spawn(|| {
            let _ = crate::engine::refresh_build_stamp();
        });
        let mut app = App {
            stale_build_cache: None,
            stale_build_cached_at: None,
            applied: config.theme,
            config,
            saved,
            last_save_check: Instant::now(),
            page: Page::Overview,
            page_seen: None,
            game: GameData::default(),
            jobs: Vec::new(),
            last_boot: None,
            last_pipeline: None,
            last_job_log: Vec::new(),
            last_job_log_path: None,
            activity: Vec::new(),
            toasts: Vec::new(),
            confirm: None,
            capture: None,
            diag_tab: pages::diagnostics::Tab::Runtime,
            diag_filter: String::new(),
            lift_limit: 4000,
            boot_on_start: false,
            build_on_start: false,
            play_after_repair: false,
            screenshot: None,
            saves_ui: Default::default(),
            trouble_ui: Default::default(),
            mods_ui: Default::default(),
            cheats_ui: Default::default(),
            tuning: None,
            frames: Default::default(),
            update: Default::default(),
            discord: Default::default(),
            ingame: Default::default(),
            log_ui: Default::default(),
            reuse: None, // Will be populated by background thread
            reuse_arc,
            tour: None,
            tour_step: 0,
            show_activity: false,
            overview_popup: None,
            activity_hover_until: None,
            nav_last: None,
            minimize_after_play: false,
            startup: Default::default(),
            last_frame: Instant::now(),
        };
        match repaired {
            Some(Ok(n)) if n > 0 => app.note(
                format!("Finished the previous update: {n} release files written"),
                Tone::Accent,
            ),
            Some(Err(e)) => app.notify(
                format!("The previous update could not finish: {e}"),
                Tone::Warn,
            ),
            _ => {}
        }
        // Command line: `--page <name>` opens a page, `--boot-test` runs the
        // boot test as soon as the game data is detected.
        let args: Vec<String> = std::env::args().collect();
        for (i, a) in args.iter().enumerate() {
            match a.as_str() {
                "--boot-test" => app.boot_on_start = true,
                "--build-engine" => app.build_on_start = true,
                "--tour" => {
                    app.tour = Some(Instant::now());
                    // The recorded tour visits every page, including the ones
                    // Standard mode normally hides. Tour settings are not saved.
                    app.config.interface_mode = crate::config::InterfaceMode::Advanced;
                }
                "--screenshot" => {
                    app.screenshot = args.get(i + 1).map(|p| (std::path::PathBuf::from(p), 0))
                }
                "--page" => {
                    if let Some(name) = args.get(i + 1) {
                        if let Some(p) = Page::ALL.iter().find(|p| {
                            p.label()
                                .eq_ignore_ascii_case(name.replace('-', " ").as_str())
                        }) {
                            app.open(*p);
                        }
                    }
                }
                _ => {}
            }
        }
        let _ = last_boot;
        if app.config.discord.enabled {
            app.discord.start_launcher();
        }
        if let Some(w) = warning {
            app.notify(w, Tone::Warn);
        }
        // A swap script that could not replace the running launcher starts the old one and leaves a note instead of
        // disappearing. Say what happened once, then clear it so a successful start is not followed by it every time.
        if let Some(e) = crate::updater::take_failure() {
            app.notify(e, Tone::Bad);
        }
        app.note("Launcher started", Tone::Neutral);
        // Say this at startup, while the player is still on Overview: a folder from an older release is fixed there, before Play.
        if let Some((built, now)) = crate::engine::stale_build() {
            let built = built.unwrap_or_else(|| "an older release".to_string());
            app.notify(format!("This game folder was built by {built}; Overview has Repair / Rebuild now to update it to {now}."), Tone::Warn);
        }
        app.start(JobKind::Detect);
        if app.config.updates.check {
            app.check_updates();
        }
        app
    }

    pub fn note(&mut self, text: impl Into<String>, tone: Tone) {
        let text = crate::i18n::tr(&text.into());
        // The crash report carries the last few of these.
        crate::crash::note(&text);
        self.activity.push(Activity {
            at: clock(),
            text,
            tone,
        });
        if self.activity.len() > 200 {
            self.activity.remove(0);
        }
    }

    pub fn notify(&mut self, text: impl Into<String>, tone: Tone) {
        let text = text.into();
        self.note(text.clone(), tone);
        self.toasts.push(Toast {
            text,
            tone,
            until: Instant::now() + Duration::from_secs(5),
            action: None,
        });
    }

    /// A toast that carries the action that answers it, and stays long enough to be used.
    pub fn notify_action(&mut self, text: String, tone: Tone, action: ToastAction) {
        self.toasts.push(Toast {
            text,
            tone,
            until: Instant::now() + Duration::from_secs(60),
            action: Some(action),
        });
    }

    pub fn running(&self, kind: JobKind) -> Option<&Job> {
        self.jobs.iter().find(|j| j.kind == kind && !j.finished())
    }

    pub fn busy(&self) -> bool {
        self.jobs
            .iter()
            .any(|j| !j.finished() && j.kind != JobKind::Detect)
    }

    /// Start a job of `kind` unless one is already running.
    pub fn start(&mut self, kind: JobKind) {
        if self.running(kind).is_some() {
            return;
        }
        // The updater is writing the engine sources; a build started now would read a half-updated tree. The update
        // closes and restarts the launcher, after which the usual build action is available again.
        if self.update.state == UpdateState::Downloading {
            self.notify(
                "The launcher is installing an update; it restarts when that finishes",
                Tone::Warn,
            );
            return;
        }
        let job = match kind {
            JobKind::Detect => jobs::detect(&self.config),
            JobKind::Decode => match self.game.exefs.clone() {
                Some(exefs) => jobs::decode(exefs),
                None => return self.notify("Choose the game folder before decoding", Tone::Warn),
            },
            JobKind::Pipeline => jobs::pipeline(self.game.clone(), self.lift_limit),
            JobKind::Boot => jobs::boot(&self.config, &self.game),
            JobKind::Engine => match (
                self.game.exefs.clone(),
                self.game.nup.clone().or_else(|| self.game.romfs.clone()),
            ) {
                (Some(exefs), Some(romfs)) => {
                    let dlc = self.game.dlc.clone();
                    jobs::spawn(JobKind::Engine, &engine::STAGES, move |ctx| {
                        engine::build(ctx, exefs, romfs, dlc)
                    })
                }
                _ => {
                    return self.notify(
                        "Choose the game data on Game Setup before building",
                        Tone::Warn,
                    )
                }
            },
            JobKind::Repair => match self.game.nup.clone().or_else(|| self.game.romfs.clone()) {
                Some(romfs) => jobs::spawn(JobKind::Repair, engine::REPAIR_STAGES, move |ctx| {
                    engine::repair(ctx, romfs)
                }),
                None => {
                    return self.notify(
                        "Choose the game data on Game Setup before checking it",
                        Tone::Warn,
                    )
                }
            },
        };
        if kind != JobKind::Detect {
            self.note(format!("{} started", kind.title()), Tone::Accent);
        }
        self.jobs.push(job);
    }

    /// Play, after preparing the game folder when it needs it: a package that keeps some of its files packed cannot be read in place, so
    /// the same check Game Setup offers runs first, and the game starts on its own the moment that finishes (issue #24).
    pub fn play(&mut self) {
        // A check already on its way (from Game Setup, or an earlier press of Play) is the same work: wait for it rather than starting twice.
        if self.running(JobKind::Repair).is_some() {
            self.play_after_repair = true;
            self.notify(
                "Writing the files this package keeps packed: the game starts when that finishes",
                Tone::Accent,
            );
            return;
        }
        if let Some(romfs) = engine::packed_package_without_overlay() {
            self.play_after_repair = true;
            self.notify("This package keeps some of its files packed: writing them first, then the game starts", Tone::Accent);
            if self.game.nup.is_none() {
                self.game.nup = Some(romfs);
            }
            self.start(JobKind::Repair);
            return;
        }
        match engine::play(&self.config) {
            Ok(()) => {
                self.minimize_after_play = self.config.minimize_on_play;
                self.startup.begin();
                self.notify("Starting Fire Emblem: Fortune's Weave", Tone::Ok);
            }
            Err(e) => self.notify(e, Tone::Bad),
        }
    }

    /// Collect finished jobs and apply their results.
    fn poll_jobs(&mut self) {
        let mut finished = Vec::new();
        self.jobs.retain(|j| {
            if j.finished() {
                finished.push((j.kind, j.progress.clone()));
                false
            } else {
                true
            }
        });
        for (kind, progress) in finished {
            let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
            if p.log_file.is_some() {
                self.last_job_log_path = p.log_file.clone();
            }
            if kind != JobKind::Detect {
                self.last_job_log = std::mem::take(&mut p.log);
            }
            if let Some(err) = p.failed.take() {
                // The log path travels with the failure: a report that says
                // "no log files are generated" is worse than one line of text.
                let where_ = p
                    .log_file
                    .as_ref()
                    .map(|f| format!(" [log: {}]", f.display()))
                    .unwrap_or_default();
                drop(p);
                let text = format!("{}: {err}{where_}", kind.title());
                // A failed build is the one failure with an obvious next step, and the step resets a broken build folder on its own, so the
                // message carries the button that runs it.
                if kind == JobKind::Engine {
                    self.notify_action(text, Tone::Bad, ToastAction::BuildAgain);
                } else {
                    self.notify(text, Tone::Bad);
                }
                // A Play waiting on the check has to be dropped, or it would start the game after a failure.
                if kind == JobKind::Repair {
                    self.play_after_repair = false;
                }
                continue;
            }
            match p.result.take() {
                Some(JobResult::Detected(g)) => {
                    self.game = *g;
                }
                Some(JobResult::Decoded(dir)) => {
                    drop(p);
                    self.config.paths.decoded = dir.to_string_lossy().into_owned();
                    self.notify("Main module decoded and verified", Tone::Ok);
                    self.start(JobKind::Detect);
                }
                Some(JobResult::Pipeline(s)) => {
                    drop(p);
                    let tone = if s.verified.is_some_and(|(ok, n)| ok == n) {
                        Tone::Ok
                    } else {
                        Tone::Warn
                    };
                    let text = match (&s.report, s.verified) {
                        (Some(r), Some((ok, n))) => format!(
                            "Pipeline finished: {} functions, {ok}/{n} verified",
                            r.lifted_functions
                        ),
                        (Some(r), None) => format!(
                            "Pipeline finished: {} functions (native build not verified)",
                            r.lifted_functions
                        ),
                        _ => "Pipeline finished".to_string(),
                    };
                    self.last_pipeline = Some(*s);
                    self.notify(text, tone);
                }
                Some(JobResult::Engine(dir)) => {
                    drop(p);
                    self.notify(
                        format!("The game is built and ready to play ({})", dir.display()),
                        Tone::Ok,
                    );
                }
                Some(JobResult::Repaired(overlay)) => {
                    drop(p);
                    let text = match overlay {
                        Some(dir) => format!("Game folder ready: the packed files are read from {}", dir.display()),
                        None => "Game folder ready: the package is read where it lies, nothing to prepare".to_string(),
                    };
                    self.notify(text, Tone::Ok);
                    if std::mem::take(&mut self.play_after_repair) {
                        self.play();
                    }
                }
                Some(JobResult::Boot(report, ms)) => {
                    drop(p);
                    let text = format!("Runtime stopped: {}", report.stop.describe());
                    self.last_boot = Some((*report, ms));
                    self.notify(text, Tone::Accent);
                }
                None => {}
            }
        }
    }

    /// One step of `--tour`: pages and actions on a fixed timetable, then
    /// the launcher closes. Settings changed on the way are never saved.
    fn tour_frame(&mut self, ctx: &egui::Context) {
        let Some(start) = self.tour else { return };
        ctx.request_repaint_after(Duration::from_millis(30));
        let t = start.elapsed().as_secs_f32();
        let steps: [f32; 13] = [
            0.0, 3.5, 5.0, 9.0, 10.0, 14.0, 17.5, 21.0, 22.0, 23.5, 25.0, 26.5, 29.0,
        ];
        while self.tour_step < steps.len() && t >= steps[self.tour_step] {
            match self.tour_step {
                0 => self.open(Page::Overview),
                1 => self.open(Page::Saves),
                2 => {
                    let auto = Config::save_dir()
                        .map(|d| crate::saves::account_dir(&d).join("000").join("Auto"));
                    if let Some(a) = auto {
                        let _ = pages::saves::open_slot(self, &a);
                        self.saves_ui.scroll_to_editor = true;
                    }
                }
                3 => self.open(Page::Mods),
                4 => pages::mods::load_gamebanana(self),
                5 => self.open(Page::Cheats),
                6 => self.open(Page::Log),
                7 => self.open(Page::Appearance),
                8 => self.config.theme = theme::Scheme::Arcade,
                9 => self.config.theme = theme::Scheme::Fe,
                10 => self.config.theme = theme::Scheme::Xion,
                11 => self.open(Page::About),
                _ => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
            self.tour_step += 1;
        }
    }

    /// Ask GitHub for the newest release.
    pub fn check_updates(&mut self) {
        if self.update.slot.is_some() || self.update.state == UpdateState::Downloading {
            return;
        }
        self.update.state = UpdateState::Checking;
        let slot: Slot<updater::Release> = std::sync::Arc::new(std::sync::Mutex::new(None));
        let handle = slot.clone();
        std::thread::spawn(move || {
            *handle.lock().unwrap() = Some(updater::latest());
        });
        self.update.slot = Some(slot);
    }

    /// Download the release found, write the launcher, docs and engine sources over this install, and swap the launcher.
    ///
    /// A running build or repair is reading the same engine folder the update would replace, so the update waits for it
    /// rather than changing files underneath it. `data`, the built game, the fetched tools and the portable-storage choice
    /// are not part of the release and are preserved by [`updater::install`].
    pub fn install_update(&mut self) {
        let UpdateState::Ready(release) = self.update.state.clone() else {
            return;
        };
        if self
            .jobs
            .iter()
            .any(|j| !j.finished() && j.kind != JobKind::Detect)
        {
            self.notify(
                "Finish the current build or task before updating",
                Tone::Warn,
            );
            return;
        }
        self.update.state = UpdateState::Downloading;
        self.notify(
            format!("Downloading EmblemWeave {}…", release.tag),
            Tone::Accent,
        );
        let engine = crate::engine::root();
        let release_tag = release.tag.clone();
        let slot: Slot<std::path::PathBuf> = std::sync::Arc::new(std::sync::Mutex::new(None));
        let handle = slot.clone();
        std::thread::spawn(move || {
            let dir = std::env::temp_dir().join("emblemweave-update");
            let r = updater::download(&release, &dir).and_then(|exe| {
                updater::install(&exe, engine.as_deref(), true, &release_tag).map(|()| exe)
            });
            *handle.lock().unwrap() = Some(r);
        });
        self.update.install = Some(slot);
    }

    fn poll_updates(&mut self, ctx: &egui::Context) {
        if let Some(slot) = self.update.slot.clone() {
            let done = slot.lock().unwrap().take();
            if let Some(r) = done {
                self.update.slot = None;
                self.update.state = match r {
                    Ok(rel) if updater::is_newer(&rel.tag, updater::version_now()) => {
                        if rel.tag == self.config.updates.skipped {
                            UpdateState::Idle
                        } else {
                            // A release without the Windows zip cannot be installed from here; the card offers its page instead.
                            if rel.asset.is_some() {
                                self.notify_action(
                                    format!("EmblemWeave {} is available", rel.tag),
                                    Tone::Accent,
                                    ToastAction::InstallUpdate,
                                );
                            } else {
                                self.notify(
                                    format!(
                                        "EmblemWeave {} is available from its release page",
                                        rel.tag
                                    ),
                                    Tone::Accent,
                                );
                            }
                            UpdateState::Ready(rel)
                        }
                    }
                    Ok(rel) => UpdateState::UpToDate(rel.tag),
                    Err(e) => UpdateState::Failed(e),
                };
            }
        }
        if let Some(slot) = self.update.install.clone() {
            let done = slot.lock().unwrap().take();
            if let Some(r) = done {
                self.update.install = None;
                match r {
                    Ok(_) => {
                        self.update.restart = true;
                        self.notify(
                            "Installing the new version: the launcher restarts",
                            Tone::Ok,
                        );
                    }
                    Err(e) => {
                        self.notify(format!("The update failed: {e}"), Tone::Bad);
                        self.update.state = UpdateState::InstallFailed(e);
                    }
                }
            }
        }
        if self.update.restart {
            ctx.request_repaint();
        } else if self.update.state == UpdateState::Checking
            || self.update.state == UpdateState::Downloading
        {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn autosave(&mut self) {
        if self.tour.is_some() || self.screenshot.is_some() {
            return;
        }
        if self.last_save_check.elapsed() < Duration::from_millis(400) {
            return;
        }
        self.last_save_check = Instant::now();
        let now = serde_json::to_string(&self.config).unwrap_or_default();
        if now != self.saved {
            let paths_changed = serde_json::to_string(&self.config.paths).ok()
                != serde_json::from_str::<Config>(&self.saved)
                    .ok()
                    .and_then(|c| serde_json::to_string(&c.paths).ok());
            match self.config.save() {
                Ok(()) => self.saved = now,
                Err(e) => {
                    self.saved = now;
                    self.notify(format!("Settings could not be saved: {e}"), Tone::Bad);
                }
            }
            if paths_changed {
                self.start(JobKind::Detect);
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.capture.is_some() {
            return;
        }
        let pages = Page::ALL;
        ctx.input(|i| {
            if i.modifiers.ctrl {
                for (n, key) in [
                    egui::Key::Num1,
                    egui::Key::Num2,
                    egui::Key::Num3,
                    egui::Key::Num4,
                    egui::Key::Num5,
                    egui::Key::Num6,
                    egui::Key::Num7,
                    egui::Key::Num8,
                    egui::Key::Num9,
                ]
                .into_iter()
                .enumerate()
                {
                    if i.key_pressed(key) {
                        if let Some(p) = pages.get(n).filter(|p| p.available(self.config.interface_mode)) {
                            self.open(*p);
                        }
                    }
                }
            }
        });
    }

    fn overlays(&mut self, ctx: &egui::Context) {
        let p = self.config.theme.palette();
        // Toasts, newest at the bottom right.
        let mut clicked: Option<ToastAction> = None;
        self.toasts.retain(|t| t.until > Instant::now());
        if !self.toasts.is_empty() {
            egui::Area::new(egui::Id::new("toasts"))
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -48.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    for t in &self.toasts {
                        egui::Frame::new()
                            .fill(p.surface)
                            .stroke(egui::Stroke::new(
                                1.0_f32,
                                theme::alpha(t.tone.colour(&p), 160),
                            ))
                            .corner_radius(egui::CornerRadius::same(theme::RADIUS_M))
                            .inner_margin(egui::Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                                ui.set_max_width(420.0);
                                ui.label(crate::i18n::text(&t.text).size(13.5).color(p.ink));
                                if let Some(action) = t.action {
                                    ui.add_space(theme::SPACE_XS);
                                    if widgets::button(ui, &p, &action.translated_label(), true).clicked() {
                                        clicked = Some(action);
                                    }
                                }
                            });
                        ui.add_space(6.0);
                    }
                });
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if let Some(action) = clicked {
            match action {
                ToastAction::BuildAgain => {
                    // One press answers the message that is still on screen.
                    self.toasts
                        .retain(|t| !matches!(t.action, Some(ToastAction::BuildAgain)));
                    self.start(JobKind::Engine);
                }
                ToastAction::InstallUpdate => {
                    self.toasts
                        .retain(|t| !matches!(t.action, Some(ToastAction::InstallUpdate)));
                    self.install_update();
                }
            }
        }
        // Confirmation dialog.
        if let Some(confirm) = &self.confirm {
            let (title, body, action) = match confirm {
                Confirm::ResetBindings => ("Reset controls?", "Every binding returns to its default. This cannot be undone.", "Reset"),
                Confirm::SafeGraphics => ("Reset graphics?", "Resolution, window mode, upscaling, sharpening and shader settings return to their defaults. Saves, mods and game files are not touched. This is the first thing to try for a black screen.", "Reset graphics"),
                Confirm::RestoreGraphics if self.config.low_memory_drifted() => (
                    "Restore the settings from before Low Memory?",
                    "Graphics settings changed after Low Memory was applied. Restoring the saved snapshot replaces those changes too. Audio, controls, saves and mods are untouched.",
                    "Restore anyway",
                ),
                Confirm::RestoreGraphics => (
                    "Restore the settings from before Low Memory?",
                    "Every graphics setting goes back to the value it had before the first Low Memory application. Audio, controls, saves and mods are untouched.",
                    "Restore",
                ),
                Confirm::CancelJob(k) => ("Stop this task?", k.title(), "Stop"),
                Confirm::AudioBoost { .. } => (
                    "Enable boost above 100%?",
                    "This raises the master volume above 100%, up to 200% (about +6 dB). Sustained loud output can damage speakers, headphones or your hearing. The engine's limiter prevents digital clipping, but it does not make a loud signal safe for your ears or your equipment. Start low and turn it up only as far as you need.",
                    "I know the risks",
                ),
            };
            let cancel_label = if matches!(confirm, Confirm::AudioBoost { .. }) {
                "Cancel"
            } else {
                "Keep"
            };
            let decision = std::cell::Cell::new(None::<bool>);
            let shown = widgets::modal(
                ctx,
                &p,
                "confirm-action",
                title,
                "This action is applied to the current configuration.",
                480.0,
                |ui| {
                    ui.label(crate::i18n::text(body).color(p.body));
                },
                |ui| {
                    ui.horizontal(|ui| {
                        if widgets::button(ui, &p, action, true).clicked() {
                            decision.set(Some(true));
                        }
                        if widgets::button(ui, &p, cancel_label, true).clicked() {
                            decision.set(Some(false));
                        }
                    });
                },
            );
            if shown.close {
                decision.set(Some(false));
            }
            if let Some(yes) = decision.get() {
                if yes {
                    match self.confirm.take() {
                        Some(Confirm::ResetBindings) => {
                            self.config.input.bindings = config::default_bindings();
                            self.notify("Controls reset to their defaults", Tone::Ok);
                        }
                        Some(Confirm::SafeGraphics) => {
                            self.config.graphics = crate::config::Graphics::default();
                            self.config.audio = crate::config::Audio::default();
                            self.notify("Graphics and audio settings reset to safe defaults. The next Play writes them.", Tone::Ok);
                        }
                        Some(Confirm::RestoreGraphics) => {
                            if self.config.restore_low_memory() {
                                self.notify("Graphics settings restored from before Low Memory. The next Play writes them.", Tone::Ok);
                            } else {
                                self.notify("There is no saved pre-Low Memory graphics snapshot to restore.", Tone::Warn);
                            }
                        }
                        Some(Confirm::CancelJob(kind)) => {
                            if let Some(j) =
                                self.jobs.iter().find(|j| j.kind == kind && !j.finished())
                            {
                                j.request_cancel();
                            }
                        }
                        Some(Confirm::AudioBoost { pct, previous }) => {
                            self.config.audio.master = audio_boost_master(pct, previous, true);
                            self.notify("Master volume boost enabled. The next Play writes it; keep the level sensible.", Tone::Warn);
                        }
                        None => {}
                    }
                } else if let Some(Confirm::AudioBoost { pct, previous }) = &self.confirm {
                    self.config.audio.master = audio_boost_master(*pct, *previous, false);
                }
                self.confirm = None;
            }
        }
        crate::onboarding::show(self, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::audio_boost_master;

    #[test]
    fn cancelling_the_above_100_warning_restores_the_previous_master_volume() {
        let previous = 0.73;
        let kept = audio_boost_master(175, previous, false);
        assert_eq!(
            kept, previous,
            "Cancel must restore the volume from before the warning, not leave 100%"
        );
        assert_eq!(audio_boost_master(175, previous, true), 1.75);
    }
}

#[cfg(test)]
mod toolchain_manager_tests;

/// A byte count for people: megabytes below a gigabyte, else gigabytes.
pub fn fmt_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= 1024.0 * MB {
        format!("{:.1} GB", bytes as f64 / (1024.0 * MB))
    } else {
        format!("{:.0} MB", bytes as f64 / MB)
    }
}

pub fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl App {
    /// Show `page`. An advanced page is an explicit request for Advanced mode, so the rail reveals its siblings instead
    /// of leaving the player on a page Standard does not list. Nothing else changes.
    pub fn open(&mut self, page: Page) {
        self.page = page;
        if !page.available(self.config.interface_mode) {
            self.config.interface_mode = crate::config::InterfaceMode::Advanced;
        }
    }

    /// For `--screenshot`: once the page has settled, ask the renderer for
    /// this window's frame, save it as PNG, and close. The image comes from
    /// the GPU frame itself, so other windows can never appear in it.
    fn take_screenshot(&mut self, ctx: &egui::Context) {
        // A capture is documentation, not a moment in use: a five-second toast
        // that happens to be up hides the controls it is drawn over. Clear the
        // transient messages well before the frame is requested; persistent
        // cards and warnings stay in the picture.
        if self
            .screenshot
            .as_ref()
            .is_some_and(|(_, frames)| *frames == 80)
        {
            self.toasts.clear();
        }
        let Some((path, frames)) = &mut self.screenshot else {
            return;
        };
        *frames += 1;
        ctx.request_repaint();
        if *frames == 90 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            let [w, h] = image.size;
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            if let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, bytes) {
                if let Err(e) = img.save(&*path) {
                    eprintln!("saving {}: {e}", path.display());
                }
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// Poll the background-loaded reuse offer and update the cached stale build check.
impl App {
    /// Check if the background thread has found an existing build to offer.
    fn poll_reuse(&mut self) {
        if self.reuse.is_none() {
            if let Ok(mut guard) = self.reuse_arc.lock() {
                if let Some(e) = guard.take() {
                    self.reuse = Some(e);
                }
            }
        }
    }

    /// Refresh the stale build cache if it's older than 30 seconds.
    fn poll_stale_build(&mut self) {
        let now = Instant::now();
        let should_refresh = self.stale_build_cached_at.map_or(true, |t| now.duration_since(t) > Duration::from_secs(30));
        if should_refresh {
            if let Some(stale) = crate::engine::stale_build() {
                self.stale_build_cache = Some(stale);
            }
            self.stale_build_cached_at = Some(now);
        }
    }
}

/// The launcher's redraw ceiling. egui asks for the next frame the moment a
/// hover animation is live, and with no vsync that runs the window at the
/// monitor's refresh rate (144 frames a second here) for a 2D interface that
/// has nothing to show between 60 and 144. Sleeping out the rest of a 60 Hz
/// interval costs at most one frame of input latency and takes the interactive
/// CPU cost down with it. A frame that already took longer than the interval
/// is never delayed further.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let since = self.last_frame.elapsed();
        if since < FRAME_INTERVAL {
            std::thread::sleep(FRAME_INTERVAL - since);
        }
        self.last_frame = Instant::now();
        // The native caption is installed by title rather than by a viewport
        // command: a command runs after egui's frame, which is the one-frame
        // catch-up slide the title bar used to have.
        platform::install_native_drag();
        if self.config.theme != self.applied {
            self.applied = self.config.theme;
            theme::apply(ctx, self.config.theme);
        }
        // Poll background-loaded caches.
        self.poll_reuse();
        self.poll_stale_build();
        self.poll_jobs();
        // The dependency report is warmed off the render thread. Keep the
        // status chips and setup guide live until its result arrives.
        if crate::deps::cached().is_none() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        // A launch that drew nothing says so here, while the window is still
        // on screen and the player can act on it.
        while let Some((said, good)) = crate::engine::take_play_alert() {
            self.notify(said, if good { Tone::Ok } else { Tone::Bad });
        }
        if crate::engine::play_watch_active() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        // The game's F1 menu is the other place its display and audio settings
        // are changed: take those back so the pages show what it will use.
        if crate::engine::adopt(
            &mut self.config.graphics,
            &mut self.config.audio,
            &mut self.config.performance,
        ) {
            ctx.request_repaint();
        }
        if self.boot_on_start
            && self.running(JobKind::Detect).is_none()
            && self.game.bootable_inputs()
        {
            self.boot_on_start = false;
            self.start(JobKind::Boot);
        }
        if self.build_on_start
            && self.running(JobKind::Detect).is_none()
            && self.game.exefs.is_some()
        {
            self.build_on_start = false;
            self.start(JobKind::Engine);
        }
        if self.running(JobKind::Engine).is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.frames.tick();
        self.poll_updates(ctx);
        if self.update.restart {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.shortcuts(ctx);
        shell::top_bar(self, ctx);
        shell::status_bar(self, ctx);
        shell::nav(self, ctx);
        let p = self.config.theme.palette();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin {
                left: if ctx.screen_rect().width() < 1240.0 {
                    16
                } else {
                    24
                },
                right: if ctx.screen_rect().width() < 1240.0 {
                    16
                } else {
                    24
                },
                top: 0,
                bottom: 0,
            }))
            .show(ctx, |ui| {
                crate::widgets::backdrop(
                    ui.painter(),
                    ui.max_rect().expand2(egui::vec2(28.0, 0.0)),
                    &p,
                );
                let fresh_page = self.page_seen != Some(self.page);
                self.page_seen = Some(self.page);
                let mut page_area = egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .id_salt(("page-scroll", self.page.label()));
                if fresh_page {
                    page_area = page_area.vertical_scroll_offset(0.0);
                }
                page_area.show(ui, |ui| {
                    // The page column stops growing at CONTENT_MAX and is
                    // centred in whatever is left, so a wide window gives the
                    // same measure of text rather than a stretched page.
                    let avail = ui.available_width();
                    let w = (avail - 12.0).min(theme::CONTENT_MAX);
                    let pad = ((avail - w) * 0.5).max(0.0);
                    ui.horizontal_top(|ui| {
                        ui.add_space(pad);
                        ui.vertical(|ui| {
                            ui.set_max_width(w.max(320.0));
                            pages::show(self, ui);
                            ui.add_space(theme::SPACE_XL);
                        });
                    });
                });
            });
        shell::activity_window(self, ctx);
        crate::pages::overview::popups(self, ctx);
        self.overlays(ctx);
        self.tour_frame(ctx);
        ingame::frame(self, ctx);
        startup_overlay::frame(self, ctx);
        pages::log::window(self, ctx);
        self.take_screenshot(ctx);
        if std::mem::take(&mut self.minimize_after_play) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        self.autosave();
        if !self.jobs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.screenshot.is_some() || self.tour.is_some() {
            // A capture run leaves the user's settings exactly as they were.
            return;
        }
        for j in &self.jobs {
            j.request_cancel();
        }
        let _ = self.config.save();
    }
}

/// Caption-button geometry. The three controls are one fixed-width cluster, so
/// the title bar can keep drag and double-click handling off them entirely.
const SHELL_BUTTON_W: f32 = 34.0;
const SHELL_BUTTON_H: f32 = 30.0;
const SHELL_BUTTON_GAP: f32 = 4.0;
const SHELL_CONTROL_INSET: f32 = 6.0;

#[derive(Clone, Copy)]
enum ShellControl {
    Minimize,
    Maximize { restore: bool },
    Close,
}

/// A caption button for the custom shell. The glyphs are painted rather than
/// taken from a fallback font, so the three icons share one stroke weight and
/// optical box at every DPI. Hover fades in, the face sinks on press, and close
/// is the one control that turns red instead of merely brightening.
fn shell_control(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    kind: ShellControl,
    tip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(SHELL_BUTTON_W, SHELL_BUTTON_H),
        egui::Sense::click(),
    );
    let hover =
        ui.ctx()
            .animate_bool_with_time(response.id.with("shell-hover"), response.hovered(), 0.12);
    let pressed = response.is_pointer_button_down_on();
    let close = matches!(kind, ShellControl::Close);
    let bg = if close {
        theme::alpha(p.bad, if pressed { 240 } else { (205.0 * hover) as u8 })
    } else if pressed {
        theme::alpha(egui::Color32::WHITE, 22)
    } else {
        theme::alpha(egui::Color32::WHITE, (10.0 * hover) as u8)
    };
    let sink = if pressed { 2.5 } else { 2.0 };
    let face = rect.shrink(sink);
    let radius = egui::CornerRadius::same(8);
    let painter = ui.painter();
    painter.rect_filled(face, radius, bg);
    if !close && hover > 0.0 {
        painter.rect_stroke(
            face,
            radius,
            egui::Stroke::new(
                1.0_f32,
                theme::alpha(egui::Color32::WHITE, (14.0 * hover) as u8),
            ),
            egui::StrokeKind::Inside,
        );
    }
    if response.has_focus() {
        painter.rect_stroke(
            face,
            radius,
            egui::Stroke::new(1.0_f32, p.electric),
            egui::StrokeKind::Inside,
        );
    }

    let ink = if close {
        mix_colour(
            p.body,
            egui::Color32::WHITE,
            if pressed { 1.0 } else { hover },
        )
    } else {
        mix_colour(p.body, p.ink, hover)
    };
    let stroke = egui::Stroke::new(1.35_f32, ink);
    let c = rect.center();
    match kind {
        ShellControl::Minimize => {
            shell_line(
                painter,
                &[
                    egui::pos2(c.x - 5.0, c.y + 0.5),
                    egui::pos2(c.x + 5.0, c.y + 0.5),
                ],
                stroke,
            );
        }
        ShellControl::Maximize { restore: false } => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(10.0, 10.0)),
                egui::CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        ShellControl::Maximize { restore: true } => {
            // The back window's top and right edges remain visible; its covered
            // edges are not drawn, so the two outlines never cross.
            shell_line(
                painter,
                &[
                    egui::pos2(c.x - 1.5, c.y - 4.5),
                    egui::pos2(c.x + 4.5, c.y - 4.5),
                    egui::pos2(c.x + 4.5, c.y + 1.0),
                ],
                stroke,
            );
            painter.rect_stroke(
                egui::Rect::from_min_max(
                    egui::pos2(c.x - 4.5, c.y - 1.5),
                    egui::pos2(c.x + 1.5, c.y + 4.5),
                ),
                egui::CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        ShellControl::Close => {
            let r = 4.25;
            shell_line(
                painter,
                &[egui::pos2(c.x - r, c.y - r), egui::pos2(c.x + r, c.y + r)],
                stroke,
            );
            shell_line(
                painter,
                &[egui::pos2(c.x + r, c.y - r), egui::pos2(c.x - r, c.y + r)],
                stroke,
            );
        }
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(crate::i18n::tr(tip))
}

/// A line with round caps, matching the softened corners of the painted box.
fn shell_line(painter: &egui::Painter, points: &[egui::Pos2], stroke: egui::Stroke) {
    if points.len() < 2 {
        return;
    }
    painter.add(egui::Shape::line(points.to_vec(), stroke));
    let cap = stroke.width * 0.5;
    for p in [points[0], points[points.len() - 1]] {
        painter.circle_filled(p, cap, stroke.color);
    }
}

fn mix_colour(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    egui::Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}

pub(crate) fn window_icon() -> Option<egui::IconData> {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).ok()
}

/// `--import-save <folder>`: import a save the way Game Setup does, print
/// what happened, and exit without opening a window.
fn import_save_cli(source: &str) -> std::process::ExitCode {
    let Some(save_dir) = config::Config::save_dir() else {
        eprintln!("APPDATA is not set");
        return std::process::ExitCode::FAILURE;
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string();
    // The same message the page shows: a console import that lands where the game will not read it has to say so rather than
    // print a success line.
    match saves::import(
        std::path::Path::new(source),
        &save_dir,
        saves::FALLBACK_TITLE_ID,
        &stamp,
    ) {
        Ok(i) => {
            let (text, ok) = saves::import_message(Ok(i));
            println!("{text}");
            if ok {
                std::process::ExitCode::SUCCESS
            } else {
                std::process::ExitCode::FAILURE
            }
        }
        Err(e) => {
            let (text, _) = saves::import_message(Err(e));
            eprintln!("{text}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// `--verify-log [path]`: read an engine build log — the newest one, or the one
/// named — and print the error that explains a failed build, the repair the
/// launcher would try, and nothing else. It runs without the window, so a log
/// pasted from another machine can be diagnosed with one command.
///
/// Exit codes: 0 an error was found and printed, 3 the log has no error line,
/// 2 the file could not be read and no log was given or found.
fn verify_log_cli(path: Option<&str>) -> i32 {
    let file = match path {
        Some(p) => std::path::PathBuf::from(p),
        None => match crate::jobs::newest_log() {
            Some(p) => p,
            None => {
                eprintln!("no log file: name one, or build the game once so a log exists in %APPDATA%\\EmblemWeave\\logs");
                return 2;
            }
        },
    };
    let bytes = match std::fs::read(&file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{}: {e}", file.display());
            return 2;
        }
    };
    // A log that came through a mail client or a chat window can carry bytes that are not UTF-8; the error is still
    // readable, so decode leniently instead of refusing the file.
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let found = crate::engine::first_error_block(&text).is_some();
    println!("log: {}", file.display());
    print!("{}", crate::engine::verify_log_text(&text));
    if found {
        0
    } else {
        3
    }
}

/// `--check-toolchain`: what the build will use, everything wrong with it, and
/// one file compiled and linked with it. It answers "can this PC build the game"
/// without starting a build, which is the only way to find out that a header is
/// missing before twenty minutes of compiling.
///
/// Exit codes: 0 ready, 1 a piece is missing or too old, 2 the pieces are there
/// but they cannot compile and link the test file.
fn check_toolchain_cli() -> i32 {
    // The registry first: it is what the Requirements page and the pre-build
    // check read, and it names every log the player can act on.
    let rows = crate::deps::check();
    println!("requirements:");
    for row in &rows {
        println!(
            "  {:<30} {:<11} {:<8} {}",
            row.dep.name,
            row.status.label(),
            if row.required { "required" } else { "optional" },
            row.status.detail()
        );
    }
    let blocking = crate::deps::blocking(&rows);
    for row in &blocking {
        let install = match row.dep.install {
            Some(i) => i.line().to_string(),
            None => row.dep.note.to_string(),
        };
        println!(
            "
missing: {} (needs {})
  detection: {}
  install: {install}",
            row.dep.name,
            row.dep.min.unwrap_or("any version"),
            row.status.detail()
        );
    }
    let tc = crate::toolchain::probe();
    for (name, value) in crate::toolchain::stale_env() {
        println!("ignoring the inherited {name}={value}");
    }
    if !blocking.is_empty() {
        return 1;
    }
    if !tc.problems.is_empty() {
        for p in &tc.problems {
            println!(
                "
problem: {p}"
            );
        }
        return 1;
    }
    let resolved = match crate::toolchain::init() {
        Ok(r) => r,
        Err(e) => {
            println!(
                "
problem: {e}"
            );
            return 1;
        }
    };
    println!(
        "
toolchain: {}",
        resolved.toolchain.summary()
    );
    println!("fingerprint: {}", resolved.toolchain.fingerprint());
    let dir = std::env::temp_dir().join("emblemweave-toolchain-probe");
    match crate::toolchain::probe_build(&resolved.compiler, &resolved.env, &dir) {
        Ok(()) => {
            println!("
the toolchain compiled and linked its test file with its own headers, libraries and SDK; the build can start");
            0
        }
        Err(e) => {
            println!(
                "
problem: {e}"
            );
            2
        }
    }
}

fn main() -> eframe::Result<()> {
    crash::install();
    let args: Vec<String> = std::env::args().collect();
    // Read-only evidence for issue #70: hash the effective ExeFS and RomFS from
    // a `.nup` and from an extracted folder, write both manifests and the
    // per-path differences as JSON, and exit 2 when they differ. This does not
    // build, play, write the overlay, or change any runtime setting. It runs
    // before first-run setup or a tool fetch so the comparison itself only
    // reads the two trees and writes its report.
    if let Some(i) = args.iter().position(|a| a == "--compare-data") {
        let nup = args.get(i + 1).cloned().unwrap_or_default();
        let folder = args.get(i + 2).cloned().unwrap_or_default();
        if nup.is_empty() || folder.is_empty() {
            eprintln!("usage: EmblemWeave.exe --compare-data <game.nup> <folder> [report.json]");
            std::process::exit(1);
        }
        let out = args
            .get(i + 3)
            .filter(|arg| !arg.starts_with("--"))
            .cloned()
            .unwrap_or_else(|| "data-compare.json".to_string());
        match manifest::compare_data_cli(
            std::path::Path::new(&nup),
            std::path::Path::new(&folder),
            std::path::Path::new(&out),
        ) {
            Ok(report) => std::process::exit(if report.equal { 0 } else { 2 }),
            Err(e) => {
                eprintln!("compare-data: {e}");
                std::process::exit(1);
            }
        }
    }
    // Before anything reads the settings: a fresh copy keeps its data beside the program, so the first run writes its config there.
    platform::adopt_portable_on_first_run();
    // Prime the tool_path cache with the correct PATH (including portable tools) before any
    // background thread or CLI command calls it. This ensures --check-toolchain and --build-engine
    // see the same CMake discovery as the build (issue #35).
    let _ = crate::toolchain::tool_path();
    // A copy without `tools\bin` carries the missing toolchain pieces into it on first run, in the background; the
    // Requirements page reports them as Carried once they land. Pieces the PC already has are not downloaded. `--fetch-tools`
    // forces all four in the foreground, so do not start the background fetch as well: both would download the same archives.
    let fetch_tools = args.iter().any(|a| a == "--fetch-tools");
    if !fetch_tools {
        portable::auto_start();
    }
    // Carry the toolchain the release ships: the same four downloads the launcher would do on first run, runnable from a
    // console so a build machine can prepare `tools\bin` without opening the window.
    if fetch_tools {
        let last = std::cell::RefCell::new(String::new());
        let r = portable::install_all(&|f, line| {
            if *last.borrow() != line {
                println!("{:>3.0}% {line}", f * 100.0);
                *last.borrow_mut() = line;
            }
        });
        match r {
            Ok(()) => {
                println!(
                    "every piece is carried in {}",
                    portable::bin_dir()
                        .map(|d| d.display().to_string())
                        .unwrap_or_default()
                );
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("fetching the toolchain: {e}");
                std::process::exit(1);
            }
        }
    }
    // The Graphics page's preset picker, runnable from a console: copies the shipped shaders and preset beside fefw.exe
    // and points ReShade.ini at them.
    if let Some(i) = args.iter().position(|a| a == "--apply-preset") {
        let name = args.get(i + 1).cloned().unwrap_or_default();
        match engine::apply_reshade_preset(&name) {
            Ok(msg) => {
                println!("{msg}");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
    if args.iter().any(|a| a == "--check-toolchain") {
        std::process::exit(check_toolchain_cli());
    }
    // The dependency table the docs carry, from the registry the build checks:
    // the same text goes into README.md and docs/TROUBLESHOOTING.md.
    if args.iter().any(|a| a == "--print-requirements") {
        print!("{}", crate::deps::markdown());
        std::process::exit(0);
    }
    // Unpack a package whose RomFS is packed with zstd, the step the build does on its own, so it can be run by hand on a package.
    if let Some(i) = args.iter().position(|a| a == "--unpack-romfs") {
        let src = args.get(i + 1).cloned().unwrap_or_default();
        let dest = args
            .get(i + 2)
            .cloned()
            .unwrap_or_else(|| "romfs-unpacked".to_string());
        let r = gamedata::unpack_romfs_command(
            std::path::Path::new(&src),
            std::path::Path::new(&dest),
            &mut |m| println!("{m}"),
        );
        match r {
            Ok(write) if write.reused => {
                println!("the overlay in {dest} is already up to date");
                std::process::exit(0);
            }
            Ok(write) => {
                println!(
                    "{} files, {:.1} GB written to {dest}",
                    write.written,
                    write.written_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
                );
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("unpack-romfs: {e}");
                std::process::exit(1);
            }
        }
    }
    // About's portable switch without the window: the same call the button makes, for a script or a support reply.
    if let Some(i) = args.iter().position(|a| a == "--portable") {
        let on = !matches!(args.get(i + 1).map(String::as_str), Some("off") | Some("0"));
        let Some(dir) = platform::exe_dir() else {
            eprintln!("portable: the folder this copy runs from could not be found");
            std::process::exit(1);
        };
        match platform::set_portable(&dir, on) {
            Ok(copied) => {
                let where_to = dir.join("data");
                if on {
                    println!(
                        "portable mode is on: {} (restart to use it)",
                        where_to.display()
                    );
                    if let Some(save) = copied {
                        println!("saves copied to {}", save.display());
                    }
                } else {
                    println!("portable mode is off; the data folder is left in place");
                }
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("portable: {e}");
                std::process::exit(1);
            }
        }
    }
    // The Graphics page's "Look for the files" without the window, for a support report or a script.
    if args.iter().any(|a| a == "--adopt-upscalers") {
        match engine::adopt_upscaler_files() {
            Ok(m) => {
                println!("{m}");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("adopt-upscalers: {e}");
                std::process::exit(1);
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--import-save") {
        let code = args
            .get(i + 1)
            .map(|s| import_save_cli(s))
            .unwrap_or(std::process::ExitCode::FAILURE);
        std::process::exit(if code == std::process::ExitCode::SUCCESS {
            0
        } else {
            1
        });
    }
    if let Some(i) = args.iter().position(|a| a == "--verify-log") {
        let path = args.get(i + 1).map(|s| s.as_str());
        std::process::exit(verify_log_cli(path));
    }
    // Screenshot and layout checks need a repeatable window size; it changes
    // this run only and never touches the saved preferences.
    let window_size = args
        .iter()
        .position(|a| a == "--size")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.split_once('x'))
        .and_then(|(w, h)| Some([w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?]))
        .map(|[w, h]| [w.max(1120.0), h.max(720.0)])
        .unwrap_or([1440.0, 900.0]);
    // The launcher draws its own top bar and window buttons. Keeping the
    // native title bar would put a stock Windows strip above that design.
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(window_size)
        .with_min_inner_size([1120.0, 720.0])
        .with_decorations(false)
        .with_resizable(true)
        .with_title("EmblemWeave");
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    eframe::run_native(
        "EmblemWeave",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
