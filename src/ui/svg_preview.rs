//! SVG preview tab: the source rasterized at the current zoom and shown in the
//! shared image viewer, or an inline error while the markup is invalid.
use super::{
    Workbench,
    image_viewer::{TOOLBAR_HEIGHT, show_image},
    theme,
};
use crate::{
    app::preview::{SvgPreview, raster_scale},
    model::DocumentId,
    ui::theme::META_SIZE,
};
use eframe::egui::{self, RichText};

impl Workbench<'_> {
    pub(super) fn svg_preview_page(
        &mut self,
        ui: &mut egui::Ui,
        preview: DocumentId,
        source: usize,
    ) {
        let colors = theme::palette(ui.visuals());
        let canvas = egui::vec2(
            ui.available_width(),
            (ui.available_height() - TOOLBAR_HEIGHT).max(0.0),
        );
        let pixels_per_point = ui.ctx().pixels_per_point();
        let state = self.view.preview.svg(preview, &self.editor.files[source]);
        let (width, height) = match state.size() {
            Ok(size) => size,
            Err(error) => {
                let error = error.to_owned();
                ui.vertical_centered(|ui| {
                    ui.add_space(super::widgets::empty_state_top(ui));
                    ui.label(RichText::new("This SVG could not be displayed").color(colors.text));
                    ui.label(RichText::new(error).size(META_SIZE).color(colors.muted));
                });
                return;
            }
        };
        let size = egui::vec2(width as f32, height as f32);
        let wanted = |state: &SvgPreview| state.view.scale(size, canvas) * pixels_per_point;
        let before = raster_scale(wanted(state));
        let image = match state.raster(wanted(state)) {
            Ok(image) => image,
            Err(error) => {
                ui.vertical_centered(|ui| {
                    ui.add_space(super::widgets::empty_state_top(ui));
                    ui.label(RichText::new("This SVG could not be displayed").color(colors.text));
                    ui.label(RichText::new(error).size(META_SIZE).color(colors.muted));
                });
                return;
            }
        };
        show_image(
            ui,
            &mut state.view,
            &image,
            &colors,
            &format!("svg-{preview:?}"),
        );
        // Zoom input this frame may need a sharper bitmap: render it next frame.
        if raster_scale(wanted(state)) != before {
            ui.ctx().request_repaint();
        }
    }
}
