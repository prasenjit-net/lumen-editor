use crate::{EditError, EditorBuffer, Selection};

#[test]
fn word_seeding_keeps_combining_marks_with_their_base() {
    let mut buffer = EditorBuffer::new("é é".into());
    assert!(buffer.select_all_occurrences());
    assert_eq!(
        buffer.selections().selections(),
        &[
            Selection { anchor: 0, head: 3 },
            Selection { anchor: 4, head: 7 }
        ]
    );
}

#[test]
fn next_occurrence_seeds_word_wraps_and_does_not_dirty_or_create_history() {
    let mut buffer = EditorBuffer::new("one two one one".into());
    buffer.set_selections(vec![Selection::caret(9)], 0).unwrap();
    assert!(buffer.select_next_occurrence());
    assert_eq!(
        buffer.selections().selections(),
        &[Selection {
            anchor: 8,
            head: 11
        }]
    );
    assert!(buffer.select_next_occurrence());
    assert!(buffer.select_next_occurrence());
    assert!(!buffer.select_next_occurrence());
    assert_eq!(buffer.selections().selections().len(), 3);
    assert_eq!(buffer.selections().primary_index(), 0);
    assert_eq!(buffer.revision(), 0);
    assert!(!buffer.is_dirty());
    assert!(!buffer.undo());
}

#[test]
fn all_occurrences_are_literal_grapheme_safe_and_keep_primary_location() {
    let mut buffer = EditorBuffer::new("漢 漢 é e".into());
    buffer
        .set_selections(vec![Selection { anchor: 4, head: 7 }], 0)
        .unwrap();
    assert!(buffer.select_all_occurrences());
    assert_eq!(buffer.selections().primary_index(), 1);
    assert_eq!(buffer.selections().selections().len(), 2);
    buffer
        .set_selections(
            vec![Selection {
                anchor: 12,
                head: 13,
            }],
            0,
        )
        .unwrap();
    assert!(!buffer.select_all_occurrences()); // Do not select the e inside é.
    buffer.set_selections(vec![Selection::caret(3)], 0).unwrap();
    // Word selection at the previous word's right boundary follows that word.
    assert!(buffer.select_next_occurrence());
}

#[test]
fn distributed_clipboard_requires_explicit_matching_metadata_and_count() {
    let mut source = EditorBuffer::new("漢\r\n👩‍💻".into());
    let len = source.text().len();
    source
        .set_selections(
            vec![
                Selection { anchor: 0, head: 3 },
                Selection {
                    anchor: 5,
                    head: len,
                },
            ],
            1,
        )
        .unwrap();
    let clipboard = source.copy_selections().unwrap();
    let before = source.selections().clone();
    let mut destination = EditorBuffer::new("a b".into());
    destination
        .set_selections(
            vec![
                Selection { anchor: 0, head: 1 },
                Selection { anchor: 2, head: 3 },
            ],
            0,
        )
        .unwrap();
    destination
        .paste(clipboard.text(), Some(&clipboard))
        .unwrap();
    assert_eq!(destination.text(), "漢 👩‍💻");
    assert!(destination.undo());
    destination.paste("outside", Some(&clipboard)).unwrap();
    assert_eq!(destination.text(), "outside outside");
    assert!(source.cut_selections().unwrap().is_some());
    assert_eq!(source.text(), "\r\n");
    source.undo();
    assert_eq!(source.selections(), &before);
    destination.undo();
    destination
        .set_selections(vec![Selection::caret(0)], 0)
        .unwrap();
    destination
        .paste(clipboard.text(), Some(&clipboard))
        .unwrap();
    assert_eq!(destination.text(), "漢\n👩‍💻a b");
}

#[test]
fn empty_document_line_copy_cut_and_empty_paste_are_non_mutating() {
    let mut buffer = EditorBuffer::new("".into());
    assert_eq!(buffer.copy_selections().unwrap().text(), "\n");
    assert!(buffer.cut_selections().unwrap().is_some());
    assert!(!buffer.paste("", None).unwrap());
    assert!(!buffer.undo());
    assert!(!buffer.is_dirty());
}

#[test]
fn logical_line_paste_preserves_crlf_column_and_restores_original_carets() {
    let mut buffer = EditorBuffer::new("é 🌍\r\nnext".into());
    buffer.set_selections(vec![Selection::caret(3)], 0).unwrap();
    let metadata = buffer.copy_selections().unwrap();
    assert_eq!(metadata.text(), "é 🌍\r\n");
    let before = buffer.selections().clone();
    buffer.paste(metadata.text(), Some(&metadata)).unwrap();
    assert_eq!(buffer.text(), "é 🌍\r\né 🌍\r\nnext");
    assert_eq!(buffer.selections().selections(), &[Selection::caret(12)]);
    buffer.undo();
    assert_eq!(buffer.selections(), &before);
    assert!(!buffer.is_dirty());
}

#[test]
fn multi_line_cut_deduplicates_lines_and_handles_final_eof_terminator() {
    let mut buffer = EditorBuffer::new("a\r\nb\r\nc".into());
    buffer
        .set_selections(
            vec![
                Selection::caret(0),
                Selection::caret(1),
                Selection::caret(6),
            ],
            2,
        )
        .unwrap();
    let before = buffer.selections().clone();
    let clipboard = buffer.cut_selections().unwrap().unwrap();
    assert_eq!(clipboard.text(), "a\r\nc\r\n");
    assert_eq!(buffer.text(), "b");
    buffer.undo();
    assert_eq!(buffer.selections(), &before);
}

#[test]
fn mixed_selections_clipboard_retains_empty_fragments_without_expanding_lines() {
    let mut buffer = EditorBuffer::new("a name".into());
    buffer
        .set_selections(
            vec![Selection::caret(0), Selection { anchor: 2, head: 6 }],
            1,
        )
        .unwrap();
    let before = buffer.selections().clone();
    let clipboard = buffer.cut_selections().unwrap().unwrap();
    assert_eq!(clipboard.text(), "\nname");
    assert_eq!(buffer.text(), "a ");
    buffer.undo();
    assert_eq!(buffer.selections(), &before);
}

#[test]
fn word_movement_and_deletion_keep_unicode_boundaries_and_original_undo_set() {
    use crate::Direction;
    let mut buffer = EditorBuffer::new("é\r\n漢字 tail".into());
    buffer.move_word(Direction::Forward, false);
    assert_eq!(buffer.selections().selections(), &[Selection::caret(5)]);
    buffer.move_word(Direction::Forward, true);
    let before = buffer.selections().clone();
    buffer.delete_word(Direction::Backward).unwrap();
    buffer.undo();
    assert_eq!(buffer.selections(), &before);
    assert!(!buffer.is_dirty());
}

#[test]
fn indentation_deduplicates_lines_preserves_direction_crlf_and_undo() {
    let mut buffer = EditorBuffer::new("a\r\nb\r\nc".into());
    buffer
        .set_selections(
            vec![Selection { anchor: 4, head: 0 }, Selection::caret(3)],
            0,
        )
        .unwrap();
    let before = buffer.selections().clone();
    assert!(buffer.indent_lines("  ", false).unwrap());
    assert_eq!(buffer.text(), "  a\r\n  b\r\nc");
    assert_eq!(
        buffer.selections().selections()[0],
        Selection { anchor: 8, head: 2 }
    );
    buffer.undo();
    assert_eq!(buffer.selections(), &before);
    assert!(!buffer.is_dirty());
    buffer.redo();
    assert!(buffer.indent_lines("  ", true).unwrap());
    assert_eq!(buffer.text(), "a\r\nb\r\nc");
}

#[test]
fn indentation_excludes_next_line_at_range_end_and_rejects_invalid_unit() {
    let mut buffer = EditorBuffer::new("a\nb\n".into());
    buffer
        .set_selections(vec![Selection { anchor: 0, head: 2 }], 0)
        .unwrap();
    buffer.indent_lines("\t", false).unwrap();
    assert_eq!(buffer.text(), "\ta\nb\n");
    let before = buffer.selections().clone();
    assert_eq!(
        buffer.indent_lines("\n", false),
        Err(EditError::InvalidIndentation)
    );
    assert_eq!(buffer.selections(), &before);
    buffer.undo();
    assert!(!buffer.indent_lines("    ", true).unwrap());
    assert!(!buffer.undo());
    assert!(buffer.redo());
}
