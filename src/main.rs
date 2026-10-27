//! janmp — a desktop music player in the spirit of AIMP.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod config;
mod hotkeys;
mod i18n;
mod media_controls;
mod meta;
mod playlist;
mod radio;
mod stats;
mod ui;
mod waveform;

/// Window and taskbar icon (the Windows executable also carries it, see build.rs).
fn icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/janmp.png")).expect("bundled icon").into_rgba8();
    egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() }
}

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let settings = config::Settings::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("janmp")
            .with_app_id("janmp")
            .with_inner_size([460.0, 720.0])
            .with_min_inner_size([380.0, 420.0])
            .with_decorations(settings.system_frame)
            .with_window_level(if settings.always_on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal })
            .with_drag_and_drop(true)
            .with_icon(std::sync::Arc::new(icon())),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native("janmp", options, Box::new(|cc| Ok(Box::new(app::JanmpApp::new(cc)))))
}
