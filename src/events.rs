//! Events sent to the app from other threads: the tray icon, the global hotkey
//! and `localgif --toggle` from another process.

use eframe::egui;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};

// Only the global shortcut sends events on platforms without a tray icon and `--toggle`.
#[cfg_attr(not(unix), allow(dead_code))]
pub enum AppEvent {
    /// Left click on the tray icon, with its screen position (may be 0,0 if unknown).
    TrayClick {
        x: i32,
        y: i32,
    },
    /// Global hotkey or `localgif --toggle`: show the picker, or hide it if it's in front.
    Toggle,
    Show,
    Quit,
}

#[derive(Clone)]
pub struct EventSender {
    tx: Sender<AppEvent>,
    ctx: Arc<OnceLock<egui::Context>>,
}

impl EventSender {
    pub fn send(&self, event: AppEvent) {
        let _ = self.tx.send(event);
        if let Some(ctx) = self.ctx.get() {
            // Wakes the app even while its window is hidden (runs `App::logic`).
            ctx.request_repaint();
        }
    }

    /// Called once the egui context exists, so events can wake the UI.
    pub fn set_context(&self, ctx: egui::Context) {
        let _ = self.ctx.set(ctx);
    }
}

pub fn channel_pair() -> (EventSender, Receiver<AppEvent>) {
    let (tx, rx) = channel();
    (
        EventSender {
            tx,
            ctx: Arc::new(OnceLock::new()),
        },
        rx,
    )
}
