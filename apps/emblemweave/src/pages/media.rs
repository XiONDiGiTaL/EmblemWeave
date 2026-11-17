//! Graphics and audio settings.
//!
//! The renderer and the audio backend do not exist yet, so every option here
//! is stored for the day they land and none of them claims to do anything
//! today. Each page says so at the top.

use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText};

/// Every upscaler the engine can run itself. `engine::upscale_mode` maps these
/// labels to the modes in its ini.
const UPSCALERS: &[&str] = &["Off", "FSR 1.0", "FSR 3.1", "NVIDIA Image Scaling", "DLSS", "DLAA", "XeSS"];

/// The engine's `[upscale] sharpness` steps: Off, Low, its own 0.8, and 1.0.
const SHARPENING: &[&str] = &["Off", "Low", "Default", "High"];

/// What the selected upscaler needs, and whether it is there: FSR 3.1 and DLSS
/// need DLLs AMD and NVIDIA do not let us ship.
fn upscaler_help(current: &str, sdk: &crate::engine::Sdk) -> String {
    match current {
        "FSR 1.0" => "AMD's spatial upscaler: scales and sharpens on any GPU, with no motion vectors.".into(),
        "FSR 3.1" => match &sdk.fsr3 {
            Some(dir) => format!("AMD's temporal upscaler, using the FidelityFX DLLs in {}. Applies at the game's next start.", dir.display()),
            None => "AMD's temporal upscaler. Put amd_fidelityfx_loader_vk.dll and amd_fidelityfx_upscaler_vk.dll in upscalers\\fsr in the play folder; without them the engine logs why and leaves the picture unscaled.".into(),
        },
        "NVIDIA Image Scaling" => "NVIDIA's spatial upscaler: any GPU, up to 2x per side, FSR 1 beyond that.".into(),
        "DLSS" | "DLAA" => match &sdk.dlss {
            Some(dir) => format!("NVIDIA's temporal upscaler through Streamline, in {}. Applies at the game's next start.", dir.display()),
            None => "NVIDIA's temporal upscaler. Needs NVIDIA's four Streamline DLLs in upscalers\\streamline in the play folder, on an RTX GPU; without them the engine keeps its own upscaler.".into(),
        },
        "XeSS" => match &sdk.xess {
            Some(dir) => format!("Intel's temporal upscaler through libxess.dll in {}. Applies at the game's next start.", dir.display()),
            None => "Intel's temporal upscaler, which runs on any GPU with DP4a (best on Intel Arc). Needs libxess.dll in upscalers\\xess in the play folder, and the Visual C++ 2015-2022 runtime; without them the engine keeps its own upscaler.".into(),
        },
        _ => "The picture is scaled by the presentation path alone.".into(),
    }
}

fn combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[&str]) {
    egui::ComboBox::from_id_salt(id).selected_text(value.clone()).width(200.0).show_ui(ui, |ui| {
        // A value the engine wrote (an aspect of 1.777…, a newer upscaler name) is
        // kept as the current choice instead of disappearing from the list.
        let current = value.clone();
        if !current.is_empty() && !options.iter().any(|o| *o == current) {
            ui.selectable_value(value, current.clone(), current);
            ui.separator();
        }
        for o in options {
            ui.selectable_value(value, o.to_string(), *o);
        }
    });
}

const UPSCALE_QUALITIES: &[&str] = &["Native", "Quality", "Balanced", "Performance", "Ultra performance"];
const FRAME_GENERATION: &[&str] = &["Off", "DLSS Frame Generation", "FSR 3 Frame Generation"];
const DLSS_UPSCALERS: &[&str] = &["Automatic", "TAAU", "DLSS", "FSR 3", "XeSS", "Bilinear"];
const ASTC_QUALITIES: &[&str] = &["High", "Fast", "Uncompressed"];
const GPU_CHOICES: &[&str] = &["Automatic", "Discrete", "Integrated"];
const COMPILE_THREADS: &[(u32, &str)] = &[(0, "Automatic"), (1, "1"), (2, "2"), (3, "3"), (4, "4"), (6, "6"), (8, "8")];
const COMPILE_PRIORITIES: &[&str] = &["Below normal", "Normal", "Idle"];

/// A combo for a number field (the compile thread count), labelled rather than typed.
fn combo_num(ui: &mut egui::Ui, id: &str, value: &mut u32, options: &[(u32, &str)]) {
    let labels: Vec<&str> = options.iter().map(|(_, l)| *l).collect();
    let mut label = options.iter().find(|(v, _)| v == value).map(|(_, l)| l.to_string()).unwrap_or_else(|| labels[0].to_string());
    combo(ui, id, &mut label, &labels);
    if let Some((v, _)) = options.iter().find(|(_, l)| *l == label) {
        *value = *v;
    }
}

pub fn graphics(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Graphics", "Display and rendering options, for the game's engine and for EmblemWeave's own renderer.");
    widgets::pending_note(
        ui,
        &p,
        "The game creates its device, memory, textures, shaders, swap chain and command buffers through the host NVN layer on your GPU, \
         but draw commands are recorded and counted, not rendered yet. Everything on this page except Quality and Precompile shaders is handed to the engine \
         in its ini files before every start, so the game itself uses it.",
    );
    memory_profile(app, ui);
    upscaler_files(app, ui);
    let g = &mut app.config.graphics;
    // The engine resolves `auto` to a float such as 1.7777777777777777. Show the
    // ratio it means, and keep the value itself when it is a custom shape.
    g.aspect = crate::engine::aspect_label(&g.aspect);
    widgets::section(ui, &p, "Display", |ui| {
        widgets::setting_row(ui, &p, "Resolution", "The picture's size and the window's, taken at the game's next start. Native display keeps the game's own console size.", |ui| {
            combo(ui, "res", &mut g.resolution, &["Native display", "1280 × 720", "1920 × 1080", "2560 × 1440", "3840 × 2160"]);
        });
        widgets::setting_row(ui, &p, "Display mode", "Windowed, borderless or exclusive, from the game's next start.", |ui| {
            combo(ui, "mode", &mut g.display_mode, &["Windowed", "Borderless window", "Exclusive fullscreen"]);
        });
        widgets::setting_row(ui, &p, "Aspect ratio", "Wider ratios need the game's camera and UI adjusted, not only the output. Applies at the next start.", |ui| {
            combo(ui, "aspect", &mut g.aspect, &["16:9", "16:10", "21:9", "32:9"]);
        });
        widgets::setting_row(ui, &p, "VSync", "Synchronise presentation with the display. Applies at the next start.", |ui| {
            widgets::toggle(ui, &p, &mut g.vsync);
        });
        widgets::setting_row(ui, &p, "Frame-rate limit", "Caps presentation only: the game's simulation keeps its own tick. The engine reads it at the next start.", |ui| {
            let mut fps = g.frame_limit as f32;
            if widgets::Slider::new(&mut fps, 30.0..=240.0).suffix(" fps").show(ui, &p).changed() {
                g.frame_limit = fps.round() as u32;
            }
        });
    });
    widgets::section(ui, &p, "Rendering", |ui| {
        widgets::setting_row(ui, &p, "Render scale", "Internal resolution relative to the output; the upscaler fills the rest. Applies at the next start.", |ui| {
            widgets::Slider::new(&mut g.render_scale, 0.5..=2.0).fixed_decimals(2).suffix("×").show(ui, &p);
        });
        let help = upscaler_help(&g.upscaling, &crate::engine::sdk());
        widgets::setting_row(ui, &p, "Upscaling", &help, |ui| {
            combo(ui, "upscale", &mut g.upscaling, UPSCALERS);
        });
        widgets::setting_row(
            ui,
            &p,
            "Sharpening",
            "How crisp the final picture is. Default is the engine's own 0.8; High suits output above the render size, where the upscaler softens edges.",
            |ui| {
                combo(ui, "sharpening", &mut g.sharpening, SHARPENING);
            },
        );
        widgets::setting_row(
            ui,
            &p,
            "Upscaler quality",
            "The temporal upscalers' preset: the share of the output they reconstruct from. Native renders every pixel and only replaces the console's own anti-aliasing.",
            |ui| {
                combo(ui, "uq", &mut g.upscale_quality, UPSCALE_QUALITIES);
            },
        );
        widgets::setting_row(
            ui,
            &p,
            "Scene scale",
            "The game's own scene buffer as a percentage of the output, below the launcher's render scale. Lower fills more of the picture from history: cheaper, and softer where motion vectors are wrong.",
            |ui| {
                let mut pct = g.scene_scale as f32;
                if widgets::Slider::new(&mut pct, 25.0..=100.0).suffix(" %").show(ui, &p).changed() {
                    g.scene_scale = pct.round() as u32;
                }
            },
        );
        widgets::setting_row(
            ui,
            &p,
            "Frame generation",
            "Generates the frames between two rendered ones. DLSS Frame Generation needs Windows' hardware-accelerated GPU scheduling, an RTX 40 or newer card and the Streamline files; FSR 3 needs amd_fidelityfx_vk.dll. Keep vsync and latency mode off.",
            |ui| {
                combo(ui, "fg", &mut g.frame_gen, FRAME_GENERATION);
            },
        );
        widgets::setting_row(
            ui,
            &p,
            "DLSS replacement",
            "What renders the frames the game asks its console's DLSS hardware for: the engine's own temporal upscaler, NVIDIA's, AMD's, or a plain bilinear stretch. Automatic uses whichever DLLs are found.",
            |ui| {
                combo(ui, "du", &mut g.dlss_upscaler, DLSS_UPSCALERS);
            },
        );
        widgets::setting_row(
            ui,
            &p,
            "Quality",
            "Original keeps the console's rendering exactly.",
            |ui| {
            combo(ui, "quality", &mut g.quality, &["Original", "Enhanced"]);
        });
        widgets::setting_row(ui, &p, "Precompile shaders", "Translate the game's shaders before play to avoid stutter.", |ui| {
            widgets::toggle(ui, &p, &mut g.shader_precompile);
        });
        widgets::setting_row(
            ui,
            &p,
            "Texture quality",
            "How the game's ASTC textures reach a GPU that cannot sample them. High transcodes once in the background for a sharper picture; Fast is quicker to start and uses less disk; Uncompressed costs four times the video memory.",
            |ui| {
                combo(ui, "astc", &mut g.astc_quality, ASTC_QUALITIES);
            },
        );
        widgets::setting_row(
            ui,
            &p,
            "GPU",
            "Which Vulkan device the engine opens. Automatic takes the discrete GPU with the most video memory.",
            |ui| {
                combo(ui, "gpu", &mut g.gpu, GPU_CHOICES);
            },
        );
        widgets::setting_row(ui, &p, "Renderer", "", |ui| {
            ui.label(RichText::new("Vulkan").font(theme::semibold(14.0)).color(p.ink));
        });
        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "FSR 4 needs AMD's DirectX 12 FidelityFX runtime and an RDNA 4 card, and OptiScaler swaps upscalers by \
                 hooking DirectX; this renderer presents through Vulkan, so neither can drive it. FSR 3.1 is the newest \
                 upscaler the engine runs itself.",
            )
            .size(12.5)
            .color(p.muted),
        );
    });

    let compile_summary = format!("{} threads · {}", g.compile_threads, g.compile_priority.to_lowercase());
    widgets::section_drawer(ui, &p, "shader-compilation", "Shader compilation (CPU)", &compile_summary, false, |ui| {
        ui.label(
            RichText::new(
                "While a new area loads, the engine compiles shader parts, whole pipelines and the boot prewarm on \
                 background threads. Two threads at idle priority in efficiency mode are the defaults now: a 24-thread \
                 laptop used to start up to ten compiler threads at normal-ish priority, which is the 6 GHz burst and the \
                 fan noise in the report. Lower counts trade a little first-draw stutter for a quieter machine.",
            )
            .size(12.5)
            .color(p.muted),
        );
        widgets::setting_row(
            ui,
            &p,
            "CPU preset",
            "One press for the three values below. Quiet is for a laptop on battery; Fast spends more CPU to remove first-draw stutter.",
            |ui| {
                if widgets::button(ui, &p, "Quiet", true).on_hover_text("1 thread, idle priority, efficiency mode").clicked() {
                    g.compile_threads = 1;
                    g.compile_priority = "Idle".into();
                    g.compile_eco = true;
                }
                if widgets::button(ui, &p, "Balanced", true).on_hover_text("2 threads, idle priority, efficiency mode").clicked() {
                    g.compile_threads = 2;
                    g.compile_priority = "Idle".into();
                    g.compile_eco = true;
                }
                if widgets::button(ui, &p, "Fast", true).on_hover_text("4 threads, below normal priority, efficiency mode off").clicked() {
                    g.compile_threads = 4;
                    g.compile_priority = "Below normal".into();
                    g.compile_eco = false;
                }
            },
        );
        widgets::setting_row(
            ui,
            &p,
            "Compile threads",
            "Background threads for whole pipelines, boot prewarm and shader parts together. Automatic leaves the engine's own count; the default is 2. Applies at the next start.",
            |ui| {
                combo_num(ui, "cthreads", &mut g.compile_threads, COMPILE_THREADS);
            },
        );
        widgets::setting_row(
            ui,
            &p,
            "Compile priority",
            "Thread priority for those compiles. Idle is the default and keeps them off the fast cores; Normal brings back the stutter-free first draw but competes with the game.",
            |ui| {
                combo(ui, "cprio", &mut g.compile_priority, COMPILE_PRIORITIES);
            },
        );
        widgets::setting_row(
            ui,
            &p,
            "Efficiency mode",
            "Runs those threads in Windows' efficiency mode (EcoQoS), so a burst of compiles does not take the fast cores from the game.",
            |ui| {
                widgets::toggle(ui, &p, &mut g.compile_eco);
            },
        );
    });

    let desired = crate::engine::desired_values(g, &app.config.audio);
    // The files the game actually reads, so a change the launcher has written
    // for the next start is visibly waiting rather than silently absent.
    if let Some(l) = crate::engine::live() {
        let (pending, kept) = waiting(&desired, &l.values);
        widgets::section_drawer(ui, &p, "graphics-live", "What the game will start with", "the files the engine reads", false, |ui| {
            widgets::kv(ui, &p, "Upscaler", crate::engine::upscale_label(&l.upscaling));
            widgets::kv(
                ui,
                &p,
                "Output",
                &match l.output.as_str() {
                    "console" => "console (the game's own size)".to_string(),
                    "window" => "the window's size".to_string(),
                    other => other.to_string(),
                },
            );
            widgets::kv(ui, &p, "Window", &format!("{} × {} · {}", l.window.0, l.window.1, l.window_mode));
            widgets::kv(ui, &p, "Picture", &format!("{} × {}", l.picture.0, l.picture.1));
            widgets::kv(
                ui,
                &p,
                "Frame limit",
                &match l.fps_limit.as_str() {
                    "0" => "unlimited".to_string(),
                    "auto" => "the display's refresh".to_string(),
                    other => other.to_string(),
                },
            );
            widgets::kv(ui, &p, "VSync", if l.vsync { "on" } else { "off" });
            widgets::kv(ui, &p, "Aspect", &l.aspect);
            waiting_line(ui, &p, pending, kept);
        });
    }
}

/// The hardware facts, the Low Memory recommendation and the explicit apply /
/// restore controls. The profile is never applied by detection alone.
fn memory_profile(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    let snap = crate::memory::snapshot();
    let advice = crate::memory::assess(&snap);
    let active = app.config.graphics_preset.low_memory;
    let dismissed = app.config.graphics_preset.dismissed;
    let summary = if active {
        "Low Memory on".to_string()
    } else if advice.recommend_low_memory {
        "Low Memory recommended".to_string()
    } else if advice.pressure {
        "memory pressure".to_string()
    } else if advice.known {
        "standard".to_string()
    } else {
        "memory facts unavailable".to_string()
    };
    let open = active || (advice.recommend_low_memory && !dismissed);
    let (mut apply, mut dismiss, mut restore) = (false, false, false);
    widgets::section_drawer(ui, &p, "memory-profile", "Memory profile", &summary, open, |ui| {
        let mut facts = Vec::new();
        if let Some(installed) = snap.installed_mb.or(snap.total_mb) {
            facts.push(format!("{} installed", crate::memory::gb(installed)));
        }
        if let Some(free) = snap.available_mb {
            facts.push(format!("{} free now", crate::memory::gb(free)));
        }
        if let Some(commit) = snap.commit_free_mb {
            facts.push(format!("{} commit free", crate::memory::gb(commit)));
        }
        if !facts.is_empty() {
            ui.label(RichText::new(facts.join("  ·  ")).size(12.5).color(p.body));
        }
        if let Some(gpu) = &snap.gpu {
            let kind = match gpu.shared {
                Some(true) => "shared with the system",
                Some(false) => "dedicated",
                None => "kind not reported",
            };
            let mut text = format!("{} ({kind})", gpu.name);
            if let Some(budget) = gpu.budget_mb {
                text.push_str(&format!("  ·  {} video budget", crate::memory::gb(budget)));
                if let Some(used) = gpu.used_mb {
                    text.push_str(&format!(", {} used in the last run", crate::memory::gb(used)));
                }
            }
            ui.label(RichText::new(text).size(12.5).color(p.muted));
        }
        ui.add_space(6.0);
        if advice.recommend_low_memory && !active && !dismissed {
            widgets::status_line(
                ui,
                &p,
                Tone::Warn,
                "This hardware is in the Low Memory class",
                &format!(
                    "{}. The profile lowers the output, render and scene sizes, turns frame generation off, uses Fast texture throughput and one idle shader-compile thread. It is a recommendation, not a claim that a previous crash was caused by memory.",
                    advice.reasons.join("; ")
                ),
            );
            ui.add_space(6.0);
        } else if advice.pressure {
            widgets::status_line(
                ui,
                &p,
                Tone::Bad,
                "Windows or the GPU is short of memory now",
                &format!(
                    "{}. Close overlays and background applications before a long session; keep the Windows page file system-managed. This warning does not identify a crash cause on its own.",
                    advice.reasons.join("; ")
                ),
            );
            ui.add_space(6.0);
        } else if advice.known {
            ui.label(RichText::new("No memory warning for the facts available now.").size(12.5).color(p.ok));
        } else {
            ui.label(
                RichText::new("Windows memory facts and the engine's GPU log are not available yet. The profile can still be applied explicitly; no warning is inferred from missing data.")
                    .size(12.5)
                    .color(p.muted),
            );
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if active {
                if widgets::button(ui, &p, "Restore settings from before Low Memory", true)
                    .on_hover_text("Puts back every graphics setting from before the first application. Audio, controls, saves and mods are untouched.")
                    .clicked()
                {
                    restore = true;
                }
                widgets::pill(ui, &p, "applied", Tone::Ok);
            } else if widgets::button(ui, &p, "Apply Low Memory", true)
                .on_hover_text("Changes only the graphics settings listed below. The previous values are kept for one-press restore.")
                .clicked()
            {
                apply = true;
            }
            if advice.recommend_low_memory && !active && !dismissed && widgets::button(ui, &p, "Not now", true).clicked() {
                dismiss = true;
            }
        });
        ui.add_space(6.0);
        let preview = if let Some(previous) = &app.config.graphics_preset.previous {
            crate::memory::changes(previous, &app.config.graphics)
        } else {
            let next = crate::memory::low_memory(&app.config.graphics);
            crate::memory::changes(&app.config.graphics, &next)
        };
        if preview.is_empty() {
            ui.label(RichText::new("The current graphics settings already match the profile.").size(12.5).color(p.muted));
        } else {
            for line in preview {
                ui.label(RichText::new(format!("• {line}")).size(12.5).color(p.body));
            }
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new("The profile does not change audio, controls, saves, mods, the game dump or the engine's automatic texture-residency and BC1 choices. The engine still decides those from the GPU it finds. The next Play writes the settings; a build is not needed.")
                .size(12.0)
                .color(p.muted),
        );
    });
    if apply {
        let changes = app.config.apply_low_memory();
        app.notify(format!("Low Memory applied: {}", changes.join("; ")), Tone::Ok);
    }
    if dismiss {
        app.config.dismiss_low_memory();
    }
    if restore {
        app.confirm = Some(crate::Confirm::RestoreGraphics);
    }
}

const FSR_SDK_URL: &str = "https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/releases";
const FSR_114_URL: &str = "https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/releases/tag/v1.1.4";
const STREAMLINE_URL: &str = "https://github.com/NVIDIA-RTX/Streamline/releases";
const XESS_URL: &str = "https://github.com/intel/xess/releases";

/// The vendor DLLs the engine cannot ship, whether they are on disk, and the
/// buttons that make finding them the launcher's job rather than the player's.
/// Missing files get the vendor's own download page next to them; the guide
/// spells out the names and destinations so nobody has to guess.
fn upscaler_files(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    let sdk = crate::engine::sdk();
    let dir = crate::engine::upscalers_dir();
    let fsr_missing = sdk.fsr3.is_none();
    let fsr_framegen_missing = !sdk.fsr_framegen();
    let dlss_missing = sdk.dlss.is_none();
    let dlss_framegen_missing = !sdk.dlss_framegen();
    let xess_missing = sdk.xess.is_none();
    let fsr = match &sdk.fsr3 {
        Some(d) if !fsr_framegen_missing => format!("found in {}, with frame generation", d.display()),
        Some(d) => format!("found in {}; frame generation still needs amd_fidelityfx_vk.dll (FidelityFX SDK v1.1.4)", d.display()),
        None => "missing: amd_fidelityfx_upscaler_vk.dll with amd_fidelityfx_loader_vk.dll (FSR SDK v2.4), or amd_fidelityfx_vk.dll (v1.1.4)".to_string(),
    };
    let dlss = match &sdk.dlss {
        Some(d) if !dlss_framegen_missing => format!("found in {}, with frame generation", d.display()),
        Some(d) => format!("found in {}; frame generation still needs sl.dlss_g.dll, sl.reflex.dll, sl.pcl.dll, nvngx_dlssg.dll and NvLowLatencyVk.dll", d.display()),
        None => "missing: sl.interposer.dll, sl.common.dll, sl.dlss.dll and nvngx_dlss.dll from Streamline".to_string(),
    };
    let xess = match &sdk.xess {
        Some(d) => format!("found in {}", d.display()),
        None => "missing: libxess.dll from the Intel XeSS SDK".to_string(),
    };
    let where_to = match &dir {
        Some(d) => format!("Put them in {} (fsr, streamline and xess), or in an upscalers folder beside the game.", d.display()),
        None => "Put them in an upscalers folder beside the game.".to_string(),
    };
    let guide_id = egui::Id::new("upscaler-guide-open");
    let mut guide_open = ui.ctx().data(|d| d.get_temp::<bool>(guide_id).unwrap_or(false));

    let found = [!fsr_missing, !dlss_missing, !xess_missing].into_iter().filter(|found| *found).count();
    let summary = format!("{found} of 3 found");
    widgets::section_drawer(ui, &p, "upscaler-files", "Upscaler files", &summary, found < 3, |ui| {
        upscaler_line(
            ui,
            &p,
            if fsr_missing { Tone::Bad } else if fsr_framegen_missing { Tone::Warn } else { Tone::Ok },
            "FSR 3.1",
            &fsr,
            if fsr_missing {
                Some(("Get FSR SDK", FSR_SDK_URL))
            } else if fsr_framegen_missing {
                Some(("Get FSR v1.1.4", FSR_114_URL))
            } else {
                None
            },
        );
        upscaler_line(
            ui,
            &p,
            if dlss_missing { Tone::Bad } else if dlss_framegen_missing { Tone::Warn } else { Tone::Ok },
            "DLSS",
            &dlss,
            if dlss_missing {
                Some(("Get Streamline", STREAMLINE_URL))
            } else if dlss_framegen_missing {
                Some(("Get Streamline", STREAMLINE_URL))
            } else {
                None
            },
        );
        upscaler_line(
            ui,
            &p,
            if xess_missing { Tone::Bad } else { Tone::Ok },
            "XeSS",
            &xess,
            xess_missing.then_some(("Get XeSS SDK", XESS_URL)),
        );
        ui.add_space(theme::SPACE_S);
        let note = format!(
            "{} AMD, NVIDIA and Intel do not let these travel with EmblemWeave, so the game falls back to its own temporal upscaler until they are there; the F1 menu's Graphics tab then names them under Files found.",
            where_to
        );
        ui.label(RichText::new(note).size(12.5).color(p.body));
        ui.add_space(theme::SPACE_S);
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, &p, "Open the upscalers folder", dir.is_some()).clicked() {
                if let Some(d) = &dir {
                    reveal_upscalers(d);
                }
            }
            if widgets::button(ui, &p, "Look for the files", true)
                .on_hover_text("Searches beside the launcher, the engine folder, Downloads and the Desktop, one folder deep, and copies what it finds into the upscalers folder.")
                .clicked()
            {
                match crate::engine::adopt_upscaler_files() {
                    Ok(m) => app.notify(m, Tone::Ok),
                    Err(e) => app.notify(e, Tone::Bad),
                }
            }
            if widgets::button(ui, &p, "Setup guide", true)
                .on_hover_text("Which files to download, what they are called and exactly where they go")
                .clicked()
            {
                guide_open = true;
            }
        });
    });

    if guide_open {
        let shown = widgets::modal(ui.ctx(), &p, "upscaler-setup-guide", "Upscaler setup guide", "Vendor DLLs stay external because their licences do not allow shipping them.", 700.0, |ui| {
                    ui.label(
                        RichText::new("Vendor DLLs are not shipped with EmblemWeave because their licences do not allow it. Copy the files into the folders below, then press Look for the files. The engine reads them at the next start.")
                            .size(12.5)
                            .color(p.body),
                    );
                    ui.add_space(theme::SPACE_S);
                    guide_block(
                        ui,
                        &p,
                        "FSR 3.1 and FSR 3 frame generation",
                        &[
                            "Download the FidelityFX SDK release. From the SDK's Windows x64 output, copy amd_fidelityfx_loader_vk.dll and amd_fidelityfx_upscaler_vk.dll into upscalers\\fsr.",
                            "For FSR 3 frame generation, use the FidelityFX SDK v1.1.4 release and copy amd_fidelityfx_vk.dll into the same upscalers\\fsr folder.",
                            "All three files can sit in that one folder; do not rename them.",
                        ],
                        Some(("Open FSR SDK releases", FSR_SDK_URL)),
                    );
                    guide_block(
                        ui,
                        &p,
                        "DLSS and DLSS Frame Generation",
                        &[
                            "Download the NVIDIA Streamline SDK release for Windows x64. From its bin\\x64 folder, copy sl.interposer.dll, sl.common.dll, sl.dlss.dll and nvngx_dlss.dll into upscalers\\streamline.",
                            "For DLSS Frame Generation also copy sl.dlss_g.dll, sl.reflex.dll, sl.pcl.dll, nvngx_dlssg.dll and NvLowLatencyVk.dll into the same folder.",
                            "DLSS needs an RTX card. Frame generation needs an RTX 40 or newer card and Windows hardware-accelerated GPU scheduling.",
                        ],
                        Some(("Open Streamline releases", STREAMLINE_URL)),
                    );
                    guide_block(
                        ui,
                        &p,
                        "XeSS",
                        &[
                            "Download the Intel XeSS SDK release. Copy libxess.dll from its Windows x64 output into upscalers\\xess.",
                            "Install the Microsoft Visual C++ 2015-2022 Redistributable (x64) if Windows reports a missing runtime. XeSS runs on any GPU with DP4a, and is fastest on Intel Arc.",
                        ],
                        Some(("Open XeSS releases", XESS_URL)),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new("Do not copy the whole SDK folder: the engine looks for these exact file names at the top of fsr, streamline and xess. If a file is renamed, the game will not see it.")
                            .size(12.0)
                            .color(p.muted),
                    );
                    ui.add_space(theme::SPACE_S);
                    if widgets::button(ui, &p, "Open the upscalers folder", dir.is_some()).clicked() {
                        if let Some(d) = &dir {
                            reveal_upscalers(d);
                        }
                    }
            }, |_| {});
        if shown.close {
            guide_open = false;
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(guide_id, guide_open));
}

fn upscaler_line(ui: &mut egui::Ui, p: &theme::Palette, tone: Tone, label: &str, detail: &str, download: Option<(&str, &str)>) {
    ui.horizontal(|ui| {
        let (dot, _) = ui.allocate_exact_size(egui::Vec2::splat(10.0), egui::Sense::hover());
        ui.painter().circle_filled(dot.center(), 4.0, tone.colour(p));
        ui.vertical(|ui| {
            let reserve = if download.is_some() { 148.0 } else { 0.0 };
            ui.set_max_width((ui.available_width() - reserve).max(120.0));
            ui.label(RichText::new(label).font(theme::semibold(13.5)).color(p.ink));
            ui.add(egui::Label::new(RichText::new(detail).size(12.5).color(p.body)).wrap());
        });
        if let Some((text, url)) = download {
            if widgets::button(ui, p, text, true)
                .on_hover_text("Opens the official download page in your browser")
                .clicked()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
    });
    ui.add_space(4.0);
}

fn guide_block(ui: &mut egui::Ui, p: &theme::Palette, title: &str, lines: &[&str], link: Option<(&str, &str)>) {
    ui.label(RichText::new(title).font(theme::semibold(14.0)).color(p.ink));
    for line in lines {
        ui.add(egui::Label::new(RichText::new(*line).size(12.5).color(p.body)).wrap());
    }
    if let Some((text, url)) = link {
        ui.hyperlink_to(text, url);
    }
    ui.add_space(theme::SPACE_S);
}

fn reveal_upscalers(dir: &std::path::Path) {
    for f in ["fsr", "streamline", "xess"] {
        let _ = std::fs::create_dir_all(dir.join(f));
    }
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
}

pub fn audio(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Audio", "Output and mixing for the native audio backend.");
    widgets::pending_note(
        ui,
        &p,
        "The game opens its audio renderer and runs on its 5 ms audio clock, but voices are not mixed to your speakers yet. Master volume is written to the engine's controls.ini before every start; the three buses wait for mixing.",
    );
    let a = &mut app.config.audio;
    widgets::section(ui, &p, "Output", |ui| {
        widgets::setting_row(ui, &p, "Device", "The output device follows the Windows default until device selection lands.", |ui| {
            ui.label(RichText::new(&a.device).color(p.ink));
        });
        widgets::setting_row(ui, &p, "Buffer", "Larger buffers resist crackling; smaller ones reduce latency.", |ui| {
            let mut ms = a.buffer_ms as f32;
            if widgets::Slider::new(&mut ms, 10.0..=200.0).suffix(" ms").show(ui, &p).changed() {
                a.buffer_ms = ms.round() as u32;
            }
        });
    });
    widgets::section(ui, &p, "Mix", |ui| {
        for (label, value) in [("Master", &mut a.master), ("Music", &mut a.music), ("Effects", &mut a.effects), ("Voice", &mut a.voice)] {
            let help = if label == "Master" { "The volume play mode mixes at, from the game's next start." } else { "" };
            widgets::setting_row(ui, &p, label, help, |ui| {
                let mut pct = (*value * 100.0).round();
                if widgets::Slider::new(&mut pct, 0.0..=100.0).suffix(" %").show(ui, &p).changed() {
                    *value = pct / 100.0;
                }
            });
        }
    });

    if let Some(l) = crate::engine::live() {
        // Only the volume is this page's business; the rest of the panel lives
        // on Graphics.
        let desired: std::collections::BTreeMap<String, String> =
            [("audio.volume".to_string(), format!("{:.2}", a.master.clamp(0.0, 1.0)))].into_iter().collect();
        let (pending, kept) = waiting(&desired, &l.values);
        let summary = format!("Master {:.0} %", l.volume * 100.0);
        widgets::section_drawer(ui, &p, "audio-live", "What the game will start with", &summary, false, |ui| {
            widgets::kv(ui, &p, "Master volume", &format!("{:.0} %", l.volume * 100.0));
            waiting_line(ui, &p, pending, kept);
        });
    }
}

/// Which of `desired`'s settings are still waiting for the launcher to write
/// them, and which the game's own menu has changed since — those the launcher
/// now leaves alone.
fn waiting(
    desired: &std::collections::BTreeMap<String, String>,
    on_file: &std::collections::BTreeMap<String, String>,
) -> (Vec<&'static str>, Vec<&'static str>) {
    let applied = crate::engine::applied_values();
    let mut pending = Vec::new();
    let mut kept = Vec::new();
    for (id, value) in desired {
        let label = match id.as_str() {
            "upscale.mode" => "upscaling",
            "fefw.output" | "display.width" | "display.height" => "resolution",
            "display.window_mode" => "display mode",
            "display.vsync" => "vsync",
            "display.aspect" => "aspect ratio",
            "display.fps_limit" => "frame-rate limit",
            "display.render_width" | "display.render_height" | "display.render_scale" => "render scale",
            "audio.volume" => "master volume",
            // The DLL paths are found, not chosen.
            _ => continue,
        };
        if matches!(on_file.get(id), Some(v) if v == value) {
            continue;   // the file already says this
        }
        if applied.get(id) == Some(value) {
            if matches!(on_file.get(id), Some(v) if v != value) {
                kept.push(label);
            }
        } else {
            pending.push(label);
        }
    }
    pending.sort_unstable();
    pending.dedup();
    kept.sort_unstable();
    kept.dedup();
    (pending, kept)
}

fn waiting_line(ui: &mut egui::Ui, p: &theme::Palette, pending: Vec<&'static str>, kept: Vec<&'static str>) {
    ui.add_space(6.0);
    if pending.is_empty() && kept.is_empty() {
        ui.label(RichText::new("The game's files match this page.").size(12.5).color(p.muted));
    }
    if !pending.is_empty() {
        ui.label(
            RichText::new(format!("Waiting for the game's next start: {}.", pending.join(", ")))
                .size(12.5)
                .color(p.muted),
        );
    }
    if !kept.is_empty() {
        ui.label(
            RichText::new(format!(
                "Changed in the game's own menu, so the launcher leaves it alone: {}.",
                kept.join(", ")
            ))
            .size(12.5)
            .color(p.muted),
        );
    }
}
