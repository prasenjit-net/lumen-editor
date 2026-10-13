use super::{Workbench, theme::MUTED};
use crate::{
    app::{command::Command, workspace_search::ReplaceScope},
    icon,
    services::workspace_search::{SearchFile, SearchHit},
};
use eframe::egui::{self, RichText, ScrollArea, TextEdit};
use std::{collections::BTreeMap, path::PathBuf};

impl Workbench<'_> {
    pub(crate) fn workspace_search_panel(&mut self, ui: &mut egui::Ui) {
        let mut start = false;
        let mut cancel = false;
        let mut preview = false;
        let mut open = None;
        let mut scoped_preview = None;
        {
            let state = &mut self.view.workspace_search;
            ui.horizontal(|ui| {
                let response = ui.add(
                    TextEdit::singleline(&mut state.query)
                        .id(egui::Id::new("workspace_search_query"))
                        .hint_text("Search")
                        .desired_width((ui.available_width() - 50.0).max(80.0)),
                );
                if state.focus_query {
                    response.request_focus();
                    state.focus_query = false;
                }
                if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    start = true;
                }
                if ui
                    .small_button(icon::MAGNIFYING_GLASS)
                    .on_hover_text("Search workspace")
                    .clicked()
                {
                    start = true;
                }
            });
            ui.horizontal(|ui| {
                ui.toggle_value(&mut state.match_case, "Aa")
                    .on_hover_text("Match Case");
                ui.toggle_value(&mut state.whole_word, "ab")
                    .on_hover_text("Match Whole Word");
                if ui
                    .small_button(if state.replace_visible {
                        icon::CARET_DOWN
                    } else {
                        icon::CARET_RIGHT
                    })
                    .on_hover_text("Toggle Replace")
                    .clicked()
                {
                    state.replace_visible = !state.replace_visible;
                }
                if state.running
                    && ui
                        .small_button(icon::X)
                        .on_hover_text("Cancel Search")
                        .clicked()
                {
                    cancel = true;
                }
                if ui
                    .small_button(icon::ELLIPSIS)
                    .on_hover_text("Toggle Search Details")
                    .clicked()
                {
                    state.details_visible = !state.details_visible;
                }
            });
            if state.replace_visible {
                ui.add(
                    TextEdit::singleline(&mut state.replacement)
                        .hint_text("Replace")
                        .desired_width((ui.available_width() - 12.0).max(80.0)),
                );
            }
            if state.details_visible {
                ui.add(
                    TextEdit::singleline(&mut state.include)
                        .hint_text("Files to include")
                        .desired_width((ui.available_width() - 12.0).max(80.0)),
                );
                ui.add(
                    TextEdit::singleline(&mut state.exclude)
                        .hint_text("Files to exclude")
                        .desired_width((ui.available_width() - 12.0).max(80.0)),
                );
            }
            if state.running {
                ui.label(RichText::new("Searching…").color(MUTED));
            }
            if state.replaced_files > 0 {
                ui.label(format!("Replaced in {} file(s)", state.replaced_files));
            }
            for error in state.report.iter().take(5) {
                ui.label(RichText::new(error).color(MUTED).size(11.0));
            }
            if state.report.len() > 5 {
                ui.label(format!(
                    "{} more replacement errors",
                    state.report.len() - 5
                ));
            }
            if let Some(results) = &state.results {
                let count: usize = results.files.iter().map(|file| file.hits.len()).sum();
                ui.label(
                    RichText::new(format!("{count} results in {} files", results.files.len()))
                        .color(MUTED)
                        .size(11.0),
                )
                .on_hover_text(format!(
                    "{} files scanned · {} skipped",
                    results.scanned, results.skipped
                ));
                if results.limited {
                    ui.label("Result limit reached; narrow the search.");
                }
                if results.cancelled {
                    ui.label("Search cancelled; partial results shown.");
                }
                if state.replace_visible
                    && count > 0
                    && ui
                        .small_button(format!("{}  Preview Replace All…", icon::REPLACE_ALL))
                        .clicked()
                {
                    preview = true;
                }
                if !results.errors.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "{} search error(s); first: {}",
                            results.errors.len(),
                            results.errors[0]
                        ))
                        .color(MUTED)
                        .size(11.0),
                    );
                }
                ScrollArea::vertical()
                    .id_salt("workspace_search_results")
                    .show(ui, |ui| {
                        let root = self.editor.workspace.as_deref();
                        for (folder, group) in grouped_files(&results.files, root) {
                            if root.is_some_and(|root| {
                                folder
                                    .ancestors()
                                    .skip(1)
                                    .take_while(|ancestor| ancestor.starts_with(root))
                                    .any(|ancestor| !state.expanded_folders.contains(ancestor))
                            }) {
                                continue;
                            }
                            let folder_open = state.expanded_folders.contains(&folder);
                            let folder_label =
                                folder.file_name().unwrap_or_default().to_string_lossy();
                            let depth = root
                                .and_then(|root| folder.strip_prefix(root).ok())
                                .map_or(0, |relative| relative.components().count());
                            let indent = depth as f32 * 12.0;
                            let hits = group.hits;
                            let (toggle, replace) = result_row(
                                ui,
                                format!(
                                    "{}  {}  {} ({hits})",
                                    if folder_open {
                                        icon::CARET_DOWN
                                    } else {
                                        icon::CARET_RIGHT
                                    },
                                    icon::FOLDER,
                                    folder_label
                                ),
                                indent,
                                state.replace_visible,
                                "Preview replace in folder",
                            );
                            if toggle {
                                if folder_open {
                                    state.expanded_folders.remove(&folder);
                                } else {
                                    state.expanded_folders.insert(folder.clone());
                                }
                            }
                            if replace {
                                scoped_preview = Some(ReplaceScope::Folder(folder.clone()));
                            }
                            if !state.expanded_folders.contains(&folder) {
                                continue;
                            }
                            for file in group.files {
                                let expanded = state.expanded_files.contains(&file.path);
                                let name =
                                    file.path.file_name().unwrap_or_default().to_string_lossy();
                                let (toggle, replace) = result_row(
                                    ui,
                                    format!(
                                        "{}  {}  {} ({})",
                                        if expanded {
                                            icon::CARET_DOWN
                                        } else {
                                            icon::CARET_RIGHT
                                        },
                                        icon::FILE,
                                        name,
                                        file.hits.len()
                                    ),
                                    indent + 12.0,
                                    state.replace_visible,
                                    "Preview replace in file",
                                );
                                if toggle {
                                    if expanded {
                                        state.expanded_files.remove(&file.path);
                                    } else {
                                        state.expanded_files.insert(file.path.clone());
                                    }
                                }
                                if replace {
                                    scoped_preview = Some(ReplaceScope::File(file.path.clone()));
                                }
                                if !state.expanded_files.contains(&file.path) {
                                    continue;
                                }
                                for hit in file.hits.iter().take(100) {
                                    let (clicked, _) = result_row(
                                        ui,
                                        format!("{}:{}  {}", hit.line, hit.column, hit.snippet),
                                        indent + 27.0,
                                        false,
                                        "",
                                    );
                                    if clicked {
                                        open = Some(Command::OpenWorkspaceHit(
                                            file.path.clone(),
                                            hit.range.clone(),
                                        ));
                                    }
                                }
                                if file.hits.len() > 100 {
                                    ui.label(
                                        RichText::new(format!(
                                            "{} more matches",
                                            file.hits.len() - 100
                                        ))
                                        .color(MUTED)
                                        .size(11.0),
                                    );
                                }
                            }
                        }
                    });
            } else if self.editor.workspace.is_none() {
                ui.label(RichText::new("Open a folder to search its files.").color(MUTED));
            }
        }
        if start {
            self.command(Command::StartWorkspaceSearch);
        }
        if cancel {
            self.command(Command::CancelWorkspaceSearch);
        }
        if preview {
            self.command(Command::PreviewWorkspaceReplace);
        }
        if let Some(scope) = scoped_preview {
            self.command(Command::PreviewWorkspaceReplaceScope(scope));
        }
        if let Some(command) = open {
            self.command(command);
        }
    }

    pub(crate) fn workspace_replace_preview(&mut self, ctx: &egui::Context) {
        if !self.view.workspace_search.preview_open && !self.view.workspace_search.confirmation_open
        {
            return;
        }
        let mut apply = false;
        let mut confirm = false;
        let mut close = false;
        let state = &mut self.view.workspace_search;
        if state.preview_open {
            egui::Window::new("Preview Workspace Replace").resizable(true).default_width(700.0).show(ctx, |ui| {
            ui.label("Only selected files will be changed. Open tabs are skipped; changed files are rejected.");
            if let Some(results) = &state.results {
                let pages = state.selected.len().div_ceil(10).max(1);
                state.preview_page = state.preview_page.min(pages - 1);
                ui.horizontal(|ui| {
                    if ui.add_enabled(state.preview_page > 0, egui::Button::new("Previous")).clicked() { state.preview_page -= 1; }
                    ui.label(format!("Page {} of {}", state.preview_page + 1, pages));
                    if ui.add_enabled(state.preview_page + 1 < pages, egui::Button::new("Next")).clicked() { state.preview_page += 1; }
                });
                ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for file in results.files.iter().filter(|file| state.selected.contains(&file.path)).skip(state.preview_page * 10).take(10) {
                        ui.label(RichText::new(file.path.display().to_string()).strong());
                        for hit in file.hits.iter().take(10) {
                            let (before, after) = preview_hit(&file.original, hit, &state.replacement);
                            ui.label(format!("{}:{}", hit.line, hit.column));
                            ui.horizontal_top(|ui| {
                                ui.vertical(|ui| { ui.label("Before"); ui.label(RichText::new(before).monospace()); });
                                ui.vertical(|ui| { ui.label("After"); ui.label(RichText::new(after).monospace()); });
                            });
                        }
                        if file.hits.len() > 10 { ui.label(format!("{} more matches in this file", file.hits.len() - 10)); }
                        ui.separator();
                    }
                });
                ui.label(RichText::new("Preview shows the first 10 matches per file; the full file is checked before writing.").color(MUTED).size(11.0));
            }
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() { close = true; }
                if ui.add_enabled(!state.selected.is_empty(), egui::Button::new("Replace Selected…")).clicked() { apply = true; }
            });
        });
        }
        if close {
            state.preview_open = false;
        }
        if apply {
            state.preview_open = false;
            state.confirmation_open = true;
        }
        if state.confirmation_open {
            egui::Window::new("Confirm Workspace Replace")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Replace matches in up to {} selected file(s)?",
                        state.selected.len()
                    ));
                    ui.label("Files changed since search and open editor tabs will be skipped.");
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            state.confirmation_open = false;
                        }
                        if ui.button("Replace Files").clicked() {
                            confirm = true;
                        }
                    });
                });
        }
        if confirm {
            self.command(Command::ApplyWorkspaceReplace);
        }
    }
}

#[derive(Default)]
struct SearchGroup<'a> {
    files: Vec<&'a SearchFile>,
    hits: usize,
}

fn grouped_files<'a>(
    files: &'a [SearchFile],
    root: Option<&std::path::Path>,
) -> BTreeMap<PathBuf, SearchGroup<'a>> {
    let mut groups = BTreeMap::new();
    for file in files {
        if let Some(folder) = file.path.parent() {
            for ancestor in folder.ancestors() {
                if root.is_some_and(|root| !ancestor.starts_with(root)) {
                    break;
                }
                let group: &mut SearchGroup<'a> = groups.entry(ancestor.to_path_buf()).or_default();
                group.hits += file.hits.len();
                if ancestor == folder {
                    group.files.push(file);
                }
                if root.is_none() || root == Some(ancestor) {
                    break;
                }
            }
        }
    }
    groups
}

fn result_row(
    ui: &mut egui::Ui,
    text: String,
    indent: f32,
    replace: bool,
    tooltip: &str,
) -> (bool, bool) {
    let mut clicked = false;
    let mut replace_clicked = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add_space(indent);
        let width = (ui.available_width() - if replace { 30.0 } else { 2.0 }).max(24.0);
        let response = ui
            .allocate_ui_with_layout(
                egui::vec2(width, 21.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(width);
                    ui.add(
                        egui::Label::new(RichText::new(&text).size(12.0))
                            .truncate()
                            .sense(egui::Sense::click()),
                    )
                },
            )
            .inner
            .on_hover_text(&text);
        clicked = response.clicked()
            || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
        if replace {
            replace_clicked = ui
                .add_sized(
                    [22.0, 21.0],
                    egui::Button::new(icon::REPLACE_ALL).frame(false),
                )
                .on_hover_text(tooltip)
                .clicked();
        }
    });
    (clicked, replace_clicked)
}

fn preview_hit(text: &str, hit: &SearchHit, replacement: &str) -> (String, String) {
    let start = text[..hit.range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let end = text[hit.range.end..]
        .find('\n')
        .map_or(text.len(), |offset| hit.range.end + offset);
    let before: String = text[start..end]
        .trim_end_matches('\r')
        .chars()
        .take(180)
        .collect();
    let after = format!(
        "{}{}{}",
        &text[start..hit.range.start],
        replacement,
        &text[hit.range.end..end]
    );
    (
        before,
        after.trim_end_matches('\r').chars().take(180).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{LumenApp, view_state::Activity},
        services::workspace_search::{SearchFile, SearchResult},
    };
    use std::{cell::Cell, path::PathBuf};

    fn sidebar_width(app: &mut LumenApp, ctx: &egui::Context, pointer: egui::Pos2) -> f32 {
        let width = Cell::new(0.0);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                events: vec![egui::Event::PointerMoved(pointer)],
                ..Default::default()
            },
            |ctx| {
                app.render_component(ctx, |workbench| workbench.sidebar(ctx));
                width.set(ctx.available_rect().min.x);
            },
        );
        width.get()
    }

    #[test]
    fn search_content_and_hover_do_not_grow_sidebar() {
        let mut app = LumenApp::default();
        app.view.activity = Activity::Search;
        app.editor.workspace = Some(PathBuf::from("/workspace"));
        app.view.workspace_search.query = "needle".into();
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let empty_widths = [
            egui::pos2(850.0, 400.0),
            egui::pos2(120.0, 120.0),
            egui::pos2(120.0, 180.0),
        ]
        .map(|point| sidebar_width(&mut app, &ctx, point));
        assert!(
            empty_widths
                .iter()
                .all(|width| (*width - empty_widths[0]).abs() < 1.0),
            "empty sidebar drifted: {empty_widths:?}"
        );
        app.view.workspace_search.results = Some(SearchResult {
            files: vec![SearchFile {
                path: PathBuf::from(
                    "/workspace/a/very-long-filename-that-should-truncate-in-a-narrow-sidebar.rs",
                ),
                original: "needle".into(),
                hits: vec![SearchHit {
                    range: 0..6,
                    line: 1,
                    column: 1,
                    snippet: "needle".into(),
                }],
            }],
            ..Default::default()
        });
        app.view
            .workspace_search
            .expanded_folders
            .insert(PathBuf::from("/workspace/a"));
        app.view
            .workspace_search
            .expanded_files
            .insert(PathBuf::from(
                "/workspace/a/very-long-filename-that-should-truncate-in-a-narrow-sidebar.rs",
            ));
        app.view.workspace_search.replace_visible = true;
        let widths = [
            egui::pos2(850.0, 400.0),
            egui::pos2(120.0, 120.0),
            egui::pos2(120.0, 180.0),
        ]
        .map(|point| sidebar_width(&mut app, &ctx, point));
        assert!(
            widths.iter().all(|width| (*width - widths[0]).abs() < 1.0),
            "sidebar drifted: {widths:?}"
        );
        assert!(
            widths[0] < 300.0,
            "search content widened default sidebar: {widths:?}"
        );
        for requested in [180.0, 240.0, 600.0] {
            let ctx = egui::Context::default();
            crate::ui::theme::configure_ui(&ctx);
            ctx.data_mut(|data| {
                data.insert_persisted(
                    egui::Id::new("explorer"),
                    egui::containers::panel::PanelState {
                        rect: egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(requested, 600.0),
                        ),
                    },
                )
            });
            let widths = [
                egui::pos2(850.0, 400.0),
                egui::pos2(120.0, 120.0),
                egui::pos2(120.0, 180.0),
            ]
            .map(|point| sidebar_width(&mut app, &ctx, point));
            assert!(
                widths.iter().all(|width| (*width - requested).abs() < 2.0),
                "sidebar width {requested} drifted: {widths:?}"
            );
        }
    }

    #[test]
    fn result_tree_has_ancestor_folders_without_duplicate_file_rows() {
        let root = PathBuf::from("/workspace");
        let files = [SearchFile {
            path: root.join("src/deep/main.rs"),
            original: "needle".into(),
            hits: vec![],
        }];
        let groups = grouped_files(&files, Some(&root));
        assert_eq!(
            groups.keys().cloned().collect::<Vec<_>>(),
            vec![root.clone(), root.join("src"), root.join("src/deep")]
        );
        assert!(groups[&root].files.is_empty());
        assert!(groups[&root.join("src")].files.is_empty());
        assert_eq!(groups[&root.join("src/deep")].files.len(), 1);
    }
}
