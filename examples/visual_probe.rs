//! Interactive visual QA only: no input worker, global hotkeys, or saved settings.
//! Run through the documented Computer Use launcher when desktop inspection is needed.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[path = "../src/design.rs"]
mod design;
#[path = "../src/glow.rs"]
mod glow;
#[path = "../src/ui.rs"]
mod ui;

use eframe::{egui, egui_wgpu};
use shard_typer::settings::AppSettings;

fn main() -> eframe::Result {
    let mut preview = ui::PreviewOptions::from_args();
    preview.preview = true;
    let settings = AppSettings {
        text: "Some words deserve a little room to breathe.\n\nA small idea, one character at a time. Quietly precise.".into(),
        position: Some([140., 100.]),
        ..Default::default()
    };
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/shard.png"))
        .expect("embedded app icon");
    let viewport = egui::ViewportBuilder::default()
        .with_title("Shard Typer — Visual QA")
        .with_decorations(false)
        .with_transparent(true)
        .with_inner_size(settings.expanded_size)
        .with_min_inner_size([400., 640.])
        .with_resizable(true)
        .with_icon(icon);
    let mut gpu_setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    gpu_setup
        .instance_descriptor
        .backend_options
        .dx12
        .presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
    let native = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        dithering: false,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: gpu_setup.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "Shard Typer — Visual QA",
        native,
        Box::new(move |cc| {
            Ok(Box::new(ui::ShardApp::new(
                cc,
                settings,
                std::path::PathBuf::new(),
                None,
                preview,
            )))
        }),
    )
}
