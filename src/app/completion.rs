//! Completion session rules: filtering as the user types and applying an item.
//!
//! The buffer stays the single mutation/history owner; accepting an item is one
//! `replace_ranges` call, so a single undo restores the pre-completion text.
use super::view_state::CompletionSession;
use lumen_editor_core::{EditorBuffer, Selection};
use lumen_language_services::incoming::CompletionItem;

pub(crate) fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Start of the identifier ending at `caret`.
pub(crate) fn word_start(text: &str, caret: usize) -> usize {
    let mut start = caret.min(text.len());
    while let Some(previous) = text[..start].chars().next_back() {
        if !is_word_char(previous) {
            break;
        }
        start -= previous.len_utf8();
    }
    start
}

/// Indices of items still matching what was typed since the session began, best
/// first (prefix matches before substring matches). `None` means the session no
/// longer applies: the caret left the word or the text before it changed shape.
pub(crate) fn visible(session: &CompletionSession, text: &str, caret: usize) -> Option<Vec<usize>> {
    if caret < session.word_start
        || caret > text.len()
        || !text.is_char_boundary(session.word_start)
        || !text.is_char_boundary(caret)
    {
        return None;
    }
    let typed = &text[session.word_start..caret];
    if !typed.chars().all(is_word_char) {
        return None;
    }
    let typed = typed.to_lowercase();
    let mut prefix = Vec::new();
    let mut infix = Vec::new();
    for (index, item) in session.items.iter().enumerate() {
        let filter = item.filter_text.to_lowercase();
        if filter.starts_with(&typed) {
            prefix.push(index);
        } else if filter.contains(&typed) {
            infix.push(index);
        }
    }
    prefix.extend(infix);
    Some(prefix)
}

/// Apply `item` at the single caret. Additional edits (such as auto-imports) are
/// applied only when they lie entirely before the completed word, where typing
/// since the request cannot have shifted them; otherwise they are skipped and the
/// returned note says so.
pub(crate) fn accept(
    buffer: &mut EditorBuffer,
    session: &CompletionSession,
    item: &CompletionItem,
) -> Result<Option<&'static str>, &'static str> {
    let selections = buffer.selections();
    let primary = selections.selections()[selections.primary_index()];
    if selections.selections().len() != 1 || primary.anchor != primary.head {
        return Err("Completion needs a single caret");
    }
    let caret = primary.head;
    let text = buffer.text();
    let main = match (&item.edit_range, caret == session.origin) {
        (Some(range), true) => range.clone(),
        _ => session.word_start..caret,
    };
    if main.start > caret
        || main.end < caret
        || main.end > text.len()
        || !text.is_char_boundary(main.start)
        || !text.is_char_boundary(main.end)
    {
        return Err("Completion no longer fits the text");
    }
    let usable = item
        .additional_edits
        .iter()
        .all(|edit| edit.range.end <= main.start && edit.range.end <= text.len());
    let mut edits: Vec<(std::ops::Range<usize>, &str)> = Vec::new();
    if usable {
        edits.extend(
            item.additional_edits
                .iter()
                .map(|edit| (edit.range.clone(), edit.new_text.as_str())),
        );
    }
    edits.push((main.clone(), item.insert_text.as_str()));
    edits.sort_by_key(|(range, _)| (range.start, range.end));
    let primary_index = edits
        .iter()
        .position(|(range, _)| *range == main)
        .unwrap_or(edits.len() - 1);
    let shift: isize = edits
        .iter()
        .filter(|(range, _)| range.end <= main.start && *range != main)
        .map(|(range, new)| new.len() as isize - range.len() as isize)
        .sum();
    let fragments: Vec<&str> = edits.iter().map(|(_, new)| *new).collect();
    let ranges = edits
        .iter()
        .map(|(range, _)| Selection {
            anchor: range.start,
            head: range.end,
        })
        .collect();
    buffer
        .replace_ranges(ranges, primary_index, &fragments)
        .map_err(|_| "Completion edits overlap or are invalid")?;
    let new_caret =
        usize::try_from(main.start as isize + shift).unwrap_or(0) + item.insert_text.len();
    buffer
        .set_selections(vec![Selection::caret(new_caret)], 0)
        .map_err(|_| "Could not place the caret after completion")?;
    Ok((!usable && !item.additional_edits.is_empty())
        .then_some("Completed without the item's extra edits (for example an import)"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_language_services::incoming::TextEdit;

    fn item(label: &str) -> CompletionItem {
        CompletionItem {
            label: label.into(),
            detail: None,
            kind: None,
            filter_text: label.into(),
            insert_text: label.into(),
            edit_range: None,
            additional_edits: Vec::new(),
        }
    }

    fn session(items: Vec<CompletionItem>, word_start: usize, origin: usize) -> CompletionSession {
        CompletionSession {
            document: crate::model::Document::untitled(0).id,
            origin,
            word_start,
            items,
            selected: 0,
            pending_accept: None,
        }
    }

    #[test]
    fn filtering_follows_typing_and_closes_when_the_word_is_left() {
        let text = "x.pus";
        let session = session(
            vec![item("len"), item("push"), item("push_str"), item("repush")],
            2,
            2,
        );
        assert_eq!(visible(&session, text, 2), Some(vec![0, 1, 2, 3]));
        assert_eq!(visible(&session, text, 5), Some(vec![1, 2, 3]));
        assert_eq!(visible(&session, "x.pu ", 5), None);
        assert_eq!(visible(&session, text, 1), None);
        assert_eq!(word_start("let ünï_x", 11), 4);
    }

    #[test]
    fn accepting_replaces_the_typed_word_in_one_undo_step() {
        let mut buffer = EditorBuffer::new("x.pu".into());
        buffer.set_selections(vec![Selection::caret(4)], 0).unwrap();
        let session = session(vec![item("push")], 2, 3);
        let note = accept(&mut buffer, &session, &session.items[0]).unwrap();
        assert_eq!(note, None);
        assert_eq!(buffer.text(), "x.push");
        assert_eq!(buffer.selections().selections()[0], Selection::caret(6));
        assert!(buffer.can_undo());
    }

    #[test]
    fn server_range_is_used_only_while_the_caret_is_where_it_was_requested() {
        let mut completion = item("push");
        completion.edit_range = Some(2..4);
        let mut buffer = EditorBuffer::new("x.pu".into());
        buffer.set_selections(vec![Selection::caret(4)], 0).unwrap();
        accept(
            &mut buffer,
            &session(vec![completion.clone()], 2, 4),
            &completion,
        )
        .unwrap();
        assert_eq!(buffer.text(), "x.push");
        // A range ending before the caret cannot be honored once the user typed more.
        let mut buffer = EditorBuffer::new("x.pus".into());
        buffer.set_selections(vec![Selection::caret(5)], 0).unwrap();
        accept(
            &mut buffer,
            &session(vec![completion.clone()], 2, 4),
            &completion,
        )
        .unwrap();
        assert_eq!(buffer.text(), "x.push");
    }

    #[test]
    fn import_edits_before_the_word_apply_and_later_ones_are_skipped() {
        let mut with_import = item("HashMap");
        with_import.additional_edits = vec![TextEdit {
            range: 0..0,
            new_text: "use std::collections::HashMap;\n".into(),
        }];
        let mut buffer = EditorBuffer::new("fn f() { Has }".into());
        buffer
            .set_selections(vec![Selection::caret(12)], 0)
            .unwrap();
        let session = session(vec![with_import.clone()], 9, 12);
        assert_eq!(accept(&mut buffer, &session, &with_import).unwrap(), None);
        assert_eq!(
            buffer.text(),
            "use std::collections::HashMap;\nfn f() { HashMap }"
        );
        let caret = buffer.selections().selections()[0].head;
        assert_eq!(&buffer.text()[caret..], " }");

        let mut after = with_import.clone();
        after.additional_edits[0].range = 13..13;
        let mut buffer = EditorBuffer::new("fn f() { Has }".into());
        buffer
            .set_selections(vec![Selection::caret(12)], 0)
            .unwrap();
        let note = accept(&mut buffer, &session, &after).unwrap();
        assert!(note.is_some());
        assert_eq!(buffer.text(), "fn f() { HashMap }");
    }

    #[test]
    fn multi_selection_or_range_selection_refuses_completion() {
        let mut buffer = EditorBuffer::new("abc".into());
        buffer
            .set_selections(vec![Selection { anchor: 0, head: 2 }], 0)
            .unwrap();
        let session = session(vec![item("abc")], 0, 2);
        assert!(accept(&mut buffer, &session, &session.items[0]).is_err());
        assert_eq!(buffer.text(), "abc");
    }
}
