//! The first-run guide. Every screen reads real detection, dependency and job
//! state; its step and dismissal are kept in the config so closing the launcher
//! halfway through resumes in the same place. The guide never claims a step is
//! done because the player pressed Continue.

use crate::jobs::JobKind;
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self};

const STEP_BUILD: u8 = 3;
const STEP_READY: u8 = 4;

/// Whether the guide belongs on screen. A built game is already past it, and a
/// screenshot/tour run must not have a dialog in the way.
pub fn wanted(app: &App) -> bool {
    app.config.onboarding.introduced
        && !app.config.onboarding.completed
        && !app.config.onboarding.dismissed
        && app.screenshot.is_none()
        && app.tour.is_none()
        && app.confirm.is_none()
        && !crate::engine::ready()
}

/// Show the guide if it is wanted. Called after the page and floating panels so
/// its veil is the topmost thing on screen.
pub fn show(app: &mut App, ctx: &egui::Context) {
    if !wanted(app) {
        return;
    }
    let p = app.config.theme.palette();
    let data_ready = app.game.exefs.is_some() && (app.game.romfs.is_some() || app.game.nup.is_some());
    let rows = crate::deps::cached();
    let blocking = rows.as_deref().map(crate::deps::blocking).map(|v| v.len()).unwrap_or(0);
    let ready = crate::engine::ready();
    let detecting = app.running(JobKind::Detect).is_some();
    let building = app.running(JobKind::Engine).is_some();

    let mut step = app.config.onboarding.step.min(STEP_READY);
    if ready {
        step = STEP_READY;
    } else if step >= STEP_READY {
        step = STEP_BUILD;
    }
    if step >= 2 && !data_ready {
        step = 1;
    }
    if step >= STEP_BUILD && rows.is_some() && blocking > 0 {
        step = 2;
    }

    let titles = ["Welcome to EmblemWeave", "Choose your game files", "Install the build tools", "Build the native game", "You are ready to play"];
    let title = titles[step as usize];
    let subtitle = if step == 0 { "A one-time setup, with the real state of your PC shown at every step.".to_string() } else { format!("Step {step} of 4  ·  saved automatically") };

    #[derive(Clone, Copy)]
    enum Action {
        Step(u8),
        ChooseFolder,
        SetupPage,
        Install,
        Recheck,
        Build,
        Play,
        Finish,
        Dismiss,
    }
    let action: std::cell::Cell<Option<Action>> = std::cell::Cell::new(None);

    let shown = widgets::modal(ctx, &p, "first-run-guide", title, &subtitle, 700.0, |ui| {
        ui.set_min_height(300.0);
        match step {
            0 => {
                ui.label(crate::i18n::text("A native PC copy, built from your own copy of the game.").font(theme::display(18.0)).color(p.ink));
                ui.add_space(5.0);
                ui.label(crate::i18n::text("Nothing is downloaded or run without asking. Your game dump is read where it is; saves, mods and settings stay in the launcher's own folder.").size(12.0).color(p.body));
                ui.add_space(14.0);
                intro_row(ui, &p, 1, "Point at your game files", "Choose the folder that holds exefs and romfs. The guide checks what it can read before it lets you continue.");
                intro_row(ui, &p, 2, "Install the tools once", "The guide asks Windows to install only what this PC is missing. A release with the bundled shader translator needs no .NET SDK.");
                intro_row(ui, &p, 3, "Build once, then play", "The first build is the slow one. Later builds only redo what changed, and this guide remembers where you stopped.");
            }
            1 => {
                ui.label(crate::i18n::text("EmblemWeave reads these files once; it never changes or moves them.").size(12.0).color(p.body));
                ui.add_space(9.0);
                file_row(ui, &p, "Game code (exefs/main)", app.game.exefs.is_some(), app.game.exefs.as_ref());
                file_row(ui, &p, "Game data (romfs or NSP)", app.game.romfs.is_some() || app.game.nup.is_some(), app.game.romfs.as_ref().or(app.game.nup.as_ref()));
                file_row(ui, &p, "Title metadata (optional)", app.game.nacp.is_some(), app.game.nacp.as_ref());
                ui.add_space(9.0);
                if detecting {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0).color(p.accent));
                        ui.label(crate::i18n::text("Checking the folder…").size(12.0).color(p.body));
                    });
                } else if data_ready {
                    if let Some((name, version)) = &app.game.title {
                        widgets::status_line(ui, &p, Tone::Ok, "Game found", &format!("{name} · v{version}"));
                    } else {
                        widgets::status_line(ui, &p, Tone::Ok, "Game files found", "The title name was not in the metadata, but the executable and data are readable.");
                    }
                } else if !app.config.paths.game_root.trim().is_empty() {
                    widgets::error_note(ui, &p, "That folder does not contain a readable exefs/main and romfs (or NSP). Choose the folder that holds both, or open Game Setup to pick each file.");
                } else {
                    widgets::empty_note(ui, &p, "No game folder chosen yet", "An extracted game folder usually has an exefs folder and a romfs folder side by side. A folder holding an NSP works too.");
                }
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    if widgets::primary_button(ui, &p, "Choose game folder…", !detecting).clicked() {
                        action.set(Some(Action::ChooseFolder));
                    }
                    if widgets::button(ui, &p, "Advanced setup", true).clicked() {
                        action.set(Some(Action::SetupPage));
                    }
                });
                ui.add_space(3.0);
                ui.label(crate::i18n::text("Advanced setup can point at each file separately and decode a packed package.").size(10.5).color(p.muted));
            }
            2 => {
                ui.label(crate::i18n::text("Only the tools that block this build are shown. The release's bundled translator removes the .NET SDK requirement on a normal install.").size(12.0).color(p.body));
                ui.add_space(9.0);
                match rows.as_deref() {
                    None => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(14.0).color(p.accent));
                            ui.label(crate::i18n::text("Checking Visual Studio, LLVM, CMake and the other build tools…").size(12.0).color(p.body));
                        });
                    }
                    Some(rows) => {
                        let blocking_rows = crate::deps::blocking(rows);
                        let install_lines = crate::deps::install_lines(rows);
                        let manual = crate::deps::manual_fixes(rows);
                        if blocking_rows.is_empty() {
                            widgets::status_line(ui, &p, Tone::Ok, "Build tools ready", "Every dependency this configuration needs is installed.");
                        } else {
                            widgets::error_note(ui, &p, &format!("{} build tool{} still need{} attention. Continue is available once they are installed and re-checked.", blocking_rows.len(), if blocking_rows.len() == 1 { "" } else { "s" }, if blocking_rows.len() == 1 { "s" } else { "" }));
                            ui.add_space(6.0);
                            for row in blocking_rows.iter().take(8) {
                                ui.horizontal_top(|ui| {
                                    widgets::status_dot(ui, &p, Tone::Warn);
                                    ui.vertical(|ui| {
                                        ui.label(crate::i18n::text(row.dep.name).font(theme::semibold(12.5)).color(p.ink));
                                        let detail = row.status.detail();
                                        ui.add(egui::Label::new(crate::i18n::text(compact_path(&detail)).size(11.0).color(p.muted)).wrap());
                                    });
                                });
                            }
                        }
                        if !manual.is_empty() {
                            ui.add_space(7.0);
                            ui.label(crate::i18n::text("Manual step").font(theme::semibold(11.5)).color(p.warn));
                            for line in &manual {
                                ui.add(egui::Label::new(crate::i18n::text(format!("• {line}")).size(11.0).color(p.body)).wrap());
                            }
                            if crate::deps::row(rows, "sdk").is_some_and(|r| r.blocking()) {
                                ui.hyperlink_to(crate::i18n::text("Open the Windows SDK download page"), crate::deps::WINDOWS_SDK_URL);
                            }
                        }
                        ui.add_space(9.0);
                        let can_install = !install_lines.is_empty() && crate::engine::have_winget();
                        ui.horizontal_wrapped(|ui| {
                            if widgets::primary_button(ui, &p, "Install missing tools", can_install)
                                .on_hover_text(if install_lines.is_empty() {
                                    "The remaining tool needs the manual step above.".to_string()
                                } else {
                                    format!("Runs these in a new window, one after another:\n{}", install_lines.join("\n"))
                                })
                                .clicked()
                            {
                                action.set(Some(Action::Install));
                            }
                            if widgets::button(ui, &p, "Check again", true).clicked() {
                                action.set(Some(Action::Recheck));
                            }
                            if !blocking_rows.is_empty() && !crate::engine::have_winget() {
                                ui.label(crate::i18n::text("winget is not available; Requirements lists the manual links.").size(10.5).color(p.warn));
                            }
                        });
                        if !blocking_rows.is_empty() {
                            ui.add_space(3.0);
                            ui.label(crate::i18n::text("A tool that was installed while the game is open may only appear after reopening the launcher. The check itself is live.").size(10.5).color(p.muted));
                        }
                    }
                }
            }
            STEP_BUILD => {
                if ready {
                    widgets::status_line(ui, &p, Tone::Ok, "Build complete", "The native executable is ready.");
                } else if let Some(job) = app.running(JobKind::Engine) {
                    let (stages, stage_fraction) = job.snapshot(|pr| (pr.stages.clone(), pr.fraction));
                    let done = stages.iter().filter(|s| matches!(s.status, crate::jobs::StageStatus::Done | crate::jobs::StageStatus::Skipped | crate::jobs::StageStatus::Failed)).count();
                    widgets::progress_bar(ui, &p, if stages.is_empty() { None } else { Some(((done as f32 + stage_fraction) / stages.len() as f32).min(1.0)) });
                    ui.add_space(5.0);
                    if let Some(active) = stages.iter().find(|s| matches!(s.status, crate::jobs::StageStatus::Active)) {
                        ui.label(crate::i18n::text(active.name).font(theme::semibold(13.0)).color(p.ink));
                        ui.label(crate::i18n::text(&active.detail).size(11.5).color(p.body));
                    } else {
                        ui.label(crate::i18n::text("Preparing the build…").size(12.0).color(p.body));
                    }
                    ui.add_space(8.0);
                    ui.label(crate::i18n::text("You can close this guide; the build continues in the activity hub at the bottom of the window. Reopening Setup guide from Overview returns here.").size(10.8).color(p.muted));
                } else {
                    if rows.as_deref().is_some_and(|r| !crate::deps::blocking(r).is_empty()) {
                        widgets::error_note(ui, &p, "The build is blocked by missing tools. Go back one step to install them.");
                    } else if app.game.exefs.is_none() {
                        widgets::error_note(ui, &p, "The game files are not chosen yet. Go back one step to point at them.");
                    } else {
                        ui.label(crate::i18n::text("This compiles a native executable from your dump. It is the one long step; later builds are incremental.").size(12.0).color(p.body));
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            widgets::pill(ui, &p, "one-time", Tone::Accent);
                            widgets::pill(ui, &p, "saves are untouched", Tone::Neutral);
                            widgets::pill(ui, &p, "can run in background", Tone::Neutral);
                        });
                    }
                    if let Some(failed) = last_build_failure(app) {
                        ui.add_space(9.0);
                        widgets::error_note(ui, &p, &failed);
                    }
                    ui.add_space(10.0);
                    let can_build = app.game.exefs.is_some()
                        && rows.as_deref().is_some_and(|r| crate::deps::blocking(r).is_empty());
                    if widgets::primary_button(ui, &p, "Build the game", can_build).clicked() {
                        action.set(Some(Action::Build));
                    }
                }
            }
            STEP_READY => {
                ui.label(crate::i18n::text("The setup path is complete. The launcher will open here next time, with Play ready.").font(theme::display(18.0)).color(p.ink));
                ui.add_space(12.0);
                ready_row(ui, &p, data_ready, "Game data", if data_ready { "Executable and data are readable" } else { "Needs game files" });
                ready_row(ui, &p, rows.as_deref().is_some_and(|r| crate::deps::blocking(r).is_empty()), "Build tools", summary_tools(rows.as_deref()));
                ready_row(ui, &p, ready, "Native build", if ready { "Built and ready to launch" } else { "Not built yet" });
                ui.add_space(12.0);
                widgets::status_line(ui, &p, if ready { Tone::Ok } else { Tone::Warn }, "Readiness", if ready { "Everything required is present." } else { "One of the checks above still needs work." });
                ui.add_space(7.0);
                ui.label(crate::i18n::text("Saves, mods, controls and graphics can be changed later from the sidebar. They are not part of this setup path.").size(11.0).color(p.muted));
            }
            _ => {}
        }

    }, |ui| {
        ui.horizontal(|ui| {
            if step > 0 && !(step == STEP_BUILD && building) {
                if widgets::button(ui, &p, "Back", true).clicked() {
                    action.set(Some(Action::Step(step - 1)));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match step {
                    0 => {
                        if widgets::button(ui, &p, "Skip for now", true).clicked() {
                            action.set(Some(Action::Dismiss));
                        }
                        if widgets::primary_button(ui, &p, "Get started", true).clicked() {
                            action.set(Some(Action::Step(1)));
                        }
                    }
                    1 => {
                        let can_continue = data_ready;
                        if widgets::primary_button(ui, &p, if detecting { "Checking…" } else { "Continue" }, can_continue).clicked() {
                            action.set(Some(Action::Step(2)));
                        }
                        if !data_ready {
                            ui.label(crate::i18n::text("Choose a readable game folder to continue.").size(10.5).color(p.muted));
                        }
                    }
                    2 => {
                        let can_continue = rows.as_deref().is_some_and(|r| crate::deps::blocking(r).is_empty());
                        if widgets::primary_button(ui, &p, "Continue", can_continue).clicked() {
                            action.set(Some(Action::Step(3)));
                        }
                    }
                    STEP_BUILD => {
                        if ready {
                            if widgets::primary_button(ui, &p, "Continue", true).clicked() {
                                action.set(Some(Action::Step(STEP_READY)));
                            }
                        } else {
                            ui.label(crate::i18n::text("The build continues if this guide is closed.").size(10.5).color(p.muted));
                        }
                    }
                    STEP_READY => {
                        if widgets::primary_button(ui, &p, "Finish setup", ready).clicked() {
                            action.set(Some(Action::Finish));
                        }
                        if ready && widgets::button(ui, &p, "Play now", true).clicked() {
                            action.set(Some(Action::Play));
                        }
                    }
                    _ => {}
                }
            });
        });
    });

    if shown.close {
        app.config.onboarding.dismissed = true;
        app.config.onboarding.step = step;
        ctx.request_repaint();
        return;
    }
    let Some(action) = action.get() else {
        // Reaching a screen is useful progress on its own, even if the player
        // closes the window before pressing Continue.
        if app.config.onboarding.step != step {
            app.config.onboarding.step = step;
        }
        return;
    };
    match action {
        Action::Step(next) => {
            app.config.onboarding.step = next;
            app.config.onboarding.dismissed = false;
        }
        Action::Dismiss => {
            app.config.onboarding.dismissed = true;
            app.config.onboarding.step = step;
            app.notify("Guided setup is ready to resume from Overview", Tone::Neutral);
        }
        Action::SetupPage => {
            app.config.onboarding.dismissed = true;
            app.config.onboarding.step = 1;
            app.open(crate::pages::Page::Setup);
        }
        Action::ChooseFolder => {
            if let Some(folder) = rfd::FileDialog::new().set_title("Choose the folder holding exefs and romfs").pick_folder() {
                app.config.paths = crate::config::Paths { game_root: folder.display().to_string(), ..Default::default() };
                app.config.onboarding.step = 1;
                app.start(JobKind::Detect);
                app.notify("Checking the chosen game folder", Tone::Accent);
            }
        }
        Action::Install => match crate::engine::start_install() {
            Ok(_) => app.notify("Installing build tools in a new window. Accept the Windows prompt, then press Check again.", Tone::Ok),
            Err(e) => app.notify(e, Tone::Bad),
        },
        Action::Recheck => {
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                crate::deps::warm();
                ctx.request_repaint();
            });
            app.notify("Checking the build tools again", Tone::Neutral);
        }
        Action::Build => app.start(JobKind::Engine),
        Action::Finish => {
            app.config.onboarding.completed = true;
            app.config.onboarding.dismissed = false;
            app.config.onboarding.step = STEP_READY;
            app.notify("Guided setup complete — Play is ready", Tone::Ok);
        }
        Action::Play => {
            app.config.onboarding.completed = true;
            app.config.onboarding.dismissed = false;
            app.config.onboarding.step = STEP_READY;
            app.play();
        }
    }
    ctx.request_repaint();
}

fn intro_row(ui: &mut egui::Ui, p: &theme::Palette, number: usize, title: &str, body: &str) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(25.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 11.0, theme::alpha(p.accent, 30));
        ui.painter().circle_stroke(rect.center(), 11.0, egui::Stroke::new(1.0_f32, theme::alpha(p.accent, 110)));
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, number.to_string(), theme::semibold(10.0), p.accent_hi);
        ui.vertical(|ui| {
            ui.label(crate::i18n::text(title).font(theme::semibold(13.0)).color(p.ink));
            ui.add(egui::Label::new(crate::i18n::text(body).size(11.5).color(p.body)).wrap());
        });
    });
    ui.add_space(9.0);
}

fn file_row(ui: &mut egui::Ui, p: &theme::Palette, label: &str, ok: bool, path: Option<&std::path::PathBuf>) {
    ui.horizontal_top(|ui| {
        widgets::status_dot(ui, p, if ok { Tone::Ok } else { Tone::Neutral });
        ui.vertical(|ui| {
            ui.label(crate::i18n::text(label).font(theme::semibold(12.5)).color(p.ink));
            let detail = path.map(|path| compact_path(&path.display().to_string())).unwrap_or_else(|| if ok { "found".into() } else { "not found".into() });
            ui.add(egui::Label::new(crate::i18n::text(detail).size(10.8).color(if ok { p.muted } else { p.warn })).wrap());
        });
    });
    ui.add_space(5.0);
}

fn ready_row(ui: &mut egui::Ui, p: &theme::Palette, ok: bool, label: &str, detail: &str) {
    ui.horizontal_top(|ui| {
        widgets::status_dot(ui, p, if ok { Tone::Ok } else { Tone::Warn });
        ui.vertical(|ui| {
            ui.label(crate::i18n::text(label).font(theme::semibold(12.5)).color(p.ink));
            ui.add(egui::Label::new(crate::i18n::text(detail).size(11.0).color(if ok { p.muted } else { p.warn })).wrap());
        });
    });
    ui.add_space(7.0);
}

fn summary_tools(rows: Option<&[crate::deps::Row]>) -> &'static str {
    match rows {
        None => "Still checking",
        Some(rows) if crate::deps::blocking(rows).is_empty() => "Every required tool is installed",
        Some(_) => "Missing at least one required tool",
    }
}

fn last_build_failure(app: &App) -> Option<String> {
    let job = app.jobs.iter().rev().find(|j| j.kind == JobKind::Engine && j.finished())?;
    if crate::engine::ready() {
        return None;
    }
    let failed = job.snapshot(|pr| {
        pr.stages
            .iter()
            .find(|s| matches!(s.status, crate::jobs::StageStatus::Failed))
            .map(|s| format!("The last build stopped at “{}”: {}", s.name, s.detail))
    });
    failed.or_else(|| Some("The last build did not produce a runnable executable. Open the game log for its output.".into()))
}

fn compact_path(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= 74 {
        return text.to_string();
    }
    let tail: String = chars[chars.len() - 58..].iter().collect();
    format!("…{tail}")
}
