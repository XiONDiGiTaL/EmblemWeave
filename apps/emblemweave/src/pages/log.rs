//! Game Log: the game engine's log, followed live while the game runs, with
//! level filters and search, in the launcher or popped out into a window of
//! its own.

use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

/// Lines kept in memory; older ones scroll away.
const KEEP: usize = 50_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

pub struct Line {
    pub level: Level,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Current,
    Previous,
}

pub struct Ui {
    lines: std::collections::VecDeque<Line>,
    read_to: u64,
    which: Which,
    /// Bytes of a line not finished yet.
    partial: String,
    pub popped: bool,
    search: String,
    show_info: bool,
    show_warn: bool,
    show_error: bool,
    follow: bool,
    last_poll: std::time::Instant,
    /// The indices that pass the filters, and the filters they were built for.
    /// A log keeps up to 50,000 lines, and walking all of them (with a
    /// lower-case copy each while searching) on every frame cost more than
    /// everything else the page draws. The list is rebuilt only when a new
    /// chunk arrives or a filter changes.
    shown_rows: Vec<usize>,
    filter: Option<(bool, bool, bool, String)>,
    counts: (usize, usize),
    /// New lines or a reset since the filter was last built.
    stale: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Ui {
            lines: Default::default(),
            read_to: 0,
            which: Which::Current,
            partial: String::new(),
            popped: false,
            search: String::new(),
            show_info: true,
            show_warn: true,
            show_error: true,
            follow: true,
            last_poll: std::time::Instant::now() - std::time::Duration::from_secs(5),
            shown_rows: Vec::new(),
            filter: None,
            counts: (0, 0),
            stale: true,
        }
    }
}

/// The level letter after the time stamp: `[  0.445563 W t20172 -] ...`.
pub fn level_of(line: &str) -> Level {
    let after = line.trim_start_matches('[').trim_start();
    match after.split_whitespace().nth(1) {
        Some("W") => Level::Warn,
        Some("E") | Some("F") => Level::Error,
        _ => Level::Info,
    }
}

fn path(which: Which) -> Option<PathBuf> {
    crate::engine::run_dir().map(|d| d.join(if which == Which::Current { "fefw.log" } else { "fefw.log.prev" }))
}

impl Ui {
    fn reset(&mut self) {
        self.lines.clear();
        self.read_to = 0;
        self.partial.clear();
        self.stale = true;
    }

    /// Rebuild the visible rows and the warning and error counts: on a new
    /// chunk of log, a changed filter, or a cleared view, never per frame.
    fn refresh(&mut self) {
        let filter = (self.show_info, self.show_warn, self.show_error, self.search.clone());
        if !self.stale && self.filter.as_ref() == Some(&filter) {
            return;
        }
        let needle = self.search.to_lowercase();
        self.counts = (0, 0);
        self.shown_rows.clear();
        for (i, l) in self.lines.iter().enumerate() {
            match l.level {
                Level::Warn => self.counts.0 += 1,
                Level::Error => self.counts.1 += 1,
                Level::Info => {}
            }
            let level_shown = match l.level {
                Level::Info => self.show_info,
                Level::Warn => self.show_warn,
                Level::Error => self.show_error,
            };
            if level_shown && (needle.is_empty() || l.text.to_lowercase().contains(&needle)) {
                self.shown_rows.push(i);
            }
        }
        self.filter = Some(filter);
        self.stale = false;
    }

    /// Read what was added to the log since the last look. A file that got
    /// shorter is a new run: start again from its top.
    fn poll(&mut self) {
        let Some(p) = path(self.which) else { return };
        let Ok(mut f) = std::fs::File::open(&p) else { return };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.read_to {
            self.reset();
        }
        if len == self.read_to || f.seek(SeekFrom::Start(self.read_to)).is_err() {
            return;
        }
        let mut buf = Vec::new();
        // At most 8 MiB per look, so a huge log does not stall the window.
        if f.by_ref().take(8 << 20).read_to_end(&mut buf).is_err() {
            return;
        }
        self.read_to += buf.len() as u64;
        self.partial.push_str(&String::from_utf8_lossy(&buf));
        let mut rest = String::new();
        if !self.partial.ends_with('\n') {
            if let Some(i) = self.partial.rfind('\n') {
                rest = self.partial[i + 1..].to_string();
                self.partial.truncate(i + 1);
            } else {
                return;
            }
        }
        for l in self.partial.lines() {
            if !l.is_empty() {
                self.lines.push_back(Line { level: level_of(l), text: l.to_string() });
            }
        }
        while self.lines.len() > KEEP {
            self.lines.pop_front();
        }
        self.partial = rest;
        self.stale = true;
    }

    fn counts(&self) -> (usize, usize) {
        self.counts
    }
}

/// The log view itself, shared by the page and the popped-out window.
fn view(app: &mut App, ui: &mut egui::Ui, in_window: bool) {
    let p = app.config.theme.palette();
    let s = &mut app.log_ui;
    if s.last_poll.elapsed() > std::time::Duration::from_millis(400) {
        s.last_poll = std::time::Instant::now();
        s.poll();
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    s.refresh();
    let (warns, errors) = s.counts();
    ui.horizontal_wrapped(|ui| {
        for (w, label) in [(Which::Current, "This run"), (Which::Previous, "Previous run")] {
            if ui.selectable_label(s.which == w, crate::i18n::text(label)).clicked() && s.which != w {
                s.which = w;
                s.reset();
                s.poll();
            }
        }
        ui.separator();
        ui.checkbox(&mut s.show_info, crate::i18n::text("Info"));
        ui.checkbox(&mut s.show_warn, crate::i18n::text(format!("Warnings ({warns})")).color(p.warn));
        ui.checkbox(&mut s.show_error, crate::i18n::text(format!("Errors ({errors})")).color(p.bad));
        ui.separator();
        ui.add(egui::TextEdit::singleline(&mut s.search).hint_text("search").desired_width(180.0));
        ui.checkbox(&mut s.follow, crate::i18n::text("Follow"));
    });
    ui.horizontal(|ui| {
        if widgets::button(ui, &p, "Copy shown", !s.lines.is_empty()).clicked() {
            let f = s.search.to_lowercase();
            let text: Vec<&str> = s
                .lines
                .iter()
                .filter(|l| shown(l, s.show_info, s.show_warn, s.show_error, &f))
                .map(|l| l.text.as_str())
                .collect();
            ui.ctx().copy_text(text.join("\n"));
        }
        if widgets::button(ui, &p, "Open log file", true).clicked() {
            if let Some(path) = path(s.which) {
                let _ = std::process::Command::new("notepad").arg(path).spawn();
            }
        }
        if widgets::button(ui, &p, "Clear view", !s.lines.is_empty()).clicked() {
            s.lines.clear();
            s.stale = true;
        }
        if !in_window && widgets::button(ui, &p, "↗ Pop out", true).on_hover_text(crate::i18n::tr("Open the log in a window of its own")).clicked() {
            s.popped = true;
        }
    });
    ui.add_space(6.0);
    // The filter may have changed in the row above; rebuild before drawing.
    s.refresh();
    let rows = &s.shown_rows;
    if s.lines.is_empty() {
        ui.label(crate::i18n::text("The log is empty. It fills in while the game runs (the game writes fefw.log in its run folder).").color(p.muted));
        return;
    }
    let row_h = 17.0;
    let height = if in_window { ui.available_height() } else { ui.available_height().max(460.0) };
    egui::Frame::new().fill(p.bg_deep).corner_radius(egui::CornerRadius::same(theme::RADIUS_S)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        egui::ScrollArea::both().auto_shrink([false, false]).max_height(height).stick_to_bottom(s.follow).show_rows(ui, row_h, rows.len(), |ui, range| {
            for &i in &rows[range] {
                let l = &s.lines[i];
                let c = match l.level {
                    Level::Error => p.bad,
                    Level::Warn => p.warn,
                    Level::Info => p.body,
                };
                ui.add(egui::Label::new(crate::i18n::text(&l.text).font(theme::mono(12.0)).color(c)).extend());
            }
        });
    });
}

fn shown(l: &Line, info: bool, warn: bool, error: bool, f: &str) -> bool {
    let level = match l.level {
        Level::Info => info,
        Level::Warn => warn,
        Level::Error => error,
    };
    level && (f.is_empty() || l.text.to_lowercase().contains(f))
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Game Log", "What the game reports while it runs: warnings, errors and everything else.");
    if app.log_ui.popped {
        widgets::section(ui, &p, "", |ui| {
            widgets::status_line(ui, &p, Tone::Neutral, "Popped out", "The log is in its own window.");
            ui.add_space(6.0);
            if widgets::button(ui, &p, "Bring it back", true).clicked() {
                app.log_ui.popped = false;
            }
        });
        return;
    }
    widgets::section(ui, &p, "", |ui| view(app, ui, false));
}

/// Draws the popped-out log window while it is open.
pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.log_ui.popped {
        return;
    }
    let builder = egui::ViewportBuilder::default().with_title("EmblemWeave — Game Log").with_inner_size([980.0, 620.0]);
    let builder = match crate::window_icon() {
        Some(icon) => builder.with_icon(std::sync::Arc::new(icon)),
        None => builder,
    };
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("emblemweave-log"), builder, |ctx, _| {
        if ctx.input(|i| i.viewport().close_requested()) {
            app.log_ui.popped = false;
            return;
        }
        let p = app.config.theme.palette();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin::same(14))).show(ctx, |ui| view(app, ui, true));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_levels() {
        assert_eq!(level_of("[  0.445563 W t20172 -] vcodec: built without libvpx"), Level::Warn);
        assert_eq!(level_of("[  0.011252 I t20172 -] runtime: nxrt starting"), Level::Info);
        assert_eq!(level_of("[207.010752 E t17528 hitch] x"), Level::Error);
        assert_eq!(level_of("no prefix"), Level::Info);
    }
}
