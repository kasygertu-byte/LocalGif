//! Global keyboard shortcut that opens/closes the picker from any app.
//!
//! Works on Windows, macOS and Linux X11. Wayland doesn't let apps grab keys; bind
//! `localgif --toggle` in your desktop's keyboard settings instead.

use crate::events::{AppEvent, EventSender};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

/// Registers `shortcut` (e.g. "Ctrl+Alt+G"). The returned manager must be kept alive.
/// Returns an error message if the shortcut can't be used.
pub fn register(shortcut: &str, events: EventSender) -> Result<GlobalHotKeyManager, String> {
    let hotkey: HotKey = shortcut
        .parse()
        .map_err(|e| format!("invalid shortcut \"{shortcut}\": {e}"))?;
    let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
    manager
        .register(hotkey)
        .map_err(|e| format!("can't use {shortcut} ({e}); another app may already use it"))?;

    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state == HotKeyState::Pressed {
            events.send(AppEvent::Toggle);
        }
    }));
    Ok(manager)
}
