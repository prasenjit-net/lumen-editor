//! Compact, single-row editor tabs with clipped overflow and fixed toolbar actions.
use super::{Workbench, widgets};
use crate::{
    app::{
        command::Command,
        tab_commands::TabMove,
        task_editor::TaskEditorId,
        view_state::{TabDrag, TabStripGeometry},
    },
    icon,
    model::DocumentId,
};
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, ScrollArea, Stroke};
use std::path::Path;

const HEIGHT: f32 = 34.0;
const MIN_WIDTH: f32 = 96.0;
const ACTIVE_WIDTH: f32 = 160.0;
const MAX_WIDTH: f32 = 220.0;
const GAP: f32 = 2.0;
const ACTION_SIZE: f32 = 22.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TabTarget {
    Document(DocumentId),
    Diff,
    Settings,
    TaskEditor(TaskEditorId),
}

#[derive(Clone, PartialEq)]
struct RevealSignature {
    active: Option<TabTarget>,
    viewport: f32,
    widths: Vec<f32>,
}

struct TabSizing {
    widths: Vec<f32>,
    viewport: f32,
    overflow: bool,
}
impl TabSizing {
    fn new(available: f32, natural: &[f32], active: Option<usize>) -> Self {
        let gaps = natural.len().saturating_sub(1) as f32 * GAP;
        let minimum = |viewport: f32, index: usize| {
            if active == Some(index) {
                ACTIVE_WIDTH.min(viewport).max(MIN_WIDTH)
            } else {
                MIN_WIDTH
            }
        };
        let initial_viewport = available - (ACTION_SIZE + 4.0);
        let overflow = (0..natural.len())
            .map(|index| minimum(initial_viewport, index))
            .sum::<f32>()
            + gaps
            > initial_viewport;
        let actions = if overflow {
            ACTION_SIZE * 2.0 + 8.0
        } else {
            ACTION_SIZE + 4.0
        };
        let viewport = (available - actions).max(0.0);
        let budget = viewport - gaps;
        let mut low = MIN_WIDTH;
        let mut high = MAX_WIDTH;
        // Cap the longest labels first; short filenames do not grow needlessly.
        for _ in 0..20 {
            let cap = (low + high) / 2.0;
            if natural
                .iter()
                .enumerate()
                .map(|(index, width)| width.min(cap).max(minimum(viewport, index)))
                .sum::<f32>()
                <= budget
            {
                low = cap;
            } else {
                high = cap;
            }
        }
        Self {
            widths: natural
                .iter()
                .enumerate()
                .map(|(index, width)| {
                    width.clamp(minimum(viewport, index), low.max(minimum(viewport, index)))
                })
                .collect(),
            viewport,
            overflow,
        }
    }
}

impl Workbench<'_> {
    pub(crate) fn editor_tabs(&mut self, ui: &mut egui::Ui) -> egui::Rect {
        let colors = crate::ui::theme::palette(ui.visuals());
        // Only the first editor group hosts the Settings and Diff tabs.
        let hide_virtual = self.view.hide_virtual_tabs;
        let diff_tab = !hide_virtual && self.view.source_control.diff.is_some();
        let settings_open = !hide_virtual && self.view.settings.open;
        let settings_tab = !hide_virtual && self.view.settings.tab_open;
        let task_tabs: Vec<(TaskEditorId, String)> = if hide_virtual {
            Vec::new()
        } else {
            self.view
                .task_editors
                .tabs()
                .iter()
                .map(|tab| (tab.id, format!("{} {}", icon::GEAR, tab.title())))
                .collect()
        };
        let task_active = (!hide_virtual)
            .then(|| self.view.task_editors.active())
            .flatten();
        let hidden_document = self
            .view
            .source_control
            .diff
            .as_ref()
            .and_then(|diff| diff.diff_only_document);
        // Each editor group's strip lists only its own documents.
        let group_documents = self.editor.documents_in(self.editor.focused_group());
        let visible_files: Vec<_> = self
            .editor
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| {
                Some(file.id) != hidden_document && group_documents.contains(&file.id)
            })
            .map(|(index, _)| index)
            .collect();
        let titles: Vec<_> = visible_files
            .iter()
            .map(|&index| &self.editor.files[index])
            .map(|file| {
                let path = file.path.as_deref().unwrap_or(Path::new(&file.name));
                format!(
                    "{} {}{}",
                    widgets::file_icon(path),
                    file.name,
                    if file.disk_changed { " !" } else { "" }
                )
            })
            .chain(
                self.view
                    .source_control
                    .diff
                    .as_ref()
                    .filter(|_| !hide_virtual)
                    .map(|diff| {
                        let name = diff
                            .path
                            .file_name()
                            .unwrap_or(diff.path.as_os_str())
                            .to_string_lossy();
                        format!("{} {} (Diff)", icon::GIT_BRANCH, name)
                    }),
            )
            .chain(settings_tab.then(|| format!("{} Settings", icon::GEAR)))
            .chain(task_tabs.iter().map(|(_, title)| title.clone()))
            .collect();
        let font = egui::TextStyle::Button.resolve(ui.style());
        let natural: Vec<_> = titles
            .iter()
            .map(|title| {
                ui.fonts_mut(|fonts| {
                    fonts
                        .layout_no_wrap(title.clone(), font.clone(), colors.text)
                        .size()
                        .x
                        + 62.0
                })
                .clamp(MIN_WIDTH, MAX_WIDTH)
            })
            .collect();
        let diff_active = diff_tab
            && self
                .view
                .source_control
                .diff
                .as_ref()
                .is_some_and(|diff| diff.active);
        let task_base = visible_files.len() + usize::from(diff_tab) + usize::from(settings_tab);
        let active_index = if let Some(id) = task_active {
            task_tabs
                .iter()
                .position(|(tab, _)| *tab == id)
                .map(|position| task_base + position)
        } else if settings_open {
            settings_tab.then_some(visible_files.len() + usize::from(diff_tab))
        } else if diff_active {
            Some(visible_files.len())
        } else {
            self.editor
                .active_index()
                .and_then(|index| visible_files.iter().position(|visible| *visible == index))
        };
        let sizing = TabSizing::new(ui.available_width(), &natural, active_index);
        let active = if let Some(id) = task_active {
            Some(TabTarget::TaskEditor(id))
        } else if settings_open {
            Some(TabTarget::Settings)
        } else if diff_active {
            Some(TabTarget::Diff)
        } else {
            self.active().map(|file| TabTarget::Document(file.id))
        };
        let reveal_id = ui.id().with("tab_reveal");
        let previous = ui
            .ctx()
            .data(|data| data.get_temp::<RevealSignature>(reveal_id));
        let signature = RevealSignature {
            active,
            viewport: sizing.viewport,
            widths: sizing.widths.clone(),
        };
        let reveal = previous.as_ref() != Some(&signature)
            || (settings_open && self.view.settings.focus_search);
        ui.ctx()
            .data_mut(|data| data.insert_temp(reveal_id, signature));
        let split_enabled = self.view.settings.boolean("workbench.split_editor");
        let groups = self.editor.group_count();
        let saved = visible_files
            .iter()
            .filter(|index| !self.editor.files[**index].dirty)
            .count();
        let mut select = None;
        let mut close = None;
        let mut menu_command: Option<Command> = None;
        // A completed tab drag: the document and its new position among the tabs.
        let mut tab_drop: Option<(usize, usize)> = None;
        let mut strip_geometry: Option<TabStripGeometry> = None;
        let mut drag_state: Option<TabDrag> = None;
        let mut outside_drop: Option<(DocumentId, egui::Pos2)> = None;
        let mut open_settings = false;
        let mut open_task: Option<TaskEditorId> = None;
        let mut close_task: Option<TaskEditorId> = None;
        let mut close_settings = false;
        let mut open_diff = false;
        let mut close_diff = false;
        let strip = ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.allocate_ui_with_layout(
                egui::vec2(sizing.viewport, HEIGHT),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ScrollArea::horizontal()
                        .id_salt("editor_tabs")
                        .animated(false)
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .auto_shrink([false, true])
                        .max_height(HEIGHT)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = GAP;
                                let mut tab_rects: Vec<egui::Rect> = Vec::new();
                                let mut dragged_tab: Option<(usize, bool)> = None;
                                for (tab_index, &file_index) in visible_files.iter().enumerate() {
                                    let file = &self.editor.files[file_index];
                                    let selected = active == Some(TabTarget::Document(file.id));
                                    let response = tab(
                                        ui,
                                        &titles[tab_index],
                                        sizing.widths[tab_index],
                                        selected,
                                        &file.path.as_ref().map_or_else(
                                            || file.name.clone(),
                                            |path| path.display().to_string(),
                                        ),
                                        "Close editor",
                                        file.dirty,
                                    );
                                    response.title.context_menu(|ui| {
                                        tab_menu(
                                            ui,
                                            TabMenuTarget {
                                                id: file.id,
                                                path: file.path.as_deref(),
                                                position: tab_index,
                                                count: visible_files.len(),
                                                saved,
                                                split_enabled,
                                                groups,
                                            },
                                            self.editor.workspace.is_some(),
                                            &mut menu_command,
                                        );
                                    });
                                    if response.select {
                                        select = Some(file_index);
                                    }
                                    if response.close {
                                        close = Some(file.id);
                                    }
                                    if selected && reveal {
                                        ui.scroll_to_rect(response.rect, None);
                                    }
                                    tab_rects.push(response.rect);
                                    if response.title.drag_started() {
                                        set_drag_cancelled(ui.ctx(), false);
                                    }
                                    if response.title.dragged() {
                                        dragged_tab = Some((tab_index, false));
                                    } else if response.title.drag_stopped() {
                                        dragged_tab = Some((tab_index, true));
                                    }
                                }
                                if !tab_rects.is_empty() {
                                    let band = egui::Rect::from_x_y_ranges(
                                        ui.clip_rect().x_range(),
                                        tab_rects
                                            .iter()
                                            .map(|rect| rect.top())
                                            .fold(f32::INFINITY, f32::min)
                                            ..=tab_rects
                                                .iter()
                                                .map(|rect| rect.bottom())
                                                .fold(f32::NEG_INFINITY, f32::max),
                                    );
                                    strip_geometry = Some(TabStripGeometry {
                                        group: self.editor.focused_group(),
                                        band,
                                        tabs: visible_files
                                            .iter()
                                            .map(|index| self.editor.files[*index].id)
                                            .zip(tab_rects.iter().copied())
                                            .collect(),
                                    });
                                }
                                if let Some((from, released)) = dragged_tab {
                                    let id = visible_files[from];
                                    let id = self.editor.files[id].id;
                                    if !released {
                                        drag_state = Some(TabDrag {
                                            source_group: self.editor.focused_group(),
                                        });
                                    }
                                    match drive_tab_drag(ui, &tab_rects, from, released) {
                                        DragOutcome::Move(position) => {
                                            tab_drop = Some((visible_files[from], position));
                                        }
                                        DragOutcome::Outside(pointer) => {
                                            outside_drop = Some((id, pointer));
                                        }
                                        DragOutcome::Holding | DragOutcome::Nothing => {}
                                    }
                                }
                                if let Some(diff) = self
                                    .view
                                    .source_control
                                    .diff
                                    .as_ref()
                                    .filter(|_| !hide_virtual)
                                {
                                    let index = visible_files.len();
                                    let selected = active == Some(TabTarget::Diff);
                                    let dirty = diff.document.is_some_and(|document| {
                                        self.editor
                                            .files
                                            .iter()
                                            .any(|file| file.id == document && file.dirty)
                                    });
                                    let response = tab(
                                        ui,
                                        &titles[index],
                                        sizing.widths[index],
                                        selected,
                                        &diff.path.display().to_string(),
                                        "Close diff editor",
                                        dirty,
                                    );
                                    open_diff = response.select;
                                    close_diff = response.close;
                                    if selected && reveal {
                                        ui.scroll_to_rect(response.rect, None);
                                    }
                                }
                                if settings_tab {
                                    let index = visible_files.len() + usize::from(diff_tab);
                                    let selected = active == Some(TabTarget::Settings);
                                    let response = tab(
                                        ui,
                                        &titles[index],
                                        sizing.widths[index],
                                        selected,
                                        "Settings",
                                        "Close Settings (drafts retained)",
                                        false,
                                    );
                                    open_settings = response.select;
                                    close_settings = response.close;
                                    if selected && reveal {
                                        ui.scroll_to_rect(response.rect, None);
                                    }
                                }
                                for (position, (id, _)) in task_tabs.iter().enumerate() {
                                    let index = task_base + position;
                                    let selected = active == Some(TabTarget::TaskEditor(*id));
                                    let tooltip = self
                                        .view
                                        .task_editors
                                        .get(*id)
                                        .map(|tab| format!("{} task", tab.scope().label()))
                                        .unwrap_or_default();
                                    let response = tab(
                                        ui,
                                        &titles[index],
                                        sizing.widths[index],
                                        selected,
                                        &tooltip,
                                        "Close task editor",
                                        self.view
                                            .task_editors
                                            .get(*id)
                                            .is_some_and(|tab| tab.dirty()),
                                    );
                                    if response.select {
                                        open_task = Some(*id);
                                    }
                                    if response.close {
                                        close_task = Some(*id);
                                    }
                                    if selected && reveal {
                                        ui.scroll_to_rect(response.rect, None);
                                    }
                                }
                            });
                        });
                },
            );
            if sizing.overflow {
                ui.spacing_mut().button_padding.x = 2.0;
                ui.spacing_mut().interact_size.x = ACTION_SIZE;
                ui.menu_button(icon::CARET_DOWN, |ui| {
                    ui.set_max_width(360.0);
                    ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for (tab_index, &file_index) in visible_files.iter().enumerate() {
                            let file = &self.editor.files[file_index];
                            let title = format!(
                                "{}{}",
                                titles[tab_index],
                                if file.dirty { " ●" } else { "" }
                            );
                            if ui
                                .selectable_label(
                                    active == Some(TabTarget::Document(file.id)),
                                    title,
                                )
                                .on_hover_text(file.path.as_ref().map_or_else(
                                    || file.name.clone(),
                                    |path| path.display().to_string(),
                                ))
                                .clicked()
                            {
                                select = Some(file_index);
                                ui.close();
                            }
                        }
                        if let Some(diff) = self
                            .view
                            .source_control
                            .diff
                            .as_ref()
                            .filter(|_| !hide_virtual)
                            && ui
                                .selectable_label(
                                    active == Some(TabTarget::Diff),
                                    format!("{} (Diff)", diff.path.display()),
                                )
                                .clicked()
                        {
                            open_diff = true;
                            ui.close();
                        }
                        if settings_tab && ui.selectable_label(settings_open, "Settings").clicked()
                        {
                            open_settings = true;
                            ui.close();
                        }
                        for (id, title) in &task_tabs {
                            if ui
                                .selectable_label(active == Some(TabTarget::TaskEditor(*id)), title)
                                .clicked()
                            {
                                open_task = Some(*id);
                                ui.close();
                            }
                        }
                    });
                })
                .response
                .on_hover_text("Open editors — including hidden tabs");
            }
            if ui
                .add_sized(
                    [ACTION_SIZE, 22.0],
                    egui::Button::new(icon::PLUS).frame(false),
                )
                .on_hover_text("New text file")
                .clicked()
            {
                self.command(Command::NewUntitled);
            }
        });
        if let Some(index) = select {
            // Choosing the current editor from the dropdown must reveal it too,
            // even if the user has manually scrolled its header out of view.
            ui.ctx()
                .data_mut(|data| data.remove::<RevealSignature>(reveal_id));
            self.view.leave_page_tabs();
            if let Some(diff) = &mut self.view.source_control.diff {
                diff.active = false;
            }
            self.editor.set_active_index(Some(index));
            self.view.focus_editor = true;
        }
        if let Some(id) = close {
            self.command(Command::RequestClose(id));
        }
        if let Some(command) = menu_command {
            self.command(command);
        }
        if let Some(geometry) = strip_geometry {
            self.view
                .tab_strips
                .retain(|strip| strip.group != geometry.group);
            self.view.tab_strips.push(geometry);
        }
        if drag_state.is_some() {
            self.view.tab_drag = drag_state;
        }
        // A release away from this strip may be a drop onto another group, which
        // is only known once every group has drawn.
        if outside_drop.is_some() && self.editor.group_count() > 1 {
            self.view.tab_drop_request = outside_drop;
        }
        if let Some((file_index, position)) = tab_drop {
            let id = self.editor.files[file_index].id;
            self.command(Command::MoveTab(id, TabMove::To(position)));
        }
        if close_diff {
            crate::app::source_control::close_diff_preview(self.editor, self.view);
        } else if open_diff {
            self.view.leave_page_tabs();
            if let Some(diff) = &mut self.view.source_control.diff {
                diff.active = true;
                self.view.focus_editor = diff.document.is_some();
                self.editor.set_active_index(
                    diff.document
                        .and_then(|id| self.editor.files.iter().position(|file| file.id == id)),
                );
            }
        }
        if let Some(id) = close_task {
            self.command(Command::CloseTaskEditor(id));
        } else if let Some(id) = open_task {
            self.command(Command::ActivateTaskEditor(id));
        }
        if close_settings {
            self.view.settings.tab_open = false;
            self.view.settings.open = false;
        } else if open_settings {
            self.command(Command::Settings);
        }
        strip.response.rect
    }
}

/// Escape cannot abort an egui drag, so the cancel is remembered and honoured
/// when the button is released.
fn set_drag_cancelled(ctx: &egui::Context, cancelled: bool) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("tab_drag_cancelled"), cancelled));
}

fn is_drag_cancelled(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(egui::Id::new("tab_drag_cancelled")))
        .unwrap_or(false)
}

/// Show the insertion marker and scroll the strip while a tab is held, and on
/// release return the position to move it to. `None` while still dragging, for
/// a drop outside the strip, after Escape, or when the tab would not move.
/// What a drag amounted to this frame.
enum DragOutcome {
    /// Still held; nothing to do yet.
    Holding,
    /// Released over its own strip: move to this position among the tabs.
    Move(usize),
    /// Released away from its own strip, at this pointer position.
    Outside(egui::Pos2),
    /// Cancelled, or dropped where it already was.
    Nothing,
}

fn drive_tab_drag(
    ui: &mut egui::Ui,
    rects: &[egui::Rect],
    from: usize,
    released: bool,
) -> DragOutcome {
    use super::tab_drag::{edge_scroll_speed, insertion_gap, marker_x, resulting_position};
    let colors = crate::ui::theme::palette(ui.visuals());
    if !released {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    // egui itself ends a drag on Escape and reports it as a stop, so an Escape in
    // the same frame as the "release" is a cancel, not a drop. Consuming it also
    // keeps it from closing Find in the editor below.
    if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        set_drag_cancelled(ui.ctx(), true);
    }
    let Some(pointer) = ui.ctx().pointer_latest_pos() else {
        return DragOutcome::Nothing;
    };
    // The strip is as tall as its tabs and as wide as the visible viewport; the
    // clip rectangle alone spans the whole window height.
    let tabs_top = rects
        .iter()
        .map(|rect| rect.top())
        .fold(f32::INFINITY, f32::min);
    let tabs_bottom = rects
        .iter()
        .map(|rect| rect.bottom())
        .fold(f32::NEG_INFINITY, f32::max);
    let viewport = egui::Rect::from_x_y_ranges(ui.clip_rect().x_range(), tabs_top..=tabs_bottom);
    let cancelled = is_drag_cancelled(ui.ctx());
    let inside = viewport.expand2(egui::vec2(0.0, 24.0)).contains(pointer);
    if released {
        set_drag_cancelled(ui.ctx(), false);
        if cancelled {
            return DragOutcome::Nothing;
        }
        // Away from the strip: possibly onto another group, else abandoned.
        if !inside {
            return DragOutcome::Outside(pointer);
        }
    }
    let centres: Vec<f32> = rects.iter().map(|rect| rect.center().x).collect();
    let gap = insertion_gap(&centres, pointer.x);
    if released {
        let position = resulting_position(from, gap);
        return if position != from {
            DragOutcome::Move(position)
        } else {
            DragOutcome::Nothing
        };
    }
    if !cancelled
        && inside
        && let Some(x) = marker_x(rects, gap)
    {
        ui.painter().vline(
            x,
            viewport.y_range(),
            egui::Stroke::new(2.0_f32, colors.accent),
        );
    }
    if !cancelled {
        let speed = edge_scroll_speed(pointer.x, viewport.left(), viewport.right(), 28.0, 480.0);
        if speed != 0.0 && inside {
            ui.scroll_with_delta(egui::vec2(speed * ui.input(|input| input.stable_dt), 0.0));
            ui.ctx().request_repaint();
        }
    }
    DragOutcome::Holding
}

struct TabMenuTarget<'a> {
    id: DocumentId,
    path: Option<&'a Path>,
    /// Position among the visible tabs, and how many there are.
    position: usize,
    count: usize,
    /// Tabs without unsaved changes.
    saved: usize,
    split_enabled: bool,
    groups: usize,
}

/// Context menu for a document tab. It only records the chosen command; the
/// caller dispatches it after the strip is drawn, by stable document ID.
fn tab_menu(
    ui: &mut egui::Ui,
    target: TabMenuTarget<'_>,
    has_workspace: bool,
    chosen: &mut Option<Command>,
) {
    use crate::app::tab_close::TabCloseScope;
    let mut item = |ui: &mut egui::Ui, label: &str, enabled: bool, why: &str, command: Command| {
        let response = ui.add_enabled(enabled, egui::Button::new(label));
        let response = if enabled {
            response
        } else {
            response.on_disabled_hover_text(why)
        };
        if response.clicked() {
            *chosen = Some(command);
            ui.close();
        }
    };
    item(ui, "Close", true, "", Command::RequestClose(target.id));
    item(
        ui,
        "Close Others",
        target.count > 1,
        "There are no other tabs",
        Command::CloseTabs(TabCloseScope::Others(target.id)),
    );
    item(
        ui,
        "Close to the Left",
        target.position > 0,
        "There are no tabs to the left",
        Command::CloseTabs(TabCloseScope::ToTheLeft(target.id)),
    );
    item(
        ui,
        "Close to the Right",
        target.position + 1 < target.count,
        "There are no tabs to the right",
        Command::CloseTabs(TabCloseScope::ToTheRight(target.id)),
    );
    item(
        ui,
        "Close Saved",
        target.saved > 0,
        "Every open tab has unsaved changes",
        Command::CloseTabs(TabCloseScope::Saved),
    );
    item(
        ui,
        "Close All",
        true,
        "",
        Command::CloseTabs(TabCloseScope::All),
    );
    ui.separator();
    item(
        ui,
        "Move Left",
        target.position > 0,
        "This is already the first tab",
        Command::MoveTab(target.id, TabMove::Left),
    );
    item(
        ui,
        "Move Right",
        target.position + 1 < target.count,
        "This is already the last tab",
        Command::MoveTab(target.id, TabMove::Right),
    );
    if target.split_enabled {
        item(
            ui,
            if target.groups > 1 {
                "Move to Other Group"
            } else {
                "Split Right"
            },
            target.groups > 1 || target.count > 1,
            "Open another tab before splitting the editor",
            Command::MoveTabToGroup(target.id),
        );
    }
    ui.separator();
    let unsaved = "This file has not been saved yet";
    let path = target.path.map(Path::to_path_buf).unwrap_or_default();
    item(
        ui,
        "Copy Path",
        target.path.is_some(),
        unsaved,
        Command::CopyPath(path.clone(), false),
    );
    item(
        ui,
        "Copy Relative Path",
        target.path.is_some() && has_workspace,
        if target.path.is_some() {
            "Open a folder to copy a relative path"
        } else {
            unsaved
        },
        Command::CopyPath(path.clone(), true),
    );
    item(
        ui,
        if cfg!(target_os = "macos") {
            "Reveal in Finder"
        } else if cfg!(target_os = "windows") {
            "Reveal in File Explorer"
        } else {
            "Reveal in File Manager"
        },
        target.path.is_some(),
        unsaved,
        Command::RevealPath(path),
    );
}

struct TabResponse {
    rect: egui::Rect,
    select: bool,
    close: bool,
    /// The clickable title, which owns the tab's context menu.
    title: egui::Response,
}

#[cfg(test)]
mod tests;
fn tab(
    ui: &mut egui::Ui,
    title: &str,
    width: f32,
    selected: bool,
    tooltip: &str,
    close_tooltip: &str,
    dirty: bool,
) -> TabResponse {
    let colors = crate::ui::theme::palette(ui.visuals());
    let mut select = false;
    let mut close = false;
    let mut title_response = None;
    let frame = Frame::new()
        .fill(if selected { colors.bg } else { colors.tab })
        .inner_margin(Margin::symmetric(10, 6))
        .corner_radius(CornerRadius {
            nw: 3,
            ne: 3,
            sw: 0,
            se: 0,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let response = ui
                    .add_sized(
                        [width - 50.0, 22.0],
                        egui::Button::new(RichText::new(title).color(if selected {
                            colors.text
                        } else {
                            colors.muted
                        }))
                        .frame(false)
                        .truncate()
                        .sense(egui::Sense::click_and_drag()),
                    )
                    .on_hover_text(tooltip);
                select = response.clicked();
                title_response = Some(response);
                close = widgets::named(
                    ui.add_sized(
                        [ACTION_SIZE, 22.0],
                        egui::Button::new(if dirty { "●" } else { icon::X }).frame(false),
                    )
                    .on_hover_text(close_tooltip),
                    true,
                    close_tooltip,
                )
                .clicked();
            });
        });
    if selected {
        ui.painter().hline(
            frame.response.rect.x_range(),
            frame.response.rect.top(),
            Stroke::new(1.0_f32, colors.accent),
        );
    }
    // Middle-click anywhere on the tab closes it, as in other editors.
    let middle_click = ui.input(|input| input.pointer.button_clicked(egui::PointerButton::Middle))
        && frame.response.contains_pointer();
    TabResponse {
        rect: frame.response.rect,
        select,
        close: close || middle_click,
        title: title_response.expect("tab title is always drawn"),
    }
}
