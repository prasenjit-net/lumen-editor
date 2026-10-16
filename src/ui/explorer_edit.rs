use super::Workbench;
use crate::{
    app::{command::Command, view_state::ExplorerEditKind},
    icon,
    model::display_name,
};
use eframe::egui::{self, RichText, TextEdit, Vec2};

impl Workbench<'_> {
    pub(crate) fn explorer_edit_row(&mut self, ui: &mut egui::Ui, depth: usize) {
        let Some(edit) = &mut self.view.explorer_edit else {
            return;
        };
        let mut commit = false;
        let mut cancel = false;
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space((depth * 14 + 20) as f32);
            let folder = matches!(edit.kind, ExplorerEditKind::NewFolder)
                || matches!(&edit.kind, ExplorerEditKind::Rename(path) if self.file_system.is_directory(path));
            ui.label(if folder { icon::FOLDER } else { icon::FILE });
            let mut output = TextEdit::singleline(&mut edit.name)
                .id(egui::Id::new("explorer_inline_name"))
                .desired_width(ui.available_width())
                .hint_text(if folder { "Folder name" } else { "File name" })
                .show(ui);
            if edit.focus_name {
                output.response.request_focus();
                if matches!(edit.kind, ExplorerEditKind::Rename(_)) {
                    let end = if folder { edit.name.chars().count() } else {
                        std::path::Path::new(&edit.name).file_stem().and_then(|stem| stem.to_str()).unwrap_or(&edit.name).chars().count()
                    };
                    output.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(end))));
                    output.state.store(ui.ctx(), output.response.id);
                }
                edit.focus_name = false;
            }
            if output.response.changed() { edit.error = None; }
            if output.response.has_focus() || output.response.lost_focus() {
                commit = ui.input(|input| input.key_pressed(egui::Key::Enter));
            }
            cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
        });
        if let Some(error) = &edit.error {
            ui.label(
                RichText::new(error)
                    .small()
                    .color(ui.visuals().error_fg_color),
            );
        }
        if cancel {
            self.view.explorer_edit = None;
        } else if commit {
            self.command(Command::CommitExplorerEdit);
        }
    }

    pub(crate) fn delete_dialog(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(prompt) = &self.view.delete_prompt else {
            return;
        };
        let path = prompt.path.clone();
        let dirty = self
            .editor
            .files
            .iter()
            .filter(|file| {
                file.dirty
                    && file
                        .path
                        .as_ref()
                        .is_some_and(|file| file.starts_with(&path))
            })
            .count();
        let mut confirm = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("explorer_delete")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading(format!("Delete {}?", display_name(&path)));
            ui.label(path.display().to_string());
            ui.label(RichText::new("This item and its contents will move to Trash/Recycle Bin. Restore them there if needed.").color(colors.muted));
            if dirty > 0 { ui.label(format!("{dirty} unsaved editor buffer(s) will remain open as untitled files so your edits are not lost.")); }
            if let Some(error) = &prompt.error { ui.label(RichText::new(error).color(ui.visuals().error_fg_color)); }
            ui.add_space(8.0);
            ui.horizontal(|ui| { confirm = ui.button(if cfg!(target_os = "windows") { "Move to Recycle Bin" } else { "Move to Trash" }).clicked(); cancel = ui.button("Cancel").clicked(); });
        });
        if cancel || modal.should_close() {
            self.view.delete_prompt = None;
        } else if confirm {
            self.command(Command::ConfirmDelete);
        }
    }
}
