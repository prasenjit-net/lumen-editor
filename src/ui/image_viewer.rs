//! Image tabs: zoom toolbar, checkerboard-backed canvas, pan and zoom input.
use super::Workbench;
use crate::{
    app::{
        command::Command,
        images::{ImageViewState, ImageZoom},
    },
    icon,
    model::image_asset::{DecodedImage, ImageLoad},
    ui::theme::{META_SIZE, Palette},
};
use eframe::egui::{self, Color32, RichText};
use std::sync::Arc;

const TOOLBAR_HEIGHT: f32 = 26.0;
const BUTTON_SIZE: f32 = 22.0;
const CHECKER_TILE: f32 = 8.0;
/// Part of the image that must stay inside the viewport while panning.
const PAN_MARGIN: f32 = 40.0;

impl Workbench<'_> {
    pub(super) fn image_page(&mut self, ui: &mut egui::Ui, index: usize) {
        let colors = crate::ui::theme::palette(ui.visuals());
        let file = &self.editor.files[index];
        let (id, disk_changed) = (file.id, file.disk_changed);
        let (decoded, failure) = match &file.image {
            Some(ImageLoad::Ready(image)) => (Some(Arc::clone(image)), None),
            Some(ImageLoad::Failed(error)) => (None, Some(error.clone())),
            _ => (None, None),
        };
        if disk_changed {
            ui.label(
                RichText::new("File no longer exists on disk")
                    .size(META_SIZE)
                    .color(colors.muted),
            );
        }
        if let Some(error) = failure {
            ui.vertical_centered(|ui| {
                ui.add_space(super::widgets::empty_state_top(ui));
                ui.label(RichText::new("This image could not be displayed").color(colors.text));
                ui.label(RichText::new(error).size(META_SIZE).color(colors.muted));
                ui.add_space(8.0);
                if ui.button("Open as Text").clicked() {
                    self.command(Command::OpenImageAsText(id));
                }
            });
            return;
        }
        let Some(decoded) = decoded else {
            ui.vertical_centered(|ui| {
                ui.add_space(super::widgets::empty_state_top(ui));
                ui.spinner();
                ui.label(RichText::new("Loading image…").color(colors.muted));
            });
            return;
        };
        let state = self.view.image_views.entry(id).or_default();
        show_image(ui, state, &decoded, &colors, &format!("{id:?}"));
    }
}

/// Draw the toolbar and canvas for one decoded image.
pub(crate) fn show_image(
    ui: &mut egui::Ui,
    state: &mut ImageViewState,
    image: &Arc<DecodedImage>,
    colors: &Palette,
    name: &str,
) {
    let size = egui::vec2(image.width as f32, image.height as f32);
    let canvas_height = (ui.available_height() - TOOLBAR_HEIGHT).max(0.0);
    let canvas_size = egui::vec2(ui.available_width(), canvas_height);
    let current = state.scale(size, canvas_size);

    toolbar(ui, state, current, colors);

    let (rect, response) = ui.allocate_exact_size(canvas_size, egui::Sense::click_and_drag());
    ui.painter().rect_filled(rect, 0.0, colors.canvas);
    let hovered = response.hovered();
    let anchor = ui
        .input(|input| input.pointer.hover_pos())
        .map_or(egui::Vec2::ZERO, |pos| pos - rect.center());

    // Ctrl/Cmd+wheel and trackpad pinch zoom around the pointer; plain wheel pans.
    let (zoom_factor, scroll) = ui.input(|input| (input.zoom_delta(), input.smooth_scroll_delta));
    if hovered {
        if (zoom_factor - 1.0).abs() > f32::EPSILON {
            state.zoom_at(current, current * zoom_factor, anchor);
        } else if scroll != egui::Vec2::ZERO {
            if matches!(state.zoom, ImageZoom::Fit) {
                state.zoom = ImageZoom::Scale(current);
            }
            state.offset += scroll;
        }
        if response.double_clicked() {
            if matches!(state.zoom, ImageZoom::Fit) {
                state.actual_size();
            } else {
                state.fit();
            }
        }
        if !ui.ctx().wants_keyboard_input() {
            keyboard_zoom(ui, state, size, canvas_size);
        }
    }
    if response.dragged() {
        state.offset += response.drag_delta();
    }
    if hovered || response.dragged() {
        ui.ctx().set_cursor_icon(if response.dragged() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        });
    }

    let scale = state.scale(size, canvas_size);
    state.offset = clamp_offset(state.offset, size * scale, canvas_size);
    let image_rect = egui::Rect::from_center_size(rect.center() + state.offset, size * scale);
    let painter = ui.painter().with_clip_rect(rect);
    checkerboard(&painter, image_rect.intersect(rect), image_rect.min, colors);
    let texture = texture_for(ui.ctx(), state, image, name);
    painter.image(
        texture,
        image_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn toolbar(ui: &mut egui::Ui, state: &mut ImageViewState, current: f32, colors: &Palette) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TOOLBAR_HEIGHT),
        egui::Sense::hover(),
    );
    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(6.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let button = |ui: &mut egui::Ui, glyph: &str, tooltip: &str| {
        let response = ui.add_sized(
            [BUTTON_SIZE, BUTTON_SIZE],
            egui::Button::new(RichText::new(glyph).size(13.0).color(colors.text)).frame(false),
        );
        super::widgets::named(response.on_hover_text(tooltip), true, tooltip)
    };
    if button(&mut bar, icon::FIT_SCREEN, "Fit to Editor").clicked() {
        state.fit();
    }
    if button(&mut bar, "1:1", "Actual Size (100%)").clicked() {
        state.actual_size();
    }
    if button(&mut bar, icon::PLUS, "Zoom In").clicked() {
        state.zoom_in(current);
    }
    bar.add_sized(
        [44.0, BUTTON_SIZE],
        egui::Label::new(
            RichText::new(format!("{:.0}%", current * 100.0))
                .size(META_SIZE)
                .color(colors.muted),
        ),
    );
    if button(&mut bar, icon::MINUS, "Zoom Out").clicked() {
        state.zoom_out(current);
    }
}

fn keyboard_zoom(ui: &egui::Ui, state: &mut ImageViewState, size: egui::Vec2, canvas: egui::Vec2) {
    let current = state.scale(size, canvas);
    let (zoom_in, zoom_out, fit, actual) = ui.input(|input| {
        (
            input.key_pressed(egui::Key::Plus) || input.key_pressed(egui::Key::Equals),
            input.key_pressed(egui::Key::Minus),
            input.key_pressed(egui::Key::Num0),
            input.key_pressed(egui::Key::Num1),
        )
    });
    if zoom_in {
        state.zoom_in(current);
    } else if zoom_out {
        state.zoom_out(current);
    } else if fit {
        state.fit();
    } else if actual {
        state.actual_size();
    }
}

/// Keep part of the image in view; an axis that fits is always centred.
fn clamp_offset(offset: egui::Vec2, image: egui::Vec2, canvas: egui::Vec2) -> egui::Vec2 {
    let axis = |offset: f32, image: f32, canvas: f32| {
        if image <= canvas {
            0.0
        } else {
            let limit = (image - canvas) / 2.0 + PAN_MARGIN.min(canvas / 2.0);
            offset.clamp(-limit, limit)
        }
    };
    egui::vec2(
        axis(offset.x, image.x, canvas.x),
        axis(offset.y, image.y, canvas.y),
    )
}

fn checkerboard(
    painter: &egui::Painter,
    visible: egui::Rect,
    origin: egui::Pos2,
    colors: &Palette,
) {
    if visible.width() <= 0.0 || visible.height() <= 0.0 {
        return;
    }
    let light = colors.canvas.lerp_to_gamma(colors.text, 0.16);
    let dark = colors.canvas.lerp_to_gamma(colors.text, 0.08);
    painter.rect_filled(visible, 0.0, dark);
    let first_column = ((visible.left() - origin.x) / CHECKER_TILE).floor() as i32;
    let first_row = ((visible.top() - origin.y) / CHECKER_TILE).floor() as i32;
    let mut row = first_row;
    while origin.y + row as f32 * CHECKER_TILE < visible.bottom() {
        let mut column = first_column;
        while origin.x + column as f32 * CHECKER_TILE < visible.right() {
            if (row + column).rem_euclid(2) == 0 {
                let tile = egui::Rect::from_min_size(
                    origin + egui::vec2(column as f32, row as f32) * CHECKER_TILE,
                    egui::Vec2::splat(CHECKER_TILE),
                )
                .intersect(visible);
                painter.rect_filled(tile, 0.0, light);
            }
            column += 1;
        }
        row += 1;
    }
}

/// The texture for `image`, uploaded once per decoded allocation.
fn texture_for(
    ctx: &egui::Context,
    state: &mut ImageViewState,
    image: &Arc<DecodedImage>,
    name: &str,
) -> egui::TextureId {
    let key = Arc::as_ptr(image) as usize;
    if state.texture.as_ref().is_none_or(|(held, _)| *held != key) {
        let pixels = egui::ColorImage::from_rgba_unmultiplied(
            [image.texture_width as usize, image.texture_height as usize],
            &image.rgba,
        );
        let options = egui::TextureOptions {
            magnification: egui::TextureFilter::Nearest,
            minification: egui::TextureFilter::Linear,
            ..Default::default()
        };
        state.texture = Some((
            key,
            ctx.load_texture(format!("image-{name}"), pixels, options),
        ));
    }
    state
        .texture
        .as_ref()
        .expect("texture was just loaded")
        .1
        .id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::images::{MAX_SCALE, MIN_SCALE};

    #[test]
    fn panning_keeps_the_image_reachable_and_centres_axes_that_fit() {
        let canvas = egui::vec2(400.0, 300.0);
        let wide = egui::vec2(1000.0, 100.0);
        let clamped = clamp_offset(egui::vec2(9999.0, 9999.0), wide, canvas);
        assert_eq!(clamped.x, (1000.0 - 400.0) / 2.0 + PAN_MARGIN);
        assert_eq!(clamped.y, 0.0);
        let small = clamp_offset(egui::vec2(50.0, -50.0), egui::vec2(100.0, 100.0), canvas);
        assert_eq!(small, egui::Vec2::ZERO);
    }

    #[test]
    fn scale_limits_bound_pinch_zoom() {
        let mut state = ImageViewState::default();
        state.zoom_at(1.0, 1e6, egui::Vec2::ZERO);
        assert_eq!(state.zoom, ImageZoom::Scale(MAX_SCALE));
        state.zoom_at(MAX_SCALE, 1e-6, egui::Vec2::ZERO);
        assert_eq!(state.zoom, ImageZoom::Scale(MIN_SCALE));
    }
}
