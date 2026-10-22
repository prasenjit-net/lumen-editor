use super::{
    Workbench,
    match_highlight::{append_highlighted, match_color},
};
use crate::app::{command::Command, settings::ThemeChoice};
use crate::icon;
use eframe::egui::{self, FontId, TextEdit, Vec2};
impl Workbench<'_> {
    /// One `Run Task` entry per defined task, recently started ones first.
    fn task_palette_entries(&self) -> Vec<(String, Command)> {
        use lumen_task_core::{Scope, TaskKey};
        let recent = &self.view.task_runs.recent;
        let mut entries: Vec<(usize, String, Command)> = Vec::new();
        for scope in [Scope::Workspace, Scope::User] {
            for task in self.view.tasks.tasks(scope) {
                let key = TaskKey::new(scope, &task.id);
                let rank = recent
                    .iter()
                    .position(|other| *other == key)
                    .unwrap_or(usize::MAX);
                entries.push((
                    rank,
                    format!(
                        "{}  Run Task: {} ({})",
                        icon::PLAY,
                        task.label,
                        scope.label().to_lowercase()
                    ),
                    Command::RunTask(key),
                ));
            }
        }
        entries.sort_by_key(|(rank, label, _)| (*rank, label.clone()));
        entries
            .into_iter()
            .map(|(_, label, command)| (label, command))
            .collect()
    }

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
                .button(format!("{}  Open Recent Folder…", icon::FOLDER_OPEN))
                .clicked()
            {
                self.command(Command::ShowRecentFolders);
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
                    self.editor.active_index().is_some(),
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
                    self.editor.active_index().is_some(),
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
                    self.editor.active_index().is_some() || self.view.page_tab_active(),
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

    pub(super) fn palette_disabled_reason(&self, command: &Command) -> Option<&'static str> {
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
        );
        if matches!(command, Save) && self.view.task_editors.active().is_some() {
            return None;
        }
        if needs_document && (self.active().is_none() || self.view.page_tab_active()) {
            return Some("Open a text document first");
        }
        if matches!(
            command,
            NewTaskEditor(lumen_task_core::Scope::Workspace)
                | OpenTaskToml(lumen_task_core::Scope::Workspace)
        ) && self.editor.workspace.is_none()
        {
            return Some("Open a folder first");
        }
        if matches!(command, CloseEditor) && self.active().is_none() && !self.view.page_tab_active()
        {
            return Some("No editor is open");
        }
        if matches!(
            command,
            NextTab
                | PreviousTab
                | CloseOtherTabs
                | CloseTabsToLeft
                | CloseTabsToRight
                | CloseSavedTabs
                | CloseAllTabs
        ) && self.editor.files.is_empty()
        {
            return Some("No tabs are open");
        }
        if matches!(command, MoveActiveTabLeft | MoveActiveTabRight) {
            let tabs = crate::app::tab_close::visible_documents(self.editor, self.view);
            let position = self
                .editor
                .active_id()
                .and_then(|id| tabs.iter().position(|tab| *tab == id));
            match (command, position) {
                (_, None) => return Some("No tab is active"),
                (MoveActiveTabLeft, Some(0)) => return Some("This is already the first tab"),
                (MoveActiveTabRight, Some(position)) if position + 1 == tabs.len() => {
                    return Some("This is already the last tab");
                }
                _ => {}
            }
        }
        if matches!(command, SplitEditor) {
            if !self.view.settings.boolean("workbench.split_editor") {
                return Some("Turn on Split Editor in Settings first");
            }
            if self.editor.active().is_none() {
                return Some("No tab is active");
            }
            let tabs = crate::app::tab_close::visible_documents(self.editor, self.view);
            if self.editor.group_count() < 2 && tabs.len() < 2 {
                return Some("Open another tab to split the editor");
            }
        }
        if matches!(command, FocusNextGroup | ToggleSplitOrientation)
            && self.editor.group_count() < 2
        {
            return Some("The editor is not split");
        }
        if matches!(command, CloseOtherTabs) && self.editor.files.len() < 2 {
            return Some("There are no other tabs");
        }
        if matches!(command, OpenActivePreview)
            && self
                .editor
                .active()
                .is_none_or(|file| file.preview_kind().is_none())
        {
            return Some("The active file has no preview");
        }
        if matches!(command, ReopenClosedTab) && self.view.closed_tabs.is_empty() {
            return Some("No recently closed tabs");
        }
        if matches!(command, ClearRecentHistory)
            && self.view.navigation.recent.is_empty()
            && self.view.navigation.recent_folders.is_empty()
        {
            return Some("There is no history to clear");
        }
        if matches!(command, NewFolder | CloseFolder | ShowWorkspaceSearch)
            && self.editor.workspace.is_none()
        {
            return Some("Open a folder first");
        }
        if matches!(command, ToggleAgentPanel) && !self.view.settings.boolean("agent.enabled") {
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
            // A quick input has no title strip: the hint in the field says what it is.
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .default_width(width)
            .min_width(width)
            .max_width(width)
            .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 48.0))
            .frame(super::widgets::floating_frame(ctx))
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
                let results = matching_with(&self.view.palette_query, self.task_palette_entries());
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
                            let mut job = egui::text::LayoutJob::default();
                            append_highlighted(
                                &mut job,
                                label,
                                &self.view.palette_query,
                                FontId::proportional(12.5),
                                colors.text,
                                match_color(ctx),
                            );
                            let response = ui.add_enabled(
                                reason.is_none(),
                                egui::Button::new(job)
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

#[cfg(test)]
fn matching_commands(query: &str) -> Vec<(String, Command)> {
    matching_with(query, Vec::new())
}

fn matching_with(query: &str, extra: Vec<(String, Command)>) -> Vec<(String, Command)> {
    let query = query.trim().to_lowercase();
    palette_commands()
        .into_iter()
        .chain(extra)
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
        ReopenClosedTab => "Shift+T",
        OpenActivePreview => "Shift+V",
        SplitEditor => "\\",
        ToggleAgentPanel => "Shift+A",
        ToggleOutput => "Shift+U",
        Replace => {
            return if cfg!(target_os = "macos") {
                super::shortcut::primary("Alt+F")
            } else {
                "Ctrl+H".into()
            };
        }
        MoveActiveTabLeft => return super::shortcut::primary("Shift+Alt+Left"),
        MoveActiveTabRight => return super::shortcut::primary("Shift+Alt+Right"),
        NextTab => return super::shortcut::explicit("Ctrl+Tab"),
        PreviousTab => return super::shortcut::explicit("Ctrl+Shift+Tab"),
        GoToDefinition => return super::shortcut::explicit("F12"),
        FindReferences => return super::shortcut::explicit("Shift+F12"),
        RenameSymbol => return super::shortcut::explicit("F2"),
        FormatDocument => return super::shortcut::explicit("Shift+Alt+F"),
        TriggerCompletion => return super::shortcut::explicit("Ctrl+Space"),
        ShowSignatureHelp => return super::shortcut::explicit("Ctrl+Shift+Space"),
        ToggleTerminal => return super::shortcut::explicit("Ctrl+`"),
        _ => return String::new(),
    };
    super::shortcut::primary(suffix)
}

fn palette_commands() -> Vec<(String, Command)> {
    [
        (
            format!("{}  Git: Switch or Manage Branches", icon::GIT_BRANCH),
            Command::OpenBranchDialog,
        ),
        (
            format!("{}  Git: Stash Changes", icon::GIT_BRANCH),
            Command::OpenStashDialog,
        ),
        (
            format!("{}  Git: Manage Tags", icon::GIT_BRANCH),
            Command::OpenTagsDialog(None),
        ),
        (
            format!("{}  Git: Show History", icon::HISTORY),
            Command::OpenHistoryDialog,
        ),
        (
            format!("{}  Git: Show File History", icon::HISTORY),
            Command::OpenFileHistory,
        ),
        (
            format!("{}  Git: Blame Current File", icon::HISTORY),
            Command::OpenBlame,
        ),
        (
            format!("{}  Git: Clone Repository", icon::GIT_BRANCH),
            Command::OpenCloneDialog,
        ),
        (
            format!("{}  Preferences: Open Settings", icon::GEAR),
            Command::Settings,
        ),
        (
            format!("{}  Tasks: New Workspace Task", icon::DEBUG),
            Command::NewTaskEditor(lumen_task_core::Scope::Workspace),
        ),
        (
            format!("{}  Tasks: New User Task", icon::DEBUG),
            Command::NewTaskEditor(lumen_task_core::Scope::User),
        ),
        (
            format!("{}  Tasks: Run Selected Task", icon::PLAY),
            Command::RunSelectedTask,
        ),
        (
            format!("{}  Tasks: Rerun Last Task", icon::PLAY),
            Command::RerunLastTask,
        ),
        (
            format!("{}  Tasks: Run Build Task", icon::PLAY),
            Command::RunDefaultTask("build"),
        ),
        (
            format!("{}  Tasks: Run Test Task", icon::PLAY),
            Command::RunDefaultTask("test"),
        ),
        (
            format!("{}  Tasks: Show Running Tasks", icon::DEBUG),
            Command::ShowRunningTasks,
        ),
        (
            format!(
                "{}  Tasks: Open Workspace Tasks File (TOML)",
                icon::FILE_CODE
            ),
            Command::OpenTaskToml(lumen_task_core::Scope::Workspace),
        ),
        (
            format!("{}  Tasks: Open User Tasks File (TOML)", icon::FILE_CODE),
            Command::OpenTaskToml(lumen_task_core::Scope::User),
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
        ("View: Keep Editor Open".into(), Command::PinActiveTab),
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
            format!("{}  File: Open Recent Folder…", icon::FOLDER_OPEN),
            Command::ShowRecentFolders,
        ),
        (
            format!("{}  File: Close Folder", icon::X),
            Command::CloseFolder,
        ),
        (
            format!("{}  File: Clear Recently Opened", icon::X),
            Command::ClearRecentHistory,
        ),
        (format!("{}  File: Save", icon::FLOPPY_DISK), Command::Save),
        (
            format!("{}  File: Close Document", icon::X),
            Command::CloseEditor,
        ),
        ("Tab: Next Tab".into(), Command::NextTab),
        ("Tab: Previous Tab".into(), Command::PreviousTab),
        ("Tab: Reopen Closed Tab".into(), Command::ReopenClosedTab),
        (
            format!("{}  Open Preview", icon::PREVIEW),
            Command::OpenActivePreview,
        ),
        (
            "View: Split Editor / Move Tab to Other Group".into(),
            Command::SplitEditor,
        ),
        (
            "View: Focus Next Editor Group".into(),
            Command::FocusNextGroup,
        ),
        (
            "View: Toggle Editor Group Layout (Side by Side / Stacked)".into(),
            Command::ToggleSplitOrientation,
        ),
        ("Tab: Move Tab Left".into(), Command::MoveActiveTabLeft),
        ("Tab: Move Tab Right".into(), Command::MoveActiveTabRight),
        ("Tab: Close Other Tabs".into(), Command::CloseOtherTabs),
        (
            "Tab: Close Tabs to the Left".into(),
            Command::CloseTabsToLeft,
        ),
        (
            "Tab: Close Tabs to the Right".into(),
            Command::CloseTabsToRight,
        ),
        ("Tab: Close Saved Tabs".into(), Command::CloseSavedTabs),
        ("Tab: Close All Tabs".into(), Command::CloseAllTabs),
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
            format!("{}  View: Output", icon::OUTPUT),
            Command::ToggleOutput,
        ),
    ]
    .into_iter()
    .chain(ThemeChoice::ALL.map(|choice| {
        (
            format!("{}  Color Theme: {}", icon::GEAR, choice.label()),
            Command::SetColorTheme(choice),
        )
    }))
    .chain([(
        format!("{}  View: Problems", icon::WARNING),
        Command::ToggleProblems,
    )])
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
        assert!(results.iter().any(|(_, command)| *command == Command::Save));
        assert!(
            results
                .iter()
                .all(|(label, _)| label.to_lowercase().contains("save"))
        );
        let exact = matching_commands("  FILE: SAVE ");
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].1, Command::Save);
        assert_eq!(matching_commands("folder explorer").len(), 1);
        assert!(matching_commands("nonexistent command").is_empty());
    }

    #[test]
    fn typing_filters_then_enter_executes_once() {
        let (mut app, ctx) = opened();
        frame(&mut app, &ctx, vec![egui::Event::Text("new text".into())]);
        assert_eq!(app.view.palette_query, "new text");
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
        assert!(!app.view.command_palette);
        assert_eq!(app.editor.files.len(), 1);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
        assert_eq!(app.editor.files.len(), 1);
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

    #[test]
    fn tab_commands_are_listed_and_explain_when_they_cannot_run() {
        let labels: Vec<_> = matching_commands("tab:").into_iter().collect();
        for command in [
            Command::NextTab,
            Command::PreviousTab,
            Command::MoveActiveTabLeft,
            Command::MoveActiveTabRight,
            Command::ReopenClosedTab,
            Command::CloseOtherTabs,
            Command::CloseTabsToLeft,
            Command::CloseTabsToRight,
            Command::CloseSavedTabs,
            Command::CloseAllTabs,
        ] {
            assert!(
                labels.iter().any(|(_, listed)| *listed == command),
                "{command:?} missing from the palette"
            );
        }
        let (mut app, ctx) = opened();
        let reason = |app: &mut LumenApp, command: Command| {
            let mut found = None;
            app.render_component(&ctx, |workbench| {
                found = workbench.palette_disabled_reason(&command);
            });
            found
        };
        assert_eq!(reason(&mut app, Command::NextTab), Some("No tabs are open"));
        assert_eq!(
            reason(&mut app, Command::ReopenClosedTab),
            Some("No recently closed tabs")
        );
        app.editor.new_file();
        assert_eq!(
            reason(&mut app, Command::MoveActiveTabLeft),
            Some("This is already the first tab")
        );
        assert_eq!(
            reason(&mut app, Command::MoveActiveTabRight),
            Some("This is already the last tab")
        );
        assert_eq!(reason(&mut app, Command::NextTab), None);
        assert_eq!(
            reason(&mut app, Command::CloseOtherTabs),
            Some("There are no other tabs")
        );
        app.editor.new_file();
        assert_eq!(reason(&mut app, Command::CloseOtherTabs), None);
        assert_eq!(reason(&mut app, Command::MoveActiveTabLeft), None);
        assert_eq!(
            reason(&mut app, Command::MoveActiveTabRight),
            Some("This is already the last tab"),
            "the newest tab is active and last"
        );
        app.view
            .closed_tabs
            .push(crate::app::view_state::ClosedTab {
                path: "/x".into(),
                caret: 0,
            });
        assert_eq!(reason(&mut app, Command::ReopenClosedTab), None);
    }

    #[test]
    fn open_preview_is_offered_only_for_previewable_active_files() {
        let (mut app, ctx) = opened();
        let reason = |app: &mut LumenApp| {
            let mut found = None;
            app.render_component(&ctx, |workbench| {
                found = workbench.palette_disabled_reason(&Command::OpenActivePreview);
            });
            found
        };
        app.editor.new_file();
        assert_eq!(reason(&mut app), Some("The active file has no preview"));
        app.editor.files.push(crate::model::Document::from_path(
            "/w/readme.md".into(),
            "# Hi".into(),
        ));
        let last = app.editor.files.len() - 1;
        app.editor.set_active_index(Some(last));
        assert_eq!(reason(&mut app), None);
    }

    #[test]
    fn run_task_entries_list_defined_tasks_with_recent_ones_first() {
        use crate::app::task_catalog::{ScopeState, ScopeTasks};
        use lumen_task_core::{Scope, TaskDef, TaskKey, TaskKind};
        let task = |id: &str, label: &str| {
            let mut task = TaskDef::new(id, label, TaskKind::Process);
            task.command = Some("true".into());
            task
        };
        let (mut app, _) = opened();
        app.view.tasks.workspace = ScopeTasks {
            path: None,
            state: ScopeState::Loaded {
                tasks: vec![
                    task("build", "Build"),
                    task("test", "Test"),
                    task("zed", "Zed"),
                ],
                text: String::new(),
            },
        };
        app.view.tasks.user = ScopeTasks {
            path: None,
            state: ScopeState::Loaded {
                tasks: vec![task("lint", "Lint")],
                text: String::new(),
            },
        };
        app.view
            .task_runs
            .note_started(&TaskKey::new(Scope::Workspace, "zed"));
        let workbench = Workbench::new(&mut app.editor, &mut app.view, app.file_system.as_ref());
        let entries = workbench.task_palette_entries();
        let labels: Vec<&str> = entries.iter().map(|(label, _)| label.as_str()).collect();
        assert!(
            labels[0].contains("Run Task: Zed (workspace)"),
            "recent first: {labels:?}"
        );
        assert!(labels.iter().any(|l| l.contains("Run Task: Lint (user)")));
        assert_eq!(labels.len(), 4);
        assert!(matches!(&entries[0].1, Command::RunTask(key) if key.id == "zed"));
        let found = matching_with("run task lint", workbench.task_palette_entries());
        assert_eq!(found.len(), 1);
    }
}
