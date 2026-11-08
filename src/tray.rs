//! Linux taskbar (system tray) icon, using the StatusNotifierItem D-Bus protocol.
//!
//! Works on KDE Plasma, XFCE, Cinnamon, MATE, LXQt, Budgie and on GNOME with the
//! "AppIndicator and KStatusNotifierItem Support" extension.

use eframe::egui;
use ksni::blocking::TrayMethods;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};

pub enum TrayEvent {
    /// Left click on the icon. Screen position of the click (may be 0,0 if unknown).
    Toggle {
        x: i32,
        y: i32,
    },
    Show,
    Quit,
}

pub struct TrayLink {
    pub events: Receiver<TrayEvent>,
    /// Filled in once the egui context exists, so the tray thread can wake the UI.
    pub ctx: Arc<OnceLock<egui::Context>>,
    _handle: ksni::blocking::Handle<GifTray>,
}

struct GifTray {
    tx: Sender<TrayEvent>,
    ctx: Arc<OnceLock<egui::Context>>,
    icon: ksni::Icon,
}

impl GifTray {
    fn send(&self, event: TrayEvent) {
        let _ = self.tx.send(event);
        if let Some(ctx) = self.ctx.get() {
            // Wakes the app even while its window is hidden (runs `App::logic`).
            ctx.request_repaint();
        }
    }
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
        self.send(TrayEvent::Toggle { x, y });
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Open LocalGif".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::Show)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Starts the tray icon. Returns `None` if the desktop has no system tray support,
/// in which case LocalGif runs as a normal window.
pub fn spawn(icon_rgba: &[u8], width: u32, height: u32) -> Option<TrayLink> {
    // ksni wants ARGB32 in network byte order.
    let data = icon_rgba
        .chunks_exact(4)
        .flat_map(|p| [p[3], p[0], p[1], p[2]])
        .collect();
    let (tx, events) = channel();
    let ctx = Arc::new(OnceLock::new());
    let tray = GifTray {
        tx,
        ctx: ctx.clone(),
        icon: ksni::Icon {
            width: width as i32,
            height: height as i32,
            data,
        },
    };
    match tray.spawn() {
        Ok(handle) => Some(TrayLink {
            events,
            ctx,
            _handle: handle,
        }),
        Err(e) => {
            eprintln!("LocalGif: no system tray available ({e}), running as a normal window.");
            None
        }
    }
}
