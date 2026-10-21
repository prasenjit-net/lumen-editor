//! One anatomy for every dialog: a title strip with a close button, a padded
//! body, and a right-aligned footer. Blocking confirmations are modal and dim the
//! workbench; reviews that can stay open are plain floating windows.
//!
//! The helpers only draw. Each caller keeps its own button handling, default
//! focus (`keep_focus`) and Escape handling (`escape_pressed`), so a restyle never
//! changes what a button does.
use super::{theme, widgets::named};
use crate::icon;
use eframe::egui::{self, Align, CornerRadius, Frame, Layout, Margin, RichText, Stroke, Vec2};

const TITLE_HEIGHT: f32 = 30.0;

pub(super) struct Dialog {
    title: String,
    width: f32,
    modal: bool,
    resizable: bool,
}

/// What a dialog's chrome reports besides its body's own result.
pub(super) struct DialogOutput<R> {
    pub inner: R,
    /// The title-strip close button was clicked; treat it as Cancel.
    pub close_clicked: bool,
    /// A modal was dismissed with Escape or a click on its backdrop. Never a
    /// confirmation; callers that already handle Escape themselves may ignore it.
    pub dismissed: bool,
}

impl Dialog {
    pub(super) fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            width: 420.0,
            modal: true,
            resizable: false,
        }
    }

    pub(super) fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// A floating window without a scrim, for content that can stay open.
    pub(super) fn non_modal(mut self, resizable: bool) -> Self {
        self.modal = false;
        self.resizable = resizable;
        self
    }

    pub(super) fn show<R>(
        self,
        ctx: &egui::Context,
        body: impl FnOnce(&mut egui::Ui) -> R,
    ) -> DialogOutput<R> {
        let colors = theme::palette(&ctx.style().visuals);
        let frame = super::widgets::floating_frame(ctx).inner_margin(Margin::ZERO);
        let width = self
            .width
            .min((ctx.content_rect().width() - 32.0).max(160.0));
        let mut close_clicked = false;
        let content = |ui: &mut egui::Ui| {
            ui.set_width(width);
            close_clicked = title_strip(ui, &self.title);
            Frame::default()
                .inner_margin(Margin::symmetric(14, 12))
                .show(ui, |ui| body(ui))
                .inner
        };
        if self.modal {
            let modal = egui::Modal::new(egui::Id::new(("dialog", self.title.as_str())))
                .frame(frame)
                .backdrop_color(colors.scrim)
                .show(ctx, content);
            DialogOutput {
                dismissed: modal.should_close(),
                inner: modal.inner,
                close_clicked,
            }
        } else {
            let response = egui::Window::new(self.title.as_str())
                .title_bar(false)
                .collapsible(false)
                .resizable(self.resizable)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .frame(frame)
                .show(ctx, content);
            DialogOutput {
                inner: response
                    .and_then(|response| response.inner)
                    .expect("a visible window runs its body"),
                close_clicked,
                dismissed: false,
            }
        }
    }
}

fn title_strip(ui: &mut egui::Ui, title: &str) -> bool {
    let colors = theme::palette(ui.visuals());
    let mut close = false;
    Frame::default()
        .fill(colors.tab)
        .corner_radius(CornerRadius {
            nw: 5,
            ne: 5,
            sw: 0,
            se: 0,
        })
        .inner_margin(Margin {
            left: 12,
            right: 4,
            top: 0,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(TITLE_HEIGHT);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = ui
                    .add(
                        egui::Button::new(RichText::new(icon::X).size(13.0).color(colors.muted))
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .min_size(Vec2::new(24.0, 22.0)),
                    )
                    .on_hover_text("Close (Escape)");
                close = named(button, true, "Close dialog").clicked();
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(title).size(12.5).strong().color(colors.text),
                        )
                        .truncate(),
                    );
                });
            });
        });
    ui.painter().hline(
        ui.min_rect().x_range(),
        ui.cursor().top(),
        Stroke::new(1.0_f32, colors.border),
    );
    close
}

/// Right-aligned button row. Add the primary action first: the layout runs right
/// to left, so reading order becomes secondary actions, Cancel, then primary.
pub(super) fn footer<R>(ui: &mut egui::Ui, buttons: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(12.0);
    ui.with_layout(Layout::right_to_left(Align::Center), buttons)
        .inner
}

/// A filled destructive action; its text color is paired with the fill.
pub(super) fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let colors = theme::palette(ui.visuals());
    ui.add(egui::Button::new(RichText::new(label).color(colors.on_danger)).fill(colors.danger))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::configure_ui;

    struct Layout {
        cancel_x: f32,
        primary_x: f32,
        title: bool,
        close_labelled: bool,
        width: f32,
    }

    fn render(screen_width: f32) -> Layout {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        ctx.enable_accesskit();
        let mut cancel_x = 0.0;
        let mut primary_x = 0.0;
        let mut width = 0.0;
        // The first pass of a new Area only measures it; the second draws it.
        let mut output = None;
        for _ in 0..2 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(screen_width, 400.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    Dialog::new("Unsaved changes").width(420.0).show(ctx, |ui| {
                        width = ui.available_width();
                        ui.label("Save changes?");
                        footer(ui, |ui| {
                            primary_x = super::super::dialogs::primary_button(ui, "Save")
                                .rect
                                .left();
                            cancel_x = ui.button("Cancel").rect.left();
                        });
                    });
                },
            ));
        }
        let output = output.unwrap();
        let labels: Vec<String> = output
            .platform_output
            .accesskit_update
            .iter()
            .flat_map(|update| update.nodes.iter())
            .filter_map(|(_, node)| node.label().map(str::to_owned))
            .collect();
        let title = output.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text() == "Unsaved changes")
        });
        Layout {
            cancel_x,
            primary_x,
            title,
            close_labelled: labels.iter().any(|label| label == "Close dialog"),
            width,
        }
    }

    #[test]
    fn every_dialog_has_a_title_strip_close_button_and_right_aligned_footer() {
        for screen_width in [360.0, 900.0] {
            let layout = render(screen_width);
            assert!(layout.title, "title strip at {screen_width}");
            assert!(
                layout.close_labelled,
                "close button is named at {screen_width}"
            );
            assert!(
                layout.cancel_x < layout.primary_x,
                "Cancel precedes the primary action at {screen_width}"
            );
            assert!(
                layout.width <= screen_width - 32.0 + 1.0,
                "dialog stays inside a {screen_width}px window, content {}",
                layout.width
            );
        }
    }
}
