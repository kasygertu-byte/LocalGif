mod bulk;
mod packs;
mod preview;
mod settings;

use crate::events::{AppEvent, EventSender};
use crate::library::{AddResult, Conflict, ConflictKind, GifEntry, Library};
use crate::loader::{Loader, PREVIEW_BIT};
use crate::tray::TrayHandle;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Pos2, Rect, Sense, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Vec2, pos2, text::LayoutJob, vec2,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

const ACCENT: Color32 = Color32::from_rgb(88, 101, 242);
const STAR_YELLOW: Color32 = Color32::from_rgb(250, 199, 50);
const NSFW_RED: Color32 = Color32::from_rgb(218, 55, 60);
const WARNING_ORANGE: Color32 = Color32::from_rgb(232, 128, 22);
/// Discord's upload limit without Nitro (20 MB).
const DISCORD_LIMIT: u64 = 20 * 1024 * 1024;
const TILE_BG: Color32 = Color32::from_rgb(30, 31, 34);
const GAP: f32 = 8.0;
const MIN_COLUMN_WIDTH: f32 = 170.0;
const TILE_RADIUS: u8 = 6;
/// Seconds a GIF can stay off-screen before its frames are freed from memory.
const EVICT_AFTER: f64 = 10.0;

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Grid,
    Settings,
}

#[derive(PartialEq, Clone, Copy)]
enum SettingsTab {
    General,
    Tags,
    Gifs,
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    All,
    Favorites,
    MostUsed,
}

struct LoadedGif {
    textures: Vec<TextureHandle>,
    /// Blurred preview shown instead of the GIF when it is marked NSFW.
    censored: TextureHandle,
    /// Cumulative end time of each frame, in seconds.
    frame_ends: Vec<f64>,
}

impl LoadedGif {
    fn total(&self) -> f64 {
        *self.frame_ends.last().unwrap_or(&0.0)
    }

    /// Returns the texture to show at `time` and the seconds until the next frame.
    fn frame_at(&self, time: f64) -> (&TextureHandle, Option<f64>) {
        let total = self.total();
        if self.textures.len() <= 1 || total <= 0.0 {
            return (&self.textures[0], None);
        }
        let t = time % total;
        let idx = self
            .frame_ends
            .partition_point(|&end| end <= t)
            .min(self.textures.len() - 1);
        (&self.textures[idx], Some(self.frame_ends[idx] - t))
    }
}

enum GifState {
    Loading,
    Ready(LoadedGif),
    Failed(String),
}

struct CacheItem {
    state: GifState,
    last_seen: f64,
}

struct Toast {
    text: String,
    until: f64,
}

enum Action {
    Copy(u64),
    Preview(u64),
    ToggleSelect(u64),
    ToggleFavorite(u64),
    ToggleNsfw(u64),
    ToggleWarning(u64),
    EditLabel(u64),
    EditTags(u64),
    ResetLabel(u64),
    AskRemove(u64),
}

pub struct LocalGifApp {
    library: Library,
    loader: Loader,
    cache: HashMap<u64, CacheItem>,
    clipboard: Option<arboard::Clipboard>,
    view: View,
    search: String,
    page: Page,
    settings_tab: SettingsTab,
    /// Whether "Start when I log in" is on (read from the system at startup).
    autostart_on: bool,
    /// Search box on the settings "GIFs" list.
    settings_search: String,
    /// Bulk selection (GIF ids).
    selected: HashSet<u64>,
    /// "Select" mode: clicking a GIF selects it instead of copying it.
    select_mode: bool,
    /// Ctrl+drag selection in progress: `Some(true)` selects, `Some(false)` deselects
    /// the GIFs the pointer passes over (decided by the first GIF pressed).
    paint_select: Option<bool>,
    bulk: Option<bulk::BulkDialog>,
    /// Ids shown in the grid last frame, for "Select all".
    last_shown: Vec<u64>,
    /// Selected tags; only GIFs that have all of them are shown.
    tag_filter: Vec<String>,
    /// Search box inside the "Filter by tag" dropdown.
    tag_filter_search: String,
    /// Search box on the Settings "Tags" tab.
    settings_tag_search: String,
    toast: Option<Toast>,
    /// GIF id + label text being edited.
    editing: Option<(u64, String)>,
    confirm_remove: Option<u64>,
    /// Enlarged view of one GIF (middle-click / "View larger").
    preview: Option<Preview>,
    /// "Also delete the original file" checkbox in the delete dialogs.
    delete_originals: bool,
    /// The GIF currently hovered and since when (for the Warning reveal delay).
    hover_start: Option<(u64, f64)>,
    tag_edit: Option<TagEdit>,
    /// GIFs the user tried to add that are already in the library, waiting for
    /// "Replace or skip?".
    conflicts: VecDeque<Conflict>,
    conflict_apply_all: bool,
    /// Folders with files extracted from GIF packs, deleted once all conflicts are answered.
    import_temp: Vec<PathBuf>,
    bg: Background,
    /// Keeps the global shortcut registered.
    _hotkey: Option<global_hotkey::GlobalHotKeyManager>,
    widget: WidgetState,
    had_focus: bool,
}

/// Things running outside the UI that send events to it.
pub struct Background {
    pub events: EventSender,
    pub event_rx: Receiver<AppEvent>,
    /// Taskbar widget mode: `Some` when the system tray icon is running.
    pub tray: Option<TrayHandle>,
}

struct Preview {
    id: u64,
    /// Decoded at full resolution (separately from the small grid thumbnail).
    state: GifState,
    /// The user clicked "Show" on a censored NSFW / Warning GIF.
    reveal: bool,
    /// Opened this frame: the click that opened it mustn't also close it.
    just_opened: bool,
}

struct TagEdit {
    id: u64,
    tags: Vec<String>,
    input: String,
}

/// Window state used in taskbar widget mode.
#[derive(Default)]
struct WidgetState {
    visible: bool,
    focused: bool,
    /// When the window last lost focus.
    unfocused_at: Option<Instant>,
    quitting: bool,
    /// Hide the window on the first frame (started with `--hidden`, e.g. at login):
    /// to the tray, or minimized when there's no tray (e.g. Windows).
    start_hidden: bool,
}

impl LocalGifApp {
    pub fn new(cc: &eframe::CreationContext<'_>, bg: Background, start_hidden: bool) -> Self {
        setup_style(&cc.egui_ctx);
        bg.events.set_context(cc.egui_ctx.clone());
        let library = Library::load();

        let hotkey = if library.hotkey.trim().is_empty() {
            None
        } else {
            match crate::hotkey::register(&library.hotkey, bg.events.clone()) {
                Ok(manager) => Some(manager),
                Err(e) => {
                    eprintln!("LocalGif: global shortcut disabled: {e}");
                    None
                }
            }
        };

        let widget = WidgetState {
            visible: true,
            start_hidden,
            ..Default::default()
        };
        let mut app = Self {
            library,
            loader: Loader::new(cc.egui_ctx.clone()),
            cache: HashMap::new(),
            clipboard: None,
            view: View::All,
            search: String::new(),
            page: Page::Grid,
            settings_tab: SettingsTab::General,
            autostart_on: crate::autostart::is_enabled(),
            settings_search: String::new(),
            selected: HashSet::new(),
            select_mode: false,
            paint_select: None,
            bulk: None,
            last_shown: Vec::new(),
            tag_filter: Vec::new(),
            tag_filter_search: String::new(),
            settings_tag_search: String::new(),
            toast: None,
            editing: None,
            confirm_remove: None,
            preview: None,
            delete_originals: false,
            hover_start: None,
            tag_edit: None,
            conflicts: VecDeque::new(),
            conflict_apply_all: false,
            import_temp: Vec::new(),
            bg,
            _hotkey: hotkey,
            widget,
            had_focus: true,
        };
        if let Some(warning) = app.library.load_warning.take() {
            app.show_toast_for(&cc.egui_ctx, warning, 20.0);
        }
        app
    }

    /// Shows the widget window, placed next to the tray icon if its position is known.
    fn show_window(&mut self, ctx: &egui::Context, click: Option<(i32, i32)>) {
        if let Some((x, y)) = click.filter(|&(x, y)| x != 0 || y != 0) {
            let (ppp, outer) = ctx.input(|i| {
                let v = i.viewport();
                (v.native_pixels_per_point.unwrap_or(1.0), v.outer_rect)
            });
            let size = outer.map_or(vec2(560.0, 720.0), |r| r.size());
            let click = pos2(x as f32 / ppp, y as f32 / ppp);
            // Panel at the bottom: open above the icon. Panel at the top: open below it.
            let y = if click.y > size.y + 12.0 {
                click.y - size.y - 12.0
            } else {
                click.y + 12.0
            };
            let pos = pos2((click.x - size.x / 2.0).max(0.0), y);
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.widget.visible = true;
    }

    /// Hides to the tray, or minimizes when there is no tray to bring it back from.
    fn hide_window(&mut self, ctx: &egui::Context) {
        if self.bg.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        self.widget.visible = false;
        self.widget.focused = false;
    }

    fn any_dialog_open(&self) -> bool {
        self.editing.is_some()
            || self.confirm_remove.is_some()
            || self.tag_edit.is_some()
            || !self.conflicts.is_empty()
            || self.bulk.is_some()
            || self.preview.is_some()
    }

    fn show_toast(&mut self, ctx: &egui::Context, text: impl Into<String>) {
        self.show_toast_for(ctx, text, 2.5);
    }

    fn show_toast_for(&mut self, ctx: &egui::Context, text: impl Into<String>, secs: f64) {
        let now = ctx.input(|i| i.time);
        self.toast = Some(Toast {
            text: text.into(),
            until: now + secs,
        });
    }

    /// Puts the GIF file on the clipboard as a file, so pasting into
    /// Discord (or a file manager, chat app, ...) uploads the actual animated GIF.
    fn copy_gif(&mut self, ctx: &egui::Context, id: u64) {
        let Some(entry) = self.library.get(id) else {
            return;
        };
        let path = entry.path.clone();

        // A WebP file (often saved from Discord, even when named .gif) can end up
        // as a still image once sent. Convert it to a real GIF first, in the
        // background; `AppEvent::Converted` finishes the copy.
        if crate::convert::needs_conversion(&path) {
            self.show_toast_for(ctx, "Converting to GIF…", 30.0);
            let events = self.bg.events.clone();
            std::thread::spawn(move || {
                let result = crate::convert::to_gif(&path);
                events.send(AppEvent::Converted { id, result });
            });
            return;
        }
        self.put_on_clipboard(ctx, id, path);
    }

    fn put_on_clipboard(&mut self, ctx: &egui::Context, id: u64, path: PathBuf) {
        let Some(name) = self.library.get(id).map(|e| e.display_name().to_string()) else {
            return;
        };
        let too_big = std::fs::metadata(&path).is_ok_and(|m| m.len() > DISCORD_LIMIT);

        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(c) => self.clipboard = Some(c),
                Err(e) => {
                    self.show_toast(ctx, format!("Clipboard unavailable: {e}"));
                    return;
                }
            }
        }
        let result = self.clipboard.as_mut().unwrap().set().file_list(&[path]);
        match result {
            Ok(()) => {
                self.library.record_use(id);
                if too_big {
                    self.show_toast_for(
                        ctx,
                        format!("Copied \"{name}\", but it's over 20 MB: Discord won't send it without Nitro"),
                        6.0,
                    );
                } else {
                    self.show_toast(ctx, format!("Copied \"{name}\" - paste it with Ctrl+V"));
                }
            }
            Err(e) => self.show_toast(ctx, format!("Copy failed: {e}")),
        }
    }

    fn add_files(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let result = self.library.add_files(&paths);
        self.report_add(ctx, result);
    }

    /// Queues duplicates for the "Replace / Keep existing" prompt and shows a summary.
    fn report_add(&mut self, ctx: &egui::Context, result: AddResult) {
        let errors = result.errors;
        // Already-added GIFs: ask "Replace or skip?" one by one.
        if self.conflicts.is_empty() {
            self.conflict_apply_all = false;
        }
        self.conflicts.extend(result.conflicts);

        let mut msg = match result.added {
            0 => "No new GIFs added".to_string(),
            1 => "Added 1 GIF".to_string(),
            n => format!("Added {n} GIFs"),
        };
        if errors.is_empty() {
            if result.added > 0 || self.conflicts.is_empty() {
                self.show_toast(ctx, msg);
            }
        } else {
            msg += &format!(", {} skipped:", errors.len());
            for e in errors.iter().take(5) {
                msg += &format!("\n• {e}");
            }
            if errors.len() > 5 {
                msg += &format!("\n…and {} more", errors.len() - 5);
            }
            self.show_toast_for(ctx, msg, 8.0);
        }
    }

    fn handle_decoded(&mut self, ctx: &egui::Context) {
        while let Some(decoded) = self.loader.try_recv() {
            // Full-resolution decode for the enlarged preview.
            if decoded.id & PREVIEW_BIT != 0 {
                let id = decoded.id & !PREVIEW_BIT;
                if let Some(preview) = self.preview.as_mut().filter(|p| p.id == id) {
                    preview.state = upload(ctx, decoded.id, decoded.result);
                }
                continue;
            }
            // Ignore results for GIFs that were evicted or removed in the meantime.
            let Some(item) = self.cache.get_mut(&decoded.id) else {
                continue;
            };
            item.state = upload(ctx, decoded.id, decoded.result);
        }
    }
}

/// Uploads decoded frames to the GPU.
fn upload(
    ctx: &egui::Context,
    key: u64,
    result: Result<crate::loader::DecodedGif, String>,
) -> GifState {
    match result {
        Ok(gif) => {
            let mut textures = Vec::with_capacity(gif.frames.len());
            let mut frame_ends = Vec::with_capacity(gif.frames.len());
            let mut t = 0.0;
            for (i, f) in gif.frames.into_iter().enumerate() {
                t += f.delay;
                frame_ends.push(t);
                textures.push(ctx.load_texture(
                    format!("gif-{key}-{i}"),
                    f.image,
                    TextureOptions::LINEAR,
                ));
            }
            let censored = ctx.load_texture(
                format!("gif-{key}-censored"),
                gif.censored,
                TextureOptions::LINEAR,
            );
            GifState::Ready(LoadedGif {
                textures,
                censored,
                frame_ends,
            })
        }
        Err(e) => GifState::Failed(e),
    }
}

impl LocalGifApp {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let fav_count = self.library.entries.iter().filter(|e| e.favorite).count();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.view, View::All, "All GIFs");
            ui.selectable_value(
                &mut self.view,
                View::Favorites,
                format!("Favorites ({fav_count})"),
            );
            ui.selectable_value(&mut self.view, View::MostUsed, "Most used");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙").on_hover_text("Settings").clicked() {
                    self.page = Page::Settings;
                }
                let mut picked = None;
                let mut import_pack = false;
                ui.menu_button("+ Add", |ui| {
                    if ui.button("GIF files…").clicked() {
                        ui.close();
                        picked = rfd::FileDialog::new()
                            .set_title("Add GIFs")
                            .add_filter("GIF / WebP", &["gif", "GIF", "webp", "WEBP"])
                            .pick_files();
                    }
                    if ui.button("Folder…").clicked() {
                        ui.close();
                        picked = rfd::FileDialog::new()
                            .set_title("Add a folder of GIFs")
                            .pick_folder()
                            .map(|dir| vec![dir]);
                    }
                    ui.separator();
                    if ui.button("GIF pack…").clicked() {
                        ui.close();
                        import_pack = true;
                    }
                });
                if import_pack {
                    let ctx = ui.ctx().clone();
                    self.start_import(&ctx, None);
                }
                if let Some(paths) = picked {
                    let ctx = ui.ctx().clone();
                    self.add_files(&ctx, paths);
                }
                if ui
                    .toggle_value(&mut self.select_mode, "Select")
                    .on_hover_text(
                        "Select several GIFs to rename, tag or delete them\n(or Ctrl+click / Ctrl+drag over GIFs)",
                    )
                    .clicked()
                    && !self.select_mode
                {
                    self.selected.clear();
                }
            });
        });
        ui.add_space(6.0);

        let hint = match self.view {
            View::All => "Search All GIFs   ( / )",
            View::Favorites => "Search Favorites   ( / )",
            View::MostUsed => "Search Most Used   ( / )",
        };
        egui::Frame::new()
            .fill(TILE_BG)
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("🔍").color(Color32::GRAY));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .id(search_id())
                            .hint_text(hint)
                            .frame(egui::Frame::NONE)
                            .desired_width(f32::INFINITY),
                    );
                });
            });

        ui.add_space(6.0);
        self.filter_row(ui);

        let skipped = &self.library.skipped;
        if !skipped.is_empty() {
            ui.add_space(4.0);
            let text = match skipped.len() {
                1 => "⚠ 1 file in your GIF folder can't be shown (hover for details)".to_string(),
                n => format!("⚠ {n} files in your GIF folder can't be shown (hover for details)"),
            };
            ui.label(
                egui::RichText::new(text)
                    .small()
                    .color(Color32::from_rgb(240, 180, 80)),
            )
            .on_hover_ui(|ui| {
                for s in skipped {
                    ui.label(s);
                }
            });
        }
        ui.add_space(8.0);
    }

    /// Tag filter, plus the "Show NSFW" and "Hover to play" options.
    fn filter_row(&mut self, ui: &mut egui::Ui) {
        let all_tags = self.library.all_tags();
        // Forget selected tags that no GIF has anymore.
        self.tag_filter
            .retain(|t| all_tags.iter().any(|a| a.eq_ignore_ascii_case(t)));

        ui.horizontal(|ui| {
            let title = if self.tag_filter.is_empty() {
                "Filter by tag ⏷".to_string()
            } else {
                format!("Tags ({}) ⏷", self.tag_filter.len())
            };
            ui.add_enabled_ui(!all_tags.is_empty(), |ui| {
                ui.menu_button(title, |ui| {
                    let search = ui.add(
                        egui::TextEdit::singleline(&mut self.tag_filter_search)
                            .hint_text("Search tags")
                            .desired_width(180.0),
                    );
                    // Type right away when the dropdown opens.
                    if ui.ctx().memory(|m| m.focused().is_none()) {
                        search.request_focus();
                    }
                    let query = self
                        .tag_filter_search
                        .trim()
                        .trim_start_matches('#')
                        .to_lowercase();
                    let matching: Vec<&String> = all_tags
                        .iter()
                        .filter(|t| query.is_empty() || t.to_lowercase().contains(&query))
                        .collect();
                    if matching.is_empty() {
                        ui.label(egui::RichText::new("No matching tags").color(Color32::GRAY));
                    }
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for tag in matching {
                                let pos = self
                                    .tag_filter
                                    .iter()
                                    .position(|t| t.eq_ignore_ascii_case(tag));
                                let mut on = pos.is_some();
                                if ui.checkbox(&mut on, format!("#{tag}")).changed() {
                                    match pos {
                                        Some(i) => {
                                            self.tag_filter.remove(i);
                                        }
                                        None => self.tag_filter.push(tag.clone()),
                                    }
                                }
                            }
                        });
                    if !self.tag_filter.is_empty() {
                        ui.separator();
                        if ui.button("Clear tag filter").clicked() {
                            self.tag_filter.clear();
                            ui.close();
                        }
                    }
                })
                .response
                .on_disabled_hover_text(
                    "Add tags to GIFs (right-click → Edit tags…) to filter by them",
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut settings = self.library.settings.clone();
                let a = ui
                    .checkbox(&mut settings.hover_to_play, "Hover to play")
                    .on_hover_text("GIFs stay still until you hover over them");
                let b = ui
                    .checkbox(&mut settings.show_nsfw, "Show NSFW")
                    .on_hover_text("Uncheck to hide NSFW GIFs completely");
                if a.changed() || b.changed() {
                    self.library.set_settings(settings);
                }
            });
        });

        // Selected tags as chips; click one to remove it from the filter.
        if !self.tag_filter.is_empty() {
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("Showing GIFs tagged:")
                        .small()
                        .color(Color32::GRAY),
                );
                for (i, tag) in self.tag_filter.iter().enumerate() {
                    if ui
                        .small_button(format!("#{tag}  ✕"))
                        .on_hover_text("Remove from filter")
                        .clicked()
                    {
                        remove = Some(i);
                    }
                }
            });
            if let Some(i) = remove {
                self.tag_filter.remove(i);
            }
        }
    }

    /// Draws the masonry grid and returns the actions the user triggered.
    fn gif_grid(&mut self, ui: &mut egui::Ui, visible_ids: &mut HashSet<u64>) -> Vec<Action> {
        let mut actions = Vec::new();
        let now = ui.input(|i| i.time);
        let query = self.search.trim().to_lowercase();

        let mut shown: Vec<&GifEntry> = self
            .library
            .entries
            .iter()
            .filter(|e| match self.view {
                View::All => true,
                View::Favorites => e.favorite,
                View::MostUsed => e.uses > 0,
            })
            .filter(|e| self.library.settings.show_nsfw || !e.nsfw)
            .filter(|e| {
                self.tag_filter
                    .iter()
                    .all(|f| e.tags.iter().any(|t| t.eq_ignore_ascii_case(f)))
            })
            .filter(|e| {
                query.is_empty()
                    || e.display_name().to_lowercase().contains(&query)
                    || e.file_name.to_lowercase().contains(&query)
                    || e.tags.iter().any(|t| t.to_lowercase().contains(&query))
            })
            .collect();
        if self.view == View::MostUsed {
            shown.sort_by(|a, b| {
                b.uses
                    .cmp(&a.uses)
                    .then_with(|| b.last_used.cmp(&a.last_used))
            });
        }

        self.last_shown = shown.iter().map(|e| e.id).collect();

        if shown.is_empty() {
            let filtered = !self.tag_filter.is_empty()
                || (!self.library.settings.show_nsfw
                    && self.library.entries.iter().any(|e| e.nsfw));
            let msg = if !query.is_empty() || filtered {
                "No GIFs match your search or filters."
            } else if self.view == View::Favorites {
                "No favorites yet.\nHover over a GIF and click the star to add it here."
            } else if self.view == View::MostUsed {
                "Nothing here yet.\nGIFs you copy show up here, most used first."
            } else {
                "No GIFs yet.\nClick \"+ Add\" or drag GIF files or folders onto this window."
            };
            ui.add_space(40.0);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new(msg).color(Color32::GRAY)));
            return actions;
        }

        // Masonry layout: each GIF goes into the currently shortest column.
        let avail_w = ui.available_width();
        let columns = (((avail_w + GAP) / (MIN_COLUMN_WIDTH + GAP)).floor() as usize).max(1);
        let col_w = (avail_w - GAP * (columns - 1) as f32) / columns as f32;
        let mut col_heights = vec![0.0f32; columns];
        let mut placed = Vec::with_capacity(shown.len());
        for entry in &shown {
            let aspect = entry.size[1] as f32 / entry.size[0] as f32;
            let h = (col_w * aspect).clamp(col_w * 0.4, col_w * 2.0);
            let col = (0..columns)
                .min_by(|&a, &b| col_heights[a].total_cmp(&col_heights[b]))
                .unwrap();
            let offset = vec2(col as f32 * (col_w + GAP), col_heights[col]);
            col_heights[col] += h + GAP;
            placed.push((offset, vec2(col_w, h)));
        }
        let total_h = col_heights.iter().cloned().fold(0.0, f32::max) - GAP;
        let (area, _) = ui.allocate_exact_size(vec2(avail_w, total_h.max(0.0)), Sense::hover());

        let clip = ui.clip_rect();
        // Start loading GIFs slightly before they scroll into view.
        let preload = clip.expand2(vec2(0.0, clip.height() * 0.75));
        let mut next_repaint: Option<f64> = None;
        let hover_to_play = self.library.settings.hover_to_play;
        let (ctrl, alt, pressed, down) = ui.input(|i| {
            (
                i.modifiers.command,
                i.modifiers.alt,
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
            )
        });
        if !down {
            self.paint_select = None;
        }
        let warning_delay = self.library.settings.warning_delay as f64;
        let uncensor_nsfw = self.library.settings.uncensor_nsfw;
        let uncensor_warning = self.library.settings.uncensor_warning;
        let mut hovered_id = None;

        for (entry, (offset, size)) in shown.iter().zip(placed) {
            let rect = Rect::from_min_size(area.min + offset, size);
            if !rect.intersects(preload) {
                continue;
            }

            visible_ids.insert(entry.id);
            let item = ensure_loaded(&mut self.cache, &self.loader, entry, now);

            if !rect.intersects(clip) {
                continue;
            }

            let tile_id = ui.make_persistent_id(("gif", entry.id));
            let response = ui
                .interact(rect, tile_id, Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            let hovered = ui.rect_contains_pointer(rect);
            // Alt + hover reveals (and plays) NSFW and Warning GIFs.
            let peek = alt && hovered;
            // NSFW GIFs are blurred unless censoring is turned off or Alt-peeked.
            let nsfw_censored = entry.nsfw && !uncensor_nsfw && !peek;
            // Warning GIFs are revealed after hovering for `warning_delay` seconds.
            let hovered_for = if hovered {
                hovered_id = Some(entry.id);
                match self.hover_start {
                    Some((id, since)) if id == entry.id => now - since,
                    _ => {
                        self.hover_start = Some((entry.id, now));
                        0.0
                    }
                }
            } else {
                0.0
            };
            let warning_revealed =
                uncensor_warning || peek || (hovered && hovered_for >= warning_delay);
            if entry.warning && hovered && !warning_revealed {
                // Repaint when the delay is over.
                let wait = warning_delay - hovered_for;
                next_repaint = Some(next_repaint.map_or(wait, |r| r.min(wait)));
            }

            // Ctrl + click / Ctrl + drag across GIFs selects them.
            // The first GIF pressed decides whether the drag selects or deselects.
            let paint = if ctrl && hovered && pressed {
                let select = !self.selected.contains(&entry.id);
                self.paint_select = Some(select);
                Some(select) // Applied even if the click is released in this same frame.
            } else if hovered && down {
                self.paint_select
            } else {
                None
            };
            match paint {
                Some(true) => {
                    self.selected.insert(entry.id);
                }
                Some(false) => {
                    self.selected.remove(&entry.id);
                }
                None => {}
            }
            let painter = ui.painter_at(rect);

            painter.rect_filled(rect, TILE_RADIUS, TILE_BG);
            match &item.state {
                // NSFW: censored unless Alt+hovered. Warning: censored until hovered.
                GifState::Ready(gif) if nsfw_censored || (entry.warning && !warning_revealed) => {
                    // Censored: stretched tiny preview (looks blurred), darkened, label on top.
                    let tex = &gif.censored;
                    egui::Image::from_texture(tex)
                        .uv(cover_uv(tex.size_vec2(), rect.size()))
                        .corner_radius(TILE_RADIUS)
                        .paint_at(ui, rect);
                    painter.rect_filled(rect, TILE_RADIUS, Color32::from_black_alpha(110));
                }
                GifState::Ready(gif) => {
                    let (tex, until_next) = if hover_to_play && !hovered {
                        (&gif.textures[0], None) // Still until hovered.
                    } else {
                        gif.frame_at(now)
                    };
                    if let Some(dt) = until_next {
                        next_repaint = Some(next_repaint.map_or(dt, |r| r.min(dt)));
                    }
                    egui::Image::from_texture(tex)
                        .uv(cover_uv(tex.size_vec2(), rect.size()))
                        .corner_radius(TILE_RADIUS)
                        .paint_at(ui, rect);
                }
                GifState::Loading => {
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "Loading…",
                        FontId::proportional(12.0),
                        Color32::GRAY,
                    );
                }
                GifState::Failed(err) => {
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "Can't load GIF",
                        FontId::proportional(12.0),
                        Color32::LIGHT_RED,
                    );
                    if hovered {
                        response.clone().on_hover_text(err);
                    }
                }
            }

            // A censored GIF shows every label it has (NSFW and/or WARNING).
            if nsfw_censored || (entry.warning && !warning_revealed) {
                draw_badges(&painter, rect, entry.nsfw, entry.warning);
            }
            draw_label(&painter, rect, entry.display_name(), &entry.tags);

            let selected = self.selected.contains(&entry.id);
            if selected {
                painter.rect_stroke(
                    rect,
                    TILE_RADIUS,
                    Stroke::new(3.0, ACCENT),
                    StrokeKind::Inside,
                );
            }
            if selected || self.select_mode {
                draw_checkbox(
                    &painter,
                    pos2(rect.left() + 16.0, rect.top() + 16.0),
                    selected,
                );
            }

            if hovered {
                painter.rect_stroke(
                    rect,
                    TILE_RADIUS,
                    Stroke::new(2.0, ACCENT),
                    StrokeKind::Inside,
                );

                // Favorite star in the top-right corner. Added after the tile so it
                // sits on top and receives the click instead of the tile.
                let star_rect = Rect::from_center_size(
                    pos2(rect.right() - 20.0, rect.top() + 20.0),
                    Vec2::splat(30.0),
                );
                let star_resp = ui
                    .interact(star_rect, tile_id.with("star"), Sense::click())
                    .on_hover_text(if entry.favorite {
                        "Remove from favorites"
                    } else {
                        "Add to favorites"
                    });
                let bg = if star_resp.hovered() {
                    Color32::from_black_alpha(220)
                } else {
                    Color32::from_black_alpha(160)
                };
                painter.circle_filled(star_rect.center(), 15.0, bg);
                draw_star(
                    &painter,
                    star_rect.center(),
                    10.0,
                    entry.favorite,
                    star_resp.hovered(),
                );
                if star_resp.clicked() {
                    actions.push(Action::ToggleFavorite(entry.id));
                }
            }

            if ctrl {
                // Selection was already handled on press (see above).
            } else if response.clicked() && self.select_mode {
                actions.push(Action::ToggleSelect(entry.id));
            } else if response.clicked() {
                actions.push(Action::Copy(entry.id));
            }
            // Scroll-wheel click (middle button): enlarged preview.
            if response.middle_clicked() {
                actions.push(Action::Preview(entry.id));
            }

            let has_label = entry.label.is_some();
            let is_fav = entry.favorite;
            let is_nsfw = entry.nsfw;
            let is_warning = entry.warning;
            response.context_menu(|ui| {
                if ui.button("Copy").clicked() {
                    actions.push(Action::Copy(entry.id));
                    ui.close();
                }
                if ui
                    .button("View larger")
                    .on_hover_text("Or click the GIF with the scroll wheel")
                    .clicked()
                {
                    actions.push(Action::Preview(entry.id));
                    ui.close();
                }
                let fav_text = if is_fav {
                    "Remove from favorites"
                } else {
                    "Add to favorites"
                };
                if ui.button(fav_text).clicked() {
                    actions.push(Action::ToggleFavorite(entry.id));
                    ui.close();
                }
                let nsfw_text = if is_nsfw {
                    "Unmark as NSFW"
                } else {
                    "Mark as NSFW"
                };
                if ui.button(nsfw_text).clicked() {
                    actions.push(Action::ToggleNsfw(entry.id));
                    ui.close();
                }
                let warning_text = if is_warning {
                    "Unmark as Warning"
                } else {
                    "Mark as Warning"
                };
                if ui
                    .button(warning_text)
                    .on_hover_text("Censored with an orange WARNING label; shown while hovered")
                    .clicked()
                {
                    actions.push(Action::ToggleWarning(entry.id));
                    ui.close();
                }
                if ui.button("Edit label…").clicked() {
                    actions.push(Action::EditLabel(entry.id));
                    ui.close();
                }
                if ui.button("Edit tags…").clicked() {
                    actions.push(Action::EditTags(entry.id));
                    ui.close();
                }
                if has_label && ui.button("Reset label to file name").clicked() {
                    actions.push(Action::ResetLabel(entry.id));
                    ui.close();
                }
                ui.separator();
                if ui.button("Remove from library").clicked() {
                    actions.push(Action::AskRemove(entry.id));
                    ui.close();
                }
            });
        }

        if hovered_id.is_none() {
            self.hover_start = None;
        }
        if let Some(dt) = next_repaint {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(dt.max(0.005)));
        }
        actions
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Copy(id) => self.copy_gif(ctx, id),
            Action::Preview(id) => {
                if let Some(e) = self.library.get(id) {
                    self.loader.request_preview(id, e.path.clone());
                    self.preview = Some(Preview {
                        id,
                        state: GifState::Loading,
                        reveal: false,
                        just_opened: true,
                    });
                }
            }
            Action::ToggleSelect(id) => {
                if !self.selected.remove(&id) {
                    self.selected.insert(id);
                }
            }
            Action::ToggleFavorite(id) => self.library.toggle_favorite(id),
            Action::ToggleNsfw(id) => self.library.toggle_nsfw(id),
            Action::ToggleWarning(id) => self.library.toggle_warning(id),
            Action::EditLabel(id) => {
                if let Some(e) = self.library.get(id) {
                    self.editing = Some((id, e.display_name().to_string()));
                }
            }
            Action::EditTags(id) => {
                if let Some(e) = self.library.get(id) {
                    self.tag_edit = Some(TagEdit {
                        id,
                        tags: e.tags.clone(),
                        input: String::new(),
                    });
                }
            }
            Action::ResetLabel(id) => self.library.set_label(id, ""),
            Action::AskRemove(id) => {
                self.delete_originals = false;
                self.confirm_remove = Some(id);
            }
        }
    }

    fn edit_label_modal(&mut self, ctx: &egui::Context) {
        let Some((id, text)) = &mut self.editing else {
            return;
        };
        let id = *id;
        let mut save = false;
        let mut cancel = false;

        let modal = egui::Modal::new(Id::unique("edit_label")).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.heading("Edit label");
            ui.add_space(6.0);
            let resp = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
            if !resp.has_focus() && !resp.lost_focus() {
                resp.request_focus();
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                save = true;
            }
            ui.label(
                egui::RichText::new("Leave empty to use the file name.")
                    .small()
                    .color(Color32::GRAY),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });

        if save {
            let text = text.clone();
            self.library.set_label(id, &text);
            self.editing = None;
        } else if cancel || modal.should_close() {
            self.editing = None;
        }
    }

    fn confirm_remove_modal(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_remove else {
            return;
        };
        let Some(name) = self.library.get(id).map(|e| e.display_name().to_string()) else {
            self.confirm_remove = None;
            return;
        };
        let known = self.library.get(id).and_then(|e| e.source.clone());
        let delete_originals = &mut self.delete_originals;
        let mut remove = false;
        let mut cancel = false;

        let modal = egui::Modal::new(Id::unique("confirm_remove")).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.heading("Remove GIF?");
            ui.add_space(6.0);
            ui.label(format!(
                "\"{name}\" will be deleted from your LocalGif library folder."
            ));
            ui.add_space(6.0);
            delete_originals_checkbox(ui, delete_originals, &[known]);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Remove").clicked() {
                    remove = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });

        if remove {
            let report = self.library.remove_many(&[id], self.delete_originals);
            self.cache.remove(&id);
            self.show_toast_for(
                ctx,
                report
                    .summary()
                    .replacen("1 GIF", &format!("\"{name}\""), 1),
                5.0,
            );
            self.confirm_remove = None;
        } else if cancel || modal.should_close() {
            self.confirm_remove = None;
        }
    }

    fn edit_tags_modal(&mut self, ctx: &egui::Context) {
        let Some(edit) = &mut self.tag_edit else {
            return;
        };
        let name = self
            .library
            .get(edit.id)
            .map(|e| e.display_name().to_string())
            .unwrap_or_default();
        let all_tags = self.library.all_tags();
        let mut save = false;

        // Left: current tags, the input box and Save. Right: your other tags to pick from.
        let total = (ctx.content_rect().width() - 80.0).clamp(280.0, 520.0);
        let (left_w, right_w) = (total * 0.6, total * 0.4 - 12.0);

        let modal = egui::Modal::new(Id::unique("edit_tags")).show(ctx, |ui| {
            ui.heading(format!("Tags for \"{name}\""));
            ui.add_space(6.0);

            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(left_w);

                    // Current tags; click one to remove it.
                    if edit.tags.is_empty() {
                        ui.label(egui::RichText::new("No tags yet.").color(Color32::GRAY));
                    } else {
                        let mut remove = None;
                        ui.horizontal_wrapped(|ui| {
                            for (i, tag) in edit.tags.iter().enumerate() {
                                if ui
                                    .button(format!("{tag}  ✕"))
                                    .on_hover_text("Remove tag")
                                    .clicked()
                                {
                                    remove = Some(i);
                                }
                            }
                        });
                        if let Some(i) = remove {
                            edit.tags.remove(i);
                        }
                    }
                    ui.add_space(6.0);

                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut edit.input)
                            .hint_text("Add tags (Enter to add, comma for several)")
                            .desired_width(f32::INFINITY),
                    );
                    if !resp.has_focus() && !resp.lost_focus() {
                        resp.request_focus();
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        if edit.input.trim().is_empty() {
                            save = true; // Enter on an empty field saves.
                        } else {
                            add_tags(&mut edit.tags, &edit.input);
                            edit.input.clear();
                        }
                        resp.request_focus();
                    }

                    ui.add_space(8.0);
                    if ui.button("Save").clicked() {
                        save = true;
                    }
                });

                ui.separator();

                ui.vertical(|ui| {
                    ui.set_width(right_w);
                    ui.label(
                        egui::RichText::new("Your tags (click to add)")
                            .small()
                            .color(Color32::GRAY),
                    );
                    // Existing tags from other GIFs, filtered by what's typed.
                    let typed = edit.input.trim().trim_start_matches('#').to_lowercase();
                    let suggestions: Vec<&String> = all_tags
                        .iter()
                        .filter(|t| !edit.tags.iter().any(|x| x.eq_ignore_ascii_case(t)))
                        .filter(|t| typed.is_empty() || t.to_lowercase().contains(&typed))
                        .collect();
                    if suggestions.is_empty() {
                        let text = if all_tags.is_empty() {
                            "No tags yet"
                        } else {
                            "No matching tags"
                        };
                        ui.label(egui::RichText::new(text).color(Color32::GRAY));
                    }
                    let mut picked = None;
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for tag in suggestions {
                                if ui
                                    .add(
                                        egui::Button::new(format!("#{tag}"))
                                            .frame(false)
                                            .truncate(),
                                    )
                                    .on_hover_text("Add this tag")
                                    .clicked()
                                {
                                    picked = Some(tag.clone());
                                }
                            }
                        });
                    if let Some(tag) = picked {
                        edit.tags.push(tag);
                        edit.input.clear();
                    }
                });
            });
        });

        if save {
            let mut tags = edit.tags.clone();
            add_tags(&mut tags, &edit.input); // Don't lose text that wasn't added with Enter.
            let id = edit.id;
            self.library.set_tags(id, &tags);
            self.tag_edit = None;
        } else if modal.should_close() {
            // Esc / clicking outside closes without saving.
            self.tag_edit = None;
        }
    }

    /// "Already in your library: replace or skip?" for each conflicting file.
    fn conflict_modal(&mut self, ctx: &egui::Context) {
        let Some(conflict) = self.conflicts.front() else {
            return;
        };
        let Some(existing_name) = self
            .library
            .get(conflict.existing)
            .map(|e| e.display_name().to_string())
        else {
            self.conflicts.pop_front(); // Removed in the meantime.
            return;
        };
        let new_name = conflict
            .src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let remaining = self.conflicts.len() - 1;
        let why = match conflict.kind {
            ConflictKind::Content => "is identical to",
            ConflictKind::Name => "has the same file name as",
            ConflictKind::File => "is the library copy of",
        };

        let mut choice: Option<bool> = None; // Some(true) = replace, Some(false) = skip.
        let from_pack = conflict.meta.is_some();
        let apply_all = &mut self.conflict_apply_all;
        egui::Modal::new(Id::unique("conflict")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading("Already in your library");
            ui.add_space(6.0);
            ui.label(format!("\"{new_name}\" {why} \"{existing_name}\"."));
            ui.label(
                egui::RichText::new(if from_pack {
                    "Replacing uses the pack's version: its file, label, tags and flags."
                } else {
                    "Replacing updates the GIF file and keeps its label, tags and favorite."
                })
                .small()
                .color(Color32::GRAY),
            );
            if remaining > 0 {
                ui.add_space(4.0);
                ui.checkbox(apply_all, format!("Do the same for the other {remaining}"));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Replace").clicked() {
                    choice = Some(true);
                }
                if ui.button("Keep existing").clicked() {
                    choice = Some(false);
                }
            });
        });
        // Escape / clicking outside counts as "Keep existing" for this one.
        if choice.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            choice = Some(false);
        }
        let Some(replace) = choice else {
            return;
        };

        let batch: Vec<Conflict> = if self.conflict_apply_all {
            self.conflicts.drain(..).collect()
        } else {
            self.conflicts.pop_front().into_iter().collect()
        };
        if !replace {
            return;
        }
        let mut replaced = 0;
        let mut failed = Vec::new();
        for c in batch {
            match self.library.replace(c.existing, &c.src, c.meta.as_ref()) {
                Ok(()) => {
                    self.cache.remove(&c.existing); // Reload the new version.
                    replaced += 1;
                }
                Err(e) => failed.push(e),
            }
        }
        match failed.first() {
            None if replaced == 1 => self.show_toast(ctx, "Replaced 1 GIF"),
            None => self.show_toast(ctx, format!("Replaced {replaced} GIFs")),
            Some(e) => self.show_toast_for(ctx, format!("Replace failed: {e}"), 6.0),
        }
    }

    fn toast(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let Some(toast) = &self.toast else {
            return;
        };
        if now >= toast.until {
            self.toast = None;
            return;
        }
        egui::Area::new(Id::unique("toast"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -16.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    // Short messages stay on one line; long ones wrap to the window width.
                    let max_width = ctx.content_rect().width() - 48.0;
                    let galley = egui::WidgetText::from(toast.text.as_str()).into_galley(
                        ui,
                        Some(egui::TextWrapMode::Extend),
                        f32::INFINITY,
                        egui::TextStyle::Body,
                    );
                    if galley.size().x > max_width {
                        ui.set_max_width(max_width);
                        ui.add(egui::Label::new(&toast.text).wrap());
                    } else {
                        ui.add(egui::Label::new(&toast.text).extend());
                    }
                });
            });
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(toast.until - now));
    }
}

impl eframe::App for LocalGifApp {
    /// Applies the scroll speed setting to mouse-wheel / touchpad scrolling.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let speed = self.library.settings.scroll_speed;
        if (speed - 1.0).abs() > f32::EPSILON {
            for event in &mut raw_input.events {
                if let egui::Event::MouseWheel { delta, .. } = event {
                    *delta *= speed;
                }
            }
        }
    }

    /// Runs every frame, and also while the window is hidden
    /// (when woken by the tray icon, the global shortcut or `localgif --toggle`).
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.widget.visible {
            let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
            if self.widget.focused && !focused {
                self.widget.unfocused_at = Some(Instant::now());
            }
            self.widget.focused = focused;
        }

        let events: Vec<AppEvent> = self.bg.event_rx.try_iter().collect();
        for event in events {
            match event {
                AppEvent::TrayClick { x, y } => {
                    if self.widget.visible {
                        self.hide_window(ctx);
                    } else {
                        self.show_window(ctx, Some((x, y)));
                    }
                }
                AppEvent::Toggle => {
                    // In front: hide. Hidden, minimized or behind other windows: bring it up.
                    // On X11, pressing a global shortcut briefly takes focus away from the
                    // window, so a focus loss just now still counts as "in front".
                    let just_unfocused = self
                        .widget
                        .unfocused_at
                        .is_some_and(|t| t.elapsed() < Duration::from_millis(500));
                    if self.widget.visible && (self.widget.focused || just_unfocused) {
                        self.hide_window(ctx);
                    } else {
                        self.show_window(ctx, None);
                    }
                }
                AppEvent::Show => self.show_window(ctx, None),
                AppEvent::Quit => {
                    self.widget.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                AppEvent::PackExported { result } => match result {
                    Ok(msg) => self.show_toast_for(ctx, msg, 6.0),
                    Err(e) => self.show_toast_for(ctx, format!("Export failed: {e}"), 8.0),
                },
                AppEvent::PackExtracted { result } => match result {
                    Ok((items, temp)) => self.finish_import(ctx, items, temp),
                    Err(e) => self.show_toast_for(ctx, format!("Import failed: {e}"), 8.0),
                },
                AppEvent::Converted { id, result } => match result {
                    Ok(gif) => self.put_on_clipboard(ctx, id, gif),
                    Err(e) => {
                        self.show_toast_for(ctx, format!("Couldn't convert to GIF: {e}"), 6.0)
                    }
                },
            }
        }

        // With a tray icon, the window's close button hides it instead of quitting.
        if self.bg.tray.is_some()
            && ctx.input(|i| i.viewport().close_requested())
            && !self.widget.quitting
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window(ctx);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_decoded(&ctx);

        // Drag & drop GIF files (or folders of GIFs) onto the window to add them.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        // Dropped GIF packs are imported; everything else is added as GIFs.
        let (packs, dropped): (Vec<PathBuf>, Vec<PathBuf>) = dropped.into_iter().partition(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(crate::pack::EXTENSION))
        });
        for p in packs {
            self.start_import(&ctx, Some(p));
        }
        self.add_files(&ctx, dropped);

        // Pick up files copied into the library folder while the app was in the background.
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if focused && !self.had_focus {
            self.library.refresh();
        }
        self.had_focus = focused;

        // "/" jumps to the search bar (unless you're already typing somewhere).
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing
            && !self.any_dialog_open()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Slash))
        {
            // Drop the "/" character so it isn't typed into the search bar.
            ctx.input_mut(|i| {
                i.events
                    .retain(|e| !matches!(e, egui::Event::Text(t) if t == "/"))
            });
            ctx.memory_mut(|m| m.request_focus(search_id()));
        }

        // Forget selected GIFs that were removed.
        let lib = &self.library;
        self.selected.retain(|id| lib.get(*id).is_some());

        let panel_frame = egui::Frame::new()
            .fill(ctx.global_style().visuals.panel_fill)
            .inner_margin(egui::Margin::symmetric(12, 0));
        let mut actions = Vec::new();
        let mut visible_ids = HashSet::new();

        if self.page == Page::Settings {
            egui::CentralPanel::default()
                .frame(panel_frame)
                .show(ui, |ui| self.settings_page(ui, &mut visible_ids));
        } else {
            egui::Panel::top("top_bar")
                .frame(panel_frame)
                .show(ui, |ui| self.top_bar(ui));

            if self.select_mode || !self.selected.is_empty() {
                egui::Panel::bottom("bulk_bar")
                    .frame(panel_frame.inner_margin(egui::Margin::symmetric(12, 8)))
                    .show(ui, |ui| {
                        let shown = self.last_shown.clone();
                        self.bulk_bar(ui, &shown, true);
                    });
            }

            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(ctx.global_style().visuals.panel_fill)
                        .inner_margin(egui::Margin::symmetric(12, 0)),
                )
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            actions = self.gif_grid(ui, &mut visible_ids);
                            ui.add_space(12.0);
                        });
                });
        }

        for action in actions {
            self.apply(&ctx, action);
        }

        // Free memory: drop queued jobs for GIFs that scrolled away and
        // textures of GIFs that have been off-screen for a while.
        self.loader
            .retain(|id| visible_ids.contains(&id) || id & PREVIEW_BIT != 0);
        let now = ctx.input(|i| i.time);
        self.cache.retain(|id, item| {
            if visible_ids.contains(id) {
                return true;
            }
            match item.state {
                // Its job was just dropped from the queue (or is already running);
                // forget it so it is requested again when it becomes visible.
                GifState::Loading => false,
                _ => now - item.last_seen < EVICT_AFTER,
            }
        });

        // Esc closes the widget, unless a dialog is open (Esc closes that instead).
        let no_dialog = !self.any_dialog_open();

        self.edit_label_modal(&ctx);
        self.edit_tags_modal(&ctx);
        self.confirm_remove_modal(&ctx);
        self.conflict_modal(&ctx);
        self.bulk_modal(&ctx);
        self.preview_modal(&ctx);
        self.cleanup_import_temp();
        self.toast(&ctx);

        // Esc leaves Settings / clears the selection before it hides the widget.
        let mut esc_used = false;
        if no_dialog
            && (self.page == Page::Settings || self.select_mode || !self.selected.is_empty())
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            if self.page == Page::Settings {
                self.page = Page::Grid;
            } else {
                self.select_mode = false;
                self.selected.clear();
            }
            esc_used = true;
        }

        if std::mem::take(&mut self.widget.start_hidden) {
            self.hide_window(&ctx);
        }
        if self.bg.tray.is_some()
            && no_dialog
            && !esc_used
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.hide_window(&ctx);
        }
    }
}

/// Returns the cache entry for a GIF, starting to decode it if needed.
fn ensure_loaded<'a>(
    cache: &'a mut HashMap<u64, CacheItem>,
    loader: &Loader,
    entry: &GifEntry,
    now: f64,
) -> &'a mut CacheItem {
    let item = cache.entry(entry.id).or_insert_with(|| {
        loader.request(entry.id, entry.path.clone());
        CacheItem {
            state: GifState::Loading,
            last_seen: now,
        }
    });
    item.last_seen = now;
    item
}

/// Round selection checkbox drawn on a tile.
fn draw_checkbox(painter: &egui::Painter, center: Pos2, checked: bool) {
    if checked {
        painter.circle_filled(center, 10.0, ACCENT);
        painter.add(egui::Shape::line(
            vec![
                center + vec2(-4.5, 0.0),
                center + vec2(-1.0, 3.5),
                center + vec2(5.0, -3.5),
            ],
            Stroke::new(2.0, Color32::WHITE),
        ));
    } else {
        painter.circle_filled(center, 10.0, Color32::from_black_alpha(140));
        painter.circle_stroke(center, 9.0, Stroke::new(1.5, Color32::WHITE));
    }
}

/// Red "Also delete the original file" checkbox for the delete dialogs.
/// `sources` are the original files of the GIFs being deleted (`None` = unknown).
fn delete_originals_checkbox(ui: &mut egui::Ui, checked: &mut bool, sources: &[Option<PathBuf>]) {
    let known = sources.iter().flatten().filter(|p| p.is_file()).count();
    let label = if sources.len() == 1 {
        "Also delete the original file"
    } else {
        "Also delete the original files"
    };
    ui.add_enabled_ui(known > 0, |ui| {
        ui.checkbox(checked, egui::RichText::new(label).color(NSFW_RED).strong());
    });
    let note = if known == 0 {
        "LocalGif doesn't know where this was added from (added before it kept track, \
         or the original is already gone)."
            .to_string()
    } else if sources.len() == 1 {
        let path = sources[0].as_ref().unwrap();
        format!("Moves {} to the Trash.", path.display())
    } else if known < sources.len() {
        format!(
            "Moves {known} original file(s) to the Trash. The other {} weren't found \
             (added before LocalGif kept track, or already gone).",
            sources.len() - known
        )
    } else {
        format!("Moves the {known} original files to the Trash.")
    };
    ui.label(egui::RichText::new(note).small().color(Color32::GRAY));
}

fn search_id() -> Id {
    Id::unique("search")
}

/// Adds comma-separated tags from `input` to `tags`, skipping duplicates (ignoring case).
fn add_tags(tags: &mut Vec<String>, input: &str) {
    for t in input.split(',') {
        let t = t.trim().trim_start_matches('#').trim();
        if !t.is_empty() && !tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            tags.push(t.to_string());
        }
    }
}

/// UV rect that crops the texture so it fills `target` without stretching ("object-fit: cover").
fn cover_uv(tex: Vec2, target: Vec2) -> Rect {
    let tex_aspect = tex.x / tex.y;
    let target_aspect = target.x / target.y;
    if tex_aspect > target_aspect {
        let w = target_aspect / tex_aspect;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = tex_aspect / target_aspect;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    }
}

/// Draws the label on a dark strip along the bottom of the tile.
fn draw_label(painter: &egui::Painter, rect: Rect, text: &str, tags: &[String]) {
    let mut job = LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat::simple(FontId::proportional(12.5), Color32::WHITE),
    );
    if !tags.is_empty() {
        let tags: Vec<String> = tags.iter().map(|t| format!("#{t}")).collect();
        job.append(
            &format!("   {}", tags.join(" ")),
            0.0,
            egui::TextFormat::simple(FontId::proportional(11.0), Color32::from_gray(170)),
        );
    }
    job.wrap = egui::text::TextWrapping {
        max_width: rect.width() - 12.0,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = painter.layout_job(job);
    let strip_h = galley.size().y + 8.0;
    let strip = Rect::from_min_max(pos2(rect.left(), rect.bottom() - strip_h), rect.max);
    painter.rect_filled(
        strip,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: TILE_RADIUS,
            se: TILE_RADIUS,
        },
        Color32::from_black_alpha(150),
    );
    painter.galley(
        pos2(strip.left() + 6.0, strip.top() + 4.0),
        galley,
        Color32::WHITE,
    );
}

/// "NSFW" / "WARNING" pill in the middle of a censored tile.
/// "NSFW" / "WARNING" pills in the middle of a censored tile, stacked when it has both.
fn draw_badges(painter: &egui::Painter, rect: Rect, nsfw: bool, warning: bool) {
    let badges: Vec<(&str, Color32)> = [
        (nsfw, "NSFW", NSFW_RED),
        (warning, "WARNING", WARNING_ORANGE),
    ]
    .into_iter()
    .filter(|(on, ..)| *on)
    .map(|(_, text, color)| (text, color))
    .collect();

    let gap = 6.0;
    let galleys: Vec<_> = badges
        .iter()
        .map(|(text, _)| {
            painter.layout_no_wrap(text.to_string(), FontId::proportional(16.0), Color32::WHITE)
        })
        .collect();
    let pill_h = galleys.first().map_or(0.0, |g| g.size().y) + 8.0;
    let total_h = badges.len() as f32 * pill_h + (badges.len().saturating_sub(1)) as f32 * gap;
    let mut y = rect.center().y - total_h / 2.0;
    for ((_, color), galley) in badges.iter().zip(galleys) {
        let pill = Rect::from_center_size(
            pos2(rect.center().x, y + pill_h / 2.0),
            galley.size() + vec2(20.0, 8.0),
        );
        painter.rect_filled(pill, CornerRadius::same(255), *color);
        painter.galley(pill.center() - galley.size() / 2.0, galley, Color32::WHITE);
        y += pill_h + gap;
    }
}

/// Five-pointed star, filled when `filled` is true, outlined otherwise.
fn draw_star(painter: &egui::Painter, center: Pos2, radius: f32, filled: bool, hovered: bool) {
    let inner = radius * 0.45;
    let points: Vec<Pos2> = (0..10)
        .map(|i| {
            let r = if i % 2 == 0 { radius } else { inner };
            let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            center + vec2(angle.cos(), angle.sin()) * r
        })
        .collect();

    if filled {
        // A star is not convex, so build it as a triangle fan around the center.
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(center, STAR_YELLOW);
        for p in &points {
            mesh.colored_vertex(*p, STAR_YELLOW);
        }
        for i in 0..10u32 {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % 10);
        }
        painter.add(mesh);
    } else {
        let color = if hovered { STAR_YELLOW } else { Color32::WHITE };
        painter.add(egui::Shape::closed_line(points, Stroke::new(1.5, color)));
    }
}

fn setup_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        let v = &mut style.visuals;
        v.panel_fill = Color32::from_rgb(43, 45, 49);
        v.window_fill = Color32::from_rgb(49, 51, 56);
        v.extreme_bg_color = TILE_BG;
        v.selection.bg_fill = ACCENT;
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(64, 66, 73);
        style.spacing.button_padding = vec2(10.0, 5.0);
    });
}
