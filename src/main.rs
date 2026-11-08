// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod library;
mod loader;
#[cfg(target_os = "linux")]
mod tray;
#[cfg(not(target_os = "linux"))]
#[path = "tray_stub.rs"]
mod tray;

use eframe::egui;

fn main() -> eframe::Result {
    // `--hidden`: start in the taskbar only (useful for autostart).
    let start_hidden = std::env::args().any(|a| a == "--hidden");

    let icon = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .expect("bundled icon is a valid PNG")
        .into_rgba8();
    let (icon_w, icon_h) = icon.dimensions();

    let tray = tray::spawn(icon.as_raw(), icon_w, icon_h);

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("LocalGif")
        .with_app_id("localgif")
        .with_icon(egui::IconData {
            rgba: icon.into_raw(),
            width: icon_w,
            height: icon_h,
        })
        .with_inner_size([560.0, 720.0])
        .with_min_inner_size([300.0, 300.0])
        .with_drag_and_drop(true);
    if tray.is_some() {
        // Taskbar widget mode: a popup that lives in the tray, not a regular window.
        viewport = viewport.with_taskbar(false).with_always_on_top();
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "LocalGif",
        options,
        Box::new(move |cc| Ok(Box::new(app::LocalGifApp::new(cc, tray, start_hidden)))),
    )
}
