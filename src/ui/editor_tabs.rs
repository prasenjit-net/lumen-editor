//! Compact, single-row editor tabs with clipped overflow and fixed toolbar actions.
use super::{
    Workbench,
    theme::{ACCENT, BG, MUTED, TAB, TEXT},
    widgets,
};
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
        let titles: Vec<_> = self
            .editor
            .files
            .iter()
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
                        .layout_no_wrap(title.clone(), font.clone(), TEXT)
                        .size()
                        .x
                        + 62.0
                })
                .clamp(MIN_WIDTH, MAX_WIDTH)
            })
            .collect();
        let active_index = if self.view.settings.open {
            self.view
                .settings
                .tab_open
                .then_some(self.editor.files.len())
        } else {
            self.editor.active_file
        };
        let sizing = TabSizing::new(ui.available_width(), &natural, active_index);
        let active = if self.view.settings.open {
            Some(TabTarget::Settings)
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
        let mut open_settings = false;
        let mut close_settings = false;
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
                                for (index, file) in self.editor.files.iter().enumerate() {
                                    let selected = active == Some(TabTarget::Document(file.id));
                                    let response = tab(
                                        ui,
                                        &titles[index],
                                        sizing.widths[index],
                                        selected,
                                        &file.path.as_ref().map_or_else(
                                            || file.name.clone(),
                                            |path| path.display().to_string(),
                                        ),
                                        "Close editor",
                                        file.dirty,
                                    );
                                    if response.select {
                                        select = Some(index);
                                    }
                                    if response.close {
                                        close = Some(index);
                                    }
                                    if selected && reveal {
                                        ui.scroll_to_rect(response.rect, None);
                                    }
                                }
                                if self.view.settings.tab_open {
                                    let index = self.editor.files.len();
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
                        for (index, file) in self.editor.files.iter().enumerate() {
                            let title =
                                format!("{}{}", titles[index], if file.dirty { " ●" } else { "" });
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
                                select = Some(index);
                                ui.close();
                            }
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
            self.editor.active_file = Some(index);
            self.view.focus_editor = true;
        }
        if let Some(index) = close {
            self.command(Command::RequestClose(index));
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

struct TabResponse {
    rect: egui::Rect,
    select: bool,
    close: bool,
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
    let mut select = false;
    let mut close = false;
    let frame = Frame::new()
        .fill(if selected { BG } else { TAB })
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
                select = ui
                    .add_sized(
                        [width - 50.0, 22.0],
                        egui::Button::new(RichText::new(title).color(if selected {
                            TEXT
                        } else {
                            MUTED
                        }))
                        .frame(false)
                        .truncate(),
                    )
                    .on_hover_text(tooltip)
                    .clicked();
                close = ui
                    .add_sized(
                        [ACTION_SIZE, 22.0],
                        egui::Button::new(if dirty { "●" } else { icon::X }).frame(false),
                    )
                    .on_hover_text(close_tooltip)
                    .clicked();
            });
        });
    if selected {
        ui.painter().hline(
            frame.response.rect.x_range(),
            frame.response.rect.top(),
            Stroke::new(1.0_f32, ACCENT),
        );
    }
    TabResponse {
        rect: frame.response.rect,
        select,
        close,
    }
}
