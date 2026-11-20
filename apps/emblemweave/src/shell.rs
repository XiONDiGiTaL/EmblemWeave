//! The application shell: compact title bar, grouped navigation rail, global
//! task strip and the expandable activity panel. Page content never draws its
//! own chrome.

use crate::pages::{NavGroup, Page};
use crate::pages::overview::Popup;
use crate::widgets::{self, Tone};
use crate::{logo, theme, App};
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::time::Duration;

/// The one-line answer about game data used by the header and status strip.
pub fn data_state(app: &App) -> (&'static str, Tone) {
    let busy = app.running(crate::jobs::JobKind::Detect).is_some();
    match (app.game.exefs.is_some(), app.game.romfs.is_some(), busy) {
        (true, true, _) => ("Ready", Tone::Ok),
        (_, _, true) => ("Checking", Tone::Accent),
        (true, false, _) => ("Incomplete", Tone::Warn),
        _ => ("Not set", Tone::Warn),
    }
}

fn build_state(app: &App) -> (&'static str, Tone) {
    if app.running(crate::jobs::JobKind::Engine).is_some() {
        ("Building", Tone::Accent)
    } else if crate::engine::ready() {
        ("Ready", Tone::Ok)
    } else if app.running(crate::jobs::JobKind::Pipeline).is_some() {
        ("Recompiling", Tone::Accent)
    } else {
        ("Not built", Tone::Neutral)
    }
}

fn tools_state() -> (String, Tone) {
    match crate::deps::cached() {
        Some(rows) => {
            let missing = crate::deps::blocking(&rows).len();
            if missing == 0 {
                ("Ready".into(), Tone::Ok)
            } else {
                (format!("{missing} missing"), Tone::Warn)
            }
        }
        None => ("Checking".into(), Tone::Neutral),
    }
}

/// A small, quiet header action. It is a text button with a subtle hover face,
/// not a second row of primary controls.
fn header_action(ui: &mut egui::Ui, p: &theme::Palette, text: &str, tip: &str) -> egui::Response {
    let text = crate::i18n::tr(text);
    let tip = crate::i18n::tr(tip);
    let font = theme::semibold(9.5);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, p.body);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + 20.0, 28.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool(response.id.with("h"), response.hovered());
        let painter = ui.painter();
        if hover > 0.0 {
            painter.rect_filled(rect, CornerRadius::same(8), alpha(Color32::WHITE, (7.0 * hover) as u8));
        }
        let ink = lerp(p.body, p.ink, hover);
        painter.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, ink);
        if response.has_focus() {
            painter.rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0_f32, p.electric), StrokeKind::Inside);
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

fn alpha(c: Color32, a: u8) -> Color32 {
    theme::alpha(c, a)
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

fn rail_toggle(ui: &mut egui::Ui, p: &theme::Palette, compact: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool(response.id.with("h"), response.hovered());
        let painter = ui.painter();
        if hover > 0.0 {
            painter.rect_filled(rect, CornerRadius::same(7), alpha(Color32::WHITE, (8.0 * hover) as u8));
        }
        let ink = lerp(p.muted, p.ink, hover);
        let s = Stroke::new(1.25_f32, ink);
        let c = rect.center();
        if compact {
            painter.line_segment([Pos2::new(c.x - 3.0, c.y - 4.0), Pos2::new(c.x + 1.0, c.y)], s);
            painter.line_segment([Pos2::new(c.x + 1.0, c.y), Pos2::new(c.x - 3.0, c.y + 4.0)], s);
        } else {
            painter.line_segment([Pos2::new(c.x + 3.0, c.y - 4.0), Pos2::new(c.x - 1.0, c.y)], s);
            painter.line_segment([Pos2::new(c.x - 1.0, c.y), Pos2::new(c.x + 3.0, c.y + 4.0)], s);
        }
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(crate::i18n::tr(if compact { "Show navigation labels" } else { "Collapse navigation to icons" }))
}

/// The application header: identity, page context, live game/build/tool state,
/// and the window controls. Forty-four pixels, one hairline, no banner.
pub fn top_bar(app: &mut App, ctx: &egui::Context) {
    let p = app.config.theme.palette();
    let width = ctx.screen_rect().width();
    egui::TopBottomPanel::top("topbar")
        .exact_height(theme::TOPBAR_HEIGHT)
        .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin { left: 8, right: 6, top: 4, bottom: 3 }))
        .show(ctx, |ui| {
            let strip = ui.available_rect_before_wrap();
            if ui.is_rect_visible(strip) {
                ui.painter().hline(strip.x_range(), strip.bottom(), Stroke::new(1.0_f32, p.line));
                let focus = Pos2::new(strip.left() + strip.width() * 0.18, strip.top());
                let mut mesh = egui::Mesh::default();
                mesh.colored_vertex(focus, alpha(p.accent, 10));
                for x in [strip.left(), strip.right()] {
                    mesh.colored_vertex(Pos2::new(x, strip.top()), Color32::TRANSPARENT);
                    mesh.colored_vertex(Pos2::new(x, strip.top() + 16.0), Color32::TRANSPARENT);
                }
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 3, 4);
                ui.painter().add(egui::Shape::mesh(mesh));
            }
            // The title strip is the window's native caption. Windows starts
            // the move in its hit-test pass (platform::install_native_drag),
            // before egui sees the press, so there is no frame of catch-up
            // slide. Controls, chips and window buttons stay outside that
            // native region and keep their normal pointer handling.

            let mut goto = None;
            let tools = tools_state();
            let (data_text, data_tone) = data_state(app);
            let (build_text, build_tone) = build_state(app);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                ui.add_space(2.0);
                if rail_toggle(ui, &p, app.config.nav.rail).clicked() {
                    app.config.nav.rail = !app.config.nav.rail;
                }
                let brand_left = ui.min_rect().right() + 3.0;
                let (mark, _) = ui.allocate_exact_size(Vec2::new(14.0, 14.0), Sense::hover());
                let c = mark.center();
                let diamond = vec![c + Vec2::new(0.0, -5.5), c + Vec2::new(5.5, 0.0), c + Vec2::new(0.0, 5.5), c + Vec2::new(-5.5, 0.0)];
                ui.painter().add(egui::Shape::convex_polygon(diamond, p.accent, Stroke::new(1.0_f32, alpha(Color32::WHITE, 170))));
                // The wordmark is the display face, not spaced capitals: it is
                // the one place the launcher signs its own name.
                ui.label(
                    crate::i18n::text("EmblemWeave")
                        .font(theme::display_bold(15.0))
                        .color(p.ink),
                );
                ui.add_space(3.0);
                let (div, _) = ui.allocate_exact_size(Vec2::new(1.0, 14.0), Sense::hover());
                ui.painter().vline(div.center().x, div.y_range(), Stroke::new(1.0_f32, p.line_strong));
                ui.add_space(4.0);
                widgets::tracked(ui, &crate::i18n::tr(app.page.label()).to_uppercase(), 8.5, p.accent_hi, theme::TRACK_MID);
                let brand_right = ui.min_rect().right() + 3.0;

                ui.add_space(10.0);
                // Keep the right-hand controls reachable at narrow widths:
                // the chips are the first thing to go, not the window buttons
                // or Settings.
                if width >= 980.0
                    && widgets::header_chip(ui, &p, "Game", data_text, data_tone).on_hover_text(crate::i18n::tr("Game data status · open Game Setup")).clicked()
                {
                    goto = Some(Page::Setup);
                }
                if width >= 1080.0
                    && widgets::header_chip(ui, &p, "Build", build_text, build_tone).on_hover_text(crate::i18n::tr("Compiled game status · open Recompilation")).clicked()
                {
                    goto = Some(Page::Recompile);
                }
                if width >= 1250.0
                    && widgets::header_chip(ui, &p, "Tools", &tools.0, tools.1)
                        .on_hover_text(crate::i18n::tr("Build dependency status · open Requirements"))
                        .clicked()
                {
                    goto = Some(Page::Requirements);
                }

                let left_inset = ui.min_rect().right() + 8.0;
                let right_group = ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    ui.add_space(super::SHELL_CONTROL_INSET);
                    if super::shell_control(ui, &p, super::ShellControl::Close, "Close").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    ui.add_space(super::SHELL_BUTTON_GAP);
                    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    if super::shell_control(ui, &p, super::ShellControl::Maximize { restore: maximized }, if maximized { "Restore" } else { "Maximize" }).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }
                    ui.add_space(super::SHELL_BUTTON_GAP);
                    if super::shell_control(ui, &p, super::ShellControl::Minimize, "Minimize").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    ui.add_space(8.0);
                    if width >= 1040.0 {
                        ui.label(crate::i18n::text(concat!("v", env!("CARGO_PKG_VERSION"), " · ", env!("EW_GIT_REV"))).font(theme::mono(9.5)).color(p.muted));
                        ui.add_space(6.0);
                    }
                    if width >= 1000.0 {
                        // The update control is also the update status: it
                        // checks GitHub when there is nothing ready and opens
                        // the Overview card once a release has been found.
                        let (value, tip, target, tone) = match &app.update.state {
                            crate::UpdateState::Ready(rel) => (
                                rel.tag.clone(),
                                "Open the update card on Overview",
                                Some(Page::Overview),
                                Tone::Accent,
                            ),
                            crate::UpdateState::Checking => (
                                "checking".to_string(),
                                "The update check is running",
                                None,
                                Tone::Neutral,
                            ),
                            crate::UpdateState::Downloading => (
                                "updating".to_string(),
                                "The update is downloading and installing",
                                None,
                                Tone::Accent,
                            ),
                            crate::UpdateState::UpToDate(_) => (
                                "latest".to_string(),
                                "You are on the newest release; check again",
                                None,
                                Tone::Ok,
                            ),
                            crate::UpdateState::Failed(_) => (
                                "retry".to_string(),
                                "The update check failed; try again",
                                None,
                                Tone::Bad,
                            ),
                            crate::UpdateState::InstallFailed(_) => (
                                "retry".to_string(),
                                "The update did not finish; try again",
                                None,
                                Tone::Bad,
                            ),
                            crate::UpdateState::Idle => (
                                "check".to_string(),
                                "Check GitHub for a newer release",
                                None,
                                Tone::Neutral,
                            ),
                        };
                        if widgets::header_chip(ui, &p, "UPDATE", &value, tone)
                            .on_hover_text(crate::i18n::tr(tip))
                            .clicked()
                        {
                            match target {
                                Some(page) => goto = Some(page),
                                None => app.check_updates(),
                            }
                        }
                        ui.add_space(4.0);
                    }
                    if width >= 900.0 && header_action(ui, &p, "SETTINGS", "Appearance and interface options").clicked() {
                        goto = Some(Page::Appearance);
                    }
                    if width >= 950.0 && header_action(ui, &p, "HELP", "Quick help, logs and troubleshooting").clicked() {
                        app.overview_popup = if app.overview_popup == Some(Popup::QuickHelp) { None } else { Some(Popup::QuickHelp) };
                    }
                    if width >= 1080.0 && header_action(ui, &p, "WHAT'S NEW", "What changed in this build").clicked() {
                        app.overview_popup = if app.overview_popup == Some(Popup::WhatsNew) { None } else { Some(Popup::WhatsNew) };
                    }
                    if width >= 1400.0 && header_action(ui, &p, "REQUIREMENTS", "Build tools and dependency checks").clicked() {
                        goto = Some(Page::Requirements);
                    }
                });
                // The native caption is the brand strip and the empty strip
                // between the chips and the right-hand actions. Reporting the
                // actual bounds keeps a press on either group in egui instead
                // of starting a move.
                let right_inset = (ctx.screen_rect().right() - right_group.response.rect.left() + 8.0).max(0.0);
                crate::platform::set_brand_caption_bounds(brand_left, brand_right);
                crate::platform::set_caption_bounds(left_inset, right_inset);
            });
            if let Some(page) = goto {
                app.open(page);
            }
        });
}

/// The grouped navigation rail. Groups are persistent drawers; the whole rail
/// can fold to an icon strip, with every label still available as a tooltip.
pub fn nav(app: &mut App, ctx: &egui::Context) {
    let p = app.config.theme.palette();
    let mode = app.config.interface_mode;
    let auto_compact = ctx.screen_rect().width() < 1200.0;
    let compact = app.config.nav.rail || auto_compact;
    let width = if compact { theme::NAV_RAIL_WIDTH } else { theme::NAV_WIDTH };
    if app.nav_last != Some(app.page) {
        app.nav_last = Some(app.page);
        let key = app.page.group().key().to_string();
        app.config.nav.collapsed.retain(|k| k != &key);
    }
    egui::SidePanel::left("nav")
        .exact_width(width)
        .resizable(false)
        .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin { left: 6, right: 3, top: 4, bottom: 4 }))
        .show(ctx, |ui| {
            let full = ui.available_rect_before_wrap();
            widgets::glass_panel(ui, full, &p, CornerRadius::same(12));
            if compact {
                ui.add_space(5.0);
                ui.vertical_centered(|ui| {
                    logo::show_lockup(ui, &p, 34.0);
                    ui.add_space(1.0);
                    if rail_toggle(ui, &p, true).clicked() {
                        app.config.nav.rail = false;
                    }
                });
            } else {
                ui.add_space(4.0);
                let header_w = ui.available_width();
                ui.horizontal_top(|ui| {
                    // Equal side gutters keep the lockup on the rail's true
                    // centre in every theme; the collapse control occupies the
                    // right gutter instead of pushing the artwork left. Both
                    // gutters shrink with the rail, so the wordmark is centred by
                    // what is left rather than by a fixed 28 px that a narrow
                    // rail cannot pay for.
                    let gutter = ((header_w - 96.0) / 2.0).clamp(4.0, 28.0);
                    let centre_w = (header_w - 2.0 * gutter - 16.0).max(24.0);
                    ui.add_space(gutter);
                    ui.allocate_ui_with_layout(
                        Vec2::new(centre_w, 0.0),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            ui.set_width(centre_w);
                        logo::show_lockup(ui, &p, centre_w.min(100.0));
                        ui.add_space(1.0);
                        widgets::tracked(ui, "RECOMPILATION LAUNCHER", 6.8, p.muted, 0.8);
                        },
                    );
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                        ui.add_space(2.0);
                        if rail_toggle(ui, &p, false).clicked() {
                            app.config.nav.rail = true;
                        }
                    });
                });
                ui.add_space(5.0);
                let line = ui.available_rect_before_wrap();
                ui.painter().hline(line.x_range(), line.top(), Stroke::new(1.0_f32, alpha(p.line, 150)));
                ui.add_space(3.0);
            }
            ui.spacing_mut().item_spacing.y = 1.0;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(ui.available_width());
                for group in NavGroup::ALL {
                    let pages = group.pages_for(mode);
                    if pages.is_empty() {
                        continue;
                    }
                    let open = !app.config.nav.collapsed.iter().any(|k| k == group.key());
                    if compact {
                        ui.add_space(5.0);
                        let _ = widgets::nav_group_header(ui, &p, group.label(), false, true, false);
                        for page in &pages {
                            if widgets::nav_item_page(ui, &p, *page, app.page == *page, true).clicked() {
                                app.open(*page);
                            }
                        }
                    } else {
                        let active = pages.contains(&app.page);
                        if widgets::nav_group_header(ui, &p, group.label(), open, false, active).clicked() {
                            if open {
                                app.config.nav.collapsed.push(group.key().to_string());
                            } else {
                                app.config.nav.collapsed.retain(|k| k != group.key());
                            }
                        }
                        if open {
                            for page in &pages {
                                let i = Page::ALL.iter().position(|q| q == page).unwrap_or(0);
                                let label = crate::i18n::tr(page.label());
                                let tip = if i < 9 { format!("{label}  (Ctrl+{})", i + 1) } else { label };
                                if widgets::nav_item_page(ui, &p, *page, app.page == *page, false).on_hover_text(tip).clicked() {
                                    app.open(*page);
                                }
                            }
                        }
                    }
                }
                ui.add_space(8.0);
                if !compact {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        let (ready, tone) = crate::pages::overview::readiness(app);
                        widgets::status_dot(ui, &p, tone);
                        ui.label(crate::i18n::text(ready).font(theme::semibold(10.0)).color(tone.colour(&p)));
                    });
                }
            });
        });
}

/// The status strip is the persistent, compact home for background work. The
/// Activity button opens the full task list without taking a permanent panel.
pub fn status_bar(app: &mut App, ctx: &egui::Context) {
    let p = app.config.theme.palette();
    egui::TopBottomPanel::bottom("status")
        .exact_height(theme::STATUSBAR_HEIGHT)
        .frame(egui::Frame::new().fill(p.bg_deep).inner_margin(egui::Margin { left: 12, right: 10, top: 2, bottom: 2 }))
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 9.0;
                let active = app.jobs.iter().find(|j| !j.finished() && j.kind != crate::jobs::JobKind::Detect);
                if let Some(job) = active {
                    let kind = job.kind;
                    let (message, copied, total, stages_done, stages_total) = job.snapshot(|pr| {
                        let done = pr.stages.iter().filter(|s| matches!(s.status, crate::jobs::StageStatus::Done | crate::jobs::StageStatus::Skipped | crate::jobs::StageStatus::Failed)).count();
                        (pr.message.clone(), pr.copied, pr.total, done, pr.stages.len())
                    });
                    let fraction = if stages_total > 0 {
                        Some(stages_done as f32 / stages_total as f32)
                    } else if total > 0 {
                        Some(copied as f32 / total as f32)
                    } else {
                        None
                    };
                    ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                    ui.label(crate::i18n::text(kind.title().to_uppercase()).font(theme::semibold(9.0)).color(p.accent));
                    let clean = message.split('[').next().unwrap_or(&message).trim();
                    let limit = if ctx.screen_rect().width() < 1000.0 { 30 } else if ctx.screen_rect().width() < 1300.0 { 46 } else { 72 };
                    let clean = if clean.chars().count() > limit { format!("{}…", clean.chars().take(limit).collect::<String>()) } else { clean.to_string() };
                    ui.label(crate::i18n::text(clean).size(10.5).color(p.body));
                    let frac = fraction.unwrap_or(0.0);
                    if fraction.is_some() {
                        let (bar, _) = ui.allocate_exact_size(Vec2::new(80.0, 5.0), Sense::hover());
                        ui.painter().rect_filled(bar, CornerRadius::same(2), p.panel_hi);
                        ui.painter().rect_filled(Rect::from_min_max(bar.min, Pos2::new(bar.left() + bar.width() * frac, bar.max.y)), CornerRadius::same(2), p.accent);
                        ui.label(crate::i18n::text(format!("{:.0}%", frac * 100.0)).font(theme::mono(9.5)).color(p.muted));
                    }
                    if widgets::button(ui, &p, "Cancel", true).clicked() {
                        app.confirm = Some(crate::Confirm::CancelJob(kind));
                    }
                    ctx.request_repaint_after(Duration::from_millis(120));
                } else {
                    let (text, tone) = crate::pages::overview::readiness(app);
                    widgets::status_dot(ui, &p, tone);
                    ui.label(crate::i18n::text(text.to_uppercase()).font(theme::semibold(9.0)).color(tone.colour(&p)));
                    // The readiness line is the one thing the strip must always say. The data state is the first to go
                    // when the window is too narrow to hold it beside the buttons on the right, instead of the two
                    // running into each other.
                    if ui.available_width() > 250.0 {
                        ui.label(crate::i18n::text("·").size(10.0).color(p.muted));
                        let (data, _) = data_state(app);
                        ui.label(
                            crate::i18n::text(format!(
                                "{}: {}",
                                crate::i18n::tr("data"),
                                crate::i18n::tr(data)
                            ))
                            .size(10.0)
                            .color(p.muted),
                        );
                    }
                    ui.add_space(10.0);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    // Right to left, each item is added only if the space that is left can hold it and a gap. The strip
                    // used to decide from the screen width, which is not the space the strip has: a narrow window drew
                    // the clock on top of LOG. The clock keeps a fixed slot so it cannot jitter as the time changes.
                    if ui.available_width() >= 104.0 {
                        // A zero-height slot left the clock hanging below the strip's centre; give it the row's height.
                        let clock_h = ui.available_height();
                        ui.allocate_ui_with_layout(Vec2::new(80.0, clock_h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(crate::i18n::text(super::clock()).font(theme::mono(9.5)).color(p.muted));
                        });
                    }
                    if ui.available_width() >= 54.0 && header_action(ui, &p, "LOG", "Open the game log").clicked() {
                        app.open(Page::Log);
                    }
                    if ui.available_width() >= 70.0 && app.jobs.iter().any(|j| !j.finished()) {
                        let (done, all) = app.jobs.iter().filter(|j| !j.finished()).fold((0, 0), |(d, a), j| {
                            let d = d + j.snapshot(|pr| pr.stages.iter().filter(|s| matches!(s.status, crate::jobs::StageStatus::Done)).count());
                            let a = a + j.snapshot(|pr| pr.stages.len());
                            (d, a)
                        });
                        if all > 0 {
                            ui.label(crate::i18n::text(format!("{done}/{all} stages")).font(theme::mono(9.5)).color(p.accent_hi));
                        }
                    }
                    let hub = activity_hub(ui, &p, &app.jobs, app.show_activity);
                    if hub.hovered() {
                        // A short grace period lets the pointer cross from the
                        // hub into the panel without it disappearing under it.
                        app.activity_hover_until = Some(std::time::Instant::now() + Duration::from_millis(450));
                        ctx.request_repaint_after(Duration::from_millis(150));
                    }
                    if hub.clicked() {
                        app.show_activity = !app.show_activity;
                        app.activity_hover_until = Some(std::time::Instant::now() + Duration::from_millis(450));
                    }
                });
            });
        });
}

/// The compact activity hub in the bottom strip. It is the only persistent
/// chrome for background work: a progress ring, a task count, and a hover/click
/// target for the translucent panel above it.
pub fn activity_hub(ui: &mut egui::Ui, p: &theme::Palette, jobs: &[crate::jobs::Job], pinned: bool) -> egui::Response {
    let (count, fraction, running) = jobs.iter().filter(|j| !j.finished()).fold((0usize, 0.0f32, false), |(n, f, _), j| {
        let (done, all) = j.snapshot(|pr| {
            let done = pr.stages.iter().filter(|s| matches!(s.status, crate::jobs::StageStatus::Done | crate::jobs::StageStatus::Skipped | crate::jobs::StageStatus::Failed)).count();
            (done, pr.stages.len())
        });
        let share = if all > 0 { done as f32 / all as f32 } else { 0.0 };
        (n + 1, f + share, true)
    });
    let fraction = if count > 0 { fraction / count as f32 } else { 0.0 };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(if running { 104.0 } else { 88.0 }, 18.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hub-hover"), response.hovered(), 0.12);
    let painter = ui.painter();
    let face = if pinned { theme::alpha(p.accent, 26) } else { theme::alpha(p.panel_hi, 185) };
    painter.rect_filled(rect, CornerRadius::same(9), lerp(face, p.surface, hover * 0.4));
    painter.rect_stroke(
        rect,
        CornerRadius::same(9),
        Stroke::new(1.0_f32, if pinned { theme::alpha(p.accent, 150) } else { p.line_strong }),
        StrokeKind::Inside,
    );
    let c = Pos2::new(rect.left() + 9.0, rect.center().y);
    painter.circle_filled(c, 5.0, theme::alpha(if running { p.accent } else { p.muted }, 28));
    let ring = if running { p.accent } else { p.muted };
    painter.circle_stroke(c, 4.0, Stroke::new(1.0_f32, theme::alpha(ring, 90)));
    if running && fraction > 0.0 {
        let mut last = c + Vec2::new(0.0, -4.0);
        for i in 1..=20 {
            let a = -std::f32::consts::FRAC_PI_2 + fraction * std::f32::consts::TAU * (i as f32 / 20.0);
            let next = c + Vec2::new(a.cos() * 4.0, a.sin() * 4.0);
            painter.line_segment([last, next], Stroke::new(1.5_f32, p.accent_hi));
            last = next;
        }
    } else {
        painter.circle_filled(c, 1.8, ring);
    }
    let label = if running { format!("{} {count}", crate::i18n::tr("ACTIVITY")) } else { crate::i18n::tr("ACTIVITY") };
    painter.text(Pos2::new(rect.left() + 18.0, rect.center().y), egui::Align2::LEFT_CENTER, label, theme::semibold(8.5), if pinned || hover > 0.0 { p.ink } else { p.body });
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(crate::i18n::tr(if pinned { "Activity is pinned · click to hide" } else { "Background tasks and recent activity · click to pin" }))
}

/// The expandable global activity panel. It lists live tasks with their stage
/// and recent messages, and never invents a percentage when one is not known.
pub fn activity_window(app: &mut App, ctx: &egui::Context) {
    // Capture and tour runs must show only the page under test.
    if app.screenshot.is_some() || app.tour.is_some() {
        return;
    }
    let now = std::time::Instant::now();
    let hovered_open = app.activity_hover_until.is_some_and(|until| until > now);
    let open = app.show_activity || hovered_open;
    // Keep the panel alive while the close animation runs, then stop drawing it.
    let fade = ctx.animate_bool_with_time(egui::Id::new("activity-fade"), open, 0.14);
    if !open && fade <= 0.001 {
        return;
    }
    let p = app.config.theme.palette();
    let mut close = false;
    let mut keep = false;
    let mut cancel = None;
    let area = egui::Area::new(egui::Id::new("activity-panel"))
        .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-14.0, -theme::STATUSBAR_HEIGHT - 8.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_opacity(fade);
            egui::Frame::new()
                .fill(theme::alpha(p.surface, 214))
                .stroke(Stroke::new(1.0_f32, p.line_strong))
                .corner_radius(CornerRadius::same(14))
                .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 28, spread: 0, color: Color32::from_black_alpha(150) })
                .inner_margin(egui::Margin::symmetric(15, 12))
                .show(ui, |ui| {
                    ui.set_width(348.0);
                    ui.horizontal(|ui| {
                        widgets::tracked(ui, "ACTIVITY", 9.5, p.accent, theme::TRACK_WIDE);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if header_action(ui, &p, "CLOSE", "Hide the preview").clicked() {
                                close = true;
                            }
                            if header_action(ui, &p, if app.show_activity { "UNPIN" } else { "PIN" }, "Keep the activity panel open").clicked() {
                                app.show_activity = !app.show_activity;
                                if !app.show_activity {
                                    app.activity_hover_until = None;
                                }
                            }
                        });
                    });
                    let line = ui.available_rect_before_wrap();
                    ui.painter().hline(line.x_range(), line.top(), Stroke::new(1.0_f32, p.line));
                    ui.add_space(7.0);
                    ui.label(crate::i18n::text("Background operations").font(theme::semibold(11.0)).color(p.ink));
                    if app.jobs.iter().all(|j| j.finished()) {
                        ui.label(crate::i18n::text("Nothing is running right now.").size(10.0).color(p.muted));
                    }
                    let list_h = (ctx.screen_rect().height() - 250.0).clamp(120.0, 350.0);
                    egui::ScrollArea::vertical().id_salt("activity-jobs").max_height(list_h).auto_shrink([false, true]).show(ui, |ui| {
                        for job in app.jobs.iter().filter(|j| !j.finished()) {
                            let stages = job.snapshot(|pr| pr.stages.clone());
                            ui.add_space(5.0);
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                                ui.label(crate::i18n::text(job.kind.title()).font(theme::semibold(11.0)).color(p.ink));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(crate::i18n::text(format!("{:.1}s", job.started.elapsed().as_secs_f32())).font(theme::mono(9.5)).color(p.muted));
                                    if widgets::button(ui, &p, "Stop", true).clicked() {
                                        cancel = Some(job.kind);
                                    }
                                });
                            });
                            if !stages.is_empty() {
                                widgets::progress_bar(ui, &p, Some(stages.iter().filter(|s| matches!(s.status, crate::jobs::StageStatus::Done | crate::jobs::StageStatus::Skipped)).count() as f32 / stages.len() as f32));
                                for s in stages.iter().filter(|s| matches!(s.status, crate::jobs::StageStatus::Active | crate::jobs::StageStatus::Failed)) {
                                    ui.label(crate::i18n::text(format!("{} · {}", s.name, s.detail)).size(10.0).color(p.body));
                                }
                            }
                        }
                    });
                    ui.add_space(7.0);
                    ui.label(crate::i18n::text("Recent launcher activity").font(theme::semibold(11.0)).color(p.ink));
                    egui::ScrollArea::vertical().id_salt("activity-log").max_height((list_h * 0.55).max(80.0)).auto_shrink([false, true]).show(ui, |ui| {
                        if app.activity.is_empty() {
                            ui.label(crate::i18n::text("Nothing yet.").size(10.0).color(p.muted));
                        }
                        for a in app.activity.iter().rev().take(14) {
                            ui.horizontal_top(|ui| {
                                ui.label(crate::i18n::text(&a.at).font(theme::mono(9.0)).color(p.muted));
                                ui.add(egui::Label::new(crate::i18n::text(&a.text).size(10.0).color(a.tone.colour(&p))).wrap());
                            });
                            ui.add_space(2.0);
                        }
                    });
                    ui.add_space(5.0);
                    if widgets::button(ui, &p, "Open game log", true).clicked() {
                        app.open(Page::Log);
                        close = true;
                    }
                });
        });
    if area.response.hovered() {
        keep = true;
    }
    if keep {
        app.activity_hover_until = Some(now + Duration::from_millis(450));
        ctx.request_repaint_after(Duration::from_millis(120));
    }
    if let Some(kind) = cancel {
        app.confirm = Some(crate::Confirm::CancelJob(kind));
    }
    if close {
        app.show_activity = false;
        app.activity_hover_until = None;
    }
}
