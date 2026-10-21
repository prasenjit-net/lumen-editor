//! Applies the document's editor color scheme around the shared editor surface.
use super::Workbench;
use eframe::egui;

impl Workbench<'_> {
    pub(super) fn editor_viewport(&mut self, ui: &mut egui::Ui, index: usize) {
        let path = self.editor.files[index].path.as_deref();
        let scheme = self
            .view
            .settings
            .document_choice("editor.color_scheme", path);
        let theme = crate::ui::theme::editor_theme(&scheme, ui.visuals());
        let original = ui.style().clone();
        *ui.visuals_mut() = crate::ui::theme::visuals(theme);
        self.editor_shared(ui, index);
        ui.set_style(original);
    }
}
