//! The Report An Issue workspace inside the Troubleshooter: a category picker,
//! conditional fields, a Markdown composer with a formatting toolbar, reviewed
//! attachments and diagnostics, a full report preview, and an explicit browser
//! handoff that never claims more than it did.

use crate::issue_report::{self, AddResult, Category, Draft, Facts, Field, Frequency, Validation};
use crate::theme::{self, Palette};
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText, Stroke};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct State {
    pub draft: Draft,
    loaded: bool,
    pub preview: bool,
    validation: Option<Validation>,
    show_validation: bool,
    facts: Option<Facts>,
    last_markdown: String,
    last_url: String,
    truncated: bool,
    dirty: bool,
    last_save: Option<Instant>,
    save_error: Option<String>,
    notice: Option<(String, Tone)>,
    reset_armed: bool,
}

impl State {
    pub fn summary(&self) -> String {
        self.draft.summary()
    }

    pub fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        if let Some(dir) = issue_report::data_dir() {
            if let Some(draft) = issue_report::load_draft(&dir) {
                self.draft = draft;
            }
        }
    }

    pub fn take_notice(&mut self) -> Option<(String, Tone)> {
        self.notice.take()
    }

    /// Saves a draft that was edited since the last autosave, for the moment the
    /// drawer is collapsed and the workspace no longer draws.
    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(dir) = issue_report::data_dir() else {
            self.save_error = Some("The launcher's data folder is not available, so the draft cannot be saved.".into());
            return;
        };
        match issue_report::save_draft(&dir, &self.draft) {
            Ok(_) => {
                self.dirty = false;
                self.last_save = Some(Instant::now());
                self.save_error = None;
            }
            Err(e) => self.save_error = Some(e),
        }
    }

    fn changed(&mut self) {
        self.dirty = true;
        self.facts = None;
        self.validation = None;
        if self.draft.confirmed_submitted {
            // A changed report is no longer the issue the user confirmed.
            self.draft.confirmed_submitted = false;
        }
    }

    fn start_new(&mut self) {
        let notice = (
            "Started a new draft. The prepared report folder and any issue already on GitHub were left untouched.".to_string(),
            Tone::Neutral,
        );
        *self = State { loaded: true, notice: Some(notice), ..State::default() };
        self.dirty = true;
        queue_save(self);
    }
}

pub fn workspace(app: &App, ui: &mut egui::Ui, p: &Palette, state: &mut State) {
    state.ensure_loaded();
    if state.preview {
        preview(app, ui, p, state);
        autosave(state);
        if state.dirty {
            ui.ctx().request_repaint_after(Duration::from_millis(1300));
        }
        return;
    }

    intro(ui, p);
    ui.add_space(theme::SPACE_M);

    ui.label(RichText::new("1 · What kind of report is this?").font(theme::semibold(13.0)).color(p.ink));
    ui.label(
        RichText::new("Pick the closest match; the fields below change to ask for the right evidence. Hover a category to see what belongs there.")
            .size(11.5)
            .color(p.muted),
    );
    ui.add_space(theme::SPACE_S);
    categories(ui, p, state);
    if let Some(cat) = state.draft.category {
        ui.add_space(theme::SPACE_XS);
        ui.label(RichText::new(cat.description()).size(11.5).color(p.body));
    }

    ui.add_space(theme::SPACE_L);
    ui.label(RichText::new("2 · Title").font(theme::semibold(13.0)).color(p.ink));
    let title = egui::TextEdit::singleline(&mut state.draft.title)
        .hint_text("Say what failed, not just \"help\" — for example: Game closes when chapter 2 loads")
        .desired_width(f32::INFINITY)
        .char_limit(160);
    if ui.add(title).changed() {
        state.changed();
    }
    ui.label(RichText::new(format!("{}/160", state.draft.title.chars().count())).size(10.5).color(p.muted));

    ui.add_space(theme::SPACE_M);
    let label = state.draft.category.map(Category::description_label).unwrap_or("What happened?");
    let hint = state.draft.category.map(Category::description_hint).unwrap_or("Describe the problem in your own words.");
    ui.label(RichText::new(format!("3 · {label}")).font(theme::semibold(13.0)).color(p.ink));
    ui.label(RichText::new(hint).size(11.5).color(p.muted));
    ui.add_space(theme::SPACE_XS);
    markdown_editor(app, ui, p, state);

    if let Some(cat) = state.draft.category {
        ui.add_space(theme::SPACE_M);
        for spec in issue_report::fields(cat) {
            field(ui, p, state, &spec);
        }
    }

    ui.add_space(theme::SPACE_M);
    environment(ui, p, state);
    ui.add_space(theme::SPACE_M);
    attachments(app, ui, p, state);
    ui.add_space(theme::SPACE_M);
    diagnostics(ui, p, state);
    ui.add_space(theme::SPACE_M);
    actions(app, ui, p, state);
    autosave(state);
    if state.dirty {
        // A draft is saved 1.2 s after the last edit; without this the window can
        // go idle before that save and lose the final keystrokes.
        ui.ctx().request_repaint_after(Duration::from_millis(1300));
    }
}

fn intro(ui: &mut egui::Ui, p: &Palette) {
    egui::Frame::new()
        .fill(theme::alpha(p.accent, 16))
        .stroke(Stroke::new(1.0_f32, theme::alpha(p.accent, 70)))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_S))
        .inner_margin(egui::Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                widgets::status_dot(ui, p, Tone::Neutral);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("A useful report is specific, reproducible, and carries the evidence.")
                            .font(theme::semibold(12.5))
                            .color(p.ink),
                    );
                    ui.label(
                        RichText::new(
                            "This workspace prepares a GitHub issue. It does not post or upload anything; you review and submit on GitHub, and attach the files there.",
                        )
                        .size(11.5)
                        .color(p.body),
                    );
                });
            });
        });
}

fn categories(ui: &mut egui::Ui, p: &Palette, state: &mut State) {
    let width = ui.available_width();
    let columns = if width < 560.0 { 1 } else { 2 };
    let gap = 7.0;
    let button_width = ((width - gap * (columns as f32 - 1.0)) / columns as f32).max(150.0);
    for row in Category::ALL.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for cat in row {
                let selected = state.draft.category == Some(*cat);
                let text = RichText::new(cat.label())
                    .font(theme::semibold(12.0))
                    .color(if selected { p.ink } else { p.body });
                let fill = if selected { theme::alpha(p.accent, 40) } else { p.panel_hi };
                let stroke = Stroke::new(1.0_f32, if selected { p.accent } else { p.line });
                let button = egui::Button::new(text)
                    .fill(fill)
                    .stroke(stroke)
                    .corner_radius(egui::CornerRadius::same(theme::RADIUS_S))
                    .min_size(egui::vec2(button_width, 33.0));
                let response = ui
                    .add_sized([button_width, 33.0], button)
                    .on_hover_text(cat.description());
                if response.clicked() {
                    state.draft.category = Some(*cat);
                    state.validation = None;
                    state.changed();
                }
            }
        });
        ui.add_space(4.0);
    }
}

fn field(ui: &mut egui::Ui, p: &Palette, state: &mut State, spec: &issue_report::FieldSpec) {
    ui.add_space(theme::SPACE_S);
    ui.label(RichText::new(spec.label).font(theme::semibold(12.5)).color(p.ink));
    ui.label(RichText::new(spec.hint).size(11.0).color(p.muted));
    let edit = egui::TextEdit::multiline(state.draft.details.get_mut(spec.field))
        .desired_rows(spec.rows as usize)
        .desired_width(f32::INFINITY)
        .hint_text(spec.hint);
    if ui.add(edit).changed() {
        state.changed();
    }
}

fn environment(ui: &mut egui::Ui, p: &Palette, state: &mut State) {
    egui::CollapsingHeader::new(RichText::new("Environment details").font(theme::semibold(12.5)).color(p.ink))
        .id_salt("issue-environment")
        .default_open(false)
        .show(ui, |ui| {
            ui.label(
                RichText::new("What you know about the setup. Leave a field empty if you are not sure; EmblemWeave will not invent a value.")
                    .size(11.0)
                    .color(p.muted),
            );
            ui.add_space(theme::SPACE_XS);
            env_field(ui, p, state, "Game version / title", "For example: 1.1.0, or the version shown on Game Setup", Field::GameVersion);
            env_field(ui, p, state, "Hardware / GPU", "GPU model and driver version, if relevant", Field::Hardware);
            env_field(ui, p, state, "Build configuration", "Release or Debug, and any build options you changed", Field::BuildConfig);

            if state.draft.category.is_some_and(Category::asks_for_frequency) {
                ui.add_space(theme::SPACE_S);
                ui.label(RichText::new("How often does it happen?").font(theme::semibold(12.0)).color(p.ink));
                ui.horizontal_wrapped(|ui| {
                    for f in Frequency::ALL {
                        if ui
                            .add(egui::Button::new(
                                RichText::new(f.label())
                                    .font(theme::semibold(11.5))
                                    .color(if state.draft.frequency == f { p.ink } else { p.body }),
                            )
                            .fill(if state.draft.frequency == f { theme::alpha(p.accent, 40) } else { p.panel_hi })
                            .stroke(Stroke::new(1.0_f32, if state.draft.frequency == f { p.accent } else { p.line })))
                            .clicked()
                        {
                            state.draft.frequency = f;
                            state.changed();
                        }
                    }
                });
            }
            if state.draft.category.is_some_and(Category::asks_when_started) {
                ui.add_space(theme::SPACE_S);
                ui.label(RichText::new("When did it start?").font(theme::semibold(12.0)).color(p.ink));
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut state.draft.started)
                            .hint_text("After updating, after changing a setting, always, not sure…")
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    state.changed();
                }
            }
        });
}

fn env_field(ui: &mut egui::Ui, p: &Palette, state: &mut State, label: &str, hint: &str, field: Field) {
    ui.add_space(theme::SPACE_S);
    ui.label(RichText::new(label).size(11.5).color(p.body));
    if ui
        .add(
            egui::TextEdit::singleline(state.draft.details.get_mut(field))
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        )
        .changed()
    {
        state.changed();
    }
}

fn markdown_editor(app: &App, ui: &mut egui::Ui, p: &Palette, state: &mut State) {
    let editor_id = egui::Id::new("issue-report-description");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (label, tip, op) in [
            ("B", "Bold", Op::Wrap("**")),
            ("I", "Italic", Op::Wrap("_")),
            ("H", "Heading", Op::LinePrefix("## ")),
            ("• List", "Bulleted list", Op::LinePrefix("- ")),
            ("1. List", "Numbered list", Op::LinePrefix("1. ")),
            ("`code`", "Inline code", Op::Wrap("`")),
            ("```", "Code block", Op::CodeBlock),
            ("Link", "Link", Op::Link),
            ("Clear", "Clear formatting", Op::Clear),
        ] {
            if widgets::button(ui, p, label, true).on_hover_text(crate::i18n::tr(tip)).clicked() {
                apply(ui, editor_id, &mut state.draft.description, op);
                state.changed();
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::button(ui, p, "Preview report", true)
                .on_hover_text(crate::i18n::tr("Shows the complete issue exactly as it will be generated. Your draft is kept."))
                .clicked()
            {
                open_preview(app, ui, p, state);
            }
        });
    });
    ui.add_space(4.0);
    let edit = egui::TextEdit::multiline(&mut state.draft.description)
        .id(editor_id)
        .desired_rows(9)
        .desired_width(f32::INFINITY)
        .hint_text("Markdown is supported. Toolbar buttons format the selection; every new line is kept.");
    if ui.add(edit).changed() {
        state.changed();
    }
    ui.label(
        RichText::new(format!(
            "{} characters · GitHub-compatible Markdown · drafts save automatically",
            state.draft.description.chars().count()
        ))
        .size(10.5)
        .color(p.muted),
    );
}

fn open_preview(app: &App, ui: &egui::Ui, _p: &Palette, state: &mut State) {
    let facts = Facts::collect(app, &state.draft.diagnostics);
    state.last_markdown = issue_report::markdown(&state.draft, &facts);
    state.facts = Some(facts);
    state.preview = true;
    state.validation = Some(issue_report::validate(&state.draft));
    state.show_validation = true;
    ui.ctx().request_repaint();
}

fn attachments(app: &App, ui: &mut egui::Ui, p: &Palette, state: &mut State) {
    ui.label(RichText::new("Evidence").font(theme::semibold(13.0)).color(p.ink));
    ui.label(
        RichText::new("Logs, a diagnostic ZIP, screenshots, configuration files. Files are copied for the handoff, never executed or unpacked.")
            .size(11.0)
            .color(p.muted),
    );
    let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
    let frame = egui::Frame::new()
        .fill(if hovering { theme::alpha(p.accent, 20) } else { p.panel_hi })
        .stroke(Stroke::new(1.0_f32, if hovering { p.accent } else { p.line }))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_S))
        .inner_margin(egui::Margin::symmetric(12, 10));
    let mut add_paths: Vec<PathBuf> = Vec::new();
    let mut remove: Option<usize> = None;
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, p, "Add files…", true)
                .on_hover_text(crate::i18n::tr("Choose logs, images, text or ZIP files from this PC."))
                .clicked()
            {
                if let Some(files) = rfd::FileDialog::new()
                    .add_filter("Logs, reports and text", &["log", "txt", "zip", "md", "json", "ini", "cfg", "toml", "csv"])
                    .add_filter("Images", &["png", "jpg", "jpeg", "webp", "gif"])
                    .add_filter("All files", &["*"])
                    .pick_files()
                {
                    add_paths.extend(files);
                }
            }
            if widgets::button(ui, p, "Add recent logs", true)
                .on_hover_text(crate::i18n::tr("Adds the newest task log, the game log, any freeze or crash report, and up to three crash reports. You can remove any of them."))
                .clicked()
            {
                add_paths.extend(recent_logs());
            }
            let can_bundle = !state.draft.title.trim().is_empty() || !state.draft.description.trim().is_empty();
            if widgets::button(ui, p, "Create diagnostic ZIP", can_bundle)
                .on_hover_text(crate::i18n::tr("Runs the existing diagnostic bundle: launcher settings, logs, game checks and crash reports, zipped under the data folder. Nothing is sent anywhere."))
                .clicked()
            {
                match crate::report::bundle(app, &state.draft.bundle_note()) {
                    Ok(path) => {
                        let validation = crate::report::validate_report(&path);
                        state.notice = Some((
                            if validation.0 {
                                format!("Diagnostic ZIP created: {}", path.display())
                            } else {
                                format!("Diagnostic ZIP created, but its check reported: {}", validation.1)
                            },
                            if validation.0 { Tone::Ok } else { Tone::Warn },
                        ));
                        add_paths.push(path);
                    }
                    Err(e) => state.notice = Some((format!("Could not create the diagnostic ZIP: {e}"), Tone::Bad)),
                }
            }
            ui.label(RichText::new("or drop files here").size(11.0).color(p.muted));
        });
        if state.draft.attachments.is_empty() {
            ui.add_space(theme::SPACE_XS);
            ui.label(RichText::new("No files attached yet.").size(11.5).color(p.muted));
        }
        for (i, a) in state.draft.attachments.iter().enumerate() {
            ui.add_space(4.0);
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0_f32, p.line))
                .corner_radius(egui::CornerRadius::same(theme::RADIUS_S))
                .inner_margin(egui::Margin::symmetric(9, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        widgets::pill(ui, p, a.kind.label(), Tone::Neutral);
                        let name = ui.label(RichText::new(&a.name).font(theme::semibold(11.5)).color(p.ink));
                        if a.path.is_file() {
                            name.on_hover_text(a.path.display().to_string());
                        } else {
                            name.on_hover_text("This file has moved or was deleted. Remove it or choose it again.");
                        }
                        ui.label(RichText::new(crate::fmt_size(a.size)).font(theme::mono(10.5)).color(p.muted));
                        if a.is_warning_name() {
                            widgets::pill(ui, p, "review", Tone::Warn).on_hover_text(
                                "The filename suggests credentials or a token. Remove it unless the file is safe to publish.",
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::button(ui, p, "Remove", true).clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                });
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!(
                "Limit: {} files, {} each, {} total (GitHub accepts 10 per comment). Executables, installers and credential files are refused. Do not attach passwords, tokens, keys, or saves with personal data.",
                issue_report::MAX_ATTACHMENTS,
                crate::fmt_size(issue_report::MAX_FILE_BYTES),
                crate::fmt_size(issue_report::MAX_TOTAL_BYTES)
            ))
            .size(10.5)
            .color(p.muted),
        );
    });
    if let Some(i) = remove {
        issue_report::remove_attachment(&mut state.draft, i);
        state.changed();
    }

    let dropped: Vec<PathBuf> = ui
        .ctx()
        .input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
    add_paths.extend(dropped);
    for path in add_paths {
        match issue_report::add_attachment(&mut state.draft, &path) {
            AddResult::Added(a) => {
                state.notice = Some((format!("Attached {} ({})", a.name, crate::fmt_size(a.size)), Tone::Ok));
                state.changed();
            }
            AddResult::Duplicate(why) => state.notice = Some((why, Tone::Warn)),
            AddResult::Rejected(why) => state.notice = Some((why, Tone::Bad)),
        }
    }
}

fn recent_logs() -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Some(log) = crate::jobs::newest_log() {
        files.push(log);
    }
    if let Some(dir) = crate::engine::run_dir() {
        for name in ["fefw.log", "fefw_freeze.txt", "fefw_fatal.txt", "fefw_crash.txt"] {
            let path = dir.join(name);
            if path.is_file() {
                files.push(path);
            }
        }
    }
    if let Some(dir) = crate::crash::reports_dir() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut reports: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            reports.sort();
            files.extend(reports.into_iter().rev().take(3));
        }
    }
    files
}

fn diagnostics(ui: &mut egui::Ui, p: &Palette, state: &mut State) {
    egui::CollapsingHeader::new(RichText::new("Diagnostics to include").font(theme::semibold(13.0)).color(p.ink))
        .id_salt("issue-diagnostics")
        .default_open(true)
        .show(ui, |ui| {
            ui.label(
                RichText::new("EmblemWeave fills these from the same checks the Troubleshooter already runs. Review them; nothing is sent until you submit on GitHub.")
                    .size(11.0)
                    .color(p.muted),
            );
            ui.add_space(4.0);
            let mut changed = false;
            changed |= diag_row(ui, p, "Application information", "EmblemWeave version, build revision, and architecture.", &mut state.draft.diagnostics.application);
            changed |= diag_row(ui, p, "System information", "Windows version, architecture, and the processor name. No user name or serial is collected.", &mut state.draft.diagnostics.system);
            changed |= diag_row(ui, p, "Build information", "Game title/version when detected, game-data verdict, whether a build is ready, and build freshness.", &mut state.draft.diagnostics.build);
            changed |= diag_row(ui, p, "Runtime diagnostics", "Last boot-test result, thread states, and recompiler counts from this session.", &mut state.draft.diagnostics.runtime);
            changed |= diag_row(ui, p, "Recent build log", "The tail of the newest build log plus the build-tool check. It can contain local paths.", &mut state.draft.diagnostics.logs);
            if changed {
                state.changed();
            }
            if !state.draft.diagnostics.any() {
                ui.label(RichText::new("No diagnostics selected. The issue can still be submitted, but developers may ask for these.").size(11.0).color(p.warn));
            }
            if state.draft.diagnostics.logs {
                ui.label(RichText::new("The log can include folder names. Remove paths you do not want public.").size(10.5).color(p.muted));
            }
        });
}

fn diag_row(ui: &mut egui::Ui, p: &Palette, label: &str, help: &str, value: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal_top(|ui| {
        if ui
            .checkbox(value, RichText::new(label).font(theme::semibold(11.5)).color(p.ink))
            .changed()
        {
            changed = true;
        }
        ui.label(RichText::new(help).size(10.5).color(p.muted));
    });
    changed
}

fn actions(app: &App, ui: &mut egui::Ui, p: &Palette, state: &mut State) {
    if let Some(validation) = state.validation.clone() {
        if state.show_validation {
            validation_list(ui, p, &validation);
        }
    }
    ui.add_space(theme::SPACE_S);
    ui.horizontal_wrapped(|ui| {
        if widgets::primary_button(ui, p, "Review report", true).clicked() {
            open_preview(app, ui, p, state);
        }
        let can_copy = !state.draft.description.trim().is_empty() || !state.draft.title.trim().is_empty();
        if widgets::button(ui, p, "Copy Markdown", can_copy).clicked() {
            let facts = state.facts.clone().unwrap_or_else(|| Facts::collect(app, &state.draft.diagnostics));
            let md = issue_report::markdown(&state.draft, &facts);
            ui.ctx().copy_text(md);
            state.notice = Some(("The generated Markdown was copied.".into(), Tone::Ok));
        }
        if widgets::primary_button(ui, p, "Open GitHub issue", true)
            .on_hover_text(crate::i18n::tr("Checks the report, copies the attachments into a handoff folder, and opens the prefilled new-issue page. EmblemWeave does not post the issue."))
            .clicked()
        {
            submit(app, ui, p, state);
        }
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new("No GitHub account is connected to EmblemWeave and no token is stored. The final Submit button is on GitHub, in your browser.")
            .size(10.5)
            .color(p.muted),
    );
    if state.dirty && state.last_save.is_some() {
        ui.label(RichText::new("Draft saved.").size(10.0).color(p.muted));
    }
    if let Some(e) = &state.save_error {
        ui.label(RichText::new(format!("Draft could not be saved: {e}")).size(10.5).color(p.warn));
    }
    ui.add_space(theme::SPACE_S);
    if state.reset_armed {
        ui.label(
            RichText::new("Start over? The current draft will be cleared. A prepared report folder on disk is not deleted.")
                .size(11.0)
                .color(p.warn),
        );
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, p, "Discard this draft", true).clicked() {
                state.start_new();
                return;
            }
            if widgets::button(ui, p, "Keep editing", true).clicked() {
                state.reset_armed = false;
            }
        });
    } else if widgets::button(ui, p, "Start a new report", true)
        .on_hover_text(crate::i18n::tr("Clears the draft so another issue can be written. Prepared folders and issues already on GitHub are not touched."))
        .clicked()
    {
        state.reset_armed = true;
    }
    if let Some((msg, tone)) = state.notice.clone() {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            widgets::status_dot(ui, p, tone);
            ui.label(RichText::new(msg).size(11.0).color(tone.colour(p)));
        });
    }
    handoff_status(app, ui, p, state);
}

fn validation_list(ui: &mut egui::Ui, p: &Palette, v: &Validation) {
    if !v.ok() {
        egui::Frame::new()
            .fill(theme::alpha(p.bad, 18))
            .stroke(Stroke::new(1.0_f32, theme::alpha(p.bad, 80)))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS_S))
            .inner_margin(egui::Margin::symmetric(11, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for e in &v.errors {
                    ui.horizontal_wrapped(|ui| {
                        widgets::status_dot(ui, p, Tone::Bad);
                        ui.label(RichText::new(e).size(11.5).color(p.ink));
                    });
                }
            });
        ui.add_space(4.0);
    }
    if !v.warnings.is_empty() {
        egui::Frame::new()
            .fill(theme::alpha(p.warn, 14))
            .stroke(Stroke::new(1.0_f32, theme::alpha(p.warn, 60)))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS_S))
            .inner_margin(egui::Margin::symmetric(11, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Worth checking before submitting").font(theme::semibold(11.5)).color(p.warn));
                for w in &v.warnings {
                    ui.horizontal_wrapped(|ui| {
                        widgets::status_dot(ui, p, Tone::Warn);
                        ui.label(RichText::new(w).size(11.0).color(p.body));
                    });
                }
            });
    }
}

fn submit(app: &App, ui: &egui::Ui, p: &Palette, state: &mut State) {
    if state.draft.confirmed_submitted {
        state.notice = Some((
            "This report is already marked submitted. Use Reopen GitHub in the prepared report panel, or start a new report, so a second issue is not created by accident."
                .into(),
            Tone::Warn,
        ));
        return;
    }
    let validation = issue_report::validate(&state.draft);
    state.validation = Some(validation.clone());
    state.show_validation = true;
    if !validation.ok() {
        state.preview = false;
        state.notice = Some(("The report is not ready: fix the errors above, then try again.".into(), Tone::Bad));
        return;
    }
    let facts = Facts::collect(app, &state.draft.diagnostics);
    let markdown = issue_report::markdown(&state.draft, &facts);
    let (url, truncated) = issue_report::issue_url(&state.draft, &facts);
    state.facts = Some(facts);
    state.last_markdown = markdown.clone();
    state.last_url = url.clone();
    state.truncated = truncated;

    let Some(data) = issue_report::data_dir() else {
        state.notice = Some((
            "The launcher's data folder is unavailable, so the attachments cannot be preserved. Nothing was opened."
                .into(),
            Tone::Bad,
        ));
        return;
    };
    let root = data.join("reports");
    match issue_report::prepare_handoff(&root, &state.draft, &markdown) {
        Ok(dir) => {
            state.draft.handoff_dir = Some(dir.clone());
            state.draft.confirmed_submitted = false;
            state.dirty = true;
            queue_save(state);
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            state.notice = Some((
                format!(
                    "GitHub was opened in your browser. This issue is NOT submitted yet. Attach files from {}",
                    dir.display()
                ),
                Tone::Warn,
            ));
        }
        Err(e) => {
            state.notice = Some((
                format!("Could not preserve the report and attachments, so GitHub was not opened: {e}"),
                Tone::Bad,
            ));
        }
    }
    let _ = p;
}

fn handoff_status(app: &App, ui: &mut egui::Ui, p: &Palette, state: &mut State) {
    let Some(dir) = state.draft.handoff_dir.clone() else { return };
    ui.add_space(theme::SPACE_M);
    egui::Frame::new()
        .fill(p.panel_hi)
        .stroke(Stroke::new(1.0_f32, if state.draft.confirmed_submitted { p.line } else { p.accent }))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_S))
        .inner_margin(egui::Margin::symmetric(13, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (title, tone) = if state.draft.confirmed_submitted {
                ("Report marked submitted by you — not verified by EmblemWeave", Tone::Warn)
            } else {
                ("Prepared for GitHub — NOT submitted", Tone::Warn)
            };
            ui.horizontal_wrapped(|ui| {
                widgets::status_dot(ui, p, tone);
                ui.label(RichText::new(title).font(theme::semibold(12.0)).color(tone.colour(p)));
            });
            if state.truncated && !state.draft.confirmed_submitted {
                ui.label(RichText::new("The URL body was shortened; paste the complete issue.md from the folder if any section is missing.").size(10.5).color(p.warn));
            }
            if state.draft.confirmed_submitted {
                ui.label(
                    RichText::new("EmblemWeave cannot see GitHub's issue list. Only the issue page in your browser confirms that GitHub accepted it.")
                        .size(10.5)
                        .color(p.muted),
                );
            } else {
                ui.label(
                    RichText::new("The files below were copied for you. Drag them into the GitHub issue before pressing Submit there.")
                        .size(11.0)
                        .color(p.body),
                );
                if !state.draft.attachments.is_empty() {
                    for a in &state.draft.attachments {
                        ui.label(RichText::new(format!("  • {}", a.name)).font(theme::mono(10.5)).color(p.body));
                    }
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if widgets::button(ui, p, "Open prepared folder", true).clicked() {
                        let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                    }
                    if widgets::button(ui, p, "Copy issue Markdown", true).clicked() {
                        let md = if state.last_markdown.is_empty() {
                            let facts = Facts::collect(app, &state.draft.diagnostics);
                            issue_report::markdown(&state.draft, &facts)
                        } else {
                            state.last_markdown.clone()
                        };
                        ui.ctx().copy_text(md);
                        state.notice = Some(("The complete issue Markdown was copied.".into(), Tone::Ok));
                    }
                    if widgets::button(ui, p, "Reopen GitHub", !state.last_url.is_empty()).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(state.last_url.clone()));
                    }
                });
                ui.add_space(4.0);
                if widgets::button(ui, p, "I submitted this issue on GitHub", true)
                    .on_hover_text(crate::i18n::tr("Only use this after GitHub shows the created issue. It records your confirmation on this PC; it does not check GitHub."))
                    .clicked()
                {
                    state.draft.confirmed_submitted = true;
                    state.dirty = true;
                    queue_save(state);
                    state.notice = Some((
                        "Marked as submitted by you. EmblemWeave did not post it and cannot verify it.".into(),
                        Tone::Warn,
                    ));
                }
            }
        });
}

fn preview(app: &App, ui: &mut egui::Ui, p: &Palette, state: &mut State) {
    let facts = state.facts.clone().unwrap_or_else(|| Facts::collect(app, &state.draft.diagnostics));
    let model = issue_report::preview(&state.draft, &facts);
    state.last_markdown = model.markdown.clone();
    ui.horizontal_wrapped(|ui| {
        if widgets::button(ui, p, "← Back to editing", true).clicked() {
            state.preview = false;
            ui.ctx().request_repaint();
        }
        ui.label(RichText::new("Preview · exactly what Open GitHub issue will prepare").size(11.0).color(p.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::button(ui, p, "Open GitHub issue", true).clicked() {
                state.preview = false;
                submit(app, ui, p, state);
            }
        });
    });
    ui.add_space(theme::SPACE_M);
    ui.horizontal_wrapped(|ui| {
        widgets::pill(ui, p, model.category, Tone::Neutral);
        widgets::pill(ui, p, &format!("{} attachment(s)", model.attachments.len()), Tone::Neutral);
        if let Some(label) = model.github_label {
            widgets::pill(ui, p, &format!("label: {label}"), Tone::Ok);
        }
    });
    ui.add_space(theme::SPACE_S);
    ui.label(RichText::new(&model.title).font(theme::semibold(19.0)).color(p.ink));
    ui.add_space(theme::SPACE_S);
    egui::Frame::new()
        .fill(p.bg_deep)
        .stroke(Stroke::new(1.0_f32, p.line))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_S))
        .inner_margin(egui::Margin::symmetric(16, 13))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // The page already scrolls; a nested ScrollArea hid the rest of the
            // report behind an almost invisible second scrollbar.
            markdown_preview(ui, p, &model.markdown);
        });
    ui.add_space(theme::SPACE_S);
    ui.label(
        RichText::new("This is the body placed in the GitHub issue. Diagnostics follow your checkboxes; empty sections are omitted. Attachments are listed but must be dragged into GitHub by you.")
            .size(10.5)
            .color(p.muted),
    );
    if let Some(v) = &state.validation {
        ui.add_space(theme::SPACE_S);
        validation_list(ui, p, v);
    }
}

fn markdown_preview(ui: &mut egui::Ui, p: &Palette, text: &str) {
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            i += 1;
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0_f32, p.line))
                .corner_radius(egui::CornerRadius::same(5))
                .inner_margin(egui::Margin::symmetric(9, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for l in &code {
                        ui.label(RichText::new(*l).font(theme::mono(10.5)).color(p.body));
                    }
                });
            ui.add_space(5.0);
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("<details") || trimmed.starts_with("</details") {
            i += 1;
            continue;
        }
        if let Some(t) = trimmed.strip_prefix("<summary>") {
            ui.add_space(4.0);
            ui.label(RichText::new(t.trim_end_matches("</summary>")).font(theme::semibold(11.0)).color(p.muted));
        } else if let Some(t) = trimmed.strip_prefix("### ") {
            ui.add_space(6.0);
            ui.label(RichText::new(strip_inline(t)).font(theme::semibold(12.0)).color(p.ink));
        } else if let Some(t) = trimmed.strip_prefix("## ") {
            ui.add_space(9.0);
            ui.label(RichText::new(strip_inline(t)).font(theme::semibold(14.0)).color(p.ink));
            ui.add_space(4.0);
        } else if let Some(t) = trimmed.strip_prefix("> ") {
            ui.label(RichText::new(strip_inline(t)).size(10.5).italics().color(p.muted));
        } else if let Some(t) = trimmed.strip_prefix("- ") {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new("•").size(11.5).color(p.accent_hi));
                ui.label(RichText::new(strip_inline(t)).size(11.5).color(p.body));
            });
        } else if let Some((label, rest)) = trimmed.split_once(':').filter(|(l, _)| {
            matches!(
                l.trim_start_matches('*').trim_end_matches('*'),
                "Category" | "Frequency" | "Started"
            )
        }) {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{}:", strip_inline(label))).font(theme::semibold(11.0)).color(p.muted));
                ui.label(RichText::new(strip_inline(rest)).size(11.5).color(p.body));
            });
        } else {
            ui.label(RichText::new(strip_inline(trimmed)).size(11.5).color(p.body));
        }
        i += 1;
    }
}

fn strip_inline(text: &str) -> String {
    text.replace("**", "").replace("__", "").replace('`', "")
}

enum Op {
    Wrap(&'static str),
    LinePrefix(&'static str),
    CodeBlock,
    Link,
    Clear,
}

fn apply(ui: &egui::Ui, id: egui::Id, text: &mut String, op: Op) {
    let (mut start, mut end) = selection(ui, id, text);
    if start > text.len() {
        start = text.len();
    }
    if end > text.len() {
        end = text.len();
    }
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    match op {
        Op::Wrap(marker) => {
            let selected = text[start..end].to_string();
            let replacement = format!("{marker}{selected}{marker}");
            text.replace_range(start..end, &replacement);
            let caret = if selected.is_empty() { start + marker.len() } else { start + replacement.len() };
            set_cursor(ui, id, text, caret);
        }
        Op::LinePrefix(prefix) => {
            let line_start = text[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
            text.insert_str(line_start, prefix);
            set_cursor(ui, id, text, start + prefix.len());
        }
        Op::CodeBlock => {
            let selected = text[start..end].to_string();
            let replacement = format!("\n```\n{selected}\n```\n");
            text.replace_range(start..end, &replacement);
            set_cursor(ui, id, text, start + replacement.len());
        }
        Op::Link => {
            let selected = text[start..end].to_string();
            let label = if selected.trim().is_empty() { "link text" } else { &selected };
            let replacement = format!("[{label}](https://)");
            text.replace_range(start..end, &replacement);
            let caret = start + replacement.len() - 1;
            set_cursor(ui, id, text, caret);
        }
        Op::Clear => {
            let (from, to) = if start == end { (0, text.len()) } else { (start, end) };
            let cleaned = clear_formatting(&text[from..to]);
            text.replace_range(from..to, &cleaned);
            set_cursor(ui, id, text, from + cleaned.len());
        }
    }
    ui.ctx().request_repaint();
}

fn clear_formatting(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut line = line.trim_start();
        while line.starts_with('#') {
            line = &line[1..];
        }
        line = line.strip_prefix(' ').unwrap_or(line);
        if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            line = rest;
        } else if let Some((n, rest)) = line.split_once(". ") {
            if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) {
                line = rest;
            }
        }
        out.push_str(&line.replace("**", "").replace("__", "").replace('`', ""));
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn selection(ui: &egui::Ui, id: egui::Id, text: &str) -> (usize, usize) {
    let Some(state) = egui::text_edit::TextEditState::load(ui.ctx(), id) else {
        return (text.len(), text.len());
    };
    let Some(range) = state.cursor.char_range() else {
        return (text.len(), text.len());
    };
    let a = char_to_byte(text, range.primary.index.min(range.secondary.index));
    let b = char_to_byte(text, range.primary.index.max(range.secondary.index));
    (a, b)
}

fn char_to_byte(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map(|(i, _)| i).unwrap_or(text.len())
}

fn set_cursor(ui: &egui::Ui, id: egui::Id, text: &str, byte: usize) {
    let index = text[..byte.min(text.len())].chars().count();
    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
        egui::text::CCursor::new(index),
        egui::text::CCursor::new(index),
    )));
    state.store(ui.ctx(), id);
    ui.ctx().memory_mut(|m| m.request_focus(id));
}

fn autosave(state: &mut State) {
    if !state.dirty {
        return;
    }
    if state.last_save.is_some_and(|t| t.elapsed() < Duration::from_millis(1200)) {
        return;
    }
    queue_save(state);
}

fn queue_save(state: &mut State) {
    let Some(dir) = issue_report::data_dir() else {
        state.save_error = Some("The launcher's data folder is not available.".into());
        return;
    };
    match issue_report::save_draft(&dir, &state.draft) {
        Ok(_) => {
            state.dirty = false;
            state.last_save = Some(Instant::now());
            state.save_error = None;
        }
        Err(e) => state.save_error = Some(e),
    }
}
