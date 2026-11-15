//! Graphics and audio settings.
//!
//! The renderer and the audio backend do not exist yet, so every option here
//! is stored for the day they land and none of them claims to do anything
//! today. Each page says so at the top.

use crate::theme;
use crate::widgets;
use crate::App;
use eframe::egui::{self, RichText};

/// Every upscaler the engine can run itself. `engine::upscale_mode` maps these
/// labels to the modes in its ini.
const UPSCALERS: &[&str] = &["Off", "FSR 1.0", "FSR 3.1", "NVIDIA Image Scaling", "DLSS", "DLAA"];

/// The engine's `[upscale] sharpness` steps: Off, Low, its own 0.8, and 1.0.
const SHARPENING: &[&str] = &["Off", "Low", "Default", "High"];

/// What the selected upscaler needs, and whether it is there: FSR 3.1 and DLSS
/// need DLLs AMD and NVIDIA do not let us ship.
fn upscaler_help(current: &str, sdk: &crate::engine::Sdk) -> String {
    match current {
        "FSR 1.0" => "AMD's spatial upscaler: scales and sharpens on any GPU, with no motion vectors.".into(),
        "FSR 3.1" => match &sdk.fsr3 {
            Some(dir) => format!("AMD's temporal upscaler, using the FidelityFX DLLs in {}. Applies at the game's next start.", dir.display()),
            None => "AMD's temporal upscaler. Put amd_fidelityfx_loader_vk.dll and amd_fidelityfx_upscaler_vk.dll in upscalers\\fsr3 next to the game; without them the engine logs why and leaves the picture unscaled.".into(),
        },
        "NVIDIA Image Scaling" => "NVIDIA's spatial upscaler: any GPU, up to 2x per side, FSR 1 beyond that.".into(),
        "DLSS" | "DLAA" => match &sdk.dlss {
            Some(dir) => format!("NVIDIA's temporal upscaler through Streamline, in {}. Applies at the game's next start.", dir.display()),
            None => "NVIDIA's temporal upscaler. Needs NVIDIA's four Streamline DLLs in upscalers\\streamline next to the game, on an RTX GPU; without them the engine keeps its own upscaler.".into(),
        },
        _ => "The picture is scaled by the presentation path alone.".into(),
    }
}

fn combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[&str]) {
    egui::ComboBox::from_id_salt(id).selected_text(value.clone()).width(200.0).show_ui(ui, |ui| {
        for o in options {
            ui.selectable_value(value, o.to_string(), *o);
        }
    });
}

const UPSCALE_QUALITIES: &[&str] = &["Native", "Quality", "Balanced", "Performance", "Ultra performance"];
const FRAME_GENERATION: &[&str] = &["Off", "DLSS Frame Generation", "FSR 3 Frame Generation"];
const DLSS_UPSCALERS: &[&str] = &["Automatic", "TAAU", "DLSS", "FSR 3", "Bilinear"];
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
    let g = &mut app.config.graphics;
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

    widgets::section(ui, &p, "Shader compilation (CPU)", |ui| {
        ui.label(
            RichText::new(
                "While a new area loads, the engine compiles the shader parts and whole pipelines its cache has \
                 never seen, on background threads. The defaults keep them below normal priority in Windows' \
                 efficiency mode; on a laptop that still raises clocks and fans, so the count and the priority \
                 are here.",
            )
            .size(12.5)
            .color(p.muted),
        );
        widgets::setting_row(
            ui,
            &p,
            "Compile threads",
            "Background threads that compile shaders while playing. Automatic leaves the engine's own count, min(4, cpus / 6), which on a 24-thread laptop is 4. Applies at the next start.",
            |ui| {
                combo_num(ui, "cthreads", &mut g.compile_threads, COMPILE_THREADS);
            },
        );
        widgets::setting_row(
            ui,
            &p,
            "Compile priority",
            "Thread priority for those compiles. Idle keeps them off the fast cores entirely; Normal brings back the stutter-free first draw but competes with the game.",
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

    // The files the game actually reads, so a change the launcher has written
    // for the next start is visibly waiting rather than silently absent.
    if let Some(l) = crate::engine::live() {
        let desired = crate::engine::desired_values(&g, &app.config.audio);
        let (pending, kept) = waiting(&desired, &l.values);
        widgets::section(ui, &p, "What the game will start with", |ui| {
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
        widgets::section(ui, &p, "What the game will start with", |ui| {
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
