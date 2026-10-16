//! Language-server edits as a reviewable, revision-validated plan.
//!
//! Server edits never reach a buffer or file directly. They are planned against
//! the exact text the server saw, shown to the user, validated again at apply
//! time, and then applied through the document core (one undo step per open
//! file) or the guarded workspace file writer.
use crate::model::DocumentId;
use lumen_editor_core::{EditorBuffer, Selection};
use lumen_language_services::incoming::TextEdit;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct PlannedFile {
    pub(crate) path: PathBuf,
    /// Set for files open in a tab: the document and the revision planned against.
    pub(crate) document: Option<(DocumentId, u64)>,
    /// Text the edits were computed for (disk text for files that are not open).
    pub(crate) base: String,
    /// Sorted, non-overlapping.
    pub(crate) edits: Vec<TextEdit>,
}

#[derive(Clone, Debug)]
pub(crate) struct EditPlan {
    pub(crate) title: String,
    pub(crate) files: Vec<PlannedFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreviewLine {
    pub(crate) path: PathBuf,
    pub(crate) line: usize,
    pub(crate) before: String,
    pub(crate) after: String,
}

impl EditPlan {
    pub(crate) fn edit_count(&self) -> usize {
        self.files.iter().map(|file| file.edits.len()).sum()
    }

    /// One before/after row per edit, using the line the edit starts on.
    pub(crate) fn preview_lines(&self) -> Vec<PreviewLine> {
        let mut rows = Vec::new();
        for file in &self.files {
            for edit in &file.edits {
                let text = &file.base;
                let start = text[..edit.range.start]
                    .rfind('\n')
                    .map_or(0, |index| index + 1);
                let end = text[edit.range.start..]
                    .find('\n')
                    .map_or(text.len(), |index| edit.range.start + index);
                let before = text[start..end].trim_end_matches('\r').to_owned();
                // A multi-line edit previews only its first line, marked with an ellipsis.
                let tail = if edit.range.end <= end {
                    &text[edit.range.end..end]
                } else {
                    "…"
                };
                let after = format!(
                    "{}{}{}",
                    &text[start..edit.range.start],
                    edit.new_text,
                    tail
                )
                .trim_end_matches('\r')
                .to_owned();
                rows.push(PreviewLine {
                    path: file.path.clone(),
                    line: text[..edit.range.start].matches('\n').count() + 1,
                    before,
                    after,
                });
            }
        }
        rows
    }
}

/// `text` with sorted, non-overlapping `edits` applied; `None` if any range is
/// out of bounds or splits a character.
pub(crate) fn apply_to_text(text: &str, edits: &[TextEdit]) -> Option<String> {
    let mut result = String::with_capacity(text.len());
    let mut position = 0;
    for edit in edits {
        if edit.range.start < position
            || edit.range.end > text.len()
            || !text.is_char_boundary(edit.range.start)
            || !text.is_char_boundary(edit.range.end)
        {
            return None;
        }
        result.push_str(&text[position..edit.range.start]);
        result.push_str(&edit.new_text);
        position = edit.range.end;
    }
    result.push_str(&text[position..]);
    Some(result)
}

/// Apply `edits` to an open buffer as one undo step and leave a single caret at
/// the old caret's mapped position. Returns whether the text changed.
pub(crate) fn apply_to_buffer(
    buffer: &mut EditorBuffer,
    edits: &[TextEdit],
) -> Result<bool, &'static str> {
    let text = buffer.text();
    let edits: Vec<&TextEdit> = edits
        .iter()
        .filter(|edit| text.get(edit.range.clone()) != Some(edit.new_text.as_str()))
        .collect();
    if edits.is_empty() {
        return Ok(false);
    }
    let selections = buffer.selections();
    let caret = selections.selections()[selections.primary_index()].head;
    let mut shift: isize = 0;
    let mut mapped = None;
    for edit in &edits {
        if edit.range.end <= caret {
            shift += edit.new_text.len() as isize - edit.range.len() as isize;
        } else if edit.range.start < caret {
            let end = edit.range.start as isize + shift + edit.new_text.len() as isize;
            mapped = Some(usize::try_from(end).unwrap_or(0));
            break;
        } else {
            break;
        }
    }
    let new_caret = mapped.unwrap_or_else(|| usize::try_from(caret as isize + shift).unwrap_or(0));
    let ranges = edits
        .iter()
        .map(|edit| Selection {
            anchor: edit.range.start,
            head: edit.range.end,
        })
        .collect();
    let fragments: Vec<&str> = edits.iter().map(|edit| edit.new_text.as_str()).collect();
    let changed = buffer
        .replace_ranges(ranges, 0, &fragments)
        .map_err(|_| "Edits overlap or no longer fit the text")?;
    let new_caret = new_caret.min(buffer.text().len());
    buffer
        .set_selections(vec![Selection::caret(new_caret)], 0)
        .map_err(|_| "Could not restore the caret after the edit")?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(range: std::ops::Range<usize>, text: &str) -> TextEdit {
        TextEdit {
            range,
            new_text: text.into(),
        }
    }

    #[test]
    fn text_edits_apply_in_order_and_reject_bad_ranges() {
        let edits = [edit(0..3, "bar"), edit(8..11, "bazz")];
        assert_eq!(
            apply_to_text("foo + 1 foo", &edits).unwrap(),
            "bar + 1 bazz"
        );
        assert!(apply_to_text("héllo", &[edit(1..2, "x")]).is_none());
        assert!(apply_to_text("ab", &[edit(1..5, "x")]).is_none());
        assert!(apply_to_text("abcd", &[edit(2..3, "x"), edit(0..1, "y")]).is_none());
    }

    #[test]
    fn buffer_edit_is_one_undo_step_and_keeps_the_caret_in_place() {
        let mut buffer = EditorBuffer::new("foo(1);\nfoo(2);".into());
        buffer
            .set_selections(vec![Selection::caret(12)], 0)
            .unwrap();
        let changed = apply_to_buffer(
            &mut buffer,
            &[edit(0..3, "renamed"), edit(8..11, "renamed")],
        )
        .unwrap();
        assert!(changed);
        assert_eq!(buffer.text(), "renamed(1);\nrenamed(2);");
        // The caret sat just after the second `foo(`; both renames shift it by 4 each.
        assert_eq!(buffer.selections().selections().len(), 1);
        assert_eq!(buffer.selections().selections()[0], Selection::caret(20));
        assert!(buffer.can_undo());
        buffer.undo();
        assert_eq!(buffer.text(), "foo(1);\nfoo(2);");
    }

    #[test]
    fn caret_inside_a_replaced_range_lands_after_the_new_text() {
        let mut buffer = EditorBuffer::new("a  b".into());
        buffer.set_selections(vec![Selection::caret(2)], 0).unwrap();
        apply_to_buffer(&mut buffer, &[edit(1..3, " ")]).unwrap();
        assert_eq!(buffer.text(), "a b");
        assert_eq!(buffer.selections().selections()[0], Selection::caret(2));
    }

    #[test]
    fn identical_text_edits_are_no_ops_without_history() {
        let mut buffer = EditorBuffer::new("same".into());
        assert_eq!(
            apply_to_buffer(&mut buffer, &[edit(0..4, "same")]),
            Ok(false)
        );
        assert!(!buffer.can_undo() && !buffer.is_dirty());
    }

    #[test]
    fn preview_rows_show_the_changed_line_before_and_after() {
        let plan = EditPlan {
            title: "Rename".into(),
            files: vec![PlannedFile {
                path: "a.rs".into(),
                document: None,
                base: "let a = 1;\r\nfoo(a);\n".into(),
                edits: vec![edit(4..5, "count"), edit(16..17, "count")],
            }],
        };
        assert_eq!(plan.edit_count(), 2);
        let rows = plan.preview_lines();
        assert_eq!((rows[0].line, rows[0].before.as_str()), (1, "let a = 1;"));
        assert_eq!(rows[0].after, "let count = 1;");
        assert_eq!((rows[1].line, rows[1].after.as_str()), (2, "foo(count);"));
    }
}
