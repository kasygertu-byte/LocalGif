//! Exporting and importing GIF packs (see `crate::pack`). Both run in the
//! background so large packs don't freeze the window.

use super::LocalGifApp;
use crate::events::AppEvent;
use crate::pack::{self, EXTENSION, ExportItem};
use eframe::egui;
use std::path::PathBuf;

impl LocalGifApp {
    /// Asks where to save, then writes a pack with the GIFs `ids` (library order).
    pub(super) fn start_export(&mut self, ctx: &egui::Context, ids: &[u64]) {
        let items: Vec<ExportItem> = self
            .library
            .entries
            .iter()
            .filter(|e| ids.contains(&e.id))
            .map(|e| ExportItem {
                path: e.path.clone(),
                file_name: e.file_name.clone(),
                meta: e.pack_meta(),
            })
            .collect();
        if items.is_empty() {
            self.show_toast(ctx, "No GIFs to export");
            return;
        }
        let default_name = if items.len() == 1 {
            format!(
                "{}.{EXTENSION}",
                self.library
                    .get(ids[0])
                    .map(|e| e.display_name().to_string())
                    .unwrap_or_else(|| "GIF".into())
            )
        } else {
            format!("LocalGif pack ({} GIFs).{EXTENSION}", items.len())
        };
        let Some(mut dest) = rfd::FileDialog::new()
            .set_title(format!("Export {} GIFs as a pack", items.len()))
            .set_file_name(default_name)
            .add_filter("LocalGif pack", &[EXTENSION])
            .save_file()
        else {
            return; // Cancelled.
        };
        if dest.extension().is_none_or(|e| e != EXTENSION) {
            dest.set_extension(EXTENSION);
        }

        self.show_toast_for(ctx, format!("Exporting {} GIFs…", items.len()), 600.0);
        let events = self.bg.events.clone();
        std::thread::spawn(move || {
            let result = pack::export(&dest, &items).map(|(count, bytes)| {
                format!(
                    "Exported {count} GIFs ({:.1} MB) to {}",
                    bytes as f64 / 1_000_000.0,
                    dest.display()
                )
            });
            events.send(AppEvent::PackExported { result });
        });
    }

    /// Imports a pack: `pack` if given (drag & drop), otherwise asks for one.
    pub(super) fn start_import(&mut self, ctx: &egui::Context, pack: Option<PathBuf>) {
        let pack = match pack {
            Some(p) => p,
            None => {
                let Some(p) = rfd::FileDialog::new()
                    .set_title("Import a GIF pack")
                    .add_filter("LocalGif pack", &[EXTENSION])
                    .pick_file()
                else {
                    return;
                };
                p
            }
        };
        let temp = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("LocalGif")
            .join("import")
            .join(format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
        self.show_toast_for(ctx, "Importing GIF pack…", 600.0);
        let events = self.bg.events.clone();
        std::thread::spawn(move || {
            let result = match pack::extract(&pack, &temp) {
                Ok(items) => Ok((items, temp)),
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&temp);
                    Err(e)
                }
            };
            events.send(AppEvent::PackExtracted { result });
        });
    }

    /// Adds the GIFs extracted from a pack. Ones already in the library go
    /// through the usual "Replace / Keep existing" prompt.
    pub(super) fn finish_import(
        &mut self,
        ctx: &egui::Context,
        items: Vec<(PathBuf, crate::library::PackMeta)>,
        temp: PathBuf,
    ) {
        let count = items.len();
        let result = self
            .library
            .add_items(items.into_iter().map(|(p, m)| (p, Some(m))).collect());
        // The extracted files are needed until every conflict is answered.
        self.import_temp.push(temp);
        if count == 0 {
            self.show_toast(ctx, "That pack has no GIFs in it");
            return;
        }
        self.report_add(ctx, result);
    }

    /// Deletes extracted pack files once nothing refers to them anymore.
    pub(super) fn cleanup_import_temp(&mut self) {
        if self.conflicts.is_empty() {
            for dir in self.import_temp.drain(..) {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}
