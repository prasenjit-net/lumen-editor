use super::Workbench;
use crate::app::{command::Command, view_state::LanguageServerStatus};
use eframe::egui::{self, RichText, Vec2};
/// Escape is the dialog's safe exit. Consume it so the editor beneath, whose
/// own Escape handling closes Find, never also reacts to the same key.
pub(super) fn escape_pressed(ctx: &egui::Context) -> bool {
    ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
}

/// The suggested, non-destructive action: filled with the accent colour.
pub(super) fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let colors = crate::ui::theme::palette(ui.visuals());
    ui.add(egui::Button::new(RichText::new(label).color(egui::Color32::WHITE)).fill(colors.accent))
}

/// Keep keyboard focus inside the dialog, starting on its safe default so a
/// stray Enter cannot discard work.
pub(super) fn keep_focus(
    ctx: &egui::Context,
    buttons: &[&egui::Response],
    default: &egui::Response,
) {
    if !buttons.iter().any(|button| button.has_focus()) {
        ctx.memory_mut(|memory| memory.request_focus(default.id));
    }
}

impl Workbench<'_> {
    pub(crate) fn language_server_details_dialog(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        if !self.view.language_server_details_open {
            return;
        }
        let Some(LanguageServerStatus::Error(message)) = self.view.language_server_status.as_ref()
        else {
            self.view.language_server_details_open = false;
            return;
        };
        let message = message.clone();
        egui::Window::new("Language Server Error")
            .open(&mut self.view.language_server_details_open)
            .collapsible(false)
            .resizable(true)
            .default_width(640.0)
            .default_height(320.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("The language server stopped. The diagnostic below is selectable and can be copied for troubleshooting.")
                        .color(colors.muted),
                );
                ui.add_space(6.0);
                egui::ScrollArea::both()
                    .id_salt("language_server_error_details")
                    .auto_shrink([false, false])
                    .max_height(260.0)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add(
                            egui::Label::new(RichText::new(&message).monospace())
                                .selectable(true),
                        );
                    });
                ui.add_space(6.0);
                if ui.button("Copy Error").clicked() {
                    ui.ctx().copy_text(message.clone());
                    self.view.status = "Language server error copied".into();
                }
            });
    }

    pub(crate) fn safety_details_dialog(&mut self, ctx: &egui::Context) {
        if !self.view.safety_details_open {
            return;
        }
        egui::Window::new("Recovery and save warnings")
            .open(&mut self.view.safety_details_open)
            .resizable(true)
            .show(ctx, |ui| {
                for notice in &self.view.safety_notices {
                    ui.label(notice);
                }
            });
    }
    pub(crate) fn save_conflict_dialog(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(conflict) = &self.view.save_conflict else {
            return;
        };
        let document = conflict.document;
        let show_comparison = conflict.show_comparison;
        let disk_text = if show_comparison {
            conflict.disk_text.as_deref().map(preview_text)
        } else {
            None
        };
        let editor_text = if show_comparison {
            self.editor
                .files
                .iter()
                .find(|file| file.id == document)
                .map(|file| preview_text(&file.text))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let name = conflict
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| conflict.path.display().to_string());
        egui::Window::new("File changed on disk")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(640.0);
                ui.add(egui::Label::new(format!(
                    "{name} changed outside Lumen since it was opened or saved."
                )).wrap());
                ui.label(RichText::new("Your edits have not been overwritten.").color(colors.muted));
                ui.label(RichText::new("Reload discards this tab's unsaved edits; Overwrite replaces the external version.").color(colors.muted));
                if ui.button(if show_comparison { "Hide Comparison" } else { "Compare Versions" }).clicked()
                    && let Some(conflict) = &mut self.view.save_conflict {
                    conflict.show_comparison = !show_comparison;
                }
                if show_comparison {
                    ui.horizontal_top(|ui| {
                        ui.set_min_width(620.0);
                        ui.vertical(|ui| {
                            ui.label("On Disk");
                            egui::ScrollArea::both().id_salt("conflict_disk").max_height(260.0).max_width(300.0).show(ui, |ui| {
                                ui.set_min_width(290.0);
                                ui.label(RichText::new(disk_text.as_deref().unwrap_or("(File missing from disk)")).monospace());
                            });
                        });
                        ui.vertical(|ui| {
                            ui.label("In Editor (unsaved)");
                            egui::ScrollArea::both().id_salt("conflict_editor").max_height(260.0).max_width(300.0).show(ui, |ui| {
                                ui.set_min_width(290.0);
                                ui.label(RichText::new(&editor_text).monospace());
                            });
                        });
                    });
                }
                ui.horizontal(|ui| {
                    let keep = primary_button(ui, "Keep Editing");
                    let overwrite = ui
                        .button("Overwrite Disk File")
                        .on_hover_text(
                            "Replace the external version with your current editor contents",
                        );
                    let reload = ui.button("Reload from Disk").on_hover_text("Discard this tab's unsaved edits and open the external version");
                    let save_as = ui.button("Save As…").on_hover_text("Keep both versions by saving editor contents to another path");
                    keep_focus(ui.ctx(), &[&keep, &overwrite, &reload, &save_as], &keep);
                    if keep.clicked() || escape_pressed(ui.ctx()) {
                        self.command(Command::DismissConflict);
                        self.view.focus_editor = true;
                    } else if overwrite.clicked() {
                        self.command(Command::OverwriteConflict(document));
                    } else if reload.clicked() {
                        self.command(Command::ReloadConflict(document));
                    } else if save_as.clicked() {
                        self.command(Command::SaveConflictAs(document));
                    }
                });
            });
    }
    pub(crate) fn quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.view.pending_quit {
            return;
        }
        egui::Window::new("Save changes before quitting?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label("There are unsaved changes in your open files.");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let save = primary_button(ui, "Save All");
                    let discard = ui.button("Don't Save");
                    let cancel = ui.button("Cancel");
                    keep_focus(ui.ctx(), &[&save, &discard, &cancel], &save);
                    if save.clicked() {
                        self.command(Command::SaveAllAndQuit);
                    } else if discard.clicked() {
                        self.command(Command::DiscardAndQuit);
                    } else if cancel.clicked() || escape_pressed(ui.ctx()) {
                        self.view.pending_quit = false;
                        self.view.focus_editor = true;
                    }
                });
            });
    }

    pub(crate) fn unsaved_changes_dialog(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(id) = self.view.pending_close else {
            return;
        };
        let Some(name) = self
            .editor
            .index_of(id)
            .map(|index| self.editor.files[index].name.clone())
        else {
            // The document went away while the prompt was open.
            self.view.pending_close = None;
            return;
        };
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.add(egui::Label::new(format!("Save changes to {name}?")).wrap());
                ui.label(
                    RichText::new("Your edits will be lost if you don't save.").color(colors.muted),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let save = primary_button(ui, "Save");
                    let discard = ui.button("Don't Save");
                    let cancel = ui.button("Cancel");
                    keep_focus(ui.ctx(), &[&save, &discard, &cancel], &save);
                    if save.clicked() {
                        self.command(Command::SaveAndClose(id));
                    } else if discard.clicked() {
                        self.view.pending_close = None;
                        self.command(Command::CloseFile(id));
                    } else if cancel.clicked() || escape_pressed(ui.ctx()) {
                        self.view.pending_close = None;
                        self.view.focus_editor = true;
                    }
                });
            });
    }
}

fn preview_text(text: &str) -> String {
    const LIMIT: usize = 20_000;
    let mut characters = text.chars();
    let mut preview: String = characters.by_ref().take(LIMIT).collect();
    if characters.next().is_some() {
        preview.push_str("\n… preview truncated; original text is unchanged");
    }
    preview
}

#[cfg(test)]
mod tests {
    use crate::app::LumenApp;
    use crate::services::FileDialogs;
    use eframe::egui;
    use std::{cell::Cell, path::PathBuf, rc::Rc};

    /// Cancels every dialog and counts Save As requests.
    struct CancelledDialogs(Rc<Cell<usize>>);
    impl FileDialogs for CancelledDialogs {
        fn open_file(&self) -> Option<PathBuf> {
            None
        }
        fn open_folder(&self) -> Option<PathBuf> {
            None
        }
        fn save_as(&self, _: &str) -> Option<PathBuf> {
            self.0.set(self.0.get() + 1);
            None
        }
    }

    fn run(app: &mut LumenApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.unsaved_changes_dialog(ctx)),
        );
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn dirty_app() -> (LumenApp, egui::Context) {
        let mut app = LumenApp::default();
        app.editor.new_file();
        app.editor.files[0].text = "unsaved".into();
        app.editor.files[0].dirty = true;
        app.view.pending_close = Some(app.editor.files[0].id);
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        for _ in 0..3 {
            run(&mut app, &ctx, vec![]);
        }
        (app, ctx)
    }

    #[test]
    fn escape_cancels_the_close_prompt_and_returns_focus_to_the_editor() {
        let (mut app, ctx) = dirty_app();
        run(&mut app, &ctx, vec![key(egui::Key::Escape)]);
        assert_eq!(app.view.pending_close, None);
        assert!(app.view.focus_editor);
        assert_eq!(app.editor.files.len(), 1);
        assert!(app.editor.files[0].dirty);
        assert_eq!(app.editor.files[0].text, "unsaved");
    }

    #[test]
    fn enter_starts_on_save_so_it_cannot_discard_the_buffer() {
        let (mut app, ctx) = dirty_app();
        let save_as_requests = Rc::new(Cell::new(0));
        app.dialogs = Box::new(CancelledDialogs(save_as_requests.clone()));
        run(&mut app, &ctx, vec![key(egui::Key::Enter)]);
        // The default is Save (an untitled buffer asks where); the cancelled
        // Save As leaves the dirty buffer untouched rather than discarding it.
        assert_eq!(save_as_requests.get(), 1);
        assert_eq!(app.editor.files.len(), 1);
        assert!(app.editor.files[0].dirty);
        assert_eq!(app.editor.files[0].text, "unsaved");
    }
}
