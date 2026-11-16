//! The landing page: the title, where the project stands, and the next step.

use crate::gamedata::{Severity, TITLE_NAME};
use crate::jobs::JobKind;
use crate::pages::Page;
use crate::theme;
use crate::widgets::{self, Tone};
use crate::{logo, App};
use eframe::egui::{self, RichText};

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
        .inner_margin(egui::Margin::symmetric(22, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                logo::show_lockup(ui, &p, 200.0_f32.min(ui.available_width() * 0.34));
                ui.add_space(theme::SPACE_L);
                ui.vertical(|ui| {
                    ui.add_space(theme::SPACE_S);
                    let (name, version) = app.game.title.clone().unwrap_or_else(|| (TITLE_NAME.to_string(), String::new()));
                    ui.label(RichText::new(name).font(theme::display(25.0)).color(p.ink));
                    ui.horizontal(|ui| {
                        if !version.is_empty() {
                            widgets::pill(ui, &p, &format!("Version {version}"), Tone::Neutral);
                        }
                        widgets::pill(ui, &p, "Nintendo Switch 2 → Windows x64", Tone::Accent);
                    });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Statically recompiled from your own copy of the game into native code. No emulator.")
                            .size(13.5)
                            .color(p.body),
                    );
                    ui.add_space(theme::SPACE_S + 2.0);
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
                    // A game folder built by an older release keeps that release's engine, so an update can look like it changed
                    // nothing. This is the repair, not only a warning: one press rebuilds the engine and writes out the packed files
                    // the old folder cannot serve in place, which is the mixed folder behind #34.
                    if let Some((built, now)) = ready.then(crate::engine::stale_build).flatten().filter(|_| !building) {
                        let built = built.unwrap_or_else(|| "an EmblemWeave release older than 0.1.20".to_string());
                        ui.add_space(8.0);
                        egui::Frame::new()
                            .fill(theme::alpha(p.warn, 14))
                            .stroke(egui::Stroke::new(1.0_f32, theme::alpha(p.warn, 90)))
                            .corner_radius(egui::CornerRadius::same(theme::RADIUS_S))
                            .inner_margin(egui::Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal_top(|ui| {
                                    ui.label(RichText::new("!").font(theme::semibold(16.0)).color(p.warn));
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new("This game folder was built by another release").font(theme::semibold(13.5)).color(p.ink));
                                        ui.label(
                                            RichText::new(format!(
                                                "Built by {built}; this launcher is {now}. Its old engine treats a packed package the old way. Rebuilding updates it and prepares the package in one run; saves and mods are not touched."
                                            ))
                                            .size(12.0)
                                            .color(p.body),
                                        );
                                        ui.add_space(6.0);
                                        ui.horizontal(|ui| {
                                            if widgets::primary_button(ui, &p, "Repair / Rebuild now", app.game.exefs.is_some() && app.game.romfs.is_some())
                                                .on_hover_text("Clears the compiled game and rebuilds it from your dump with this release's runtime, then writes out any files the package keeps packed. Your dump is only read. If it still fails, the engine folder itself came from an older download: unzip this release into a new folder and move your saves across.")
                                                .clicked()
                                            {
                                                match crate::engine::clean_runtime_build() {
                                                    Ok(()) => app.start(JobKind::Engine),
                                                    Err(e) => app.notify(format!("Could not start the repair: {e}"), Tone::Bad),
                                                }
                                            }
                                            ui.label(RichText::new("Takes as long as a rebuild; later starts are unchanged.").size(11.5).color(p.muted));
                                        });
                                    });
                                });
                            });
                    }
                    if ready && !building {
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

    // The main menu is a carousel rather than another row of buttons: the
    // selected card is the landing point, and the cards on either side preview
    // the sections around it. The rail still has every page, so this is a
    // faster path, not a replacement for the old navigation.
    widgets::section(ui, &p, "Main menu", |ui| {
        carousel(app, ui, &p);
    });

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
    if !crate::engine::ready() && steps.iter().any(|s| !s.0) {
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

/// One-click access to what the player changes most between sessions: which
/// save to play, the save editor, mods and cheats.
fn quick_actions(app: &mut App, ui: &mut egui::Ui, p: &crate::theme::Palette) {
    let save_dir = crate::config::Config::save_dir();
    let profiles = save_dir.as_deref().map(crate::saves::profiles).unwrap_or_default();
    let mods_on = crate::mods::dir().map(|d| crate::mods::list(&d).iter().filter(|m| m.enabled).count()).unwrap_or(0);
    let cheats_on: usize = crate::cheats::dir().map(|d| crate::cheats::load_enabled(&d).values().map(|v| v.len()).sum()).unwrap_or(0);
    let running = crate::platform::game_running();
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
                for pr in &profiles {
                    if ui.add_enabled(!running, egui::Button::new(&pr.name)).on_disabled_hover_text("Close the game first").clicked() {
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
        if widgets::button(ui, p, &format!("Mods ({mods_on} on)"), true).clicked() {
            app.page = crate::pages::Page::Mods;
        }
        if widgets::button(ui, p, &format!("Cheats ({cheats_on} on)"), true).clicked() {
            app.page = crate::pages::Page::Cheats;
        }
    });
    if let (Some(pr), Some(dir)) = (load, save_dir) {
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

/// What a carousel card does when the front card is opened.
#[derive(Clone, Copy)]
enum MenuAction {
    Play,
    Open(Page),
}

struct MenuCard {
    icon: &'static str,
    eyebrow: &'static str,
    title: &'static str,
    body: &'static str,
    status: String,
    tone: Tone,
    action: MenuAction,
}

/// The menu cards are computed once per frame, not once per painted card: the
/// statuses read the same game, save and mod state the rest of Overview uses.
fn menu_cards(app: &App) -> Vec<MenuCard> {
    let save_dir = crate::config::Config::save_dir();
    let profiles = save_dir.as_deref().map(crate::saves::profiles).unwrap_or_default();
    let mods_on = crate::mods::dir().map(|d| crate::mods::list(&d).iter().filter(|m| m.enabled).count()).unwrap_or(0);
    let cheats_on: usize = crate::cheats::dir().map(|d| crate::cheats::load_enabled(&d).values().map(|v| v.len()).sum()).unwrap_or(0);
    let ready = crate::engine::ready();
    let running = app.running(JobKind::Boot).is_some();
    let building = app.running(JobKind::Engine).is_some();
    let (play_status, play_tone) = if running {
        ("RUNNING".to_string(), Tone::Ok)
    } else if building {
        ("BUILDING".to_string(), Tone::Accent)
    } else if ready {
        ("READY".to_string(), Tone::Ok)
    } else {
        ("BUILD FIRST".to_string(), Tone::Warn)
    };
    let (data_status, data_tone) = if app.game.exefs.is_some() && app.game.romfs.is_some() {
        ("DETECTED".to_string(), Tone::Ok)
    } else {
        ("NEEDS DATA".to_string(), Tone::Warn)
    };
    let (recompile_status, recompile_tone) = match &app.last_pipeline {
        Some(s) if s.verified.is_some_and(|(ok, n)| ok == n) => ("VERIFIED".to_string(), Tone::Ok),
        Some(_) => ("LIFTED".to_string(), Tone::Accent),
        None => ("NOT RUN".to_string(), Tone::Neutral),
    };
    let saves_status = if profiles.is_empty() { "NO PROFILES".to_string() } else { format!("{} SAVED", profiles.len()) };
    vec![
        MenuCard {
            icon: "▶",
            eyebrow: "PLAY",
            title: "Fortune's Weave",
            body: "Start the native build, or build it from your own dump if this is a fresh install.",
            status: play_status,
            tone: play_tone,
            action: MenuAction::Play,
        },
        MenuCard {
            icon: "📁",
            eyebrow: "START HERE",
            title: "Game Setup",
            body: "Point at a folder, .nup, .nx or .nsp and verify the dump before anything is compiled.",
            status: data_status,
            tone: data_tone,
            action: MenuAction::Open(Page::Setup),
        },
        MenuCard {
            icon: "💾",
            eyebrow: "CONTINUE",
            title: "Saves",
            body: "Profiles, slots, backups and the editor for the save you are playing.",
            status: saves_status,
            tone: Tone::Accent,
            action: MenuAction::Open(Page::Saves),
        },
        MenuCard {
            icon: "📦",
            eyebrow: "LIBRARY",
            title: "Mods",
            body: "Lay RomFS mods over the game, browse GameBanana and keep each install switchable.",
            status: format!("{mods_on} ENABLED"),
            tone: if mods_on > 0 { Tone::Ok } else { Tone::Neutral },
            action: MenuAction::Open(Page::Mods),
        },
        MenuCard {
            icon: "⚡",
            eyebrow: "LIBRARY",
            title: "Cheats",
            body: "Quick per-unit cheats, Atmosphere codes and Cheat Engine tables, applied while the game runs.",
            status: format!("{cheats_on} ACTIVE"),
            tone: if cheats_on > 0 { Tone::Accent } else { Tone::Neutral },
            action: MenuAction::Open(Page::Cheats),
        },
        MenuCard {
            icon: "⚙",
            eyebrow: "PIPELINE",
            title: "Recompilation",
            body: "Watch the lift, emit, compile and verify stages, with the current run and its report.",
            status: recompile_status,
            tone: recompile_tone,
            action: MenuAction::Open(Page::Recompile),
        },
        MenuCard {
            icon: "🖥",
            eyebrow: "TUNING",
            title: "Graphics",
            body: "Display, upscalers, frame generation, texture quality and quieter shader compilation.",
            status: format!("{} · {}", app.config.graphics.upscaling.to_uppercase(), app.config.graphics.resolution.to_uppercase()),
            tone: Tone::Accent,
            action: MenuAction::Open(Page::Graphics),
        },
        MenuCard {
            icon: "ℹ",
            eyebrow: "PROJECT",
            title: "About",
            body: "This build, updates, portable storage, the source mirrors and everyone who made it.",
            status: format!("v{}", env!("CARGO_PKG_VERSION")),
            tone: Tone::Neutral,
            action: MenuAction::Open(Page::About),
        },
    ]
}

/// The overview carousel. It keeps the rail's job intact and adds a faster,
/// more visual way to reach the same pages. The cards on either side are
/// previews: clicking one brings it to the front, and clicking the front card
/// opens it. Arrow keys work while the pointer is over the carousel.
fn carousel(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    let cards = menu_cards(app);
    let n = cards.len();
    if n == 0 {
        return;
    }
    let id = egui::Id::new("overview-carousel-index");
    let index = ui.ctx().data(|d| d.get_temp::<usize>(id).unwrap_or(0)).min(n - 1);
    let mut next = index;
    let width = ui.available_width().max(1.0);
    let height = if width < 620.0 { 230.0 } else { 258.0 };
    let (canvas, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let mut action = None;
    let mut focus_hover = false;

    if ui.is_rect_visible(canvas) {
        let center_w = (canvas.width() * 0.42).clamp(300.0, 460.0).min(canvas.width() - 24.0).max(240.0);
        let center = egui::Rect::from_center_size(canvas.center(), egui::vec2(center_w, height - 10.0));
        let side_w = (center_w * 0.52).clamp(170.0, 250.0);
        let side_offset = center_w * 0.52 + side_w * 0.36;
        // Both side cards have to fit inside the canvas. The old threshold
        // allowed the visible edges of a card to sit outside the panel, which
        // read as a clipped carousel at 1280 wide. A narrower window keeps the
        // centre card and the arrows clean instead.
        let show_sides = n > 1 && canvas.width() >= side_offset * 2.0 + side_w + 48.0;
        let side_size = egui::vec2(side_w, height - 22.0);
        let left_index = (index + n - 1) % n;
        let right_index = (index + 1) % n;

        if show_sides {
            let left = egui::Rect::from_center_size(center.center() - egui::vec2(side_offset, 4.0), side_size);
            let response = ui.interact(left, ui.id().with(("carousel-card", left_index)), egui::Sense::click());
            let hover = ui.ctx().animate_bool(response.id.with("hover"), response.hovered());
            paint_menu_card(ui, p, left, &cards[left_index], false, hover);
            if response.clicked() {
                next = left_index;
            }
            focus_hover |= response.hovered();
        }

        if show_sides {
            let right = egui::Rect::from_center_size(center.center() + egui::vec2(side_offset, -4.0), side_size);
            let response = ui.interact(right, ui.id().with(("carousel-card", right_index)), egui::Sense::click());
            let hover = ui.ctx().animate_bool(response.id.with("hover"), response.hovered());
            paint_menu_card(ui, p, right, &cards[right_index], false, hover);
            if response.clicked() {
                next = right_index;
            }
            focus_hover |= response.hovered();
        }

        let response = ui.interact(center, ui.id().with(("carousel-card", index)), egui::Sense::click());
        let hover = ui.ctx().animate_bool(response.id.with("hover"), response.hovered());
        paint_menu_card(ui, p, center, &cards[index], true, hover.max(0.25));
        if response.clicked() {
            action = Some(cards[index].action);
        }
        focus_hover |= response.hovered();
        if focus_hover {
            ui.ctx().request_repaint();
        }
    }

    if focus_hover {
        let (left, right, enter) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Enter),
            )
        });
        if left {
            next = (next + n - 1) % n;
        }
        if right {
            next = (next + 1) % n;
        }
        if enter && action.is_none() {
            action = Some(cards[index].action);
        }
    }

    ui.add_space(theme::SPACE_XS);
    let mut goto = None;
    let controls_room = ui.available_width();
    ui.horizontal(|ui| {
        if widgets::button(ui, p, "<", n > 1).on_hover_text("Previous card").clicked() {
            goto = Some((next + n - 1) % n);
        }
        ui.label(RichText::new(format!("{:02} / {:02}", next + 1, n)).font(theme::mono(11.0)).color(p.muted));
        if controls_room > 560.0 {
            for i in 0..n {
                let (dot, response) = ui.allocate_exact_size(egui::vec2(16.0, 18.0), egui::Sense::click());
                let active = i == next;
                let colour = if active { p.accent_hi } else if response.hovered() { p.body } else { p.line_strong };
                ui.painter().circle_filled(dot.center(), if active { 3.5 } else { 2.3 }, colour);
                if response.clicked() {
                    goto = Some(i);
                }
            }
        }
        if widgets::button(ui, p, ">", n > 1).on_hover_text("Next card").clicked() {
            goto = Some((next + 1) % n);
        }
        if controls_room > 760.0 {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new("Click the front card to open  ·  use ← → while hovering").size(11.5).color(p.muted));
            });
        }
    });
    if let Some(g) = goto {
        next = g;
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, next));

    if let Some(action) = action {
        match action {
            MenuAction::Play => {
                if crate::engine::ready() {
                    app.play();
                } else {
                    app.page = Page::Setup;
                    app.notify("Choose your game data first, then build the game from Overview", Tone::Warn);
                }
            }
            MenuAction::Open(page) => app.page = page,
        }
    }
}

/// A card in the carousel. The selected card is larger, has the accent glow,
/// and shows its full description. The side cards are dimmed previews, as in
/// the reference carousel, and keep the same hover response.
fn paint_menu_card(ui: &egui::Ui, p: &theme::Palette, rect: egui::Rect, card: &MenuCard, focus: bool, hover: f32) {
    let radius = egui::CornerRadius::same(theme::RADIUS_M + if focus { 2 } else { 0 });
    let painter = ui.painter();
    let shadow = rect.translate(egui::vec2(0.0, 8.0)).expand2(egui::vec2(5.0, 3.0));
    painter.rect_filled(shadow, radius, theme::alpha(egui::Color32::BLACK, if focus { 120 } else { 80 }));

    if focus || hover > 0.05 {
        let glow = if focus { 1.0 } else { hover * 0.65 };
        for (spread, alpha) in [(2.0, 74.0), (5.0, 38.0), (9.0, 18.0)] {
            painter.rect_stroke(
                rect.expand(spread),
                egui::CornerRadius::same((theme::RADIUS_M as f32 + spread) as u8),
                egui::Stroke::new(1.0_f32, theme::alpha(p.accent_hi, (alpha * glow) as u8)),
                egui::StrokeKind::Outside,
            );
        }
    }

    painter.rect_filled(rect, radius, p.panel_hi);
    widgets::gradient_fill(
        painter,
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.center().y)),
        radius.nw as f32,
        theme::alpha(p.accent, if focus { 46 } else { 18 }),
        egui::Color32::TRANSPARENT,
    );

    let art_h = if focus { 92.0 } else { (rect.height() * 0.48).clamp(76.0, 112.0) };
    let art = egui::Rect::from_min_max(
        rect.min + egui::vec2(1.0, 1.0),
        egui::pos2(rect.max.x - 1.0, rect.top() + art_h),
    );
    let art_painter = painter.with_clip_rect(rect);
    widgets::gradient_fill(
        &art_painter,
        art,
        radius.nw as f32 - 1.0,
        theme::alpha(p.accent, if focus { 62 } else { 28 }),
        egui::Color32::TRANSPARENT,
    );
    let sun = egui::pos2(art.right() - 48.0, art.center().y + 3.0);
    // A glyph watermark and a thin orbit read as artwork; a filled disc read as
    // a pale moon behind the title, especially on the dimmed side cards.
    art_painter.text(
        sun,
        egui::Align2::CENTER_CENTER,
        card.icon,
        egui::FontId::proportional(if focus { 34.0 } else { 28.0 }),
        theme::alpha(p.accent_hi, if focus { 70 } else { 42 }),
    );
    art_painter.circle_stroke(sun, if focus { 29.0 } else { 22.0 }, egui::Stroke::new(1.0_f32, theme::alpha(p.accent_hi, 58)));
    art_painter.circle_stroke(sun, if focus { 40.0 } else { 31.0 }, egui::Stroke::new(1.0_f32, theme::alpha(p.accent_hi, 24)));
    let horizon = art.bottom() - 13.0;
    art_painter.line_segment(
        [egui::pos2(art.left() + 16.0, horizon), egui::pos2(art.right() - 16.0, horizon)],
        egui::Stroke::new(1.0_f32, theme::alpha(p.accent_hi, 45)),
    );
    for i in 0..4 {
        let x = art.left() + 22.0 + i as f32 * 18.0;
        art_painter.line_segment(
            [egui::pos2(x, horizon), egui::pos2(x + 22.0, art.bottom())],
            egui::Stroke::new(1.0_f32, theme::alpha(p.accent_hi, 18)),
        );
    }

    let badge = egui::Rect::from_min_size(rect.min + egui::vec2(17.0, 16.0), egui::vec2(36.0, 36.0));
    art_painter.rect_filled(badge, egui::CornerRadius::same(10), theme::alpha(p.accent, 35));
    art_painter.rect_stroke(badge, egui::CornerRadius::same(10), egui::Stroke::new(1.0_f32, theme::alpha(p.accent_hi, 110)), egui::StrokeKind::Inside);
    art_painter.text(badge.center(), egui::Align2::CENTER_CENTER, card.icon, egui::FontId::proportional(17.0), p.accent_hi);
    if focus {
        art_painter.text(
            egui::pos2(rect.right() - 18.0, rect.top() + 26.0),
            egui::Align2::RIGHT_CENTER,
            "SELECTED",
            theme::semibold(9.0),
            theme::alpha(p.accent_hi, 220),
        );
    }

    let content = painter.with_clip_rect(rect);
    let x = rect.left() + 18.0;
    let text_w = (rect.width() - 36.0).max(40.0);
    let kicker_y = rect.top() + art_h + 12.0;
    content.text(
        egui::pos2(x, kicker_y),
        egui::Align2::LEFT_TOP,
        card.eyebrow,
        theme::semibold(9.5),
        p.muted,
    );
    let title = content.layout(
        card.title.to_owned(),
        theme::display(if focus { 23.0 } else { 19.0 }),
        p.ink,
        text_w,
    );
    let title_y = kicker_y + 16.0;
    content.galley(egui::pos2(x, title_y), title.clone(), p.ink);
    if focus {
        // The side cards are previews: their body would be half-covered by the
        // front card, so it is left out rather than shown as cut-off text.
        let body = content.layout(card.body.to_owned(), egui::FontId::proportional(12.5), p.body, text_w);
        content.galley(egui::pos2(x, title_y + title.size().y + 6.0), body, p.body);
    }

    let footer_y = rect.bottom() - 19.0;
    content.text(
        egui::pos2(x, footer_y),
        egui::Align2::LEFT_CENTER,
        &card.status,
        theme::semibold(9.5),
        card.tone.colour(p),
    );
    content.text(
        egui::pos2(rect.right() - 18.0, footer_y),
        egui::Align2::RIGHT_CENTER,
        if focus { "OPEN  →" } else { "VIEW" },
        theme::semibold(9.5),
        if focus { p.accent_hi } else { p.muted },
    );
    content.rect_stroke(rect, radius, egui::Stroke::new(if focus { 1.4_f32 } else { 1.0_f32 }, if focus { theme::alpha(p.accent_hi, 205) } else { p.line_strong }), egui::StrokeKind::Inside);
}
