//! Tagged native text reads. No egui dependency or newline normalization.
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClipboardRequestId(u64);

impl ClipboardRequestId {
    pub(crate) fn allocate() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

pub(crate) struct ClipboardRead {
    pub(crate) request: ClipboardRequestId,
    pub(crate) result: Result<Option<String>, String>,
}

pub(crate) trait ClipboardService {
    /// Resolve a platform paste before widget input, preserving event order.
    /// The native adapter rereads text because winit normalizes CRLF. The hint
    /// lets injected adapters replay platform events without touching the OS.
    fn platform_text(&mut self, hint: &str) -> Result<Option<String>, String>;
    fn request_text(&mut self, request: ClipboardRequestId);
    fn poll(&mut self) -> Vec<ClipboardRead>;
}

#[derive(Default)]
pub(crate) struct NativeClipboard {
    requests: Option<mpsc::Sender<ClipboardRequestId>>,
    results: Option<mpsc::Receiver<ClipboardRead>>,
    errors: Vec<ClipboardRead>,
    outstanding: Vec<ClipboardRequestId>,
}

impl NativeClipboard {
    fn start(&mut self) -> Result<(), String> {
        if self.requests.is_some() {
            return Ok(());
        }
        let (requests, receiver) = mpsc::channel::<ClipboardRequestId>();
        let (sender, results) = mpsc::channel();
        thread::Builder::new()
            .name("lumen-clipboard".into())
            .spawn(move || {
                for request in receiver {
                    let result = read_native_text();
                    if sender.send(ClipboardRead { request, result }).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        self.requests = Some(requests);
        self.results = Some(results);
        Ok(())
    }
}

fn read_native_text() -> Result<Option<String>, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    text_result(clipboard.get_text())
}

fn text_result(result: Result<String, arboard::Error>) -> Result<Option<String>, String> {
    match result {
        Ok(text) => Ok((!text.is_empty()).then_some(text)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

impl ClipboardService for NativeClipboard {
    fn platform_text(&mut self, _hint: &str) -> Result<Option<String>, String> {
        read_native_text()
    }

    fn request_text(&mut self, request: ClipboardRequestId) {
        let result = self.start().and_then(|()| {
            self.requests
                .as_ref()
                .unwrap()
                .send(request)
                .map_err(|error| error.to_string())
        });
        if let Err(error) = result {
            self.errors.push(ClipboardRead {
                request,
                result: Err(error),
            });
        } else {
            self.outstanding.push(request);
        }
    }

    fn poll(&mut self) -> Vec<ClipboardRead> {
        let mut result = std::mem::take(&mut self.errors);
        let mut disconnected = false;
        if let Some(receiver) = &self.results {
            loop {
                match receiver.try_recv() {
                    Ok(response) => {
                        self.outstanding
                            .retain(|request| *request != response.request);
                        result.push(response);
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            result.extend(self.outstanding.drain(..).map(|request| ClipboardRead {
                request,
                result: Err("Clipboard worker stopped".into()),
            }));
            self.requests = None;
            self.results = None;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_transport_retains_endings_and_reports_empty_or_unavailable() {
        assert_eq!(
            text_result(Ok("é 🌍\r\nnext\n".into())).unwrap(),
            Some("é 🌍\r\nnext\n".into())
        );
        assert_eq!(text_result(Ok(String::new())).unwrap(), None);
        assert_eq!(
            text_result(Err(arboard::Error::ContentNotAvailable)).unwrap(),
            None
        );
        assert!(text_result(Err(arboard::Error::ClipboardOccupied)).is_err());
    }

    #[test]
    fn worker_disconnection_finishes_every_outstanding_request() {
        let (requests, _receiver) = mpsc::channel();
        let (sender, results) = mpsc::channel::<ClipboardRead>();
        let mut service = NativeClipboard {
            requests: Some(requests),
            results: Some(results),
            ..Default::default()
        };
        let first = ClipboardRequestId::allocate();
        let second = ClipboardRequestId::allocate();
        service.request_text(first);
        service.request_text(second);
        drop(sender);
        let responses = service.poll();
        assert_eq!(
            responses
                .iter()
                .map(|response| response.request)
                .collect::<Vec<_>>(),
            [first, second]
        );
        assert!(responses.iter().all(|response| response.result.is_err()));
        assert!(service.outstanding.is_empty());
        assert!(service.requests.is_none());
    }
}
