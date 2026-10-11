use crate::{EditorSurface, TextGeometry};
use egui::{Rect, Stroke, Ui};

impl EditorSurface {
    pub(crate) fn paint_preedit(
        &self,
        ui: &Ui,
        geometry: &TextGeometry,
        clip: Rect,
    ) -> Option<Rect> {
        let (start, caret) = geometry.preedit_rects()?;
        let painter = ui.painter().with_clip_rect(clip);
        painter.line_segment(
            [start.left_bottom(), caret.left_bottom()],
            Stroke::new(1.0_f32, ui.visuals().selection.stroke.color),
        );
        painter.line_segment(
            [caret.left_top(), caret.left_bottom()],
            Stroke::new(2.0_f32, ui.visuals().text_color()),
        );
        Some(caret)
    }
}
