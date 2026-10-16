use super::Workbench;
use crate::app::{command::Command, view_state::ExplorerSelection};
use eframe::egui;
use std::path::{Path, PathBuf};

impl Workbench<'_> {
    pub(crate) fn explorer_row_actions(
        &mut self,
        response: &egui::Response,
        path: &Path,
        directory: bool,
    ) {
        if response.clicked() || response.secondary_clicked() {
            self.view.explorer_focused = true;
            response.request_focus();
            self.view.explorer_selection = Some(if directory {
                ExplorerSelection::Directory(path.into())
            } else {
                ExplorerSelection::File(path.into())
            });
        }
        let root = self.editor.workspace.as_deref() == Some(path);
        response.context_menu(|ui| {
            if !directory && ui.button("Open").clicked() {
                self.command(Command::OpenPath(path.into()));
                ui.close();
            }
            if ui.button("New File…").clicked() {
                self.command(Command::NewFile);
                ui.close();
            }
            if ui.button("New Folder…").clicked() {
                self.command(Command::NewFolder);
                ui.close();
            }
            if !directory && ui.button("Add to Agent Chat").clicked() {
                self.command(Command::AddFileToAgent(path.into()));
                ui.close();
            }
            ui.separator();
            if ui.add_enabled(!root, egui::Button::new("Cut")).clicked() {
                self.command(Command::ExplorerCut(path.into()));
                ui.close();
            }
            if ui.add_enabled(!root, egui::Button::new("Copy")).clicked() {
                self.command(Command::ExplorerCopy(path.into()));
                ui.close();
            }
            if ui
                .add_enabled(
                    self.view.explorer_clipboard.is_some(),
                    egui::Button::new("Paste"),
                )
                .clicked()
            {
                self.command(Command::ExplorerPaste);
                ui.close();
            }
            ui.separator();
            if ui.button("Copy Path").clicked() {
                self.command(Command::CopyPath(path.into(), false));
                ui.close();
            }
            if ui.button("Copy Relative Path").clicked() {
                self.command(Command::CopyPath(path.into(), true));
                ui.close();
            }
            if ui
                .button(if cfg!(target_os = "macos") {
                    "Reveal in Finder"
                } else if cfg!(target_os = "windows") {
                    "Reveal in File Explorer"
                } else {
                    "Reveal in File Manager"
                })
                .clicked()
            {
                self.command(Command::RevealPath(path.into()));
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled(!root, egui::Button::new("Rename…"))
                .clicked()
            {
                self.command(Command::RenamePath(path.into()));
                ui.close();
            }
            if ui.add_enabled(!root, egui::Button::new("Delete")).clicked() {
                self.command(Command::DeletePath(path.into()));
                ui.close();
            }
        });
    }

    pub(crate) fn explorer_keyboard(&mut self, ui: &mut egui::Ui, root: &Path) {
        if !self.view.explorer_focused
            || self.view.explorer_edit.is_some()
            || self.view.delete_prompt.is_some()
            || self.view.command_palette
            || self.view.navigation.mode.is_some()
            || ui.ctx().wants_keyboard_input()
        {
            return;
        }
        let Some(selection) = &self.view.explorer_selection else {
            return;
        };
        let path = selection.path().to_path_buf();
        let directory = self.file_system.is_directory(&path);
        let pressed = |key| ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key));
        if pressed(egui::Key::F2) || (cfg!(target_os = "macos") && pressed(egui::Key::Enter)) {
            self.command(Command::RenamePath(path.clone()));
        } else if pressed(egui::Key::Delete)
            || (cfg!(target_os = "macos")
                && ui.input_mut(|input| {
                    input.consume_key(egui::Modifiers::COMMAND, egui::Key::Backspace)
                }))
        {
            self.command(Command::DeletePath(path.clone()));
        } else if (!cfg!(target_os = "macos") && pressed(egui::Key::Enter))
            || ui.input_mut(|input| {
                input.consume_key(egui::Modifiers::COMMAND, egui::Key::ArrowDown)
            })
        {
            if directory {
                self.view.expanded_dirs.insert(path.clone());
            } else {
                self.command(Command::OpenPath(path.clone()));
            }
        } else if pressed(egui::Key::ArrowRight) && directory {
            self.view.expanded_dirs.insert(path.clone());
        } else if pressed(egui::Key::ArrowLeft) {
            if !self.view.expanded_dirs.remove(&path)
                && let Some(parent) = path.parent()
                && parent.starts_with(root)
            {
                self.view.explorer_selection = Some(ExplorerSelection::Directory(parent.into()));
            }
        } else {
            let step = if pressed(egui::Key::ArrowDown) {
                1
            } else if pressed(egui::Key::ArrowUp) {
                -1
            } else {
                0
            };
            if step != 0 {
                let mut entries = Vec::new();
                self.visible_explorer_entries(root, &mut entries);
                if let Some(index) = entries.iter().position(|entry| entry == &path) {
                    let next = (index as isize + step)
                        .clamp(0, entries.len().saturating_sub(1) as isize)
                        as usize;
                    let next = &entries[next];
                    self.view.explorer_selection = Some(if self.file_system.is_directory(next) {
                        ExplorerSelection::Directory(next.clone())
                    } else {
                        ExplorerSelection::File(next.clone())
                    });
                }
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        for (key, command) in [
            (egui::Key::C, Command::ExplorerCopy(path.clone())),
            (egui::Key::X, Command::ExplorerCut(path.clone())),
            (egui::Key::V, Command::ExplorerPaste),
        ] {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, key)) {
                self.command(command);
            }
        }
    }

    fn visible_explorer_entries(&self, path: &Path, entries: &mut Vec<PathBuf>) {
        entries.push(path.into());
        if self.file_system.is_directory(path) && self.view.expanded_dirs.contains(path) {
            for child in self.file_system.children(path) {
                if !self.explorer_entry_hidden(&child) {
                    self.visible_explorer_entries(&child, entries);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::LumenApp;
    use lumen_settings_core::Value;

    #[test]
    fn keyboard_traversal_skips_hidden_entries_unless_they_are_shown() {
        let root = std::env::temp_dir().join(format!("lumen-hidden-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".secret")).unwrap();
        std::fs::write(root.join(".env"), "K=1").unwrap();
        std::fs::write(root.join("a.txt"), "a").unwrap();
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.clone());
        app.view.expanded_dirs.insert(root.clone());
        app.view.expanded_dirs.insert(root.join(".secret"));
        let names = |app: &mut LumenApp| {
            let mut entries = Vec::new();
            app.render_component(&eframe::egui::Context::default(), |workbench| {
                workbench.visible_explorer_entries(&root, &mut entries)
            });
            entries
                .iter()
                .map(|path| {
                    path.strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect::<Vec<_>>()
        };
        let shown = names(&mut app);
        assert!(shown.contains(&".env".to_string()) && shown.contains(&".secret".to_string()));
        app.view
            .settings
            .user
            .persisted
            .insert("explorer.show_hidden".into(), Value::Bool(false));
        assert_eq!(names(&mut app), ["", "a.txt"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
