use super::Workbench;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use super::theme::MUTED;
use super::theme::STATUS;
#[cfg(not(target_os = "windows"))]
use super::theme::{TEXT, TITLE};
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use crate::icon;
use eframe::egui::{self, Color32, Frame, Margin, RichText, TopBottomPanel};
#[cfg(target_os = "macos")]
use eframe::egui::{FontId, Vec2};
impl Workbench<'_> {
    pub(crate) fn title_bar(&mut self, ctx: &egui::Context) {
        let title = self
            .active()
            .map(|file| {
                format!(
                    "{}{} — Lumen",
                    file.name,
                    if file.dirty { " ●" } else { "" }
                )
            })
            .unwrap_or_else(|| "Lumen".into());
        if self.view.window_title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.view.window_title = title;
        }
        #[cfg(target_os = "macos")]
        TopBottomPanel::top("mac_titlebar")
            .exact_height(32.0)
            .frame(Frame::default().fill(TITLE))
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 32.0),
                    egui::Sense::hover(),
                );
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &self.view.window_title,
                    FontId::proportional(13.0),
                    TEXT,
                );
                // Leave the leftmost 80px to AppKit's real traffic-light buttons.
                let drag_rect = egui::Rect::from_min_max(rect.min + Vec2::new(80.0, 0.0), rect.max);
                let response = ui.interact(
                    drag_rect,
                    ui.id().with("window_drag"),
                    egui::Sense::click_and_drag(),
                );
                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if response.double_clicked() {
                    let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
            });
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        TopBottomPanel::top("titlebar")
            .exact_height(35.0)
            .frame(
                Frame::default()
                    .fill(TITLE)
                    .inner_margin(Margin::symmetric(8, 0)),
            )
            .show(ctx, |ui| {
                Frame::default().fill(TITLE).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if !self.view.safety_notices.is_empty() {
                            let label = format!("Safety: {}", self.view.safety_notices.len());
                            if ui
                                .small_button(label)
                                .on_hover_text("Show recovery and save warnings")
                                .clicked()
                            {
                                self.view.safety_details_open = true;
                            }
                        }
                        ui.label(
                            RichText::new(format!("{}  LUMEN", icon::BRACKETS_CURLY))
                                .strong()
                                .color(MUTED),
                        );
                        ui.add_space(12.0);
                        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                        {
                            self.file_menu(ui);
                            for item in
                                ["Edit", "Selection", "View", "Go", "Run", "Terminal", "Help"]
                            {
                                ui.label(RichText::new(item).size(12.0).color(TEXT));
                                ui.add_space(2.0);
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let title = self
                                .active()
                                .map(|file| {
                                    format!(
                                        "{}{} — Lumen",
                                        file.name,
                                        if file.dirty { " ●" } else { "" }
                                    )
                                })
                                .unwrap_or_else(|| "Lumen".into());
                            ui.label(RichText::new(title).color(MUTED).size(12.0));
                        });
                    });
                });
            });
    }
    pub(crate) fn status_bar(&mut self, ctx: &egui::Context) {
        TopBottomPanel::bottom("status")
            .exact_height(22.0)
            .frame(
                Frame::default()
                    .fill(STATUS)
                    .inner_margin(Margin::symmetric(8, 0)),
            )
            .show(ctx, |ui| {
                Frame::default().fill(STATUS).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&self.view.status)
                                .size(11.0)
                                .color(Color32::WHITE),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            #[cfg(feature = "shared-editor")]
                            let cursors = self
                                .active()
                                .and_then(|file| file.editing.as_ref())
                                .map(|buffer| {
                                    let selections = buffer.selections().selections();
                                    if selections.len() > 1 {
                                        let noun = if selections
                                            .iter()
                                            .any(|selection| !selection.range().is_empty())
                                        {
                                            "selections"
                                        } else {
                                            "cursors"
                                        };
                                        format!("{} {noun}   ", selections.len())
                                    } else {
                                        String::new()
                                    }
                                })
                                .unwrap_or_default();
                            #[cfg(not(feature = "shared-editor"))]
                            let cursors = String::new();
                            ui.label(
                                RichText::new(format!(
                                    "{cursors}{}   UTF-8   LF",
                                    self.active()
                                        .map(|file| file.language.as_str())
                                        .unwrap_or("Plain Text")
                                ))
                                .size(11.0)
                                .color(Color32::WHITE),
                            );
                        });
                    });
                });
            });
    }
}
