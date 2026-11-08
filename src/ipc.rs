//! Single instance + `localgif --toggle`.
//!
//! The running app listens on a Unix socket. Starting `localgif` again sends it a
//! command instead of opening a second copy. Binding `localgif --toggle` to a key in
//! your desktop's keyboard settings gives a global shortcut that also works on Wayland.

use crate::events::{AppEvent, EventSender};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

fn socket_path() -> PathBuf {
    match dirs::runtime_dir() {
        Some(dir) => dir.join("localgif.sock"),
        None => {
            let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
            std::env::temp_dir().join(format!("localgif-{user}.sock"))
        }
    }
}

/// Sends `command` to an already running LocalGif. Returns false if none is running.
pub fn send_to_running(command: &str) -> bool {
    match UnixStream::connect(socket_path()) {
        Ok(mut stream) => stream.write_all(command.as_bytes()).is_ok(),
        Err(_) => false,
    }
}

/// Listens for commands from other `localgif` processes.
pub fn listen(events: EventSender) {
    let path = socket_path();
    // A leftover socket from a crashed run; `send_to_running` already found nobody there.
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "LocalGif: can't listen on {} ({e}); --toggle won't work.",
                path.display()
            );
            return;
        }
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut command = String::new();
            let mut stream = stream;
            if stream.read_to_string(&mut command).is_err() {
                continue;
            }
            match command.trim() {
                "toggle" => events.send(AppEvent::Toggle),
                "show" => events.send(AppEvent::Show),
                "quit" => events.send(AppEvent::Quit),
                _ => {}
            }
        }
    });
}
