// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod library;
mod loader;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("LocalGif")
            .with_inner_size([560.0, 720.0])
            .with_min_inner_size([300.0, 300.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "LocalGif",
        options,
        Box::new(|cc| Ok(Box::new(app::LocalGifApp::new(cc)))),
    )
}
