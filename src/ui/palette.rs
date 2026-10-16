use super::Workbench;
use crate::app::command::Command;
use crate::icon;
use eframe::egui::{self, CornerRadius, Frame, Stroke, TextEdit, Vec2};
impl Workbench<'_> {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    pub(crate) fn file_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            if ui
                .button(format!(
                    "{}  New Text File   {}",
                    icon::FILE_PLUS,
                    super::shortcut::primary("N")
                ))
                .clicked()
            {
                self.command(Command::NewUntitled);
                ui.close();
            }
            if ui
                .button(format!(
                    "{}  Open File…   {}",
                    icon::FILE_ARROW_UP,
                    super::shortcut::primary("O")
                ))
                .clicked()
            {
                self.command(Command::OpenFile);
                ui.close();
            }
            if ui
                .button(format!(
                    "{}  Open Folder…   {}",
                    icon::FOLDER_OPEN,
                    super::shortcut::primary("Shift+O")
                ))
                .clicked()
            {
                self.command(Command::OpenFolder);
                ui.close();
            }
            if ui
                .add_enabled(
                    self.editor.workspace.is_some(),
                    egui::Button::new(format!("{}  Close Folder", icon::X)),
                )
                .clicked()
            {
                self.command(Command::CloseFolder);
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled(
                    self.editor.active_file.is_some(),
                    egui::Button::new(format!(
                        "{}  Save   {}",
                        icon::FLOPPY_DISK,
                        super::shortcut::primary("S")
                    )),
                )
                .clicked()
            {
                self.command(Command::Save);
                ui.close();
            }
            if ui
                .add_enabled(
                    self.editor.active_file.is_some(),
                    egui::Button::new(format!(
                        "{}  Save As…   {}",
                        icon::FLOPPY_DISK,
                        super::shortcut::primary("Shift+S")
                    )),
                )
                .clicked()
            {
                self.command(Command::SaveAs);
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled(
                    self.editor.active_file.is_some() || self.view.settings.open,
                    egui::Button::new(format!(
                        "{}  Close Document   {}",
                        icon::X,
                        super::shortcut::primary("W")
                    )),
                )
                .clicked()
            {
                self.command(Command::CloseEditor);
                ui.close();
            }
        });
    }

    fn close_palette(&mut self, ctx: &egui::Context) {
        self.view.command_palette = false;
        self.view.command_palette_focus = false;
        if let Some(previous) = self.view.palette_previous_focus.take() {
            ctx.memory_mut(|memory| memory.request_focus(previous));
        }
    }

    fn palette_disabled_reason(&self, command: &Command) -> Option<&'static str> {
        use Command::*;
        let needs_document = matches!(
            command,
            Save | Find
                | Replace
                | GoToLine
                | GoToDefinition
                | ShowHover
                | TriggerCompletion
                | FindReferences
                | RenameSymbol
                | FormatDocument
                | ShowSignatureHelp
                | ShowDocumentSymbols
                | AskAgentAboutSelection
        );
        if needs_document && (self.active().is_none() || self.view.settings.open) {
            return Some("Open a text document first");
        }
        if matches!(command, CloseEditor) && self.active().is_none() && !self.view.settings.open {
            return Some("No editor is open");
        }
        if matches!(command, NewFolder | CloseFolder | ShowWorkspaceSearch)
            && self.editor.workspace.is_none()
        {
            return Some("Open a folder first");
        }
        if matches!(command, ToggleAgentPanel | AskAgentAboutSelection)
            && !self.view.settings.boolean("agent.enabled")
        {
            return Some("Enable coding agents in Settings");
        }
        let capability = match command {
            GoToDefinition => Some("Go to Definition"),
            ShowHover => Some("Hover"),
            TriggerCompletion => Some("Completion"),
            FindReferences => Some("References"),
            RenameSymbol => Some("Rename"),
            FormatDocument => Some("Formatting"),
            ShowSignatureHelp => Some("Signature help"),
            ShowDocumentSymbols => Some("Document symbols"),
            _ => None,
        };
        if let Some(capability) = capability {
            if !matches!(
                self.view.language_server_status,
                Some(crate::app::view_state::LanguageServerStatus::Ready(_))
            ) {
                return Some("Wait for a language server to be ready for this document");
            }
            if !self
                .view
                .language_server_summary
                .as_ref()
                .is_some_and(|summary| summary.capabilities.contains(&capability))
            {
                return Some("The language server does not support this command");
            }
        }
        None
    }

    pub(crate) fn palette(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        if !self.view.command_palette {
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.close_palette(ctx);
            return;
        }
        // Consume list keys before TextEdit can move its caret or surrender focus.
        let (up, down, enter) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            )
        });
        let width = (ctx.content_rect().width() - 40.0).clamp(160.0, 560.0);
        let height = (ctx.content_rect().height() - 150.0).clamp(44.0, 360.0);
        let mut open = true;
        let mut chosen = None;
        egui::Window::new("Command Palette")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(width)
            .min_width(width)
            .max_width(width)
            .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 48.0))
            .frame(
                Frame::window(&ctx.style())
                    .fill(colors.panel)
                    .corner_radius(CornerRadius::same(6))
                    .stroke(Stroke::new(1.0_f32, colors.border)),
            )
            .show(ctx, |ui| {
                let query = ui.add(
                    TextEdit::singleline(&mut self.view.palette_query)
                        .id(egui::Id::new("command_palette_query"))
                        .hint_text("Type a command")
                        .desired_width(f32::INFINITY),
                );
                let opening = self.view.command_palette_focus;
                if opening {
                    query.request_focus();
                    self.view.command_palette_focus = false;
                }
                let results = matching_commands(&self.view.palette_query);
                if query.changed() {
                    self.view.palette_selected = 0;
                }
                self.view.palette_selected = self
                    .view
                    .palette_selected
                    .min(results.len().saturating_sub(1));
                if down {
                    self.view.palette_selected =
                        (self.view.palette_selected + 1).min(results.len().saturating_sub(1));
                }
                if up {
                    self.view.palette_selected = self.view.palette_selected.saturating_sub(1);
                }
                ui.separator();
                if results.is_empty() {
                    ui.label("No commands match your search.");
                }
                egui::ScrollArea::vertical()
                    .id_salt("command_palette_results")
                    .max_height(height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (index, (label, command)) in results.iter().enumerate() {
                            let reason = self.palette_disabled_reason(command);
                            let selected = index == self.view.palette_selected;
                            let shortcut = command_shortcut(command);
                            let response = ui.add_enabled(
                                reason.is_none(),
                                egui::Button::new(egui::RichText::new(label).color(colors.text))
                                    .selected(selected)
                                    .shortcut_text(shortcut)
                                    .truncate()
                                    .min_size(egui::vec2(ui.available_width(), 22.0)),
                            );
                            if selected && (opening || query.changed() || up || down) {
                                response.scroll_to_me(None);
                            }
                            if response.clicked() || (selected && enter && reason.is_none()) {
                                chosen = Some(command.clone());
                            }
                            if let Some(reason) = reason {
                                response.on_disabled_hover_text(reason);
                                if selected {
                                    ui.label(
                                        egui::RichText::new(reason).small().color(colors.muted),
                                    );
                                }
                            }
                        }
                    });
            });
        if !open || chosen.is_some() {
            self.close_palette(ctx);
        }
        if let Some(command) = chosen {
            self.command(command);
        }
    }
}

fn matching_commands(query: &str) -> Vec<(String, Command)> {
    let query = query.trim().to_lowercase();
    palette_commands()
        .into_iter()
        .filter(|(label, _)| {
            let label = label.to_lowercase();
            query.split_whitespace().all(|word| label.contains(word))
        })
        .collect()
}

fn command_shortcut(command: &Command) -> String {
    use Command::*;
    let suffix = match command {
        Settings => ",",
        NewUntitled => "N",
        OpenFile => "O",
        OpenFolder => "Shift+O",
        Save => "S",
        CloseEditor => "W",
        QuickOpen => "P",
        GoToLine => "G",
        Find => "F",
        ShowWorkspaceSearch => "Shift+F",
        ShowDocumentSymbols => "Shift+O",
        ToggleAgentPanel => "Shift+A",
        ToggleOutput => "Shift+U",
        Replace => {
            return if cfg!(target_os = "macos") {
                super::shortcut::primary("Alt+F")
            } else {
                "Ctrl+H".into()
            };
        }
        GoToDefinition => return "F12".into(),
        FindReferences => return "Shift+F12".into(),
        RenameSymbol => return "F2".into(),
        FormatDocument => return "Shift+Alt+F".into(),
        TriggerCompletion => return "Ctrl+Space".into(),
        ShowSignatureHelp => return "Ctrl+Shift+Space".into(),
        ToggleTerminal => return "Ctrl+`".into(),
        _ => return String::new(),
    };
    super::shortcut::primary(suffix)
}

fn palette_commands() -> Vec<(String, Command)> {
    [
        (
            format!("{}  Preferences: Open Settings", icon::GEAR),
            Command::Settings,
        ),
        (
            format!("{}  File: New Text File", icon::FILE_PLUS),
            Command::NewUntitled,
        ),
        (
            format!("{}  Explorer: New File", icon::FILE_PLUS),
            Command::NewFile,
        ),
        (
            format!("{}  Explorer: New Folder", icon::NEW_FOLDER),
            Command::NewFolder,
        ),
        (
            format!("{}  File: Open File", icon::FILE_ARROW_UP),
            Command::OpenFile,
        ),
        (format!("{}  Go to File", icon::FILE), Command::QuickOpen),
        ("Go to Line/Column".into(), Command::GoToLine),
        ("Go to Definition".into(), Command::GoToDefinition),
        ("Show Hover".into(), Command::ShowHover),
        ("Trigger Suggest".into(), Command::TriggerCompletion),
        ("Find All References".into(), Command::FindReferences),
        ("Rename Symbol".into(), Command::RenameSymbol),
        ("Format Document".into(), Command::FormatDocument),
        ("Show Signature Help".into(), Command::ShowSignatureHelp),
        (
            "Go to Symbol in Editor".into(),
            Command::ShowDocumentSymbols,
        ),
        (
            format!("{}  File: Open Folder", icon::FOLDER_OPEN),
            Command::OpenFolder,
        ),
        (
            format!("{}  File: Close Folder", icon::X),
            Command::CloseFolder,
        ),
        (format!("{}  File: Save", icon::FLOPPY_DISK), Command::Save),
        (
            format!("{}  File: Close Document", icon::X),
            Command::CloseEditor,
        ),
        (
            format!("{}  Editor: Find", icon::MAGNIFYING_GLASS),
            Command::Find,
        ),
        ("Editor: Replace".into(), Command::Replace),
        (
            format!("{}  Search: Find in Files", icon::MAGNIFYING_GLASS),
            Command::ShowWorkspaceSearch,
        ),
        (
            format!("{}  Terminal: Toggle Terminal", icon::TERMINAL),
            Command::ToggleTerminal,
        ),
        (
            format!("{}  View: Coding Agents", icon::AGENT),
            Command::ToggleAgentPanel,
        ),
        (
            format!("{}  Agent: Ask About Selection", icon::AGENT),
            Command::AskAgentAboutSelection,
        ),
        (
            format!("{}  View: Output", icon::OUTPUT),
            Command::ToggleOutput,
        ),
        (
            format!("{}  View: Problems", icon::WARNING),
            Command::ToggleProblems,
        ),
    ]
    .into_iter()
    .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::LumenApp, ui::theme::configure_ui};

    fn frame(app: &mut LumenApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.palette(ctx)),
        );
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn opened() -> (LumenApp, egui::Context) {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        app.execute(Command::Palette, &ctx);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        (app, ctx)
    }

    #[test]
    fn final_result_is_keyboard_reachable_in_short_and_wide_windows() {
        for size in [
            egui::vec2(360.0, 300.0),
            egui::vec2(800.0, 600.0),
            egui::vec2(1400.0, 900.0),
        ] {
            let (mut app, ctx) = opened();
            let mut render = |events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |ctx| app.render_component(ctx, |workbench| workbench.palette(ctx)),
                )
            };
            for _ in 0..palette_commands().len() {
                render(vec![key(egui::Key::ArrowDown)]);
            }
            // Scroll animation needs elapsed frames before inspecting clipping.
            for _ in 0..30 {
                render(vec![]);
            }
            let output = render(vec![]);
            let visible = output.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains("View: Problems") => {
                    let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                    shape.clip_rect.contains_rect(rect)
                        && egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect)
                }
                _ => false,
            });
            assert!(visible, "last command must be visible at {size:?}");
            render(vec![key(egui::Key::Enter)]);
            assert!(app.view.problems.open);
            assert!(!app.view.command_palette);
        }
    }

    #[test]
    fn matching_is_case_insensitive_and_matches_all_words() {
        let results = matching_commands("  SAVE ");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, Command::Save);
        assert_eq!(matching_commands("folder explorer").len(), 1);
        assert!(matching_commands("nonexistent command").is_empty());
    }

    #[test]
    fn typing_filters_then_enter_executes_once_without_changing_extension_search() {
        let (mut app, ctx) = opened();
        app.view.search = "extension query".into();
        frame(&mut app, &ctx, vec![egui::Event::Text("new text".into())]);
        assert_eq!(app.view.palette_query, "new text");
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
        assert!(!app.view.command_palette);
        assert_eq!(app.editor.files.len(), 1);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
        assert_eq!(app.editor.files.len(), 1);
        assert_eq!(app.view.search, "extension query");
    }

    #[test]
    fn arrows_select_results_and_query_changes_reset_selection() {
        let (mut app, ctx) = opened();
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
        assert_eq!(app.view.palette_selected, 1);
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowUp)]);
        assert_eq!(app.view.palette_selected, 0);
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
        frame(&mut app, &ctx, vec![egui::Event::Text("new text".into())]);
        assert_eq!(app.view.palette_selected, 0);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
        assert_eq!(app.editor.files.len(), 1);
    }

    #[test]
    fn disabled_and_empty_results_cannot_execute() {
        let (mut app, ctx) = opened();
        frame(&mut app, &ctx, vec![egui::Event::Text("save".into())]);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
        assert!(app.view.command_palette);
        assert!(app.editor.files.is_empty());
        app.view.palette_query = "nothing matches this".into();
        frame(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowDown), key(egui::Key::Enter)],
        );
        assert!(app.view.command_palette);
        assert_eq!(app.view.palette_selected, 0);
    }

    #[test]
    fn escape_restores_the_original_widget_and_reopening_resets_query() {
        let (mut app, ctx) = opened();
        let original = egui::Id::new("terminal_canvas");
        app.view.palette_previous_focus = Some(original);
        app.view.palette_query = "output".into();
        frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
        assert!(ctx.memory(|memory| memory.has_focus(original)));
        app.execute(Command::Palette, &ctx);
        assert!(app.view.palette_query.is_empty());
        assert_eq!(app.view.palette_previous_focus, Some(original));
    }

    #[test]
    fn quick_open_remains_available_without_a_workspace_for_recent_files() {
        let (mut app, ctx) = opened();
        frame(&mut app, &ctx, vec![egui::Event::Text("go to file".into())]);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
        assert!(!app.view.command_palette);
        assert_eq!(
            app.view.navigation.mode,
            Some(crate::app::view_state::NavigationMode::QuickOpen)
        );
    }

    #[test]
    fn language_actions_require_ready_server_and_advertised_capability() {
        let (mut app, _) = opened();
        app.editor.new_file();
        let reason = |app: &mut LumenApp| {
            let workbench =
                Workbench::new(&mut app.editor, &mut app.view, app.file_system.as_ref());
            workbench.palette_disabled_reason(&Command::ShowHover)
        };
        assert!(reason(&mut app).is_some());
        app.view.language_server_status = Some(
            crate::app::view_state::LanguageServerStatus::Ready("test".into()),
        );
        app.view.language_server_summary = Some(crate::app::view_state::LanguageServerSummary {
            capabilities: vec!["Hover"],
            ..Default::default()
        });
        assert!(reason(&mut app).is_none());
        app.view
            .language_server_summary
            .as_mut()
            .unwrap()
            .capabilities
            .clear();
        assert!(reason(&mut app).is_some());
    }

    #[test]
    fn escape_closes_palette_even_when_query_has_focus() {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        app.execute(Command::Palette, &ctx);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("command_palette_query"))));
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(!app.view.command_palette);
    }
}
