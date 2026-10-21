use super::{
    Workbench,
    dialog::{Dialog, danger_button, footer},
};
use crate::app::{command::Command, view_state::LanguageServerStatus};
use eframe::egui::{self, RichText};
/// Escape is the dialog's safe exit. Consume it so the editor beneath, whose
/// own Escape handling closes Find, never also reacts to the same key.
pub(super) fn escape_pressed(ctx: &egui::Context) -> bool {
    ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
}

/// The suggested, non-destructive action: filled with the accent colour.
pub(super) fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let colors = crate::ui::theme::palette(ui.visuals());
    ui.add(egui::Button::new(RichText::new(label).color(colors.on_accent)).fill(colors.accent))
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

/// The three-way answer shared by the save-before-closing prompts.
enum Answer {
    Save,
    Discard,
    Cancel,
}

enum ConflictAnswer {
    Keep,
    Overwrite,
    Reload,
    SaveAs,
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
        let dialog = Dialog::new("Language Server Error")
            .width(640.0)
            .non_modal(false)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("The language server stopped. The diagnostic below is selectable and can be copied for troubleshooting.")
                        .color(colors.muted),
                );
                ui.add_space(6.0);
                egui::ScrollArea::both()
                    .id_salt("language_server_error_details")
                    .auto_shrink([false, true])
                    .max_height(260.0)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add(
                            egui::Label::new(RichText::new(&message).monospace())
                                .selectable(true),
                        );
                    });
                footer(ui, |ui| ui.button("Copy Error").clicked())
            });
        if dialog.inner {
            ctx.copy_text(message);
            self.view.status = "Language server error copied".into();
        }
        if dialog.close_clicked {
            self.view.language_server_details_open = false;
        }
    }

    pub(crate) fn safety_details_dialog(&mut self, ctx: &egui::Context) {
        if !self.view.safety_details_open {
            return;
        }
        let dialog = Dialog::new("Recovery and save warnings")
            .width(480.0)
            .non_modal(false)
            .show(ctx, |ui| {
                for notice in &self.view.safety_notices {
                    ui.add(egui::Label::new(notice).wrap());
                }
            });
        if dialog.close_clicked {
            self.view.safety_details_open = false;
        }
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
        let dialog = Dialog::new("File changed on disk").width(640.0).show(ctx, |ui| {
            ui.add(
                egui::Label::new(format!(
                    "{name} changed outside Lumen since it was opened or saved."
                ))
                .wrap(),
            );
            ui.label(RichText::new("Your edits have not been overwritten.").color(colors.muted));
            ui.label(
                RichText::new(
                    "Reload discards this tab's unsaved edits; Overwrite replaces the external version.",
                )
                .color(colors.muted),
            );
            let toggle = ui.button(if show_comparison {
                "Hide Comparison"
            } else {
                "Compare Versions"
            });
            if toggle.clicked()
                && let Some(conflict) = &mut self.view.save_conflict
            {
                conflict.show_comparison = !show_comparison;
            }
            if show_comparison {
                ui.columns(2, |columns| {
                    columns[0].label("On Disk");
                    egui::ScrollArea::both()
                        .id_salt("conflict_disk")
                        .max_height(260.0)
                        .show(&mut columns[0], |ui| {
                            ui.label(
                                RichText::new(
                                    disk_text.as_deref().unwrap_or("(File missing from disk)"),
                                )
                                .monospace(),
                            );
                        });
                    columns[1].label("In Editor (unsaved)");
                    egui::ScrollArea::both()
                        .id_salt("conflict_editor")
                        .max_height(260.0)
                        .show(&mut columns[1], |ui| {
                            ui.label(RichText::new(&editor_text).monospace());
                        });
                });
            }
            footer(ui, |ui| {
                let keep = primary_button(ui, "Keep Editing");
                let save_as = ui
                    .button("Save As…")
                    .on_hover_text("Keep both versions by saving editor contents to another path");
                let reload = ui
                    .button("Reload from Disk")
                    .on_hover_text("Discard this tab's unsaved edits and open the external version");
                let overwrite = ui
                    .button("Overwrite Disk File")
                    .on_hover_text("Replace the external version with your current editor contents");
                keep_focus(ui.ctx(), &[&keep, &overwrite, &reload, &save_as], &keep);
                if keep.clicked() || escape_pressed(ui.ctx()) {
                    Some(ConflictAnswer::Keep)
                } else if overwrite.clicked() {
                    Some(ConflictAnswer::Overwrite)
                } else if reload.clicked() {
                    Some(ConflictAnswer::Reload)
                } else if save_as.clicked() {
                    Some(ConflictAnswer::SaveAs)
                } else {
                    None
                }
            })
        });
        match dialog
            .inner
            .or(dialog.close_clicked.then_some(ConflictAnswer::Keep))
        {
            Some(ConflictAnswer::Keep) => {
                self.command(Command::DismissConflict);
                self.view.focus_editor = true;
            }
            Some(ConflictAnswer::Overwrite) => self.command(Command::OverwriteConflict(document)),
            Some(ConflictAnswer::Reload) => self.command(Command::ReloadConflict(document)),
            Some(ConflictAnswer::SaveAs) => self.command(Command::SaveConflictAs(document)),
            None => {}
        }
    }
    pub(crate) fn quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.view.pending_quit {
            return;
        }
        let dialog = Dialog::new("Save changes before quitting?").show(ctx, |ui| {
            ui.label("There are unsaved changes in your open files.");
            footer(ui, |ui| {
                let save = primary_button(ui, "Save All");
                let cancel = ui.button("Cancel");
                let discard = ui.button("Don't Save");
                keep_focus(ui.ctx(), &[&save, &discard, &cancel], &save);
                if save.clicked() {
                    Some(Answer::Save)
                } else if discard.clicked() {
                    Some(Answer::Discard)
                } else if cancel.clicked() || escape_pressed(ui.ctx()) {
                    Some(Answer::Cancel)
                } else {
                    None
                }
            })
        });
        match dialog
            .inner
            .or(dialog.close_clicked.then_some(Answer::Cancel))
        {
            Some(Answer::Save) => self.command(Command::SaveAllAndQuit),
            Some(Answer::Discard) => self.command(Command::DiscardAndQuit),
            Some(Answer::Cancel) => {
                self.view.pending_quit = false;
                self.view.focus_editor = true;
            }
            None => {}
        }
    }

    /// Confirmation before a task definition is removed from its tasks file.
    pub(crate) fn task_delete_dialog(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(key) = self.view.task_delete_prompt.clone() else {
            return;
        };
        let Some(label) = self.view.tasks.get(&key).map(|task| task.label.clone()) else {
            self.view.task_delete_prompt = None;
            return;
        };
        let dependents = self.view.tasks.dependents(&key);
        let dialog = Dialog::new("Delete task definition").show(ctx, |ui| {
            ui.add(
                egui::Label::new(format!(
                    "Delete the {} task '{label}' from its tasks file?",
                    key.scope.label().to_lowercase()
                ))
                .wrap(),
            );
            ui.label(RichText::new("Files the task produced are not touched.").color(colors.muted));
            if !dependents.is_empty() {
                ui.add_space(4.0);
                let names: Vec<String> = dependents
                    .iter()
                    .map(|dependent| dependent.id.clone())
                    .collect();
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "These tasks depend on it and will stop validating: {}",
                            names.join(", ")
                        ))
                        .color(colors.error),
                    )
                    .wrap(),
                );
            }
            footer(ui, |ui| {
                let delete = danger_button(ui, "Delete");
                let cancel = ui.button("Cancel");
                keep_focus(ui.ctx(), &[&delete, &cancel], &cancel);
                if delete.clicked() {
                    Some(true)
                } else if cancel.clicked() || escape_pressed(ui.ctx()) {
                    Some(false)
                } else {
                    None
                }
            })
        });
        match dialog.inner.or(dialog.close_clicked.then_some(false)) {
            Some(true) => self.command(Command::ConfirmDeleteTask(key)),
            Some(false) => self.command(Command::CancelDeleteTask),
            None => {}
        }
    }

    /// Save / Don't Save / Cancel for a task editor with unsaved edits.
    pub(crate) fn task_close_dialog(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(id) = self.view.task_close_prompt else {
            return;
        };
        let Some(title) = self.view.task_editors.get(id).map(|tab| tab.title()) else {
            self.view.task_close_prompt = None;
            return;
        };
        let dialog = Dialog::new("Unsaved task changes").show(ctx, |ui| {
            ui.add(egui::Label::new(format!("Save changes to task '{title}'?")).wrap());
            ui.label(
                RichText::new("Your edits will be lost if you don't save.").color(colors.muted),
            );
            footer(ui, |ui| {
                let save = primary_button(ui, "Save");
                let cancel = ui.button("Cancel");
                let discard = ui.button("Don't Save");
                keep_focus(ui.ctx(), &[&save, &discard, &cancel], &save);
                if save.clicked() {
                    Some(Answer::Save)
                } else if discard.clicked() {
                    Some(Answer::Discard)
                } else if cancel.clicked() || escape_pressed(ui.ctx()) {
                    Some(Answer::Cancel)
                } else {
                    None
                }
            })
        });
        match dialog
            .inner
            .or(dialog.close_clicked.then_some(Answer::Cancel))
        {
            Some(Answer::Save) => self.command(Command::SaveAndCloseTaskEditor(id)),
            Some(Answer::Discard) => self.command(Command::DiscardTaskEditor(id)),
            Some(Answer::Cancel) => self.command(Command::CancelTaskEditorClose),
            None => {}
        }
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
        let dialog = Dialog::new("Unsaved changes").show(ctx, |ui| {
            ui.add(egui::Label::new(format!("Save changes to {name}?")).wrap());
            ui.label(
                RichText::new("Your edits will be lost if you don't save.").color(colors.muted),
            );
            footer(ui, |ui| {
                let save = primary_button(ui, "Save");
                let cancel = ui.button("Cancel");
                let discard = ui.button("Don't Save");
                keep_focus(ui.ctx(), &[&save, &discard, &cancel], &save);
                if save.clicked() {
                    Some(Answer::Save)
                } else if discard.clicked() {
                    Some(Answer::Discard)
                } else if cancel.clicked() || escape_pressed(ui.ctx()) {
                    Some(Answer::Cancel)
                } else {
                    None
                }
            })
        });
        let answer = dialog
            .inner
            .or(dialog.close_clicked.then_some(Answer::Cancel));
        match answer {
            Some(Answer::Save) => self.command(Command::SaveAndClose(id)),
            Some(Answer::Discard) => {
                self.view.pending_close = None;
                self.command(Command::CloseFile(id));
            }
            Some(Answer::Cancel) => {
                self.view.pending_close = None;
                self.view.focus_editor = true;
            }
            None => {}
        }
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
