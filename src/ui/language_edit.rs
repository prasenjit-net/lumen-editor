//! Rename prompt and the review dialog for language-server edits.
use super::Workbench;
use crate::app::command::Command;
use eframe::egui::{self, CornerRadius, Frame, RichText, ScrollArea, Stroke, TextEdit, Vec2};

const MAX_PREVIEW_ROWS: usize = 400;

fn window_frame(ctx: &egui::Context) -> Frame {
    let colors = crate::ui::theme::palette(&ctx.style().visuals);
    Frame::window(&ctx.style())
        .fill(colors.panel)
        .corner_radius(CornerRadius::same(6))
        .stroke(Stroke::new(1.0_f32, colors.border))
}

impl Workbench<'_> {
    pub(crate) fn rename_prompt(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(prompt) = self.view.rename_prompt.as_mut() else {
            return;
        };
        let mut command = None;
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            command = Some(Command::CancelRename);
        }
        egui::Window::new("Rename Symbol")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 96.0))
            .frame(window_frame(ctx))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "Rename “{}” everywhere the server finds it",
                        prompt.original
                    ))
                    .color(colors.muted),
                );
                let input = ui.add(
                    TextEdit::singleline(&mut prompt.name)
                        .id(egui::Id::new("language_rename_input"))
                        .desired_width(320.0),
                );
                if prompt.focus {
                    input.request_focus();
                    prompt.focus = false;
                }
                if input.lost_focus() && ui.input(|state| state.key_pressed(egui::Key::Enter)) {
                    command = Some(Command::SubmitRename);
                }
                ui.horizontal(|ui| {
                    if ui.button("Preview Rename").clicked() {
                        command = Some(Command::SubmitRename);
                    }
                    if ui.button("Cancel").clicked() {
                        command = Some(Command::CancelRename);
                    }
                });
            });
        if let Some(command) = command {
            self.command(command);
        }
    }

    pub(crate) fn language_edit_preview(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(plan) = self.view.language_edit_plan.as_ref() else {
            return;
        };
        let rows = plan.preview_lines();
        let (title, files, edits) = (plan.title.clone(), plan.files.len(), plan.edit_count());
        let root = self.editor.workspace.clone();
        let mut command = None;
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            command = Some(Command::CancelLanguageEdit);
        }
        egui::Window::new(format!("Review: {title}"))
            .collapsible(false)
            .resizable(true)
            .default_width(640.0)
            .default_height(380.0)
            .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 72.0))
            .frame(window_frame(ctx))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "{edits} edit(s) in {files} file(s). Open files change in their buffers and stay unsaved; other files are written when you apply."
                    ))
                    .color(colors.muted),
                );
                ui.separator();
                ScrollArea::vertical()
                    .max_height(ui.available_height() - 36.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut current = None;
                        for row in rows.iter().take(MAX_PREVIEW_ROWS) {
                            if current != Some(&row.path) {
                                current = Some(&row.path);
                                let shown = root
                                    .as_ref()
                                    .and_then(|root| row.path.strip_prefix(root).ok())
                                    .unwrap_or(&row.path);
                                ui.add_space(4.0);
                                ui.label(RichText::new(shown.display().to_string()).strong().color(colors.text));
                            }
                            ui.label(
                                RichText::new(format!("{:>5} - {}", row.line, row.before))
                                    .monospace()
                                    .size(11.0)
                                    .color(colors.error),
                            );
                            ui.label(
                                RichText::new(format!("{:>5} + {}", row.line, row.after))
                                    .monospace()
                                    .size(11.0)
                                    .color(colors.diff_added_text),
                            );
                        }
                        if rows.len() > MAX_PREVIEW_ROWS {
                            ui.label(
                                RichText::new(format!(
                                    "… and {} more edits (all are applied)",
                                    rows.len() - MAX_PREVIEW_ROWS
                                ))
                                .color(colors.muted),
                            );
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Apply Changes").clicked() {
                        command = Some(Command::ApplyLanguageEdit);
                    }
                    if ui.button("Cancel").clicked() {
                        command = Some(Command::CancelLanguageEdit);
                    }
                });
            });
        if let Some(command) = command {
            self.command(command);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        LumenApp,
        workspace_edit::{EditPlan, PlannedFile},
    };
    use lumen_language_services::incoming::TextEdit;

    fn frame(app: &mut LumenApp, events: Vec<egui::Event>) {
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 700.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                app.render_component(ctx, |workbench| workbench.rename_prompt(ctx));
                app.render_component(ctx, |workbench| workbench.language_edit_preview(ctx));
            },
        );
    }

    fn escape() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn escape_cancels_the_rename_prompt_and_the_edit_review() {
        let mut app = LumenApp::default();
        app.view.rename_prompt = Some(crate::app::view_state::RenamePrompt {
            document: crate::model::Document::untitled(0).id,
            name: "foo".into(),
            original: "foo".into(),
            focus: true,
        });
        frame(&mut app, vec![escape()]);
        assert!(app.view.rename_prompt.is_none());

        app.view.language_edit_plan = Some(EditPlan {
            title: "Rename Symbol".into(),
            files: vec![PlannedFile {
                path: "a.rs".into(),
                document: None,
                base: "foo".into(),
                edits: vec![TextEdit {
                    range: 0..3,
                    new_text: "bar".into(),
                }],
            }],
        });
        frame(&mut app, vec![]);
        assert!(
            app.view.language_edit_plan.is_some(),
            "stays open until decided"
        );
        assert!(
            crate::app::editor_action::blocked(&app.view),
            "editor input is blocked"
        );
        frame(&mut app, vec![escape()]);
        assert!(app.view.language_edit_plan.is_none());
        assert!(!crate::app::editor_action::blocked(&app.view));
    }
}
