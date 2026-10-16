use super::Workbench;
use crate::{
    app::{command::Command, view_state::WorkspaceSearchView, workspace_search::ReplaceScope},
    icon,
    services::workspace_search::{SearchFile, SearchHit, SearchResult},
};
use eframe::egui::{self, RichText, ScrollArea, TextEdit};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

const PREVIEW_HUNKS_PER_PAGE: usize = 12;
const MAX_MATCHES_PER_HUNK: usize = 8;

impl Workbench<'_> {
    pub(crate) fn workspace_search_panel(&mut self, ui: &mut egui::Ui) {
        let colors = crate::ui::theme::palette(ui.visuals());
        let mut start = false;
        let mut cancel = false;
        let mut clear = false;
        let mut preview = false;
        let mut open = None;
        let mut scoped_preview = None;
        {
            let state = &mut self.view.workspace_search;
            ui.horizontal(|ui| {
                let can_clear = !state.query.is_empty() || state.results.is_some() || state.running;
                let response = ui.add(
                    TextEdit::singleline(&mut state.query)
                        .id(egui::Id::new("workspace_search_query"))
                        .hint_text("Search")
                        .desired_width(
                            (ui.available_width() - if can_clear { 76.0 } else { 50.0 }).max(80.0),
                        ),
                );
                if state.focus_query {
                    response.request_focus();
                    state.focus_query = false;
                }
                if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    start = true;
                }
                if can_clear
                    && ui
                        .small_button(icon::X)
                        .on_hover_text("Clear search")
                        .clicked()
                {
                    clear = true;
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
                )
                .on_hover_text("Adds to the effective Search: Exclude Patterns setting.");
            }
            if state.running {
                let progress = state.progress.unwrap_or_default();
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "Searching… {} files · {} matches",
                            progress.scanned, progress.matches
                        ))
                        .color(colors.muted)
                        .size(11.0),
                    )
                    .truncate(),
                )
                .on_hover_text(format!("{} skipped", progress.skipped));
            }
            if state.replaced_files > 0 {
                ui.label(format!("Replaced in {} file(s)", state.replaced_files));
            }
            for error in state.report.iter().take(5) {
                ui.label(RichText::new(error).color(colors.muted).size(11.0));
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
                        .color(colors.muted)
                        .size(11.0),
                )
                .on_hover_text(format!(
                    "{} files scanned · {} entries skipped\n{} symlinks/internal directories · {} ignored/excluded entries · {} binary, non-UTF-8 or over 2 MiB\nEntries inside skipped directories are not counted.",
                    results.scanned,
                    results.skipped,
                    results.skipped_links_or_internal,
                    results.skipped_ignored_or_excluded,
                    results.skipped_non_text_or_large,
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
                        .color(colors.muted)
                        .size(11.0),
                    );
                }
                let rows = visible_rows(
                    &results.files,
                    self.editor.workspace.as_deref(),
                    &state.expanded_folders,
                    &state.expanded_files,
                );
                ui.add_space(4.0);
                ScrollArea::vertical()
                    .id_salt("workspace_search_results")
                    .max_height(ui.available_height())
                    .show_rows(ui, 21.0, rows.len(), |ui, range| {
                        for row in &rows[range] {
                            match row {
                                SearchRow::Folder { path, hits, depth } => {
                                    let expanded = state.expanded_folders.contains(path);
                                    let name =
                                        path.file_name().unwrap_or_default().to_string_lossy();
                                    let (toggle, replace) = result_row(
                                        ui,
                                        format!(
                                            "{}  {}  {} ({hits})",
                                            caret(expanded),
                                            icon::FOLDER,
                                            name
                                        ),
                                        *depth as f32 * 12.0,
                                        state.replace_visible,
                                        "Preview replace in folder",
                                    );
                                    if toggle {
                                        toggle_path(&mut state.expanded_folders, path);
                                    }
                                    if replace {
                                        scoped_preview = Some(ReplaceScope::Folder(path.clone()));
                                    }
                                }
                                SearchRow::File { file, depth } => {
                                    let expanded = state.expanded_files.contains(&file.path);
                                    let name =
                                        file.path.file_name().unwrap_or_default().to_string_lossy();
                                    let (toggle, replace) = result_row(
                                        ui,
                                        format!(
                                            "{}  {}  {} ({})",
                                            caret(expanded),
                                            icon::FILE,
                                            name,
                                            file.hits.len()
                                        ),
                                        *depth as f32 * 12.0 + 12.0,
                                        state.replace_visible,
                                        "Preview replace in file",
                                    );
                                    if toggle {
                                        toggle_path(&mut state.expanded_files, &file.path);
                                    }
                                    if replace {
                                        scoped_preview =
                                            Some(ReplaceScope::File(file.path.clone()));
                                    }
                                }
                                SearchRow::Hit { file, hit, depth } => {
                                    let (clicked, _) = result_row(
                                        ui,
                                        format!("{}:{}  {}", hit.line, hit.column, hit.snippet),
                                        *depth as f32 * 12.0 + 27.0,
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
                            }
                        }
                    });
            } else if self.editor.workspace.is_none() {
                ui.label(RichText::new("Open a folder to search its files.").color(colors.muted));
            }
        }
        if clear {
            self.command(Command::ClearWorkspaceSearch);
        } else if start {
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
        if state.preview_open
            && let Some(response) = egui::Window::new("Preview Workspace Replace")
                .resizable(true)
                .default_width(700.0)
                .max_width((ctx.content_rect().width() - 16.0).max(320.0))
                .show(ctx, |ui| show_workspace_replace_preview(ui, state))
            && let Some((was_closed, was_applied)) = response.inner
        {
            close = was_closed;
            apply = was_applied;
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

fn show_workspace_replace_preview(
    ui: &mut egui::Ui,
    state: &mut WorkspaceSearchView,
) -> (bool, bool) {
    let colors = crate::ui::theme::palette(ui.visuals());
    ui.add(
        egui::Label::new(
            "Only selected files will be changed. Open tabs are skipped; changed files are rejected.",
        )
        .wrap(),
    );
    if let Some(results) = &state.results {
        let hunks = selected_preview_hunks(results, &state.selected);
        let pages = hunks.len().div_ceil(PREVIEW_HUNKS_PER_PAGE).max(1);
        state.preview_page = state.preview_page.min(pages - 1);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(state.preview_page > 0, egui::Button::new("Previous"))
                .clicked()
            {
                state.preview_page -= 1;
            }
            ui.label(format!("Page {} of {}", state.preview_page + 1, pages));
            if ui
                .add_enabled(state.preview_page + 1 < pages, egui::Button::new("Next"))
                .clicked()
            {
                state.preview_page += 1;
            }
        });
        ui.label(
            RichText::new(format!(
                "{} matches in {} selected files · {} change groups",
                selected_preview_matches(results, &state.selected).len(),
                state.selected.len(),
                hunks.len()
            ))
            .color(colors.muted)
            .size(11.0),
        );
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("− Removed")
                    .color(colors.diff_removed_text)
                    .size(11.0),
            );
            ui.label(
                RichText::new("+ Replacement")
                    .color(colors.diff_added_text)
                    .size(11.0),
            );
        });
        show_preview_hunk_page(ui, &hunks, state.preview_page, &state.replacement);
        ui.add(
            egui::Label::new(
                RichText::new("Unchanged lines are shown for context. Long groups use per-match excerpts; full files are checked before writing.")
                    .color(colors.muted)
                    .size(11.0),
            )
            .wrap(),
        );
    }
    let mut close = false;
    let mut apply = false;
    ui.horizontal(|ui| {
        close = ui.button("Cancel").clicked();
        apply = ui
            .add_enabled(
                !state.selected.is_empty(),
                egui::Button::new("Replace Selected…"),
            )
            .clicked();
    });
    (close, apply)
}

fn show_preview_hunk_page(
    ui: &mut egui::Ui,
    hunks: &[PreviewHunk<'_>],
    page: usize,
    replacement: &str,
) {
    let colors = crate::ui::theme::palette(ui.visuals());
    ScrollArea::vertical()
        .id_salt(("workspace_replace_preview", page))
        .max_height(320.0)
        .show(ui, |ui| {
            let mut previous_file = None;
            for hunk in hunks
                .iter()
                .skip(page * PREVIEW_HUNKS_PER_PAGE)
                .take(PREVIEW_HUNKS_PER_PAGE)
            {
                let file = hunk.file;
                if previous_file != Some(file.path.as_path()) {
                    if previous_file.is_some() {
                        ui.separator();
                    }
                    let path = file.path.display().to_string();
                    ui.add(egui::Label::new(RichText::new(&path).strong()).truncate())
                        .on_hover_text(path);
                    previous_file = Some(file.path.as_path());
                }
                ui.label(
                    RichText::new(format!(
                        "Line {} · {} match(es)",
                        hunk.line,
                        hunk.hits.len()
                    ))
                    .color(colors.muted)
                    .size(11.0),
                );
                super::workspace_search_diff::show_hunk(
                    ui,
                    file,
                    hunk.hits,
                    hunk.start..hunk.end,
                    replacement,
                );
            }
        });
}

#[derive(Default)]
struct SearchGroup<'a> {
    files: Vec<&'a SearchFile>,
    hits: usize,
}

enum SearchRow<'a> {
    Folder {
        path: PathBuf,
        hits: usize,
        depth: usize,
    },
    File {
        file: &'a SearchFile,
        depth: usize,
    },
    Hit {
        file: &'a SearchFile,
        hit: &'a SearchHit,
        depth: usize,
    },
}

fn visible_rows<'a>(
    files: &'a [SearchFile],
    root: Option<&Path>,
    expanded_folders: &HashSet<PathBuf>,
    expanded_files: &HashSet<PathBuf>,
) -> Vec<SearchRow<'a>> {
    let mut rows = Vec::new();
    for (folder, group) in grouped_files(files, root) {
        if root.is_some_and(|root| {
            folder
                .ancestors()
                .skip(1)
                .take_while(|ancestor| ancestor.starts_with(root))
                .any(|ancestor| !expanded_folders.contains(ancestor))
        }) {
            continue;
        }
        let depth = root
            .and_then(|root| folder.strip_prefix(root).ok())
            .map_or(0, |relative| relative.components().count());
        let folder_open = expanded_folders.contains(&folder);
        rows.push(SearchRow::Folder {
            path: folder,
            hits: group.hits,
            depth,
        });
        if !folder_open {
            continue;
        }
        for file in group.files {
            rows.push(SearchRow::File { file, depth });
            if expanded_files.contains(&file.path) {
                rows.extend(
                    file.hits
                        .iter()
                        .map(|hit| SearchRow::Hit { file, hit, depth }),
                );
            }
        }
    }
    rows
}

fn caret(expanded: bool) -> &'static str {
    if expanded {
        icon::CARET_DOWN
    } else {
        icon::CARET_RIGHT
    }
}

fn toggle_path(paths: &mut HashSet<PathBuf>, path: &Path) {
    if !paths.remove(path) {
        paths.insert(path.to_path_buf());
    }
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
        let row = ui.allocate_ui_with_layout(
            egui::vec2(width, 21.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_width(width);
                ui.add(
                    egui::Label::new(RichText::new(&text).size(12.0))
                        .truncate()
                        .selectable(false),
                );
            },
        );
        let response = ui
            .interact(row.response.rect, row.response.id, egui::Sense::click())
            .on_hover_text(&text)
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        clicked = response.clicked()
            || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
        if replace {
            replace_clicked = ui
                .add_sized(
                    [22.0, 21.0],
                    egui::Button::new(icon::REPLACE_ALL).frame(false),
                )
                .on_hover_text(tooltip)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked();
        }
    });
    (clicked, replace_clicked)
}

fn selected_preview_matches<'a>(
    results: &'a SearchResult,
    selected: &HashSet<PathBuf>,
) -> Vec<(&'a SearchFile, &'a SearchHit)> {
    results
        .files
        .iter()
        .filter(|file| selected.contains(&file.path))
        .flat_map(|file| file.hits.iter().map(move |hit| (file, hit)))
        .collect()
}

struct PreviewHunk<'a> {
    file: &'a SearchFile,
    hits: &'a [SearchHit],
    start: usize,
    end: usize,
    line: usize,
}

fn selected_preview_hunks<'a>(
    results: &'a SearchResult,
    selected: &HashSet<PathBuf>,
) -> Vec<PreviewHunk<'a>> {
    let mut hunks = Vec::new();
    for file in results
        .files
        .iter()
        .filter(|file| selected.contains(&file.path))
    {
        let text = &file.original;
        let line_starts: Vec<_> = std::iter::once(0)
            .chain(
                text.bytes()
                    .enumerate()
                    .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
            )
            .collect();
        let mut active: Option<(usize, usize, usize, usize)> = None;
        for (index, hit) in file.hits.iter().enumerate() {
            if text.get(hit.range.clone()).is_none() {
                continue;
            }
            let line = line_starts
                .partition_point(|start| *start <= hit.range.start)
                .saturating_sub(1);
            let start = line_starts[line.saturating_sub(1)];
            let end = line_starts.get(line + 2).copied().unwrap_or(text.len());
            if let Some((first, last, _, active_end)) = &mut active
                && start <= *active_end
                && *last - *first < MAX_MATCHES_PER_HUNK
            {
                *last = index + 1;
                *active_end = (*active_end).max(end);
            } else {
                if let Some((first, last, start, end)) = active.take() {
                    hunks.push(PreviewHunk {
                        file,
                        hits: &file.hits[first..last],
                        start,
                        end,
                        line: file.hits[first].line,
                    });
                }
                active = Some((index, index + 1, start, end));
            }
        }
        if let Some((first, last, start, end)) = active {
            hunks.push(PreviewHunk {
                file,
                hits: &file.hits[first..last],
                start,
                end,
                line: file.hits[first].line,
            });
        }
    }
    hunks
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
    fn search_result_hover_uses_clickable_cursor() {
        for width in [180.0, 300.0, 600.0] {
            let ctx = egui::Context::default();
            crate::ui::theme::configure_ui(&ctx);
            let draw = |events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 100.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            result_row(ui, "1:1  needle".into(), 0.0, false, "");
                        });
                    },
                )
                .platform_output
                .cursor_icon
            };
            draw(vec![]);
            let cursor = draw(vec![egui::Event::PointerMoved(egui::pos2(35.0, 19.0))]);
            assert_eq!(cursor, egui::CursorIcon::PointingHand, "width {width}");
        }
    }

    #[test]
    fn search_hit_whitespace_is_clickable() {
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let draw = |events| {
            let clicked = Cell::new(false);
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(300.0, 100.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        clicked.set(result_row(ui, "1:1  needle".into(), 0.0, false, "").0);
                    });
                },
            );
            clicked.get()
        };
        draw(vec![]);
        let pos = egui::pos2(220.0, 19.0);
        draw(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(draw(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]));
    }

    #[test]
    fn search_result_text_stays_left_aligned_at_panel_widths() {
        for width in [180.0, 300.0, 600.0] {
            let ctx = egui::Context::default();
            crate::ui::theme::configure_ui(&ctx);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 100.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        result_row(ui, "1:1  needle".into(), 0.0, false, "");
                    });
                },
            );
            let text_x = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains("needle") => {
                    Some(text.pos.x)
                }
                _ => None,
            });
            assert!(text_x.unwrap() < 20.0, "width {width}: {text_x:?}");
        }
    }

    #[test]
    fn replace_preview_pages_cover_every_selected_match() {
        let first = PathBuf::from("/workspace/first.rs");
        let second = PathBuf::from("/workspace/second.rs");
        let file = |path: PathBuf, count| SearchFile {
            path,
            original: "needle\n\n\n\n".repeat(count),
            hits: (0..count)
                .map(|index| SearchHit {
                    range: index * 10..index * 10 + 6,
                    line: index * 4 + 1,
                    column: 1,
                    snippet: "needle".into(),
                })
                .collect(),
        };
        let results = SearchResult {
            files: vec![file(first.clone(), 23), file(second.clone(), 2)],
            ..Default::default()
        };
        let hunks = selected_preview_hunks(&results, &HashSet::from([first]));
        assert_eq!(hunks.len(), 23);
        assert_eq!(hunks.chunks(PREVIEW_HUNKS_PER_PAGE).count(), 2);
        assert_eq!(
            hunks.chunks(PREVIEW_HUNKS_PER_PAGE).nth(1).unwrap().len(),
            11
        );
        assert_eq!(hunks.iter().map(|hunk| hunk.hits.len()).sum::<usize>(), 23);

        let hunks = selected_preview_hunks(&results, &HashSet::from([second]));
        assert_eq!(hunks.len(), 2);
        assert!(
            hunks
                .iter()
                .all(|hunk| hunk.file.path == results.files[1].path)
        );
    }

    #[test]
    fn nearby_matches_share_a_hunk_with_combined_replacement_and_context() {
        let path = PathBuf::from("/workspace/a.txt");
        let original = "before\r\nα needle and needle\r\nafter\r\n";
        let first = original.find("needle").unwrap();
        let second = original.rfind("needle").unwrap();
        let results = SearchResult {
            files: vec![SearchFile {
                path: path.clone(),
                original: original.into(),
                hits: vec![
                    SearchHit {
                        range: first..first + 6,
                        line: 2,
                        column: 3,
                        snippet: String::new(),
                    },
                    SearchHit {
                        range: second..second + 6,
                        line: 2,
                        column: 14,
                        snippet: String::new(),
                    },
                ],
            }],
            ..Default::default()
        };
        let hunks = selected_preview_hunks(&results, &HashSet::from([path]));
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].hits.len(), 2);
    }

    #[test]
    fn dense_long_line_preview_is_bounded_and_covers_every_match() {
        let path = PathBuf::from("/workspace/dense.txt");
        let results = SearchResult {
            files: vec![SearchFile {
                path: path.clone(),
                original: "x".repeat(6_000),
                hits: (0..100)
                    .map(|index| SearchHit {
                        range: index * 2..index * 2 + 1,
                        line: 1,
                        column: index * 2 + 1,
                        snippet: String::new(),
                    })
                    .collect(),
            }],
            ..Default::default()
        };
        let hunks = selected_preview_hunks(&results, &HashSet::from([path]));
        assert_eq!(hunks.len(), 13);
        assert!(
            hunks
                .iter()
                .all(|hunk| hunk.hits.len() <= MAX_MATCHES_PER_HUNK)
        );
        assert_eq!(hunks.iter().map(|hunk| hunk.hits.len()).sum::<usize>(), 100);
        assert!(hunks.iter().all(|hunk| hunk.end - hunk.start > 4_000));
    }

    #[test]
    fn replace_preview_stays_within_narrow_and_wide_viewports() {
        for width in [520.0, 900.0] {
            let mut app = LumenApp::default();
            let path = PathBuf::from("/workspace/long-name.rs");
            app.view.workspace_search.results = Some(SearchResult {
                files: vec![SearchFile {
                    path: path.clone(),
                    original: "needle\n\n\n\n".repeat(30),
                    hits: (0..30)
                        .map(|index| SearchHit {
                            range: index * 10..index * 10 + 6,
                            line: index * 4 + 1,
                            column: 1,
                            snippet: "needle".into(),
                        })
                        .collect(),
                }],
                ..Default::default()
            });
            app.view.workspace_search.selected.insert(path);
            app.view.workspace_search.preview_open = true;
            app.view.workspace_search.preview_page = 1;
            let ctx = egui::Context::default();
            crate::ui::theme::configure_ui(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.0));
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ctx| {
                    app.render_component(ctx, |workbench| workbench.workspace_replace_preview(ctx))
                },
            );
            let rect = ctx
                .memory(|memory| memory.area_rect(egui::Id::new("Preview Workspace Replace")))
                .expect("preview window should be shown");
            assert!(
                !output.shapes.iter().any(|shape| match &shape.shape {
                    egui::Shape::Text(text) =>
                        text.galley.text().contains("First use of widget ID"),
                    _ => false,
                }),
                "preview widget IDs must be unique"
            );
            assert!(rect.width() <= screen.width(), "width {width}: {rect:?}");
            assert_eq!(app.view.workspace_search.preview_page, 1);
        }
    }

    #[test]
    fn dense_preview_hunks_use_distinct_scroll_ids() {
        let mut app = LumenApp::default();
        let path = PathBuf::from("/workspace/dense.txt");
        app.view.workspace_search.results = Some(SearchResult {
            files: vec![SearchFile {
                path: path.clone(),
                original: "x".repeat(6_000),
                hits: (0..100)
                    .map(|index| SearchHit {
                        range: index * 2..index * 2 + 1,
                        line: 1,
                        column: index * 2 + 1,
                        snippet: String::new(),
                    })
                    .collect(),
            }],
            ..Default::default()
        });
        app.view.workspace_search.selected.insert(path);
        app.view.workspace_search.preview_open = true;
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.workspace_replace_preview(ctx)),
        );
        assert!(
            !output.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::Text(text) => text.galley.text().contains("First use of widget ID"),
                _ => false,
            }),
            "dense preview widget IDs must be unique"
        );
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

    #[test]
    fn progress_text_and_cancel_action_keep_a_narrow_sidebar_stable() {
        let mut app = LumenApp::default();
        app.view.activity = Activity::Search;
        app.editor.workspace = Some(PathBuf::from("/workspace"));
        app.view.workspace_search.running = true;
        app.view.workspace_search.progress =
            Some(crate::services::workspace_search::SearchProgress {
                scanned: 100_000,
                skipped: 999,
                matches: 10_000,
            });
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let widths = [
            egui::pos2(850.0, 400.0),
            egui::pos2(120.0, 120.0),
            egui::pos2(120.0, 180.0),
        ]
        .map(|point| sidebar_width(&mut app, &ctx, point));
        assert!(
            widths.iter().all(|width| (*width - widths[0]).abs() < 1.0),
            "progress widened sidebar: {widths:?}"
        );
    }

    #[test]
    fn virtualized_tree_exposes_every_hit_and_respects_collapsed_ancestors() {
        let root = PathBuf::from("/workspace");
        let file_path = root.join("src/deep/main.rs");
        let files = [SearchFile {
            path: file_path.clone(),
            original: "x".into(),
            hits: (0..150)
                .map(|line| SearchHit {
                    range: 0..1,
                    line,
                    column: 1,
                    snippet: "x".into(),
                })
                .collect(),
        }];
        let folders = HashSet::from([root.clone(), root.join("src"), root.join("src/deep")]);
        let expanded_files = HashSet::from([file_path]);
        let rows = visible_rows(&files, Some(&root), &folders, &expanded_files);
        assert_eq!(rows.len(), 3 + 1 + 150);
        assert!(matches!(rows.last(), Some(SearchRow::Hit { hit, .. }) if hit.line == 149));

        let collapsed = HashSet::from([root.clone()]);
        let rows = visible_rows(&files, Some(&root), &collapsed, &expanded_files);
        assert_eq!(rows.len(), 2);
        assert!(
            matches!(&rows[1], SearchRow::Folder { path, hits: 150, .. } if path == &root.join("src"))
        );
    }
}
