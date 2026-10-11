use crate::{EditError, EditorBuffer, Selection, SelectionSet, selection::grapheme_boundaries};

#[test]
fn grapheme_deletion_at_multiple_carets_restores_original_carets_on_undo() {
    let mut buffer = EditorBuffer::new("e\u{301} 👩‍💻\r\n".into());
    buffer
        .set_selections(vec![Selection::caret(3), Selection::caret(15)], 1)
        .unwrap();
    let before = buffer.selections().clone();
    buffer.delete_graphemes(crate::Direction::Backward).unwrap();
    assert_eq!(buffer.text(), " \r\n");
    assert!(buffer.undo());
    assert_eq!(buffer.text(), "e\u{301} 👩‍💻\r\n");
    assert_eq!(buffer.selections(), &before);
}

#[test]
fn horizontal_movement_and_shift_extension_use_complete_graphemes() {
    let mut buffer = EditorBuffer::new("e\u{301}👩‍💻".into());
    buffer.move_horizontal(crate::Direction::Forward, false);
    assert_eq!(buffer.selections().selections(), [Selection::caret(3)]);
    buffer.move_horizontal(crate::Direction::Forward, true);
    assert_eq!(buffer.selections().selections(), [range(3, 14)]);
    buffer.move_horizontal(crate::Direction::Backward, false);
    assert_eq!(buffer.selections().selections(), [Selection::caret(3)]);
    assert_eq!(buffer.revision(), 0);
    assert!(!buffer.is_dirty());
}

#[test]
fn overlapping_delete_targets_merge_without_double_deletion() {
    let mut buffer = EditorBuffer::new("ab\r\n".into());
    buffer
        .set_selections(vec![range(0, 2), Selection::caret(2)], 1)
        .unwrap();
    buffer.delete_graphemes(crate::Direction::Forward).unwrap();
    assert_eq!(buffer.text(), "\r\n");
    buffer.undo();
    buffer.set_selections(vec![Selection::caret(2)], 0).unwrap();
    buffer.delete_graphemes(crate::Direction::Forward).unwrap();
    assert_eq!(buffer.text(), "ab");
    assert!(!buffer.delete_graphemes(crate::Direction::Forward).unwrap());
}

fn range(anchor: usize, head: usize) -> Selection {
    Selection { anchor, head }
}

#[test]
fn normalization_merges_duplicates_overlaps_and_keeps_primary_direction() {
    let set = SelectionSet::new(
        "abcdef",
        vec![
            Selection::caret(6),
            range(1, 4),
            range(5, 2),
            Selection::caret(3),
            Selection::caret(6),
        ],
        2,
    )
    .unwrap();
    assert_eq!(set.selections(), [range(5, 1), Selection::caret(6)]);
    assert_eq!(set.primary_index(), 0);
}

#[test]
fn touching_ranges_remain_distinct_but_edge_carets_merge() {
    let set = SelectionSet::new("abcd", vec![range(0, 2), range(2, 4)], 1).unwrap();
    assert_eq!(set.selections(), [range(0, 2), range(2, 4)]);
    assert_eq!(set.primary_index(), 1);
    let set = SelectionSet::new("abcd", vec![Selection::caret(2), range(0, 2)], 0).unwrap();
    assert_eq!(set.selections(), [range(0, 2)]);
}

#[test]
fn boundaries_reject_utf8_combining_emoji_and_crlf_interiors() {
    for (text, offset) in [("é", 1), ("e\u{301}", 1), ("👩‍💻", 4), ("\r\n", 1), ("a", 2)] {
        assert_eq!(
            SelectionSet::new(text, vec![Selection::caret(offset)], 0),
            Err(EditError::InvalidBoundary(offset))
        );
    }
    assert_eq!(
        SelectionSet::new("", vec![], 0),
        Err(EditError::EmptySelections)
    );
    assert_eq!(
        SelectionSet::new("", vec![Selection::caret(0)], 1),
        Err(EditError::InvalidPrimary)
    );
    assert!(SelectionSet::new("", vec![Selection::caret(0)], 0).is_ok());
}

#[test]
fn invalid_selection_change_is_non_mutating() {
    let mut buffer = EditorBuffer::new("é".into());
    let before = buffer.selections().clone();
    assert!(buffer.set_selections(vec![Selection::caret(1)], 0).is_err());
    assert_eq!(buffer.selections(), &before);
    assert_eq!(buffer.revision(), 0);
    assert!(!buffer.is_dirty());
}

#[test]
fn multi_location_typing_undoes_and_redoes_as_one_action() {
    let mut buffer = EditorBuffer::new("one two three".into());
    buffer
        .set_selections(vec![range(8, 13), range(0, 3)], 0)
        .unwrap();
    let before = buffer.selections().clone();
    assert!(buffer.replace_selections(&["🌍"]).unwrap());
    assert_eq!(buffer.text(), "🌍 two 🌍");
    assert_eq!(
        buffer.selections().selections(),
        [Selection::caret(4), Selection::caret(13)]
    );
    assert_eq!(buffer.selections().primary_index(), 1);
    let after = buffer.selections().clone();
    assert!(buffer.is_dirty());
    assert!(buffer.undo());
    assert_eq!(buffer.text(), "one two three");
    assert_eq!(buffer.selections(), &before);
    assert!(!buffer.is_dirty());
    assert!(!buffer.undo());
    assert!(buffer.redo());
    assert_eq!(buffer.text(), "🌍 two 🌍");
    assert_eq!(buffer.selections(), &after);
    assert_eq!(buffer.revision(), 3);
    assert!(!buffer.redo());
}

#[test]
fn distributed_fragments_follow_canonical_document_order_and_preserve_endings() {
    let mut buffer = EditorBuffer::new("é\r\n漢字".into());
    buffer
        .set_selections(vec![range(4, 10), range(0, 2)], 0)
        .unwrap();
    buffer.replace_selections(&["first", "second"]).unwrap();
    assert_eq!(buffer.text(), "first\r\nsecond");
    assert_eq!(buffer.selections().primary_index(), 1);
}

#[test]
fn fragment_mismatch_does_not_destroy_redo_or_checkpoint() {
    let mut buffer = EditorBuffer::new("ab".into());
    buffer.replace_selections(&["x"]).unwrap();
    buffer.undo();
    let before = buffer.selections().clone();
    assert_eq!(
        buffer.replace_selections(&["x", "y"]),
        Err(EditError::FragmentCount)
    );
    assert_eq!(buffer.selections(), &before);
    assert_eq!(buffer.revision(), 2);
    assert_eq!(buffer.text(), "ab");
    assert!(!buffer.is_dirty());
    assert!(buffer.redo());
    assert_eq!(buffer.text(), "xab");
}

#[test]
fn overlapping_selection_replacement_is_applied_once() {
    let mut buffer = EditorBuffer::new("abcdef".into());
    buffer
        .set_selections(vec![range(1, 4), range(3, 5), Selection::caret(4)], 1)
        .unwrap();
    buffer.replace_selections(&["X"]).unwrap();
    assert_eq!(buffer.text(), "aXf");
    assert_eq!(buffer.selections().selections(), [Selection::caret(2)]);
}

#[test]
fn duplicate_carets_insert_once_and_adjacent_replacements_keep_both_fragments() {
    let mut buffer = EditorBuffer::new("ab".into());
    buffer
        .set_selections(vec![Selection::caret(0), Selection::caret(0)], 1)
        .unwrap();
    buffer.replace_selections(&["X"]).unwrap();
    assert_eq!(buffer.text(), "Xab");
    buffer.undo();
    buffer
        .set_selections(vec![range(0, 1), range(1, 2)], 0)
        .unwrap();
    buffer.replace_selections(&["X", "Y"]).unwrap();
    assert_eq!(buffer.text(), "XY");
}

#[test]
fn joining_graphemes_snap_final_carets_without_splitting_a_cluster() {
    let mut buffer = EditorBuffer::new("\u{301}b".into());
    buffer.replace_selections(&["a"]).unwrap();
    assert_eq!(buffer.text(), "a\u{301}b");
    assert_eq!(buffer.selections().selections(), [Selection::caret(3)]);
    buffer.undo();
    assert_eq!(buffer.text(), "\u{301}b");
    assert_eq!(buffer.selections().selections(), [Selection::caret(0)]);
}

#[test]
fn deletion_merges_resulting_carets_and_restores_full_set_on_undo() {
    let mut buffer = EditorBuffer::new("abc".into());
    buffer
        .set_selections(vec![range(0, 1), range(1, 2), range(2, 3)], 2)
        .unwrap();
    let before = buffer.selections().clone();
    buffer.replace_selections(&[""]).unwrap();
    assert_eq!(buffer.text(), "");
    assert_eq!(buffer.selections().selections(), [Selection::caret(0)]);
    assert_eq!(buffer.selections().primary_index(), 0);
    buffer.undo();
    assert_eq!(buffer.selections(), &before);
}

#[test]
fn no_op_preserves_history_selection_and_redo_branch() {
    let mut buffer = EditorBuffer::new("abc".into());
    buffer.replace_selections(&["X"]).unwrap();
    buffer.undo();
    buffer.set_selections(vec![range(0, 3)], 0).unwrap();
    let before = buffer.selections().clone();
    assert!(!buffer.replace_selections(&["abc"]).unwrap());
    assert_eq!(buffer.selections(), &before);
    assert_eq!(buffer.revision(), 2);
    assert!(buffer.redo());
}

#[test]
fn saved_checkpoint_is_independent_of_undo_history() {
    let mut buffer = EditorBuffer::new("abc".into());
    buffer.replace_selections(&["X"]).unwrap();
    buffer.mark_saved();
    assert!(!buffer.is_dirty());
    buffer.undo();
    assert!(buffer.is_dirty());
    buffer.redo();
    assert!(!buffer.is_dirty());
    buffer.undo();
    buffer.replace_selections(&["Y"]).unwrap();
    assert!(buffer.is_dirty());
    assert!(!buffer.redo());
}

#[test]
fn history_limit_does_not_evict_current_or_saved_text() {
    let mut buffer = EditorBuffer::new(String::new());
    for _ in 0..110 {
        buffer.replace_selections(&["x"]).unwrap();
    }
    buffer.mark_saved();
    let mut count = 0;
    while buffer.undo() {
        count += 1;
    }
    assert_eq!(count, 100);
    assert_eq!(buffer.text(), "xxxxxxxxxx");
    assert!(buffer.is_dirty());
    while buffer.redo() {}
    assert_eq!(buffer.text().len(), 110);
    assert!(!buffer.is_dirty());
}

#[test]
fn exhaustive_small_unicode_fixtures_round_trip_text_and_selection_sets() {
    for text in ["", "abc", "é\r\n漢字", "e\u{301}👩‍💻🇮🇳"] {
        let boundaries = grapheme_boundaries(text);
        for &first in &boundaries {
            for &second in &boundaries {
                for fragment in ["X", "", "🌍\r\n", "\u{301}", "\u{200d}👩"] {
                    let mut buffer = EditorBuffer::new(text.into());
                    buffer
                        .set_selections(vec![Selection::caret(first), Selection::caret(second)], 1)
                        .unwrap();
                    let before = buffer.selections().clone();
                    if buffer.replace_selections(&[fragment]).unwrap() {
                        let changed = buffer.text().to_owned();
                        let after = buffer.selections().clone();
                        assert!(buffer.undo());
                        assert_eq!(buffer.text(), text);
                        assert_eq!(buffer.selections(), &before);
                        assert!(buffer.redo());
                        assert_eq!(buffer.text(), changed);
                        assert_eq!(buffer.selections(), &after);
                    } else {
                        assert_eq!(buffer.text(), text);
                        assert_eq!(buffer.selections(), &before);
                        assert!(!buffer.undo());
                    }
                }
            }
        }
    }
}
