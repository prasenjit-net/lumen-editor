//! Service results enter the existing validated widget input path.
use super::LumenApp;
use crate::services::ClipboardRead;
use eframe::egui;
use std::time::Duration;

impl LumenApp {
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
