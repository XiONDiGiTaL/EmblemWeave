//! Graphics and audio settings for the integrated engine plus the optional
//! ReShade Vulkan layer. Values are written into the engine's ini files before
//! every start; ReShade itself stays an external, user-installed tool.

use crate::config::{eq_preset_bands, eq_preset_label, Graphics, InterfaceMode, EQ_PRESETS};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self};

/// Every upscaler the engine can run itself. `engine::upscale_mode` maps these
/// labels to the modes in its ini.
const UPSCALERS: &[&str] = &["Off", "FSR 1.0", "FSR 4.1.2", "FSR 3.1", "NVIDIA Image Scaling", "MAKO (spatial)", "DLSS", "DLAA", "XeSS"];

/// The Standard-mode upscalers: the ones that need no vendor DLLs. A value the
/// player already selected in Advanced mode stays visible in the combo box.
const BASIC_UPSCALERS: &[&str] = &["Off", "FSR 1.0", "MAKO (spatial)"];

/// The engine's `[upscale] sharpness` steps: Off, Low, its own 0.8, and 1.0.
const SHARPENING: &[&str] = &["Off", "Low", "Default", "High"];

/// The engine's two AV1 decoders for cutscene movies.
const VIDEO_DECODERS: &[&str] = &["dav1d (CPU)", "FFmpeg (libavcodec)"];

/// The visible warning when FSR 4.1.2 is selected but cannot run: the launcher
/// downgrades the value it writes, so say which mode will actually start.
fn fsr4_fallback_note(current: &str, sdk: &crate::engine::Sdk) -> Option<&'static str> {
    if current != "FSR 4.1.2" || sdk.fsr4() {
        return None;
    }
    Some(if sdk.fsr3.is_some() {
        "FSR 4.1.2 is selected, but the FidelityFX SDK 2.4 loader pair (amd_fidelityfx_loader_vk.dll and amd_fidelityfx_upscaler_vk.dll) is not in upscalers\\fsr; only the v1.1.4 amd_fidelityfx_vk.dll (FSR 3.1) is available, so the game will run FSR 3.1."
    } else {
        "FSR 4.1.2 is selected, but no usable FidelityFX layout was found in upscalers\\fsr, so the game will run FSR 1.0. Put the SDK 2.4 loader pair there for FSR 4.1.2, or amd_fidelityfx_vk.dll (v1.1.4) for FSR 3.1."
    })
}

/// What the selected upscaler needs, and whether it is there: FSR 4.1.2 and
/// FSR 3.1 need DLLs AMD does not let us ship, and DLSS needs NVIDIA's.
fn upscaler_help(current: &str, sdk: &crate::engine::Sdk) -> String {
    match current {
        "FSR 1.0" => "AMD's spatial upscaler: scales and sharpens on any GPU, with no motion vectors.".into(),
        "FSR 4.1.2" if sdk.fsr4() => match &sdk.fsr3 {
            Some(dir) => format!("AMD's ML temporal upscaler through the FidelityFX SDK 2.4 Vulkan provider in {}. It selects FSR 4.1.2 where the driver offers it and FSR 3.1 otherwise. Applies at the game's next start.", dir.display()),
            None => "AMD's ML temporal upscaler. Put amd_fidelityfx_loader_vk.dll and amd_fidelityfx_upscaler_vk.dll from FidelityFX SDK 2.4 in upscalers\\fsr in the play folder; without them the engine logs why and leaves the picture unscaled.".into(),
        },
        "FSR 4.1.2" => "FSR 4.1.2 needs the FidelityFX SDK 2.4 loader pair (amd_fidelityfx_loader_vk.dll and amd_fidelityfx_upscaler_vk.dll). The single amd_fidelityfx_vk.dll is FSR 3.1 only; put the pair in upscalers\\fsr in the play folder.".into(),
        "FSR 3.1" => match &sdk.fsr3 {
            Some(dir) => format!("AMD's temporal upscaler, using the FidelityFX DLLs in {}. Applies at the game's next start.", dir.display()),
            None => "AMD's temporal upscaler. Put amd_fidelityfx_loader_vk.dll and amd_fidelityfx_upscaler_vk.dll in upscalers\\fsr in the play folder; without them the engine logs why and leaves the picture unscaled.".into(),
        },
        "NVIDIA Image Scaling" => "NVIDIA's spatial upscaler: any GPU, up to 2x per side, FSR 1 beyond that.".into(),
        "MAKO (spatial)" => "A single-pass sharpened-cubic spatial upscaler adapted from the MAKO layer: any GPU, any scale, and one dispatch instead of FSR 1's two.".into(),
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
    egui::ComboBox::from_id_salt(id)
        .selected_text(crate::i18n::tr(value))
        .width(200.0)
        .show_ui(ui, |ui| {
            // A value the engine wrote (an aspect of 1.777…, a newer upscaler name) is
            // kept as the current choice instead of disappearing from the list.
            let current = value.clone();
            if !current.is_empty() && !options.iter().any(|o| *o == current) {
                ui.selectable_value(value, current.clone(), crate::i18n::tr(&current));
                ui.separator();
            }
            for o in options {
                ui.selectable_value(value, o.to_string(), crate::i18n::tr(o));
            }
        });
}

const UPSCALE_QUALITIES: &[&str] = &["Native", "Quality", "Balanced", "Performance", "Ultra performance"];
const FRAME_GENERATION: &[&str] = &["Off", "DLSS Frame Generation", "FSR 3 Frame Generation"];
const DLSS_UPSCALERS: &[&str] = &["Automatic", "TAAU", "DLSS", "FSR 4 / 3 (FidelityFX)", "XeSS", "Bilinear"];
const ASTC_QUALITIES: &[&str] = &["High", "Fast", "Uncompressed"];
const GPU_CHOICES: &[&str] = &["Automatic", "Discrete", "Integrated"];
const PRESENT_GUARDS: &[&str] = &["Automatic", "Always on", "Off"];
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
    if app.config.interface_mode == InterfaceMode::Standard {
        standard_graphics(app, ui);
    } else {
        advanced_graphics(app, ui);
    }
}

/// The Standard-mode graphics page: the choices that change how the game looks
/// and runs, without compiler, Vulkan, upscaler-DLL or cache controls.
fn standard_graphics(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(
        ui,
        &p,
        "Graphics",
        "A few practical presets, with the display and picture controls most players need.",
    );
    widgets::pending_note(
        ui,
        &p,
        "Presets change only the picture and frame cap; your resolution, display mode and saves stay as they are. Advanced mode adds upscaler DLLs, the movie decoder, frame generation, ReShade and shader-compile tuning.",
    );
    let mut chosen = None;
    widgets::section(ui, &p, "Presets", |ui| {
        ui.label(
            crate::i18n::text("Pick a starting point, then adjust anything below. Changing a preset never changes the game installation.")
                .size(12.0)
                .color(p.body),
        );
        ui.add_space(7.0);
        ui.horizontal_wrapped(|ui| {
            for preset in Preset::ALL {
                if widgets::button(ui, &p, preset.label(), true)
                    .on_hover_text(preset.detail())
                    .clicked()
                {
                    chosen = Some(preset);
                }
            }
        });
        ui.add_space(6.0);
        let g = &app.config.graphics;
        ui.label(
            crate::i18n::text(format!(
                "Current: {:.2}× render scale · {} · {} fps cap · {} shading",
                g.render_scale,
                g.upscaling,
                g.frame_limit,
                if g.shader_precompile { "precompiled" } else { "on demand" },
            ))
            .size(10.8)
            .color(p.muted),
        );
    });
    if let Some(preset) = chosen {
        preset.apply(&mut app.config.graphics);
        app.note(format!("{} graphics preset applied", preset.label()), Tone::Neutral);
    }
    let g = &mut app.config.graphics;
    widgets::section(ui, &p, "Display", |ui| {
        widgets::setting_row(ui, &p, "Resolution", "The picture's size and the window's, taken at the game's next start. Native display keeps the game's own console size.", |ui| {
            combo(ui, "std-res", &mut g.resolution, &["Native display", "1280 × 720", "1920 × 1080", "2560 × 1440", "3840 × 2160"]);
        });
        widgets::setting_row(ui, &p, "Display mode", "Windowed, borderless or exclusive, from the game's next start.", |ui| {
            combo(ui, "std-mode", &mut g.display_mode, &["Windowed", "Borderless window", "Exclusive fullscreen"]);
        });
        widgets::setting_row(ui, &p, "VSync", "Synchronise presentation with the display. Applies at the next start.", |ui| {
            widgets::toggle(ui, &p, &mut g.vsync);
        });
        widgets::setting_row(
            ui,
            &p,
            "Present mode safeguard",
            "Avoids the immediate present path, which can reset Intel's Windows driver (issue #72). Automatic guards Intel graphics that share system memory; Always on guards every GPU; Off allows immediate everywhere. Applies at the next start.",
            |ui| {
                combo(ui, "std-guard", &mut g.present_guard, PRESENT_GUARDS);
            },
        );
        widgets::setting_row(ui, &p, "Frame-rate limit", "Caps presentation only: the game's simulation keeps its own tick. The engine reads it at the next start.", |ui| {
            let mut fps = g.frame_limit as f32;
            if widgets::Slider::new(&mut fps, 30.0..=240.0).suffix(" fps").show(ui, &p).changed() {
                g.frame_limit = fps.round() as u32;
            }
        });
    });
    widgets::section(ui, &p, "Picture", |ui| {
        widgets::setting_row(ui, &p, "Render scale", "Internal resolution relative to the output; the upscaler fills the rest. Applies at the next start.", |ui| {
            widgets::Slider::new(&mut g.render_scale, 0.5..=2.0).fixed_decimals(2).suffix("×").show(ui, &p);
        });
        widgets::setting_row(ui, &p, "Upscaling", "FSR 1.0 works on any GPU. Off keeps the native render scale.", |ui| {
            combo(ui, "std-upscale", &mut g.upscaling, BASIC_UPSCALERS);
        });
        if g.upscaling == "FSR 4.1.2" {
            let sdk = crate::engine::sdk();
            if let Some(note) = fsr4_fallback_note(&g.upscaling, &sdk) {
                widgets::pending_note(ui, &p, note);
            }
        }
        widgets::setting_row(ui, &p, "Sharpening", "How crisp the final picture is. Default is the engine's own setting.", |ui| {
            combo(ui, "std-sharpening", &mut g.sharpening, SHARPENING);
        });
        widgets::setting_row(ui, &p, "Precompile shaders", "Translate the game's shaders before play to reduce first-area stutter.", |ui| {
            widgets::toggle(ui, &p, &mut g.shader_precompile);
        });
    });
    ui.add_space(5.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(crate::i18n::text("Need frame generation, a specific GPU, ReShade or shader-compilation tuning?").size(10.8).color(p.muted));
        if widgets::button(ui, &p, "Show advanced graphics", true).clicked() {
            app.config.interface_mode = InterfaceMode::Advanced;
            app.note("Advanced graphics controls are now visible", Tone::Neutral);
        }
    });
}

/// How a stored GPU value is shown in the dropdown. A category keeps its name;
/// a device name gets the adapter type and video memory the engine logged.
fn gpu_display(value: &str, devices: &[crate::gpu::Device]) -> String {
    match value.trim() {
        "" | "Automatic" => "Automatic (recommended)".into(),
        "Discrete" => "Discrete (most video memory)".into(),
        "Integrated" => "Integrated".into(),
        name => devices
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(name))
            .map(|d| {
                if d.vram_mb > 0 {
                    format!("{} ({}, {:.1} GB)", d.name, d.kind, d.vram_mb as f64 / 1024.0)
                } else {
                    format!("{} ({})", d.name, d.kind)
                }
            })
            .unwrap_or_else(|| name.to_string()),
    }
}

/// The GPU picker: the engine's categories plus every Vulkan 1.3 device the
/// loader can see. Selecting a device stores its full name, which the engine
/// matches case-insensitively and independently of enumeration order.
fn gpu_combo(ui: &mut egui::Ui, value: &mut String, devices: &[crate::gpu::Device]) {
    egui::ComboBox::from_id_salt("gpu")
        .selected_text(crate::i18n::tr(&gpu_display(value, devices)))
        .width(280.0)
        .show_ui(ui, |ui| {
            for choice in GPU_CHOICES {
                ui.selectable_value(value, (*choice).to_string(), gpu_display(choice, devices));
            }
            if !devices.is_empty() {
                ui.separator();
                for d in devices {
                    ui.selectable_value(value, d.name.clone(), gpu_display(&d.name, devices));
                }
            }
            if !value.is_empty()
                && !GPU_CHOICES.contains(&value.as_str())
                && !devices.iter().any(|d| d.name.eq_ignore_ascii_case(value))
            {
                // A hand-written name or a device that is no longer installed:
                // keep it visible so selecting another adapter is deliberate.
                let current = value.clone();
                ui.separator();
                ui.selectable_value(value, current.clone(), format!("{} (not detected)", current));
            }
        });
}

/// The Standard graphics presets: labels, hover text and values in one place.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Preset {
    Performance,
    Balanced,
    Quality,
}

impl Preset {
    const ALL: [Preset; 3] = [Preset::Performance, Preset::Balanced, Preset::Quality];

    fn label(self) -> &'static str {
        match self {
            Preset::Performance => "Performance",
            Preset::Balanced => "Balanced",
            Preset::Quality => "Quality",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Preset::Performance => "Lower internal resolution with FSR 1.0 and a 60 fps cap",
            Preset::Balanced => "Native render scale, no upscaler, 60 fps cap",
            Preset::Quality => "Native render scale with sharper output",
        }
    }

    /// Apply the picture and frame-cap values this preset owns. The resolution, the display mode and every Advanced-only
    /// field (`upscale_quality`, `frame_gen`, `re_shade`, the compiler controls and the rest) stay exactly as they were.
    fn apply(self, g: &mut Graphics) {
        let (render_scale, upscaling, sharpening) = match self {
            Preset::Performance => (0.75, "FSR 1.0", "Default"),
            Preset::Balanced => (1.0, "Off", "Default"),
            Preset::Quality => (1.0, "Off", "High"),
        };
        g.render_scale = render_scale;
        g.upscaling = upscaling.into();
        g.sharpening = sharpening.into();
        g.frame_limit = 60;
        g.vsync = true;
        g.shader_precompile = true;
    }
}

fn advanced_graphics(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    let sdk = crate::engine::sdk();
    widgets::page_header(ui, &p, "Graphics", "Display and rendering options for the game's integrated engine, plus the optional ReShade layer.");
    widgets::pending_note(
        ui,
        &p,
        "The game renders through the host Vulkan NVN layer. Everything on this page except Quality and Precompile shaders is written \
         into the engine's ini files before every start. FSR 4.1.2 / 3.1 and FSR 3 frame generation need AMD's FidelityFX DLLs; ReShade is \
         optional and stays user-installed.",
    );
    memory_profile(app, ui);
    upscaler_files(app, ui);
    video_hw_settings(app, ui, &sdk);
    reshade_settings(app, ui);
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
        widgets::setting_row(
            ui,
            &p,
            "Present mode safeguard",
            "Avoids the immediate present path, which can reset Intel's Windows driver (issue #72). Automatic guards Intel graphics that share system memory; Always on guards every GPU; Off allows immediate everywhere. Applies at the next start.",
            |ui| {
                combo(ui, "guard", &mut g.present_guard, PRESENT_GUARDS);
            },
        );
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
        let help = upscaler_help(&g.upscaling, &sdk);
        widgets::setting_row(ui, &p, "Upscaling", &help, |ui| {
            combo(ui, "upscale", &mut g.upscaling, UPSCALERS);
        });
        if let Some(note) = fsr4_fallback_note(&g.upscaling, &sdk) {
            widgets::pending_note(ui, &p, note);
        }
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
        let decoder_help = match &sdk.ffmpeg {
            Some(dir) => format!("AV1 decoder for cutscene movies. dav1d is the engine's built-in CPU decoder. FFmpeg loads the libavcodec DLLs found in {}; its hardware option is configured in the optional D3D11VA card above. Applies at the next start.", dir.display()),
            None => "AV1 decoder for cutscene movies. dav1d is the engine's built-in CPU decoder and needs nothing. FFmpeg uses the avcodec-62.dll, avutil-60.dll and swresample-6.dll in upscalers\\ffmpeg; if that folder is missing, use Hardware video decode above or the game uses dav1d. Applies at the next start.".into(),
        };
        widgets::setting_row(ui, &p, "Video decoder", &decoder_help, |ui| {
            combo(ui, "vdec", &mut g.video_decoder, VIDEO_DECODERS);
        });
        if g.video_decoder == "FFmpeg (libavcodec)" && sdk.ffmpeg.is_none() {
            widgets::pending_note(ui, &p, "FFmpeg is selected, but avcodec-62.dll, avutil-60.dll and swresample-6.dll were not found in upscalers\\ffmpeg, so the game will use its built-in dav1d instead. Download the runtime under Hardware video decode, or put a matching FFmpeg 8.x shared build there.");
        }
        let fsr4 = sdk.fsr4();
        let framegen = sdk.fsr_framegen();
        let preset_ready = sdk.fsr3.is_some();
        let preset_name = if !preset_ready {
            "FSR preset"
        } else if fsr4 && framegen {
            "FSR 4 + frame generation"
        } else if fsr4 {
            "FSR 4 upscaling"
        } else {
            "FSR 3 + frame generation"
        };
        let preset_description = if fsr4 && framegen {
            "One press for the AMD path: FSR 4.1.2, FSR 3 frame generation, the FidelityFX DLSS replacement and Vsync off. FSR 4.1.2 is verified on an RX 7900 XT; the 60 → 120 fps result was measured with FSR 3.1 frame generation."
        } else if fsr4 {
            "One press for the validated AMD path: FSR 4.1.2 upscaling, the FidelityFX DLSS replacement and Vsync off. Add amd_fidelityfx_vk.dll (FidelityFX SDK v1.1.4) for FSR 3 frame generation."
        } else if framegen {
            "One press for the validated AMD path: FSR 3.1, FSR 3 frame generation, the FidelityFX DLSS replacement and Vsync off. On an RX 7900 XT at 2560 × 1440 this presented 120 fps from 60 rendered fps."
        } else {
            "Needs AMD's FidelityFX DLLs in upscalers\\fsr: the SDK 2.4 loader pair for FSR 4.1.2, or amd_fidelityfx_vk.dll (v1.1.4) for FSR 3.1."
        };
        let preset_help = if !preset_ready {
            "Needs amd_fidelityfx_loader_vk.dll and amd_fidelityfx_upscaler_vk.dll (FSR SDK 2.4), or amd_fidelityfx_vk.dll (v1.1.4), in upscalers\\fsr."
        } else if fsr4 && framegen {
            "Sets FSR 4.1.2, FSR 3 Frame Generation, FSR 4 / 3 as the DLSS replacement, and Vsync off. The next start uses it."
        } else if fsr4 {
            "Sets FSR 4.1.2, FSR 4 / 3 as the DLSS replacement, and Vsync off. FSR 3 Frame Generation also needs amd_fidelityfx_vk.dll (FidelityFX SDK v1.1.4)."
        } else {
            "Sets FSR 3.1, FSR 3 Frame Generation, FSR 4 / 3 as the DLSS replacement, and Vsync off. The next start uses it."
        };
        widgets::setting_row(ui, &p, "AMD preset", preset_description, |ui| {
            if widgets::button(ui, &p, preset_name, preset_ready)
                .on_hover_text(preset_help)
                .clicked()
            {
                g.upscaling = if fsr4 { "FSR 4.1.2" } else { "FSR 3.1" }.into();
                g.frame_gen = if framegen { "FSR 3 Frame Generation" } else { "Off" }.into();
                g.dlss_upscaler = "FSR 4 / 3 (FidelityFX)".into();
                g.vsync = false;
            }
        });
        widgets::setting_row(
            ui,
            &p,
            "Frame generation",
            "Generates the frames between two rendered ones. FSR 3 is implemented and validated on the RX 7900 XT; it needs amd_fidelityfx_vk.dll (FidelityFX SDK v1.1.4) and Vsync off. DLSS Frame Generation needs Windows' hardware-accelerated GPU scheduling, an RTX 40 or newer card and the Streamline files.",
            |ui| {
                combo(ui, "fg", &mut g.frame_gen, FRAME_GENERATION);
            },
        );
        widgets::setting_row(
            ui,
            &p,
            "DLSS replacement",
            "What renders the frames the game asks its console's DLSS hardware for: the engine's own temporal upscaler, NVIDIA's, AMD's FidelityFX (FSR 4.1.2 or 3.1, whichever the Upscaling setting selects), or a plain bilinear stretch. Automatic uses whichever DLLs are found.",
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
            "Which Vulkan device the engine opens. Automatic takes the discrete GPU with the most video memory. The list below shows the Vulkan 1.3 devices this PC reports; choosing one writes its full name to [gfx] gpu.",
            |ui| {
                // The combo and the line about the devices share the control column: the line sits under
                // the combo and wraps to that width. As a sibling in setting_row's right-to-left row it
                // was laid out beside the 280 px combo in whatever width was left, which could be nothing,
                // so it ran left over the description instead of wrapping (issue #92).
                let width = ui.available_width();
                ui.vertical(|ui| {
                    ui.set_width(width);
                    if let Some(devices) = crate::gpu::cached() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            gpu_combo(ui, &mut g.gpu, devices);
                        });
                        if devices.is_empty() {
                            ui.label(
                                crate::i18n::text("No Vulkan 1.3 device was reported; Automatic, Discrete and Integrated still work through the engine's own rule.")
                                    .size(10.8)
                                    .color(p.muted),
                            );
                        } else {
                            ui.label(
                                crate::i18n::text(format!("Detected: {}", devices.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(" · ")))
                                    .size(10.8)
                                    .color(p.muted),
                            );
                        }
                    } else {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            gpu_combo(ui, &mut g.gpu, &[]);
                        });
                        ui.label(crate::i18n::text("Looking for Vulkan devices...").size(10.8).color(p.muted));
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
                    }
                });
            },
        );
        widgets::setting_row(ui, &p, "Renderer", "", |ui| {
            ui.label(crate::i18n::text("Vulkan").font(theme::semibold(14.0)).color(p.ink));
        });
        ui.add_space(10.0);
        ui.label(
            crate::i18n::text(
                "FSR 4.1.2 now runs through the engine's native Vulkan FidelityFX path on supported Radeon GPUs (verified on the RX 7900 XT with Adrenalin 26.10.1). The DX12 shader-override DLLs and OptiScaler still cannot drive this Vulkan renderer, so they are not used; FSR 3.1 remains available as the pinned fallback.",
            )
            .size(12.5)
            .color(p.muted),
        );
    });

    let compile_summary = format!("{} threads · {}", g.compile_threads, g.compile_priority.to_lowercase());
    widgets::section_drawer(ui, &p, "shader-compilation", "Shader compilation (CPU)", &compile_summary, false, |ui| {
        ui.label(
            crate::i18n::text(
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
                if widgets::button(ui, &p, "Quiet", true).on_hover_text(crate::i18n::tr("1 thread, idle priority, efficiency mode")).clicked() {
                    g.compile_threads = 1;
                    g.compile_priority = "Idle".into();
                    g.compile_eco = true;
                }
                if widgets::button(ui, &p, "Balanced", true).on_hover_text(crate::i18n::tr("2 threads, idle priority, efficiency mode")).clicked() {
                    g.compile_threads = 2;
                    g.compile_priority = "Idle".into();
                    g.compile_eco = true;
                }
                if widgets::button(ui, &p, "Fast", true).on_hover_text(crate::i18n::tr("4 threads, below normal priority, efficiency mode off")).clicked() {
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

    // Debug section: console visibility for troubleshooting.
    widgets::section(ui, &p, "Debug", |ui| {
        widgets::setting_row(
            ui,
            &p,
            "Show debug console",
            "Displays the runtime's console window (fefw.exe) for troubleshooting. Off by default to avoid an unwanted console window; logging to fefw.log continues regardless.",
            |ui| {
                widgets::toggle(ui, &p, &mut app.config.show_debug_console);
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
            ui.label(crate::i18n::text(facts.join("  ·  ")).size(12.5).color(p.body));
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
            ui.label(crate::i18n::text(text).size(12.5).color(p.muted));
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
            ui.label(crate::i18n::text("No memory warning for the facts available now.").size(12.5).color(p.ok));
        } else {
            ui.label(
                crate::i18n::text("Windows memory facts and the engine's GPU log are not available yet. The profile can still be applied explicitly; no warning is inferred from missing data.")
                    .size(12.5)
                    .color(p.muted),
            );
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if active {
                if widgets::button(ui, &p, "Restore settings from before Low Memory", true)
                    .on_hover_text(crate::i18n::tr("Puts back every graphics setting from before the first application. Audio, controls, saves and mods are untouched."))
                    .clicked()
                {
                    restore = true;
                }
                widgets::pill(ui, &p, "applied", Tone::Ok);
            } else if widgets::button(ui, &p, "Apply Low Memory", true)
                .on_hover_text(crate::i18n::tr("Changes only the graphics settings listed below. The previous values are kept for one-press restore."))
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
            ui.label(crate::i18n::text("The current graphics settings already match the profile.").size(12.5).color(p.muted));
        } else {
            for line in preview {
                ui.label(crate::i18n::text(format!("• {line}")).size(12.5).color(p.body));
            }
        }
        ui.add_space(6.0);
        ui.label(
            crate::i18n::text("The profile does not change audio, controls, saves, mods, the game dump or the engine's automatic texture-residency and BC1 choices. The engine still decides those from the GPU it finds. The next Play writes the settings; a build is not needed.")
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

/// The optional hardware movie path. It is deliberately a drawer of its own:
/// the user can see whether this GPU reports AV1 decode, install or repair the
/// pinned runtime, and make D3D11VA this system's choice without changing the
/// default for anyone else.
fn video_hw_settings(app: &mut App, ui: &mut egui::Ui, sdk: &crate::engine::Sdk) {
    let p = app.config.theme.palette();
    let installed = sdk.ffmpeg.is_some();
    let check = crate::video_hw::check(&app.config.graphics.gpu, crate::gpu::cached());
    let enabled = app.config.graphics.video_decoder == "FFmpeg (libavcodec)"
        && app.config.graphics.video_hwaccel;
    let progress = app.running(crate::jobs::JobKind::VideoRuntime).map(|job| {
        job.snapshot(|p| (p.fraction, p.message.clone()))
    });
    let summary = if !installed {
        "runtime not installed".to_string()
    } else if enabled {
        "enabled for this system".to_string()
    } else if check.supported() {
        format!("available · {}", check.device().unwrap_or("GPU"))
    } else {
        "installed · off".to_string()
    };

    widgets::section_drawer(
        ui,
        &p,
        "video-hw",
        "Hardware video decode (D3D11VA, optional)",
        &summary,
        false,
        |ui| {
            ui.label(
                crate::i18n::text(
                    "Uses the GPU's AV1 decoder through FFmpeg's D3D11VA path and copies NV12 frames back to the engine. \
                     It stays off unless you enable it, so dav1d remains the default for everyone else. The check below \
                     reads the Vulkan driver's AV1 decode capability; if the driver refuses a movie, the engine falls \
                     back to its built-in dav1d.",
                )
                .size(12.5)
                .color(p.body),
            );

            let mut on = enabled;
            let mut changed = false;
            if installed && !matches!(check, crate::video_hw::Check::Checking) {
                widgets::setting_row(
                    ui,
                    &p,
                    "Use D3D11VA for this system",
                    "Sets Video decoder to FFmpeg and hardware acceleration on. Turning it off restores the built-in dav1d. This is stored in your own settings and written before the next Play.",
                    |ui| {
                        if widgets::toggle(ui, &p, &mut on).changed() {
                            changed = true;
                        }
                    },
                );
                if changed {
                    let g = &mut app.config.graphics;
                    if on {
                        g.video_decoder = "FFmpeg (libavcodec)".into();
                        g.video_hwaccel = true;
                    } else {
                        g.video_decoder = "dav1d (CPU)".into();
                        g.video_hwaccel = false;
                    }
                    app.notify(
                        if on {
                            "D3D11VA enabled for this system; the next Play writes it"
                        } else {
                            "D3D11VA disabled; the built-in dav1d is selected again"
                        },
                        Tone::Ok,
                    );
                }
            }

            let (tone, detail) = match &check {
                crate::video_hw::Check::Checking => (
                    Tone::Neutral,
                    "Checking this GPU's AV1 decode support…".to_string(),
                ),
                crate::video_hw::Check::NoDevice => (
                    Tone::Warn,
                    "No Vulkan 1.3 GPU was detected, so suitability cannot be checked. You can still download the runtime; the engine falls back to dav1d if the hardware refuses a movie.".to_string(),
                ),
                crate::video_hw::Check::Supported(name) => (
                    Tone::Ok,
                    format!("{name} reports AV1 decode. D3D11VA can use its hardware decoder."),
                ),
                crate::video_hw::Check::Unsupported(name) => (
                    Tone::Warn,
                    format!("{name} does not report AV1 decode. Enabling it is allowed, but the engine will probably fall back to dav1d."),
                ),
            };
            ui.label(crate::i18n::text(detail).size(12.5).color(tone.colour(&p)));

            ui.add_space(theme::SPACE_S);
            ui.horizontal_wrapped(|ui| {
                if installed {
                    if widgets::button(ui, &p, "Reinstall runtime", progress.is_none())
                        .on_hover_text(crate::i18n::tr("Downloads the pinned FFmpeg DLLs again and verifies every SHA-256 before replacing the runtime."))
                        .clicked()
                    {
                        app.start(crate::jobs::JobKind::VideoRuntime);
                    }
                    if let Some(dir) = &sdk.ffmpeg {
                        if widgets::button(ui, &p, "Open runtime folder", true).clicked() {
                            let _ = std::process::Command::new("explorer").arg(dir).spawn();
                        }
                    }
                } else if widgets::primary_button(ui, &p, "Download runtime (about 97 MB)", progress.is_none())
                    .on_hover_text(crate::i18n::tr("Downloads the exact FFmpeg 8.1 runtime from the pinned project commit. Every file is checked against its SHA-256 before it is installed."))
                    .clicked()
                {
                    app.start(crate::jobs::JobKind::VideoRuntime);
                }
            });
            if let Some((fraction, message)) = &progress {
                ui.add(egui::ProgressBar::new(*fraction).text(message.clone()).desired_width(ui.available_width()));
            }
            ui.label(
                crate::i18n::text(if installed {
                    "The runtime is included in this release. Reinstall only if the folder is missing or damaged."
                } else {
                    "The release normally includes this runtime. Downloading repairs a missing or removed upscalers\\ffmpeg folder."
                })
                .size(11.5)
                .color(p.muted),
            );
        },
    );
}

const FSR_SDK_URL: &str = "https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/releases";
const FSR_114_URL: &str = "https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/releases/tag/v1.1.4";
const STREAMLINE_URL: &str = "https://github.com/NVIDIA-RTX/Streamline/releases";
const XESS_URL: &str = "https://github.com/intel/xess/releases";
const RESHADE_URL: &str = "https://reshade.me";

/// ReShade stays an external tool: its licence forbids redistributing the
/// binaries or shader files, so the launcher only detects the install, starts
/// the official setup for this build, and lets the user switch the layer off for
/// this game. The add-on build is the one with full add-on support.
fn reshade_settings(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    let status = crate::engine::reshade_status();
    let mut re_shade = app.config.graphics.re_shade;
    let summary = if !status.installed {
        "not installed".to_string()
    } else if !status.enabled {
        "installed · not enabled for this build".to_string()
    } else if !re_shade {
        "installed · off in EmblemWeave".to_string()
    } else if !status.configured {
        "installed · no ReShade.ini yet".to_string()
    } else {
        "installed · ready".to_string()
    };
    widgets::section_drawer(ui, &p, "reshade", "ReShade (optional)", &summary, false, |ui| {
        ui.label(
            crate::i18n::text(
                "ReShade adds its own post-processing layer and presets. Its Vulkan layer is installed once for Windows, \
                 then ReShadeApps.ini decides which executables load it. The official setup tool writes both; EmblemWeave \
                 opens it for this build's fefw.exe with Vulkan preselected. Nothing from ReShade is bundled.",
            )
            .size(12.5)
            .color(p.body),
        );
        widgets::setting_row(
            ui,
            &p,
            "Load ReShade",
            "On lets the installed ReShade layer apply its presets while playing. Off sets the Vulkan loader's disable variable for this game only; it does not uninstall ReShade or touch other games.",
            |ui| {
                widgets::toggle(ui, &p, &mut re_shade);
            },
        );
        let (tone, detail) = if !status.installed {
            (Tone::Warn, "No ReShade64.dll and manifest were found in %ProgramData%\\ReShade. Run the official setup, choose this build's fefw.exe, then select Vulkan.".to_string())
        } else if !status.enabled {
            (Tone::Warn, "ReShade is installed, but this build's fefw.exe is not in ReShadeApps.ini. Run setup for this executable; it adds the allowlist entry.".to_string())
        } else if !status.configured {
            (Tone::Neutral, "The layer will load; ReShade.ini is created the first time the game runs.".to_string())
        } else if !re_shade {
            (Tone::Neutral, "ReShade is installed and allowed for this executable, but EmblemWeave disables it for the next start.".to_string())
        } else if status.logged {
            (Tone::Ok, "ReShade has loaded for this game before. Press Home in game to open its overlay; effect packs come from the setup's download step, and a preset is what those effects are saved as.".to_string())
        } else {
            (Tone::Ok, "ReShade is installed and allowed for this executable. Press Home in game to open its overlay; EmblemWeave also clears the modifier rule 6.8 adds to that key.".to_string())
        };
        ui.label(crate::i18n::text(detail).size(12.5).color(tone.colour(&p)));
        ui.add_space(theme::SPACE_S);

        // The presets the release ships. Applying one copies the shaders and the preset beside fefw.exe and points
        // ReShade.ini at it, so the look is a single press instead of files a player has to find.
        let presets = crate::engine::ship_presets();
        if !presets.is_empty() {
            let current = crate::engine::current_reshade_preset();
            let choice_id = egui::Id::new("reshade-preset-choice");
            let mut choice: String = ui.data(|d| d.get_temp(choice_id)).unwrap_or_default();
            if choice.is_empty() {
                choice = current.clone().unwrap_or_default();
            }
            ui.horizontal(|ui| {
                ui.label(crate::i18n::text("Preset").font(theme::semibold(11.0)).color(p.ink));
                egui::ComboBox::from_id_salt("reshade-preset")
                    .selected_text(if choice.is_empty() { crate::i18n::tr("Choose…") } else { choice.clone() })
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        for preset in &presets {
                            ui.selectable_value(&mut choice, preset.name.clone(), &preset.name);
                        }
                    });
                if widgets::button(ui, &p, "Apply preset", !choice.is_empty()).clicked() {
                    match crate::engine::apply_reshade_preset(&choice) {
                        Ok(msg) => app.notify(msg, Tone::Ok),
                        Err(e) => app.notify(e, Tone::Bad),
                    }
                }
            });
            ui.data_mut(|d| d.insert_temp(choice_id, choice.clone()));
            let hint = if status.configured {
                "Copied beside fefw.exe. In game, F3 steps to the next preset, F4 to the previous one, and Home opens ReShade's overlay."
            } else {
                "ReShade.ini is written the first time the game runs with ReShade on; the preset can be applied then."
            };
            ui.label(crate::i18n::text(hint).size(11.5).color(p.muted));
        }
        ui.add_space(theme::SPACE_S);
        let game = crate::engine::game_folder();
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, &p, "Set up ReShade…", true)
                .on_hover_text(crate::i18n::tr("Opens the official ReShade setup for this build with Vulkan preselected. Windows asks for administrator approval when it registers the layer."))
                .clicked()
            {
                let picked = crate::engine::find_reshade_setup().or_else(|| {
                    rfd::FileDialog::new()
                        .set_title("Choose ReShade Setup")
                        .add_filter("ReShade Setup", &["exe"])
                        .pick_file()
                });
                if let Some(path) = picked {
                    match crate::engine::launch_reshade_setup(&path) {
                        Ok(()) => app.notify("ReShade setup opened for fefw.exe. Choose Vulkan, effects and any add-ons there; Windows will ask for permission to register the layer.", Tone::Ok),
                        Err(e) => app.notify(e, Tone::Bad),
                    }
                }
            }
            if widgets::button(ui, &p, "ReShade website", true).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(RESHADE_URL));
            }
            if let Some(dir) = game.as_ref().filter(|dir| dir.is_dir()) {
                if widgets::button(ui, &p, "Open game folder", true).clicked() {
                    let _ = std::process::Command::new("explorer").arg(dir).spawn();
                }
                if status.configured {
                    if widgets::button(ui, &p, "Open ReShade.ini", true).clicked() {
                        let _ = std::process::Command::new("explorer")
                            .arg(format!("/select,{}", dir.join("ReShade.ini").display()))
                            .spawn();
                    }
                }
                if status.logged {
                    if widgets::button(ui, &p, "Open ReShade log", true).clicked() {
                        let _ = std::process::Command::new("explorer")
                            .arg(format!("/select,{}", dir.join("ReShade.log").display()))
                            .spawn();
                    }
                }
            }
        });
        ui.add_space(4.0);
        ui.label(
            crate::i18n::text("Use the Add-on build if you want ReShade add-ons: it loads .addon64 files from beside fefw.exe. Presets and shader packs also live there. Depth-based effects depend on the depth buffer the engine exposes, which is not guaranteed.")
                .size(12.0)
                .color(p.muted),
        );
    });
    app.config.graphics.re_shade = re_shade;
}

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
        Some(d) if sdk.fsr4() && !fsr_framegen_missing => {
            format!("FSR 4.1.2 available in {}, with frame generation", d.display())
        }
        Some(d) if sdk.fsr4() => format!(
            "FSR 4.1.2 available in {}; frame generation still needs amd_fidelityfx_vk.dll (FidelityFX SDK v1.1.4)",
            d.display()
        ),
        Some(d) if !fsr_framegen_missing => format!("FSR 3.1 found in {}, with frame generation", d.display()),
        Some(d) => format!(
            "FSR 3.1 found in {}; FSR 4.1.2 needs the SDK 2.4 loader pair, and frame generation still needs amd_fidelityfx_vk.dll (v1.1.4)",
            d.display()
        ),
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
            "FSR 4.1.2 / 3.1",
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
        ui.label(
            crate::i18n::text("AMD Adrenalin can add fefw.exe as a game (Gaming > Add) for driver features such as Fluid Motion Frames 2.1. AFMF works with Vulkan on RX 7000 in borderless fullscreen, needs Adrenalin 25.3.1 or newer, and requires VSync off in both the game and the driver; the launcher cannot toggle a driver-side feature.")
                .size(12.0)
                .color(p.muted),
        );
        ui.add_space(theme::SPACE_S);
        let note = format!(
            "{} AMD, NVIDIA and Intel do not let these travel with EmblemWeave, so the game falls back to its own temporal upscaler until they are there; the F1 menu's Graphics tab then names them under Files found.",
            where_to
        );
        ui.label(crate::i18n::text(note).size(12.5).color(p.body));
        ui.add_space(theme::SPACE_S);
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, &p, "Open the upscalers folder", dir.is_some()).clicked() {
                if let Some(d) = &dir {
                    reveal_upscalers(d);
                }
            }
            if widgets::button(ui, &p, "Look for the files", true)
                .on_hover_text(crate::i18n::tr("Searches beside the launcher, the engine folder, Downloads and the Desktop, one folder deep, and copies what it finds into the upscalers folder."))
                .clicked()
            {
                match crate::engine::adopt_upscaler_files() {
                    Ok(m) => app.notify(m, Tone::Ok),
                    Err(e) => app.notify(e, Tone::Bad),
                }
            }
            if widgets::button(ui, &p, "Setup guide", true)
                .on_hover_text(crate::i18n::tr("Which files to download, what they are called and exactly where they go"))
                .clicked()
            {
                guide_open = true;
            }
        });
    });

    if guide_open {
        let shown = widgets::modal(ui.ctx(), &p, "upscaler-setup-guide", "Upscaler setup guide", "Vendor DLLs stay external because their licences do not allow shipping them.", 700.0, |ui| {
                    ui.label(
                        crate::i18n::text("Vendor DLLs are not shipped with EmblemWeave because their licences do not allow it. Copy the files into the folders below, then press Look for the files. The engine reads them at the next start.")
                            .size(12.5)
                            .color(p.body),
                    );
                    ui.add_space(theme::SPACE_S);
                    guide_block(
                        ui,
                        &p,
                        "FSR 4.1.2 / 3.1 and FSR 3 frame generation",
                        &[
                            "Download the FidelityFX SDK release. From the SDK's Windows x64 output, copy amd_fidelityfx_loader_vk.dll and amd_fidelityfx_upscaler_vk.dll into upscalers\\fsr.",
                            "The SDK 2.4 Vulkan DLL selects FSR 4.1.2 on supported Radeon GPUs and FSR 3.1 on older ones; no separate FSR 4 DLL is needed.",
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
                        crate::i18n::text("Do not copy the whole SDK folder: the engine looks for these exact file names at the top of fsr, streamline and xess. If a file is renamed, the game will not see it.")
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
            ui.label(crate::i18n::text(label).font(theme::semibold(13.5)).color(p.ink));
            ui.add(egui::Label::new(crate::i18n::text(detail).size(12.5).color(p.body)).wrap());
        });
        if let Some((text, url)) = download {
            if widgets::button(ui, p, text, true)
                .on_hover_text(crate::i18n::tr("Opens the official download page in your browser"))
                .clicked()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
    });
    ui.add_space(4.0);
}

fn guide_block(ui: &mut egui::Ui, p: &theme::Palette, title: &str, lines: &[&str], link: Option<(&str, &str)>) {
    ui.label(crate::i18n::text(title).font(theme::semibold(14.0)).color(p.ink));
    for line in lines {
        ui.add(egui::Label::new(crate::i18n::text(*line).size(12.5).color(p.body)).wrap());
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
    widgets::page_header(ui, &p, "Audio", "Output, master gain and the engine's equalizer.");
    widgets::pending_note(
        ui,
        &p,
        "Audio settings are written to the engine's controls.ini before every start. The F1 menu changes the master volume and the equalizer live; this page is where you set their next-start values.",
    );
    widgets::section(ui, &p, "Output", |ui| {
        widgets::setting_row(ui, &p, "Device", "The output device follows the Windows default until device selection lands.", |ui| {
            ui.label(crate::i18n::text(&app.config.audio.device).color(p.ink));
        });
        widgets::setting_row(ui, &p, "Buffer", "Larger buffers resist crackling; smaller ones reduce latency.", |ui| {
            let mut ms = app.config.audio.buffer_ms as f32;
            if widgets::Slider::new(&mut ms, 10.0..=200.0).suffix(" ms").show(ui, &p).changed() {
                app.config.audio.buffer_ms = ms.round() as u32;
            }
        });
    });
    widgets::section(ui, &p, "Mix", |ui| {
        widgets::setting_row(
            ui,
            &p,
            "Master",
            "The volume everything the game plays is mixed at. Above 100% it is a limited boost, up to 200%, and it applies immediately; the limiter holds peaks below clipping. Start low.",
            |ui| {
                let mut pct = (app.config.audio.master * 100.0).round();
                if widgets::Slider::new(&mut pct, 0.0..=200.0).suffix(" %").show(ui, &p).changed() {
                    // Master gain is independent of the EQ preamp. Above 100% is the
                    // engine's limited boost; the row's help carries the warning, and
                    // the value applies at once instead of waiting on a modal that read
                    // as a hard 100% cap.
                    app.config.audio.master = (pct / 100.0).clamp(0.0, 2.0);
                }
            },
        );
        for (label, help) in [
            ("Music", "The music bus. The engine's mixer consumes it once the three buses are wired."),
            ("Effects", "The effects bus. The engine's mixer consumes it once the three buses are wired."),
            ("Voice", "The voice bus. The engine's mixer consumes it once the three buses are wired."),
        ] {
            widgets::setting_row(ui, &p, label, help, |ui| {
                let value = match label {
                    "Music" => &mut app.config.audio.music,
                    "Effects" => &mut app.config.audio.effects,
                    _ => &mut app.config.audio.voice,
                };
                let mut pct = (*value * 100.0).round();
                if widgets::Slider::new(&mut pct, 0.0..=100.0).suffix(" %").show(ui, &p).changed() {
                    *value = pct / 100.0;
                }
            });
        }
    });

    widgets::section(ui, &p, "Equalizer", |ui| {
        widgets::setting_row(
            ui,
            &p,
            "Enable equalizer",
            "A ten-band software EQ on the final mix. It applies to every output device without a second audio stack.",
            |ui| {
                widgets::toggle(ui, &p, &mut app.config.audio.eq);
            },
        );
        widgets::setting_row(ui, &p, "Preset", "Bass, treble and other ready-made curves. Move a band in the F1 menu to make a Custom curve.", |ui| {
            let current = app.config.audio.eq_preset.clone();
            egui::ComboBox::from_id_salt("audio-eq-preset")
                .selected_text(crate::i18n::tr(eq_preset_label(&current)))
                .width(180.0)
                .show_ui(ui, |ui| {
                    if eq_preset_label(&current) == "Custom" {
                        ui.label(crate::i18n::text("Custom (set in the F1 menu)").size(12.5).color(p.muted));
                        ui.separator();
                    }
                    for (key, label, _, preamp) in EQ_PRESETS {
                        if ui.selectable_label(current.eq_ignore_ascii_case(key), crate::i18n::text(label)).clicked() {
                            app.config.audio.eq_preset = key.to_string();
                            app.config.audio.eq_bands = eq_preset_bands(key).to_vec();
                            app.config.audio.eq_preamp = Some(preamp);
                        }
                    }
                });
        });
        ui.label(crate::i18n::text("The same ten bands and presets are in the F1 menu's Audio Enhancements panel, where they apply live.").size(12.5).color(p.muted));
    });

    if let Some(l) = crate::engine::live() {
        let a = &app.config.audio;
        let bands = a.eq_bands.iter().map(|v| format!("{v:.1}")).collect::<Vec<_>>().join(", ");
        let mut desired: std::collections::BTreeMap<String, String> = [
            ("audio.volume".to_string(), format!("{:.2}", a.master.clamp(0.0, 2.0))),
            ("audio.eq".to_string(), if a.eq { "1" } else { "0" }.to_string()),
            ("audio.eq_preset".to_string(), a.eq_preset.clone()),
            ("audio.eq_bands".to_string(), bands),
        ]
        .into_iter()
        .collect();
        if let Some(preamp) = a.eq_preamp {
            desired.insert("audio.eq_preamp".to_string(), format!("{preamp:.1}"));
        }
        let (pending, kept) = waiting(&desired, &l.values);
        let summary = format!("Master {:.0} %  ·  EQ {}", l.volume * 100.0, if a.eq { "on" } else { "off" });
        widgets::section_drawer(ui, &p, "audio-live", "What the game will start with", &summary, false, |ui| {
            widgets::kv(ui, &p, "Master volume", &format!("{:.0} %", l.volume * 100.0));
            widgets::kv(ui, &p, "Equalizer", &format!("{} ({})", if a.eq { "on" } else { "off" }, eq_preset_label(&a.eq_preset)));
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
            "audio.eq" => "equalizer",
            "audio.eq_preset" | "audio.eq_bands" => "equalizer preset",
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
        ui.label(crate::i18n::text("The game's files match this page.").size(12.5).color(p.muted));
    }
    if !pending.is_empty() {
        ui.label(
            crate::i18n::text(format!("Waiting for the game's next start: {}.", pending.join(", ")))
                .size(12.5)
                .color(p.muted),
        );
    }
    if !kept.is_empty() {
        ui.label(
            crate::i18n::text(format!(
                "Changed in the game's own menu, so the launcher leaves it alone: {}.",
                kept.join(", ")
            ))
            .size(12.5)
            .color(p.muted),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_display(resolution: &str, mode: &str) -> Graphics {
        Graphics {
            resolution: resolution.into(),
            display_mode: mode.into(),
            ..Default::default()
        }
    }

    #[test]
    fn every_preset_keeps_the_resolution_and_display_mode() {
        for (resolution, mode) in [
            ("Native display", "Borderless window"),
            ("3840 × 2160", "Exclusive fullscreen"),
            ("1280 × 720", "Windowed"),
        ] {
            for preset in Preset::ALL {
                let mut g = with_display(resolution, mode);
                preset.apply(&mut g);
                assert_eq!(g.resolution, resolution, "{} must not change the resolution", preset.label());
                assert_eq!(g.display_mode, mode, "{} must not change the display mode", preset.label());
            }
        }
    }

    #[test]
    fn the_presets_set_the_picture_and_frame_cap_they_describe() {
        let mut g = Graphics::default();

        Preset::Performance.apply(&mut g);
        assert_eq!(g.render_scale, 0.75);
        assert_eq!(g.upscaling, "FSR 1.0");
        assert_eq!(g.sharpening, "Default");
        assert_eq!(g.frame_limit, 60);
        assert!(g.vsync && g.shader_precompile);

        Preset::Balanced.apply(&mut g);
        assert_eq!(g.render_scale, 1.0);
        assert_eq!(g.upscaling, "Off");
        assert_eq!(g.sharpening, "Default");

        Preset::Quality.apply(&mut g);
        assert_eq!(g.render_scale, 1.0);
        assert_eq!(g.upscaling, "Off");
        assert_eq!(g.sharpening, "High");
        assert_eq!(g.frame_limit, 60);
        assert!(g.vsync && g.shader_precompile);
    }

    #[test]
    fn presets_leave_advanced_only_values_alone() {
        let mut g = Graphics {
            upscale_quality: "Ultra performance".into(),
            scene_scale: 80,
            frame_gen: "FSR 3 Frame Generation".into(),
            re_shade: false,
            dlss_upscaler: "DLSS".into(),
            astc_quality: "Fast".into(),
            gpu: "Discrete".into(),
            compile_threads: 6,
            compile_priority: "Normal".into(),
            compile_eco: false,
            quality: "Sharp".into(),
            aspect: "21:9".into(),
            ..Default::default()
        };
        for preset in Preset::ALL {
            preset.apply(&mut g);
        }
        // Everything below is set only on the Advanced graphics page; a Standard preset must not reset it.
        assert_eq!(g.upscale_quality, "Ultra performance");
        assert_eq!(g.scene_scale, 80);
        assert_eq!(g.frame_gen, "FSR 3 Frame Generation");
        assert!(!g.re_shade);
        assert_eq!(g.dlss_upscaler, "DLSS");
        assert_eq!(g.astc_quality, "Fast");
        assert_eq!(g.gpu, "Discrete");
        assert_eq!(g.compile_threads, 6);
        assert_eq!(g.compile_priority, "Normal");
        assert!(!g.compile_eco);
        assert_eq!(g.quality, "Sharp");
        assert_eq!(g.aspect, "21:9");
    }

}
