//! Selection orchestration for shaped visual arrows; the core owns state/history.
use crate::TextGeometry;
use lumen_editor_core::{EditorBuffer, Selection, SelectionAffinity};

pub(crate) fn move_horizontal(
    buffer: &mut EditorBuffer,
    geometry: &TextGeometry,
    right: bool,
    extend: bool,
) {
    let set = buffer.selections();
    let selections = set
        .selections()
        .iter()
        .zip(set.affinities())
        .map(|(selection, affinity)| {
            let target = if !extend && !selection.range().is_empty() {
                let anchor = geometry
                    .caret_with_affinity(selection.anchor, affinity.anchor)
                    .min;
                let head = geometry
                    .caret_with_affinity(selection.head, affinity.head)
                    .min;
                let anchor_first = anchor.y < head.y || (anchor.y == head.y && anchor.x < head.x);
                if right == anchor_first {
                    (selection.head, affinity.head)
                } else {
                    (selection.anchor, affinity.anchor)
                }
            } else {
                geometry
                    .visual_step(selection.head, affinity.head, right)
                    .unwrap_or((selection.head, affinity.head))
            };
            (
                Selection {
                    anchor: if extend { selection.anchor } else { target.0 },
                    head: target.0,
                },
                SelectionAffinity {
                    anchor: if extend { affinity.anchor } else { target.1 },
                    head: target.1,
                },
            )
        })
        .collect();
    buffer
        .set_selections_with_affinity(selections, set.primary_index())
        .unwrap();
}
