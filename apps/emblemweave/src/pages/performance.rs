//! Performance: one policy, the working systems it coordinates, and the
//! runtime's own measured status. Every control maps to a key the engine reads;
//! the sections that cannot be changed live say so instead of showing a switch
//! that does nothing.

use crate::config::Performance;
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui;

const MODES: &[&str] = &["Automatic", "Conservative", "Balanced", "Aggressive", "Manual"];
const SCENES: &[&str] = &["Auto", "Exploration", "Crowded", "Combat", "Cinematic", "Menu", "Loading"];
const RESIDENCY: &[&str] = &["Automatic", "On", "Off"];

fn combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[&str]) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(crate::i18n::tr(value))
        .width(190.0)
        .show_ui(ui, |ui| {
            let current = value.clone();
            if !current.is_empty() && !options.iter().any(|o| *o == current) {
                ui.selectable_value(value, current.clone(), crate::i18n::tr(&current));
                ui.separator();
            }
            for o in options {
                ui.selectable_value(value, (*o).into(), crate::i18n::tr(o));
            }
        });
}

fn number(ui: &mut egui::Ui, p: &theme::Palette, value: &mut u32, range: std::ops::RangeInclusive<f32>, suffix: &'static str) {
    let mut v = *value as f32;
    if widgets::Slider::new(&mut v, range).suffix(suffix).show(ui, p).changed() {
        *value = v.round() as u32;
    }
}

fn runtime(status: Option<&crate::performance::RuntimeStatus>, ui: &mut egui::Ui, p: &theme::Palette) {
    let Some(s) = status else {
        ui.label(
            crate::i18n::text(
                "No live engine snapshot. Start the game and it writes one per second to play/perf_status.json; the F1 Diagnostics tab has the live graphs.",
            )
            .size(12.5)
            .color(p.muted),
        );
        return;
    };
    let tone = if s.bottleneck.contains("VRAM") || s.bottleneck.contains("compilation") {
        Tone::Warn
    } else if s.bottleneck.contains("No dominant") {
        Tone::Ok
    } else {
        Tone::Accent
    };
    widgets::status_line(
        ui,
        p,
        tone,
        &format!("{:.1} fps · {} · {}", s.avg_fps, s.bottleneck, s.scene),
        &format!(
            "1% low {:.1} · 0.1% low {:.1} · P95 {:.2} ms · P99 {:.2} ms",
            s.low1_fps, s.low01_fps, s.p95_ms, s.p99_ms
        ),
    );
    ui.add_space(theme::SPACE_S);
    ui.horizontal_wrapped(|ui| {
        for (label, value) in [
            ("CPU", format!("{:.2} ms", s.cpu_ms)),
            ("GPU", format!("{:.2} ms", s.gpu_ms)),
            ("Upload", format!("{:.2} ms", s.upload_ms)),
            ("Replay", format!("{:.2} ms", s.replay_ms)),
            (
                "VRAM",
                if s.vram_percent >= 0 { format!("{}%", s.vram_percent) } else { "unknown".into() },
            ),
            ("Shaders", format!("{} waiting", s.shader_backlog)),
            ("Mip offset", format!("{:+.2}", s.mip_bias)),
            ("Changes", format!("{}", s.adjustments)),
        ] {
            widgets::pill(ui, p, &format!("{label}  {value}"), Tone::Neutral);
        }
    });
    if !s.profile_reason.is_empty() {
        ui.label(
            crate::i18n::text(format!("Profile: {} · {}", s.scene, s.profile_reason))
                .size(12.0)
                .color(p.muted),
        );
    }
    if !s.action.is_empty() {
        ui.add_space(4.0);
        ui.label(crate::i18n::text(&s.action).size(12.5).color(p.accent_hi));
    }
    ui.add_space(4.0);
    ui.label(
        crate::i18n::text("This is the game's measurement, refreshed once a second. The graph below is the launcher window's own timing.")
            .size(11.5)
            .color(p.muted),
    );
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(
        ui,
        &p,
        "Performance",
        "A measured controller over the engine systems that already exist — no decorative switches, and no silent quality changes.",
    );
    widgets::pending_note(
        ui,
        &p,
        "Stable systems are on by default. Adaptive texture detail is experimental and changes only sampler mip bias. Settings marked restart are read once when the game starts; the F1 Performance tab edits the same keys live.",
    );

    let runtime_now = crate::performance::runtime_status();
    if let Some(s) = &runtime_now {
        widgets::status_line(
            ui,
            &p,
            Tone::Ok,
            "Game is running",
            &format!("{} · {:.1} fps · {}", s.mode, s.avg_fps, s.scene),
        );
        ui.add_space(theme::SPACE_M);
    }

    let mut apply_reshade = false;
    {
        let perf: &mut Performance = &mut app.config.performance;
        widgets::section(ui, &p, "Automatic optimization", |ui| {
            widgets::setting_row(
                ui,
                &p,
                "Adaptive Performance Controller",
                "Classifies CPU, GPU, VRAM, streaming, shader, scheduling and synchronization pressure from the frame-time distribution. It reports its bottleneck and every action in the F1 Performance tab.",
                |ui| {
                    widgets::toggle(ui, &p, &mut perf.controller);
                },
            );
            widgets::setting_row(
                ui,
                &p,
                "Operating mode",
                "Automatic chooses the strength from measured pressure; Conservative, Balanced and Aggressive keep a fixed strength; Manual never actuates. Combat and cinematics are protected in every mode.",
                |ui| {
                    combo(ui, "perf-mode", &mut perf.mode, MODES);
                },
            );
            widgets::setting_row(
                ui,
                &p,
                "Target frame rate",
                "The controller's frame-time budget, not a frame limiter. Pacing lives on Display & Frame Pacing in the F1 menu and on Graphics here.",
                |ui| {
                    number(ui, &p, &mut perf.target_fps, 30.0..=240.0, " fps");
                },
            );
            widgets::setting_row(
                ui,
                &p,
                "Frame-Time Stability Optimizer",
                "Holds a detail reduction through a spike and counts recurring spikes. It adds no synchronization to the frame loop; the F1 Diagnostics tab records the distribution.",
                |ui| {
                    widgets::toggle(ui, &p, &mut perf.stability);
                },
            );
            widgets::setting_row(
                ui,
                &p,
                "Scene-aware profiles",
                "Recognises exploration, crowded areas, combat, cinematics, menus and loading from replay/CPU/GPU workload shape rather than location names.",
                |ui| {
                    widgets::toggle(ui, &p, &mut perf.scene_profiles);
                },
            );
            widgets::setting_row(
                ui,
                &p,
                "Profile override",
                "Auto follows the workload. A fixed profile is useful for an A/B run; it does not change the bottleneck classifier.",
                |ui| {
                    combo(ui, "perf-scene", &mut perf.scene_override, SCENES);
                },
            );
        });

        widgets::section(ui, &p, "Rendering", |ui| {
            ui.horizontal_wrapped(|ui| {
                widgets::pill(ui, &p, "Stable", Tone::Ok);
                ui.label(
                    crate::i18n::text("Adaptive LOD, occlusion culling, shadow resizing, effects quality, dynamic resolution, GPU-driven draws and object batching are not exposed as switches: the NVN replay renderer fixes those at startup or has no compatible path. The F1 Performance tab states each limit precisely.")
                        .size(12.0)
                        .color(p.muted),
                );
            });
            ui.add_space(theme::SPACE_S);
            widgets::setting_row(
                ui,
                &p,
                "Adaptive texture detail",
                "Experimental. Under sustained GPU or VRAM pressure the sampler LOD bias moves toward cheaper mips. The native UI is drawn by a separate path and stays sharp; the offset is released when pressure ends. It can soften high-frequency surfaces while it is active.",
                |ui| {
                    widgets::toggle(ui, &p, &mut perf.adaptive_mip);
                },
            );
            widgets::setting_row(
                ui,
                &p,
                "Maximum mip offset",
                "The ceiling for that automatic offset. 0.25 is subtle, 0.75 the default, 2.0 aggressive. Only used when adaptive texture detail is on.",
                |ui| {
                    widgets::Slider::new(&mut perf.max_mip_bias, 0.25..=2.0)
                        .fixed_decimals(2)
                        .show(ui, &p);
                },
            );
        });

        widgets::section(ui, &p, "CPU and scheduling", |ui| {
            ui.label(
                crate::i18n::text("Worker pools are sized when the game starts; live resizing of an already-created pool is unsupported because the pools own fixed command buffers and queues. These controls set the startup size and priority that the pools actually use.")
                    .size(12.0)
                    .color(p.muted),
            );
            ui.add_space(theme::SPACE_S);
            widgets::setting_row(
                ui,
                &p,
                "Background texture preparation",
                "Stable. Hashes, cache reads, untiling and BC7 work happen on workers when the game initialises a texture, not on the render thread at first use. Applies at the next start.",
                |ui| {
                    widgets::toggle(ui, &p, &mut perf.tex_prep);
                },
            );
            widgets::setting_row(
                ui,
                &p,
                "Preparation workers",
                "0 chooses a third of the hardware threads (2 to 8). Lower it on a low-core CPU so the game and render threads keep their share. Applies at the next start.",
                |ui| {
                    number(ui, &p, &mut perf.tex_prep_threads, 0.0..=8.0, " threads");
                },
            );
            widgets::setting_row(
                ui,
                &p,
                "Preparation memory budget",
                "Most finished or queued background texture data held at once. Over it the oldest unused results are dropped and prepared again only if needed. Applies at the next start.",
                |ui| {
                    number(ui, &p, &mut perf.tex_prep_mb, 128.0..=4096.0, " MB");
                },
            );
            widgets::setting_row(
                ui,
                &p,
                "Pipeline prewarm and shader parts",
                "Stable. Prewarm builds recorded pipelines for the next run; the pipeline library compiles shader parts on background workers. Both use the persistent pipeline cache. The shader worker cap is on Graphics (Compile threads) so it is set in one place.",
                |ui| {
                    ui.horizontal(|ui| {
                        widgets::toggle(ui, &p, &mut perf.prewarm);
                        ui.add_space(6.0);
                        widgets::toggle(ui, &p, &mut perf.gpl);
                        ui.label(
                            crate::i18n::text("prewarm / library")
                                .size(11.5)
                                .color(p.muted),
                        );
                    });
                },
            );
            widgets::setting_row(
                ui,
                &p,
                "Lean draw replay",
                "Stable, existing path: applies only changed Vulkan state, reads resolved bindings in place and pushes only changed uniform buffers. Turn it off only to troubleshoot.",
                |ui| {
                    widgets::toggle(ui, &p, &mut perf.lean_draws);
                },
            );
        });

        widgets::section(ui, &p, "Memory and streaming", |ui| {
            widgets::setting_row(
                ui,
                &p,
                "Upload staging ring",
                "Stable. A persistent host buffer reused as a ring for texture uploads, instead of allocating pinned memory per texture. 0 restores per-upload buffers. Applies at the next start.",
                |ui| {
                    number(ui, &p, &mut perf.staging_ring_mb, 0.0..=1024.0, " MB");
                },
            );
            widgets::setting_row(
                ui,
                &p,
                "Upload budget per frame",
                "Stable. Render-thread milliseconds for asset textures that only a pool needs. Past it they stream in over later frames, avoiding one multi-second load-in frame. 0 means no limit. Applies at the next start.",
                |ui| {
                    number(ui, &p, &mut perf.pool_upload_ms, 0.0..=24.0, " ms");
                },
            );
            widgets::setting_row(
                ui,
                &p,
                "VRAM residency",
                "Stable. Demotes textures no recent draw uses when video memory approaches the budget, then restores them when needed. Automatic follows the GPU and the Intel compatibility quirk. Applies at the next start.",
                |ui| {
                    combo(ui, "perf-residency", &mut perf.residency, RESIDENCY);
                },
            );
            widgets::setting_row(
                ui,
                &p,
                "Residency threshold",
                "Share of the video-memory budget over which idle textures may be demoted. Higher uses more VRAM and demotes later; lower protects smaller cards. Applies at the next start.",
                |ui| {
                    number(ui, &p, &mut perf.residency_percent, 50.0..=100.0, " %");
                },
            );
        });

        widgets::section(ui, &p, "Diagnostics and ReShade", |ui| {
            runtime(runtime_now.as_ref(), ui, &p);
            ui.add_space(theme::SPACE_S);
            widgets::setting_row(
                ui,
                &p,
                "XiØN style for the ReShade overlay",
                "Writes the palette into ReShade.ini's documented [STYLE] custom-style keys (StyleIndex 4) when Play starts. ReShade remains third-party: no binary is patched. The first write keeps a .xion-backup beside it.",
                |ui| {
                    widgets::toggle(ui, &p, &mut perf.reshade_theme);
                },
            );
            if widgets::button(ui, &p, "Apply the XiØN style now", true)
                .on_hover_text(crate::i18n::tr("Requires ReShade.ini next to the built game executable."))
                .clicked()
            {
                apply_reshade = true;
            }
            if let Some(path) = crate::performance::reshade_ini() {
                ui.label(
                    crate::i18n::text(format!("ReShade.ini found at {}", path.display()))
                        .size(11.5)
                        .color(p.muted),
                );
            } else {
                ui.label(
                    crate::i18n::text("ReShade.ini is not installed beside the game; install ReShade for this game to use its overlay.")
                        .size(11.5)
                        .color(p.muted),
                );
            }
        });
    }

    // The launcher's own graph; the game's numbers are the live snapshot above.
    crate::pages::diagnostics::frame_times(app, ui, &p);

    if apply_reshade {
        match crate::performance::apply_reshade_theme() {
            Ok(msg) => {
                app.config.performance.reshade_theme = true;
                app.notify(msg, Tone::Ok);
            }
            Err(e) => app.notify(e, Tone::Warn),
        }
    }
}
