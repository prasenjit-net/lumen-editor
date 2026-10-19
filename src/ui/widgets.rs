use crate::{app::command::Command, icon};
use eframe::egui::{self, Color32, CornerRadius, FontId, Frame, Margin, RichText, Stroke, Vec2};
use std::path::Path;

/// Give an icon-only control a spoken name. Tooltips are not read by screen
/// readers, and a bare glyph would be announced as a private-use character.
pub(crate) fn named(response: egui::Response, enabled: bool, name: &str) -> egui::Response {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
    response
}

pub(crate) struct HeaderAction {
    pub(crate) glyph: &'static str,
    pub(crate) tooltip: &'static str,
    pub(crate) command: Command,
    pub(crate) enabled: bool,
}

/// Compact workbench header: title stays left; actions stay at the right edge.
/// Reserve action space first so a long title never pushes buttons out of view.
pub(crate) fn panel_header(
    ui: &mut egui::Ui,
    title: &str,
    actions: &[HeaderAction],
) -> Option<Command> {
    let colors = crate::ui::theme::palette(ui.visuals());
    header_row(ui, actions, |ui| {
        ui.add(
            egui::Label::new(RichText::new(title).size(11.0).color(colors.text))
                .halign(egui::Align::LEFT)
                .truncate(),
        );
        None
    })
}

/// A selectable tab in a bottom-panel header (Terminal / Output).
pub(crate) struct PanelTab<'a> {
    pub(crate) label: &'a str,
    pub(crate) active: bool,
    pub(crate) command: Command,
    /// Hover text; empty for none.
    pub(crate) tooltip: &'a str,
}

/// Longest a tab label may be before it is truncated with an ellipsis.
const MAX_TAB_WIDTH: f32 = 170.0;

/// Same geometry as `panel_header`, with tabs where the title would be. The
/// active tab is underlined in the accent color; actions stay at the right.
pub(crate) fn tabbed_panel_header(
    ui: &mut egui::Ui,
    tabs: &[PanelTab],
    actions: &[HeaderAction],
) -> Option<Command> {
    let colors = crate::ui::theme::palette(ui.visuals());
    header_row(ui, actions, |ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let mut clicked = None;
        // Many tabs (task runs) must stay reachable: the strip scrolls sideways
        // inside the space left of the actions, and the active tab is kept in view.
        let seen_id = ui.id().with("tab_strip_active");
        let active_label = tabs
            .iter()
            .find(|tab| tab.active)
            .map(|tab| tab.label.to_string());
        let previous: Option<Option<String>> = ui.data(|data| data.get_temp(seen_id));
        let reveal = previous.as_ref() != Some(&active_label);
        ui.data_mut(|data| data.insert_temp(seen_id, active_label));
        egui::ScrollArea::horizontal()
            .id_salt("bottom_tab_strip")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for tab in tabs {
                    let response = ui
                        .scope(|ui| {
                            ui.set_max_width(MAX_TAB_WIDTH);
                            ui.add(
                                egui::Label::new(RichText::new(tab.label).size(11.0).color(
                                    if tab.active {
                                        colors.text
                                    } else {
                                        colors.muted
                                    },
                                ))
                                // A selectable label would start a text selection on click, and
                                // that selection would then sweep over whatever panel replaces it.
                                .selectable(false)
                                .sense(egui::Sense::click())
                                .truncate(),
                            )
                        })
                        .inner;
                    let response = if tab.tooltip.is_empty() {
                        response
                    } else {
                        response.on_hover_text(tab.tooltip)
                    };
                    if tab.active {
                        let rect = response.rect;
                        ui.painter().hline(
                            rect.x_range(),
                            rect.bottom() + 3.0,
                            Stroke::new(1.0_f32, colors.accent),
                        );
                        if reveal {
                            response.scroll_to_me(None);
                        }
                    }
                    if response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                        && !tab.active
                    {
                        clicked = Some(tab.command.clone());
                    }
                }
            });
        clicked
    })
}

fn header_row(
    ui: &mut egui::Ui,
    actions: &[HeaderAction],
    draw_title: impl FnOnce(&mut egui::Ui) -> Option<Command>,
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
    let title_command = header_ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(title_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.set_clip_rect(title_rect.intersect(ui.clip_rect()));
            draw_title(ui)
        },
    );
    let mut clicked = title_command.inner;
    for (index, action) in actions.iter().enumerate() {
        let button_rect = egui::Rect::from_min_size(
            egui::pos2(
                actions_left + index as f32 * (BUTTON_SIZE + GAP),
                rect.center().y - BUTTON_SIZE / 2.0,
            ),
            Vec2::splat(BUTTON_SIZE),
        );
        let button = header_ui
            .add_enabled_ui(action.enabled, |ui| {
                ui.put(button_rect, egui::Button::new(action.glyph).frame(false))
            })
            .inner
            .on_hover_text(action.tooltip);
        if named(button, action.enabled, action.tooltip).clicked() {
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
pub(crate) fn canvas(ctx: &egui::Context) -> Frame {
    Frame::default()
        .fill(crate::ui::theme::palette(&ctx.style().visuals).canvas)
        .inner_margin(Margin::same(2))
}
pub(crate) fn explorer_row(
    ui: &mut egui::Ui,
    text: String,
    indent: usize,
    selected: bool,
) -> egui::Response {
    explorer_row_with_icon(ui, None, None, text, indent, selected)
}

/// An explorer row with an optional disclosure caret and a coloured file or
/// folder icon ahead of the label. `indent` is where the row's first element starts.
pub(crate) fn explorer_row_with_icon(
    ui: &mut egui::Ui,
    caret: Option<&str>,
    file_icon: Option<crate::ui::file_icons::FileIcon>,
    text: String,
    indent: usize,
    selected: bool,
) -> egui::Response {
    let colors = crate::ui::theme::palette(ui.visuals());
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::ZERO,
            if selected {
                colors.active
            } else {
                colors.hover
            },
        );
    }
    if response.has_focus() {
        // Keyboard focus is independent of selection and survives hover.
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            CornerRadius::same(2),
            Stroke::new(1.0_f32, colors.accent),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &text)
    });
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let mut x = rect.left() + indent as f32;
    if let Some(caret) = caret {
        painter.text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            caret,
            FontId::proportional(13.0),
            colors.text,
        );
        x += 16.0;
    }
    if let Some(file_icon) = file_icon {
        painter.text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            file_icon.glyph,
            FontId::new(15.0, crate::ui::file_icons::family()),
            file_icon.color,
        );
        x += 20.0;
    }
    painter.text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.0),
        colors.text,
    );
    response
}
pub(crate) fn icon_button(ui: &mut egui::Ui, icon: &str, selected: bool) -> egui::Response {
    let colors = crate::ui::theme::palette(ui.visuals());
    ui.add(
        egui::Button::new(RichText::new(icon).size(24.0).color(if selected {
            colors.text
        } else {
            colors.muted
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

    #[test]
    fn explorer_rows_expose_name_selection_and_a_keyboard_focus_ring() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        ctx.enable_accesskit();
        let id = std::cell::Cell::new(None);
        let run = |focus: Option<egui::Id>| {
            ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = explorer_row(ui, "main.rs".into(), 0, true);
                    id.set(Some(response.id));
                    if let Some(focus) = focus {
                        ctx.memory_mut(|memory| memory.request_focus(focus));
                    }
                });
            })
        };
        let mut nodes = Vec::new();
        let mut collect = |output: &egui::FullOutput| {
            if let Some(update) = &output.platform_output.accesskit_update {
                nodes.extend(update.nodes.iter().map(|(_, node)| node.clone()));
            }
        };
        collect(&run(None));
        collect(&run(id.get()));
        let output = run(None);
        collect(&output);
        let accent = crate::ui::theme::Theme::Dark.palette().accent;
        assert!(output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Rect(rect) if rect.stroke.color == accent
        )));
        assert!(nodes.iter().any(|node| {
            node.label() == Some("main.rs") && format!("{:?}", node.toggled()) == "Some(True)"
        }));
    }

    fn actions() -> [HeaderAction; 3] {
        [
            HeaderAction {
                glyph: "N",
                tooltip: "New file",
                command: Command::NewFile,
                enabled: true,
            },
            HeaderAction {
                glyph: "O",
                tooltip: "Open file",
                command: Command::OpenFile,
                enabled: true,
            },
            HeaderAction {
                glyph: "F",
                tooltip: "Open folder",
                command: Command::OpenFolder,
                enabled: true,
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
