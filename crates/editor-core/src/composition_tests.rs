use super::*;
use crate::Selection;

#[test]
fn preedit_never_changes_text_revision_saved_checkpoint_or_history() {
    let mut buffer = EditorBuffer::new("é original".into());
    let mut composition = Composition::begin(&buffer);
    composition.update("候補".into());
    composition.update("確定".into());
    assert_eq!(buffer.text(), "é original");
    assert_eq!(buffer.revision(), 0);
    assert!(!buffer.is_dirty());
    assert!(!buffer.undo());
    assert!(!composition.commit(&mut buffer, "").unwrap());
    assert_eq!(buffer.text(), "é original");
}

#[test]
fn stale_revision_or_selection_rejects_commit_atomically() {
    for revision in [false, true] {
        let mut buffer = EditorBuffer::new("abc".into());
        let composition = Composition::begin(&buffer);
        if revision {
            buffer.replace_document("different").unwrap();
            buffer.undo();
        } else {
            buffer.set_selections(vec![Selection::caret(1)], 0).unwrap();
        }
        let before = buffer.selections().clone();
        assert_eq!(
            composition.commit(&mut buffer, "漢"),
            Err(EditError::StaleComposition)
        );
        assert_eq!(buffer.text(), "abc");
        assert_eq!(buffer.selections(), &before);
    }
}
