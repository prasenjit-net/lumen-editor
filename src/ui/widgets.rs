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
    panel_header_with_menu(ui, title, actions, None)
}

/// `panel_header` with a trailing `⋯` menu at the fixed right edge, after the
/// icon actions. The menu is for commands that do not deserve a header icon.
pub(crate) fn panel_header_with_menu(
    ui: &mut egui::Ui,
    title: &str,
    actions: &[HeaderAction],
    menu: Option<(&str, &[MenuEntry])>,
) -> Option<Command> {
    let colors = crate::ui::theme::palette(ui.visuals());
    header_row(ui, actions, menu, |ui| {
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
    tabbed_panel_header_with_menu(ui, tabs, actions, None)
}

/// `tabbed_panel_header` with a trailing `⋯` menu after the icon actions, for
/// commands that move out of the header when it is too narrow to show them all.
pub(crate) fn tabbed_panel_header_with_menu(
    ui: &mut egui::Ui,
    tabs: &[PanelTab],
    actions: &[HeaderAction],
    menu: Option<(&str, &[MenuEntry])>,
) -> Option<Command> {
    let colors = crate::ui::theme::palette(ui.visuals());
    header_row(ui, actions, menu, |ui| {
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
    menu: Option<(&str, &[MenuEntry])>,
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
    let slots = actions.len() + usize::from(menu.is_some());
    let actions_width = slots as f32 * BUTTON_SIZE + slots.saturating_sub(1) as f32 * GAP;
    let actions_left = rect.right() - RIGHT_INSET - actions_width;
    let title_left = rect.left() + LEFT_INSET;
    let title_right = if slots == 0 {
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
            .inner;
        // egui shows a disabled widget's tooltip only through the disabled variant.
        let button = if action.enabled {
            button.on_hover_text(action.tooltip)
        } else {
            button.on_disabled_hover_text(action.tooltip)
        };
        if named(button, action.enabled, action.tooltip).clicked() {
            clicked = Some(action.command.clone());
        }
    }
    if let Some((name, entries)) = menu {
        let menu_rect = egui::Rect::from_min_size(
            egui::pos2(
                actions_left + actions.len() as f32 * (BUTTON_SIZE + GAP),
                rect.center().y - BUTTON_SIZE / 2.0,
            ),
            Vec2::splat(BUTTON_SIZE),
        );
        if let Some(command) = overflow_menu(&mut header_ui, menu_rect, name, entries) {
            clicked = Some(command);
        }
    }
    clicked
}

/// One entry of a `⋯` menu. Entries only name a `Command`; the menu never runs
/// logic, so palette, shortcuts and menus keep sharing command behavior.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MenuEntry {
    /// Non-interactive line naming what the menu acts on.
    Caption(String),
    Separator,
    Item(MenuItem),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MenuItem {
    pub(crate) label: String,
    pub(crate) shortcut: Option<&'static str>,
    pub(crate) command: Command,
    /// Why the item cannot be used right now; `None` when it can.
    pub(crate) disabled: Option<String>,
    /// Destructive or history-rewriting; drawn in the error color.
    pub(crate) danger: bool,
}

impl MenuItem {
    pub(crate) fn new(label: impl Into<String>, command: Command) -> Self {
        Self {
            label: label.into(),
            shortcut: None,
            command,
            disabled: None,
            danger: false,
        }
    }

    /// Disabled with a reason shown on hover when `reason` is `Some`.
    pub(crate) fn unless(mut self, reason: Option<impl Into<String>>) -> Self {
        self.disabled = reason.map(Into::into);
        self
    }

    pub(crate) fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub(crate) fn entry(self) -> MenuEntry {
        MenuEntry::Item(self)
    }
}

/// A frameless `⋯` button at `rect` that opens `entries`. Returns the chosen
/// command; the menu closes on click and disabled items explain themselves.
pub(crate) fn overflow_menu(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    name: &str,
    entries: &[MenuEntry],
) -> Option<Command> {
    let colors = crate::ui::theme::palette(ui.visuals());
    let mut chosen = None;
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::centered_and_justified(
                egui::Direction::LeftToRight,
            )),
        |ui| {
            let button = egui::Button::new(RichText::new(icon::ELLIPSIS).color(colors.muted))
                .frame(false)
                .min_size(rect.size());
            let (response, _) =
                egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                    ui.set_min_width(220.0);
                    chosen = menu_entries(ui, entries);
                });
            named(response.on_hover_text(name), true, name);
        },
    );
    chosen
}

/// Draws `entries` inside an open menu or context menu and returns the chosen
/// command, closing the menu when one is chosen.
pub(crate) fn menu_entries(ui: &mut egui::Ui, entries: &[MenuEntry]) -> Option<Command> {
    let colors = crate::ui::theme::palette(ui.visuals());
    let mut chosen = None;
    for entry in entries {
        match entry {
            MenuEntry::Caption(text) => {
                ui.label(RichText::new(text).size(11.0).color(colors.muted));
                ui.separator();
            }
            MenuEntry::Separator => {
                ui.separator();
            }
            MenuEntry::Item(item) => {
                let mut text = RichText::new(&item.label);
                if item.danger {
                    text = text.color(colors.error);
                }
                let mut button = egui::Button::new(text);
                if let Some(shortcut) = item.shortcut {
                    button = button.shortcut_text(shortcut);
                }
                let response = ui.add_enabled(item.disabled.is_none(), button);
                let response = match &item.disabled {
                    Some(reason) => response.on_disabled_hover_text(reason),
                    None => response,
                };
                if response.clicked() {
                    chosen = Some(item.command.clone());
                    ui.close();
                }
            }
        }
    }
    chosen
}

/// Gap above an empty state's content: generous in a tall pane, small enough in a
/// short split group that the title and its actions are not pushed out of view.
pub(crate) fn empty_state_top(ui: &egui::Ui) -> f32 {
    (ui.available_height() * 0.2).clamp(12.0, 130.0)
}

/// `text` cut with an ellipsis so it is no wider than `max_width` when drawn in
/// `font`; unchanged when it already fits. Measures glyphs, not characters.
pub(crate) fn ellipsize(ui: &egui::Ui, text: &str, font: &FontId, max_width: f32) -> String {
    let measure = |candidate: &str| {
        ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(candidate.to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
    };
    if measure(text) <= max_width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut low, mut high) = (0, chars.len());
    while low < high {
        let mid = (low + high).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if measure(&candidate) <= max_width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    chars[..low].iter().collect::<String>() + "…"
}

/// Lay `text` out on one line no wider than `max_width`, ending in an ellipsis
/// when it does not fit. Long branch, path and error text must never widen a pane.
pub(crate) fn fit_galley(
    ui: &egui::Ui,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping {
        max_width: max_width.max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

/// A small action drawn at the right of a `disclosure_row` while it is hovered.
pub(crate) struct RowAction {
    pub(crate) glyph: &'static str,
    pub(crate) tooltip: &'static str,
    pub(crate) enabled: bool,
}

pub(crate) struct DisclosureOutcome {
    /// The row itself was clicked: expand or collapse.
    pub(crate) toggled: bool,
    /// Index into the `actions` passed in.
    pub(crate) action: Option<usize>,
}

/// A collapsible group header: caret, title, count badge, and hover actions.
pub(crate) fn disclosure_row(
    ui: &mut egui::Ui,
    collapsed: bool,
    title: &str,
    count: usize,
    actions: &[RowAction],
) -> DisclosureOutcome {
    const BUTTON: f32 = 20.0;
    let colors = crate::ui::theme::palette(ui.visuals());
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, colors.hover);
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{title} {count}"))
    });
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.text(
        egui::pos2(rect.left() + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        if collapsed {
            icon::CARET_RIGHT
        } else {
            icon::CARET_DOWN
        },
        FontId::proportional(12.0),
        colors.muted,
    );
    let heading = painter.layout_no_wrap(title.to_owned(), FontId::proportional(11.0), colors.text);
    let heading_pos = egui::pos2(rect.left() + 20.0, rect.center().y - heading.size().y / 2.0);
    let heading_right = heading_pos.x + heading.size().x;
    painter.galley(heading_pos, heading, colors.text);
    if count > 0 {
        let badge =
            painter.layout_no_wrap(count.to_string(), FontId::proportional(10.0), colors.muted);
        let badge_rect = egui::Rect::from_min_size(
            egui::pos2(heading_right + 6.0, rect.center().y - 7.5),
            Vec2::new(badge.size().x + 10.0, 15.0),
        );
        painter.rect_filled(badge_rect, CornerRadius::same(8), colors.input);
        painter.galley(
            egui::pos2(
                badge_rect.center().x - badge.size().x / 2.0,
                badge_rect.center().y - badge.size().y / 2.0,
            ),
            badge,
            colors.muted,
        );
    }
    let mut action = None;
    // Keep the actions while the row or one of them has keyboard focus so Tab
    // can reach them; remembered because they only exist once shown.
    let focus_key = response.id.with("actions_focus");
    let focus_within = ui
        .data(|data| data.get_temp::<bool>(focus_key))
        .unwrap_or(false);
    let mut focused = response.has_focus();
    if hovered || focus_within {
        for (index, spec) in actions.iter().rev().enumerate() {
            let button_rect = egui::Rect::from_min_size(
                egui::pos2(
                    rect.right() - 4.0 - (index as f32 + 1.0) * BUTTON,
                    rect.center().y - BUTTON / 2.0,
                ),
                Vec2::splat(BUTTON),
            );
            let button = ui
                .add_enabled_ui(spec.enabled, |ui| {
                    ui.put(button_rect, egui::Button::new(spec.glyph).frame(false))
                })
                .inner;
            // egui shows tooltips on disabled widgets only via the disabled variant.
            let button = if spec.enabled {
                button.on_hover_text(spec.tooltip)
            } else {
                button.on_disabled_hover_text(spec.tooltip)
            };
            focused |= button.has_focus();
            if named(button, spec.enabled, spec.tooltip).clicked() {
                action = Some(actions.len() - 1 - index);
            }
        }
    }
    ui.data_mut(|data| data.insert_temp(focus_key, focused));
    DisclosureOutcome {
        toggled: action.is_none() && response.clicked(),
        action,
    }
}

pub(crate) fn surface(fill: Color32) -> Frame {
    Frame::default()
        .fill(fill)
        .corner_radius(CornerRadius::same(5))
}
/// Frame shared by every surface that floats above the editor: quick inputs,
/// prompts and dialogs. The drop shadow comes from the theme's window shadow.
pub(crate) fn floating_frame(ctx: &egui::Context) -> Frame {
    let colors = crate::ui::theme::palette(&ctx.style().visuals);
    Frame::window(&ctx.style())
        .fill(colors.panel)
        .corner_radius(CornerRadius::same(6))
        .stroke(Stroke::new(1.0_f32, colors.float_border))
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
    explorer_row_with_icon(ui, None, None, text, None, indent, selected)
}

/// Width reserved at a row's right edge for its Git status letter.
const BADGE_COLUMN: f32 = 20.0;

/// Spoken form of a Git status letter drawn in an Explorer row.
pub(crate) fn git_badge_label(badge: char) -> &'static str {
    match badge {
        '!' => "Conflict",
        'U' => "Untracked",
        'A' => "Added",
        'D' => "Deleted",
        'S' => "Staged",
        _ => "Modified",
    }
}

/// An explorer row with an optional disclosure caret and a coloured file or
/// folder icon ahead of the label, and a Git status letter in its own column at
/// the right edge. The name is cut with an ellipsis before it can reach the
/// badge. `indent` is where the row's first element starts.
pub(crate) fn explorer_row_with_icon(
    ui: &mut egui::Ui,
    caret: Option<&str>,
    file_icon: Option<crate::ui::file_icons::FileIcon>,
    text: String,
    badge: Option<char>,
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
    let spoken = match badge {
        Some(badge) => format!("{text}, {}", git_badge_label(badge)),
        None => text.clone(),
    };
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &spoken)
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
    let badge_width = if badge.is_some() { BADGE_COLUMN } else { 0.0 };
    let name = fit_galley(
        ui,
        &text,
        FontId::proportional(13.0),
        colors.text,
        rect.right() - badge_width - x - 6.0,
    );
    painter.galley(
        egui::pos2(x, rect.center().y - name.size().y / 2.0),
        name,
        colors.text,
    );
    if let Some(badge) = badge {
        painter.text(
            egui::pos2(rect.right() - BADGE_COLUMN / 2.0 - 2.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            badge,
            FontId::proportional(11.5),
            crate::ui::theme::change_color(&colors, badge),
        );
    }
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

    #[test]
    fn long_explorer_names_ellipsize_before_the_git_badge_at_every_sidebar_width() {
        let long = "a_very_long_nested_component_file_name_for_the_sidebar.rs";
        for width in [180.0_f32, 240.0, 600.0] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            ctx.enable_accesskit();
            let mut row = egui::Rect::NOTHING;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 120.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            row = explorer_row_with_icon(
                                ui,
                                None,
                                None,
                                long.into(),
                                Some('M'),
                                28,
                                false,
                            )
                            .rect;
                        });
                },
            );
            let mut name_right = None;
            let mut badge_left = None;
            for clipped in &output.shapes {
                if let egui::Shape::Text(text) = &clipped.shape {
                    let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                    match text.galley.text() {
                        "M" => badge_left = Some(rect.left()),
                        name if name.starts_with("a_very") => name_right = Some(rect.right()),
                        _ => {}
                    }
                }
            }
            let (name_right, badge_left) = (name_right.expect("name"), badge_left.expect("badge"));
            assert!(
                name_right <= badge_left,
                "{width}: name ends {name_right}, badge starts {badge_left}"
            );
            assert!(
                badge_left + 4.0 <= row.right(),
                "{width}: badge inside the row"
            );
            let spoken: Vec<_> = output
                .platform_output
                .accesskit_update
                .iter()
                .flat_map(|update| update.nodes.iter())
                .filter_map(|(_, node)| node.label().map(str::to_owned))
                .collect();
            assert!(
                spoken.iter().any(|label| label.ends_with(", Modified")),
                "{spoken:?}"
            );
        }
    }

    #[test]
    fn ellipsize_measures_rendered_width_not_characters() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let font = FontId::proportional(11.0);
        let measure = |ui: &egui::Ui, text: &str| {
            ui.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
                    .size()
                    .x
            })
        };
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert_eq!(ellipsize(ui, "Build", &font, 150.0), "Build");
                let wide = "W".repeat(40);
                let narrow = "i".repeat(40);
                let cut_wide = ellipsize(ui, &wide, &font, 100.0);
                let cut_narrow = ellipsize(ui, &narrow, &font, 100.0);
                for cut in [&cut_wide, &cut_narrow] {
                    assert!(cut.ends_with('…'));
                    assert!(measure(ui, cut) <= 100.0, "{cut} is {}", measure(ui, cut));
                }
                assert!(
                    cut_narrow.chars().count() > cut_wide.chars().count(),
                    "narrow glyphs keep more characters in the same width"
                );
                assert_eq!(ellipsize(ui, "anything", &font, 0.0), "…");
            });
        });
    }

    #[test]
    fn empty_state_gap_shrinks_in_short_panes() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut gaps = Vec::new();
        for height in [120.0_f32, 400.0, 1200.0] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, height),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| gaps.push(empty_state_top(ui)));
                },
            );
        }
        assert!(gaps[0] < gaps[1] && gaps[1] < gaps[2], "{gaps:?}");
        assert!(gaps[0] >= 12.0 && gaps[2] <= 130.0, "{gaps:?}");
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
