//! Toasts for notices that need to be seen: errors until dismissed, warnings for
//! a while. Information stays in the status bar (`chrome.rs`).
use super::Workbench;
use super::widgets;
use crate::{
    app::{
        command::Command,
        notices::{Notice, Severity},
    },
    icon,
};
use eframe::egui::{self, RichText};

/// Width of a toast's text column.
const TEXT_WIDTH: f32 = 290.0;
/// Seconds a toast takes to fade in.
const FADE_IN_SECS: f32 = 0.18;
/// Space kept below the toasts: the status bar plus a margin.
const BOTTOM_OFFSET: f32 = 22.0 + 10.0;

pub(crate) fn glyph(severity: Severity) -> Option<&'static str> {
    match severity {
        Severity::Error => Some(icon::ERROR),
        Severity::Warning => Some(icon::WARNING),
        Severity::Success => Some(icon::CHECK),
        Severity::Info => None,
    }
}

/// What the user did to a toast this frame.
#[derive(Default)]
struct ToastAction {
    dismissed: bool,
    show_output: bool,
}

/// Opacity of a toast `age` seconds after it was posted: it fades in over a
/// moment, or is simply there when motion is reduced.
fn fade_in(ctx: &egui::Context, age: f64) -> f32 {
    let fade = crate::ui::theme::motion_time(ctx, FADE_IN_SECS);
    if fade > 0.0 {
        (age as f32 / fade).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn severity_color(colors: &crate::ui::theme::Palette, severity: Severity) -> egui::Color32 {
    match severity {
        Severity::Error => colors.error,
        Severity::Warning => colors.warning,
        _ => colors.success,
    }
}

/// The message column: the text, and for errors a way to open the log.
fn message_column(ui: &mut egui::Ui, notice: &Notice, action: &mut ToastAction) {
    let colors = crate::ui::theme::palette(ui.visuals());
    ui.set_width(TEXT_WIDTH);
    let text = if notice.count > 1 {
        format!("{} (×{})", notice.text, notice.count)
    } else {
        notice.text.clone()
    };
    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(colors.text)).wrap());
    if notice.severity == Severity::Error {
        ui.add_space(6.0);
        let show = ui
            .button("Show Output")
            .on_hover_text("Open the Output panel, which logs this message");
        action.show_output |= show.clicked();
    }
}

fn dismiss_button(ui: &mut egui::Ui) -> bool {
    let close = widgets::tool_button(ui, false, 0.0, |ui| {
        ui.add(
            egui::Button::new(icon::X)
                .frame(false)
                .min_size(egui::vec2(22.0, 22.0)),
        )
    })
    .on_hover_text("Dismiss");
    widgets::named(close, true, "Dismiss notification").clicked()
}

/// One toast card.
fn toast(ctx: &egui::Context, ui: &mut egui::Ui, notice: &Notice, now: f64) -> ToastAction {
    let colors = crate::ui::theme::palette(&ctx.style().visuals);
    let opacity = fade_in(ctx, now - notice.posted_at);
    if opacity < 1.0 {
        ctx.request_repaint();
    }
    ui.set_opacity(opacity);
    let mut action = ToastAction::default();
    widgets::floating_frame(ctx)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(TEXT_WIDTH + 38.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if let Some(glyph) = glyph(notice.severity) {
                    let color = severity_color(&colors, notice.severity);
                    ui.label(RichText::new(glyph).size(14.0).color(color));
                }
                ui.vertical(|ui| message_column(ui, notice, &mut action));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    action.dismissed = dismiss_button(ui);
                });
            });
        });
    action
}

impl Workbench<'_> {
    pub(crate) fn notice_toasts(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        let toasts: Vec<Notice> = self.view.notices.toasts(now).into_iter().cloned().collect();
        if toasts.is_empty() {
            return;
        }
        let mut dismissed = Vec::new();
        let mut show_output = false;
        egui::Area::new(egui::Id::new("notice_toasts"))
            .order(egui::Order::Foreground)
            .anchor(
                egui::Align2::RIGHT_BOTTOM,
                egui::vec2(-12.0, -BOTTOM_OFFSET),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                for notice in &toasts {
                    let action = toast(ctx, ui, notice, now);
                    if action.dismissed {
                        dismissed.push(notice.id);
                    }
                    show_output |= action.show_output;
                }
            });
        for id in dismissed {
            self.view.notices.dismiss(id);
        }
        if show_output && !self.view.output.open {
            self.command(Command::ToggleOutput);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LumenApp;
    use crate::ui::theme::configure_ui;

    fn frame(app: &mut LumenApp, ctx: &egui::Context, time: f64) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.notice_toasts(ctx)),
        )
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

    #[test]
    fn an_error_toast_stays_and_information_never_pops_up() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = LumenApp::default();
        app.view
            .notices
            .post(Severity::Info, "Explorer refreshed", 0.0);
        assert!(texts(&frame(&mut app, &ctx, 0.5)).is_empty());
        app.view
            .notices
            .post(Severity::Error, "Could not pull: offline", 1.0);
        // An area measures itself on its first frame and draws on the next.
        frame(&mut app, &ctx, 1.05);
        let shown = texts(&frame(&mut app, &ctx, 1.1));
        assert!(
            shown.iter().any(|text| text.contains("Could not pull")),
            "{shown:?}"
        );
        assert!(shown.iter().any(|text| text == "Show Output"));
        let later = texts(&frame(&mut app, &ctx, 120.0));
        assert!(later.iter().any(|text| text.contains("Could not pull")));
    }

    #[test]
    fn dismissing_removes_the_toast_and_never_touches_the_document() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = LumenApp::default();
        app.editor.new_file();
        app.editor.files[0].text = "unsaved".into();
        app.editor.files[0].dirty = true;
        app.view
            .notices
            .post(Severity::Error, "Failed to save", 0.0);
        let id = app.view.notices.toasts(0.0)[0].id;
        app.view.notices.dismiss(id);
        assert!(texts(&frame(&mut app, &ctx, 1.0)).is_empty());
        assert_eq!(app.editor.files[0].text, "unsaved");
        assert!(app.editor.files[0].dirty);
    }
}
