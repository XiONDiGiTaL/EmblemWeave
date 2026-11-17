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
    /// The game data: an `.nx` archive, a `.nup` dump, an `.nsp` package, a
    /// plaintext `.xci` game card, or a folder.
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
                .add_filter("Game package (.nx, .nup, .nsp, .xci)", &["nx", "nup", "nsp", "xci"])
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
    widgets::page_header(ui, &p, "Game Setup", "Point EmblemWeave at your own copy of the game: a .nx archive, a .nup dump, a .nsp package or a plaintext .xci game card as it is, or an extracted folder. Nothing here modifies your files.");

    let mut rescan = false;
    widgets::section(ui, &p, "Game data", |ui| {
        rescan |= path_row(
            ui,
            &p,
            "Game data",
            &mut app.config.paths.game_root,
            r"your .nx archive, .nup dump, .nsp package, .xci game card, or a folder with exefs and romfs",
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

    // One click for the failure that otherwise shows up two seconds into play: a package that packs some of its files with zstd, or a game
    // folder built by an older release that points at such a package without the overlay of files it cannot serve in place (issue #24).
    if app.game.romfs.is_some() {
        widgets::section(ui, &p, "Game folder check", |ui| {
            ui.label(
                RichText::new(
                    "Reads the package and writes out the files it cannot serve in place, then points the game at them. Nothing is rebuilt \
                     and the package itself is only read. Do this once after updating, or if Play says the game was built against a packed \
                     package.",
                )
                .size(13.0)
                .color(p.body),
            );
            ui.add_space(theme::SPACE_S);
            let busy = app.running(JobKind::Repair).is_some();
            if widgets::button(ui, &p, "Check my game folder", !busy)
                .on_hover_text("Writes the packed files into the engine's work folder and records them in fefw.ini. Nothing is compiled.")
                .clicked()
            {
                app.start(JobKind::Repair);
            }
        });
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

    widgets::section(ui, &p, "Engine folder", |ui| {
        let current = crate::engine::root();
        widgets::kv(ui, &p, "Engine", &current.as_ref().map(|r| r.display().to_string()).unwrap_or_else(|| "not found".into()));
        ui.label(RichText::new("The engine folder holds the built game and its build cache. Keeping one anywhere on disk is what spares a full rebuild when a new launcher version arrives: point the new copy here instead of unzipping a whole engine beside it.").size(12.5).color(p.muted));
        ui.horizontal(|ui| {
            if widgets::button(ui, &p, "Use another engine folder…", true)
                .on_hover_text("Choose the checkout (…\\engine\\fefw-recomp) or a folder that holds it, such as an older EmblemWeave copy")
                .clicked()
            {
                if let Some(dir) = rfd::FileDialog::new().set_title("Choose an engine folder (engine\\fefw-recomp)").pick_folder() {
                    match crate::engine::remember_root(&dir) {
                        Ok(root) => app.notify(format!("Using the engine in {}", root.display()), Tone::Ok),
                        Err(e) => app.notify(e, Tone::Bad),
                    }
                }
            }
            if let Some(root) = &current {
                if widgets::button(ui, &p, "Open folder", true).clicked() {
                    let _ = std::process::Command::new("explorer").arg(root).spawn();
                }
            }
        });
    });

    widgets::section_drawer(
        ui,
        &p,
        "updating",
        "Updating EmblemWeave",
        "what to keep, and why nothing is rebuilt",
        false,
        |ui| {
            // Issue #56 asked for the steps in one place: which folders to keep, and why a new launcher does not need a new build.
            let steps = [
                "1. Unzip the new release over this folder. It replaces the launcher; data\\ and engine\\ stay as they are, so \
                 the game is not rebuilt.",
                "2. Unpacking into a new folder instead? Copy data\\ and engine\\ across, or keep the old folder and press \
                 Use another engine folder… above. Nothing is moved or deleted.",
                "3. A copy with no build finds one already on this PC by itself: Overview opens with A build is already on \
                 this PC and a Use this build button. Not now declines it until you say otherwise.",
            ];
            for step in steps {
                ui.label(RichText::new(step).size(12.5).color(p.body));
                ui.add_space(3.0);
            }
            ui.add_space(theme::SPACE_XS);
            if let Some(dir) = crate::config::Config::dir() {
                widgets::kv(ui, &p, "This copy keeps settings and saves in", &dir.display().to_string());
            }
            if let Some(root) = crate::engine::root() {
                widgets::kv(ui, &p, "The engine and its build", &root.display().to_string());
            }
        },
    );

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

        // Scanning all yuzu-family profiles in APPDATA and LOCALAPPDATA every
        // frame is wasted work. Keep the list for five seconds, and let the
        // player force a rescan after copying a save into an emulator.
        let now = ui.input(|i| i.time);
        if app.saves_ui.sources_title != Some(title) || now - app.saves_ui.sources_at > 5.0 {
            app.saves_ui.sources = crate::saves::find_here(title);
            app.saves_ui.sources_title = Some(title);
            app.saves_ui.sources_at = now;
        }
        let found = app.saves_ui.sources.clone();

        ui.horizontal(|ui| {
            ui.label(RichText::new("Emulator saves").font(theme::semibold(13.0)).color(p.ink));
            if widgets::button(ui, p, "Scan again", true).on_hover_text("Looks in yuzu, suyu, sudachi, citron, Eden, Lichii, Fritchii, torzu and Ryujinx-yuzu").clicked() {
                app.saves_ui.sources = crate::saves::find_here(title);
                app.saves_ui.sources_title = Some(title);
                app.saves_ui.sources_at = now;
            }
            if !found.is_empty() && widgets::primary_button(ui, p, "Import newest", true).on_hover_text("Imports the most recently written emulator save above").clicked() {
                message = Some(import_notice(crate::saves::import(&found[0].path, &save_dir, title, &stamp())));
            }
        });
        if found.is_empty() {
            ui.label(RichText::new("No emulator saves for this game were found. Lichii, Fritchii and Eden use the yuzu NAND layout; a portable install can be picked with Import from folder.").size(12.5).color(p.muted));
        }
        for f in &found {
            ui.horizontal(|ui| {
                let label = match f.source.as_str() {
                    "yuzu" => "yuzu",
                    "suyu" => "suyu",
                    "sudachi" => "sudachi",
                    "citron" => "citron",
                    "eden" => "Eden",
                    "lichii" => "Lichii",
                    "fritchii" => "Fritchii",
                    "torzu" => "torzu",
                    other => other,
                };
                let when = f
                    .modified
                    .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
                    .map(|d| match d.as_secs() {
                        0..=59 => "just now".to_string(),
                        60..=3599 => format!("{} min ago", d.as_secs() / 60),
                        3600..=86_399 => format!("{} h ago", d.as_secs() / 3600),
                        secs => format!("{} days ago", secs / 86_400),
                    })
                    .unwrap_or_default();
                ui.label(format!("{}: {} files, {}", label, f.files, when)).on_hover_text(f.path.display().to_string());
                if widgets::button(ui, p, "Import", true).clicked() {
                    message = Some(import_notice(crate::saves::import(&f.path, &save_dir, title, &stamp())));
                }
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if widgets::button(ui, p, "Import from folder…", true).clicked() {
                if let Some(dir) = rfd::FileDialog::new().set_title("Choose the game's save folder (the one named by its title id)").pick_folder() {
                    message = Some(import_notice(crate::saves::import(&dir, &save_dir, title, &stamp())));
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

/// The page's tone for an import result. The wording itself lives in [`crate::saves::import_message`], so the command line and
/// the window say the same thing without the CLI importing a page module.
fn import_notice(r: Result<crate::saves::Imported, String>) -> (String, Tone) {
    let (text, ok) = crate::saves::import_message(r);
    (text, if ok { Tone::Ok } else { Tone::Bad })
}
