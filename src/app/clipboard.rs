//! Service results enter the existing validated widget input path.
use super::LumenApp;
use crate::services::ClipboardRead;
use eframe::egui;
use std::time::Duration;

impl LumenApp {
    pub(crate) fn route_platform_paste(&mut self, ctx: &egui::Context) {
        let Some(document) = self.editor.active() else {
            return;
        };
        if self.view.editor_paste_target.is_some()
            || super::editor_action::blocked(&self.view)
            || self.view.explorer_focused
            || ctx.input(|input| input.pointer.any_pressed() || input.pointer.any_released())
            || self
                .view
                .find
                .get(&document.id)
                .is_some_and(|find| find.focus)
            || !ctx.memory(|memory| {
                memory.has_focus(super::editor_action::text_widget_id(document.id))
            })
            || !ctx.input(|input| {
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Paste(_)))
            })
        {
            return;
        }
        // Keep the payload at its original position. An asynchronous replacement
        // here would let later typing/navigation overtake the keyboard paste.
        // Do not perform native I/O while holding egui's input lock.
        let events = ctx.input_mut(|input| std::mem::take(&mut input.events));
        let mut routed = Vec::with_capacity(events.len());
        for event in events {
            if let egui::Event::Paste(hint) = event {
                match self.clipboard.platform_text(&hint) {
                    Ok(Some(text)) if !text.is_empty() => routed.push(egui::Event::Paste(text)),
                    Ok(_) => self.view.status = "Clipboard contains no text".into(),
                    Err(error) => self.view.status = format!("Could not read clipboard: {error}"),
                }
            } else {
                routed.push(event);
            }
        }
        ctx.input_mut(|input| input.events = routed);
    }

    pub(crate) fn start_clipboard_read(&mut self, ctx: &egui::Context) {
        if let Some(request) = self.view.clipboard_read.take() {
            self.clipboard.request_text(request);
            ctx.request_repaint();
        }
    }

    pub(crate) fn poll_clipboard(&mut self, ctx: &egui::Context) {
        let mut delivered = None;
        for ClipboardRead { request, result } in self.clipboard.poll() {
            if !self
                .view
                .editor_paste_target
                .is_some_and(|target| target.request == request)
                || delivered == Some(request)
            {
                continue; // A superseded request cannot impersonate the latest read.
            }
            delivered = Some(request);
            match result {
                Ok(Some(text)) => ctx.input_mut(|input| {
                    // A platform event can accompany a native menu action.
                    // Prefer the tagged, unnormalized read, delivering once.
                    input
                        .events
                        .retain(|event| !matches!(event, egui::Event::Paste(_)));
                    input.events.push(egui::Event::Paste(text));
                }),
                Ok(None) => {
                    self.view.editor_paste_target = None;
                    self.view.status = "Clipboard contains no text".into();
                }
                Err(error) => {
                    self.view.editor_paste_target = None;
                    self.view.status = format!("Could not read clipboard: {error}");
                }
            }
        }
        if self.view.editor_paste_target.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}
