//! Searchable language-server location picker.
use super::Workbench;
use crate::app::command::Command;
use eframe::egui::{self, RichText, TextEdit, Vec2};

impl Workbench<'_> {
    pub(crate) fn language_navigation(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        if !self.view.language_navigation.open {
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.view.language_navigation.open = false;
            return;
        }

        let query = self.view.language_navigation.query.to_lowercase();
        let matches = self
            .view
            .language_navigation
            .locations
            .iter()
            .enumerate()
            .filter(|(_, location)| {
                query.is_empty() || location.label.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if matches.is_empty() {
            self.view.language_navigation.selected = 0;
        } else {
            self.view.language_navigation.selected = self
                .view
                .language_navigation
                .selected
                .min(matches.len() - 1);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown))
            && !matches.is_empty()
        {
            self.view.language_navigation.selected =
                (self.view.language_navigation.selected + 1).min(matches.len() - 1);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
            self.view.language_navigation.selected =
                self.view.language_navigation.selected.saturating_sub(1);
        }

        let mut open = true;
        let mut chosen = None;
        egui::Window::new(self.view.language_navigation.title.clone())
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .default_height(320.0)
            .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 72.0))
            .frame(super::widgets::floating_frame(ctx))
            .show(ctx, |ui| {
                let search = ui.add(
                    TextEdit::singleline(&mut self.view.language_navigation.query)
                        .id(egui::Id::new("language_navigation_query"))
                        .hint_text("Search symbols or locations")
                        .desired_width(f32::INFINITY),
                );
                if self.view.language_navigation.focus {
                    search.request_focus();
                    self.view.language_navigation.focus = false;
                }
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if matches.is_empty() {
                        ui.label(RichText::new("No matching locations").color(colors.muted));
                    }
                    for (row, index) in matches.iter().copied().enumerate() {
                        let location = &self.view.language_navigation.locations[index];
                        let path = self
                            .editor
                            .workspace
                            .as_ref()
                            .and_then(|root| location.path.strip_prefix(root).ok())
                            .unwrap_or(&location.path);
                        let response = ui
                            .selectable_label(
                                row == self.view.language_navigation.selected,
                                RichText::new(&location.label).color(colors.text),
                            )
                            .on_hover_text(path.display().to_string());
                        if response.clicked() {
                            chosen = Some(index);
                        }
                    }
                });
            });

        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
            chosen = matches.get(self.view.language_navigation.selected).copied();
        }
        if let Some(index) = chosen {
            self.command(Command::OpenLanguageLocation(index));
        }
        if !open {
            self.view.language_navigation.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LumenApp;

    #[test]
    fn escape_closes_the_language_location_picker() {
        let mut app = LumenApp::default();
        app.view.language_navigation.open = true;
        app.view.language_navigation.title = "Go to Definition".into();
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.language_navigation(ctx)),
        );
        assert!(!app.view.language_navigation.open);
    }
}
