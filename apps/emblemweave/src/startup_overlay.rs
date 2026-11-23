//! The small loading card between Play and the game's first real frame.
//!
//! The engine writes `startup ready` when it presents a frame with enough draws
//! to be the title screen rather than the three-draw black loading frame. This
//! card watches the new part of `fefw.log` for that line, so it stays up while
//! the game window is black and leaves when the game is actually interactive.

use eframe::egui;
use std::io::{Read, Seek, SeekFrom};
use std::time::{Duration, Instant};

use crate::App;

const READY_MARKER: &str = "startup ready";
const WIDTH: f32 = 420.0;
const HEIGHT: f32 = 116.0;
const READY_HOLD: Duration = Duration::from_millis(700);
const MAX_WAIT: Duration = Duration::from_secs(180);
const GAME_START_GRACE: Duration = Duration::from_secs(60);

pub struct StartupOverlay {
    open: bool,
    log_from: u64,
    started: Instant,
    ready_at: Option<Instant>,
}

impl Default for StartupOverlay {
    fn default() -> Self {
        Self {
            open: false,
            log_from: 0,
            started: Instant::now(),
            ready_at: None,
        }
    }
}

impl StartupOverlay {
    /// Show the card for a launch that has just been spawned.
    pub fn begin(&mut self) {
        self.open = true;
        self.started = Instant::now();
        self.ready_at = None;
        self.log_from = crate::engine::run_dir()
            .and_then(|d| std::fs::metadata(d.join("fefw.log")).ok())
            .map(|m| m.len())
            .unwrap_or(0);
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
}

/// Draw the card while it is open. Safe to call every frame; it does nothing
/// otherwise.
pub fn frame(app: &mut App, ctx: &egui::Context) {
    if !app.startup.open {
        return;
    }

    let now = Instant::now();
    if app.startup.ready_at.is_none() {
        if let Some(text) = new_log_text(&mut app.startup.log_from) {
            if text.contains(READY_MARKER) {
                app.startup.ready_at = Some(now);
            }
        }
        if app.startup.started.elapsed() > MAX_WAIT {
            app.startup.open = false;
            return;
        }
        // A failed spawn leaves no process. The engine's own first-frame alert
        // still reports the useful error; this only keeps the card from sitting
        // on screen forever after a crash.
        if app.startup.started.elapsed() > GAME_START_GRACE && crate::cheats::game().is_none() {
            app.startup.open = false;
            return;
        }
    } else if app
        .startup
        .ready_at
        .is_some_and(|at| now.duration_since(at) >= READY_HOLD)
    {
        app.startup.open = false;
        return;
    }

    let monitor = ctx
        .input(|i| i.viewport().monitor_size)
        .unwrap_or(egui::vec2(1920.0, 1080.0));
    let position = egui::pos2(
        (monitor.x - WIDTH) * 0.5,
        (monitor.y - HEIGHT) * 0.5,
    );
    let vp = egui::ViewportId::from_hash_of("emblemweave-startup");
    let builder = egui::ViewportBuilder::default()
        .with_title("EmblemWeave — loading")
        .with_inner_size([WIDTH, HEIGHT])
        .with_position(position)
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top()
        .with_active(false);
    ctx.show_viewport_immediate(vp, builder, |ctx, _class| {
        if ctx.input(|i| i.viewport().close_requested()) {
            app.startup.open = false;
            return;
        }
        let p = app.config.theme.palette();
        let frame = egui::Frame::new()
            .fill(crate::theme::alpha(p.bg, 232))
            .stroke(egui::Stroke::new(1.0_f32, crate::theme::alpha(p.accent, 120)))
            .corner_radius(egui::CornerRadius::same(16))
            .inner_margin(egui::Margin::symmetric(22, 18));
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("Loading the game, please wait")
                            .font(crate::theme::semibold(15.0))
                            .color(p.ink),
                    );
                    ui.label(
                        egui::RichText::new("The first frame can take a little while on a cold start.")
                            .size(12.0)
                            .color(p.muted),
                    );
                });
            });
        });
    });
    ctx.request_repaint_after(Duration::from_millis(100));
}

fn new_log_text(from: &mut u64) -> Option<String> {
    let path = crate::engine::run_dir()?.join("fefw.log");
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len < *from {
        *from = 0;
        return None;
    }
    if len == *from {
        return None;
    }
    file.seek(SeekFrom::Start(*from)).ok()?;
    let mut text = String::new();
    file.take(len - *from).read_to_string(&mut text).ok()?;
    *from = len;
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_split_across_reads_is_still_found_by_the_caller() {
        // The caller keeps the whole new text for one read; a line split over
        // two writes would be two reads and is not a promise this parser makes.
        let text = "[ 1.0 I t1 nvn-render] startup ready: first frame with 64+ draws";
        assert!(text.contains(READY_MARKER));
    }
}