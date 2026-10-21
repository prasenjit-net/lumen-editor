//! Schema-driven presentation. Controls emit drafts; only commands persist them.
use super::{
    Workbench,
    widgets::{HeaderAction, panel_header},
};
use crate::{
    app::{command::Command, settings::SettingsState, view_state::LanguageServerSummary},
    icon,
};
use eframe::egui::{self, RichText, ScrollArea, TextEdit};
use lumen_settings_core::{Control, SECTIONS, Scope, Setting, Value, catalog};

impl Workbench<'_> {
    pub(crate) fn settings_quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.view.settings.pending_quit {
            return;
        }
        let dialog = super::dialog::Dialog::new("Unapplied settings").show(ctx, |ui| {
            ui.add(
                egui::Label::new(
                    "Settings drafts have not been applied. Quitting will discard them.",
                )
                .wrap(),
            );
            ui.add(
                egui::Label::new(
                    "Reopen any previously edited project to apply its retained draft.",
                )
                .wrap(),
            );
            super::dialog::footer(ui, |ui| {
                let keep = super::dialogs::primary_button(ui, "Keep Editing");
                let cancel = ui.button("Cancel");
                let discard = ui.button("Discard Settings and Continue");
                super::dialogs::keep_focus(ui.ctx(), &[&keep, &discard, &cancel], &keep);
                if keep.clicked() {
                    Some(0)
                } else if discard.clicked() {
                    Some(1)
                } else if cancel.clicked() || super::dialogs::escape_pressed(ui.ctx()) {
                    Some(2)
                } else {
                    None
                }
            })
        });
        match dialog.inner.or(dialog.close_clicked.then_some(2)) {
            Some(0) => {
                self.view.settings.pending_quit = false;
                self.command(Command::Settings);
            }
            Some(1) => self.command(Command::DiscardSettingsAndQuit),
            Some(_) => {
                self.view.settings.pending_quit = false;
                self.view.focus_editor = true;
            }
            None => {}
        }
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
        // Inset the body like the header's content instead of touching the pane edge.
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(10, 0))
            .show(ui, |ui| {
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
                                                (category.is_empty()
                                                    || setting.category == category)
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
                        results(
                            ui,
                            &mut self.view.settings,
                            self.view.language_server_summary.as_ref(),
                        );
                    });
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
    let colors = crate::ui::theme::palette(ui.visuals());
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
        // Scope chooses which file edits go to; the filters below only narrow the list.
        ui.selectable_value(&mut state.scope, Scope::User, "User");
        ui.add_enabled_ui(state.root.is_some(), |ui| {
            ui.selectable_value(&mut state.scope, Scope::Project, "Project");
        })
        .response
        .on_hover_text("Project overrides User settings. Open a folder to enable this scope.");
        ui.separator();
        let pending = state.selected().pending();
        for (label, filter) in [
            ("Modified".to_owned(), "@modified"),
            (format!("Pending · {pending}"), "@pending"),
        ] {
            let on = state.query.trim() == filter;
            if ui
                .selectable_label(on, RichText::new(label).small())
                .clicked()
            {
                state.query = if on { String::new() } else { filter.into() };
            }
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
                    .color(colors.muted),
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
            .color(colors.muted),
    );
    ui.add_space(6.0);
}
fn results(
    ui: &mut egui::Ui,
    state: &mut SettingsState,
    language_server: Option<&LanguageServerSummary>,
) {
    let colors = crate::ui::theme::palette(ui.visuals());
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
                    ui.label(
                        RichText::new(category)
                            .strong()
                            .size(crate::ui::theme::SECTION_HEADING_SIZE)
                            .color(colors.text),
                    );
                    if category == "Language" {
                        if cfg!(feature = "language-server") {
                            language_server_status(ui, language_server);
                        } else {
                            ui.label(
                                RichText::new(
                                    "Not available in this build. Saved values are kept and apply again in a build with language server support.",
                                )
                                .small()
                                .color(colors.muted),
                            );
                        }
                    }
                }
                ui.push_id(setting.id, |ui| setting_row(ui, state, setting));
            }
            if category.is_empty() {
                ui.label("No matching settings. Clear filters or try another search.");
            }
        });
}
/// Read-only status for the active file's server; settings below stay the source of truth.
fn language_server_status(ui: &mut egui::Ui, summary: Option<&LanguageServerSummary>) {
    let colors = crate::ui::theme::palette(ui.visuals());
    ui.add_space(4.0);
    let Some(summary) = summary else {
        ui.label(
            RichText::new("Open a Rust or Go file to see its language server status.")
                .small()
                .color(colors.muted),
        );
        return;
    };
    let mut line = |label: &str, value: &str| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(label).small().color(colors.muted));
            ui.label(RichText::new(value).small().color(colors.text));
        });
    };
    line("Active language", summary.language);
    line("Server state", &summary.state);
    if let Some(command) = &summary.command {
        line("Command", command);
    }
    if !summary.capabilities.is_empty() {
        line("Capabilities", &summary.capabilities.join(", "));
    }
    if let Some(error) = &summary.error {
        line("Last error", error);
    }
    if summary.project_path_ignored {
        line(
            "Note",
            "Project settings cannot choose the server executable; set the path in User settings.",
        );
    }
}
/// Settings whose feature is not compiled into this build: still listed so saved
/// values are visible and survive, but not editable.
fn unavailable_in_this_build(setting: &Setting) -> bool {
    !cfg!(feature = "language-server") && setting.id.starts_with("language.")
}

fn setting_row(ui: &mut egui::Ui, state: &mut SettingsState, setting: &Setting) {
    let colors = crate::ui::theme::palette(ui.visuals());
    let modified = state.selected().persisted.contains_key(setting.id);
    let pending =
        state.selected().draft.get(setting.id) != state.selected().persisted.get(setting.id);
    let unavailable = unavailable_in_this_build(setting);
    let details_id = ui.id().with(("setting_details", setting.id));
    let show_details = ui
        .data(|data| data.get_temp::<bool>(details_id))
        .unwrap_or(false);
    ui.add_space(6.0);
    let top = ui.cursor().top();
    let left = ui.cursor().left();
    ui.add_enabled_ui(!unavailable, |ui| {
        let (effective, origin) = state.effective(setting, None);
        let mut value = state
            .selected()
            .draft
            .get(setting.id)
            .cloned()
            .unwrap_or_else(|| match state.scope {
                Scope::User => setting.default_value(),
                Scope::Project => {
                    lumen_settings_core::resolve(setting, &state.user.persisted, None).0
                }
            });
        // One visible label per setting: a boolean's checkbox carries the name, any
        // other control gets the name above it.
        ui.horizontal_wrapped(|ui| {
            if !matches!(setting.control, Control::Bool) {
                ui.label(RichText::new(setting.name).strong());
            }
            let changed = control(ui, setting, &mut value);
            if changed {
                state
                    .selected_mut()
                    .draft
                    .insert(setting.id.into(), value.clone());
            }
            if pending {
                ui.label(RichText::new("Pending").small().color(colors.info));
            } else if modified {
                ui.label(RichText::new("Modified").small().color(colors.muted));
            }
        });
        ui.add(egui::Label::new(RichText::new(setting.help).color(colors.muted)).wrap());
        ui.horizontal_wrapped(|ui| {
            let status = if pending {
                format!("Draft: {value} · Live: {effective}")
            } else {
                format!("Live: {effective}")
            };
            ui.label(RichText::new(status).small().color(colors.muted));
            let toggle = ui
                .add(
                    egui::Button::new(
                        RichText::new(if show_details {
                            "Hide details"
                        } else {
                            "Details"
                        })
                        .small()
                        .color(colors.muted),
                    )
                    .frame(false),
                )
                .on_hover_text("Show the setting ID, where its value comes from, and its default");
            if toggle.clicked() {
                ui.data_mut(|data| data.insert_temp(details_id, !show_details));
            }
            let gear = ui.menu_button(icon::GEAR, |ui| {
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
            });
            super::widgets::named(
                gear.response.on_hover_text("Setting actions"),
                true,
                "Setting actions",
            );
        });
        if show_details {
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "{} · {} scope · {origin} · default: {}",
                        setting.id,
                        state.scope.label(),
                        setting.default_value()
                    ))
                    .small()
                    .monospace()
                    .color(colors.muted),
                )
                .selectable(true)
                .wrap(),
            );
        }
    });
    if pending {
        // A draft that is not live yet reads differently from a saved value.
        ui.painter().vline(
            left - 6.0,
            top..=ui.cursor().top(),
            egui::Stroke::new(2.0_f32, colors.accent),
        );
    }
    ui.add_space(6.0);
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
    let colors = crate::ui::theme::palette(ui.visuals());
    let pending = state.selected().pending();
    let can_save = pending > 0 && state.selected().path.is_some();
    let mut command = None;
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                can_save,
                egui::Button::new(RichText::new("Apply").color(colors.on_accent))
                    .fill(colors.accent),
            )
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
            RichText::new(match pending {
                0 => "No pending changes".to_owned(),
                1 => "1 pending change".to_owned(),
                n => format!("{n} pending changes"),
            })
            .small()
            .color(colors.muted),
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
                ui.label(RichText::new(warning).small().color(colors.muted));
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

    fn texts(output: &egui::FullOutput) -> Vec<String> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                _ => None,
            })
            .collect()
    }

    fn open_settings(query: &str) -> (crate::app::LumenApp, egui::Context) {
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let mut app = crate::app::LumenApp::default();
        app.view.settings.open = true;
        app.view.settings.query = query.into();
        (app, ctx)
    }

    #[test]
    fn a_boolean_setting_shows_its_name_once_and_keeps_metadata_in_details() {
        let setting = catalog()
            .find(|setting| {
                matches!(setting.control, Control::Bool) && setting.id.starts_with("editor.")
            })
            .expect("a boolean editor setting");
        let (mut app, ctx) = open_settings(&format!("@id:{}", setting.id));
        let output = frame(&mut app, &ctx, 900.0, vec![]);
        let all = texts(&output);
        assert_eq!(
            all.iter()
                .filter(|text| text.as_str() == setting.name)
                .count(),
            1,
            "{} appears once: {all:?}",
            setting.name
        );
        assert!(
            !all.iter()
                .any(|text| text.contains(setting.id) && !text.starts_with("@id:")),
            "the technical ID waits behind Details"
        );
        assert!(all.iter().any(|text| text == "Details"));
    }

    #[test]
    fn a_pending_draft_shows_draft_and_live_values() {
        let setting = catalog()
            .find(|setting| {
                matches!(setting.control, Control::Bool) && setting.id.starts_with("editor.")
            })
            .expect("a boolean editor setting");
        let (mut app, ctx) = open_settings(&format!("@id:{}", setting.id));
        let live = match setting.default_value() {
            Value::Bool(value) => value,
            other => panic!("{other:?}"),
        };
        app.view
            .settings
            .user
            .draft
            .insert(setting.id.into(), Value::Bool(!live));
        let output = frame(&mut app, &ctx, 900.0, vec![]);
        let all = texts(&output);
        assert!(all.iter().any(|text| text == "Pending"), "{all:?}");
        assert!(
            all.iter()
                .any(|text| text.starts_with("Draft:") && text.contains("Live:")),
            "{all:?}"
        );
    }

    #[cfg(not(feature = "language-server"))]
    #[test]
    fn language_server_settings_are_marked_unavailable_without_the_feature() {
        let language = catalog()
            .find(|setting| setting.id.starts_with("language."))
            .expect("a language setting");
        assert!(unavailable_in_this_build(language));
        assert!(!unavailable_in_this_build(
            catalog()
                .find(|setting| setting.id.starts_with("editor."))
                .unwrap()
        ));
        let (mut app, ctx) = open_settings("");
        app.view.settings.category = "Language".into();
        let output = frame(&mut app, &ctx, 900.0, vec![]);
        assert!(
            texts(&output)
                .iter()
                .any(|text| text.starts_with("Not available in this build")),
        );
    }
}
