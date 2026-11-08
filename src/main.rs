// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod events;
mod hotkey;
#[cfg(unix)]
mod ipc;
#[cfg(not(unix))]
#[path = "ipc_stub.rs"]
mod ipc;
mod library;
mod loader;
#[cfg(target_os = "linux")]
mod tray;
#[cfg(not(target_os = "linux"))]
#[path = "tray_stub.rs"]
mod tray;

use eframe::egui;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);

    // Only one LocalGif runs at a time. If one is already running, pass the
    // request on to it and exit:
    //   --toggle  show the picker, or hide it if it's in front (bind this to a key)
    //   --hidden  nothing (it's already running in the tray)
    //   (none)    show the picker
    // If the running one is a different build (you updated), it quits and this
    // build takes over instead, so an old copy can't keep running with old data.
    let command = if has("--toggle") {
        "toggle"
    } else if has("--hidden") {
        "ping"
    } else {
        "show"
    };
    let build = ipc::build_id();
    if let ipc::Outcome::Handled = ipc::send_to_running(command, &build) {
        return Ok(());
    }
    // `--toggle` with nothing running just starts LocalGif normally (visible).
    let start_hidden = has("--hidden") && !has("--toggle");

    let (events, event_rx) = events::channel_pair();
    ipc::listen(events.clone(), build);

    let icon = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .expect("bundled icon is a valid PNG")
        .into_rgba8();
    let (icon_w, icon_h) = icon.dimensions();

    let tray = tray::spawn(icon.as_raw(), icon_w, icon_h, events.clone());

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
        Box::new(move |cc| {
            Ok(Box::new(app::LocalGifApp::new(
                cc,
                app::Background {
                    events,
                    event_rx,
                    tray,
                },
                start_hidden,
            )))
        }),
    )
}
