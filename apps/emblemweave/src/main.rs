//! EmblemWeave: the native PC recompilation launcher for Fire Emblem:
//! Fortune's Weave, by XiØN.
//!
//! The launcher is the product interface to the recompilation system: it
//! locates and validates the user's own game data, decodes and recompiles the
//! game's code, runs the runtime, and reports exactly how far it gets. It never
//! claims more than the runtime delivers: Play stays unavailable until the game
//! can present a frame.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod engine;
#[cfg(windows)]
mod exe_icon;
mod gamedata;
mod jobs;
mod logo;
mod pages;
mod cheats;
mod ingame;
mod crash;
mod mods;
mod pad;
mod perf;
mod snapshots;
mod updater;
mod saves;
mod savefile;
mod theme;
mod widgets;

use config::Config;
use eframe::egui::{self, RichText};
use gamedata::GameData;
use jobs::{Job, JobKind, JobResult};
use pages::Page;
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
}

/// A question the user must answer before an action runs.
pub enum Confirm {
    ResetBindings,
    CancelJob(JobKind),
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
    /// `--screenshot PATH`: save the window's own rendered frame and close.
    screenshot: Option<(std::path::PathBuf, u32)>,
    applied: theme::Scheme,
    pub saves_ui: pages::saves::Ui,
    pub mods_ui: pages::mods::Ui,
    pub cheats_ui: pages::cheats::Ui,
    /// The game's `[pad]` settings as the Controls page last read them.
    pub tuning: Option<pad::Tuning>,
    /// The launcher's own frame times.
    pub frames: perf::Frames,
    pub update: Updating,
    pub ingame: ingame::Ingame,
    pub log_ui: pages::log::Ui,
    /// `--tour`: walk through the features by itself, for recordings.
    tour: Option<Instant>,
    tour_step: usize,
}

fn clock() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = secs % 86_400;
    format!("{:02}:{:02}:{:02} UTC", s / 3600, s % 3600 / 60, s % 60)
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> App {
        theme::configure_fonts(&cc.egui_ctx);
        let (config, warning) = Config::load();
        theme::apply(&cc.egui_ctx, config.theme);
        let saved = serde_json::to_string(&config).unwrap_or_default();
        let last_boot = Config::dir()
            .map(|d| d.join("last-boot.json"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .map(|_| ());
        // The game's taskbar icon comes from fefw.exe, which a build or the
        // engine can replace: make it ours again at every start, off the render
        // thread because the executable is rewritten on disk.
        std::thread::spawn(|| {
            let _ = crate::engine::brand_game();
        });
        let mut app = App {
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
            activity: Vec::new(),
            toasts: Vec::new(),
            confirm: None,
            capture: None,
            diag_tab: pages::diagnostics::Tab::Runtime,
            diag_filter: String::new(),
            lift_limit: 4000,
            boot_on_start: false,
            build_on_start: false,
            screenshot: None,
            saves_ui: Default::default(),
            mods_ui: Default::default(),
            cheats_ui: Default::default(),
            tuning: None,
            frames: Default::default(),
            update: Default::default(),
            ingame: Default::default(),
            log_ui: Default::default(),
            tour: None,
            tour_step: 0,
        };
        // Command line: `--page <name>` opens a page, `--boot-test` runs the
        // boot test as soon as the game data is detected.
        let args: Vec<String> = std::env::args().collect();
        for (i, a) in args.iter().enumerate() {
            match a.as_str() {
                "--boot-test" => app.boot_on_start = true,
                "--build-engine" => app.build_on_start = true,
                "--tour" => app.tour = Some(Instant::now()),
                "--screenshot" => app.screenshot = args.get(i + 1).map(|p| (std::path::PathBuf::from(p), 0)),
                "--page" => {
                    if let Some(name) = args.get(i + 1) {
                        if let Some(p) = Page::ALL.iter().find(|p| p.label().eq_ignore_ascii_case(name.replace('-', " ").as_str())) {
                            app.page = *p;
                        }
                    }
                }
                _ => {}
            }
        }
        let _ = last_boot;
        if let Some(w) = warning {
            app.notify(w, Tone::Warn);
        }
        app.note("Launcher started", Tone::Neutral);
        app.start(JobKind::Detect);
        if app.config.updates.check {
            app.check_updates();
        }
        app
    }

    pub fn note(&mut self, text: impl Into<String>, tone: Tone) {
        let text = text.into();
        // The crash report carries the last few of these.
        crate::crash::note(&text);
        self.activity.push(Activity { at: clock(), text, tone });
        if self.activity.len() > 200 {
            self.activity.remove(0);
        }
    }

    pub fn notify(&mut self, text: impl Into<String>, tone: Tone) {
        let text = text.into();
        self.note(text.clone(), tone);
        self.toasts.push(Toast { text, tone, until: Instant::now() + Duration::from_secs(5) });
    }

    pub fn running(&self, kind: JobKind) -> Option<&Job> {
        self.jobs.iter().find(|j| j.kind == kind && !j.finished())
    }

    pub fn busy(&self) -> bool {
        self.jobs.iter().any(|j| !j.finished() && j.kind != JobKind::Detect)
    }

    /// Start a job of `kind` unless one is already running.
    pub fn start(&mut self, kind: JobKind) {
        if self.running(kind).is_some() {
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
            JobKind::Engine => match (self.game.exefs.clone(), self.game.nup.clone().or_else(|| self.game.romfs.clone())) {
                (Some(exefs), Some(romfs)) => jobs::spawn(JobKind::Engine, &engine::STAGES, move |ctx| engine::build(ctx, exefs, romfs)),
                _ => return self.notify("Choose the game data on Game Setup before building", Tone::Warn),
            },
        };
        if kind != JobKind::Detect {
            self.note(format!("{} started", kind.title()), Tone::Accent);
        }
        self.jobs.push(job);
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
            if kind != JobKind::Detect {
                self.last_job_log = std::mem::take(&mut p.log);
            }
            if let Some(err) = p.failed.take() {
                drop(p);
                self.notify(format!("{}: {err}", kind.title()), Tone::Bad);
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
                    let tone = if s.verified.is_some_and(|(ok, n)| ok == n) { Tone::Ok } else { Tone::Warn };
                    let text = match (&s.report, s.verified) {
                        (Some(r), Some((ok, n))) => format!("Pipeline finished: {} functions, {ok}/{n} verified", r.lifted_functions),
                        (Some(r), None) => format!("Pipeline finished: {} functions (native build not verified)", r.lifted_functions),
                        _ => "Pipeline finished".to_string(),
                    };
                    self.last_pipeline = Some(*s);
                    self.notify(text, tone);
                }
                Some(JobResult::Engine(dir)) => {
                    drop(p);
                    self.notify(format!("The game is built and ready to play ({})", dir.display()), Tone::Ok);
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
        let steps: [f32; 13] = [0.0, 3.5, 5.0, 9.0, 10.0, 14.0, 17.5, 21.0, 22.0, 23.5, 25.0, 26.5, 29.0];
        while self.tour_step < steps.len() && t >= steps[self.tour_step] {
            match self.tour_step {
                0 => self.page = Page::Overview,
                1 => self.page = Page::Saves,
                2 => {
                    let auto = Config::save_dir().map(|d| crate::saves::account_dir(&d).join("000").join("Auto"));
                    if let Some(a) = auto {
                        let _ = pages::saves::open_slot(self, &a);
                        self.saves_ui.scroll_to_editor = true;
                    }
                }
                3 => self.page = Page::Mods,
                4 => pages::mods::load_gamebanana(self),
                5 => self.page = Page::Cheats,
                6 => self.page = Page::Log,
                7 => self.page = Page::Appearance,
                8 => self.config.theme = theme::Scheme::Arcade,
                9 => self.config.theme = theme::Scheme::Fe,
                10 => self.config.theme = theme::Scheme::Xion,
                11 => self.page = Page::About,
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

    /// Download the release found and swap this build for it.
    pub fn install_update(&mut self) {
        let UpdateState::Ready(release) = self.update.state.clone() else { return };
        self.update.state = UpdateState::Downloading;
        let slot: Slot<std::path::PathBuf> = std::sync::Arc::new(std::sync::Mutex::new(None));
        let handle = slot.clone();
        std::thread::spawn(move || {
            let dir = std::env::temp_dir().join("emblemweave-update");
            let r = updater::download(&release, &dir).and_then(|exe| updater::install(&exe, true).map(|()| exe));
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
                            self.notify(format!("EmblemWeave {} is available", rel.tag), Tone::Accent);
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
                        self.notify("Installing the new version: the launcher restarts", Tone::Ok);
                    }
                    Err(e) => self.update.state = UpdateState::Failed(e),
                }
            }
        }
        if self.update.restart {
            ctx.request_repaint();
        } else if self.update.state == UpdateState::Checking || self.update.state == UpdateState::Downloading {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// The brand strip above every page: the project, the page it is on, and
    /// the launcher's own frame rate.
    fn top_bar(&mut self, ctx: &egui::Context) {
        let p = self.config.theme.palette();
        let s = self.frames.stats();
        let fps = self.frames.fps().unwrap_or(0.0);
        egui::TopBottomPanel::top("topbar")
            .exact_height(theme::TOPBAR_HEIGHT)
            .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin { left: 14, right: 14, top: 5, bottom: 3 }))
            .show(ctx, |ui| {
                let strip = ui.available_rect_before_wrap();
                widgets::glass_panel(ui, strip, &p, egui::CornerRadius::same(14));
                ui.horizontal_centered(|ui| {
                    ui.add_space(14.0);
                    // The brand mark: a lit diamond, as on the reference bar.
                    let (mark, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                    let c = mark.center();
                    let diamond = vec![
                        c + egui::vec2(0.0, -5.5),
                        c + egui::vec2(5.5, 0.0),
                        c + egui::vec2(0.0, 5.5),
                        c + egui::vec2(-5.5, 0.0),
                    ];
                    ui.painter().add(egui::Shape::convex_polygon(diamond, p.accent, egui::Stroke::new(1.0_f32, theme::alpha(egui::Color32::WHITE, 170))));
                    ui.add_space(6.0);
                    widgets::tracked(ui, "EMBLEMWEAVE", 10.5, p.ink, 3.0);
                    ui.add_space(12.0);
                    let (div, _) = ui.allocate_exact_size(egui::vec2(1.0, 14.0), egui::Sense::hover());
                    ui.painter().vline(div.center().x, div.y_range(), egui::Stroke::new(1.0_f32, theme::alpha(p.accent, 70)));
                    ui.add_space(11.0);
                    widgets::tracked(ui, &self.page.label().to_uppercase(), 9.5, p.accent_hi, theme::TRACK_MID);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(14.0);
                        if self.config.hud.fps {
                            if ui.small_button("✕").on_hover_text("Hide the frame rate").clicked() {
                                self.config.hud.fps = false;
                            }
                            ui.label(RichText::new(format!("{fps:.0} fps · {:.1} ms", s.avg_ms)).font(theme::mono(12.0)).color(p.accent_hi));
                        } else {
                            ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"), " · ", env!("EW_GIT_REV"))).font(theme::mono(11.0)).color(p.muted));
                        }
                    });
                });
            });
    }

    fn autosave(&mut self) {
        if self.tour.is_some() {
            return;
        }
        if self.last_save_check.elapsed() < Duration::from_millis(400) {
            return;
        }
        self.last_save_check = Instant::now();
        let now = serde_json::to_string(&self.config).unwrap_or_default();
        if now != self.saved {
            let paths_changed = serde_json::to_string(&self.config.paths).ok()
                != serde_json::from_str::<Config>(&self.saved).ok().and_then(|c| serde_json::to_string(&c.paths).ok());
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
                for (n, key) in [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6, egui::Key::Num7, egui::Key::Num8, egui::Key::Num9]
                    .into_iter()
                    .enumerate()
                {
                    if i.key_pressed(key) {
                        if let Some(p) = pages.get(n) {
                            self.page = *p;
                        }
                    }
                }
            }
        });
    }

    /// The rail: the lockup, then the pages in named groups.
    fn nav(&mut self, ctx: &egui::Context) {
        const GROUPS: [(&str, &[Page]); 4] = [
            ("Play", &[Page::Overview, Page::Setup]),
            ("Library", &[Page::Saves, Page::Mods, Page::Cheats]),
            ("Runtime", &[Page::Recompile, Page::Graphics, Page::Audio, Page::Controls]),
            ("System", &[Page::Appearance, Page::Diagnostics, Page::Log, Page::About]),
        ];
        let p = self.config.theme.palette();
        egui::SidePanel::left("nav")
            .exact_width(theme::NAV_WIDTH)
            .resizable(false)
            .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin { left: 8, right: 4, top: 6, bottom: 8 }))
            .show(ctx, |ui| {
                let full = ui.available_rect_before_wrap();
                widgets::glass_panel(ui, full, &p, egui::CornerRadius::same(18));
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    logo::show_lockup(ui, &p, theme::NAV_WIDTH - 96.0);
                    ui.add_space(1.0);
                    widgets::tracked(ui, "NATIVE PC RECOMPILATION", 8.0, p.muted, theme::TRACK_TIGHT);
                });
                ui.add_space(1.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (name, pages) in GROUPS {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        widgets::eyebrow(ui, &p, name);
                    });
                    for page in pages {
                        let i = Page::ALL.iter().position(|q| q == page).unwrap_or(0);
                        let r = widgets::nav_item(ui, &p, page.icon(), page.label(), self.page == *page)
                            .on_hover_text(format!("{}  (Ctrl+{})", page.label(), i + 1));
                        if r.clicked() {
                            self.page = *page;
                        }
                    }
                }
                });
            });
    }

    /// The status column beside the Overview page: where the game, the
    /// recompiler and the runtime stand, and what the launcher just did.
    fn side_panel(&mut self, ctx: &egui::Context) {
        // The column steps aside on a narrow window: the page keeps its room
        // instead of both being squeezed into scroll boxes.
        if self.page != Page::Overview || ctx.screen_rect().width() < theme::SIDE_MIN_WINDOW {
            return;
        }
        let p = self.config.theme.palette();
        egui::SidePanel::right("status-column")
            .exact_width(theme::SIDE_WIDTH)
            .resizable(false)
            .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin { left: 0, right: 14, top: 12, bottom: 2 }))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let (ready, r_tone) = pages::overview::readiness(self);
                    widgets::section(ui, &p, "Status", |ui| {
                        ui.horizontal(|ui| {
                            widgets::status_dot(ui, &p, r_tone);
                            ui.label(RichText::new(ready).font(theme::semibold(14.0)).color(r_tone.colour(&p)));
                        });
                        ui.add_space(theme::SPACE_S);
                        let severities = [
                            (
                                "Game data",
                                match self.game.worst() {
                                    crate::gamedata::Severity::Ok | crate::gamedata::Severity::Info => ("Ready", Tone::Ok),
                                    crate::gamedata::Severity::Warn => ("Needs attention", Tone::Warn),
                                    crate::gamedata::Severity::Error => ("Incomplete", Tone::Bad),
                                },
                            ),
                            (
                                "Recompilation",
                                match &self.last_pipeline {
                                    Some(s) => match s.verified {
                                        Some((ok, n)) if ok == n => ("Verified", Tone::Ok),
                                        Some(_) => ("Differs", Tone::Warn),
                                        None => ("Lifted", Tone::Accent),
                                    },
                                    None => ("Not run", Tone::Neutral),
                                },
                            ),
                            (
                                "Runtime",
                                match &self.last_boot {
                                    Some(_) => ("Booted", Tone::Accent),
                                    None => ("Not run", Tone::Neutral),
                                },
                            ),
                        ];
                        for (label, (value, tone)) in severities {
                            ui.horizontal(|ui| {
                                widgets::status_dot(ui, &p, tone);
                                ui.label(RichText::new(label).size(12.5).color(p.body));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new(value).font(theme::semibold(12.0)).color(tone.colour(&p)));
                                });
                            });
                            ui.add_space(3.0);
                        }
                    });
                    widgets::section(ui, &p, "Recent activity", |ui| {
                        if self.activity.is_empty() {
                            ui.label(RichText::new("Nothing yet").size(12.5).color(p.muted));
                        }
                        for a in self.activity.iter().rev().take(8) {
                            ui.horizontal_top(|ui| {
                                ui.label(RichText::new(&a.at).font(theme::mono(10.5)).color(p.muted));
                                ui.add(egui::Label::new(RichText::new(&a.text).size(12.0).color(a.tone.colour(&p))).wrap());
                            });
                            ui.add_space(3.0);
                        }
                    });
                });
            });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let p = self.config.theme.palette();
        egui::TopBottomPanel::bottom("status")
            .exact_height(theme::STATUSBAR_HEIGHT)
            .frame(egui::Frame::new().fill(p.bg_deep).inner_margin(egui::Margin { left: 16, right: 16, top: 4, bottom: 4 }))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let active = self.jobs.iter().find(|j| !j.finished() && j.kind != JobKind::Detect);
                    match active {
                        Some(job) => {
                            let kind = job.kind;
                            let message = job.snapshot(|pr| pr.message.clone());
                            let steps = job.steps.load(std::sync::atomic::Ordering::Relaxed);
                            ui.spinner();
                            ui.label(RichText::new(kind.title().to_uppercase()).font(theme::semibold(10.0)).color(p.accent));
                            ui.label(RichText::new(message).font(theme::mono(11.0)).color(p.body));
                            if kind == JobKind::Boot && steps > 0 {
                                ui.label(RichText::new(format!("{} instr", fmt_count(steps))).font(theme::mono(10.5)).color(p.muted));
                            }
                            ui.label(RichText::new(format!("{:.1}s", job.started.elapsed().as_secs_f32())).font(theme::mono(10.5)).color(p.muted));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Cancel").on_hover_text("Stop at the next safe point").clicked() {
                                    self.confirm = Some(Confirm::CancelJob(kind));
                                }
                            });
                            ctx.request_repaint_after(Duration::from_millis(100));
                        }
                        None => {
                            let (text, tone) = pages::overview::readiness(self);
                            widgets::status_dot(ui, &p, tone);
                            ui.label(RichText::new(text.to_uppercase()).font(theme::semibold(9.5)).color(tone.colour(&p)));
                            ui.label(RichText::new(clock()).font(theme::mono(10.5)).color(p.muted));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"), " ", env!("EW_GIT_REV"))).font(theme::mono(10.5)).color(p.muted));
                                ui.label(RichText::new("by XiØN").font(theme::mono(10.5)).color(p.muted));
                                ui.label(RichText::new("EMBLEMWEAVE").font(theme::semibold(9.5)).color(p.accent));
                            });
                        }
                    }
                });
            });
    }

    fn overlays(&mut self, ctx: &egui::Context) {
        let p = self.config.theme.palette();
        // Toasts, newest at the bottom right.
        self.toasts.retain(|t| t.until > Instant::now());
        if !self.toasts.is_empty() {
            egui::Area::new(egui::Id::new("toasts"))
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -48.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    for t in &self.toasts {
                        egui::Frame::new()
                            .fill(p.surface)
                            .stroke(egui::Stroke::new(1.0_f32, theme::alpha(t.tone.colour(&p), 160)))
                            .corner_radius(egui::CornerRadius::same(theme::RADIUS_M))
                            .inner_margin(egui::Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                                ui.set_max_width(420.0);
                                ui.label(RichText::new(&t.text).size(13.5).color(p.ink));
                            });
                        ui.add_space(6.0);
                    }
                });
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        // Confirmation dialog.
        if let Some(confirm) = &self.confirm {
            let (title, body, action) = match confirm {
                Confirm::ResetBindings => ("Reset controls?", "Every binding returns to its default. This cannot be undone.", "Reset"),
                Confirm::CancelJob(k) => ("Stop this task?", k.title(), "Stop"),
            };
            let mut decision = None;
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(RichText::new(body).color(p.body));
                    ui.add_space(theme::SPACE_M);
                    ui.horizontal(|ui| {
                        if widgets::button(ui, &p, action, true).clicked() {
                            decision = Some(true);
                        }
                        if widgets::button(ui, &p, "Keep", true).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            decision = Some(false);
                        }
                    });
                });
            if let Some(yes) = decision {
                if yes {
                    match self.confirm.take() {
                        Some(Confirm::ResetBindings) => {
                            self.config.input.bindings = config::default_bindings();
                            self.notify("Controls reset to their defaults", Tone::Ok);
                        }
                        Some(Confirm::CancelJob(kind)) => {
                            if let Some(j) = self.jobs.iter().find(|j| j.kind == kind && !j.finished()) {
                                j.request_cancel();
                            }
                        }
                        None => {}
                    }
                }
                self.confirm = None;
            }
        }
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
    /// For `--screenshot`: once the page has settled, ask the renderer for
    /// this window's frame, save it as PNG, and close. The image comes from
    /// the GPU frame itself, so other windows can never appear in it.
    fn take_screenshot(&mut self, ctx: &egui::Context) {
        let Some((path, frames)) = &mut self.screenshot else { return };
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.config.theme != self.applied {
            self.applied = self.config.theme;
            theme::apply(ctx, self.config.theme);
        }
        self.poll_jobs();
        // The game's F1 menu is the other place its display and audio settings
        // are changed: take those back so the pages show what it will use.
        if crate::engine::adopt(&mut self.config.graphics, &mut self.config.audio) {
            ctx.request_repaint();
        }
        if self.boot_on_start && self.running(JobKind::Detect).is_none() && self.game.bootable_inputs() {
            self.boot_on_start = false;
            self.start(JobKind::Boot);
        }
        if self.build_on_start && self.running(JobKind::Detect).is_none() && self.game.exefs.is_some() {
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
        if self.config.hud.fps {
            // An indicator that only moves when something else does measures
            // nothing: draw every frame while it is up.
            ctx.request_repaint();
        }
        self.shortcuts(ctx);
        self.top_bar(ctx);
        self.status_bar(ctx);
        self.nav(ctx);
        self.side_panel(ctx);
        let p = self.config.theme.palette();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin { left: 28, right: 28, top: 0, bottom: 0 }))
            .show(ctx, |ui| {
                crate::widgets::backdrop(ui.painter(), ui.max_rect().expand2(egui::vec2(28.0, 0.0)), &p);
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
        self.overlays(ctx);
        self.tour_frame(ctx);
        ingame::frame(self, ctx);
        pages::log::window(self, ctx);
        self.take_screenshot(ctx);
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
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string();
    match saves::import(std::path::Path::new(source), &save_dir, saves::FALLBACK_TITLE_ID, &stamp) {
        Ok(i) => {
            println!("imported {} files into {}", i.files, saves::account_dir(&save_dir).display());
            if let Some(b) = i.backup {
                println!("the previous save is in {}", b.display());
            }
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("import failed: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn main() -> eframe::Result<()> {
    crash::install();
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--import-save") {
        let code = args.get(i + 1).map(|s| import_save_cli(s)).unwrap_or(std::process::ExitCode::FAILURE);
        std::process::exit(if code == std::process::ExitCode::SUCCESS { 0 } else { 1 });
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([1120.0, 720.0])
        .with_title("EmblemWeave");
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    eframe::run_native(
        "EmblemWeave",
        eframe::NativeOptions { viewport, ..Default::default() },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
