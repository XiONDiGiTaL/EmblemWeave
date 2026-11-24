//! The landing page: the title, where the project stands, and the next step.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::gamedata::{Severity, TITLE_NAME};
use crate::jobs::JobKind;
use crate::pages::Page;
use crate::theme;
use crate::widgets::{self, Tone};
use crate::{logo, App};
use eframe::egui::{self, Color32, RichText, Sense, Vec2};

/// A lightweight panel opened from the header or a status button. Only one is
/// open at a time, so the page never accumulates floating cards.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Popup {
    WhatsNew,
    QuickHelp,
    GameStatus,
    BuildStatus,
    SetupGuide,
}

/// One honest word for the sidebar: what the user can do right now.
pub fn readiness(app: &App) -> (&'static str, Tone) {
    if app.running(JobKind::Boot).is_some() {
        ("Runtime running", Tone::Accent)
    } else if crate::engine::ready() {
        // The built game is the strongest thing the user can do, and it is what
        // the Overview card says; without this the sidebar still read "Setup
        // incomplete" over a card that read "Ready to play".
        ("Ready to play", Tone::Accent)
    } else if app.game.bootable_inputs() {
        ("Boot test available", Tone::Accent)
    } else if app.game.exefs.is_some() {
        ("Setup incomplete", Tone::Warn)
    } else {
        ("No game data", Tone::Neutral)
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    ui.add_space(theme::SPACE_L);

    // Hero: the lockup over a faint electric wash, then the title card.
    egui::Frame::new()
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0_f32, p.line))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_M + 4))
        .inner_margin(egui::Margin::symmetric(24, 20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                logo::show_lockup(ui, &p, 244.0_f32.min(ui.available_width() * 0.38));
                ui.add_space(theme::SPACE_L);
                ui.vertical(|ui| {
                    ui.add_space(theme::SPACE_M);
                    let (name, version) = app.game.title.clone().unwrap_or_else(|| (TITLE_NAME.to_string(), String::new()));
                    ui.label(RichText::new(name).font(theme::display(26.0)).color(p.ink));
                    ui.horizontal(|ui| {
                        if !version.is_empty() {
                            widgets::pill(ui, &p, &format!("Version {version}"), Tone::Neutral);
                        }
                        widgets::pill(ui, &p, "Nintendo Switch 2 → Windows x64", Tone::Accent);
                    });
                    ui.add_space(theme::SPACE_S);
                    ui.label(
                        RichText::new("Statically recompiled from your own copy of the game into native code. No emulator.")
                            .size(14.0)
                            .color(p.body),
                    );
                    ui.add_space(theme::SPACE_M);
                    let ready = crate::engine::ready();
                    let building = app.running(JobKind::Engine).is_some();
                    ui.horizontal(|ui| {
                        if widgets::primary_button(ui, &p, "▶  Play", ready && !building)
                            .on_disabled_hover_text("Build the game first: it is compiled from your dump once, on this PC.")
                            .clicked()
                        {
                            app.play();
                        }
                        let label = if ready { "Rebuild" } else { "Build the game" };
                        let can_build = app.game.exefs.is_some() && app.game.romfs.is_some() && !building;
                        if widgets::button(ui, &p, label, can_build)
                            .on_hover_text("Compiles the game from your dump and translates its shaders. The first build takes a while; later ones only redo what changed.")
                            .clicked()
                        {
                            app.start(JobKind::Engine);
                        }
                    });
                    ui.add_space(4.0);
                    let missing = crate::engine::missing_tools();
                    let (text, colour) = if building {
                        ("Building the game from your dump; progress is in the status bar and on Diagnostics.".to_string(), p.body)
                    } else if ready {
                        ("Ready to play. Saves are kept in EmblemWeave's save folder; RomFS mods go in the run folder's mods folder.".to_string(), p.ok)
                    } else if !missing.is_empty() {
                        (format!("To build the game, install: {}.", missing.join(", ")), p.warn)
                    } else {
                        ("Not built yet: press Build the game. It runs once, on this PC, from your own dump.".to_string(), p.warn)
                    };
                    ui.label(RichText::new(text).size(12.5).color(colour));
                    // A game folder built by an older release keeps that release's engine, which is why an update can look like it changed
                    // nothing: name the version and the one press that replaces it (#24, reported again on a build from before 0.1.20).
                    if ready && !building {
                        let now = env!("CARGO_PKG_VERSION");
                        // No stamp at all is a build from before the stamp existed, which is older than any release that writes one.
                        let stale = match crate::engine::built_by() {
                            Some(v) => v != now,
                            None => true,
                        };
                        if stale {
                            let built = crate::engine::built_by().unwrap_or_else(|| "an earlier release".to_string());
                            ui.label(
                                RichText::new(format!(
                                    "This game folder was built by {built}; press Rebuild to update its engine to {now}."
                                ))
                                .size(12.5)
                                .color(p.warn),
                            );
                        }
                        // Code mods installed or updated since the last build bring hook sites the game lacks (their code would not run),
                        // or a .sites file has a problem that will stop the build.
                        if let Some(note) = crate::sites::installed_note() {
                            ui.label(RichText::new(note).size(12.5).color(p.warn));
                        }
                    }
                    // One click for the tool everyone trips on: winget does the
                    // download and the permission prompt, the launcher only
                    // starts it. Restarting is needed because a new PATH is
                    // only read by a new process.
                    if !building && !missing.is_empty() {
                        ui.add_space(6.0);
                        let winget = crate::engine::have_winget();
                        let label = if missing.len() == 1 { "Install the missing build tool" } else { "Install the missing build tools" };
                        if widgets::button(ui, &p, label, winget)
                            .on_hover_text(if winget {
                                format!(
                                    "Runs winget in a new window for: {}. Visual Studio is a few GB, the others are small. Accept the Windows permission prompt, then press Check again on Requirements; a restart is only needed if a tool still is not found.",
                                    missing.join(", ")
                                )
                            } else {
                                "winget is not available on this PC. Install App Installer from the Microsoft Store, or install the tools by hand from docs/TROUBLESHOOTING.md.".to_string()
                            })
                            .clicked()
                        {
                            let n = missing.len();
                            match crate::engine::start_install() {
                                Ok(_) => app.notify(
                                    format!("Installing {n} build tool(s) in a new window. Accept the Windows prompt, then restart EmblemWeave and press Build the game."),
                                    Tone::Ok,
                                ),
                                Err(e) => app.notify(e, Tone::Bad),
                            }
                        }
                    }
                    ui.add_space(theme::SPACE_S);
                    quick_actions(app, ui, &p);
                });
            });
        });
    ui.add_space(theme::SPACE_M);

    // Three status cards.
    ui.columns(3, |cols| {
        let worst = app.game.worst();
        let (data_text, data_tone) = match worst {
            Severity::Ok | Severity::Info => ("Ready", Tone::Ok),
            Severity::Warn => ("Needs attention", Tone::Warn),
            Severity::Error => ("Incomplete", Tone::Bad),
        };
        // The reason first and the version line under it: "Needs attention"
        // has to say what needs it, and the version still belongs on the card.
        let (data_detail, data_note) = match app.game.attention_line() {
            Some(reason) => (reason, Some(summary_of_data(app))),
            None => (summary_of_data(app), None),
        };
        card(&mut cols[0], &p, "Game data", data_text, data_tone, &data_detail, data_note.as_deref());

        let (rtext, rtone, rdetail) = match &app.last_pipeline {
            Some(s) => match (&s.report, s.verified) {
                (Some(r), Some((ok, n))) => (
                    "Recompiled".to_string(),
                    if ok == n { Tone::Ok } else { Tone::Warn },
                    format!("{} functions, {:.2}% decode, {ok}/{n} verified", r.lifted_functions, r.decode_coverage * 100.0),
                ),
                (Some(r), None) => ("Lifted".to_string(), Tone::Accent, format!("{} functions; native build not verified", r.lifted_functions)),
                _ => ("Run incomplete".to_string(), Tone::Warn, String::new()),
            },
            None => ("Not run this session".to_string(), Tone::Neutral, "Run the pipeline on the Recompilation page".to_string()),
        };
        card(&mut cols[1], &p, "Recompilation", &rtext, rtone, &rdetail, None);

        let (btext, btone, bdetail) = match &app.last_boot {
            Some((r, ms)) => (
                format!("Reached {}", r.phase),
                Tone::Accent,
                format!(
                    "{} instructions, {} threads, {:.1}s. {}",
                    crate::fmt_count(r.steps),
                    r.threads,
                    *ms as f32 / 1000.0,
                    widgets::demangle_line(&r.stop.describe())
                ),
            ),
            None => ("No run yet".to_string(), Tone::Neutral, "Run the boot test to execute the game's code".to_string()),
        };
        card(&mut cols[2], &p, "Runtime", &btext, btone, &bdetail, None);
    });
    ui.add_space(theme::SPACE_M);

    // Next steps, while setup is incomplete.
    let steps = [
        (app.game.exefs.is_some() && app.game.romfs.is_some(), "Choose your game folder", "Game Setup finds ExeFS, RomFS and control.nacp", Page::Setup),
        (app.game.bootable_inputs(), "Decode the main module", "One-time decode of the ZBIC-compressed executable", Page::Setup),
        (app.last_pipeline.is_some(), "Run the recompilation pipeline", "Lift, emit, compile and verify the game's code", Page::Recompile),
        (app.last_boot.is_some(), "Run the boot test", "Execute the game through startup and see where it stops", Page::Overview),
    ];
    if steps.iter().any(|s| !s.0) {
        widgets::section(ui, &p, "Next steps", |ui| {
            for (i, (done, title, help, page)) in steps.iter().enumerate() {
                ui.horizontal(|ui| {
                    let mark = if *done { RichText::new("✔").color(p.ok) } else { RichText::new(format!("{}", i + 1)).color(p.accent_hi) };
                    ui.label(mark.font(theme::semibold(14.0)));
                    ui.label(RichText::new(*title).font(theme::semibold(14.0)).color(if *done { p.body } else { p.ink }));
                    ui.label(RichText::new(*help).size(12.5).color(p.muted));
                    if !*done && *page != Page::Overview {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.link(format!("Open {}", page.label())).clicked() {
                                app.page = *page;
                            }
                        });
                    }
                });
            }
        });
    }

    ui.columns(2, |cols| {
        widgets::section(&mut cols[0], &p, "Current configuration", |ui| {
            widgets::kv(ui, &p, "Language", &app.config.system.language);
            widgets::kv(ui, &p, "Profile", &app.config.system.user_name);
            widgets::kv(ui, &p, "Theme", app.config.theme.label());
            widgets::kv(ui, &p, "Instruction budget", &crate::fmt_count(app.config.runtime.max_steps));
            widgets::kv(ui, &p, "Guest heap", &format!("{} MiB", app.config.runtime.heap_mib));
        });
        widgets::section(&mut cols[1], &p, "Recent activity", |ui| {
            if app.activity.is_empty() {
                ui.label(RichText::new("Nothing yet").color(p.muted));
            }
            for a in app.activity.iter().rev().take(7) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&a.at).font(theme::mono(11.5)).color(p.muted));
                    ui.add(egui::Label::new(RichText::new(&a.text).size(13.0).color(a.tone.colour(&p))).wrap());
                });
            }
        });
    });
}

fn summary_of_data(app: &App) -> String {
    let g = &app.game;
    match (&g.exefs, &g.title) {
        (None, _) => "Choose the folder with your extracted game".into(),
        (Some(_), Some((_, v))) => format!("Version {v}, {} RomFS entries", g.romfs_entries),
        (Some(_), None) => format!("{} RomFS entries", g.romfs_entries),
    }
}

/// The three cards share one body height: they sit side by side, and a card with
/// more to say must not make the row ragged. The tallest case is a three-line
/// reason with the version line under it, so that is what every card reserves.
const CARD_BODY: f32 = 104.0;

fn card(ui: &mut egui::Ui, p: &theme::Palette, title: &str, value: &str, tone: Tone, detail: &str, note: Option<&str>) {
    widgets::section(ui, p, title, |ui| {
        ui.set_min_height(CARD_BODY);
        ui.label(RichText::new(value).font(theme::display(20.0)).color(tone.colour(p)));
        ui.add_space(2.0);
        ui.label(RichText::new(detail).size(12.5).color(p.body));
        if let Some(note) = note {
            ui.add_space(2.0);
            ui.label(RichText::new(note).size(12.5).color(p.muted));
        }
    });
}

/// The Overview's save/mod/cheat summary is display-only, but reading it walks
/// the save folder, every mod tree and the cheats file. Doing that on every
/// hover frame was measurable, so keep the values for one second: long enough
/// to stay current on the page, short enough that a change made elsewhere
/// appears as soon as the player returns.
const QUICK_TTL: Duration = Duration::from_secs(1);

struct QuickState {
    profiles: Vec<crate::saves::Profile>,
    mods_label: String,
    cheats_label: String,
    running: bool,
}

thread_local! {
    static QUICK: RefCell<Option<(Instant, Rc<QuickState>)>> = const { RefCell::new(None) };
}

fn quick_state() -> Rc<QuickState> {
    QUICK.with(|cell| {
        let mut cache = cell.borrow_mut();
        let stale = cache
            .as_ref()
            .map_or(true, |(at, _)| at.elapsed() >= QUICK_TTL);
        if stale {
            let save_dir = crate::config::Config::save_dir();
            let profiles = save_dir
                .as_deref()
                .map(crate::saves::profiles)
                .unwrap_or_default();
            let mods_on = crate::mods::dir()
                .map(|d| crate::mods::list(&d).iter().filter(|m| m.enabled).count())
                .unwrap_or(0);
            let cheats_on: usize = crate::cheats::dir()
                .map(|d| {
                    crate::cheats::load_enabled(&d)
                        .values()
                        .map(|v| v.len())
                        .sum()
                })
                .unwrap_or(0);
            *cache = Some((
                Instant::now(),
                Rc::new(QuickState {
                    profiles,
                    mods_label: format!("Mods ({mods_on} on)"),
                    cheats_label: format!("Cheats ({cheats_on} on)"),
                    running: crate::platform::game_running(),
                }),
            ));
        }
        cache
            .as_ref()
            .expect("the quick-action cache was just refreshed")
            .1
            .clone()
    })
}

/// One-click access to what the player changes most between sessions: which
/// save to play, the save editor, mods and cheats.
fn quick_actions(app: &mut App, ui: &mut egui::Ui, p: &crate::theme::Palette) {
    let quick = quick_state();
    let profiles = &quick.profiles;
    let mut load: Option<crate::saves::Profile> = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Save").color(p.muted));
        egui::ComboBox::from_id_salt("quick-profile")
            .selected_text(if profiles.is_empty() { "current save" } else { "load a profile…" })
            .width(150.0)
            .show_ui(ui, |ui| {
                if profiles.is_empty() {
                    ui.label("No profiles yet: keep one on the Saves page.");
                }
                for pr in profiles {
                    if ui.add_enabled(!quick.running, egui::Button::new(&pr.name)).on_disabled_hover_text("Close the game first").clicked() {
                        load = Some(pr.clone());
                    }
                }
            });
        if widgets::button(ui, p, "Saves", true)
            .on_hover_text("Open the Saves page: profiles, the slots in this save and the editor")
            .clicked()
        {
            app.page = crate::pages::Page::Saves;
        }
        if widgets::button(ui, p, &quick.mods_label, true).clicked() {
            app.page = crate::pages::Page::Mods;
        }
        if widgets::button(ui, p, &quick.cheats_label, true).clicked() {
            app.page = crate::pages::Page::Cheats;
        }
    });
    if let Some(pr) = load {
        let Some(dir) = crate::config::Config::save_dir() else {
            return;
        };
        let title = app
            .game
            .title_id
            .or_else(|| app.game.nacp.as_deref().and_then(crate::saves::nacp_save_owner))
            .unwrap_or(crate::saves::FALLBACK_TITLE_ID);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string();
        match crate::saves::load_profile(&dir, &pr, title, &stamp) {
            Ok(_) => {
                app.saves_ui.editor = None;
                app.notify(format!("Loaded save profile {}; the previous save is in the backups folder", pr.name), Tone::Ok);
            }
            Err(e) => app.notify(format!("Could not load the profile: {e}"), Tone::Bad),
        }
    }
}

struct SetupStage {
    name: &'static str,
    ok: bool,
    counts: bool,
}

/// Draws any open overview popup (Quick Help, What's New, etc.).
pub fn popups(app: &mut App, ctx: &egui::Context) {
    if app.screenshot.is_some() || app.tour.is_some() {
        return;
    }
    let Some(kind) = app.overview_popup else {
        return;
    };
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.overview_popup = None;
        return;
    }
    let p = app.config.theme.palette();
    let (title, subtitle) = match kind {
        Popup::WhatsNew => {
            let update = match &app.update.state {
                crate::UpdateState::Ready(release) => format!("{} is available", release.tag),
                crate::UpdateState::UpToDate(tag) => format!("Up to date ({tag})"),
                _ => format!("Launcher v{}", env!("CARGO_PKG_VERSION")),
            };
            ("What's new", update)
        }
        Popup::QuickHelp => ("Quick help", "Checks, logs and troubleshooting".to_string()),
        Popup::GameStatus => ("Game status", data_summary(app).0),
        Popup::BuildStatus => {
            let summary = if app.running(JobKind::Engine).is_some() {
                "Building now"
            } else if crate::engine::ready() {
                "Native build ready"
            } else {
                "Not built yet"
            };
            ("Build status", summary.to_string())
        }
        Popup::SetupGuide => {
            let stages = setup_stages(app);
            let total = stages.iter().filter(|s| s.counts).count();
            let done = if crate::engine::ready() {
                total
            } else {
                stages
                    .iter()
                    .filter(|s| s.counts && s.ok)
                    .count()
            };
            ("Setup guide", format!("{done}/{total} ready"))
        }
    };
    let mut close = false;
    egui::Area::new(egui::Id::new(("overview-popup", kind as u8)))
        .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-18.0, 54.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::alpha(p.surface, 238))
                .stroke(egui::Stroke::new(1.0_f32, p.line_strong))
                .corner_radius(egui::CornerRadius::same(14))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 10],
                    blur: 28,
                    spread: 0,
                    color: Color32::from_black_alpha(150),
                })
                .show(ui, |ui| {
                    ui.set_min_width(280.0);
                    ui.vertical(|ui| {
                        ui.add_space(theme::SPACE_S);
                        ui.label(
                            RichText::new(title)
                                .font(theme::semibold(15.0))
                                .color(p.ink),
                        );
                        ui.add_space(theme::SPACE_XS);
                        ui.label(RichText::new(subtitle).size(13.0).color(p.body));
                        ui.add_space(theme::SPACE_M);
                        ui.horizontal(|ui| {
                            if widgets::button(ui, &p, "Close", true).clicked() {
                                close = true;
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if widgets::button(ui, &p, "Open page", true).clicked() {
                                    match kind {
                                        Popup::WhatsNew => app.page = Page::About,
                                        Popup::QuickHelp => app.page = Page::Troubleshoot,
                                        Popup::GameStatus => app.page = Page::Overview,
                                        Popup::BuildStatus => app.page = Page::Recompile,
                                        Popup::SetupGuide => app.page = Page::Setup,
                                    }
                                    close = true;
                                }
                            });
                        });
                    });
                });
        });
    if close {
        app.overview_popup = None;
    }
}

fn setup_stages(app: &App) -> Vec<SetupStage> {
    let mut stages = Vec::new();
    let exefs = app.game.exefs.is_some();
    let romfs = app.game.romfs.is_some();
    let title = app.game.title.is_some();
    let control = app.game.nacp.is_some();
    let tools = crate::engine::missing_tools().is_empty();
    let built = crate::engine::ready();
    stages.push(SetupStage {
        name: "ExeFS",
        ok: exefs,
        counts: true,
    });
    stages.push(SetupStage {
        name: "RomFS",
        ok: romfs,
        counts: true,
    });
    stages.push(SetupStage {
        name: "Title",
        ok: title,
        counts: true,
    });
    stages.push(SetupStage {
        name: "control.nacp",
        ok: control,
        counts: true,
    });
    stages.push(SetupStage {
        name: "Build tools",
        ok: tools,
        counts: true,
    });
    stages.push(SetupStage {
        name: "Game built",
        ok: built,
        counts: true,
    });
    stages
}

fn data_summary(app: &App) -> (String, Tone) {
    let worst = app.game.worst();
    match worst {
        Severity::Ok | Severity::Info => ("Ready".to_string(), Tone::Ok),
        Severity::Warn => ("Needs attention".to_string(), Tone::Warn),
        Severity::Error => ("Incomplete".to_string(), Tone::Bad),
    }
}
