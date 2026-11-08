# LocalGif

A local GIF picker written in Rust (egui). Scroll through your own GIFs, click one to
copy it, and paste it into Discord or any other app with **Ctrl+V**.

On Linux it runs as a **taskbar widget**: an icon in the system tray that pops the picker
open above the icon.

## Features

- Masonry grid of animated GIFs (resizes to the window width)
- **Click** a GIF to copy it to the clipboard as a file, so pasting uploads the real animated GIF
- **Labels**: every GIF has a label; if none is set, the file name is shown.
  Right-click a GIF → *Edit label…* to change it
- **Favorites**: hover a GIF and click the star in its top-right corner;
  favorited GIFs appear in the *Favorites* tab
- **Tags**: right-click a GIF → *Edit tags…* to give it any number of tags (Enter or commas
  to add, click a tag to remove it, click a suggestion to reuse one). Tags show on the tile and
  are searchable
- **Most used** tab: GIFs you've copied, most copied first
- **NSFW tag**: right-click a GIF → *Mark as NSFW* to show it blurred with an NSFW label.
  It still copies normally when clicked; *Unmark as NSFW* restores it
- **Search bar** that filters by label, tags and file name, in every tab. Press **/** to jump to it
- **Global shortcut** **Ctrl+Alt+G** opens/closes the picker from any app (see below)
- **Add GIFs** with `+ Add` → *GIF files…* or *Folder…* (subfolders included), or drag files or
  folders onto the window
- **Duplicates**: adding a GIF that's already in the library (same content or same file name)
  asks whether to **Replace** it or **Keep existing**. Replacing keeps its label, tags, favorite
  and use count
- Reads **animated WebP** too (Discord often saves "GIFs" as WebP, sometimes still named `.gif`)

Right-click menu: Copy, Add/Remove favorite, Mark/Unmark as NSFW, Edit label, Edit tags, Reset label, Remove from library.

## Global shortcut

**Ctrl+Alt+G** shows the picker, or hides it if it's already in front. Change it with the
`hotkey` setting in `library.json` (e.g. `"Ctrl+Shift+Space"`, `"Super+G"`; `""` turns it off),
then restart LocalGif.

This works on **Windows** and **Linux X11**. **Wayland** doesn't allow apps to grab keys, so
there, add a custom shortcut in your desktop's keyboard settings that runs:

```bash
localgif --toggle
```

(KDE: *System Settings → Shortcuts → Add Command*; GNOME: *Settings → Keyboard → Custom Shortcuts*.)
`--toggle` also works on X11 if you prefer your desktop's shortcut settings.

Only one LocalGif runs at a time: launching it again brings up the running one.

## Build & run

Requires Rust (https://rustup.rs).

```bash
cargo run --release
```

The executable ends up at `target/release/localgif` (`localgif.exe` on Windows).

On Linux you may need the usual GUI libraries, e.g. on Debian/Ubuntu:

```bash
sudo apt install libxkbcommon-x11-0 libgl1-mesa-dri
```

## Linux taskbar widget

When your desktop has a system tray, LocalGif puts a **GIF** icon in it:

| Action | Result |
|---|---|
| Left-click the tray icon | Opens the picker above (or below) the icon; click again to close it |
| Press Esc | Hides it |
| Window close button | Hides it to the tray (keeps running) |
| Right-click the tray icon | Menu with *Open LocalGif* and *Quit* |

Start it in the tray only (no window) with:

```bash
localgif --hidden
```

**Supported desktops:** KDE Plasma, XFCE, Cinnamon, MATE, LXQt, Budgie.
On **GNOME**, install the *AppIndicator and KStatusNotifierItem Support* extension
(on Ubuntu it's already enabled). Without a system tray, LocalGif simply runs as a normal window.

**Wayland note:** Wayland doesn't let apps choose their window position, so the picker opens
where the compositor puts it instead of next to the icon.

### Start automatically on login

Copy the binary somewhere permanent and add an autostart entry:

```bash
mkdir -p ~/.local/bin ~/.config/autostart
cp target/release/localgif ~/.local/bin/
cat > ~/.config/autostart/localgif.desktop <<'EOF'
[Desktop Entry]
Type=Application
Name=LocalGif
Exec=sh -c '$HOME/.local/bin/localgif --hidden'
X-GNOME-Autostart-enabled=true
EOF
```

## Where things are stored

| What | Windows | Linux | macOS |
|---|---|---|---|
| GIF files (copied here when added) | `%APPDATA%\LocalGif\gifs` | `~/.local/share/LocalGif/gifs` | `~/Library/Application Support/LocalGif/gifs` |
| Labels, tags, favorites, NSFW, use counts, shortcut (`library.json`) | `%APPDATA%\LocalGif` | `~/.config/LocalGif` | `~/Library/Application Support/LocalGif` |

Added GIFs are **copied** into the library folder, so moving or deleting the original file
doesn't break anything. You can also put GIF files into that folder directly; they show up the
next time the LocalGif window gets focus. To use a different folder, edit `gif_dir` in `library.json`.

Files that can't be shown (for example an MP4 video saved with a `.gif` name) are listed under
the search bar. Hover the warning to see why.

## Notes

- The GIF is copied as a *file* (CF_HDROP on Windows, `text/uri-list` on Linux, file URL on macOS),
  which is what Discord expects for pasting an animated GIF. Copying it as an image would lose the animation.
- Discord's upload size limit still applies: 20 MB per file without Nitro.
- Video files (MP4/WebM) aren't supported. Discord's GIF picker (Tenor) often serves GIFs as MP4.
