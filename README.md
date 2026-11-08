# LocalGif

A local GIF picker written in Rust (egui). Scroll through your own GIFs, click one to
copy it, and paste it into Discord or any other app with **Ctrl+V**.

## Features

- Masonry grid of animated GIFs (resizes to the window width)
- **Click** a GIF to copy it to the clipboard as a file, so pasting uploads the real animated GIF
- **Labels**: every GIF has a label; if none is set, the file name is shown.
  Right-click a GIF → *Edit label…* to change it
- **Favorites**: hover a GIF and click the star in its top-right corner;
  favorited GIFs appear in the *Favorites* tab
- **Search bar** that filters by label (and file name), in both *All GIFs* and *Favorites*
- **Add GIFs** with the `+ Add GIFs` button or by dragging `.gif` files onto the window

Right-click menu: Copy, Add/Remove favorite, Edit label, Reset label, Remove from library.

## Build & run

Requires Rust (https://rustup.rs).

```bash
cargo run --release
```

The executable ends up at `target/release/localgif` (`localgif.exe` on Windows).

On Linux you may need the usual GUI dev libraries, e.g. on Debian/Ubuntu:

```bash
sudo apt install libxkbcommon-x11-0 libgl1-mesa-dri
```

## Where things are stored

| What | Windows | Linux | macOS |
|---|---|---|---|
| GIF files (copied here when added) | `%APPDATA%\LocalGif\gifs` | `~/.local/share/LocalGif/gifs` | `~/Library/Application Support/LocalGif/gifs` |
| Labels & favorites (`library.json`) | `%APPDATA%\LocalGif` | `~/.config/LocalGif` | `~/Library/Application Support/LocalGif` |

Added GIFs are **copied** into the library folder, so moving or deleting the original file
doesn't break anything. You can also drop `.gif` files into the GIF folder directly; they are
picked up on the next start. To use a different folder, edit `gif_dir` in `library.json`.

## Notes

- The GIF is copied as a *file* (CF_HDROP on Windows, `text/uri-list` on Linux, file URL on macOS),
  which is what Discord expects for pasting an animated GIF. Copying it as an image would lose the animation.
- Discord's upload size limit still applies (10 MB without Nitro).
