//! The settings page: options, all tags with counts, and a list of every GIF.

use super::{
    GifState, LocalGifApp, NSFW_RED, Page, STAR_YELLOW, SettingsTab, WARNING_ORANGE, ensure_loaded,
};
use eframe::egui::{self, Color32, RichText, Sense, vec2};
use std::collections::HashSet;

const ROW_HEIGHT: f32 = 44.0;
const THUMB_SIZE: f32 = 40.0;

impl LocalGifApp {
    pub(super) fn settings_page(&mut self, ui: &mut egui::Ui, visible_ids: &mut HashSet<u64>) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("← Back").clicked() {
                self.page = Page::Grid;
            }
            ui.heading("Settings");
        });
        ui.add_space(6.0);

        let tag_count = self.library.all_tags().len();
        let gif_count = self.library.entries.len();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.settings_tab, SettingsTab::General, "General");
            ui.selectable_value(
                &mut self.settings_tab,
                SettingsTab::Tags,
                format!("Tags ({tag_count})"),
            );
            ui.selectable_value(
                &mut self.settings_tab,
                SettingsTab::Gifs,
                format!("GIFs ({gif_count})"),
            );
        });
        ui.separator();

        match self.settings_tab {
            SettingsTab::General => self.settings_general(ui),
            SettingsTab::Tags => self.settings_tags(ui),
            SettingsTab::Gifs => self.settings_gifs(ui, visible_ids),
        }
    }

    fn settings_general(&mut self, ui: &mut egui::Ui) {
        let mut s = self.library.settings.clone();
        let mut save = false;

        egui::Grid::new("settings_general")
            .num_columns(2)
            .spacing([16.0, 12.0])
            .show(ui, |ui| {
                ui.label("Scroll speed");
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::Slider::new(&mut s.scroll_speed, 0.25..=4.0)
                            .step_by(0.05)
                            .suffix("×"),
                    );
                    // Save when the user lets go of the slider, not on every frame of the drag.
                    save |= resp.drag_stopped() || (resp.changed() && !resp.dragged());
                    if ui.small_button("Reset").clicked() {
                        s.scroll_speed = 1.0;
                        save = true;
                    }
                });
                ui.end_row();

                ui.label("NSFW GIFs");
                save |= ui
                    .checkbox(&mut s.show_nsfw, "Show NSFW GIFs")
                    .on_hover_text("Unchecked: NSFW GIFs are hidden completely")
                    .changed();
                ui.end_row();

                ui.label("");
                ui.add_enabled_ui(s.show_nsfw, |ui| {
                    save |= ui
                        .checkbox(&mut s.uncensor_nsfw, "Don't censor NSFW GIFs")
                        .on_hover_text("Show NSFW GIFs normally instead of blurred")
                        .on_disabled_hover_text("Only applies while NSFW GIFs are shown")
                        .changed();
                });
                ui.end_row();

                ui.label("Warning delay");
                ui.horizontal(|ui| {
                    let resp = ui
                        .add(
                            egui::Slider::new(&mut s.warning_delay, 0.0..=5.0)
                                .step_by(0.1)
                                .fixed_decimals(1)
                                .suffix(" s"),
                        )
                        .on_hover_text("How long to hover over a Warning GIF before it's shown");
                    save |= resp.drag_stopped() || (resp.changed() && !resp.dragged());
                });
                ui.end_row();

                ui.label("Animation");
                save |= ui
                    .checkbox(&mut s.hover_to_play, "Play GIFs only while hovered")
                    .changed();
                ui.end_row();

                if crate::autostart::SUPPORTED {
                    ui.label("Startup");
                    let mut on = self.autostart_on;
                    if ui
                        .checkbox(&mut on, "Start LocalGif when I log in")
                        .on_hover_text(
                            "Starts in the tray (or minimized if there's no tray) when you log in",
                        )
                        .changed()
                    {
                        match crate::autostart::set_enabled(on) {
                            Ok(()) => self.autostart_on = crate::autostart::is_enabled(),
                            Err(e) => {
                                let ctx = ui.ctx().clone();
                                self.show_toast_for(
                                    &ctx,
                                    format!("Couldn't change startup setting: {e}"),
                                    6.0,
                                );
                            }
                        }
                    }
                    ui.end_row();
                }
            });

        // Apply right away (the slider is live); write to disk when `save`.
        self.library.settings = s.clone();
        if save {
            self.library.set_settings(s);
        }

        ui.add_space(16.0);
        ui.label(RichText::new("GIF packs").strong());
        ui.label(
            RichText::new(
                "Move GIFs to another device: a pack is one file with the GIFs (unchanged, \
                 no quality loss) plus their labels, tags, favorites and NSFW/Warning flags. \
                 To export only some GIFs, select them and use Export… in the selection bar.",
            )
            .small()
            .color(Color32::GRAY),
        );
        ui.horizontal(|ui| {
            let ctx = ui.ctx().clone();
            if ui
                .add_enabled(
                    !self.library.entries.is_empty(),
                    egui::Button::new(format!("Export all {} GIFs…", self.library.entries.len())),
                )
                .clicked()
            {
                let ids: Vec<u64> = self.library.entries.iter().map(|e| e.id).collect();
                self.start_export(&ctx, &ids);
            }
            if ui.button("Import a pack…").clicked() {
                self.start_import(&ctx, None);
            }
        });

        ui.add_space(16.0);
        let shortcut = if self.library.hotkey.trim().is_empty() {
            "off".to_string()
        } else {
            self.library.hotkey.clone()
        };
        ui.label(
            RichText::new(format!(
                "Global shortcut: {shortcut}  (change \"hotkey\" in library.json)"
            ))
            .small()
            .color(Color32::GRAY),
        );
        ui.label(
            RichText::new(format!("GIF folder: {}", self.library.gif_dir.display()))
                .small()
                .color(Color32::GRAY),
        );
    }

    fn settings_tags(&mut self, ui: &mut egui::Ui) {
        let counts = self.library.tag_counts();
        if counts.is_empty() {
            ui.add_space(20.0);
            ui.label(
                RichText::new("No tags yet. Right-click a GIF → Edit tags… to add some.")
                    .color(Color32::GRAY),
            );
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("tag_counts")
                    .num_columns(2)
                    .striped(true)
                    .spacing([40.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("Tag").strong());
                        ui.label(RichText::new("GIFs").strong());
                        ui.end_row();
                        for (tag, count) in counts {
                            ui.label(format!("#{tag}"));
                            ui.label(count.to_string());
                            ui.end_row();
                        }
                    });
            });
    }

    fn settings_gifs(&mut self, ui: &mut egui::Ui, visible_ids: &mut HashSet<u64>) {
        let s_uncensor = self.library.settings.uncensor_nsfw;
        ui.add(
            egui::TextEdit::singleline(&mut self.settings_search)
                .hint_text("Search by label, file name or tag")
                .desired_width(f32::INFINITY),
        );
        let query = self.settings_search.trim().to_lowercase();
        // Every GIF, including NSFW ones hidden in the grid.
        let rows: Vec<u64> = self
            .library
            .entries
            .iter()
            .filter(|e| {
                query.is_empty()
                    || e.display_name().to_lowercase().contains(&query)
                    || e.file_name.to_lowercase().contains(&query)
                    || e.tags.iter().any(|t| t.to_lowercase().contains(&query))
            })
            .map(|e| e.id)
            .collect();

        ui.add_space(4.0);
        self.bulk_bar(ui, &rows, false);
        ui.separator();

        let now = ui.input(|i| i.time);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                for &id in &rows[range] {
                    let Some(entry) = self.library.entries.iter().find(|e| e.id == id) else {
                        continue;
                    };
                    visible_ids.insert(id);
                    let item = ensure_loaded(&mut self.cache, &self.loader, entry, now);

                    ui.horizontal(|ui| {
                        ui.set_height(ROW_HEIGHT);
                        let mut checked = self.selected.contains(&id);
                        if ui.checkbox(&mut checked, "").changed() {
                            if checked {
                                self.selected.insert(id);
                            } else {
                                self.selected.remove(&id);
                            }
                        }

                        // Thumbnail: first frame, or the blurred version for NSFW / Warning.
                        let (rect, _) =
                            ui.allocate_exact_size(vec2(THUMB_SIZE, THUMB_SIZE), Sense::hover());
                        ui.painter().rect_filled(rect, 4, super::TILE_BG);
                        if let GifState::Ready(gif) = &item.state {
                            let censor = (entry.nsfw && !s_uncensor) || entry.warning;
                            let tex = if censor {
                                &gif.censored
                            } else {
                                &gif.textures[0]
                            };
                            egui::Image::from_texture(tex)
                                .uv(super::cover_uv(tex.size_vec2(), rect.size()))
                                .corner_radius(4)
                                .paint_at(ui, rect);
                        }

                        ui.vertical(|ui| {
                            ui.add_space(3.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(entry.display_name()).strong());
                                if entry.favorite {
                                    ui.label(RichText::new("★").color(STAR_YELLOW));
                                }
                                if entry.nsfw {
                                    ui.label(RichText::new("NSFW").small().color(NSFW_RED));
                                }
                                if entry.warning {
                                    ui.label(
                                        RichText::new("WARNING").small().color(WARNING_ORANGE),
                                    );
                                }
                            });
                            let mut details = entry.file_name.clone();
                            if entry.uses > 0 {
                                details += &format!(" · copied {}×", entry.uses);
                            }
                            if !entry.tags.is_empty() {
                                let tags: Vec<String> =
                                    entry.tags.iter().map(|t| format!("#{t}")).collect();
                                details += &format!(" · {}", tags.join(" "));
                            }
                            ui.add(
                                egui::Label::new(
                                    RichText::new(details).small().color(Color32::GRAY),
                                )
                                .truncate(),
                            );
                        });
                    });
                }
            });
    }
}
