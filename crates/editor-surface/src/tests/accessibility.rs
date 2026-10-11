use super::*;
use egui::accesskit::{
    Action, ActionData, ActionRequest, NodeId, Role, TextPosition, TextSelection,
};

fn request(action: Action, data: ActionData, target: Id) -> Event {
    Event::AccessKitActionRequest(ActionRequest {
        action,
        data: Some(data),
        target: NodeId(target.value()),
    })
}

fn position(row: usize, column: usize) -> TextPosition {
    TextPosition {
        node: NodeId(Id::new("test_surface").with(row).value()),
        character_index: column,
    }
}

#[test]
fn tree_publishes_editable_text_rows_and_primary_selection() {
    let (mut surface, mut buffer, ctx) = fixture("é\r\n漢字");
    ctx.enable_accesskit();
    buffer
        .set_selections(
            vec![
                Selection::caret(0),
                Selection {
                    anchor: 4,
                    head: 10,
                },
            ],
            1,
        )
        .unwrap();
    let output = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            surface.show(ui, Id::new("test_surface"), &mut buffer);
        });
    });
    let tree = output.platform_output.accesskit_update.unwrap();
    let node = tree
        .nodes
        .iter()
        .find(|(id, _)| *id == NodeId(Id::new("test_surface").value()))
        .unwrap()
        .1
        .clone();
    assert_eq!(node.role(), Role::MultilineTextInput);
    assert!(node.supports_action(Action::SetTextSelection));
    assert!(node.supports_action(Action::SetValue));
    assert_eq!(node.text_selection().unwrap().anchor, position(1, 0));
    assert_eq!(node.text_selection().unwrap().focus, position(1, 2));
    let values: Vec<_> = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::TextRun)
        .map(|(_, node)| node.value().unwrap())
        .collect();
    assert!(values.contains(&"é\n"));
    assert!(values.contains(&"漢字"));
}

#[test]
fn accessible_selection_maps_crlf_rows_to_source_bytes_and_collapses_secondaries() {
    let (mut surface, mut buffer, ctx) = fixture("é\r\n漢字");
    buffer
        .set_selections(vec![Selection::caret(0), Selection::caret(4)], 1)
        .unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![request(
            Action::SetTextSelection,
            ActionData::SetTextSelection(TextSelection {
                anchor: position(1, 0),
                focus: position(1, 2),
            }),
            Id::new("test_surface"),
        )],
        500.0,
    );
    assert_eq!(
        buffer.selections().selections(),
        [Selection {
            anchor: 4,
            head: 10
        }]
    );
    assert_eq!(buffer.revision(), 0);
}

#[test]
fn foreign_rows_out_of_bounds_and_grapheme_interior_actions_are_rejected() {
    for focus in [position(99, 0), position(0, 100), position(0, 1)] {
        let (mut surface, mut buffer, ctx) = fixture("e\u{301}");
        let before = buffer.selections().clone();
        frame(
            &mut surface,
            &mut buffer,
            &ctx,
            vec![request(
                Action::SetTextSelection,
                ActionData::SetTextSelection(TextSelection {
                    anchor: position(0, 0),
                    focus,
                }),
                Id::new("test_surface"),
            )],
            500.0,
        );
        assert_eq!(buffer.selections(), &before);
    }
    let (mut surface, mut buffer, ctx) = fixture("abc");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![request(
            Action::SetValue,
            ActionData::Value("wrong".into()),
            Id::new("other_document"),
        )],
        500.0,
    );
    assert_eq!(buffer.text(), "abc");
}

#[test]
fn accessible_value_replacement_uses_one_transaction_and_restores_original_set_on_undo() {
    let (mut surface, mut buffer, ctx) = fixture("abc");
    buffer
        .set_selections(vec![Selection::caret(0), Selection::caret(2)], 1)
        .unwrap();
    let before = buffer.selections().clone();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![request(
            Action::SetValue,
            ActionData::Value("é\r\n漢".into()),
            Id::new("test_surface"),
        )],
        500.0,
    );
    assert_eq!(buffer.text(), "é\r\n漢");
    assert_eq!(buffer.revision(), 1);
    buffer.undo();
    assert_eq!(buffer.text(), "abc");
    assert_eq!(buffer.selections(), &before);
    assert!(!buffer.is_dirty());
}
