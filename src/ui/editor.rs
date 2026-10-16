use super::Workbench;
use super::widgets::{canvas, surface};
use crate::app::command::Command;
use crate::icon;
use crate::model::display_name;
use eframe::egui::{self, RichText};
impl Workbench<'_> {
    pub(crate) fn editor(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        egui::CentralPanel::default()
            .frame(canvas(ctx))
            .show(ctx, |ui| {
                if ui.rect_contains_pointer(ui.max_rect())
                    && ui.input(|input| input.pointer.any_pressed())
                {
                    self.view.explorer_focused = false;
                }
                if self.editor.group_count() < 2 {
                    surface(colors.bg).show(ui, |ui| {
                        self.editor_group_body(ui, false);
                    });
                } else {
                    self.editor_split(ui);
                }
            });
    }
}

impl Workbench<'_> {
    /// Recently opened folders, then files. Whether an entry still exists is only
    /// known when it is opened; a missing folder is reported and pruned, a missing
    /// file reports a recoverable error, and neither disturbs open buffers.
    fn recent_files_list(&mut self, ui: &mut egui::Ui) {
        const SHOWN: usize = 5;
        let colors = crate::ui::theme::palette(ui.visuals());
        let take = |paths: &[std::path::PathBuf]| -> Vec<std::path::PathBuf> {
            paths.iter().take(SHOWN).cloned().collect()
        };
        let folders = take(&self.view.navigation.recent_folders);
        let files = take(&self.view.navigation.recent);
        for (title, paths, is_folder) in [
            ("Recent folders", folders, true),
            ("Recent files", files, false),
        ] {
            if paths.is_empty() {
                continue;
            }
            ui.add_space(18.0);
            ui.label(RichText::new(title).size(11.0).color(colors.muted));
            for path in paths {
                let name = display_name(&path);
                let parent = path
                    .parent()
                    .map(display_name)
                    .filter(|parent| !parent.is_empty())
                    .unwrap_or_default();
                let label = if parent.is_empty() {
                    name
                } else {
                    format!("{name}  \u{b7}  {parent}")
                };
                if ui
                    .add(egui::Button::new(RichText::new(label).size(12.0)).frame(false))
                    .on_hover_text(path.display().to_string())
                    .clicked()
                {
                    self.command(if is_folder {
                        Command::OpenRecentFolder(path)
                    } else {
                        Command::OpenPath(path)
                    });
                }
            }
        }
    }
}

impl Workbench<'_> {
    /// Everything inside one editor group: tab strip, then its Settings page,
    /// Diff view, welcome screen or the active document.
    pub(super) fn editor_group_body(&mut self, ui: &mut egui::Ui, hide_virtual: bool) {
        let colors = crate::ui::theme::palette(ui.visuals());
        ui.set_min_size(ui.available_size());
        self.view.hide_virtual_tabs = hide_virtual;
        self.editor_tabs(ui);
        self.view.hide_virtual_tabs = false;
        if !hide_virtual && self.view.settings.open {
            self.settings_page(ui);
            return;
        }
        if !hide_virtual
            && self
                .view
                .source_control
                .diff
                .as_ref()
                .is_some_and(|diff| diff.active)
        {
            self.source_diff_editor(ui);
            return;
        }
        let Some(index) = self.editor.active_index() else {
            ui.vertical_centered(|ui| {
                ui.add_space(130.0);
                ui.label(
                    RichText::new("LUMEN")
                        .size(28.0)
                        .strong()
                        .color(colors.muted),
                );
                ui.label(RichText::new("A focused local file editor").color(colors.muted));
                ui.add_space(20.0);
                if ui
                    .button(format!("{}  Open File", icon::FILE_ARROW_UP))
                    .clicked()
                {
                    self.command(Command::OpenFile);
                }
                if ui
                    .button(format!("{}  Open Folder", icon::FOLDER_OPEN))
                    .clicked()
                {
                    self.command(Command::OpenFolder);
                }
                if ui
                    .button(format!("{}  New File", icon::FILE_PLUS))
                    .clicked()
                {
                    self.command(Command::NewUntitled);
                }
                ui.label(
                    RichText::new(format!(
                        "{} to open a file  •  {} for a new file",
                        super::shortcut::primary("O"),
                        super::shortcut::primary("N")
                    ))
                    .size(11.0)
                    .color(colors.muted),
                );
                self.recent_files_list(ui);
            });
            return;
        };
        self.breadcrumb_bar(ui, index);
        ui.add_space(2.0);
        let document_id = self.editor.files[index].id;
        if self.editor.files[index].disk_changed {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("File changed on disk")
                        .size(11.0)
                        .color(colors.muted),
                );
                if ui.small_button("Review Changes").clicked() {
                    self.command(Command::ReviewDiskChange(document_id));
                }
            });
        }
        self.editor_find_bar(ui, document_id);
        #[cfg(not(feature = "shared-editor"))]
        let hover_position = ui.cursor().min + egui::vec2(56.0, 22.0);
        self.editor_viewport(ui, index);
        // The shared editor anchors hover to the caret in its own popups.
        #[cfg(not(feature = "shared-editor"))]
        if let Some(hover) = self.view.language_hover.clone() {
            egui::Area::new(egui::Id::new("language_server_hover"))
                .order(egui::Order::Foreground)
                .fixed_pos(hover_position)
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(colors.panel)
                        .stroke(egui::Stroke::new(1.0_f32, colors.border))
                        .corner_radius(3)
                        .inner_margin(egui::Margin::symmetric(8, 5))
                        .show(ui, |ui| {
                            ui.set_max_width(520.0);
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(hover)
                                            .monospace()
                                            .size(11.0)
                                            .color(colors.text),
                                    )
                                    .wrap(),
                                );
                                if ui
                                    .small_button(icon::X)
                                    .on_hover_text("Close hover")
                                    .clicked()
                                {
                                    self.view.language_hover = None;
                                }
                            });
                        });
                });
        }
    }
}
