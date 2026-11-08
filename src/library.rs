//! The GIF library: which GIFs exist, their labels and favorite state.
//!
//! GIF files live in a single folder (`gif_dir`). Metadata (labels, favorites,
//! date added) is stored next to the config as `library.json`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Default, Clone)]
struct EntryMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(default)]
    favorite: bool,
    #[serde(default)]
    added: u64,
}

#[derive(Serialize, Deserialize)]
struct LibraryFile {
    gif_dir: PathBuf,
    #[serde(default)]
    gifs: HashMap<String, EntryMeta>,
}

pub struct GifEntry {
    /// Session-unique id, used as a key for the texture cache.
    pub id: u64,
    pub file_name: String,
    pub path: PathBuf,
    /// User-defined label. `None` means "use the file name".
    pub label: Option<String>,
    pub favorite: bool,
    /// Unix timestamp (seconds) of when the GIF was added.
    pub added: u64,
    /// Pixel size `[width, height]`, read from the GIF header.
    pub size: [u32; 2],
}

impl GifEntry {
    /// The label if set, otherwise the file name without the extension.
    pub fn display_name(&self) -> &str {
        match &self.label {
            Some(label) => label,
            None => Path::new(&self.file_name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&self.file_name),
        }
    }
}

pub struct Library {
    config_path: PathBuf,
    pub gif_dir: PathBuf,
    pub entries: Vec<GifEntry>,
    next_id: u64,
}

impl Library {
    pub fn load() -> Self {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("LocalGif");
        let config_path = config_dir.join("library.json");

        let file: Option<LibraryFile> = fs::read_to_string(&config_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok());

        let (gif_dir, meta) = match file {
            Some(f) => (f.gif_dir, f.gifs),
            None => (
                dirs::data_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("LocalGif")
                    .join("gifs"),
                HashMap::new(),
            ),
        };
        let _ = fs::create_dir_all(&gif_dir);

        let mut lib = Self {
            config_path,
            gif_dir,
            entries: Vec::new(),
            next_id: 0,
        };
        lib.scan(&meta);
        lib.save();
        lib
    }

    fn scan(&mut self, meta: &HashMap<String, EntryMeta>) {
        let Ok(dir) = fs::read_dir(&self.gif_dir) else {
            return;
        };
        for item in dir.flatten() {
            let path = item.path();
            if !path.is_file() || !is_gif(&path) {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()).map(String::from)
            else {
                continue;
            };
            let Ok((w, h)) = image::image_dimensions(&path) else {
                continue;
            };
            let m = meta.get(&file_name).cloned().unwrap_or_else(|| EntryMeta {
                added: modified_secs(&path),
                ..Default::default()
            });
            let id = self.new_id();
            self.entries.push(GifEntry {
                id,
                file_name,
                path,
                label: m.label,
                favorite: m.favorite,
                added: m.added,
                size: [w.max(1), h.max(1)],
            });
        }
        self.sort();
    }

    /// Copies the given files into the library folder. Non-GIF files are skipped.
    /// Returns the number of GIFs added and a list of error messages.
    pub fn add_files(&mut self, paths: &[PathBuf]) -> (usize, Vec<String>) {
        let mut added = 0;
        let mut errors = Vec::new();
        let _ = fs::create_dir_all(&self.gif_dir);

        for src in paths {
            let name = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !is_gif(src) {
                errors.push(format!("{name}: not a .gif file"));
                continue;
            }
            let size = match image::image_dimensions(src) {
                Ok((w, h)) => [w.max(1), h.max(1)],
                Err(e) => {
                    errors.push(format!("{name}: {e}"));
                    continue;
                }
            };
            let dest = unique_path(&self.gif_dir, src);
            if let Err(e) = fs::copy(src, &dest) {
                errors.push(format!("{name}: {e}"));
                continue;
            }
            let id = self.new_id();
            self.entries.push(GifEntry {
                id,
                file_name: dest.file_name().unwrap().to_string_lossy().into_owned(),
                path: dest,
                label: None,
                favorite: false,
                added: now_secs(),
                size,
            });
            added += 1;
        }
        self.sort();
        self.save();
        (added, errors)
    }

    pub fn get(&self, id: u64) -> Option<&GifEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut GifEntry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    pub fn toggle_favorite(&mut self, id: u64) {
        if let Some(e) = self.get_mut(id) {
            e.favorite = !e.favorite;
            self.save();
        }
    }

    /// Sets the label. An empty / whitespace-only label resets it to the file name.
    pub fn set_label(&mut self, id: u64, label: &str) {
        if let Some(e) = self.get_mut(id) {
            let label = label.trim();
            e.label = (!label.is_empty()).then(|| label.to_string());
            self.save();
        }
    }

    /// Removes the GIF from the library and deletes its copy in the library folder.
    pub fn remove(&mut self, id: u64) -> Result<(), String> {
        let Some(idx) = self.entries.iter().position(|e| e.id == id) else {
            return Ok(());
        };
        fs::remove_file(&self.entries[idx].path).map_err(|e| e.to_string())?;
        self.entries.remove(idx);
        self.save();
        Ok(())
    }

    fn save(&self) {
        let file = LibraryFile {
            gif_dir: self.gif_dir.clone(),
            gifs: self.meta_map(),
        };
        let Ok(json) = serde_json::to_string_pretty(&file) else {
            return;
        };
        if let Some(parent) = self.config_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // Write to a temp file first so a crash can't leave a half-written library.
        let tmp = self.config_path.with_extension("json.tmp");
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(&tmp, &self.config_path);
        }
    }

    fn meta_map(&self) -> HashMap<String, EntryMeta> {
        self.entries
            .iter()
            .map(|e| {
                (
                    e.file_name.clone(),
                    EntryMeta {
                        label: e.label.clone(),
                        favorite: e.favorite,
                        added: e.added,
                    },
                )
            })
            .collect()
    }

    /// Newest first.
    fn sort(&mut self) {
        self.entries.sort_by(|a, b| {
            b.added
                .cmp(&a.added)
                .then_with(|| a.file_name.cmp(&b.file_name))
        });
    }

    fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}

pub fn is_gif(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
}

/// `dir/name.gif`, or `dir/name (2).gif`, `dir/name (3).gif`, ... if taken.
fn unique_path(dir: &Path, src: &Path) -> PathBuf {
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "gif".into());
    let mut candidate = dir.join(format!("{stem}.gif"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem} ({n}).gif"));
        n += 1;
    }
    candidate
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn modified_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
