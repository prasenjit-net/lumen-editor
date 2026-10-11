use crate::{EditorSurface, TextGeometry};
use egui::{FontId, Rect, Stroke, Ui};

impl EditorSurface {
    pub(crate) fn paint_preedit(
        &self,
        ui: &Ui,
        geometry: &TextGeometry,
        clip: Rect,
    ) -> Option<Rect> {
        let composition = self.composition.as_ref()?;
        let start = geometry.caret_rect(composition.insertion_byte()).min;
        let galley = ui.fonts_mut(|fonts| {
            fonts.layout_no_wrap(
                composition.preedit().into(),
                FontId::monospace(14.0),
                ui.visuals().text_color(),
            )
        });
        let painter = ui.painter().with_clip_rect(clip);
        let rect = Rect::from_min_size(start, galley.size());
        painter.rect_filled(rect, 0, ui.visuals().panel_fill);
        painter.galley(start, galley.clone(), ui.visuals().text_color());
        painter.line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0_f32, ui.visuals().selection.stroke.color),
        );
        let caret = galley
            .pos_from_cursor(galley.end())
            .translate(start.to_vec2());
        painter.line_segment(
            [caret.left_top(), caret.left_bottom()],
            Stroke::new(2.0_f32, ui.visuals().text_color()),
        );
        Some(caret)
    }
}
