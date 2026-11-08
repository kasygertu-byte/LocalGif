//! Enlarged view of a GIF (middle-click a GIF, or right-click → View larger),
//! so small text in it is readable.

use super::{GifState, LocalGifApp, draw_badges};
use eframe::egui::{self, Color32, Id, RichText, Sense, vec2};

/// Small GIFs are scaled up at most this much.
const MAX_UPSCALE: f32 = 3.0;

impl LocalGifApp {
    pub(super) fn preview_modal(&mut self, ctx: &egui::Context) {
        let Some(preview) = &mut self.preview else {
            return;
        };
        let Some(entry) = self.library.get(preview.id) else {
            self.preview = None;
            return;
        };
        let name = entry.display_name().to_string();
        let tags = entry.tags.clone();
        let id = entry.id;
        let alt = ctx.input(|i| i.modifiers.alt);
        let nsfw_censored = entry.nsfw && !self.library.settings.uncensor_nsfw;
        let censored = (nsfw_censored || entry.warning) && !preview.reveal && !alt;
        let (nsfw, warning) = (entry.nsfw, entry.warning);

        // Leave room around the image for the backdrop, the name and the buttons.
        let room = ctx.content_rect().size() - vec2(64.0, 150.0);
        let now = ctx.input(|i| i.time);
        let mut copy = false;
        let mut close = false;

        let modal = egui::Modal::new(Id::unique("preview")).show(ctx, |ui| {
            match &preview.state {
                GifState::Ready(gif) => {
                    let (tex, until_next) = if censored {
                        (&gif.censored, None)
                    } else {
                        gif.frame_at(now)
                    };
                    // Size from the full-resolution frame (the censored one is tiny).
                    let full = gif.textures[0].size_vec2();
                    let scale = (room.x / full.x).min(room.y / full.y).min(MAX_UPSCALE);
                    let size = full * scale.max(0.1);
                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                    egui::Image::from_texture(tex)
                        .corner_radius(6)
                        .paint_at(ui, rect);
                    if censored {
                        let painter = ui.painter_at(rect);
                        painter.rect_filled(rect, 6, Color32::from_black_alpha(110));
                        draw_badges(&painter, rect, nsfw, warning);
                    }
                    if let Some(dt) = until_next {
                        ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                            dt.max(0.005),
                        ));
                    }
                }
                GifState::Loading => {
                    ui.allocate_ui(vec2(320.0, 240.0), |ui| {
                        ui.centered_and_justified(|ui| ui.spinner());
                    });
                }
                GifState::Failed(err) => {
                    ui.colored_label(Color32::LIGHT_RED, format!("Can't load GIF: {err}"));
                }
            }

            ui.add_space(6.0);
            ui.label(RichText::new(&name).strong().size(16.0));
            if !tags.is_empty() {
                let tags: Vec<String> = tags.iter().map(|t| format!("#{t}")).collect();
                ui.label(RichText::new(tags.join(" ")).color(Color32::GRAY));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Copy").clicked() {
                    copy = true;
                }
                if censored
                    && ui
                        .button("Show")
                        .on_hover_text("Reveal this GIF (or hold Alt)")
                        .clicked()
                {
                    preview.reveal = true;
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });

        // Esc, clicking outside, or another scroll-wheel click closes it.
        let middle = !std::mem::take(&mut preview.just_opened)
            && ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Middle));
        if copy {
            self.preview = None;
            self.copy_gif(ctx, id);
        } else if close || modal.should_close() || middle {
            self.preview = None;
        }
    }
}
