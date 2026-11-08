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
    /// Files in the library folder that could not be loaded, as "name: reason".
    pub skipped: Vec<String>,
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
            skipped: Vec::new(),
            next_id: 0,
        };
        lib.scan(&meta);
        lib.save();
        lib
    }

    /// Picks up files that were added to or removed from the library folder
    /// while the app was running. Known GIFs keep their state.
    pub fn refresh(&mut self) {
        let meta = self.meta_map();
        if self.scan(&meta) {
            self.save();
        }
    }

    /// Syncs `entries` with the files in the library folder. Returns true if anything changed.
    fn scan(&mut self, meta: &HashMap<String, EntryMeta>) -> bool {
        let Ok(dir) = fs::read_dir(&self.gif_dir) else {
            return false;
        };
        let mut known: HashMap<String, GifEntry> = self
            .entries
            .drain(..)
            .map(|e| (e.file_name.clone(), e))
            .collect();
        let mut changed = false;
        self.skipped.clear();

        for item in dir.flatten() {
            let path = item.path();
            if !path.is_file() || !(is_supported(&path) || is_video_ext(&path)) {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()).map(String::from)
            else {
                continue;
            };
            if let Some(entry) = known.remove(&file_name) {
                self.entries.push(entry);
                continue;
            }
            let size = match probe(&path) {
                Ok(size) => size,
                Err(reason) => {
                    self.skipped.push(format!("{file_name}: {reason}"));
                    continue;
                }
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
                size,
            });
            changed = true;
        }
        // Whatever is left in `known` was deleted from the folder.
        changed |= !known.is_empty();
        self.skipped.sort();
        self.sort();
        changed
    }

    /// Copies the given files into the library folder. Folders are searched for GIFs.
    /// Returns the number of GIFs added and a list of error messages.
    pub fn add_files(&mut self, paths: &[PathBuf]) -> (usize, Vec<String>) {
        let mut added = 0;
        let mut errors = Vec::new();
        let _ = fs::create_dir_all(&self.gif_dir);

        let mut files = Vec::new();
        for path in paths {
            if path.is_dir() {
                collect_media_files(path, &mut files);
            } else {
                files.push(path.clone());
            }
        }

        for src in &files {
            let name = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let size = match probe(src) {
                Ok(size) => size,
                Err(reason) => {
                    errors.push(format!("{name}: {reason}"));
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

/// File extensions LocalGif accepts. Discord often saves GIFs as animated WebP.
pub fn is_supported(path: &Path) -> bool {
    has_ext(path, &["gif", "webp"])
}

fn is_video_ext(path: &Path) -> bool {
    has_ext(path, &["mp4", "webm", "mov"])
}

fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Checks what a file really contains (not just its extension) and returns its pixel size.
/// Files saved from Discord are often WebP or MP4 even when their name ends in `.gif`.
pub fn probe(path: &Path) -> Result<[u32; 2], String> {
    use image::ImageFormat;
    if is_video(path) {
        return Err(
            "this is actually a video (Discord often saves GIFs as MP4); videos aren't supported"
                .into(),
        );
    }
    let reader = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| e.to_string())?;
    match reader.format() {
        Some(ImageFormat::Gif | ImageFormat::WebP) => {
            let (w, h) = reader
                .into_dimensions()
                .map_err(|e| format!("damaged or unreadable file ({e})"))?;
            Ok([w.max(1), h.max(1)])
        }
        Some(format) => Err(format!(
            "this is a {} image, not a GIF",
            format
                .extensions_str()
                .first()
                .unwrap_or(&"different")
                .to_uppercase()
        )),
        None => Err("not a GIF file".into()),
    }
}

/// Detects MP4/MOV (`ftyp` box) and WebM/MKV (EBML header) by their first bytes.
fn is_video(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 12];
    let Ok(mut f) = fs::File::open(path) else {
        return false;
    };
    if f.read_exact(&mut head).is_err() {
        return false;
    }
    &head[4..8] == b"ftyp" || head[..4] == [0x1A, 0x45, 0xDF, 0xA3]
}

/// Recursively collects GIF/WebP (and video, to report them) files in a folder.
fn collect_media_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_media_files(&path, out);
        } else if is_supported(&path) || is_video_ext(&path) {
            out.push(path);
        }
    }
}

/// `dir/name.ext`, or `dir/name (2).ext`, `dir/name (3).ext`, ... if taken.
fn unique_path(dir: &Path, src: &Path) -> PathBuf {
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "gif".into());
    let ext = src
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "gif".into());
    let mut candidate = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem} ({n}).{ext}"));
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
