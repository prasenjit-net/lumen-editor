use crate::{EditorSurface, TextGeometry};
use egui::{Key, Modifiers, Pos2, Ui};
use lumen_editor_core::{Direction, EditorBuffer, Selection, SelectionSet};

/// Preferred visual columns belong to a complete, unchanged selection snapshot.
/// Revision validation prevents reusing byte positions after an external edit.
pub(crate) struct VerticalGoals {
    revision: u64,
    selections: SelectionSet,
    columns: Vec<f32>,
}

impl EditorSurface {
    pub(crate) fn selection_command(
        &mut self,
        ui: &Ui,
        key: Key,
        modifiers: Modifiers,
        origin: Pos2,
        buffer: &mut EditorBuffer,
    ) -> bool {
        if !matches!(
            key,
            Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown
        ) {
            self.vertical_goals = None;
        }
        match key {
            Key::Backspace | Key::Delete
                if cfg!(target_os = "macos") && modifiers.mac_cmd && !modifiers.alt =>
            {
                let selections = buffer
                    .selections()
                    .selections()
                    .iter()
                    .map(|selection| {
                        if !selection.range().is_empty() {
                            return *selection;
                        }
                        let head = if key == Key::Backspace {
                            buffer.text()[..selection.head]
                                .rfind('\n')
                                .map_or(0, |newline| newline + 1)
                        } else {
                            let end = buffer.text()[selection.head..]
                                .find('\n')
                                .map_or(buffer.text().len(), |offset| selection.head + offset);
                            if end > 0 && buffer.text()[..end].ends_with('\r') {
                                end - 1
                            } else {
                                end
                            }
                        };
                        Selection {
                            anchor: selection.head,
                            head,
                        }
                    })
                    .collect();
                buffer
                    .replace_ranges(selections, buffer.selections().primary_index(), &[""])
                    .unwrap();
            }
            Key::ArrowLeft | Key::ArrowRight if word_shortcut(modifiers) => {
                buffer.move_word(
                    if key == Key::ArrowLeft {
                        Direction::Backward
                    } else {
                        Direction::Forward
                    },
                    modifiers.shift,
                );
            }
            Key::Backspace | Key::Delete if word_shortcut(modifiers) => {
                buffer
                    .delete_word(if key == Key::Backspace {
                        Direction::Backward
                    } else {
                        Direction::Forward
                    })
                    .unwrap();
            }
            Key::Home | Key::End | Key::ArrowLeft | Key::ArrowRight
                if matches!(key, Key::Home | Key::End)
                    || (cfg!(target_os = "macos") && modifiers.mac_cmd && !modifiers.alt) =>
            {
                let end = matches!(key, Key::End | Key::ArrowRight);
                let selections = buffer
                    .selections()
                    .selections()
                    .iter()
                    .map(|selection| {
                        let head = if modifiers.command && !cfg!(target_os = "macos") {
                            if end { buffer.text().len() } else { 0 }
                        } else if end {
                            let end = buffer.text()[selection.head..]
                                .find('\n')
                                .map_or(buffer.text().len(), |offset| selection.head + offset);
                            if end > 0 && buffer.text()[..end].ends_with('\r') {
                                end - 1
                            } else {
                                end
                            }
                        } else {
                            buffer.text()[..selection.head]
                                .rfind('\n')
                                .map_or(0, |newline| newline + 1)
                        };
                        Selection {
                            anchor: if modifiers.shift {
                                selection.anchor
                            } else {
                                head
                            },
                            head,
                        }
                    })
                    .collect();
                buffer
                    .set_selections(selections, buffer.selections().primary_index())
                    .unwrap();
            }
            Key::D if modifiers.command && !modifiers.alt && !modifiers.shift => {
                buffer.select_next_occurrence();
            }
            Key::L if modifiers.command && modifiers.shift && !modifiers.alt => {
                buffer.select_all_occurrences();
            }
            Key::Tab if !modifiers.command && !modifiers.alt && !modifiers.ctrl => {
                if modifiers.shift
                    || buffer
                        .selections()
                        .selections()
                        .iter()
                        .any(|selection| !selection.range().is_empty())
                {
                    buffer.indent_lines("    ", modifiers.shift).unwrap();
                } else {
                    buffer.replace_selections(&["    "]).unwrap();
                }
            }
            Key::ArrowUp | Key::ArrowDown if cursor_shortcut(modifiers) => {
                let geometry = self.geometry(ui, buffer.text(), origin, None);
                let set = buffer.selections();
                let head = set.selections()[set.primary_index()].head;
                let mut columns = self.preferred_columns(buffer, &geometry);
                let x = columns[set.primary_index()];
                if let Some(byte) = adjacent_row(&geometry, head, key == Key::ArrowDown, x) {
                    let mut selections = set.selections().to_vec();
                    let primary = selections.len();
                    selections.push(Selection::caret(byte));
                    columns.push(x);
                    buffer.set_selections(selections.clone(), primary).unwrap();
                    // Normalization can merge a newly added cursor with an
                    // existing selection. Start a fresh sequence in that case.
                    self.remember_columns(buffer, &selections, columns);
                }
            }
            Key::ArrowUp | Key::ArrowDown if !modifiers.alt && !modifiers.ctrl => {
                let geometry = self.geometry(ui, buffer.text(), origin, None);
                let set = buffer.selections();
                let columns = self.preferred_columns(buffer, &geometry);
                let selections: Vec<_> = set
                    .selections()
                    .iter()
                    .enumerate()
                    .map(|(index, selection)| {
                        let column = columns[index];
                        let head = if modifiers.command && cfg!(target_os = "macos") {
                            if key == Key::ArrowDown {
                                buffer.text().len()
                            } else {
                                0
                            }
                        } else {
                            adjacent_row(&geometry, selection.head, key == Key::ArrowDown, column)
                                .unwrap_or(selection.head)
                        };
                        Selection {
                            anchor: if modifiers.shift {
                                selection.anchor
                            } else {
                                head
                            },
                            head,
                        }
                    })
                    .collect();
                buffer
                    .set_selections(selections.clone(), set.primary_index())
                    .unwrap();
                self.remember_columns(buffer, &selections, columns);
            }
            Key::PageUp | Key::PageDown => {
                let geometry = self.geometry(ui, buffer.text(), origin, None);
                let columns = self.preferred_columns(buffer, &geometry);
                let distance = ui.clip_rect().height().max(18.0) - 18.0;
                let selections: Vec<_> = buffer
                    .selections()
                    .selections()
                    .iter()
                    .enumerate()
                    .map(|(index, selection)| {
                        let caret = geometry.caret_rect(selection.head);
                        let head = geometry.hit_test(Pos2::new(
                            origin.x + columns[index],
                            caret.center().y
                                + if key == Key::PageDown {
                                    distance
                                } else {
                                    -distance
                                },
                        ));
                        Selection {
                            anchor: if modifiers.shift {
                                selection.anchor
                            } else {
                                head
                            },
                            head,
                        }
                    })
                    .collect();
                buffer
                    .set_selections(selections.clone(), buffer.selections().primary_index())
                    .unwrap();
                self.remember_columns(buffer, &selections, columns);
            }
            _ => return false,
        }
        true
    }

    fn preferred_columns(&self, buffer: &EditorBuffer, geometry: &TextGeometry) -> Vec<f32> {
        if let Some(goals) = &self.vertical_goals
            && goals.revision == buffer.revision()
            && &goals.selections == buffer.selections()
        {
            return goals.columns.clone();
        }
        buffer
            .selections()
            .selections()
            .iter()
            .map(|selection| geometry.caret_rect(selection.head).left() - geometry.origin.x)
            .collect()
    }

    fn remember_columns(
        &mut self,
        buffer: &EditorBuffer,
        requested: &[Selection],
        columns: Vec<f32>,
    ) {
        // SelectionSet orders new cursors by document position, not insertion
        // order. Remap columns before caching; merges deliberately reset goals.
        self.vertical_goals =
            (columns.len() == buffer.selections().selections().len()).then(|| VerticalGoals {
                revision: buffer.revision(),
                selections: buffer.selections().clone(),
                columns: buffer
                    .selections()
                    .selections()
                    .iter()
                    .map(|selection| {
                        columns[requested
                            .iter()
                            .position(|candidate| candidate == selection)
                            .unwrap()]
                    })
                    .collect(),
            });
    }
}

fn word_shortcut(modifiers: Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.alt && !modifiers.mac_cmd && !modifiers.ctrl
    } else {
        modifiers.ctrl && !modifiers.alt
    }
}

fn cursor_shortcut(modifiers: Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.mac_cmd && modifiers.alt && !modifiers.shift
    } else if cfg!(target_os = "windows") {
        modifiers.ctrl && modifiers.alt && !modifiers.shift
    } else {
        modifiers.shift && modifiers.alt && !modifiers.ctrl && !modifiers.command
    }
}

fn adjacent_row(geometry: &TextGeometry, head: usize, down: bool, x: f32) -> Option<usize> {
    let caret = geometry.caret_rect(head);
    let y = caret.center().y;
    let rows = geometry.row_rects();
    let row = rows
        .iter()
        .position(|row| row.top() <= y && y <= row.bottom())?;
    let next = if down {
        row.checked_add(1)?
    } else {
        row.checked_sub(1)?
    };
    let next = rows.get(next)?;
    Some(geometry.hit_test(Pos2::new(geometry.origin.x + x, next.center().y)))
}
