use crate::{EditorBuffer, Selection};

#[test]
fn consecutive_unicode_typing_at_multiple_cursors_is_one_undo_group() {
    let mut buffer = EditorBuffer::new("a b".into());
    buffer
        .set_selections(
            vec![
                Selection { anchor: 0, head: 1 },
                Selection { anchor: 2, head: 3 },
            ],
            1,
        )
        .unwrap();
    let original = buffer.selections().clone();
    buffer.type_text("e", 0.0).unwrap();
    buffer.type_text("\u{301}", 0.1).unwrap();
    buffer.type_text("漢", 0.2).unwrap();
    assert_eq!(buffer.text(), "é漢 é漢");
    let final_set = buffer.selections().clone();
    assert_eq!(buffer.revision(), 3);
    assert!(buffer.undo());
    assert_eq!(buffer.text(), "a b");
    assert_eq!(buffer.selections(), &original);
    assert!(!buffer.undo());
    assert!(buffer.redo());
    assert_eq!(buffer.text(), "é漢 é漢");
    assert_eq!(buffer.selections(), &final_set);
}

#[test]
fn pause_clock_reversal_and_nonfinite_time_start_new_groups() {
    for time in [2.0, -1.0, f64::NAN, f64::INFINITY] {
        let mut buffer = EditorBuffer::new(String::new());
        buffer.type_text("a", 0.0).unwrap();
        buffer.type_text("b", time).unwrap();
        buffer.undo();
        assert_eq!(buffer.text(), "a");
        buffer.undo();
        assert_eq!(buffer.text(), "");
    }
}

#[test]
fn save_selection_changes_and_explicit_boundaries_split_typing() {
    for boundary in 0..3 {
        let mut buffer = EditorBuffer::new(String::new());
        buffer.type_text("a", 0.0).unwrap();
        match boundary {
            0 => buffer.mark_saved(),
            1 => buffer.set_selections(vec![Selection::caret(1)], 0).unwrap(),
            _ => buffer.break_undo_group(),
        }
        buffer.type_text("b", 0.1).unwrap();
        buffer.undo();
        assert_eq!(buffer.text(), "a");
        assert_eq!(buffer.is_dirty(), boundary != 0);
    }
}

#[test]
fn paste_newline_and_ime_style_atomic_replacements_do_not_join_typing() {
    let mut buffer = EditorBuffer::new(String::new());
    buffer.type_text("a", 0.0).unwrap();
    buffer.replace_selections(&["paste"]).unwrap();
    buffer.type_text("b", 0.1).unwrap();
    buffer.type_text("\r\n", 0.2).unwrap();
    buffer.type_text("c", 0.3).unwrap();
    for expected in ["apasteb\r\n", "apasteb", "apaste", "a", ""] {
        assert!(buffer.undo());
        assert_eq!(buffer.text(), expected);
    }
}

#[test]
fn typing_after_undo_clears_redo_without_joining_the_previous_group() {
    let mut buffer = EditorBuffer::new(String::new());
    buffer.type_text("a", 0.0).unwrap();
    buffer.break_undo_group();
    buffer.type_text("b", 0.1).unwrap();
    buffer.undo();
    buffer.type_text("c", 0.2).unwrap();
    assert!(!buffer.can_redo());
    buffer.undo();
    assert_eq!(buffer.text(), "a");
    buffer.undo();
    assert_eq!(buffer.text(), "");
}

#[test]
fn coalescing_at_history_limit_does_not_evict_another_group() {
    let mut buffer = EditorBuffer::new(String::new());
    for index in 0..100 {
        buffer.type_text("a", index as f64 * 2.0).unwrap();
    }
    buffer.type_text("b", 198.1).unwrap();
    let mut steps = 0;
    while buffer.undo() {
        steps += 1;
    }
    assert_eq!(steps, 100);
    assert_eq!(buffer.text(), "");
}
