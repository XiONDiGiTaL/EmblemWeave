//! Performance Lab: the system profile, a measured engine-session benchmark,
//! saved comparisons, and recommendations that require approval.

use crate::labb::{self, Level, Mode, Recommendation, Run, Session, Summary};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui;
use std::collections::HashSet;

fn yes(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn level_tone(level: Level) -> Tone {
    match level {
        Level::Normal => Tone::Ok,
        Level::Elevated => Tone::Neutral,
        Level::High => Tone::Warn,
        Level::Critical => Tone::Bad,
    }
}

fn unavailable(ui: &mut egui::Ui, p: &theme::Palette, key: &str, why: &str) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(170.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(170.0);
            ui.label(crate::i18n::text(key).size(13.0).color(p.muted));
        });
        ui.label(crate::i18n::text(format!("Unavailable · {why}")).size(12.0).color(p.warn));
    });
}

fn bytes_or_unavailable(ui: &mut egui::Ui, p: &theme::Palette, key: &str, value: Option<u64>, why: &str) {
    match value {
        Some(mb) => widgets::kv(ui, p, key, &crate::memory::gb(mb)),
        None => unavailable(ui, p, key, why),
    }
}

fn tab_button(app: &mut App, ui: &mut egui::Ui, tab: labb::Tab, label: &str) {
    if ui
        .selectable_label(app.lab.tab == tab, crate::i18n::text(label).font(theme::semibold(14.0)))
        .clicked()
    {
        app.lab.tab = tab;
    }
}

fn profile_tab(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    let Some(profile) = app.lab.profile.clone() else {
        widgets::section(ui, p, "System Profile", |ui| {
            ui.label(crate::i18n::text("Reading the hardware and Vulkan devices…").color(p.body));
        });
        return;
    };
    let cpu = &profile.cpu;
    widgets::section(ui, p, "Processor", |ui| {
        widgets::kv(ui, p, "Model", &cpu.brand);
        widgets::kv(ui, p, "Architecture", &cpu.architecture);
        match cpu.physical {
            Some(n) => widgets::kv(ui, p, "Physical cores", &n.to_string()),
            None => unavailable(ui, p, "Physical cores", "the Windows processor-topology call was unavailable"),
        }
        widgets::kv(ui, p, "Logical threads", &cpu.logical.to_string());
        widgets::kv(
            ui,
            p,
            "Instruction sets",
            &format!(
                "SSE4.2 {} · POPCNT {} · CMPXCHG16B {} · AVX {} · FMA3 {}",
                yes(cpu.sse42),
                yes(cpu.popcnt),
                yes(cpu.cx16),
                yes(cpu.avx),
                yes(cpu.fma)
            ),
        );
        widgets::kv(ui, p, "Game build", &cpu.build);
        unavailable(ui, p, "Clock speed", "Windows does not expose one reliable current-clock value through the APIs used here");
        ui.label(
            crate::i18n::text("Detected from CPUID and Windows topology. The game build is the engine's own selection, not an estimate.")
                .size(11.5)
                .color(p.muted),
        );
    });

    widgets::section(ui, p, "System memory", |ui| {
        bytes_or_unavailable(ui, p, "Installed physical", profile.memory.installed_mb, "the Windows memory query did not answer");
        bytes_or_unavailable(ui, p, "Total usable", profile.memory.total_mb, "the Windows memory query did not answer");
        bytes_or_unavailable(ui, p, "Available now", profile.memory.available_mb, "the Windows memory query did not answer");
        if let (Some(installed), Some(available)) = (profile.memory.installed_mb, profile.memory.available_mb) {
            let used = installed.saturating_sub(available);
            widgets::kv(ui, p, "In use (physical)", &format!("{} ({:.0}%)", crate::memory::gb(used), used as f64 * 100.0 / installed.max(1) as f64));
        }
        match (profile.memory.commit_free_mb, profile.memory.commit_limit_mb) {
            (Some(free), Some(limit)) => {
                widgets::kv(ui, p, "Commit", &format!("{} free of {}", crate::memory::gb(free), crate::memory::gb(limit)));
            }
            _ => unavailable(ui, p, "Commit", "the Windows commit counters were unavailable"),
        }
        if let Some((level, note)) = &profile.pressure.host {
            widgets::status_line(ui, p, level_tone(*level), &format!("Host pressure: {}", level.label()), note);
        }
        ui.label(
            crate::i18n::text("Host memory is tracked separately from GPU memory. The guest heap reservation and the runtime's host guard are not counted as committed RAM here.")
                .size(11.5)
                .color(p.muted),
        );
    });

    widgets::section(ui, p, "Graphics adapters", |ui| {
        if profile.gpus.is_empty() {
            widgets::status_line(
                ui,
                p,
                Tone::Warn,
                "No Vulkan 1.3 adapter reported",
                "The scan may still be running, Vulkan may be unavailable, or every adapter is older than 1.3. The rest of the profile does not depend on it.",
            );
        }
        for (i, gpu) in profile.gpus.iter().enumerate() {
            if i > 0 {
                ui.separator();
            }
            let active = profile.active_gpu == Some(i);
            ui.horizontal_wrapped(|ui| {
                ui.label(crate::i18n::text(&gpu.device.name).font(theme::semibold(14.0)).color(p.ink));
                widgets::pill(ui, p, gpu.device.kind, if active { Tone::Accent } else { Tone::Neutral });
                if active {
                    widgets::pill(ui, p, "engine selection", Tone::Ok);
                }
                if gpu.device.av1_decode {
                    widgets::pill(ui, p, "AV1 decode", Tone::Ok);
                }
            });
            widgets::kv(ui, p, "Vendor / device", &format!("{} · 0x{:04X}:0x{:04X}", gpu.vendor, gpu.vendor_id, gpu.device_id));
            widgets::kv(ui, p, "Vulkan API", &gpu.api_version);
            widgets::kv(ui, p, "Driver", &gpu.driver);
            widgets::kv(ui, p, "Dedicated memory", &crate::memory::gb(gpu.device.vram_mb));
            widgets::kv(ui, p, "Extensions", &format!("{} advertised; VK_EXT_memory_budget {}", gpu.extensions.len(), yes(gpu.has_extension("VK_EXT_memory_budget"))));
            for heap in &gpu.heaps {
                let kind = if heap.device_local { "device-local" } else { "shared/host-visible" };
                match (heap.budget_mb, heap.usage_mb) {
                    (Some(budget), Some(used)) => widgets::kv(
                        ui,
                        p,
                        &format!("Heap · {kind}"),
                        &format!(
                            "{} · {} used of {} budget ({}%)",
                            crate::memory::gb(heap.size_mb),
                            crate::memory::gb(used),
                            crate::memory::gb(budget),
                            used.saturating_mul(100) / budget.max(1)
                        ),
                    ),
                    _ => unavailable(
                        ui,
                        p,
                        &format!("Heap · {kind}"),
                        &format!("{} reported; budget unavailable (driver did not expose a usable VK_EXT_memory_budget figure)", crate::memory::gb(heap.size_mb)),
                    ),
                }
            }
            if let Some((level, note)) = &profile.pressure.gpu {
                widgets::status_line(ui, p, level_tone(*level), &format!("GPU pressure: {}", level.label()), note);
            }
        }
        if let Some((_, _, source)) = &profile.gpu_budget {
            ui.label(crate::i18n::text(format!("Budget source: {source}. The lab never adds Vulkan and DXGI figures together.")).size(11.5).color(p.muted));
        }
    });

    widgets::section(ui, p, "Display, OS and active configuration", |ui| {
        match profile.display.resolution {
            Some((w, h)) => widgets::kv(ui, p, "Primary display", &format!("{w} × {h}")),
            None => unavailable(ui, p, "Primary display", "the Windows display query did not answer"),
        }
        match profile.display.refresh_hz {
            Some(hz) => widgets::kv(ui, p, "Refresh rate", &format!("{hz} Hz")),
            None => unavailable(ui, p, "Refresh rate", "GetDeviceCaps did not report a refresh rate"),
        }
        widgets::kv(ui, p, "Operating system", &labb::os_label());
        widgets::kv(ui, p, "Launcher", &format!("{} ({})", env!("CARGO_PKG_VERSION"), env!("EW_GIT_REV")));
        widgets::kv(ui, p, "Engine selection", profile.active_name().unwrap_or("not reported"));
        widgets::kv(ui, p, "Graphics", &labb::config_summary(&app.config));
        widgets::kv(ui, p, "GPU field", &app.config.graphics.gpu);
        widgets::kv(ui, p, "Performance mode", &format!("{} · controller {}", app.config.performance.mode, yes(app.config.performance.controller)));
        match &profile.storage {
            Some(s) => widgets::kv(
                ui,
                p,
                &format!("Storage ({})", s.volume),
                &format!("{} free of {}", crate::memory::gb(s.free_mb), crate::memory::gb(s.total_mb)),
            ),
            None => unavailable(ui, p, "Storage", "the Windows volume-capacity query did not answer for the game folder"),
        }
    });

    widgets::section(ui, p, "Pressure thresholds", |ui| {
        ui.label(
            crate::i18n::text("These affect the read-only pressure indicator and recommendation evidence. Defaults are conservative starting points; they are not experimentally optimal for every GPU.")
                .size(11.5)
                .color(p.muted),
        );
        let mut changed = false;
        let mut gpu = [app.config.lab.gpu_elevated, app.config.lab.gpu_high, app.config.lab.gpu_critical];
        ui.horizontal_wrapped(|ui| {
            changed |= ui.add(egui::Slider::new(&mut gpu[0], 40..=90).suffix("%")).changed();
            changed |= ui.add(egui::Slider::new(&mut gpu[1], 60..=95).suffix("%")).changed();
            changed |= ui.add(egui::Slider::new(&mut gpu[2], 80..=100).suffix("%")).changed();
        });
        app.config.lab.gpu_elevated = gpu[0];
        app.config.lab.gpu_high = gpu[1];
        app.config.lab.gpu_critical = gpu[2];
        ui.label(crate::i18n::text(format!("GPU budget in use: elevated {}% · high {}% · critical {}%", app.config.lab.gpu_elevated, app.config.lab.gpu_high, app.config.lab.gpu_critical)).size(12.0).color(p.body));
        ui.add_space(4.0);
        let mut host = [app.config.lab.host_elevated, app.config.lab.host_high, app.config.lab.host_critical];
        ui.horizontal_wrapped(|ui| {
            changed |= ui.add(egui::Slider::new(&mut host[0], 5..=40).suffix("%")).changed();
            changed |= ui.add(egui::Slider::new(&mut host[1], 2..=25).suffix("%")).changed();
            changed |= ui.add(egui::Slider::new(&mut host[2], 1..=10).suffix("%")).changed();
        });
        app.config.lab.host_elevated = host[0];
        app.config.lab.host_high = host[1];
        app.config.lab.host_critical = host[2];
        ui.label(crate::i18n::text(format!("Physical memory available: elevated {}% · high {}% · critical {}%", app.config.lab.host_elevated, app.config.lab.host_high, app.config.lab.host_critical)).size(12.0).color(p.body));
        if changed {
            app.config.lab.normalize();
            app.lab.refresh_profile(&app.config, true);
        }
    });

    ui.horizontal(|ui| {
        if widgets::button(ui, p, "Refresh profile", true).clicked() {
            app.lab.refresh_profile(&app.config, true);
        }
        ui.label(crate::i18n::text("Detected values refresh every five seconds while this page is open.").size(11.5).color(p.muted));
    });
}

fn start_run(app: &mut App, mode: Mode) {
    if app.lab.session.is_some() {
        return;
    }
    let stamp = labb::unix_s();
    let run_dir = labb::lab_dir().map(|d| d.join("runs").join(format!("{stamp}-{}", mode.label().to_ascii_lowercase())));
    if let Some(dir) = &run_dir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            app.lab.notify(format!("Could not create the run folder: {e}"), false);
            return;
        }
    }
    let already_running = crate::platform::game_running();
    let soak_path = (!already_running).then(|| run_dir.as_ref().map(|d| d.join("soak.jsonl"))).flatten();
    if !already_running {
        if let Some(path) = &soak_path {
            std::env::set_var("NXRT_DIAG_SOAK_CSV", path);
            std::env::set_var("NXRT_DIAG_SOAK_INTERVAL_S", "1");
        }
        let played = crate::engine::play(&app.config, false);
        std::env::remove_var("NXRT_DIAG_SOAK_CSV");
        std::env::remove_var("NXRT_DIAG_SOAK_INTERVAL_S");
        if let Err(e) = played {
            app.lab.notify(e, false);
            return;
        }
    }
    app.lab.refresh_profile(&app.config, true);
    let metadata = app.lab.profile.as_ref().map(|p| labb::RunMetadata::collect(&app.config, p));
    let recorder = labb::Recorder::new(mode, labb::unix_ms());
    let mut session = Session::new(recorder, soak_path, !already_running);
    session.metadata = metadata;
    app.lab.summary = None;
    app.lab.session = Some(session);
    app.notify(
        if already_running {
            "Benchmark attached to the running game. Play a representative scene; the engine snapshot starts the clock."
        } else {
            "Benchmark started. The game is starting; play a representative scene. The clock starts at the first engine snapshot."
        },
        Tone::Ok,
    );
}

fn benchmark_tab(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    widgets::section(ui, p, "What this measures", |ui| {
        ui.label(
            crate::i18n::text("A run records the real game's one-second engine telemetry, not a synthetic test. Quick targets 2.5 minutes of measured frame delivery; Extended targets 15 minutes of sustained play. Both are observation windows, not stress certification.")
                .size(12.5)
                .color(p.body),
        );
        ui.label(
            crate::i18n::text("When the launcher starts the game, it also enables the runtime's soak sampler: process working set, shader and pipeline counters, asset reads, texture preparation and movie delivery. If you attach to a game that is already running, those extra counters are unavailable.")
                .size(11.5)
                .color(p.muted),
        );
        ui.add_space(theme::SPACE_S);
        widgets::status_line(ui, p, Tone::Neutral, "No synthetic frame rate", "This never presents engine measurements as guaranteed gameplay FPS and never reduces the result to one score.");
    });

    let now = labb::unix_ms();
    let session_running = app.lab.session.is_some();
    let ready = crate::engine::ready();
    let mut begin: Option<Mode> = None;
    widgets::section(ui, p, "Run", |ui| {
        ui.horizontal_wrapped(|ui| {
            if widgets::primary_button(ui, p, "Quick benchmark · 2.5 min", ready && !session_running)
                .on_disabled_hover_text(crate::i18n::tr("Build the game first; a quick run measures 150 seconds of engine telemetry"))
                .clicked()
            {
                begin = Some(Mode::Quick);
            }
            if widgets::button(ui, p, "Extended benchmark · 15 min", ready && !session_running)
                .on_hover_text("A longer observation window; you will be asked to confirm before the game starts")
                .clicked()
            {
                app.lab.confirm_extended = true;
            }
            if session_running && widgets::button(ui, p, "Cancel run", true).clicked() {
                app.lab.confirm_cancel = true;
            }
        });
        if !ready {
            widgets::status_line(ui, p, Tone::Warn, "Game not built", "The Benchmark measures the real game; build it on Game Setup first.");
        }
        if session_running {
            if let Some(s) = &app.lab.session {
                ui.add_space(theme::SPACE_S);
                widgets::progress_bar(ui, p, Some(s.recorder.progress(now)));
                ui.horizontal_wrapped(|ui| {
                    widgets::pill(ui, p, &format!("{} · {} s remaining", s.recorder.mode.label(), s.recorder.remaining_s(now)), Tone::Accent);
                    widgets::pill(ui, p, &format!("{} samples", s.recorder.samples.len()), Tone::Neutral);
                    if s.recorder.active_since_ms.is_none() {
                        widgets::pill(ui, p, "waiting for first engine snapshot", Tone::Warn);
                    }
                    if s.launched_game {
                        widgets::pill(ui, p, "launcher started the game", Tone::Neutral);
                    } else {
                        widgets::pill(ui, p, "attached to running game", Tone::Neutral);
                    }
                });
                if let Some(status) = &s.last_status {
                    widgets::status_line(
                        ui,
                        p,
                        Tone::Ok,
                        &format!("{:.1} fps · {}", status.avg_fps, status.bottleneck),
                        &format!(
                            "1% low {:.1} · P99 {:.1} ms · VRAM {}% · shaders waiting {}",
                            status.low1_fps, status.p99_ms, status.vram_percent, status.shader_backlog
                        ),
                    );
                }
                ui.label(
                    crate::i18n::text("Cancelling stops recording and discards the partial run. It does not change settings or kill the game; close the game normally when you are done.")
                        .size(11.5)
                        .color(p.muted),
                );
            }
        }
    });

    if let Some(mode) = begin {
        start_run(app, mode);
    }

    if let Some(summary) = app.lab.summary.clone() {
        results(ui, p, app, &summary);
    } else if !session_running {
        widgets::section(ui, p, "Results", |ui| {
            ui.label(crate::i18n::text("No run in this session yet. Start a benchmark, or review saved runs below.").size(12.5).color(p.muted));
        });
    }

    history(ui, p, app);
}

fn result_line(ui: &mut egui::Ui, p: &theme::Palette, key: &str, value: Option<String>, why: &str) {
    match value {
        Some(v) => widgets::kv(ui, p, key, &v),
        None => unavailable(ui, p, key, why),
    }
}

fn results(ui: &mut egui::Ui, p: &theme::Palette, app: &mut App, s: &Summary) {
    widgets::section(ui, p, "Engine result", |ui| {
        if s.samples == 0 {
            // A completed observation with no engine snapshots has no frame
            // rate at all. Showing 0 fps would look like a measurement.
            widgets::status_line(
                ui,
                p,
                Tone::Warn,
                "No frame telemetry",
                "The engine did not publish performance snapshots for this run. No frame rate or frame-time figure is reported.",
            );
        } else {
            ui.horizontal_wrapped(|ui| {
                for (label, value) in [
                    ("Average", format!("{:.1} fps", s.avg_fps)),
                    ("1% low", format!("{:.1} fps", s.low1_fps)),
                    ("0.1% low", format!("{:.1} fps", s.low01_fps)),
                    ("P95", format!("{:.1} ms", s.p95_ms)),
                    ("P99", format!("{:.1} ms", s.p99_ms)),
                ] {
                    widgets::pill(ui, p, &format!("{label}  {value}"), Tone::Neutral);
                }
            });
            ui.add_space(theme::SPACE_S);
            result_line(ui, p, "Observation", Some(format!("{} samples over {:.1} s", s.samples, s.observed_s)), "");
            result_line(ui, p, "CPU frame time", Some(format!("{:.2} ms", s.cpu_ms)), "");
            result_line(ui, p, "GPU frame time", Some(format!("{:.2} ms", s.gpu_ms)), "");
            result_line(ui, p, "Upload / replay", Some(format!("{:.2} / {:.2} ms", s.upload_ms, s.replay_ms)), "");
            result_line(ui, p, "GPU budget peak", s.gpu_peak_pct().map(|v| format!("{v}% of {}", crate::memory::gb(s.gpu_budget_mb.unwrap_or(0)))), "no budget and usage pair was available");
            result_line(ui, p, "GPU memory seen by the engine", if s.peak_vram_percent >= 0 { Some(format!("{}%", s.peak_vram_percent)) } else { None }, "the engine did not report a VRAM percentage");
            result_line(ui, p, "Host available minimum", s.host_available_min_mb.map(crate::memory::gb), "host samples were unavailable");
            result_line(ui, p, "Commit free minimum", s.commit_free_min_mb.map(crate::memory::gb), "commit samples were unavailable");
            result_line(ui, p, "Stutter / spike count", Some(s.spikes.to_string()), "");
            result_line(ui, p, "Shader backlog peak", Some(s.max_shader_backlog.to_string()), "");
        }
    });

    widgets::section(ui, p, "Engine subsystem evidence", |ui| {
        let soak = &s.soak;
        result_line(ui, p, "Process working set", match (soak.process_ws_start_mb, soak.process_ws_end_mb) {
            (Some(a), Some(b)) => Some(format!("{} → {} (peak {})", crate::memory::gb(a), crate::memory::gb(b), soak.process_ws_peak_mb.map(crate::memory::gb).unwrap_or_else(|| "?".into()))),
            _ => None,
        }, "the soak sampler did not run for this session");
        result_line(ui, p, "Shader translations", soak.shader_translations_per_s.map(|v| format!("{v:.1}/s")), "shader translation counters were unavailable");
        result_line(ui, p, "Shader cache", soak.shader_cache_hit_pct.map(|v| format!("{v:.1}% hits")), "cache hit/miss counters were unavailable");
        result_line(ui, p, "Pipelines created", soak.pipelines_created.map(|v| v.to_string()), "pipeline counters were unavailable");
        result_line(ui, p, "Pool import failures", soak.pool_import_failures.map(|v| v.to_string()), "pool import counters were unavailable");
        result_line(ui, p, "Pool copy fallbacks", soak.pool_copy_fallbacks.map(|v| v.to_string()), "pool copy counters were unavailable");
        result_line(ui, p, "Live Vulkan resources at end", soak.vulkan_images_end.zip(soak.vulkan_buffers_end).map(|(i, b)| format!("{i} images · {b} buffers")), "live Vulkan object counters were unavailable");
        result_line(ui, p, "Asset reads", soak.asset_read_mb_s.map(|v| format!("{v:.1} MiB/s")), "the engine's FS counters were unavailable");
        result_line(ui, p, "Texture preparation", soak.texture_prep_mb_s.map(|v| format!("{v:.1} MiB/s")), "texture-preparation gauges were unavailable");
        result_line(ui, p, "Video delivery", soak.video_fps.map(|v| format!("{v:.1} frames/s")), "no movie was decoded during the run or the counter was absent");
        result_line(ui, p, "GPU work (timestamps)", soak.gpu_work_ms_per_frame.map(|v| format!("{v:.2} ms/frame")), "GPU timestamp counters were unavailable");
        if !soak.source.is_empty() {
            ui.label(crate::i18n::text(&soak.source).size(11.5).color(p.muted));
        }
    });

    if !s.warnings.is_empty() {
        widgets::section(ui, p, "Warnings", |ui| {
            for w in &s.warnings {
                widgets::status_line(ui, p, Tone::Warn, "Measured", w);
            }
        });
    }
    if !s.unavailable.is_empty() {
        widgets::section(ui, p, "Unavailable or limited", |ui| {
            for u in &s.unavailable {
                ui.label(crate::i18n::text(format!("• {u}")).size(12.0).color(p.muted));
            }
        });
    }
    ui.horizontal(|ui| {
        if widgets::button(ui, p, "Export GitHub report…", true).clicked() {
            if let Some(run) = app.lab.history.last().cloned() {
                export_report(&run, app);
            } else {
                app.lab.notify("Save a completed run before exporting a report", false);
            }
        }
    });
}

fn when(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| format!("{secs}"))
}

fn history(ui: &mut egui::Ui, p: &theme::Palette, app: &mut App) {
    widgets::section(ui, p, "Saved runs", |ui| {
        if app.lab.history.is_empty() {
            ui.label(crate::i18n::text("No saved runs yet. A completed quick or extended run is stored locally, without paths or account names.").size(12.5).color(p.muted));
            return;
        }
        let mut export: Option<Run> = None;
        let mut rows: Vec<&Run> = app.lab.history.iter().rev().take(6).collect();
        rows.reverse();
        egui::Grid::new("lab-history").num_columns(7).spacing([12.0, 6.0]).show(ui, |ui| {
            for h in ["Run", "Mode", "Measured", "Average", "1% low", "GPU peak", "Report"] {
                ui.label(crate::i18n::text(h).font(theme::semibold(11.5)).color(p.muted));
            }
            ui.end_row();
            for run in rows {
                ui.label(crate::i18n::text(when(run.created_unix_s)).font(theme::mono(11.5)).color(p.body));
                ui.label(crate::i18n::text(run.mode.label()).size(12.0).color(p.body));
                ui.label(crate::i18n::text(format!("{:.0} s", run.duration_s)).size(12.0).color(p.body));
                let average = if run.summary.samples > 0 { format!("{:.1} fps", run.summary.avg_fps) } else { "no telemetry".into() };
                let low = if run.summary.samples > 0 { format!("{:.1} fps", run.summary.low1_fps) } else { "—".into() };
                ui.label(crate::i18n::text(average).font(theme::mono(11.5)).color(p.body));
                ui.label(crate::i18n::text(low).font(theme::mono(11.5)).color(p.body));
                ui.label(crate::i18n::text(run.summary.gpu_peak_pct().map(|v| format!("{v}%")).unwrap_or_else(|| "—".into())).font(theme::mono(11.5)).color(p.body));
                if widgets::button(ui, p, "Export", true).clicked() {
                    export = Some(run.clone());
                }
                ui.end_row();
            }
        });
        if app.lab.history.len() >= 2
            && app.lab.history[app.lab.history.len() - 2].summary.samples > 0
            && app.lab.history[app.lab.history.len() - 1].summary.samples > 0
        {
            let a = &app.lab.history[app.lab.history.len() - 2];
            let b = &app.lab.history[app.lab.history.len() - 1];
            let d_avg = b.summary.avg_fps - a.summary.avg_fps;
            let d_low = b.summary.low1_fps - a.summary.low1_fps;
            ui.add_space(theme::SPACE_S);
            ui.label(
                crate::i18n::text(format!(
                    "Latest vs previous: average {:+.1} fps, 1% low {:+.1} fps. Compare like configurations and scenes; a different driver, engine build or scene is recorded in each run's metadata."
                , d_avg, d_low))
                .size(11.5)
                .color(if d_avg >= 0.0 && d_low >= 0.0 { p.muted } else { p.warn }),
            );
        }
        if let Some(run) = export {
            export_report(&run, app);
        }
    });
}

fn export_report(run: &Run, app: &mut App) {
    let Some(path) = rfd::FileDialog::new()
        .set_file_name(format!("emblemweave-performance-{}.md", run.id))
        .save_file()
    else {
        return;
    };
    match std::fs::write(&path, labb::report_markdown(run)) {
        Ok(()) => app.lab.notify(format!("Report written to {}", path.display()), true),
        Err(e) => app.lab.notify(format!("Could not write the report: {e}"), false),
    }
}

fn recommend_tab(app: &mut App, ui: &mut egui::Ui, p: &theme::Palette) {
    widgets::section(ui, p, "How these are chosen", |ui| {
        ui.label(
            crate::i18n::text("Recommendations use the detected CPU, GPU class, Vulkan heap size, installed memory, and any completed run's measured frame-time, VRAM, shader and host-memory evidence. They are proposals: nothing is written until you select and approve them.")
                .size(12.5)
                .color(p.body),
        );
        ui.label(
            crate::i18n::text("Every listed setting maps to a key the engine already reads. No BIOS, driver, overclocking, power-plan or kernel setting is touched. Saved graphics and performance values are backed up before an approved change.")
                .size(11.5)
                .color(p.muted),
        );
    });

    let Some(profile) = app.lab.profile.clone() else {
        ui.label(crate::i18n::text("Reading hardware before suggesting changes…").color(p.body));
        return;
    };
    let recs = labb::recommend(&app.config, Some(&profile), app.lab.summary.as_ref());
    if recs.is_empty() {
        widgets::section(ui, p, "No recommendation", |ui| {
            ui.label(
                crate::i18n::text("The detected hardware and current configuration do not justify a change from the evidence available. Run a benchmark for measured frame-time and memory evidence.")
                    .size(12.5)
                    .color(p.body),
            );
        });
    } else {
        widgets::section(ui, p, "Proposed changes", |ui| {
            for r in &recs {
                if app.lab.known.insert(r.id.clone()) {
                    app.lab.selected.insert(r.id.clone());
                }
                let mut checked = app.lab.selected.contains(&r.id);
                let header = format!("{} → {}", r.setting, r.proposed());
                if ui.checkbox(&mut checked, crate::i18n::text(header).font(theme::semibold(13.0))).changed() {
                    if checked {
                        app.lab.selected.insert(r.id.clone());
                    } else {
                        app.lab.selected.remove(&r.id);
                    }
                }
                ui.indent(("rec", &r.id), |ui| {
                    ui.label(crate::i18n::text(format!("Why: {}", r.reason)).size(12.0).color(p.body));
                    ui.label(crate::i18n::text(format!("Expected benefit: {}", r.benefit)).size(12.0).color(p.body));
                    ui.label(crate::i18n::text(format!("Trade-off: {}", r.tradeoff)).size(12.0).color(p.warn));
                    ui.label(crate::i18n::text(if r.restart { "Restart required: the engine reads this at start." } else { "No engine restart required." }).size(11.5).color(p.muted));
                });
                ui.separator();
            }
            let chosen = recs.iter().filter(|r| app.lab.selected.contains(&r.id)).count();
            ui.horizontal_wrapped(|ui| {
                if widgets::primary_button(ui, p, &format!("Review and apply {chosen} change{}", if chosen == 1 { "" } else { "s" }), chosen > 0).clicked() {
                    app.lab.confirm_apply = true;
                }
                if widgets::button(ui, p, "Restore previous settings", true)
                    .on_hover_text("Restores graphics and performance values saved before the last approved Lab change")
                    .clicked()
                {
                    app.lab.confirm_restore = true;
                }
                if let Some(b) = labb::load_backup() {
                    ui.label(crate::i18n::text(format!("Backup from {}", when(b.created_unix_s))).size(11.5).color(p.muted));
                } else {
                    ui.label(crate::i18n::text("No Lab backup yet.").size(11.5).color(p.muted));
                }
            });
        });
    }

    if let Some(s) = &app.lab.summary {
        widgets::section(ui, p, "Evidence behind the suggestions", |ui| {
            widgets::kv(ui, p, "Measured average", &format!("{:.1} fps", s.avg_fps));
            widgets::kv(ui, p, "Measured 1% low", &format!("{:.1} fps", s.low1_fps));
            widgets::kv(ui, p, "GPU budget peak", &s.gpu_peak_pct().map(|v| format!("{v}%")).unwrap_or_else(|| "unavailable".into()));
            widgets::kv(ui, p, "Host available minimum", &s.host_available_min_mb.map(crate::memory::gb).unwrap_or_else(|| "unavailable".into()));
            widgets::kv(ui, p, "Shader backlog peak", &s.max_shader_backlog.to_string());
            ui.label(crate::i18n::text("A short run is evidence for that scene, not a promise for every area of the game.").size(11.5).color(p.muted));
        });
    }
}

fn apply_modal(app: &mut App, ctx: &egui::Context, p: &theme::Palette) {
    if !app.lab.confirm_apply {
        return;
    }
    let recs = labb::recommend(&app.config, app.lab.profile.as_ref(), app.lab.summary.as_ref());
    let chosen: Vec<&Recommendation> = recs.iter().filter(|r| app.lab.selected.contains(&r.id)).collect();
    let mut decision = None;
    let modal = widgets::modal(
        ctx,
        p,
        "lab-apply",
        "Apply Performance Lab changes?",
        "The current graphics and performance settings are saved first. You can restore them from this page.",
        580.0,
        |ui| {
            for r in &chosen {
                ui.label(crate::i18n::text(format!("{} → {}", r.setting, r.proposed())).font(theme::semibold(13.0)).color(p.ink));
                ui.label(crate::i18n::text(&r.reason).size(12.0).color(p.body));
                if r.restart {
                    ui.label(crate::i18n::text("Restart the game for this to take effect.").size(11.5).color(p.warn));
                }
                ui.add_space(6.0);
            }
            ui.label(crate::i18n::text("No file outside the launcher's graphics and performance configuration is changed.").size(11.5).color(p.muted));
        },
        |ui| {
            if widgets::primary_button(ui, p, "Apply approved changes", true).clicked() {
                decision = Some(true);
            }
            if widgets::button(ui, p, "Cancel", true).clicked() {
                decision = Some(false);
            }
        },
    );
    if modal.close {
        decision = Some(false);
    }
    if let Some(true) = decision {
        let ids: HashSet<String> = chosen.iter().map(|r| r.id.clone()).collect();
        match labb::apply_approved(&mut app.config, &recs, &ids) {
            Ok(n) => {
                let saved = app.config.save();
                match saved {
                    Ok(()) => app.notify(format!("Applied {n} Performance Lab change(s). Restart the game to use them."), Tone::Ok),
                    Err(e) => app.notify(format!("Changes applied but the settings file could not be saved: {e}"), Tone::Warn),
                }
            }
            Err(e) => app.lab.notify(e, false),
        }
        app.lab.confirm_apply = false;
    } else if decision == Some(false) {
        app.lab.confirm_apply = false;
    }
}

fn restore_modal(app: &mut App, ctx: &egui::Context, p: &theme::Palette) {
    if !app.lab.confirm_restore {
        return;
    }
    let mut decision = None;
    let modal = widgets::modal(
        ctx,
        p,
        "lab-restore",
        "Restore settings from before the last Lab change?",
        "This replaces the current graphics and performance values with the Lab backup.",
        520.0,
        |ui| {
            ui.label(crate::i18n::text("The rest of the launcher configuration is untouched. Restart the game for restored settings to take effect.").color(p.body));
        },
        |ui| {
            if widgets::primary_button(ui, p, "Restore backup", true).clicked() {
                decision = Some(true);
            }
            if widgets::button(ui, p, "Cancel", true).clicked() {
                decision = Some(false);
            }
        },
    );
    if modal.close {
        decision = Some(false);
    }
    if let Some(true) = decision {
        match labb::restore_backup(&mut app.config) {
            Ok(_) => {
                let _ = app.config.save();
                app.notify("Restored graphics and performance settings from the Lab backup. Restart the game to use them.", Tone::Ok);
            }
            Err(e) => app.lab.notify(e, false),
        }
        app.lab.confirm_restore = false;
    } else if decision == Some(false) {
        app.lab.confirm_restore = false;
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(
        ui,
        &p,
        "Performance Lab",
        "Measure this PC and the real engine, understand the memory budget, and approve changes only when the evidence supports them.",
    );
    app.lab.refresh_profile(&app.config, false);
    ui.horizontal(|ui| {
        tab_button(app, ui, labb::Tab::Profile, "System Profile");
        tab_button(app, ui, labb::Tab::Benchmark, "Benchmark");
        tab_button(app, ui, labb::Tab::Optimize, "Optimization");
    });
    ui.add_space(theme::SPACE_M);
    if let Some((text, ok)) = app.lab.message.clone() {
        widgets::status_line(ui, &p, if ok { Tone::Ok } else { Tone::Bad }, "Performance Lab", &text);
        ui.add_space(theme::SPACE_M);
    }

    match app.lab.tab {
        labb::Tab::Profile => profile_tab(app, ui, &p),
        labb::Tab::Benchmark => benchmark_tab(app, ui, &p),
        labb::Tab::Optimize => recommend_tab(app, ui, &p),
    }

    let ctx = ui.ctx().clone();
    if app.lab.confirm_extended {
        let mut decision = None;
        let modal = widgets::modal(
            &ctx,
            &p,
            "lab-extended",
            "Run the extended benchmark?",
            "This observes 15 minutes of real game frame delivery.",
            540.0,
            |ui| {
                ui.label(crate::i18n::text("Extended testing is longer and is not a stress certification. The launcher does not raise power limits, overclock anything, or change driver settings. You can cancel at any time; cancelling stops sampling and leaves the game running.").color(p.body));
            },
            |ui| {
                if widgets::primary_button(ui, &p, "Start extended run", true).clicked() {
                    decision = Some(true);
                }
                if widgets::button(ui, &p, "Cancel", true).clicked() {
                    decision = Some(false);
                }
            },
        );
        if modal.close {
            decision = Some(false);
        }
        if let Some(true) = decision {
            app.lab.confirm_extended = false;
            start_run(app, Mode::Extended);
        } else if decision == Some(false) {
            app.lab.confirm_extended = false;
        }
    }
    if app.lab.confirm_cancel {
        let mut decision = None;
        let modal = widgets::modal(
            &ctx,
            &p,
            "lab-cancel",
            "Stop this benchmark?",
            "The partial samples are discarded. No setting is changed.",
            500.0,
            |ui| {
                ui.label(crate::i18n::text("The game keeps running if the launcher started it. Close it normally when you are finished.").color(p.body));
            },
            |ui| {
                if widgets::primary_button(ui, &p, "Stop recording", true).clicked() {
                    decision = Some(true);
                }
                if widgets::button(ui, &p, "Keep running", true).clicked() {
                    decision = Some(false);
                }
            },
        );
        if modal.close {
            decision = Some(false);
        }
        if let Some(true) = decision {
            if let Some(mut s) = app.lab.session.take() {
                s.recorder.cancel();
                if let Some(path) = s.soak_path.as_deref() {
                    labb::delete_partial_soak(path);
                }
            }
            app.lab.confirm_cancel = false;
            app.lab.notify("Benchmark cancelled; no partial run was saved. The game was not terminated.", true);
        } else if decision == Some(false) {
            app.lab.confirm_cancel = false;
        }
    }
    apply_modal(app, &ctx, &p);
    restore_modal(app, &ctx, &p);
}
