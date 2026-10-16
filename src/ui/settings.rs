//! Schema-driven presentation. Controls emit drafts; only commands persist them.
use super::{
    Workbench,
    theme::{MUTED, TEXT},
    widgets::{HeaderAction, panel_header},
};
use crate::{
    app::{command::Command, settings::SettingsState},
    icon,
};
use eframe::egui::{self, RichText, ScrollArea, TextEdit};
use lumen_settings_core::{Control, SECTIONS, Scope, Setting, Value, catalog};

impl Workbench<'_> {
    pub(crate) fn settings_quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.view.settings.pending_quit {
            return;
        }
        egui::Window::new("Unapplied settings")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("Settings drafts have not been applied. Quitting will discard them.");
                ui.label("Reopen any previously edited project to apply its retained draft.");
                ui.horizontal(|ui| {
                    if ui.button("Keep Editing").clicked() {
                        self.view.settings.pending_quit = false;
                        self.command(Command::Settings);
                    }
                    if ui.button("Discard Settings and Continue").clicked() {
                        self.command(Command::DiscardSettingsAndQuit);
                    }
                    if ui.button("Cancel").clicked() {
                        self.view.settings.pending_quit = false;
                    }
                });
            });
    }
    pub(super) fn settings_page(&mut self, ui: &mut egui::Ui) {
        let actions = [
            HeaderAction {
                glyph: icon::FILE_ARROW_UP,
                tooltip: "Open saved config file",
                command: Command::OpenSettingsFile,
                enabled: self.view.settings.selected().original.is_some(),
            },
            HeaderAction {
                glyph: icon::REFRESH,
                tooltip: "Reload config from disk (cancel pending changes first)",
                command: Command::ReloadSettings,
                enabled: true,
            },
        ];
        if let Some(command) = panel_header(ui, "SETTINGS", &actions) {
            self.command(command);
        }
        let wide = ui.available_width() >= 600.0;
        toolbar(ui, &mut self.view.settings, wide);
        // The footer reserves space before category/results panels are allocated.
        // No result list can push Apply or errors outside the editor viewport.
        egui::TopBottomPanel::bottom("settings_footer")
            .resizable(false)
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| {
                if let Some(command) = footer(ui, &mut self.view.settings) {
                    self.commands.push(command);
                }
            });
        if wide {
            egui::SidePanel::left("settings_categories")
                .default_width(155.0)
                .width_range(110.0..=240.0)
                .resizable(true)
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    ScrollArea::vertical()
                        .id_salt("settings_category_scroll")
                        .show(ui, |ui| {
                            for category in categories() {
                                let count = catalog()
                                    .filter(|setting| {
                                        (category.is_empty() || setting.category == category)
                                            && matches(&self.view.settings, setting)
                                    })
                                    .count();
                                ui.selectable_value(
                                    &mut self.view.settings.category,
                                    category.into(),
                                    format!("{} ({count})", category_label(category)),
                                );
                            }
                        });
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| {
                results(ui, &mut self.view.settings);
            });
    }
}
fn categories() -> impl Iterator<Item = &'static str> {
    std::iter::once("").chain(SECTIONS.iter().map(|section| section.title))
}
fn category_label(category: &str) -> &str {
    if category.is_empty() {
        "All Settings"
    } else {
        category
    }
}
fn matches(state: &SettingsState, setting: &Setting) -> bool {
    let config = state.selected();
    setting.matches(
        &state.query,
        &config.persisted,
        config.draft.get(setting.id) != config.persisted.get(setting.id),
    )
}
fn toolbar(ui: &mut egui::Ui, state: &mut SettingsState, wide: bool) {
    ui.add_space(4.0);
    let response = ui
        .add(
            TextEdit::singleline(&mut state.query)
                .id_salt("settings_search")
                .hint_text("Search settings")
                .desired_width(f32::INFINITY),
        )
        .on_hover_text(
            "Search by name, ID or description. Filters: @modified, @pending, @id:editor.font_size",
        );
    if state.focus_search {
        response.request_focus();
        state.focus_search = false;
    }
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut state.scope, Scope::User, "User");
        ui.add_enabled_ui(state.root.is_some(), |ui| {
            ui.selectable_value(&mut state.scope, Scope::Project, "Project");
        })
        .response
        .on_hover_text("Project overrides User settings. Open a folder to enable this scope.");
        if ui.small_button("Modified").clicked() {
            state.query = "@modified".into();
        }
        if ui.small_button("Pending").clicked() {
            state.query = "@pending".into();
        }
        if ui.small_button("Clear").clicked() {
            state.query.clear();
            state.category.clear();
        }
    });
    if let Some(path) = &state.selected().path {
        ui.add(
            egui::Label::new(
                RichText::new(path.display().to_string())
                    .small()
                    .color(MUTED),
            )
            .truncate(),
        );
    }
    if !wide {
        egui::ComboBox::from_id_salt("settings_category")
            .selected_text(category_label(&state.category))
            .show_ui(ui, |ui| {
                for category in categories() {
                    ui.selectable_value(
                        &mut state.category,
                        category.into(),
                        category_label(category),
                    );
                }
            });
    }
    let count = catalog()
        .filter(|setting| {
            (state.category.is_empty() || state.category == setting.category)
                && matches(state, setting)
        })
        .count();
    ui.label(
        RichText::new(format!("{count} settings found"))
            .small()
            .color(MUTED),
    );
    ui.add_space(6.0);
}
fn results(ui: &mut egui::Ui, state: &mut SettingsState) {
    ScrollArea::vertical()
        .id_salt(("settings_results", state.scope.label()))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut category = "";
            for setting in catalog()
                .filter(|setting| state.category.is_empty() || state.category == setting.category)
                .collect::<Vec<_>>()
            {
                if !matches(state, setting) {
                    continue;
                }
                if category != setting.category {
                    category = setting.category;
                    ui.add_space(8.0);
                    ui.label(RichText::new(category).strong().size(16.0).color(TEXT));
                }
                ui.push_id(setting.id, |ui| setting_row(ui, state, setting));
            }
            if category.is_empty() {
                ui.label("No matching settings. Clear filters or try another search.");
            }
        });
}
fn setting_row(ui: &mut egui::Ui, state: &mut SettingsState, setting: &Setting) {
    let modified = state.selected().persisted.contains_key(setting.id);
    let pending =
        state.selected().draft.get(setting.id) != state.selected().persisted.get(setting.id);
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(setting.name).strong());
        if modified {
            ui.label(RichText::new("Modified").small().color(MUTED));
        }
        if pending {
            ui.label(RichText::new("Pending").small().color(MUTED));
        }
    });
    ui.label(RichText::new(setting.id).small().color(MUTED));
    ui.label(setting.help);
    let (effective, origin) = state.effective(setting, None);
    let mut value = state
        .selected()
        .draft
        .get(setting.id)
        .cloned()
        .unwrap_or_else(|| match state.scope {
            Scope::User => setting.default_value(),
            Scope::Project => lumen_settings_core::resolve(setting, &state.user.persisted, None).0,
        });
    let changed = control(ui, setting, &mut value);
    if changed {
        state.selected_mut().draft.insert(setting.id.into(), value);
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!("Effective: {effective} · {origin} · Live"))
                .small()
                .color(MUTED),
        );
        ui.menu_button(icon::GEAR, |ui| {
            if ui.button("Remove Override (inherit)").clicked() {
                state.selected_mut().draft.remove(setting.id);
                ui.close();
            }
            if ui.button("Set Built-in Default").clicked() {
                state
                    .selected_mut()
                    .draft
                    .insert(setting.id.into(), setting.default_value());
                ui.close();
            }
            if ui.button("Copy Setting ID").clicked() {
                ui.ctx().copy_text(setting.id.into());
                ui.close();
            }
        })
        .response
        .on_hover_text("Setting actions");
    });
    ui.add_space(8.0);
    ui.separator();
}
fn control(ui: &mut egui::Ui, setting: &Setting, value: &mut Value) -> bool {
    match (setting.control, value) {
        (Control::Number(min, max), Value::Number(value)) => ui
            .add(egui::DragValue::new(value).range(min..=max).speed(0.1))
            .on_hover_text(setting.name)
            .changed(),
        (Control::Bool, Value::Bool(value)) => ui.checkbox(value, setting.name).changed(),
        (Control::Choice(choices), Value::Choice(value)) => {
            let before = value.clone();
            egui::ComboBox::from_id_salt("value")
                .selected_text(value.as_str())
                .show_ui(ui, |ui| {
                    for choice in choices {
                        ui.selectable_value(value, (*choice).into(), *choice);
                    }
                });
            *value != before
        }
        (Control::Text(_), Value::Text(value)) => ui
            .add(
                TextEdit::singleline(value)
                    .hint_text(if setting.id.starts_with("language.") {
                        "Empty: search PATH"
                    } else {
                        "e.g. generated,*.min.js"
                    })
                    .desired_width(ui.available_width().clamp(100.0, 420.0)),
            )
            .on_hover_text(setting.help)
            .changed(),
        _ => false,
    }
}
fn footer(ui: &mut egui::Ui, state: &mut SettingsState) -> Option<Command> {
    let pending = state.selected().pending();
    let can_save = pending > 0 && state.selected().path.is_some();
    let mut command = None;
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(can_save, egui::Button::new("Apply"))
            .clicked()
        {
            command = Some(Command::ApplySettings);
        }
        if ui
            .add_enabled(pending > 0, egui::Button::new("Cancel"))
            .clicked()
        {
            state.selected_mut().draft = state.selected().persisted.clone();
        }
        ui.label(
            RichText::new(format!("{pending} pending changes"))
                .small()
                .color(MUTED),
        );
    });
    ScrollArea::vertical()
        .id_salt("settings_diagnostics")
        .max_height(70.0)
        .show(ui, |ui| {
            if let Some(error) = &state.selected().error {
                ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
            }
            for warning in &state.selected().warnings {
                ui.label(RichText::new(warning).small().color(MUTED));
            }
        });
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(
        app: &mut crate::app::LumenApp,
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.editor(ctx)),
        )
    }
    fn label_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains(label) => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing tab {label}"))
    }
    fn click(app: &mut crate::app::LumenApp, ctx: &egui::Context, width: f32, pos: egui::Pos2) {
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                width,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    #[test]
    fn switching_tabs_keeps_settings_present_and_drafts_intact_until_explicit_close() {
        for width in [300.0, 900.0, 1400.0] {
            let ctx = egui::Context::default();
            crate::ui::theme::configure_ui(&ctx);
            let mut app = crate::app::LumenApp::default();
            app.execute(Command::NewUntitled, &ctx);
            let name = app.editor.active().unwrap().name.clone();
            let id = app.editor.active().unwrap().id;
            app.execute(Command::Settings, &ctx);
            app.view.settings.query = "font".into();
            app.view
                .settings
                .user
                .draft
                .insert("editor.font_size".into(), Value::Number(16.0));
            let output = frame(&mut app, &ctx, width, vec![]);
            click(&mut app, &ctx, width, label_position(&output, &name));
            assert!(!app.view.settings.open);
            assert!(app.view.settings.tab_open);
            let output = frame(&mut app, &ctx, width, vec![]);
            click(&mut app, &ctx, width, label_position(&output, "Settings"));
            assert!(app.view.settings.open);
            assert_eq!(app.view.settings.query, "font");
            assert_eq!(app.view.settings.user.pending(), 1);
            assert_eq!(app.editor.active().unwrap().id, id);
            app.execute(Command::CloseEditor, &ctx);
            assert!(!app.view.settings.tab_open && !app.view.settings.open);
            assert_eq!(app.view.settings.user.pending(), 1);
            app.execute(Command::Settings, &ctx);
            assert!(app.view.settings.tab_open && app.view.settings.open);
        }
    }
    #[test]
    fn responsive_search_keeps_footer_visible_at_all_widths() {
        for width in [300.0, 900.0, 1400.0] {
            let ctx = egui::Context::default();
            crate::ui::theme::configure_ui(&ctx);
            let mut app = crate::app::LumenApp::default();
            app.view.settings.open = true;
            app.view.settings.query = "text size".into();
            let result_count = catalog()
                .filter(|setting| matches(&app.view.settings, setting))
                .count();
            let result_label = format!("{result_count} settings found");
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.render_component(ctx, |workbench| workbench.editor(ctx)),
            );
            for label in ["Apply", "Cancel", result_label.as_str(), "Font Size"] {
                let shape = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == label => Some(text),
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("missing {label} at width {width}"));
                assert!(shape.pos.x >= 0.0 && shape.pos.x + shape.galley.size().x <= width);
                assert!(shape.pos.y >= 0.0 && shape.pos.y + shape.galley.size().y <= 600.0);
            }
        }
    }
}
