//! Non-Linux builds have no tray icon; LocalGif runs as a normal window there.

use crate::events::EventSender;

pub struct TrayHandle;

pub fn spawn(_icon_rgba: &[u8], _w: u32, _h: u32, _events: EventSender) -> Option<TrayHandle> {
    None
}
