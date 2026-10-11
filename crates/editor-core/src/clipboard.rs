//! Explicit selection and logical-line clipboard policies, independent of UI/I/O.
use crate::{EditError, EditorBuffer, Selection};
use std::collections::BTreeSet;

/// Explicit session metadata, in canonical document order. External text is
/// never implicitly interpreted as distributed multi-selection content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionClipboard {
    pub(crate) fragments: Vec<String>,
    pub(crate) plain_text: String,
    lines: bool,
}

impl SelectionClipboard {
    pub fn text(&self) -> &str {
        &self.plain_text
    }
}

impl EditorBuffer {
    /// Empty selections copy deduplicated logical lines using the shared rules.
    pub fn copy_selections(&self) -> Option<SelectionClipboard> {
        if self
            .selections()
            .selections()
            .iter()
            .all(|selection| selection.range().is_empty())
        {
            let mut seen = BTreeSet::new();
            let fragments: Vec<_> = self
                .selections()
                .selections()
                .iter()
                .filter_map(|selection| {
                    let caret = self.text()[..selection.head].chars().count();
                    let line = crate::text_line::line_clipboard(self.text(), caret)?;
                    seen.insert(line.deletion.start).then_some(line.text)
                })
                .collect();
            return Some(SelectionClipboard {
                plain_text: fragments.concat(),
                fragments,
                lines: true,
            });
        }
        let fragments: Vec<_> = self
            .selections()
            .selections()
            .iter()
            .map(|selection| self.text()[selection.range()].to_owned())
            .collect();
        Some(SelectionClipboard {
            plain_text: fragments.join("\n"),
            fragments,
            lines: false,
        })
    }

    pub fn cut_selections(&mut self) -> Result<Option<SelectionClipboard>, EditError> {
        let Some(clipboard) = self.copy_selections() else {
            return Ok(None);
        };
        if clipboard.lines {
            let ranges = self
                .selections()
                .selections()
                .iter()
                .map(|selection| {
                    let caret = self.text()[..selection.head].chars().count();
                    let line = crate::text_line::line_clipboard(self.text(), caret).unwrap();
                    let byte = |index| {
                        self.text()
                            .char_indices()
                            .map(|(byte, _)| byte)
                            .chain([self.text().len()])
                            .nth(index)
                            .unwrap()
                    };
                    Selection {
                        anchor: byte(line.deletion.start),
                        head: byte(line.deletion.end),
                    }
                })
                .collect();
            self.replace_ranges(ranges, self.selections().primary_index(), &[""])?;
        } else {
            self.replace_selections(&[""])?;
        }
        Ok(Some(clipboard))
    }

    pub fn paste(
        &mut self,
        text: &str,
        metadata: Option<&SelectionClipboard>,
    ) -> Result<bool, EditError> {
        if text.is_empty() {
            return Ok(false);
        }
        if let Some(metadata) = metadata.filter(|metadata| {
            metadata.lines
                && metadata.text() == text
                && self
                    .selections()
                    .selections()
                    .iter()
                    .all(|selection| selection.range().is_empty())
        }) {
            let before = self.selections().clone();
            let mut insertions = std::collections::BTreeMap::new();
            for (index, selected) in before.selections().iter().enumerate() {
                let start = self.text()[..selected.head]
                    .rfind('\n')
                    .map_or(0, |newline| newline + 1);
                let fragment = if metadata.fragments.len() == before.selections().len() {
                    metadata.fragments[index].as_str()
                } else {
                    text
                };
                insertions.entry(start).or_insert(fragment);
            }
            let primary_byte = before.selections()[before.primary_index()].head;
            let primary_start = self.text()[..primary_byte]
                .rfind('\n')
                .map_or(0, |newline| newline + 1);
            let primary = insertions
                .keys()
                .position(|start| *start == primary_start)
                .unwrap();
            let ranges = insertions
                .keys()
                .map(|byte| Selection::caret(*byte))
                .collect();
            let fragments = insertions.values().copied().collect::<Vec<_>>();
            let carets = before
                .selections()
                .iter()
                .map(|selected| {
                    Selection::caret(
                        selected.head
                            + insertions
                                .iter()
                                .filter(|(start, _)| **start <= selected.head)
                                .map(|(_, fragment)| fragment.len())
                                .sum::<usize>(),
                    )
                })
                .collect();
            let changed = self.replace_ranges(ranges, primary, &fragments)?;
            if changed {
                self.set_selections(carets, before.primary_index())?;
            }
            return Ok(changed);
        }
        if let Some(metadata) = metadata.filter(|metadata| {
            metadata.text() == text
                && metadata.fragments.len() == self.selections().selections().len()
        }) {
            let fragments: Vec<_> = metadata.fragments.iter().map(String::as_str).collect();
            self.replace_selections(&fragments)
        } else {
            self.replace_selections(&[text])
        }
    }
}
