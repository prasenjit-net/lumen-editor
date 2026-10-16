//! Feature-gated workbench adapter. The document core owns every mutation/history.
use super::{
    Workbench,
    theme::{ACCENT, BG},
};
use crate::app::editor_action::{blocked, prepare_action, text_widget_id};
use crate::icon;
use eframe::egui::{self, Color32, Frame, Stroke, StrokeKind};

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
        let fold_state = self.view.fold_tabs.entry(document).or_default();
        fold_state.refresh(
            &self.editor.files[index].text,
            self.editor.files[index].language,
            self.editor.files[index].revision,
        );
        surface.set_folding(
            &self.editor.files[index].text,
            &fold_state.ranges,
            &fold_state.collapsed(),
            icon::CARET_RIGHT,
            icon::CARET_DOWN,
        );
        let path = self.editor.files[index].path.as_deref();
        surface.set_indentation(lumen_editor_core::Indentation {
            width: self.view.settings.number("editor.indent_size", path) as usize,
            insert_spaces: self
                .view
                .settings
                .document_boolean("editor.insert_spaces", path),
            detect: self
                .view
                .settings
                .document_boolean("editor.detect_indentation", path),
            auto_indent: self
                .view
                .settings
                .document_boolean("editor.auto_indent", path),
        });
        surface.set_pairing(lumen_editor_surface::PairingOptions {
            auto_closing_brackets: self
                .view
                .settings
                .document_boolean("editor.auto_closing_brackets", path),
            auto_closing_quotes: self
                .view
                .settings
                .document_boolean("editor.auto_closing_quotes", path),
            bracket_matching: self
                .view
                .settings
                .document_boolean("editor.bracket_matching", path),
        });
        surface.set_typography(lumen_editor_surface::Typography {
            font_size: self.view.settings.number("editor.font_size", path),
            line_height: self.view.settings.number("editor.line_height", path),
            bundled_font: self.view.settings.font_family(path) == "Hack",
        });
        surface.set_language(self.editor.files[index].language);
        let diff_lines = self.view.source_control.diff.as_ref().and_then(|diff| {
            (diff.active
                && diff.document == Some(document)
                && diff.opened_revision == Some(self.editor.files[index].revision))
            .then_some(&diff.comparison)
            .and_then(|result| result.as_ref().ok())
            .map(|comparison| &comparison.new_lines)
        });
        surface.set_diff_lines(diff_lines, super::theme::DIFF_ADDED_BG);
        surface.set_syntax(
            self.view
                .settings
                .document_boolean("editor.syntax_highlighting", path)
                .then(|| {
                    (
                        self.editor.files[index].language,
                        super::theme::syntax_palette(
                            &self
                                .view
                                .settings
                                .document_choice("editor.color_scheme", path),
                        ),
                    )
                }),
        );
        if let Some(find) = self.view.find.get_mut(&document)
            && find.reveal
            && let Some(range) = find.matches.get(find.current)
        {
            surface.reveal(range.start);
            find.reveal = false;
        }
        if let Some((target, byte)) = self.view.pending_goto.take()
            && target == document
        {
            if self.editor.files[index]
                .editing
                .as_mut()
                .unwrap()
                .set_selections(vec![lumen_editor_core::Selection::caret(byte)], 0)
                .is_ok()
            {
                surface.reveal(byte);
            } else {
                self.view.status = "Could not move to that text position".into();
            }
        }
        if let Some((target, range)) = self.view.pending_problem_selection.take()
            && target == document
        {
            let selection = lumen_editor_core::Selection {
                anchor: range.start,
                head: range.end,
            };
            if self.editor.files[index]
                .editing
                .as_mut()
                .unwrap()
                .set_selections(vec![selection], 0)
                .is_ok()
            {
                surface.reveal(range.start);
            }
        }
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
        if let Some(headers) = &output.folded_headers {
            self.view
                .fold_tabs
                .entry(document)
                .or_default()
                .set_collapsed(headers, &self.editor.files[index].text);
        }
        if let Some(notice) = &surface.notice {
            self.view.status = notice.clone();
        }
        if let Some(diagnostics) = self.view.language_server_diagnostics.get(&document) {
            let painter = ui.painter_at(output.text_clip);
            for diagnostic in diagnostics {
                let color = match diagnostic.severity {
                    lumen_language_services::diagnostics::DiagnosticSeverity::Error => {
                        Color32::from_rgb(241, 76, 76)
                    }
                    lumen_language_services::diagnostics::DiagnosticSeverity::Warning => {
                        Color32::from_rgb(204, 167, 0)
                    }
                    _ => Color32::from_rgb(117, 190, 255),
                };
                for rect in output.geometry.selection_rects(diagnostic.range.clone()) {
                    painter.line_segment(
                        [
                            egui::pos2(rect.left(), rect.bottom() - 1.0),
                            egui::pos2(rect.right(), rect.bottom() - 1.0),
                        ],
                        Stroke::new(1.0_f32, color),
                    );
                }
            }
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
