//! "Start LocalGif when I log in" (off unless the user turns it on).
//!
//! - Linux: `~/.config/autostart/localgif.desktop` (XDG autostart; KDE, GNOME, XFCE,
//!   Cinnamon, MATE, LXQt, Budgie...).
//! - Windows: the per-user `Run` registry key (like apps' "Start with Windows").
//!
//! Both start LocalGif with `--hidden`, so it waits in the tray (or minimized).
//! Whether it's on is read from the system each time, so it can't get out of sync.

/// False on platforms where this isn't implemented.
pub const SUPPORTED: bool = cfg!(any(target_os = "linux", windows));

pub fn is_enabled() -> bool {
    imp::is_enabled()
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if on {
        imp::enable(&exe)
    } else {
        imp::disable()
    }
}

/// If autostart is on, points it at this executable, so a rebuilt or moved
/// LocalGif is the one that starts at login (not an old copy).
pub fn keep_current() {
    if is_enabled() {
        let _ = set_enabled(true);
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::fs;
    use std::path::{Path, PathBuf};

    fn entry_path() -> Option<PathBuf> {
        Some(
            dirs::config_dir()?
                .join("autostart")
                .join("localgif.desktop"),
        )
    }

    pub fn is_enabled() -> bool {
        let Some(path) = entry_path() else {
            return false;
        };
        let Ok(text) = fs::read_to_string(path) else {
            return false;
        };
        // Desktops can switch an entry off without deleting it.
        !text.lines().any(|l| {
            let l = l.trim().replace(' ', "");
            l == "Hidden=true" || l == "X-GNOME-Autostart-enabled=false"
        })
    }

    pub fn enable(exe: &Path) -> Result<(), String> {
        let path = entry_path().ok_or("no config folder")?;
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        let entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=LocalGif\n\
             Comment=Local GIF picker\n\
             Exec={} --hidden\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n",
            quote(&exe.to_string_lossy())
        );
        // Don't rewrite an identical file (this runs at every start).
        if fs::read_to_string(&path).is_ok_and(|t| t == entry) {
            return Ok(());
        }
        fs::write(&path, entry).map_err(|e| e.to_string())
    }

    pub fn disable() -> Result<(), String> {
        match entry_path() {
            Some(path) if path.exists() => fs::remove_file(path).map_err(|e| e.to_string()),
            _ => Ok(()),
        }
    }

    /// Quotes a path for a .desktop `Exec=` line (Desktop Entry spec).
    fn quote(s: &str) -> String {
        let mut out = String::from("\"");
        for c in s.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        // In .desktop files, "%" starts a field code and must be doubled.
        out.replace('%', "%%")
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn quotes_exec_paths() {
            assert_eq!(
                super::quote("/home/me/my gifs/localgif"),
                "\"/home/me/my gifs/localgif\""
            );
            assert_eq!(super::quote("/a$b/100%"), "\"/a\\$b/100%%\"");
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::path::Path;
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "LocalGif";

    pub fn is_enabled() -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN, KEY_READ)
            .and_then(|k| k.get_value::<String, _>(NAME))
            .is_ok()
    }

    pub fn enable(exe: &Path) -> Result<(), String> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(RUN)
            .map_err(|e| e.to_string())?;
        let command = format!("\"{}\" --hidden", exe.display());
        key.set_value(NAME, &command).map_err(|e| e.to_string())
    }

    pub fn disable() -> Result<(), String> {
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN, KEY_SET_VALUE)
            .map_err(|e| e.to_string())?;
        match key.delete_value(NAME) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    use std::path::Path;

    pub fn is_enabled() -> bool {
        false
    }
    pub fn enable(_exe: &Path) -> Result<(), String> {
        Err("not supported on this system".into())
    }
    pub fn disable() -> Result<(), String> {
        Ok(())
    }
}
