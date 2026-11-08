//! The GIF library: which GIFs exist, their labels and favorite state.
//!
//! GIF files live in a single folder (`gif_dir`). Metadata (labels, tags, favorites,
//! usage counts) is stored next to the config as `library.json`.

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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    nsfw: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    warning: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(default)]
    added: u64,
    /// How many times the GIF was copied.
    #[serde(default, skip_serializing_if = "is_zero")]
    uses: u32,
    /// Unix timestamp (seconds) of the last copy.
    #[serde(default, skip_serializing_if = "is_zero")]
    last_used: u64,
}

fn is_zero<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// Default global shortcut that opens/closes the picker.
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+G";

fn default_hotkey() -> String {
    DEFAULT_HOTKEY.into()
}

/// View options, saved between sessions.
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(default)]
pub struct Settings {
    /// Show GIFs marked NSFW (censored). When false they're hidden completely.
    pub show_nsfw: bool,
    /// GIFs stay on their first frame until hovered.
    pub hover_to_play: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_nsfw: true,
            hover_to_play: false,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct LibraryFile {
    gif_dir: PathBuf,
    /// Global shortcut, e.g. "Ctrl+Alt+G". Empty disables it.
    #[serde(default = "default_hotkey")]
    hotkey: String,
    #[serde(default)]
    settings: Settings,
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
    /// Shown censored (blurred with an NSFW label) in the grid.
    pub nsfw: bool,
    /// Shown censored with an orange WARNING label; revealed while hovered.
    pub warning: bool,
    pub tags: Vec<String>,
    /// Unix timestamp (seconds) of when the GIF was added.
    pub added: u64,
    pub uses: u32,
    pub last_used: u64,
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

#[derive(Default)]
pub struct AddResult {
    pub added: usize,
    pub errors: Vec<String>,
    pub conflicts: Vec<Conflict>,
}

/// A file the user tried to add that is already in the library.
pub struct Conflict {
    pub src: PathBuf,
    pub existing: u64,
    pub kind: ConflictKind,
}

#[derive(Clone, Copy)]
pub enum ConflictKind {
    /// Exactly the same file content.
    Content,
    /// Different content, but a GIF with the same file name exists.
    Name,
    /// The file *is* the library copy (e.g. dragged from the library folder).
    File,
}

pub struct Library {
    config_path: PathBuf,
    pub gif_dir: PathBuf,
    pub hotkey: String,
    pub settings: Settings,
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

        let (gif_dir, hotkey, settings, meta) = match file {
            Some(f) => (f.gif_dir, f.hotkey, f.settings, f.gifs),
            None => (
                dirs::data_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("LocalGif")
                    .join("gifs"),
                default_hotkey(),
                Settings::default(),
                HashMap::new(),
            ),
        };
        let _ = fs::create_dir_all(&gif_dir);

        let mut lib = Self {
            config_path,
            gif_dir,
            hotkey,
            settings,
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
                nsfw: m.nsfw,
                warning: m.warning,
                tags: m.tags,
                added: m.added,
                uses: m.uses,
                last_used: m.last_used,
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
    /// GIFs that are already in the library are not added; they are returned as
    /// conflicts so the user can choose to replace them or not.
    pub fn add_files(&mut self, paths: &[PathBuf]) -> AddResult {
        let mut result = AddResult::default();
        let _ = fs::create_dir_all(&self.gif_dir);

        let mut files = Vec::new();
        for path in paths {
            if path.is_dir() {
                collect_media_files(path, &mut files);
            } else {
                files.push(path.clone());
            }
        }

        for src in files {
            let name = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let size = match probe(&src) {
                Ok(size) => size,
                Err(reason) => {
                    result.errors.push(format!("{name}: {reason}"));
                    continue;
                }
            };
            if let Some((existing, kind)) = self.find_existing(&src, &name) {
                result.conflicts.push(Conflict {
                    src,
                    existing,
                    kind,
                });
                continue;
            }
            let dest = unique_path(&self.gif_dir, &src);
            if let Err(e) = fs::copy(&src, &dest) {
                result.errors.push(format!("{name}: {e}"));
                continue;
            }
            let id = self.new_id();
            self.entries.push(GifEntry {
                id,
                file_name: dest.file_name().unwrap().to_string_lossy().into_owned(),
                path: dest,
                label: None,
                favorite: false,
                nsfw: false,
                warning: false,
                tags: Vec::new(),
                added: now_secs(),
                uses: 0,
                last_used: 0,
                size,
            });
            result.added += 1;
        }
        self.sort();
        self.save();
        result
    }

    /// Finds a library GIF that `src` duplicates: identical content, or the same file name.
    fn find_existing(&self, src: &Path, name: &str) -> Option<(u64, ConflictKind)> {
        let src_len = fs::metadata(src).ok()?.len();
        let mut src_bytes: Option<Vec<u8>> = None;
        for e in &self.entries {
            if same_file(src, &e.path) {
                return Some((e.id, ConflictKind::File));
            }
            if fs::metadata(&e.path).is_ok_and(|m| m.len() == src_len) {
                let bytes = src_bytes.get_or_insert_with(|| fs::read(src).unwrap_or_default());
                if fs::read(&e.path).is_ok_and(|b| &b == bytes) {
                    return Some((e.id, ConflictKind::Content));
                }
            }
        }
        self.entries
            .iter()
            .find(|e| e.file_name.eq_ignore_ascii_case(name))
            .map(|e| (e.id, ConflictKind::Name))
    }

    /// Overwrites the library copy of GIF `id` with `src`.
    /// Its label, tags, favorite and NSFW state are kept.
    pub fn replace(&mut self, id: u64, src: &Path) -> Result<(), String> {
        let size = probe(src)?;
        let entry = self.get_mut(id).ok_or("GIF no longer exists")?;
        if !same_file(src, &entry.path) {
            fs::copy(src, &entry.path).map_err(|e| e.to_string())?;
        }
        entry.size = size;
        self.save();
        Ok(())
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

    pub fn toggle_warning(&mut self, id: u64) {
        if let Some(e) = self.get_mut(id) {
            e.warning = !e.warning;
            self.save();
        }
    }

    pub fn set_settings(&mut self, settings: Settings) {
        self.settings = settings;
        self.save();
    }

    pub fn toggle_nsfw(&mut self, id: u64) {
        if let Some(e) = self.get_mut(id) {
            e.nsfw = !e.nsfw;
            self.save();
        }
    }

    /// Counts a copy, for the "Most used" view.
    pub fn record_use(&mut self, id: u64) {
        if let Some(e) = self.get_mut(id) {
            e.uses += 1;
            e.last_used = now_secs();
            self.save();
        }
    }

    /// Replaces the GIF's tags. Tags are trimmed and de-duplicated (ignoring case).
    pub fn set_tags(&mut self, id: u64, tags: &[String]) {
        let mut clean: Vec<String> = Vec::new();
        for t in tags {
            let t = t.trim();
            if !t.is_empty() && !clean.iter().any(|c| c.eq_ignore_ascii_case(t)) {
                clean.push(t.to_string());
            }
        }
        if let Some(e) = self.get_mut(id) {
            e.tags = clean;
            self.save();
        }
    }

    /// Every tag used in the library, sorted, without duplicates.
    pub fn all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = Vec::new();
        for t in self.entries.iter().flat_map(|e| &e.tags) {
            if !tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                tags.push(t.clone());
            }
        }
        tags.sort_by_key(|t| t.to_lowercase());
        tags
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
            hotkey: self.hotkey.clone(),
            settings: self.settings,
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
                        nsfw: e.nsfw,
                        warning: e.warning,
                        tags: e.tags.clone(),
                        added: e.added,
                        uses: e.uses,
                        last_used: e.last_used,
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

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
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
