//! Bulk selection: rename, tag or delete several GIFs at once.

use super::{LocalGifApp, NSFW_RED, add_tags};
use eframe::egui::{self, Color32, Id, RichText};

pub enum BulkDialog {
    Rename {
        label: String,
        numbered: bool,
    },
    Tags {
        add: Vec<String>,
        input: String,
        remove: Vec<String>,
    },
    Delete,
}

impl LocalGifApp {
    /// Selected GIF ids, in library order.
    fn selected_ids(&self) -> Vec<u64> {
        self.library
            .entries
            .iter()
            .filter(|e| self.selected.contains(&e.id))
            .map(|e| e.id)
            .collect()
    }

    /// "N selected · Select all · None · Rename… · Tags… · Delete…".
    /// `shown` is what "Select all" selects. `in_grid` adds a "Done" button.
    pub(super) fn bulk_bar(&mut self, ui: &mut egui::Ui, shown: &[u64], in_grid: bool) {
        let n = self.selected.len();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{n} selected")).strong());
            if ui.small_button("Select all").clicked() {
                self.selected.extend(shown.iter().copied());
            }
            if ui.small_button("None").clicked() {
                self.selected.clear();
            }
            ui.separator();
            ui.add_enabled_ui(n > 0, |ui| {
                if ui.button("Rename…").clicked() {
                    self.bulk = Some(BulkDialog::Rename {
                        label: String::new(),
                        numbered: n > 1,
                    });
                }
                if ui.button("Tags…").clicked() {
                    self.bulk = Some(BulkDialog::Tags {
                        add: Vec::new(),
                        input: String::new(),
                        remove: Vec::new(),
                    });
                }
                if ui
                    .button(RichText::new("Delete…").color(NSFW_RED))
                    .clicked()
                {
                    self.bulk = Some(BulkDialog::Delete);
                }
            });
            if in_grid && ui.button("Done").clicked() {
                self.select_mode = false;
                self.selected.clear();
            }
        });
    }

    pub(super) fn bulk_modal(&mut self, ctx: &egui::Context) {
        if self.bulk.is_none() {
            return;
        }
        let ids = self.selected_ids();
        if ids.is_empty() {
            self.bulk = None;
            return;
        }
        let n = ids.len();
        let names: Vec<String> = ids
            .iter()
            .filter_map(|id| self.library.get(*id))
            .map(|e| e.display_name().to_string())
            .collect();
        let all_tags = self.library.all_tags();
        // Tags on the selected GIFs, with how many of them have each.
        let mut sel_tags: Vec<(String, usize)> = Vec::new();
        for t in ids
            .iter()
            .filter_map(|id| self.library.get(*id))
            .flat_map(|e| &e.tags)
        {
            match sel_tags.iter_mut().find(|(x, _)| x.eq_ignore_ascii_case(t)) {
                Some((_, c)) => *c += 1,
                None => sel_tags.push((t.clone(), 1)),
            }
        }
        sel_tags.sort_by_key(|(t, _)| t.to_lowercase());

        let mut apply = false;
        let mut cancel = false;
        let dialog = self.bulk.as_mut().unwrap();

        let modal = egui::Modal::new(Id::unique("bulk")).show(ctx, |ui| {
            ui.set_width(360.0);
            match dialog {
                BulkDialog::Rename { label, numbered } => {
                    ui.heading(format!("Rename {n} GIFs"));
                    ui.add_space(6.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(label)
                            .hint_text("New label")
                            .desired_width(f32::INFINITY),
                    );
                    if !resp.has_focus() && !resp.lost_focus() {
                        resp.request_focus();
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        apply = true;
                    }
                    if n > 1 {
                        ui.checkbox(numbered, "Number them (Label 1, Label 2, …)");
                    }
                    ui.add_space(4.0);
                    let preview: Vec<String> = (0..n.min(3))
                        .map(|i| match (label.trim().is_empty(), *numbered && n > 1) {
                            (true, _) => format!("{} (file name)", names[i]),
                            (false, true) => format!("{} {}", label.trim(), i + 1),
                            (false, false) => label.trim().to_string(),
                        })
                        .collect();
                    let more = if n > 3 { ", …" } else { "" };
                    ui.label(
                        RichText::new(format!("Result: {}{more}", preview.join(", ")))
                            .small()
                            .color(Color32::GRAY),
                    );
                    ui.label(
                        RichText::new("Leave empty to reset them to their file names.")
                            .small()
                            .color(Color32::GRAY),
                    );
                }
                BulkDialog::Tags { add, input, remove } => {
                    ui.heading(format!("Tags for {n} GIFs"));
                    ui.add_space(6.0);
                    ui.label(RichText::new("Add tags").strong());
                    if !add.is_empty() {
                        let mut drop = None;
                        ui.horizontal_wrapped(|ui| {
                            for (i, t) in add.iter().enumerate() {
                                if ui.button(format!("{t}  ✕")).clicked() {
                                    drop = Some(i);
                                }
                            }
                        });
                        if let Some(i) = drop {
                            add.remove(i);
                        }
                    }
                    let resp = ui.add(
                        egui::TextEdit::singleline(input)
                            .hint_text("Type a tag, Enter to add (comma for several)")
                            .desired_width(f32::INFINITY),
                    );
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        add_tags(add, input);
                        input.clear();
                        resp.request_focus();
                    }
                    let typed = input.trim().to_lowercase();
                    let suggestions: Vec<&String> = all_tags
                        .iter()
                        .filter(|t| !add.iter().any(|x| x.eq_ignore_ascii_case(t)))
                        .filter(|t| typed.is_empty() || t.to_lowercase().contains(&typed))
                        .take(20)
                        .collect();
                    if !suggestions.is_empty() {
                        let mut picked = None;
                        ui.horizontal_wrapped(|ui| {
                            for t in suggestions {
                                if ui.small_button(t.as_str()).clicked() {
                                    picked = Some(t.clone());
                                }
                            }
                        });
                        if let Some(t) = picked {
                            add.push(t);
                            input.clear();
                        }
                    }

                    if !sel_tags.is_empty() {
                        ui.add_space(8.0);
                        ui.label(RichText::new("Remove tags").strong());
                        ui.horizontal_wrapped(|ui| {
                            for (t, count) in &sel_tags {
                                let pos = remove.iter().position(|r| r.eq_ignore_ascii_case(t));
                                let mut on = pos.is_some();
                                if ui
                                    .checkbox(&mut on, format!("#{t} ({count} of {n})"))
                                    .changed()
                                {
                                    match pos {
                                        Some(i) => {
                                            remove.remove(i);
                                        }
                                        None => remove.push(t.clone()),
                                    }
                                }
                            }
                        });
                    }
                }
                BulkDialog::Delete => {
                    ui.heading(format!("Delete {n} GIFs?"));
                    ui.add_space(6.0);
                    for name in names.iter().take(6) {
                        ui.label(format!("• {name}"));
                    }
                    if n > 6 {
                        ui.label(format!("…and {} more", n - 6));
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "They're deleted from your LocalGif library folder. \
                             The original files you added are not affected.",
                        )
                        .small()
                        .color(Color32::GRAY),
                    );
                }
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let label = match dialog {
                    BulkDialog::Rename { .. } => RichText::new("Rename"),
                    BulkDialog::Tags { .. } => RichText::new("Apply"),
                    BulkDialog::Delete => RichText::new("Delete").color(NSFW_RED),
                };
                if ui.button(label).clicked() {
                    apply = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });

        if apply {
            let dialog = self.bulk.take().unwrap();
            match dialog {
                BulkDialog::Rename { label, numbered } => {
                    let label = label.trim();
                    let labels: Vec<(u64, String)> = ids
                        .iter()
                        .enumerate()
                        .map(|(i, id)| {
                            let text = if label.is_empty() {
                                String::new()
                            } else if numbered && n > 1 {
                                format!("{label} {}", i + 1)
                            } else {
                                label.to_string()
                            };
                            (*id, text)
                        })
                        .collect();
                    self.library.set_labels(&labels);
                    self.show_toast(ctx, format!("Renamed {n} GIFs"));
                }
                BulkDialog::Tags {
                    mut add,
                    input,
                    remove,
                } => {
                    add_tags(&mut add, &input); // Text typed but not added with Enter.
                    self.library.edit_tags_bulk(&ids, &add, &remove);
                    self.show_toast(ctx, format!("Updated tags on {n} GIFs"));
                }
                BulkDialog::Delete => {
                    let (removed, error) = self.library.remove_many(&ids);
                    for id in &ids {
                        self.cache.remove(id);
                    }
                    self.selected.clear();
                    match error {
                        None => self.show_toast(ctx, format!("Deleted {removed} GIFs")),
                        Some(e) => self.show_toast_for(
                            ctx,
                            format!("Deleted {removed} GIFs; some failed: {e}"),
                            6.0,
                        ),
                    }
                }
            }
        } else if cancel || modal.should_close() {
            self.bulk = None;
        }
    }
}
