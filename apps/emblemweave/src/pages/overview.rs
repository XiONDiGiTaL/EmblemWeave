//! The landing page: the game, whether it can run, and the one action that
//! moves the player forward. Setup and build reporting use the same detection
//! and job state as the rest of the launcher; nothing here is a mock status.

use crate::gamedata::{Severity, TITLE_NAME};
use crate::jobs::JobKind;
use crate::pages::Page;
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, Color32, RichText, Sense, Vec2};

/// One honest word for the sidebar: what the user can do right now.
pub fn readiness(app: &App) -> (&'static str, Tone) {
    if app.running(JobKind::Boot).is_some() {
        ("Runtime running", Tone::Accent)
    } else if crate::engine::ready() {
        ("Ready to play", Tone::Accent)
    } else if app.game.bootable_inputs() {
        ("Boot test available", Tone::Accent)
    } else if app.game.exefs.is_some() {
        ("Setup incomplete", Tone::Warn)
    } else {
        ("No game data", Tone::Neutral)
    }
}

#[derive(Clone, Copy)]
enum Action {
    Open(Page),
    Run(JobKind),
    Play,
    InstallTools,
}

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

/// A newer release updates the launcher, docs and engine sources in place. Put it on the landing page with the one action
/// it needs, rather than making the player find About and wonder which folders to keep (issue #56).
fn update_offer(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    match app.update.state.clone() {
        crate::UpdateState::Ready(release) => {
            let mut install = false;
            let mut skip = false;
            widgets::section(ui, p, "Update available", |ui| {
                ui.label(
                    RichText::new(format!(
                        "EmblemWeave {} is available. Updating replaces the launcher, docs and engine sources in place; your data, \
                         saves, built game and fetched tools stay where they are.",
                        release.tag
                    ))
                    .size(13.0)
                    .color(p.ink),
                );
                ui.add_space(theme::SPACE_XS);
                ui.horizontal_wrapped(|ui| {
                    if widgets::primary_button(ui, p, "Update now", release.asset.is_some())
                        .clicked()
                    {
                        install = true;
                    }
                    if !release.page.is_empty() {
                        ui.hyperlink_to("Release notes", &release.page);
                    }
                    if widgets::button(ui, p, "Skip this version", true).clicked() {
                        skip = true;
                    }
                });
                if release.asset.is_none() {
                    ui.label(
                        RichText::new(
                            "That release has no Windows download; use the release page.",
                        )
                        .size(12.0)
                        .color(p.warn),
                    );
                }
            });
            ui.add_space(theme::SPACE_M);
            if install {
                app.install_update();
            }
            if skip {
                let tag = release.tag;
                app.config.updates.skipped = tag.clone();
                app.update.state = crate::UpdateState::Idle;
                app.notify(
                    format!("Skipped {tag}; undo this on About > Updates"),
                    Tone::Neutral,
                );
            }
        }
        crate::UpdateState::Downloading => {
            widgets::section(ui, p, "Updating EmblemWeave", |ui| {
                widgets::status_line(
                    ui,
                    p,
                    Tone::Accent,
                    "Downloading and writing the new version…",
                    "The launcher restarts when it is ready. Your data and built game are kept.",
                );
                ui.add_space(4.0);
                ui.spinner();
            });
            ui.add_space(theme::SPACE_M);
        }
        _ => {}
    }
}

/// A built engine this copy found in an older EmblemWeave folder, offered before the checklist asks for a build. The build is
/// the expensive part of an install and a new release zip does not carry it, so reusing an older folder's is the difference
/// between updating and recompiling the game from scratch.
fn reuse_offer(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    let Some(found) = app.reuse.clone() else {
        return;
    };
    let built = crate::pages::saves::ago(found.built);
    let when = if built.is_empty() {
        String::new()
    } else {
        format!(", built {built}")
    };
    let mut use_it = false;
    let mut dismiss = false;
    widgets::section(ui, p, "A build is already on this PC", |ui| {
        ui.label(
            RichText::new(format!(
                "{} already has the game built{when}. Use it instead of compiling the engine again here? Your saves, settings \
                 and game data are not moved.",
                found.install.display()
            ))
            .size(13.0)
            .color(p.ink),
        );
        ui.add_space(theme::SPACE_XS);
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, p, "Use this build", true).clicked() {
                use_it = true;
            }
            if widgets::button(ui, p, "Not now", true)
                .on_hover_text("Game Setup > Engine folder can still point at it later")
                .clicked()
            {
                dismiss = true;
            }
        });
    });
    ui.add_space(theme::SPACE_M);
    if use_it {
        match crate::engine::remember_root(&found.root) {
            Ok(root) => {
                app.reuse = None;
                app.notify(format!("Using the build in {}", root.display()), Tone::Ok);
            }
            Err(e) => app.notify(e, Tone::Bad),
        }
    } else if dismiss {
        app.reuse = None;
        let root = found.root.display().to_string();
        if !app.config.reuse_dismissed.contains(&root) {
            app.config.reuse_dismissed.push(root);
        }
    }
}

struct SetupStage {
    name: &'static str,
    detail: String,
    tone: Tone,
    action: Option<Action>,
    action_label: &'static str,
    counts: bool,
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    ui.add_space(theme::SPACE_S);
    reuse_offer(app, ui, &p);
    update_offer(app, ui, &p);

    let stages = setup_stages(app);
    let total = stages.iter().filter(|s| s.counts).count();
    // A built game is the strongest proof that the setup path succeeded: do
    // not show "4 of 6" beside a Play button that is ready.
    let done = if crate::engine::ready() {
        total
    } else {
        stages
            .iter()
            .filter(|s| s.counts && s.tone == Tone::Ok)
            .count()
    };
    let mut action: Option<Action> = None;

    // The primary game area and the status buttons sit side by side, as in a
    // launcher rather than a dashboard. On a narrow window they stack.
    let wide = ui.available_width() >= 900.0;
    if wide {
        let w = ui.available_width();
        let left = ((w - 12.0) * 0.665).floor();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(left, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(left);
                    hero(app, ui, &p, done, total, &mut action);
                },
            );
            ui.add_space(12.0);
            ui.allocate_ui_with_layout(
                Vec2::new((w - left - 12.0).max(300.0), 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width((w - left - 12.0).max(300.0));
                    system_panel(app, ui, &p);
                },
            );
        });
    } else {
        hero(app, ui, &p, done, total, &mut action);
        ui.add_space(theme::SPACE_S);
        system_panel(app, ui, &p);
    }

    ui.add_space(theme::SPACE_S);
    quick_access(app, ui, &p);
    ui.add_space(theme::SPACE_S);

    if let Some(action) = action {
        run_action(app, action);
    }
}

fn hero(
    app: &mut App,
    ui: &mut egui::Ui,
    p: &theme::Palette,
    done: usize,
    total: usize,
    action: &mut Option<Action>,
) {
    let running = app.running(JobKind::Boot).is_some();
    let building = app.running(JobKind::Engine).is_some();
    let ready = crate::engine::ready();
    let data_ready =
        app.game.exefs.is_some() && (app.game.romfs.is_some() || app.game.nup.is_some());
    let missing_tools = crate::engine::missing_tools();
    let (title, version) = app
        .game
        .title
        .clone()
        .unwrap_or_else(|| (TITLE_NAME.to_string(), String::new()));
    let (data_text, data_tone) = data_summary(app);
    let stale = if ready {
        crate::engine::stale_build()
    } else {
        None
    };
    let mut repair_clicked = false;

    let frame = widgets::card_frame(p, egui::Margin::symmetric(18, 14));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        // A quiet brand watermark fills the right half of the game panel. It is
        // intentionally faint: the title and the Play action remain the focus.
        if ui.available_width() > 520.0 {
            let face = ui.max_rect();
            let art = egui::Rect::from_min_size(egui::pos2(face.right() - 226.0, face.top() + 76.0), Vec2::new(188.0, 90.0));
            crate::logo::lockup_tinted(ui.painter(), art, p, Color32::from_white_alpha(22));
        }
        ui.horizontal(|ui| {
            widgets::tracked(ui, "FIRE EMBLEM", 8.5, p.accent_hi, 2.0);
            widgets::pill(ui, p, "NATIVE PC", Tone::Accent);
            if !version.is_empty() {
                widgets::pill(ui, p, &format!("v{version}"), Tone::Neutral);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let state = if crate::engine::ready() { "READY".to_string() } else { format!("SETUP {done}/{total}") };
                ui.label(RichText::new(state).font(theme::mono(10.0)).color(if crate::engine::ready() { p.ok } else { p.muted }));
            });
        });
        ui.add_space(6.0);
        ui.label(RichText::new(title).font(theme::display(25.0)).color(p.ink))
            .on_hover_text("Statically recompiled from your own copy of the game into native PC code. No emulator.");
        ui.add_space(5.0);
        ui.horizontal_wrapped(|ui| {
            widgets::status_dot(ui, p, data_tone);
            ui.label(RichText::new(data_text).size(11.5).color(data_tone.colour(p)));
            ui.label(RichText::new("·").size(11.0).color(p.muted));
            let (text, tone) = if building {
                ("Building the game from your dump".to_string(), Tone::Accent)
            } else if running {
                ("The game is running".to_string(), Tone::Ok)
            } else if ready {
                ("Ready to launch".to_string(), Tone::Ok)
            } else if !missing_tools.is_empty() && data_ready {
                (format!("{} build tool{} missing", missing_tools.len(), if missing_tools.len() == 1 { "" } else { "s" }), Tone::Warn)
            } else if data_ready {
                ("One build step left".to_string(), Tone::Warn)
            } else {
                ("Choose your game data to begin".to_string(), Tone::Neutral)
            };
            ui.label(RichText::new(text).size(11.5).color(tone.colour(p)));
        });

        if let Some((built, now)) = stale {
            let built = built.unwrap_or_else(|| "an older release".to_string());
            ui.add_space(7.0);
            egui::Frame::new()
                .fill(theme::alpha(p.warn, 14))
                .stroke(egui::Stroke::new(1.0_f32, theme::alpha(p.warn, 80)))
                .corner_radius(egui::CornerRadius::same(9))
                .inner_margin(egui::Margin::symmetric(11, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_top(|ui| {
                        widgets::status_dot(ui, p, Tone::Warn);
                        ui.vertical(|ui| {
                            ui.label(RichText::new("This game folder came from another release").font(theme::semibold(11.5)).color(p.ink));
                            ui.label(RichText::new(format!("Built by {built}; this launcher is {now}. Rebuilding updates it once; saves and mods are untouched.")).size(11.0).color(p.body));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::button(ui, p, "Repair now", data_ready && !building).clicked() {
                                repair_clicked = true;
                            }
                        });
                    });
                });
        }

        ui.add_space(9.0);
        let (label, sub, enabled, press) = if running {
            ("RUNNING", "THE GAME IS OPEN", false, None)
        } else if building {
            ("BUILDING", "PLEASE WAIT", false, None)
        } else if ready {
            ("PLAY", "CONTINUE YOUR ADVENTURE", true, Some(Action::Play))
        } else if !data_ready {
            ("SET UP", "CHOOSE YOUR GAME DATA", true, Some(Action::Open(Page::Setup)))
        } else if !missing_tools.is_empty() {
            ("INSTALL TOOLS", "ONE-TIME SETUP", true, Some(Action::InstallTools))
        } else {
            ("BUILD GAME", "ONE-TIME SETUP", true, Some(Action::Run(JobKind::Engine)))
        };
        ui.horizontal(|ui| {
            if widgets::large_action(ui, p, label, sub, enabled).clicked() && enabled {
                *action = press;
            }
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.add_space(2.0);
                if widgets::button(ui, p, if ready { "Rebuild" } else { "Build options" }, data_ready && !building)
                    .on_hover_text("Recompile from your dump. The first build takes longer; later ones only redo what changed.")
                    .clicked()
                {
                    *action = Some(if ready { Action::Run(JobKind::Engine) } else { Action::Open(Page::Recompile) });
                }
                ui.add_space(5.0);
                if widgets::button(ui, p, "Setup", true).clicked() {
                    *action = Some(Action::Open(Page::Setup));
                }
            });
        });
        if !ready && data_ready && !building {
            if let Some(note) = crate::sites::installed_note() {
                ui.add_space(6.0);
                ui.label(RichText::new(note).size(11.0).color(p.warn));
            }
        }

        // The one piece of save state that belongs on the landing page: which
        // playthrough the next Play will continue.
        let save_dir = crate::config::Config::save_dir();
        let profiles = save_dir.as_deref().map(crate::saves::profiles).unwrap_or_default();
        if !profiles.is_empty() && !running {
            ui.add_space(7.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Continue").size(10.5).color(p.muted));
                let mut load = None;
                egui::ComboBox::from_id_salt("overview-profile")
                    .selected_text("Choose a save profile…")
                    .width(168.0)
                    .show_ui(ui, |ui| {
                        for pr in &profiles {
                            if ui.add_enabled(!building, egui::Button::new(&pr.name)).clicked() {
                                load = Some(pr.clone());
                            }
                        }
                    });
                if let Some(pr) = load {
                    if let Some(dir) = save_dir {
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
                ui.label(RichText::new(format!("{} saved", profiles.len())).size(10.0).color(p.muted));
            });
        }
        ui.add_space(7.0);
        ui.horizontal(|ui| {
            if ui
                .link(RichText::new(format!("Setup guide · {done}/{total} ready")).size(10.5).color(p.accent_hi))
                .on_hover_text("Open the guided checklist without leaving Overview")
                .clicked()
            {
                app.overview_popup = Some(Popup::SetupGuide);
            }
            if !ready && !data_ready && !building {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("Nothing is downloaded or executed without asking.").size(9.5).color(p.muted));
                });
            }
        });
    });
    if repair_clicked {
        match crate::engine::clean_runtime_build() {
            Ok(()) => *action = Some(Action::Run(JobKind::Engine)),
            Err(e) => app.notify(format!("Could not start the repair: {e}"), Tone::Bad),
        }
    }
}

fn system_panel(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    let frame = widgets::card_frame(p, egui::Margin::symmetric(16, 12));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            widgets::tracked(ui, "SYSTEM CHECK", 8.5, p.accent_hi, 2.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .link(RichText::new("Full status").size(10.0).color(p.accent_hi))
                    .clicked()
                {
                    app.page = Page::Requirements;
                }
            });
        });
        ui.add_space(5.0);

        let (data_value, data_tone) = data_summary(app);
        if status_button(
            ui,
            p,
            "GAME",
            &data_value,
            data_tone,
            "Game data and the built title",
        ) {
            app.overview_popup = Some(Popup::GameStatus);
        }
        let built = crate::engine::ready();
        let building = app.running(JobKind::Engine).is_some();
        let (build_value, build_tone) = if building {
            ("Building…".to_string(), Tone::Accent)
        } else if built {
            ("Built and ready".to_string(), Tone::Ok)
        } else {
            ("Not built yet".to_string(), Tone::Neutral)
        };
        if status_button(
            ui,
            p,
            "BUILD",
            &build_value,
            build_tone,
            "Recompilation progress and outputs",
        ) {
            app.overview_popup = Some(Popup::BuildStatus);
        }
        let (tool_value, tool_tone) = match crate::deps::cached() {
            Some(rows) => {
                let required = rows.iter().filter(|r| r.required).count().max(1);
                let ok = rows.iter().filter(|r| r.required && r.status.ok()).count();
                (
                    format!("{ok}/{required} ready"),
                    if ok == required { Tone::Ok } else { Tone::Warn },
                )
            }
            None => ("Checking…".to_string(), Tone::Neutral),
        };
        let mut goto = None;
        if status_button(
            ui,
            p,
            "TOOLS",
            &tool_value,
            tool_tone,
            "Open the dependency checks",
        ) {
            goto = Some(Page::Requirements);
        }
        let (runtime_value, runtime_tone) = match &app.last_boot {
            Some((r, _)) => (format!("Boot test · {}", r.phase), Tone::Ok),
            None if app.game.bootable_inputs() => {
                ("Boot test available".to_string(), Tone::Neutral)
            }
            None => ("Waiting for setup".to_string(), Tone::Neutral),
        };
        if status_button(
            ui,
            p,
            "RUNTIME",
            &runtime_value,
            runtime_tone,
            "Open runtime diagnostics",
        ) {
            goto = Some(Page::Diagnostics);
        }
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            if widgets::button(ui, p, "Re-check", true)
                .on_hover_text("Checks the game folder and build tools again")
                .clicked()
            {
                app.start(JobKind::Detect);
                std::thread::spawn(|| crate::deps::warm());
                app.notify("Checking the game folder and build tools", Tone::Accent);
            }
            if let Some(page) = goto {
                app.page = page;
            }
        });
    });
}

/// A one-line status control that opens its detail panel. The old progress
/// bars made five rows taller than the hero they sat beside; the detail is now
/// one click away, where it has room to breathe.
fn status_button(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    label: &str,
    value: &str,
    tone: Tone,
    tip: &str,
) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 31.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui
            .ctx()
            .animate_bool(response.id.with("h"), response.hovered());
        let painter = ui.painter();
        if hover > 0.0 {
            painter.rect_filled(
                rect.shrink2(Vec2::new(2.0, 1.0)),
                egui::CornerRadius::same(7),
                theme::alpha(Color32::WHITE, (5.0 * hover) as u8),
            );
        }
        let dot = egui::pos2(rect.left() + 8.0, rect.center().y);
        painter.circle_filled(dot, 3.0, tone.colour(p));
        painter.circle_filled(dot, 5.2, theme::alpha(tone.colour(p), 24));
        painter.text(
            egui::pos2(rect.left() + 18.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            theme::semibold(8.0),
            p.muted,
        );
        painter.text(
            egui::pos2(rect.left() + 72.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            value,
            theme::semibold(10.0),
            p.ink,
        );
        let arrow = egui::pos2(rect.right() - 10.0, rect.center().y);
        let ink = lerp(p.muted, p.accent_hi, hover);
        painter.line_segment(
            [arrow + Vec2::new(-3.0, -3.0), arrow + Vec2::new(0.0, 0.0)],
            egui::Stroke::new(1.2_f32, ink),
        );
        painter.line_segment(
            [arrow + Vec2::new(0.0, 0.0), arrow + Vec2::new(-3.0, 3.0)],
            egui::Stroke::new(1.2_f32, ink),
        );
        if response.has_focus() {
            painter.rect_stroke(
                rect.shrink(2.0),
                egui::CornerRadius::same(7),
                egui::Stroke::new(1.0_f32, p.electric),
                egui::StrokeKind::Inside,
            );
        }
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip)
        .clicked()
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let [ar, ag, ab, aa] = a.to_array();
    let [br, bg, bb, _] = b.to_array();
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(m(ar, br), m(ag, bg), m(ab, bb), aa)
}

fn quick_access(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
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
    let (data_text, data_tone) = data_summary(app);
    let pipeline = match &app.last_pipeline {
        Some(s) if s.verified.is_some_and(|(ok, n)| ok == n) => ("Verified".to_string(), Tone::Ok),
        Some(_) => ("Lifted".to_string(), Tone::Accent),
        None => ("Open pipeline".to_string(), Tone::Neutral),
    };
    let rows = crate::deps::cached();
    let missing = rows
        .as_deref()
        .map(crate::deps::blocking)
        .map(|r| r.len())
        .unwrap_or(0);
    let tools = if rows.is_none() {
        ("Checking…".to_string(), Tone::Neutral)
    } else if missing == 0 {
        ("All ready".to_string(), Tone::Ok)
    } else {
        (format!("{missing} missing"), Tone::Warn)
    };
    let tiles: [(Page, String, Tone); 8] = [
        (Page::Setup, data_text, data_tone),
        (
            Page::Saves,
            if profiles.is_empty() {
                "No profiles yet".to_string()
            } else {
                format!(
                    "{} profile{}",
                    profiles.len(),
                    if profiles.len() == 1 { "" } else { "s" }
                )
            },
            Tone::Accent,
        ),
        (
            Page::Mods,
            if mods_on == 0 {
                "No mods enabled".to_string()
            } else {
                format!("{mods_on} enabled")
            },
            if mods_on > 0 { Tone::Ok } else { Tone::Neutral },
        ),
        (
            Page::Cheats,
            if cheats_on == 0 {
                "No cheats active".to_string()
            } else {
                format!("{cheats_on} active")
            },
            if cheats_on > 0 {
                Tone::Accent
            } else {
                Tone::Neutral
            },
        ),
        (
            Page::Graphics,
            app.config.graphics.resolution.clone(),
            Tone::Neutral,
        ),
        (
            Page::Controls,
            format!("{} bindings", app.config.input.bindings.len()),
            Tone::Neutral,
        ),
        (Page::Recompile, pipeline.0, pipeline.1),
        (Page::Requirements, tools.0, tools.1),
    ];
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        widgets::tracked(ui, "QUICK ACCESS", 8.5, p.muted, 2.0);
        ui.add_space(4.0);
        for (page, detail, tone) in tiles {
            if widgets::shortcut_pill(ui, p, page, &detail, tone).clicked() {
                app.page = page;
            }
        }
    });
}

fn setup_guide_body(
    app: &mut App,
    ui: &mut egui::Ui,
    p: &theme::Palette,
    stages: &[SetupStage],
    done: usize,
    total: usize,
) {
    ui.label(RichText::new("EmblemWeave reads your own game data once, then builds a native copy for this PC. You can leave this page while it works.").size(11.5).color(p.body));
    ui.add_space(7.0);
    widgets::progress_bar(ui, p, Some(done as f32 / total.max(1) as f32));
    ui.add_space(8.0);
    let mut action = None;
    for (i, stage) in stages.iter().enumerate() {
        if setup_row(ui, p, i + 1, stage) {
            action = stage.action;
        }
    }
    if let Some(action) = action {
        run_action(app, action);
    }
    if !app.config.onboarding.completed {
        ui.add_space(6.0);
        let label = if app.config.onboarding.dismissed {
            "Resume guided setup"
        } else {
            "Open guided setup"
        };
        if widgets::button(ui, p, label, true)
            .on_hover_text(
                "Continue from the step you stopped at: game files, build tools, build, readiness",
            )
            .clicked()
        {
            app.config.onboarding.dismissed = false;
        }
    }
    if crate::engine::missing_tools().len() > 0 && crate::engine::have_winget() {
        ui.add_space(5.0);
        if widgets::button(ui, p, "Install missing build tools", true).clicked() {
            match crate::engine::start_install() {
                    Ok(_) => app.notify("Installing build tools in a new window. Accept the Windows prompt, then re-check Requirements.", Tone::Ok),
                    Err(e) => app.notify(e, Tone::Bad),
                }
        }
    }
}

fn setup_row(ui: &mut egui::Ui, p: &theme::Palette, index: usize, stage: &SetupStage) -> bool {
    let clickable = stage.action.is_some();
    let sense = if clickable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), sense);
    if ui.is_rect_visible(rect) {
        let hover = ui
            .ctx()
            .animate_bool(response.id.with("h"), clickable && response.hovered());
        let painter = ui.painter();
        if hover > 0.0 {
            painter.rect_filled(
                rect.shrink2(Vec2::new(2.0, 1.0)),
                egui::CornerRadius::same(7),
                theme::alpha(Color32::WHITE, (4.0 * hover) as u8),
            );
        }
        let dot = egui::pos2(rect.left() + 10.0, rect.center().y);
        let c = stage.tone.colour(p);
        if stage.tone == Tone::Ok {
            painter.circle_filled(dot, 6.0, theme::alpha(c, 32));
            painter.circle_filled(dot, 3.2, c);
        } else {
            painter.circle_stroke(dot, 5.0, egui::Stroke::new(1.2_f32, c));
            if stage.tone == Tone::Accent {
                painter.circle_filled(dot, 2.0, c);
            }
        }
        painter.text(
            egui::pos2(rect.left() + 24.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{index:02}"),
            theme::mono(9.5),
            p.muted,
        );
        painter.text(
            egui::pos2(rect.left() + 48.0, rect.top() + 14.0),
            egui::Align2::LEFT_CENTER,
            stage.name,
            theme::semibold(11.5),
            p.ink,
        );
        let detail = if stage.detail.len() > 78 {
            format!("{}…", stage.detail.chars().take(78).collect::<String>())
        } else {
            stage.detail.clone()
        };
        painter.text(
            egui::pos2(rect.left() + 48.0, rect.top() + 29.0),
            egui::Align2::LEFT_CENTER,
            detail,
            egui::FontId::proportional(10.5),
            p.muted,
        );
        if clickable {
            painter.text(
                egui::pos2(rect.right() - 8.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                stage.action_label,
                theme::semibold(8.5),
                p.accent_hi,
            );
        }
        if response.has_focus() {
            painter.rect_stroke(
                rect.shrink(2.0),
                egui::CornerRadius::same(7),
                egui::Stroke::new(1.0_f32, p.electric),
                egui::StrokeKind::Inside,
            );
        }
    }
    let response = if clickable {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    response.clicked()
}

fn game_status_body(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    let (title, version) = app
        .game
        .title
        .clone()
        .unwrap_or_else(|| (TITLE_NAME.to_string(), String::new()));
    widgets::kv(ui, p, "Title", &title);
    if !version.is_empty() {
        widgets::kv(ui, p, "Version", &version);
    }
    widgets::kv(
        ui,
        p,
        "RomFS entries",
        &crate::fmt_count(app.game.romfs_entries.max(0) as u64),
    );
    let (text, tone) = data_summary(app);
    widgets::status_line(ui, p, tone, "Game data", &text);
    if let Some(attention) = app.game.attention_line() {
        ui.label(RichText::new(attention).size(11.5).color(p.warn));
    }
    ui.add_space(5.0);
    if widgets::button(ui, p, "Open Game Setup", true).clicked() {
        app.page = Page::Setup;
        app.overview_popup = None;
    }
    ui.add_space(4.0);
    ui.collapsing(
        RichText::new("Technical details").size(11.0).color(p.muted),
        |ui| {
            if let Some(root) = app.game.exefs.as_ref() {
                widgets::kv(ui, p, "ExeFS", &root.display().to_string());
            }
            if let Some(romfs) = app.game.romfs.as_ref() {
                widgets::kv(ui, p, "RomFS", &romfs.display().to_string());
            }
            if let Some(dlc) = app.game.dlc.as_ref() {
                widgets::kv(
                    ui,
                    p,
                    "Add-on content",
                    &format!("{} pack(s) — {}", app.game.dlc_ids.len(), dlc.display()),
                );
            }
            if let Some(nup) = app.game.nup.as_ref() {
                widgets::kv(ui, p, "Package", &nup.display().to_string());
            }
            for c in &app.game.checks {
                let tone = match c.severity {
                    Severity::Ok => Tone::Ok,
                    Severity::Info => Tone::Accent,
                    Severity::Warn => Tone::Warn,
                    Severity::Error => Tone::Bad,
                };
                widgets::status_line(ui, p, tone, &c.label, &c.detail);
            }
        },
    );
}

fn build_status_body(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    let ready = crate::engine::ready();
    let building = app.running(JobKind::Engine).is_some();
    let missing = crate::engine::missing_tools();
    if building {
        if let Some(job) = app.running(JobKind::Engine) {
            let stages = job.snapshot(|pr| pr.stages.clone());
            let done = stages
                .iter()
                .filter(|s| {
                    matches!(
                        s.status,
                        crate::jobs::StageStatus::Done
                            | crate::jobs::StageStatus::Skipped
                            | crate::jobs::StageStatus::Failed
                    )
                })
                .count();
            widgets::progress_bar(
                ui,
                p,
                if stages.is_empty() {
                    None
                } else {
                    Some(done as f32 / stages.len() as f32)
                },
            );
            if let Some(active) = stages
                .iter()
                .find(|s| matches!(s.status, crate::jobs::StageStatus::Active))
            {
                ui.label(
                    RichText::new(format!("{} · {}", active.name, active.detail))
                        .size(11.5)
                        .color(p.body),
                );
            }
        }
        ui.add_space(5.0);
        if widgets::button(ui, p, "Open build details", true).clicked() {
            app.page = Page::Recompile;
            app.overview_popup = None;
        }
    } else if ready {
        widgets::status_line(
            ui,
            p,
            Tone::Ok,
            "Native build",
            "The game is compiled and ready to start on this PC.",
        );
        ui.add_space(4.0);
        if widgets::button(ui, p, "Rebuild the game", true)
            .on_hover_text(
                "Recompiles from your dump; your saves, mods and settings are untouched.",
            )
            .clicked()
        {
            app.start(JobKind::Engine);
        }
    } else {
        widgets::status_line(
            ui,
            p,
            if missing.is_empty() {
                Tone::Warn
            } else {
                Tone::Bad
            },
            "Native build",
            if missing.is_empty() {
                "Your game data can be built now."
            } else {
                "Some build tools are missing."
            },
        );
        if !missing.is_empty() {
            ui.label(
                RichText::new(format!("Missing: {}", missing.join(", ")))
                    .size(11.0)
                    .color(p.body),
            );
            if widgets::primary_button(ui, p, "Review requirements", true).clicked() {
                app.page = Page::Requirements;
                app.overview_popup = None;
            }
        } else if widgets::primary_button(
            ui,
            p,
            "Build the game",
            app.game.exefs.is_some() && app.game.romfs.is_some(),
        )
        .clicked()
        {
            app.start(JobKind::Engine);
        }
    }
    ui.add_space(5.0);
    ui.collapsing(
        RichText::new("Technical details").size(11.0).color(p.muted),
        |ui| {
            widgets::kv(
                ui,
                p,
                "Engine build",
                if ready { "found" } else { "not found" },
            );
            if let Some(pipeline) = &app.last_pipeline {
                if let Some(report) = &pipeline.report {
                    widgets::kv(
                        ui,
                        p,
                        "Functions lifted",
                        &crate::fmt_count(report.lifted_functions as u64),
                    );
                    widgets::kv(
                        ui,
                        p,
                        "Decode coverage",
                        &format!("{:.2}%", report.decode_coverage * 100.0),
                    );
                }
                if let Some((ok, n)) = pipeline.verified {
                    widgets::kv(ui, p, "Verification", &format!("{ok}/{n} identical"));
                }
                if let Some(out) = &pipeline.output {
                    widgets::kv(ui, p, "Output", &out.display().to_string());
                }
            }
            if let Some(path) = &app.last_job_log_path {
                widgets::kv(ui, p, "Last log", &path.display().to_string());
            }
        },
    );
}

fn quick_help_body(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    ui.label(
        RichText::new("If a build or launch does not work, start with the check that matches it.")
            .size(11.5)
            .color(p.body),
    );
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if widgets::button(ui, p, "Requirements", true).clicked() {
            app.page = Page::Requirements;
            app.overview_popup = None;
        }
        if widgets::button(ui, p, "Troubleshooter", true).clicked() {
            app.page = Page::Troubleshoot;
            app.overview_popup = None;
        }
        if widgets::button(ui, p, "Game Log", true).clicked() {
            app.page = Page::Log;
            app.overview_popup = None;
        }
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.hyperlink_to(
            "Documentation",
            "https://github.com/XiONDiGiTaL/EmblemWeave#readme",
        );
        ui.hyperlink_to(
            "Report an issue",
            "https://github.com/XiONDiGiTaL/EmblemWeave/issues",
        );
    });
}

fn whats_new_body(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    ui.label(
        RichText::new("This build")
            .font(theme::semibold(11.5))
            .color(p.ink),
    );
    for line in [
        "A compact launcher header and grouped, collapsible navigation.",
        "A guided setup checklist that uses the real dependency and build checks.",
        "Plain-language status first, with technical details folded away.",
        "One prominent Play, Set Up or Build action on the landing page.",
    ] {
        ui.horizontal_top(|ui| {
            widgets::status_dot(ui, p, Tone::Accent);
            ui.label(RichText::new(line).size(11.0).color(p.body));
        });
    }
    if matches!(app.update.state, crate::UpdateState::Ready(_)) {
        ui.add_space(4.0);
        ui.label(
            RichText::new("The Update available card above installs it in place.")
                .size(11.5)
                .color(p.muted),
        );
    }
}

/// The shared floating panel for What's New, Help, the two status buttons and
/// the setup guide. It deliberately has no dimming veil: the page stays usable
/// and one click on another header control swaps the panel instead of stacking
/// another card on the screen.
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
                    .filter(|s| s.counts && s.tone == Tone::Ok)
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
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_width(392.0);
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            widgets::tracked(ui, "OVERVIEW", 8.5, p.accent_hi, 2.0);
                            ui.label(RichText::new(title).font(theme::display(18.0)).color(p.ink));
                            ui.label(RichText::new(subtitle).size(10.5).color(p.muted));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            if widgets::button(ui, &p, "Close", true).clicked() {
                                close = true;
                            }
                        });
                    });
                    ui.add_space(8.0);
                    let line = ui.available_rect_before_wrap();
                    ui.painter().hline(
                        line.x_range(),
                        line.top(),
                        egui::Stroke::new(1.0_f32, p.line),
                    );
                    ui.add_space(8.0);
                    let max_height = (ctx.screen_rect().height() - 180.0).max(160.0);
                    egui::ScrollArea::vertical()
                        .id_salt(("overview-popup-body", kind as u8))
                        .max_height(max_height)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            match kind {
                                Popup::WhatsNew => whats_new_body(app, ui, &p),
                                Popup::QuickHelp => quick_help_body(app, ui, &p),
                                Popup::GameStatus => game_status_body(app, ui, &p),
                                Popup::BuildStatus => build_status_body(app, ui, &p),
                                Popup::SetupGuide => {
                                    let stages = setup_stages(app);
                                    let total = stages.iter().filter(|s| s.counts).count();
                                    let done = if crate::engine::ready() {
                                        total
                                    } else {
                                        stages
                                            .iter()
                                            .filter(|s| s.counts && s.tone == Tone::Ok)
                                            .count()
                                    };
                                    setup_guide_body(app, ui, &p, &stages, done, total);
                                }
                            }
                        });
                });
        });
    if close {
        app.overview_popup = None;
    }
}

fn data_summary(app: &App) -> (String, Tone) {
    if app.running(JobKind::Detect).is_some() {
        return ("Checking game data…".into(), Tone::Accent);
    }
    let g = &app.game;
    match (&g.exefs, &g.title) {
        (None, _) => ("Not selected".into(), Tone::Warn),
        (Some(_), Some((_, v))) => (
            format!(
                "v{v} · {} entries",
                crate::fmt_count(g.romfs_entries.max(0) as u64)
            ),
            Tone::Ok,
        ),
        (Some(_), None) => (
            format!(
                "{} entries",
                crate::fmt_count(g.romfs_entries.max(0) as u64)
            ),
            Tone::Warn,
        ),
    }
}

fn setup_stages(app: &App) -> Vec<SetupStage> {
    let data_ready =
        app.game.exefs.is_some() && (app.game.romfs.is_some() || app.game.nup.is_some());
    let detecting = app.running(JobKind::Detect).is_some();
    let rows = crate::deps::cached();
    let blocked = rows.as_deref().map(|r| crate::deps::blocking(r).len());
    let python_ready = rows.as_deref().map(|r| {
        let py = crate::deps::row(r, "python").is_some_and(|x| x.status.ok());
        let zbic = crate::deps::row(r, "zbic").is_some_and(|x| x.status.ok());
        py && zbic
    });
    let building = app.running(JobKind::Engine).is_some();
    let ready = crate::engine::ready();
    let mut out = Vec::new();

    out.push(SetupStage {
        name: "Game data",
        detail: if data_ready {
            "ExeFS and RomFS were found and selected.".into()
        } else if detecting {
            "Looking for your game files…".into()
        } else {
            "Choose your own dump or extracted game folder.".into()
        },
        tone: if data_ready {
            Tone::Ok
        } else if detecting {
            Tone::Accent
        } else {
            Tone::Warn
        },
        action: Some(Action::Open(Page::Setup)),
        action_label: if data_ready { "CHANGE" } else { "CHOOSE" },
        counts: true,
    });
    out.push(SetupStage {
        name: "Version check",
        detail: match &app.game.title {
            Some((_, v)) => format!("Detected version {v}."),
            None if app.game.exefs.is_some() => "The title metadata has not been read yet.".into(),
            None => "Waiting for game data.".into(),
        },
        tone: if app.game.title.is_some() {
            Tone::Ok
        } else if data_ready {
            Tone::Warn
        } else {
            Tone::Neutral
        },
        action: Some(Action::Open(Page::Setup)),
        action_label: "VIEW",
        counts: true,
    });
    out.push(SetupStage {
        name: "Build tools",
        detail: match (rows.is_some(), blocked) {
            (false, _) => "Checking Visual Studio, LLVM, CMake and Ninja…".into(),
            (true, Some(0)) => "Every required build tool was found.".into(),
            (true, Some(n)) => format!(
                "{n} required tool{} need attention.",
                if n == 1 { "" } else { "s" }
            ),
            _ => "Waiting for the dependency check.".into(),
        },
        tone: match blocked {
            Some(0) => Tone::Ok,
            Some(_) => Tone::Warn,
            None => Tone::Neutral,
        },
        action: Some(Action::Open(Page::Requirements)),
        action_label: "CHECK",
        counts: true,
    });
    out.push(SetupStage {
        name: "Python environment",
        detail: match python_ready {
            Some(true) => "Python and the ZBIC package are ready.".into(),
            Some(false) => "Python or ZBIC is missing; needed to decode the main module.".into(),
            None => "Checking Python and the ZBIC package…".into(),
        },
        tone: match python_ready {
            Some(true) => Tone::Ok,
            Some(false) => Tone::Warn,
            None => Tone::Neutral,
        },
        action: Some(Action::Open(Page::Requirements)),
        action_label: "CHECK",
        counts: true,
    });
    out.push(SetupStage {
        name: "Native build",
        detail: if building {
            "Compiling the game from your dump…".into()
        } else if ready {
            "The game is compiled and ready for this PC.".into()
        } else if data_ready {
            "Build the game once from your own dump.".into()
        } else {
            "Available after game data is chosen.".into()
        },
        tone: if ready {
            Tone::Ok
        } else if building {
            Tone::Accent
        } else if data_ready {
            Tone::Warn
        } else {
            Tone::Neutral
        },
        action: if ready {
            Some(Action::Open(Page::Recompile))
        } else if data_ready && !building {
            Some(Action::Run(JobKind::Engine))
        } else {
            None
        },
        action_label: if ready { "OPEN" } else { "BUILD" },
        counts: true,
    });
    out.push(SetupStage {
        name: "Runtime check",
        detail: match &app.last_boot {
            Some((r, _)) => format!("Last boot test reached {}.", r.phase),
            None if app.game.bootable_inputs() => {
                "A boot test can run before the first launch.".into()
            }
            None => "Available after the build.".into(),
        },
        tone: if app.last_boot.is_some() {
            Tone::Ok
        } else {
            Tone::Neutral
        },
        action: Some(Action::Open(Page::Diagnostics)),
        action_label: "OPEN",
        counts: false,
    });
    out.push(SetupStage {
        name: "Ready to play",
        detail: if ready {
            "Press Play on the Overview to start.".to_string()
        } else {
            "Finish the steps above and Play will unlock.".to_string()
        },
        tone: if ready { Tone::Ok } else { Tone::Neutral },
        action: if ready {
            Some(Action::Play)
        } else {
            Some(Action::Open(Page::Requirements))
        },
        action_label: if ready { "PLAY" } else { "WHY?" },
        counts: true,
    });
    out
}

fn run_action(app: &mut App, action: Action) {
    match action {
        Action::Open(page) => app.page = page,
        Action::Run(kind) => app.start(kind),
        Action::Play => app.play(),
        Action::InstallTools => match crate::engine::start_install() {
            Ok(_) => app.notify("Installing the missing build tools in a new window. Accept the Windows prompt, then press Re-check.", Tone::Ok),
            Err(e) => app.notify(e, Tone::Bad),
        },
    }
}
