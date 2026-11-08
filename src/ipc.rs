//! Single instance + `localgif --toggle`.
//!
//! The running app listens on a Unix socket. Starting `localgif` again sends it a
//! command instead of opening a second copy. Binding `localgif --toggle` to a key in
//! your desktop's keyboard settings gives a global shortcut that also works on Wayland.
//!
//! When a *different build* is started (e.g. after updating), the running copy quits
//! and the new build takes over, so an outdated copy never keeps running in the tray.

use crate::events::{AppEvent, EventSender};
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn socket_path() -> PathBuf {
    match dirs::runtime_dir() {
        Some(dir) => dir.join("localgif.sock"),
        None => {
            let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
            std::env::temp_dir().join(format!("localgif-{user}.sock"))
        }
    }
}

/// Identifies this build: the executable's path and modification time.
/// Rebuilding changes it, so a running older build can be told apart.
pub fn build_id() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    let modified = std::fs::metadata(&exe)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}@{modified}", exe.display())
}

pub enum Outcome {
    /// No LocalGif is running.
    NotRunning,
    /// The running LocalGif handled the command; this process should exit.
    Handled,
    /// The running LocalGif was a different build and has quit; start normally.
    Replaced,
}

/// Sends `command` ("show", "toggle" or "ping") to a running LocalGif.
/// If it's a different build, it is asked to quit (except for "toggle", which
/// never replaces anything) so this build can take over.
pub fn send_to_running(command: &str, build: &str) -> Outcome {
    let Ok(mut stream) = UnixStream::connect(socket_path()) else {
        return Outcome::NotRunning;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if stream
        .write_all(format!("{command}\n{build}").as_bytes())
        .is_err()
    {
        return Outcome::NotRunning;
    }
    let _ = stream.shutdown(Shutdown::Write);
    let mut reply = String::new();
    let _ = stream.read_to_string(&mut reply);

    let reply = reply.trim();
    if let Some(pid) = reply.strip_prefix("replaced") {
        wait_until_gone(pid.trim().parse().ok());
        return Outcome::Replaced;
    }
    // Builds from before this check send no reply. Replace them too.
    if reply.is_empty() && command != "toggle" {
        if let Ok(mut s) = UnixStream::connect(socket_path()) {
            let _ = s.write_all(b"quit");
        }
        wait_until_gone(None);
        return Outcome::Replaced;
    }
    Outcome::Handled
}

/// Waits (up to 5 s) for the old instance to exit, so its global shortcut and
/// tray icon are released before this one registers its own.
fn wait_until_gone(pid: Option<u32>) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let gone = match pid {
            Some(pid) if cfg!(target_os = "linux") => {
                !std::path::Path::new(&format!("/proc/{pid}")).exists()
            }
            // Without a pid: an old build stops listening only when it exits.
            _ => UnixStream::connect(socket_path()).is_err(),
        };
        if gone {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Listens for commands from other `localgif` processes.
pub fn listen(events: EventSender, build: String) {
    let path = socket_path();
    // A leftover socket from a crashed run, or from the build that just quit.
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
        for mut stream in listener.incoming().flatten() {
            let mut message = String::new();
            if stream.read_to_string(&mut message).is_err() {
                continue;
            }
            let mut parts = message.splitn(2, '\n');
            let command = parts.next().unwrap_or("").trim();
            let sender_build = parts.next().map(str::trim);

            // A different build was started (not via --toggle): let it take over.
            if command != "toggle" && sender_build.is_some_and(|b| b != build) {
                let _ = stream.write_all(format!("replaced {}", std::process::id()).as_bytes());
                drop(stream);
                drop(listener); // Stop accepting so the new build can bind the socket.
                events.send(AppEvent::Quit);
                return;
            }
            let _ = stream.write_all(b"ok");
            match command {
                "toggle" => events.send(AppEvent::Toggle),
                "show" => events.send(AppEvent::Show),
                "quit" => events.send(AppEvent::Quit),
                _ => {}
            }
        }
    });
}
