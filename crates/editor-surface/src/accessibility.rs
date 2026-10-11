use crate::TextGeometry;
use egui::{
    Pos2, Response, Ui,
    accesskit::{Action, ActionData, ActionRequest, Role},
};
use lumen_editor_core::{EditorBuffer, Selection};

pub(crate) fn publish(
    ui: &Ui,
    response: &Response,
    geometry: &TextGeometry,
    buffer: &EditorBuffer,
) {
    let text = geometry.galley.text();
    response.widget_info(|| {
        egui::WidgetInfo::text_edit(ui.is_enabled(), text, text, "Editor prototype")
    });
    let primary = buffer.selections().selections()[buffer.selections().primary_index()];
    let transform = ui
        .ctx()
        .layer_transform_to_global(ui.layer_id())
        .unwrap_or_default();
    egui::text_selection::accesskit_text::update_accesskit_for_text_widget(
        ui.ctx(),
        response.id,
        Some(egui::text::CCursorRange::two(
            geometry.cursor(primary.anchor),
            geometry.cursor(primary.head),
        )),
        Role::MultilineTextInput,
        transform * egui::emath::TSTransform::from_translation(geometry.origin.to_vec2()),
        &geometry.galley,
    );
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_label("Multi-cursor editor prototype");
        node.set_description(format!("{} cursors. Accessibility exposes the primary selection; setting a selection collapses secondary cursors.", buffer.selections().selections().len()));
        node.add_action(Action::SetTextSelection);
        node.add_action(Action::SetValue);
    });
}

pub(crate) fn apply_action(
    ui: &Ui,
    response: &Response,
    request: &ActionRequest,
    origin: Pos2,
    buffer: &mut EditorBuffer,
) -> bool {
    if request.target != egui::accesskit::NodeId(response.id.value()) || !ui.is_enabled() {
        return false;
    }
    match (&request.action, &request.data) {
        (Action::SetTextSelection, Some(ActionData::SetTextSelection(selection))) => {
            let geometry = TextGeometry::layout(ui, buffer.text(), origin);
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
