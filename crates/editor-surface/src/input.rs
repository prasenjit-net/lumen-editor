use crate::{DragSelection, EditorSurface, TextGeometry};
use egui::{Event, ImeEvent, Key, PointerButton, Pos2, Rect, Response, Ui};
use lumen_editor_core::{Composition, Direction, EditorBuffer, Selection};

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
            self.drag_anchor = None;
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
        for event in events {
            if let Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            } = event
            {
                if pressed {
                    self.cancel_composition();
                    if clip.contains(pos) {
                        response.request_focus();
                        let byte = TextGeometry::layout(ui, buffer.text(), origin).hit_test(pos);
                        let set = buffer.selections();
                        let anchor = if modifiers.shift {
                            set.selections()[set.primary_index()].anchor
                        } else {
                            byte
                        };
                        if modifiers.alt {
                            let mut selections = set.selections().to_vec();
                            selections.push(Selection::caret(byte));
                            buffer
                                .set_selections(selections, set.selections().len())
                                .unwrap();
                            self.drag_anchor = None;
                        } else {
                            buffer
                                .set_selections(vec![Selection { anchor, head: byte }], 0)
                                .unwrap();
                            self.drag_anchor = Some(DragSelection {
                                anchor,
                                revision: buffer.revision(),
                            });
                        }
                    } else {
                        self.drag_anchor = None;
                        response.surrender_focus();
                    }
                } else {
                    self.drag_anchor = None;
                }
                continue;
            }
            if let Event::PointerMoved(pos) = event {
                if let Some(drag) = self.drag_anchor {
                    if drag.revision != buffer.revision() {
                        self.drag_anchor = None;
                        continue;
                    }
                    let byte = TextGeometry::layout(ui, buffer.text(), origin).hit_test(pos);
                    if buffer
                        .set_selections(
                            vec![Selection {
                                anchor: drag.anchor,
                                head: byte,
                            }],
                            0,
                        )
                        .is_err()
                    {
                        self.drag_anchor = None;
                    }
                }
                continue;
            }
            if let Event::AccessKitActionRequest(ref request) = event {
                if crate::accessibility::apply_action(ui, response, request, origin, buffer) {
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
                Event::Text(text) | Event::Paste(text) if self.composition.is_none() => {
                    self.drag_anchor = None;
                    buffer.replace_selections(&[&text]).unwrap();
                }
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if self.composition.is_none() => {
                    self.drag_anchor = None;
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
                            buffer.move_horizontal(
                                if key == Key::ArrowLeft {
                                    Direction::Backward
                                } else {
                                    Direction::Forward
                                },
                                modifiers.shift,
                            );
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
                            buffer.replace_selections(&["\n"]).unwrap();
                        }
                        Key::Escape => {
                            let set = buffer.selections();
                            buffer
                                .set_selections(vec![set.selections()[set.primary_index()]], 0)
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
            self.drag_anchor = None;
            self.cancel_composition();
        }
        if !ui.input(|input| input.pointer.primary_down()) {
            self.drag_anchor = None;
        }
        before != buffer.revision()
    }

    fn cancel_composition(&mut self) {
        if self.composition.take().is_some() {
            self.notice = Some("Composition cancelled".into());
        }
        self.composition_cancelled = true;
    }
}
