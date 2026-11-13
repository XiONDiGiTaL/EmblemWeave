//! Graphics and audio settings.
//!
//! The renderer and the audio backend do not exist yet, so every option here
//! is stored for the day they land and none of them claims to do anything
//! today. Each page says so at the top.

use crate::theme;
use crate::widgets;
use crate::App;
use eframe::egui::{self, RichText};

fn combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[&str]) {
    egui::ComboBox::from_id_salt(id).selected_text(value.clone()).width(200.0).show_ui(ui, |ui| {
        for o in options {
            ui.selectable_value(value, o.to_string(), *o);
        }
    });
}

pub fn graphics(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Graphics", "Display and rendering options for the native renderer.");
    widgets::pending_note(
        ui,
        &p,
        "The game creates its device, memory, textures, shaders, swap chain and command buffers through the host NVN layer on your GPU, \
         but draw commands are recorded and counted, not rendered yet. These settings are saved now and take effect when the renderer draws. None of them changes anything today.",
    );
    let g = &mut app.config.graphics;
    widgets::section(ui, &p, "Display", |ui| {
        widgets::setting_row(ui, &p, "Resolution", "Output resolution of the game window.", |ui| {
            combo(ui, "res", &mut g.resolution, &["Native display", "1280 × 720", "1920 × 1080", "2560 × 1440", "3840 × 2160"]);
        });
        widgets::setting_row(ui, &p, "Display mode", "", |ui| {
            combo(ui, "mode", &mut g.display_mode, &["Windowed", "Borderless window", "Exclusive fullscreen"]);
        });
        widgets::setting_row(ui, &p, "Aspect ratio", "Wider ratios need the game's camera and UI adjusted, not only the output.", |ui| {
            combo(ui, "aspect", &mut g.aspect, &["16:9", "16:10", "21:9", "32:9"]);
        });
        widgets::setting_row(ui, &p, "VSync", "Synchronise presentation with the display.", |ui| {
            widgets::toggle(ui, &p, &mut g.vsync);
        });
        widgets::setting_row(ui, &p, "Frame-rate limit", "The game's simulation keeps its own tick; this limits presentation only.", |ui| {
            let mut fps = g.frame_limit as f32;
            if widgets::Slider::new(&mut fps, 30.0..=240.0).suffix(" fps").show(ui, &p).changed() {
                g.frame_limit = fps.round() as u32;
            }
        });
    });
    widgets::section(ui, &p, "Rendering", |ui| {
        widgets::setting_row(ui, &p, "Render scale", "Internal resolution relative to the output.", |ui| {
            widgets::Slider::new(&mut g.render_scale, 0.5..=2.0).fixed_decimals(2).suffix("×").show(ui, &p);
        });
        widgets::setting_row(ui, &p, "Upscaling", "", |ui| {
            combo(ui, "upscale", &mut g.upscaling, &["Off", "Bilinear", "FSR 1.0"]);
        });
        widgets::setting_row(ui, &p, "Quality", "Original keeps the console's rendering exactly.", |ui| {
            combo(ui, "quality", &mut g.quality, &["Original", "Enhanced"]);
        });
        widgets::setting_row(ui, &p, "Precompile shaders", "Translate the game's shaders before play to avoid stutter.", |ui| {
            widgets::toggle(ui, &p, &mut g.shader_precompile);
        });
        widgets::setting_row(ui, &p, "Renderer", "", |ui| {
            ui.label(RichText::new("Vulkan").font(theme::semibold(14.0)).color(p.ink));
        });
    });
}

pub fn audio(app: &mut App, ui: &mut egui::Ui) {
    let p = app.config.theme.palette();
    widgets::page_header(ui, &p, "Audio", "Output and mixing for the native audio backend.");
    widgets::pending_note(
        ui,
        &p,
        "The game opens its audio renderer and runs on its 5 ms audio clock, but voices are not mixed to your speakers yet. These settings are saved now and apply when mixing lands.",
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
            widgets::setting_row(ui, &p, label, "", |ui| {
                let mut pct = (*value * 100.0).round();
                if widgets::Slider::new(&mut pct, 0.0..=100.0).suffix(" %").show(ui, &p).changed() {
                    *value = pct / 100.0;
                }
            });
        }
    });
}
