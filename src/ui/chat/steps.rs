//! The agent's working steps (notes, tool calls, plans, notices) drawn as a
//! chain, as VS Code's chat does: each step has an icon in a left gutter, a
//! thin rail joins the icons, and the text sits beside them. Once an answer
//! follows, the chain folds into one summary line that opens on click.
use super::{TranscriptPass, notice_card, plan_card, tool_detail, tool_glyph};
use crate::{
    icon,
    model::{
        chat_turns, patch_view,
        tool_groups::{self, Block},
    },
    ui::{
        markdown_render::{self, Host, Metrics},
        theme,
    },
};
use eframe::egui::{self, Color32, FontId, RichText, Sense, Stroke};
use lumen_agent_core::model::{ItemKind, ToolCall, ToolKind, ToolStatus, TranscriptItem};

/// Width of the icon column; the step's text starts after it.
const GUTTER: f32 = 24.0;
/// Size of a step's icon.
const ICON: f32 = 14.0;
/// Space between steps.
const STEP_GAP: f32 = 10.0;
/// The rail stops this far from each icon, so it reads as joining them.
const RAIL_CLEARANCE: f32 = 4.0;
/// Text size of a step line.
const STEP_TEXT: f32 = 12.5;

/// What sits in a step's icon slot.
enum Mark {
    Glyph(&'static str, Color32),
    Spinner(Color32),
    Dot(Color32),
}

/// A run of working steps. `foldable` steps (an answer came after them and the
/// turn is over) start folded into a summary line.
pub(super) fn steps(
    ui: &mut egui::Ui,
    items: &[TranscriptItem],
    blocks: &[Block],
    foldable: bool,
    pass: &mut TranscriptPass,
) {
    let Some(first) = first_index(blocks) else {
        return;
    };
    let key = egui::Id::new(("chat_steps", pass.session.0, &items[first].id.0));
    if foldable {
        let mut open = ui.data(|data| data.get_temp::<bool>(key)).unwrap_or(false);
        let (title, detail) = chat_turns::steps_summary(items, blocks);
        if summary_line(ui, &title, &detail, open, pass.colors).clicked() {
            open = !open;
            ui.data_mut(|data| data.insert_temp(key, open));
        }
        if !open {
            return;
        }
        ui.add_space(6.0);
    }
    chain(ui, items, blocks, pass);
}

fn first_index(blocks: &[Block]) -> Option<usize> {
    blocks.first().and_then(|block| match block {
        Block::Item(index) => Some(*index),
        Block::Explored(group) => group.first().copied(),
    })
}

/// "▸ Worked through 6 steps · read 3 files, ran 2 commands", clickable.
fn summary_line(
    ui: &mut egui::Ui,
    title: &str,
    detail: &str,
    open: bool,
    colors: &theme::Palette,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, colors.control_hover);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            3.0,
            Stroke::new(1.0_f32, colors.accent),
            egui::StrokeKind::Inside,
        );
    }
    let painter = ui.painter().with_clip_rect(rect);
    let caret = if open {
        icon::CARET_DOWN
    } else {
        icon::CARET_RIGHT
    };
    painter.text(
        egui::pos2(rect.left() + GUTTER / 2.0 - 1.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        caret,
        FontId::proportional(11.0),
        colors.muted,
    );
    let mut job = egui::text::LayoutJob::default();
    job.append(
        title,
        0.0,
        egui::TextFormat::simple(FontId::proportional(STEP_TEXT), colors.muted),
    );
    if !detail.is_empty() {
        job.append(
            &format!("  ·  {detail}"),
            0.0,
            egui::TextFormat::simple(FontId::proportional(theme::META_SIZE), colors.muted),
        );
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - GUTTER - 4.0);
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    painter.galley(
        egui::pos2(
            rect.left() + GUTTER,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        colors.muted,
    );
    let spoken = format!("{title}. {detail}");
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, &spoken)
    });
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if open {
            "Hide the steps"
        } else {
            "Show every step"
        })
}

/// The steps one under another, each with its icon, joined by a rail.
fn chain(ui: &mut egui::Ui, items: &[TranscriptItem], blocks: &[Block], pass: &mut TranscriptPass) {
    let colors = *pass.colors;
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&FontId::proportional(STEP_TEXT)));
    let mut icons: Vec<egui::Rect> = Vec::new();
    for (position, block) in blocks.iter().enumerate() {
        if position > 0 {
            ui.add_space(STEP_GAP);
        }
        let mark = mark(items, block, pass.accent, &colors);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let (gutter, _) =
                ui.allocate_exact_size(egui::vec2(GUTTER, row_height), Sense::hover());
            let icon = egui::Rect::from_center_size(
                egui::pos2(
                    gutter.left() + ICON / 2.0 + 1.0,
                    gutter.top() + row_height / 2.0,
                ),
                egui::Vec2::splat(ICON),
            );
            paint_mark(ui, icon, &mark);
            icons.push(icon);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                step(ui, items, block, pass);
            });
        });
    }
    let rail = Stroke::new(1.0_f32, colors.float_border);
    for pair in icons.windows(2) {
        let (top, bottom) = (
            pair[0].bottom() + RAIL_CLEARANCE,
            pair[1].top() - RAIL_CLEARANCE,
        );
        if bottom > top {
            ui.painter().vline(pair[0].center().x, top..=bottom, rail);
        }
    }
}

fn mark(items: &[TranscriptItem], block: &Block, accent: Color32, colors: &theme::Palette) -> Mark {
    let index = match block {
        Block::Explored(_) => return Mark::Glyph(icon::MAGNIFYING_GLASS, colors.muted),
        Block::Item(index) => *index,
    };
    match &items[index].kind {
        ItemKind::Tool(call) => match call.status {
            ToolStatus::Pending | ToolStatus::Running => Mark::Spinner(accent),
            ToolStatus::Failed => Mark::Glyph(icon::ERROR, colors.error),
            ToolStatus::Declined | ToolStatus::Cancelled => {
                Mark::Glyph(tool_glyph(call.kind), colors.warning)
            }
            ToolStatus::Completed => Mark::Glyph(tool_glyph(call.kind), colors.muted),
        },
        ItemKind::Plan(_) => Mark::Glyph(icon::OUTPUT, colors.muted),
        ItemKind::Error(_) => Mark::Glyph(icon::ERROR, colors.error),
        ItemKind::Agent { .. } | ItemKind::Notice(_) | ItemKind::User(_) => Mark::Dot(colors.muted),
    }
}

fn paint_mark(ui: &egui::Ui, rect: egui::Rect, mark: &Mark) {
    match mark {
        Mark::Glyph(glyph, color) => {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                *glyph,
                FontId::proportional(12.0),
                *color,
            );
        }
        Mark::Spinner(color) => {
            egui::Spinner::new()
                .size(ICON - 2.0)
                .color(*color)
                .paint_at(ui, rect.shrink(1.0));
        }
        Mark::Dot(color) => {
            ui.painter().circle_filled(rect.center(), 2.5, *color);
        }
    }
}

/// The text of one step.
fn step(ui: &mut egui::Ui, items: &[TranscriptItem], block: &Block, pass: &mut TranscriptPass) {
    let colors = *pass.colors;
    let index = match block {
        Block::Explored(group) => {
            explored(ui, items, group, pass);
            return;
        }
        Block::Item(index) => *index,
    };
    let item = &items[index];
    match &item.kind {
        ItemKind::Agent { text, .. } => markdown_render::show(
            ui,
            text,
            Metrics::CHAT_NOTES,
            Host::Chat {
                session: pass.session,
                item: &item.id.0,
            },
            true,
            pass.commands,
        ),
        ItemKind::Tool(call) => tool(ui, &item.id.0, call, pass),
        ItemKind::Plan(entries) => plan_card(ui, entries, &colors),
        ItemKind::Error(text) => notice_card(ui, text, colors.error),
        ItemKind::Notice(text) | ItemKind::User(text) => {
            ui.add(
                egui::Label::new(RichText::new(text).size(STEP_TEXT).color(colors.muted)).wrap(),
            );
        }
    }
}

/// A tool call as one quiet line. A call with output or a patch opens on click;
/// a file edit reads "Edited name +3 −1".
fn tool(ui: &mut egui::Ui, item_id: &str, call: &ToolCall, pass: &mut TranscriptPass) {
    let colors = *pass.colors;
    let has_detail = !call.preview.is_empty() || call.patch.is_some();
    let key = egui::Id::new(("chat_tool", pass.session.0, item_id));
    let mut open = ui.data(|data| data.get_temp::<bool>(key)).unwrap_or(false);
    let format = |color: Color32| egui::TextFormat::simple(FontId::proportional(STEP_TEXT), color);
    let mut job = egui::text::LayoutJob::default();
    match edit_summary(call) {
        Some((name, added, removed)) => {
            job.append("Edited ", 0.0, format(colors.muted));
            job.append(&name, 0.0, format(colors.text));
            job.append(&format!("  +{added}"), 0.0, format(colors.success));
            job.append(&format!(" −{removed}"), 0.0, format(colors.error));
        }
        None => job.append(&call.title, 0.0, format(colors.muted)),
    }
    let state = match call.status {
        ToolStatus::Failed => Some(("failed", colors.error)),
        ToolStatus::Declined => Some(("declined", colors.warning)),
        ToolStatus::Cancelled => Some(("cancelled", colors.warning)),
        ToolStatus::Pending | ToolStatus::Running | ToolStatus::Completed => None,
    };
    if let Some((state, color)) = state {
        job.append(&format!("  ·  {state}"), 0.0, format(color));
    }
    if has_detail {
        let caret = if open {
            icon::CARET_DOWN
        } else {
            icon::CARET_RIGHT
        };
        job.append(
            &format!("  {caret}"),
            0.0,
            egui::TextFormat::simple(FontId::proportional(theme::META_SIZE), colors.muted),
        );
    }
    let sense = if has_detail {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.add(egui::Label::new(job).wrap().sense(sense));
    if has_detail {
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(if open {
                "Hide the output"
            } else {
                "Show the output"
            });
        if response.clicked() {
            open = !open;
            ui.data_mut(|data| data.insert_temp(key, open));
        }
    }
    if open && has_detail {
        egui::Frame::default()
            .fill(colors.panel)
            .corner_radius(4)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                tool_detail(ui, item_id, call, pass);
            });
    }
}

/// File name and line counts of a single-file edit, when its patch can be read.
fn edit_summary(call: &ToolCall) -> Option<(String, usize, usize)> {
    if call.kind != ToolKind::FileEdit {
        return None;
    }
    let files = patch_view::parse(call.patch.as_deref()?, &call.locations)?;
    let [file] = files.as_slice() else {
        return None;
    };
    Some((file.display_name().to_string(), file.added, file.removed))
}

/// A folded run of reads and searches: one line that lists them on click.
fn explored(
    ui: &mut egui::Ui,
    items: &[TranscriptItem],
    group: &[usize],
    pass: &mut TranscriptPass,
) {
    let colors = *pass.colors;
    let key = egui::Id::new(("chat_explored", pass.session.0, &items[group[0]].id.0));
    let mut open = ui.data(|data| data.get_temp::<bool>(key)).unwrap_or(false);
    let caret = if open {
        icon::CARET_DOWN
    } else {
        icon::CARET_RIGHT
    };
    let summary = tool_groups::summary(items, group);
    let response = ui
        .add(
            egui::Label::new(
                RichText::new(format!("{summary}  {caret}"))
                    .size(STEP_TEXT)
                    .color(colors.muted),
            )
            .wrap()
            .sense(Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if open {
            "Hide these calls"
        } else {
            "Show every call"
        });
    if response.clicked() {
        open = !open;
        ui.data_mut(|data| data.insert_temp(key, open));
    }
    if open {
        for &index in group {
            if let ItemKind::Tool(call) = &items[index].kind {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{}  {}", tool_glyph(call.kind), call.title))
                            .size(theme::META_SIZE)
                            .color(colors.muted),
                    )
                    .truncate(),
                )
                .on_hover_text(&call.title);
            }
        }
    }
}
