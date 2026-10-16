//! Compact, single-row editor tabs with clipped overflow and fixed toolbar actions.
use super::{Workbench, widgets};
use crate::{app::command::Command, icon, model::DocumentId};
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
        let hidden_document = self
            .view
            .source_control
            .diff
            .as_ref()
            .and_then(|diff| diff.diff_only_document);
        let visible_files: Vec<_> = self
            .editor
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| Some(file.id) != hidden_document)
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
            .chain(self.view.source_control.diff.as_ref().map(|diff| {
                let name = diff
                    .path
                    .file_name()
                    .unwrap_or(diff.path.as_os_str())
                    .to_string_lossy();
                format!("{} {} (Diff)", icon::GIT_BRANCH, name)
            }))
            .chain(
                self.view
                    .settings
                    .tab_open
                    .then(|| format!("{} Settings", icon::GEAR)),
            )
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
        let diff_active = self
            .view
            .source_control
            .diff
            .as_ref()
            .is_some_and(|diff| diff.active);
        let active_index = if self.view.settings.open {
            self.view.settings.tab_open.then_some(
                visible_files.len() + usize::from(self.view.source_control.diff.is_some()),
            )
        } else if diff_active {
            Some(visible_files.len())
        } else {
            self.editor
                .active_file
                .and_then(|index| visible_files.iter().position(|visible| *visible == index))
        };
        let sizing = TabSizing::new(ui.available_width(), &natural, active_index);
        let active = if self.view.settings.open {
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
            || (self.view.settings.open && self.view.settings.focus_search);
        ui.ctx()
            .data_mut(|data| data.insert_temp(reveal_id, signature));
        let mut select = None;
        let mut close = None;
        let mut menu_command: Option<Command> = None;
        let mut open_settings = false;
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
                                                index: file_index,
                                                id: file.id,
                                                path: file.path.as_deref(),
                                                position: tab_index,
                                                count: visible_files.len(),
                                            },
                                            self.editor.workspace.is_some(),
                                            &mut menu_command,
                                        );
                                    });
                                    if response.select {
                                        select = Some(file_index);
                                    }
                                    if response.close {
                                        close = Some(file_index);
                                    }
                                    if selected && reveal {
                                        ui.scroll_to_rect(response.rect, None);
                                    }
                                }
                                if let Some(diff) = &self.view.source_control.diff {
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
                                if self.view.settings.tab_open {
                                    let index = visible_files.len()
                                        + usize::from(self.view.source_control.diff.is_some());
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
                        if let Some(diff) = &self.view.source_control.diff
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
                        if self.view.settings.tab_open
                            && ui
                                .selectable_label(self.view.settings.open, "Settings")
                                .clicked()
                        {
                            open_settings = true;
                            ui.close();
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
            self.view.settings.open = false;
            if let Some(diff) = &mut self.view.source_control.diff {
                diff.active = false;
            }
            self.editor.active_file = Some(index);
            self.view.focus_editor = true;
        }
        if let Some(index) = close {
            self.command(Command::RequestClose(index));
        }
        if let Some(command) = menu_command {
            self.command(command);
        }
        if close_diff {
            crate::app::source_control::close_diff_preview(self.editor, self.view);
        } else if open_diff {
            self.view.settings.open = false;
            if let Some(diff) = &mut self.view.source_control.diff {
                diff.active = true;
                self.view.focus_editor = diff.document.is_some();
                self.editor.active_file = diff
                    .document
                    .and_then(|id| self.editor.files.iter().position(|file| file.id == id));
            }
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

struct TabMenuTarget<'a> {
    index: usize,
    id: DocumentId,
    path: Option<&'a Path>,
    /// Position among the visible tabs, and how many there are.
    position: usize,
    count: usize,
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
    item(ui, "Close", true, "", Command::RequestClose(target.index));
    item(
        ui,
        "Close Others",
        target.count > 1,
        "There are no other tabs",
        Command::CloseTabs(TabCloseScope::Others(target.id)),
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
        "Close All",
        true,
        "",
        Command::CloseTabs(TabCloseScope::All),
    );
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
                        .truncate(),
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
