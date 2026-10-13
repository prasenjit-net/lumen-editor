use crate::{DragSelection, EditorSurface, TextGeometry};
use egui::{Event, ImeEvent, Key, PointerButton, Pos2, Rect, Response, Ui};
use lumen_editor_core::{Composition, Direction, EditorBuffer, Selection, SelectionAffinity};

impl EditorSurface {
    pub(crate) fn handle_input(
        &mut self,
        ui: &Ui,
        response: &Response,
        clip: Rect,
        origin: Pos2,
        buffer: &mut EditorBuffer,
    ) -> bool {
        let before = buffer.revision();
        if !ui.is_enabled() || !ui.input(|input| input.focused) {
            buffer.break_undo_group();
            self.drag_anchor = None;
            self.pending_shortcut_text = false;
            self.vertical_goals = None;
            self.cancel_composition();
            return false;
        }
        let events = ui.input(|input| input.events.clone());
        if events.iter().any(|event| matches!(event, Event::Ime(_))) {
            ui.ctx().request_repaint(); // Status renders before editor input in the example.
        }
        if self
            .composition
            .as_ref()
            .is_some_and(|composition| !composition.is_current(buffer))
        {
            self.cancel_composition();
            self.notice = Some("Composition cancelled: editor target changed".into());
        }
        let shortcut_in_events = events.iter().any(|event| {
            matches!(event, Event::Key { key: Key::I, pressed: true, modifiers, .. }
                if modifiers.alt && modifiers.shift)
        });
        let mut suppress_shortcut_text =
            shortcut_in_events || std::mem::take(&mut self.pending_shortcut_text);
        let mut shortcut_this_frame = false;
        let mut consumed_shortcut_text = false;
        for event in events {
            // Passive pointer motion and key releases must not erase preferred
            // columns between consecutive vertical moves. Actual edits and
            // selection changes are also validated against the saved snapshot.
            if matches!(
                event,
                Event::Text(_) | Event::Paste(_) | Event::Cut | Event::Ime(_)
            ) || matches!(event, Event::PointerButton { pressed: true, .. })
            {
                self.vertical_goals = None;
            }
            if matches!(
                event,
                Event::Copy | Event::Cut | Event::Paste(_) | Event::Ime(_)
            ) || matches!(event, Event::PointerButton { pressed: true, .. })
            {
                buffer.break_undo_group();
            }
            if let Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            } = event
            {
                if pressed {
                    let geometry =
                        self.geometry(ui, buffer.text(), origin, self.composition.as_ref());
                    self.cancel_composition();
                    if clip.contains(pos) {
                        response.request_focus();
                        let byte = geometry.hit_test(pos);
                        let affinity = geometry.pointer_affinity(pos, byte);
                        let set = buffer.selections();
                        let anchor = if modifiers.shift {
                            set.selections()[set.primary_index()].anchor
                        } else {
                            byte
                        };
                        let anchor_affinity = if modifiers.shift {
                            set.affinities()[set.primary_index()].anchor
                        } else {
                            affinity
                        };
                        if modifiers.alt && modifiers.shift {
                            buffer
                                .set_selections_with_affinity(
                                    vec![(
                                        Selection::caret(byte),
                                        SelectionAffinity {
                                            anchor: affinity,
                                            head: affinity,
                                        },
                                    )],
                                    0,
                                )
                                .unwrap();
                            self.drag_anchor = Some(DragSelection::Column {
                                origin: pos,
                                revision: buffer.revision(),
                            });
                        } else if modifiers.alt {
                            let previous: Vec<_> = set
                                .selections()
                                .iter()
                                .copied()
                                .zip(set.affinities().iter().copied())
                                .collect();
                            let mut selections = previous.clone();
                            selections.push((
                                Selection::caret(byte),
                                SelectionAffinity {
                                    anchor: affinity,
                                    head: affinity,
                                },
                            ));
                            buffer
                                .set_selections_with_affinity(selections, previous.len())
                                .unwrap();
                            self.drag_anchor = Some(DragSelection::Additive {
                                anchor: byte,
                                revision: buffer.revision(),
                                affinity,
                                previous,
                            });
                        } else {
                            buffer
                                .set_selections_with_affinity(
                                    vec![(
                                        Selection { anchor, head: byte },
                                        SelectionAffinity {
                                            anchor: anchor_affinity,
                                            head: affinity,
                                        },
                                    )],
                                    0,
                                )
                                .unwrap();
                            self.drag_anchor = Some(DragSelection::Linear {
                                anchor,
                                revision: buffer.revision(),
                                affinity: anchor_affinity,
                            });
                        }
                    } else {
                        self.drag_anchor = None;
                        response.surrender_focus();
                    }
                } else {
                    self.drag_anchor = None;
                    if clip.contains(pos)
                        && !modifiers.alt
                        && response.double_clicked_by(PointerButton::Primary)
                    {
                        buffer.select_next_occurrence();
                    }
                    if clip.contains(pos)
                        && !modifiers.alt
                        && response.triple_clicked_by(PointerButton::Primary)
                    {
                        let set = buffer.selections();
                        let head = set.selections()[set.primary_index()].head;
                        let start = buffer.text()[..head]
                            .rfind('\n')
                            .map_or(0, |newline| newline + 1);
                        let end = buffer.text()[head..]
                            .find('\n')
                            .map_or(buffer.text().len(), |offset| head + offset + 1);
                        buffer
                            .set_selections(
                                vec![Selection {
                                    anchor: start,
                                    head: end,
                                }],
                                0,
                            )
                            .unwrap();
                    }
                }
                continue;
            }
            if let Event::PointerMoved(pos) = event {
                if let Some(drag) = self.drag_anchor.clone() {
                    if drag.revision() != buffer.revision() {
                        self.drag_anchor = None;
                        continue;
                    }
                    let geometry = self.geometry(ui, buffer.text(), origin, None);
                    let (selections, primary) = match drag {
                        DragSelection::Column { origin, .. } => {
                            column_selections(&geometry, origin, pos)
                        }
                        DragSelection::Linear {
                            anchor, affinity, ..
                        } => (vec![drag_range(&geometry, anchor, affinity, pos)], 0),
                        DragSelection::Additive {
                            anchor,
                            affinity,
                            mut previous,
                            ..
                        } => {
                            let primary = previous.len();
                            previous.push(drag_range(&geometry, anchor, affinity, pos));
                            (previous, primary)
                        }
                    };
                    if buffer
                        .set_selections_with_affinity(selections, primary)
                        .is_err()
                    {
                        self.drag_anchor = None;
                    }
                }
                continue;
            }
            if let Event::AccessKitActionRequest(ref request) = event {
                let geometry = self.geometry(ui, buffer.text(), origin, self.composition.as_ref());
                if crate::accessibility::apply_action(ui, response, request, &geometry, buffer) {
                    self.cancel_composition();
                }
                continue;
            }
            if !ui.memory(|memory| memory.has_focus(response.id)) {
                continue;
            }
            match event {
                Event::Ime(ImeEvent::Enabled) => {
                    self.composition = Some(Composition::begin(buffer));
                    self.composition_cancelled = false;
                    self.notice = Some(
                        "IME preview at primary cursor; commit edits all captured cursors".into(),
                    );
                }
                Event::Ime(ImeEvent::Preedit(text)) if !self.composition_cancelled => {
                    if !text.contains(['\r', '\n']) {
                        self.composition
                            .get_or_insert_with(|| Composition::begin(buffer))
                            .update(text);
                        self.notice = Some(
                            "IME preview at primary cursor; commit edits all captured cursors"
                                .into(),
                        );
                    }
                }
                Event::Ime(ImeEvent::Disabled) => self.cancel_composition(),
                Event::Ime(ImeEvent::Commit(text)) => {
                    if let Some(composition) = self.composition.take() {
                        if text.contains(['\r', '\n']) {
                            self.notice =
                                Some("Composition cancelled: unexpected multiline commit".into());
                        } else if composition.commit(buffer, &text).is_err() {
                            self.notice =
                                Some("Composition cancelled: editor target changed".into());
                        } else {
                            self.notice = None;
                        }
                    }
                    self.composition_cancelled = true;
                }
                Event::Key {
                    key: Key::Escape,
                    pressed: true,
                    ..
                } if self.composition.is_some() => {
                    self.cancel_composition();
                    self.notice = Some("Composition cancelled".into());
                }
                Event::Copy | Event::Cut if self.composition.is_none() => {
                    let clipboard = if matches!(event, Event::Cut) {
                        buffer.cut_selections().unwrap()
                    } else {
                        buffer.copy_selections()
                    };
                    if let Some(clipboard) = clipboard {
                        ui.ctx().copy_text(clipboard.text().to_owned());
                        self.clipboard = Some(clipboard);
                    }
                }
                Event::Paste(text) if self.composition.is_none() => {
                    self.drag_anchor = None;
                    if self
                        .clipboard
                        .as_ref()
                        .is_some_and(|clipboard| clipboard.text() != text)
                    {
                        self.clipboard = None;
                    }
                    buffer.paste(&text, self.clipboard.as_ref()).unwrap();
                }
                Event::Text(text) if self.composition.is_none() => {
                    if suppress_shortcut_text && matches!(text.as_str(), "ˆ" | "^" | "\u{0302}") {
                        suppress_shortcut_text = false;
                        consumed_shortcut_text = true;
                        continue;
                    }
                    self.drag_anchor = None;
                    buffer
                        .type_text(&text, ui.input(|input| input.time))
                        .unwrap();
                }
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if self.composition.is_none() => {
                    self.drag_anchor = None;
                    if modifiers.command
                        || matches!(
                            key,
                            Key::ArrowLeft
                                | Key::ArrowRight
                                | Key::ArrowUp
                                | Key::ArrowDown
                                | Key::Home
                                | Key::End
                                | Key::PageUp
                                | Key::PageDown
                                | Key::Backspace
                                | Key::Delete
                                | Key::Enter
                                | Key::Tab
                                | Key::Escape
                        )
                    {
                        buffer.break_undo_group();
                    }
                    if self.selection_command(ui, key, modifiers, origin, buffer) {
                        // Option+Shift+I can also emit macOS's dead-key text.
                        // The command owns this keypress, not the text event.
                        shortcut_this_frame = key == Key::I && modifiers.alt && modifiers.shift;
                        suppress_shortcut_text = shortcut_this_frame && !consumed_shortcut_text;
                        continue;
                    }
                    match key {
                        Key::Z if modifiers.command => {
                            if modifiers.shift {
                                buffer.redo();
                            } else {
                                buffer.undo();
                            }
                        }
                        Key::Y if modifiers.command => {
                            buffer.redo();
                        }
                        Key::A if modifiers.command => {
                            buffer
                                .set_selections(
                                    vec![Selection {
                                        anchor: 0,
                                        head: buffer.text().len(),
                                    }],
                                    0,
                                )
                                .unwrap();
                        }
                        Key::ArrowLeft | Key::ArrowRight
                            if !modifiers.command && !modifiers.alt && !modifiers.ctrl =>
                        {
                            let geometry = self.geometry(ui, buffer.text(), origin, None);
                            if geometry.is_shaped() {
                                crate::visual_navigation::move_horizontal(
                                    buffer,
                                    &geometry,
                                    key == Key::ArrowRight,
                                    modifiers.shift,
                                );
                            } else {
                                buffer.move_horizontal(
                                    if key == Key::ArrowLeft {
                                        Direction::Backward
                                    } else {
                                        Direction::Forward
                                    },
                                    modifiers.shift,
                                );
                            }
                        }
                        Key::Backspace | Key::Delete
                            if !modifiers.command && !modifiers.alt && !modifiers.ctrl =>
                        {
                            buffer
                                .delete_graphemes(if key == Key::Backspace {
                                    Direction::Backward
                                } else {
                                    Direction::Forward
                                })
                                .unwrap();
                        }
                        Key::Enter => {
                            let fragments = self
                                .indentation
                                .effective(buffer.text())
                                .newline_fragments(buffer.text(), buffer.selections());
                            let fragments: Vec<&str> =
                                fragments.iter().map(String::as_str).collect();
                            buffer.replace_selections(&fragments).unwrap();
                        }
                        Key::Escape => {
                            let set = buffer.selections();
                            buffer
                                .set_selections_with_affinity(
                                    vec![(
                                        set.selections()[set.primary_index()],
                                        set.affinities()[set.primary_index()],
                                    )],
                                    0,
                                )
                                .unwrap();
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        // A lost native focus/release cannot leave a drag or composition alive.
        if !ui.input(|input| input.focused) || !ui.memory(|memory| memory.has_focus(response.id)) {
            buffer.break_undo_group();
            self.drag_anchor = None;
            self.pending_shortcut_text = false;
            self.vertical_goals = None;
            self.cancel_composition();
        }
        if !ui.input(|input| input.pointer.primary_down()) {
            self.drag_anchor = None;
        }
        if ui.input(|input| input.focused) && ui.memory(|memory| memory.has_focus(response.id)) {
            self.pending_shortcut_text = shortcut_this_frame && suppress_shortcut_text;
        }
        before != buffer.revision()
    }

    pub(crate) fn cancel_composition(&mut self) {
        if self.composition.take().is_some() {
            self.notice = Some("Composition cancelled".into());
        }
        self.composition_cancelled = true;
    }
}

fn drag_range(
    geometry: &TextGeometry,
    anchor: usize,
    anchor_affinity: lumen_editor_core::CaretAffinity,
    pointer: Pos2,
) -> (Selection, SelectionAffinity) {
    let head = geometry.hit_test(pointer);
    (
        Selection { anchor, head },
        SelectionAffinity {
            anchor: anchor_affinity,
            head: geometry.pointer_affinity(pointer, head),
        },
    )
}

/// Shift+Alt drag creates one range per displayed row. Both endpoints come
/// from the same geometry used for text painting and ordinary pointer mapping.
fn column_selections(
    geometry: &TextGeometry,
    origin: Pos2,
    pointer: Pos2,
) -> (Vec<(Selection, SelectionAffinity)>, usize) {
    let rows = geometry.row_rects();
    let row_at = |y: f32| {
        rows.iter()
            .position(|row| y < row.bottom())
            .unwrap_or(rows.len() - 1)
    };
    let first = row_at(origin.y);
    let last = row_at(pointer.y);
    let start = first.min(last);
    let end = first.max(last);
    let selections = rows[start..=end]
        .iter()
        .map(|row| {
            let anchor_point = Pos2::new(origin.x, row.center().y);
            let anchor = geometry.hit_test(anchor_point);
            drag_range(
                geometry,
                anchor,
                geometry.pointer_affinity(anchor_point, anchor),
                Pos2::new(pointer.x, row.center().y),
            )
        })
        .collect();
    (selections, last - start)
}
