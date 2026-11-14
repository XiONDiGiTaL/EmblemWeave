//! Game Setup: locate, validate and prepare the user's own game data, and the
//! system values the runtime reports to the game.

use crate::gamedata::Severity;
use crate::jobs::JobKind;
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText};

const LANGUAGES: [(&str, &str); 12] = [
    ("en-US", "English (US)"),
    ("en-GB", "English (UK)"),
    ("ja", "日本語"),
    ("fr", "Français"),
    ("fr-CA", "Français (Canada)"),
    ("de", "Deutsch"),
    ("es", "Español"),
    ("es-419", "Español (Latinoamérica)"),
    ("it", "Italiano"),
    ("ko", "한국어"),
    ("zh-Hans", "简体中文"),
    ("zh-Hant", "繁體中文"),
];

/// What a path row's Browse button picks.
#[derive(Clone, Copy, PartialEq)]
enum Pick {
    Folder,
    Nacp,
    /// The game data: a `.nup` dump, an `.nsp` package, or a folder.
    Dump,
}

fn path_row(ui: &mut egui::Ui, p: &theme::Palette, label: &str, value: &mut String, hint: &str, pick: Pick) -> bool {
    let mut changed = false;
    ui.label(RichText::new(label).font(theme::semibold(13.5)).color(p.ink));
    ui.horizontal(|ui| {
        let w = ui.available_width() - if pick == Pick::Dump { 236.0 } else { 110.0 };
        changed |= ui.add(egui::TextEdit::singleline(value).desired_width(w).hint_text(hint).font(theme::mono(12.5))).lost_focus();
        let mut picked = None;
        if pick == Pick::Dump && widgets::button(ui, p, "Choose package…", true).clicked() {
            picked = rfd::FileDialog::new()
                .add_filter("Game package (.nup, .nsp)", &["nup", "nsp"])
                .pick_file();
        }
        let browse = if pick == Pick::Dump { "Folder…" } else { "Browse…" };
        if widgets::button(ui, p, browse, true).clicked() {
            picked = match pick {
                Pick::Nacp => rfd::FileDialog::new().add_filter("control.nacp", &["nacp"]).pick_file(),
                _ => rfd::FileDialog::new().pick_folder(),
            };
        }
        if let Some(path) = picked {
            *value = path.to_string_lossy().into_owned();
            changed = true;
        }
    });
    ui.add_space(theme::SPACE_XS);
    changed
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Game Setup", "Point EmblemWeave at your own copy of the game: a .nup dump or a .nsp package as it is, or an extracted folder. Nothing here modifies your files.");

    let mut rescan = false;
    widgets::section(ui, &p, "Game data", |ui| {
        rescan |= path_row(
            ui,
            &p,
            "Game data",
            &mut app.config.paths.game_root,
            r"your .nup dump, .nsp package, or a folder with exefs and romfs",
            Pick::Dump,
        );
        ui.horizontal(|ui| {
            if widgets::button(ui, &p, "Detect again", app.running(JobKind::Detect).is_none()).clicked() {
                rescan = true;
            }
            if app.running(JobKind::Detect).is_some() && widgets::button(ui, &p, "Cancel", true).clicked() {
                app.confirm = Some(crate::Confirm::CancelJob(JobKind::Detect));
            }
            if let Some(job) = app.running(JobKind::Detect) {
                let (copied, total) = job.snapshot(|pr| (pr.copied, pr.total));
                if total == 0 {
                    ui.spinner();
                } else {
                    ui.label(
                        RichText::new(format!("{} of {}", crate::fmt_size(copied), crate::fmt_size(total)))
                            .size(12.5)
                            .color(p.body),
                    );
                }
            }
        });
        // A package is unpacked before anything else can be checked: show how
        // far that has got and how much of it is left.
        if let Some(job) = app.running(JobKind::Detect) {
            let (copied, total, message) = job.snapshot(|pr| (pr.copied, pr.total, pr.message.clone()));
            if total > 0 {
                ui.add_space(theme::SPACE_XS);
                widgets::progress_bar(ui, &p, Some(copied as f32 / total.max(1) as f32));
                ui.label(RichText::new(message).size(12.5).color(p.body));
                ui.add_space(theme::SPACE_XS);
            }
        }
        egui::CollapsingHeader::new(RichText::new("Individual locations").color(p.body)).show(ui, |ui| {
            ui.label(RichText::new("Leave empty to detect from the game folder.").size(12.5).color(p.muted));
            rescan |= path_row(ui, &p, "ExeFS", &mut app.config.paths.exefs, "folder containing main, sdk, main.npdm", Pick::Folder);
            rescan |= path_row(ui, &p, "RomFS", &mut app.config.paths.romfs, "folder containing Fs.bin.zst and the asset folders", Pick::Folder);
            rescan |= path_row(ui, &p, "control.nacp", &mut app.config.paths.nacp, "the title metadata file", Pick::Nacp);
            rescan |= path_row(ui, &p, "Decoded module", &mut app.config.paths.decoded, "folder with text.bin, rodata.bin, data.bin", Pick::Folder);
        });
    });
    if rescan {
        app.start(JobKind::Detect);
    }

    widgets::section(ui, &p, "Validation", |ui| {
        if app.game.checks.is_empty() {
            ui.label(RichText::new("No game folder chosen yet.").color(p.muted));
        }
        for c in &app.game.checks {
            let tone = match c.severity {
                Severity::Ok => Tone::Ok,
                Severity::Info => Tone::Accent,
                Severity::Warn => Tone::Warn,
                Severity::Error => Tone::Bad,
            };
            widgets::status_line(ui, &p, tone, &c.label, &c.detail);
        }
    });

    // Title and executable details.
    if app.game.modules.is_some() || app.game.title.is_some() {
        ui.columns(2, |cols| {
            widgets::section(&mut cols[0], &p, "Title", |ui| {
                if let Some((name, version)) = &app.game.title {
                    widgets::kv(ui, &p, "Name", name);
                    widgets::kv(ui, &p, "Display version", version);
                }
                if let Some(dir) = &app.game.romfs {
                    widgets::kv(ui, &p, "RomFS", &dir.display().to_string());
                }
            });
            widgets::section(&mut cols[1], &p, "Decoded module", |ui| {
                if app.game.segments.is_empty() {
                    ui.label(RichText::new("The main module has not been decoded.").color(p.body));
                }
                for s in &app.game.segments {
                    let tone = if s.ok() { Tone::Ok } else { Tone::Bad };
                    let detail = match (s.actual, s.declared) {
                        (Some(a), Some(d)) if a == d => format!("{} bytes, matches the header", crate::fmt_count(a)),
                        (Some(a), Some(d)) => format!("{} bytes, header declares {}", crate::fmt_count(a), crate::fmt_count(d)),
                        (None, _) => "missing".to_string(),
                        (Some(a), None) => format!("{} bytes, no header to compare", crate::fmt_count(a)),
                    };
                    widgets::status_line(ui, &p, tone, s.name, &detail);
                }
                ui.add_space(theme::SPACE_S);
                let busy = app.running(JobKind::Decode).is_some();
                let label = if app.game.decoded.is_some() { "Decode again" } else { "Decode main module" };
                if widgets::button(ui, &p, label, app.game.exefs.is_some() && !busy)
                    .on_hover_text("Runs the ZBIC bridge, which is built into the launcher and needs Python's zbic package. Output goes to %APPDATA%\\EmblemWeave\\work; your dump is only read.")
                    .clicked()
                {
                    app.start(JobKind::Decode);
                }
            });
        });
    }

    if let Some(exefs) = &app.game.modules {
        widgets::section(ui, &p, "Executable modules", |ui| {
            egui::Grid::new("modules").num_columns(5).spacing([18.0, 6.0]).striped(true).show(ui, |ui| {
                for h in ["File", "Module", "Compression", "Image size", "Status"] {
                    ui.label(RichText::new(h).font(theme::semibold(12.0)).color(p.muted));
                }
                ui.end_row();
                for m in &exefs.modules {
                    ui.label(RichText::new(&m.file_name).font(theme::mono(12.5)).color(p.ink));
                    ui.label(RichText::new(&m.module_name).color(p.body));
                    ui.label(RichText::new(m.compressions.join(" + ")).color(p.body));
                    ui.label(RichText::new(format!("{:.1} MiB", m.image_size() as f64 / 1048576.0)).font(theme::mono(12.5)));
                    let (t, tone) = if m.file_name == "main" {
                        ("game code", Tone::Accent)
                    } else if m.is_zbic() {
                        ("ZBIC", Tone::Warn)
                    } else {
                        ("SDK / library", Tone::Neutral)
                    };
                    widgets::pill(ui, &p, t, tone);
                    ui.end_row();
                }
            });
        });
    }

    widgets::section(ui, &p, "Profile and region", |ui| {
        widgets::setting_row(ui, &p, "Language", "Reported to the game through nn::oe; selects the game's text and voice language.", |ui| {
            let current = LANGUAGES.iter().find(|(c, _)| *c == app.config.system.language).map(|(_, n)| *n).unwrap_or("Custom");
            egui::ComboBox::from_id_salt("language").selected_text(current).width(220.0).show_ui(ui, |ui| {
                for (code, name) in LANGUAGES {
                    ui.selectable_value(&mut app.config.system.language, code.to_string(), format!("{name}  ({code})"));
                }
            });
        });
        widgets::setting_row(ui, &p, "Profile name", "The local user. Its account identifier, and so the save data, derive from it.", |ui| {
            ui.add(egui::TextEdit::singleline(&mut app.config.system.user_name).desired_width(220.0));
        });
    });

    widgets::section(ui, &p, "Save data", |ui| {
        ui.label("Importing, save profiles and the save editor are on the Saves page.");
    });

    widgets::section(ui, &p, "Runtime", |ui| {
        widgets::setting_row(ui, &p, "Instruction budget", "A boot test stops after this many guest instructions.", |ui| {
            let mut millions = (app.config.runtime.max_steps / 1_000_000) as f32;
            if widgets::Slider::new(&mut millions, 1.0..=2000.0).suffix(" M").logarithmic(true).show(ui, &p).changed() {
                app.config.runtime.max_steps = millions.round() as u64 * 1_000_000;
            }
        });
        widgets::setting_row(ui, &p, "Guest heap", "Memory reserved for the game's heap. Committed by the OS only as used.", |ui| {
            let mut heap = app.config.runtime.heap_mib as f32;
            if widgets::Slider::new(&mut heap, 256.0..=8192.0).suffix(" MiB").logarithmic(true).show(ui, &p).changed() {
                app.config.runtime.heap_mib = heap.round() as u64;
            }
        });
    });
}

/// Save data: where EmblemWeave keeps it, saves found from emulators, and
/// import and export.
pub(crate) fn saves_section(app: &mut App, ui: &mut egui::Ui, p: &crate::theme::Palette) {
    let Some(save_dir) = crate::config::Config::save_dir() else { return };
    let title = app
        .game
        .title_id
        .or_else(|| app.game.nacp.as_deref().and_then(crate::saves::nacp_save_owner))
        .unwrap_or(crate::saves::FALLBACK_TITLE_ID);
    let stamp = || {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string()
    };
    let mut message: Option<(String, Tone)> = None;
    widgets::section(ui, p, "Save data", |ui| {
        let (files, _) = crate::saves::summary(&crate::saves::account_dir(&save_dir));
        widgets::kv(ui, p, "Save folder", &save_dir.display().to_string());
        widgets::kv(ui, p, "Files", &if files == 0 { "none yet".to_string() } else { files.to_string() });
        widgets::kv(ui, p, "Title", &format!("{title:016X}"));
        ui.add_space(6.0);
        let found = crate::saves::find_here(title);
        if found.is_empty() {
            ui.label("No emulator saves for this game were found on this PC.");
        }
        for f in &found {
            ui.horizontal(|ui| {
                let when = f
                    .modified
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| format!(", last written {} days ago", (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|n| n.as_secs()).unwrap_or(0).saturating_sub(d.as_secs())) / 86_400))
                    .unwrap_or_default();
                ui.label(format!("{}: {} files{when}", f.source, f.files)).on_hover_text(f.path.display().to_string());
                if widgets::button(ui, p, "Import", true).clicked() {
                    message = Some(import_message(crate::saves::import(&f.path, &save_dir, title, &stamp())));
                }
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if widgets::button(ui, p, "Import from folder…", true).clicked() {
                if let Some(dir) = rfd::FileDialog::new().set_title("Choose the game's save folder (the one named by its title id)").pick_folder() {
                    message = Some(import_message(crate::saves::import(&dir, &save_dir, title, &stamp())));
                }
            }
            if widgets::button(ui, p, "Export…", files > 0).clicked() {
                if let Some(dir) = rfd::FileDialog::new().set_title("Choose a folder to export the save into").pick_folder() {
                    message = Some(match crate::saves::export(&save_dir, &dir, title) {
                        Ok((to, n)) => (format!("Exported {n} save files to {}", to.display()), Tone::Ok),
                        Err(e) => (format!("Export failed: {e}"), Tone::Bad),
                    });
                }
            }
            if widgets::button(ui, p, "Open folder", true).clicked() {
                let _ = std::fs::create_dir_all(&save_dir);
                let _ = std::process::Command::new("explorer").arg(&save_dir).spawn();
            }
        });
        ui.label("Importing moves the current save to the backups folder first; the source is only read.");
    });
    if let Some((text, tone)) = message {
        app.notify(text, tone);
    }
}

pub(crate) fn import_message(r: Result<crate::saves::Imported, String>) -> (String, Tone) {
    match r {
        Ok(i) => match i.backup {
            Some(b) => (format!("Imported {} save files; the previous save is in {}", i.files, b.display()), Tone::Ok),
            None => (format!("Imported {} save files", i.files), Tone::Ok),
        },
        Err(e) => (format!("Import failed: {e}"), Tone::Bad),
    }
}
