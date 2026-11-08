//! Non-Linux builds have no tray icon; LocalGif runs as a normal window there.

use eframe::egui;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, OnceLock};

#[allow(dead_code)]
pub enum TrayEvent {
    Toggle { x: i32, y: i32 },
    Show,
    Quit,
}

pub struct TrayLink {
    pub events: Receiver<TrayEvent>,
    pub ctx: Arc<OnceLock<egui::Context>>,
}

pub fn spawn(_icon_rgba: &[u8], _width: u32, _height: u32) -> Option<TrayLink> {
    None
}
