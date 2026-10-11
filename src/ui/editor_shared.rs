//! Feature-gated workbench adapter. The document core owns every mutation/history.
use super::{
    Workbench,
    theme::{ACCENT, BG},
};
use crate::app::editor_action::{blocked, prepare_action, text_widget_id};
use eframe::egui::{self, Frame, Stroke, StrokeKind};

impl Workbench<'_> {
    pub(super) fn editor_shared(&mut self, ui: &mut egui::Ui, index: usize) {
        let ctx = ui.ctx().clone();
        self.editor.files[index].enable_shared_editing();
        let document = self.editor.files[index].id;
        prepare_action(self.view, &self.editor.files[index], &ctx);
        let blocked = blocked(self.view);
        if let Some(find) = self.view.find.get_mut(&document)
            && let Some(status) = find.prepare_shared(&mut self.editor.files[index], &ctx, blocked)
        {
            self.view.status = status;
        }
        if self.view.focus_editor {
            ctx.memory_mut(|memory| memory.request_focus(text_widget_id(document)));
            self.view.focus_editor = false;
        }
        let mut surface = self.view.shared_surface.take().unwrap_or_else(|| {
            let definitions = ui.fonts(|fonts| fonts.definitions().clone());
            lumen_editor_surface::EditorSurface::with_shaper(lumen_editor_surface::Shaper::new(
                &definitions,
            ))
        });
        surface.set_gutter_id(super::editor_gutter::gutter_id(document));
        let output = Frame::new()
            .fill(BG)
            .corner_radius(4)
            .show(ui, |ui| {
                let document_state = &mut self.editor.files[index];
                ui.add_enabled_ui(!blocked, |ui| {
                    surface.show(
                        ui,
                        text_widget_id(document),
                        document_state.editing.as_mut().unwrap(),
                    )
                })
                .inner
            })
            .inner;
        self.editor.files[index].sync_shared_editing();
        if let Some(notice) = &surface.notice {
            self.view.status = notice.clone();
        }
        self.view.shared_surface = Some(surface);
        if let Some(find) = self.view.find.get_mut(&document) {
            find.refresh(&self.editor.files[index]);
            if find.open {
                let painter = ui.painter_at(output.text_clip);
                for (index, range) in find.matches.iter().enumerate() {
                    for rect in output.geometry.selection_rects(range.clone()) {
                        painter.rect_stroke(
                            rect,
                            1,
                            Stroke::new(
                                if index == find.current {
                                    2.0_f32
                                } else {
                                    1.0_f32
                                },
                                ACCENT,
                            ),
                            StrokeKind::Inside,
                        );
                    }
                }
            }
            if find.reveal {
                if let Some(range) = find.matches.get(find.current) {
                    self.view
                        .shared_surface
                        .as_mut()
                        .unwrap()
                        .reveal(range.start);
                }
                find.reveal = false;
                ctx.request_repaint();
            }
        }
        let buffer = self.editor.files[index].editing.as_ref().unwrap();
        let can_undo = buffer.can_undo();
        let can_redo = buffer.can_redo();
        self.editor_popup(&output.response, document, can_undo, can_redo);
        self.view.shared_output = Some(output);
    }
}
