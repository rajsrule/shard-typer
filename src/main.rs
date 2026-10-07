#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod design;
mod glow;
mod ui;
use eframe::{egui, egui_wgpu};
use shard_typer::{platform, settings};

fn main() -> eframe::Result {
    let options = ui::PreviewOptions::from_args();
    let _instance = if options.preview {
        None
    } else {
        match platform::SingleInstance::acquire() {
            Ok(lock) => Some(lock),
            Err(message) => {
                eprintln!("{message}");
                return Ok(());
            }
        }
    };
    let path = settings::settings_path();
    let (mut settings, notice) = if options.preview {
        (settings::AppSettings::default(), None)
    } else {
        settings::load(&path)
    };
    if options.preview {
        settings.text="Some words deserve a little room to breathe.\n\nA small idea, one character at a time. Quietly precise.".into();
        settings.collapsed = options.pill;
        if options.small {
            settings.expanded_size = [400., 640.];
        }
        if options.target {
            settings.target_minutes = "5".into();
        }
    }
    let size = if settings.collapsed {
        [settings.expanded_size[0], 72.]
    } else {
        settings.expanded_size
    };
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/shard.png"))
        .expect("embedded app icon");
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Shard Typer")
        .with_decorations(false)
        .with_transparent(true)
        .with_inner_size(size)
        .with_min_inner_size(if settings.collapsed {
            [400., 72.]
        } else {
            [400., 640.]
        })
        .with_resizable(!settings.collapsed)
        .with_icon(icon);
    if settings.pinned {
        viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnTop);
    }
    let mut gpu_setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    // HWND swap chains ignore alpha. DirectComposition preserves per-pixel
    // transparency, including the completely clear rounded corners.
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
        "Shard Typer",
        native,
        Box::new(move |cc| {
            Ok(Box::new(ui::ShardApp::new(
                cc, settings, path, notice, options,
            )))
        }),
    )
}
