use crate::{app::command::Command, icon, ui::theme::*};
use eframe::egui::{self, Color32, CornerRadius, FontId, Frame, Margin, RichText, Stroke, Vec2};
use std::path::Path;

pub(crate) struct HeaderAction {
    pub(crate) glyph: &'static str,
    pub(crate) tooltip: &'static str,
    pub(crate) command: Command,
}

/// Compact workbench header: title stays left; actions stay at the right edge.
/// Reserve action space first so a long title never pushes buttons out of view.
pub(crate) fn panel_header(
    ui: &mut egui::Ui,
    title: &str,
    actions: &[HeaderAction],
) -> Option<Command> {
    const HEIGHT: f32 = 35.0;
    const LEFT_INSET: f32 = 20.0;
    const RIGHT_INSET: f32 = 6.0;
    const BUTTON_SIZE: f32 = 22.0;
    const GAP: f32 = 4.0;

    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), HEIGHT),
        egui::Sense::hover(),
    );
    let actions_width =
        actions.len() as f32 * BUTTON_SIZE + actions.len().saturating_sub(1) as f32 * GAP;
    let actions_left = rect.right() - RIGHT_INSET - actions_width;
    let title_left = rect.left() + LEFT_INSET;
    let title_right = if actions.is_empty() {
        rect.right() - RIGHT_INSET
    } else {
        actions_left - GAP
    };
    let title_rect = egui::Rect::from_min_max(
        egui::pos2(title_left, rect.top()),
        egui::pos2(title_right.max(title_left), rect.bottom()),
    );
    // Use a child UI for positioned controls: `put` advances its own cursor,
    // which must not change the parent row's already-allocated height.
    let mut header_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("panel_header")
            .max_rect(rect),
    );
    header_ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(title_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.set_clip_rect(title_rect.intersect(ui.clip_rect()));
            ui.add(
                egui::Label::new(RichText::new(title).size(11.0).color(TEXT))
                    .halign(egui::Align::LEFT)
                    .truncate(),
            );
        },
    );

    let mut clicked = None;
    for (index, action) in actions.iter().enumerate() {
        let button_rect = egui::Rect::from_min_size(
            egui::pos2(
                actions_left + index as f32 * (BUTTON_SIZE + GAP),
                rect.center().y - BUTTON_SIZE / 2.0,
            ),
            Vec2::splat(BUTTON_SIZE),
        );
        if header_ui
            .put(button_rect, egui::Button::new(action.glyph).frame(false))
            .on_hover_text(action.tooltip)
            .clicked()
        {
            clicked = Some(action.command.clone());
        }
    }
    clicked
}

pub(crate) fn surface(fill: Color32) -> Frame {
    Frame::default()
        .fill(fill)
        .corner_radius(CornerRadius::same(5))
}
pub(crate) fn canvas() -> Frame {
    Frame::default()
        .fill(Color32::from_gray(24))
        .inner_margin(Margin::same(2))
}
pub(crate) fn explorer_row(
    ui: &mut egui::Ui,
    text: String,
    indent: usize,
    selected: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::ZERO,
            if selected { ACTIVE } else { HOVER },
        );
    }
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .text(
            egui::pos2(rect.left() + indent as f32, rect.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            FontId::proportional(13.0),
            TEXT,
        );
    response
}
pub(crate) fn icon_button(ui: &mut egui::Ui, icon: &str, selected: bool) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(icon).size(24.0).color(if selected {
            Color32::from_gray(215)
        } else {
            MUTED
        }))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::ZERO)
        .min_size(Vec2::splat(48.0)),
    )
}
pub(crate) fn file_icon(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
    {
        "rs" | "go" | "c" | "h" | "cpp" | "hpp" | "js" | "ts" | "tsx" | "jsx" => icon::FILE_CODE,
        "toml" | "json" | "yaml" | "yml" => icon::GEAR,
        "md" | "txt" => icon::FILE_TEXT,
        _ => icon::FILE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::view_state::Activity, ui::theme::configure_ui};

    fn actions() -> [HeaderAction; 3] {
        [
            HeaderAction {
                glyph: "N",
                tooltip: "New file",
                command: Command::NewFile,
            },
            HeaderAction {
                glyph: "O",
                tooltip: "Open file",
                command: Command::OpenFile,
            },
            HeaderAction {
                glyph: "F",
                tooltip: "Open folder",
                command: Command::OpenFolder,
            },
        ]
    }

    fn header_frame(
        ctx: &egui::Context,
        width: f32,
        title: &str,
        actions: &[HeaderAction],
        events: Vec<egui::Event>,
    ) -> (Option<Command>, egui::FullOutput) {
        let mut clicked = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 200.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(Frame::NONE)
                    .show(ctx, |ui| {
                        clicked = panel_header(ui, title, actions);
                        assert!(
                            (ui.cursor().top() - 35.0).abs() < 0.1,
                            "header height must stay fixed: {}",
                            ui.cursor().top()
                        );
                    });
            },
        );
        (clicked, output)
    }

    fn text_shape<'a>(output: &'a egui::FullOutput, text: &str) -> &'a egui::epaint::TextShape {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(shape) if shape.galley.job.text == text => Some(shape),
                _ => None,
            })
            .expect("header text should be rendered")
    }

    #[test]
    fn resizing_keeps_title_left_and_commands_at_the_right_edge() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        for width in [180.0, 240.0, 600.0] {
            let (_, output) = header_frame(&ctx, width, "EXPLORER", &actions(), vec![]);
            let title = text_shape(&output, "EXPLORER");
            assert!((title.pos.x - 20.0).abs() < 0.1);
            for (index, glyph) in ["N", "O", "F"].iter().enumerate() {
                let text = text_shape(&output, glyph);
                let center = text.pos.x + text.galley.size().x / 2.0;
                assert!((center - (width - 69.0 + index as f32 * 26.0)).abs() < 0.6);
            }
            assert!(title.pos.x + title.galley.size().x < width - 80.0);
        }
    }

    #[test]
    fn every_sidebar_title_is_left_aligned_without_actions() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        for activity in [
            Activity::Explorer,
            Activity::Search,
            Activity::SourceControl,
            Activity::RunDebug,
            Activity::Extensions,
        ] {
            for width in [180.0, 240.0, 600.0] {
                let (_, output) = header_frame(&ctx, width, activity.title(), &[], vec![]);
                assert!(
                    (text_shape(&output, activity.title()).pos.x - 20.0).abs() < 0.1,
                    "title x: {}",
                    text_shape(&output, activity.title()).pos.x
                );
            }
        }
    }

    #[test]
    fn long_titles_truncate_without_displacing_actions() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let title = "A VERY LONG WORKBENCH PANEL TITLE";
        let (_, output) = header_frame(&ctx, 180.0, title, &actions(), vec![]);
        let text = text_shape(&output, title);
        assert!(text.galley.elided);
        assert!(text.pos.x + text.galley.size().x <= 96.0);
        assert!(text.galley.size().y < 35.0);
        assert!(text_shape(&output, "N").pos.x > 100.0);
    }

    #[test]
    fn header_buttons_dispatch_the_correct_commands_at_each_width() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        for width in [180.0, 240.0, 600.0] {
            for (index, expected) in [Command::NewFile, Command::OpenFile, Command::OpenFolder]
                .into_iter()
                .enumerate()
            {
                header_frame(&ctx, width, "EXPLORER", &actions(), vec![]);
                let pos = egui::pos2(width - 69.0 + index as f32 * 26.0, 17.5);
                let button_event = |pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                header_frame(
                    &ctx,
                    width,
                    "EXPLORER",
                    &actions(),
                    vec![egui::Event::PointerMoved(pos), button_event(true)],
                );
                let (clicked, _) = header_frame(
                    &ctx,
                    width,
                    "EXPLORER",
                    &actions(),
                    vec![button_event(false)],
                );
                assert_eq!(clicked, Some(expected));
            }
        }
    }
}
