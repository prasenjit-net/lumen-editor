//! Markdown preview tab: renders the parsed block tree of the source document.
//! Pure drawing; links become typed commands and images are requested from the
//! app, which loads them off the UI thread.
use super::{
    Workbench,
    markdown_render::{Host, Metrics, Renderer},
    theme,
};
use crate::model::previewable::PreviewKind;
use eframe::egui::{self, RichText, ScrollArea};

/// Space around the document, as in VS Code's preview: it uses the whole
/// editor, with this much room at each side.
const SIDE_PADDING: i8 = 26;

impl Workbench<'_> {
    pub(super) fn preview_page(&mut self, ui: &mut egui::Ui, index: usize) {
        let colors = theme::palette(ui.visuals());
        let preview = self.editor.files[index].id;
        let Some(source) = self.editor.files[index]
            .view_of
            .and_then(|id| self.editor.index_of(id))
        else {
            ui.label(RichText::new("The previewed file was closed").color(colors.muted));
            return;
        };
        // The source may not have rendered this frame; bring its text up to date.
        self.editor.files[source].sync_shared_editing();
        if self.editor.files[source].preview_kind() == Some(PreviewKind::Svg) {
            self.svg_preview_page(ui, preview, source);
            return;
        }
        let blocks = self.view.preview.blocks(&self.editor.files[source]);
        let source_path = self.editor.files[source].path.clone();
        let mut anchor = match self.view.preview.anchor_request.take() {
            Some((target, anchor)) if target == preview => Some(anchor),
            other => {
                self.view.preview.anchor_request = other;
                None
            }
        };
        let mut commands = Vec::new();
        ScrollArea::vertical()
            .id_salt(("markdown-preview", preview))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin {
                        left: SIDE_PADDING,
                        right: SIDE_PADDING,
                        top: 16,
                        bottom: 32,
                    })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let mut renderer = Renderer::new(
                            colors,
                            Metrics::PREVIEW,
                            Host::Preview {
                                document: preview,
                                source_path: source_path.as_deref(),
                                state: &mut self.view.preview,
                                anchor: &mut anchor,
                            },
                            &mut commands,
                        );
                        renderer.blocks(ui, &blocks, Metrics::PREVIEW.body);
                    });
            });
        if anchor.is_some() {
            self.view.status = "No heading matches that link".into();
        }
        for command in commands {
            self.command(command);
        }
    }
}
