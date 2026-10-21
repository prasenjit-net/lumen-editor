use crate::TextGeometry;
use egui::{
    Response, Ui,
    accesskit::{Action, ActionData, ActionRequest, Role},
};
use lumen_editor_core::{EditorBuffer, Selection};

pub(crate) fn publish(
    ui: &Ui,
    response: &Response,
    geometry: &TextGeometry,
    buffer: &EditorBuffer,
) {
    let text = geometry.visual_text();
    response.widget_info(|| {
        egui::WidgetInfo::text_edit(ui.is_enabled(), text, text, "Editor prototype")
    });
    let primary = buffer.selections().selections()[buffer.selections().primary_index()];
    let transform = ui
        .ctx()
        .layer_transform_to_global(ui.layer_id())
        .unwrap_or_default();
    if geometry.shaped.is_some() {
        publish_shaped(ui, response, geometry, buffer);
        return;
    }
    egui::text_selection::accesskit_text::update_accesskit_for_text_widget(
        ui.ctx(),
        response.id,
        Some(egui::text::CCursorRange::two(
            geometry.cursor(primary.anchor),
            geometry.cursor(primary.head),
        )),
        Role::MultilineTextInput,
        transform * egui::emath::TSTransform::from_translation(geometry.origin.to_vec2()),
        geometry.galley(),
    );
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_label("Multi-cursor editor prototype");
        node.set_description(format!("{} cursors. Accessibility exposes the primary selection; setting a selection collapses secondary cursors.", buffer.selections().selections().len()));
        node.add_action(Action::SetTextSelection);
        node.add_action(Action::SetValue);
    });
}

fn publish_shaped(ui: &Ui, response: &Response, geometry: &TextGeometry, buffer: &EditorBuffer) {
    use egui::accesskit::{NodeId, TextDirection, TextPosition, TextSelection};
    let shaped = geometry.shaped.as_ref().unwrap();
    let primary = buffer.selections().selections()[buffer.selections().primary_index()];
    let position = |byte| {
        let cursor = shaped.cursor(geometry.visual_byte(geometry.cursor(byte).index));
        TextPosition {
            node: NodeId(response.id.with(cursor.line).value()),
            character_index: geometry.visual_text()
                [shaped.starts[cursor.line]..shaped.starts[cursor.line] + cursor.index]
                .chars()
                .count(),
        }
    };
    let mut enabled = false;
    ui.ctx().accesskit_node_builder(response.id, |node| {
        enabled = true;
        node.set_role(Role::MultilineTextInput);
        node.set_label("Lumen editor");
        node.set_description(format!("{} cursors; accessible selection changes collapse secondary cursors. Composition is transient.",buffer.selections().selections().len()));
        node.set_text_selection(TextSelection {anchor:position(primary.anchor),focus:position(primary.head)});
        node.set_children(shaped.starts.iter().enumerate().map(|(line,_)|NodeId(response.id.with(line).value())).collect::<Vec<_>>());
        node.add_action(Action::SetTextSelection);
        node.add_action(Action::SetValue);
    });
    // egui invokes the builder only while AccessKit is enabled. Avoid computing
    // character geometry for the entire document when no tree is requested.
    if !enabled {
        return;
    }
    let transform = ui
        .ctx()
        .layer_transform_to_global(ui.layer_id())
        .unwrap_or_default();
    let rows = geometry.row_rects();
    let text = geometry.visual_text();
    let character_carets = shaped.character_carets(text);
    let mut character = 0;
    for (line, start) in shaped.starts.iter().copied().enumerate() {
        let end = shaped.starts.get(line + 1).copied().unwrap_or(text.len());
        let value = &text[start..end];
        let id = response.id.with(line);
        let rect = transform * rows.get(line).copied().unwrap_or(response.rect);
        let positions: Vec<_> = value
            .chars()
            .enumerate()
            .map(|(offset, _)| {
                let left = character_carets[character + offset].x;
                let right = character_carets[character + offset + 1].x;
                (left.min(right), (right - left).abs())
            })
            .collect();
        ui.ctx().accesskit_node_builder(id, |node| {
            node.set_role(Role::TextRun);
            node.set_value(value.to_owned());
            node.set_bounds(egui::accesskit::Rect {
                x0: rect.left().into(),
                y0: rect.top().into(),
                x1: rect.right().into(),
                y1: rect.bottom().into(),
            });
            node.set_text_direction(if shaped.is_rtl(line) {
                TextDirection::RightToLeft
            } else {
                TextDirection::LeftToRight
            });
            node.set_character_lengths(
                value
                    .chars()
                    .map(|c| c.len_utf8() as u8)
                    .collect::<Vec<_>>(),
            );
            node.set_character_positions(positions.iter().map(|(x, _)| *x).collect::<Vec<_>>());
            node.set_character_widths(
                positions
                    .iter()
                    .map(|(_, width)| *width)
                    .collect::<Vec<_>>(),
            );
        });
        character += value.chars().count();
    }
    // Direct public node construction attaches unregistered nodes to egui's
    // root. Reparent only our rows, retaining every unrelated root child.
    let row_ids: std::collections::HashSet<_> = shaped
        .starts
        .iter()
        .enumerate()
        .map(|(line, _)| NodeId(response.id.with(line).value()))
        .collect();
    ui.ctx()
        .accesskit_node_builder(egui::accesskit_root_id(), |root| {
            let children = root
                .children()
                .iter()
                .copied()
                .filter(|child| !row_ids.contains(child))
                .collect::<Vec<_>>();
            root.set_children(children);
        });
}

pub(crate) fn apply_action(
    ui: &Ui,
    response: &Response,
    request: &ActionRequest,
    geometry: &TextGeometry,
    buffer: &mut EditorBuffer,
) -> bool {
    if request.target != egui::accesskit::NodeId(response.id.value()) || !ui.is_enabled() {
        return false;
    }
    match (&request.action, &request.data) {
        (Action::SetTextSelection, Some(ActionData::SetTextSelection(selection))) => {
            let Some(anchor) = geometry.accessibility_byte(response.id, &selection.anchor) else {
                return false;
            };
            let Some(head) = geometry.accessibility_byte(response.id, &selection.focus) else {
                return false;
            };
            if buffer
                .set_selections(vec![Selection { anchor, head }], 0)
                .is_err()
            {
                return false;
            }
            response.request_focus();
            true
        }
        (Action::SetValue, Some(ActionData::Value(text))) => buffer.replace_document(text).is_ok(),
        _ => false,
    }
}
