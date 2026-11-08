//! Linux taskbar (system tray) icon, using the StatusNotifierItem D-Bus protocol.
//!
//! Works on KDE Plasma, XFCE, Cinnamon, MATE, LXQt, Budgie and on GNOME with the
//! "AppIndicator and KStatusNotifierItem Support" extension.

use crate::events::{AppEvent, EventSender};
use ksni::blocking::TrayMethods;

/// Keeps the tray icon alive while it exists.
pub struct TrayHandle(#[allow(dead_code)] ksni::blocking::Handle<GifTray>);

pub struct GifTray {
    events: EventSender,
    icon: ksni::Icon,
}

impl ksni::Tray for GifTray {
    fn id(&self) -> String {
        "localgif".into()
    }

    fn title(&self) -> String {
        "LocalGif".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![self.icon.clone()]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "LocalGif".into(),
            description: "Click to open your GIFs".into(),
            ..Default::default()
        }
    }

    fn activate(&mut self, x: i32, y: i32) {
        self.events.send(AppEvent::TrayClick { x, y });
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Open LocalGif".into(),
                activate: Box::new(|t: &mut Self| t.events.send(AppEvent::Show)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut Self| t.events.send(AppEvent::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Starts the tray icon. Returns `None` if the desktop has no system tray support,
/// in which case LocalGif runs as a normal window.
pub fn spawn(icon_rgba: &[u8], width: u32, height: u32, events: EventSender) -> Option<TrayHandle> {
    // ksni wants ARGB32 in network byte order.
    let data = icon_rgba
        .chunks_exact(4)
        .flat_map(|p| [p[3], p[0], p[1], p[2]])
        .collect();
    let tray = GifTray {
        events,
        icon: ksni::Icon {
            width: width as i32,
            height: height as i32,
            data,
        },
    };
    match tray.spawn() {
        Ok(handle) => Some(TrayHandle(handle)),
        Err(e) => {
            eprintln!("LocalGif: no system tray available ({e}), running as a normal window.");
            None
        }
    }
}
