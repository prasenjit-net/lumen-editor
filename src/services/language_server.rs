//! Process boundary for optional language-server executables.
//!
//! Only candidates declared by Lumen's built-in language registry can be
//! discovered and started; workspace configuration is never interpreted as an
//! executable command. Stdio is consumed off the UI thread and server stderr is
//! retained in a bounded buffer for actionable failure reporting.
use lumen_language_services::{
    LanguageDescriptor, ServerDescriptor, lsp::encode_message, protocol::ContentLengthDecoder,
};
use serde_json::Value;
use std::{
    env,
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

const MAX_LSP_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_SERVER_STDERR_BYTES: usize = 8 * 1024;
/// Reserved positive i32 ID, compatible with servers that use `lsp_server::RequestId`.
pub(crate) const SHUTDOWN_REQUEST_ID: u64 = i32::MAX as u64;
pub(crate) type ServerWake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ServerAvailability {
    NotSupported,
    Missing { candidates: Vec<String> },
    Available { executable: PathBuf },
}

pub(crate) trait LanguageServerService: Send + Sync {
    fn availability(&self, language: &LanguageDescriptor) -> ServerAvailability;

    fn start(
        &self,
        executable: &Path,
        workspace: &Path,
        wake: ServerWake,
    ) -> Result<Box<dyn LanguageServerConnection>, String>;
}

pub(crate) trait LanguageServerConnection: Send {
    fn send(&mut self, message: &Value) -> Result<(), String>;
    fn drain_events(&self) -> Vec<ServerEvent>;
    fn exit_reason(&mut self) -> Result<Option<String>, String>;
    fn output_closed_reason(&mut self) -> String;
    fn begin_shutdown(&mut self) -> Result<(), String>;
    fn finish_shutdown(&mut self);
}

/// Events leave the service on a channel so an LSP reader never blocks an
/// egui frame. App code supplies generation/document correlation separately.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ServerEvent {
    Message(Value),
    ProtocolError(String),
    Exited,
}

/// One registry-approved language-server process and its bounded stdio stream.
///
/// This type deliberately accepts an already-discovered executable path. The
/// service never interprets workspace configuration as a command line.
pub(crate) struct ServerProcess {
    child: Child,
    input: ChildStdin,
    events: Receiver<ServerEvent>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl ServerProcess {
    pub(crate) fn start(
        executable: &Path,
        workspace: &Path,
        wake: ServerWake,
    ) -> Result<Self, String> {
        let mut child = Command::new(executable)
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("Could not start language server: {error}"))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| "Language server did not provide stdin".to_owned())?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| "Language server did not provide stdout".to_owned())?;
        let error_output = child
            .stderr
            .take()
            .ok_or_else(|| "Language server did not provide stderr".to_owned())?;
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let stderr_reader = Arc::clone(&stderr);
        thread::spawn(move || read_server_stderr(error_output, stderr_reader));
        let (sender, events) = mpsc::channel();
        thread::spawn(move || read_server_output(output, sender, wake));
        Ok(Self {
            child,
            input,
            events,
            stderr,
        })
    }

    pub(crate) fn send(&mut self, message: &Value) -> Result<(), String> {
        let framed = encode_message(message)
            .map_err(|error| format!("Could not encode language-server message: {error}"))?;
        let result = self
            .input
            .write_all(&framed)
            .and_then(|_| self.input.flush());
        result.map_err(|error| {
            let detail = self.stderr_text();
            if detail.is_empty() {
                format!("Could not send language-server message: {error}")
            } else {
                format!("Could not send language-server message: {error}: {detail}")
            }
        })
    }

    pub(crate) fn drain_events(&self) -> Vec<ServerEvent> {
        self.events.try_iter().collect()
    }

    pub(crate) fn exit_reason(&mut self) -> Result<Option<String>, String> {
        let status = self
            .child
            .try_wait()
            .map_err(|error| format!("Could not inspect language server: {error}"))?;
        Ok(status.map(|status| {
            let detail = self.stderr_text();
            if detail.is_empty() {
                format!("Language server exited with {status}")
            } else {
                format!("Language server exited with {status}: {detail}")
            }
        }))
    }

    pub(crate) fn output_closed_reason(&mut self) -> String {
        if let Ok(Some(reason)) = self.exit_reason() {
            return reason;
        }
        let detail = self.stderr_text();
        if detail.is_empty() {
            "Language server closed its output stream".into()
        } else {
            format!("Language server closed its output stream: {detail}")
        }
    }

    pub(crate) fn begin_shutdown(&mut self) -> Result<(), String> {
        self.send(&lumen_language_services::lsp::shutdown_request(
            SHUTDOWN_REQUEST_ID,
        ))
    }

    pub(crate) fn finish_shutdown(&mut self) {
        let _ = self.send(&lumen_language_services::lsp::exit_notification());
    }

    fn stderr_text(&self) -> String {
        self.stderr
            .lock()
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
            .unwrap_or_default()
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

impl LanguageServerConnection for ServerProcess {
    fn send(&mut self, message: &Value) -> Result<(), String> {
        Self::send(self, message)
    }

    fn drain_events(&self) -> Vec<ServerEvent> {
        Self::drain_events(self)
    }

    fn exit_reason(&mut self) -> Result<Option<String>, String> {
        Self::exit_reason(self)
    }

    fn output_closed_reason(&mut self) -> String {
        Self::output_closed_reason(self)
    }

    fn begin_shutdown(&mut self) -> Result<(), String> {
        Self::begin_shutdown(self)
    }

    fn finish_shutdown(&mut self) {
        Self::finish_shutdown(self);
    }
}

fn read_server_output(mut output: impl Read, sender: Sender<ServerEvent>, wake: ServerWake) {
    let mut decoder = ContentLengthDecoder::new(MAX_LSP_MESSAGE_BYTES);
    let mut buffer = [0; 4096];
    loop {
        let count = match output.read(&mut buffer) {
            Ok(0) => break,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => {
                let _ = sender.send(ServerEvent::ProtocolError(format!(
                    "Could not read language-server output: {error}"
                )));
                wake();
                return;
            }
            Ok(count) => count,
        };
        let payloads = match decoder.push(&buffer[..count]) {
            Ok(payloads) => payloads,
            Err(error) => {
                let _ = sender.send(ServerEvent::ProtocolError(error.to_string()));
                wake();
                return;
            }
        };
        for payload in payloads {
            match serde_json::from_slice(&payload) {
                Ok(message) => {
                    if sender.send(ServerEvent::Message(message)).is_err() {
                        return;
                    }
                    wake();
                }
                Err(error) => {
                    let _ = sender.send(ServerEvent::ProtocolError(format!(
                        "Invalid language-server JSON: {error}"
                    )));
                    wake();
                    return;
                }
            }
        }
    }
    let _ = sender.send(ServerEvent::Exited);
    wake();
}

fn read_server_stderr(mut output: impl Read, retained: Arc<Mutex<Vec<u8>>>) {
    let mut buffer = [0; 1024];
    loop {
        let count = match output.read(&mut buffer) {
            Ok(0) => return,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return,
            Ok(count) => count,
        };
        let Ok(mut bytes) = retained.lock() else {
            return;
        };
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > MAX_SERVER_STDERR_BYTES {
            let excess = bytes.len() - MAX_SERVER_STDERR_BYTES;
            bytes.drain(..excess);
        }
    }
}

#[derive(Default)]
pub(crate) struct LocalLanguageServer;

impl LanguageServerService for LocalLanguageServer {
    fn availability(&self, language: &LanguageDescriptor) -> ServerAvailability {
        if !cfg!(feature = "language-server") {
            return ServerAvailability::NotSupported;
        }
        let Some(server) = language.server else {
            return ServerAvailability::NotSupported;
        };
        discover(server, env::var_os("PATH").as_deref())
    }

    fn start(
        &self,
        executable: &Path,
        workspace: &Path,
        wake: ServerWake,
    ) -> Result<Box<dyn LanguageServerConnection>, String> {
        ServerProcess::start(executable, workspace, wake)
            .map(|process| Box::new(process) as Box<dyn LanguageServerConnection>)
    }
}

fn discover(server: ServerDescriptor, path: Option<&std::ffi::OsStr>) -> ServerAvailability {
    let candidates = server
        .executable_candidates
        .iter()
        .map(|candidate| (*candidate).to_owned())
        .collect();
    let paths = path.map(env::split_paths).into_iter().flatten();
    for directory in paths {
        for candidate in server.executable_candidates {
            let executable = directory.join(candidate);
            if executable.is_file() {
                return ServerAvailability::Available { executable };
            }
            #[cfg(windows)]
            {
                let executable = directory.join(format!("{candidate}.exe"));
                if executable.is_file() {
                    return ServerAvailability::Available { executable };
                }
            }
        }
    }
    ServerAvailability::Missing { candidates }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_language_services::LanguageRegistry;
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct InterruptedOnce {
        interrupted: bool,
        remaining: io::Cursor<Vec<u8>>,
    }

    impl Read for InterruptedOnce {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::Error::from(ErrorKind::Interrupted));
            }
            self.remaining.read(buffer)
        }
    }

    #[test]
    fn discovery_finds_only_registered_candidates() {
        let temporary = tempfile::tempdir().unwrap();
        let analyzer = temporary.path().join("rust-analyzer");
        std::fs::write(&analyzer, []).unwrap();
        let rust = LanguageRegistry::by_id(lumen_language_services::LanguageId::RUST).unwrap();
        let path = env::join_paths([temporary.path()]).unwrap();
        assert_eq!(
            discover(rust.server.unwrap(), Some(&path)),
            ServerAvailability::Available {
                executable: analyzer
            }
        );
    }

    #[test]
    fn discovery_reports_missing_and_unsupported_languages_without_execution() {
        let go = LanguageRegistry::by_id(lumen_language_services::LanguageId::GO).unwrap();
        assert_eq!(
            discover(go.server.unwrap(), None),
            ServerAvailability::Missing {
                candidates: vec!["gopls".into()]
            }
        );
        let json = LanguageRegistry::by_id(lumen_language_services::LanguageId::JSON).unwrap();
        assert_eq!(
            LocalLanguageServer.availability(json),
            ServerAvailability::NotSupported
        );
    }

    #[test]
    fn stdio_reader_decodes_fragmented_messages_and_reports_invalid_json() {
        let message = serde_json::json!({"jsonrpc": "2.0", "method": "ready"});
        let framed = encode_message(&message).unwrap();
        let (sender, receiver) = mpsc::channel();
        read_server_output(std::io::Cursor::new(framed), sender, Arc::new(|| {}));
        assert_eq!(
            receiver.try_iter().collect::<Vec<_>>(),
            vec![ServerEvent::Message(message), ServerEvent::Exited]
        );

        let (sender, receiver) = mpsc::channel();
        read_server_output(
            std::io::Cursor::new(b"Content-Length: 1\r\n\r\nx"),
            sender,
            Arc::new(|| {}),
        );
        assert!(matches!(
            receiver.try_iter().next(),
            Some(ServerEvent::ProtocolError(_))
        ));
    }

    #[test]
    fn stdio_reader_retries_interrupted_reads() {
        let message = serde_json::json!({"jsonrpc": "2.0", "method": "ready"});
        let framed = encode_message(&message).unwrap();
        let input = InterruptedOnce {
            interrupted: false,
            remaining: io::Cursor::new(framed),
        };
        let (sender, receiver) = mpsc::channel();

        read_server_output(input, sender, Arc::new(|| {}));

        assert_eq!(
            receiver.try_iter().collect::<Vec<_>>(),
            vec![ServerEvent::Message(message), ServerEvent::Exited]
        );
    }

    #[test]
    fn stdio_reader_wakes_the_ui_for_messages_and_exit() {
        let message = serde_json::json!({"jsonrpc": "2.0", "method": "ready"});
        let framed = encode_message(&message).unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let wake_count = Arc::clone(&wakes);
        let (sender, _receiver) = mpsc::channel();
        read_server_output(
            std::io::Cursor::new(framed),
            sender,
            Arc::new(move || {
                wake_count.fetch_add(1, Ordering::Relaxed);
            }),
        );
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
    }

    /// Live checks against real servers; run with
    /// `cargo test live_ -- --ignored --nocapture` (add `~/go/bin` to PATH for gopls).
    #[test]
    #[cfg(feature = "language-server")]
    #[ignore = "requires rust-analyzer and a Rust toolchain on PATH"]
    fn live_rust_analyzer_publishes_diagnostics_for_a_broken_file() {
        live_diagnostics(
            lumen_language_services::LanguageId::RUST,
            &[(
                "Cargo.toml",
                "[package]\nname = \"live\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )],
            "src/main.rs",
            "fn main() { let x: u32 = \"no\"; }\n",
        );
    }

    #[test]
    #[cfg(feature = "language-server")]
    #[ignore = "requires gopls and a Go toolchain on PATH"]
    fn live_gopls_publishes_diagnostics_for_a_broken_file() {
        live_diagnostics(
            lumen_language_services::LanguageId::GO,
            &[("go.mod", "module live\n\ngo 1.21\n")],
            "main.go",
            "package main\n\nfunc main() { var x int = \"no\"; _ = x }\n",
        );
    }

    #[cfg(feature = "language-server")]
    fn live_diagnostics(
        language_id: lumen_language_services::LanguageId,
        project_files: &[(&str, &str)],
        source_name: &str,
        text: &str,
    ) {
        use lumen_language_services::{DocumentUri, ServerDocumentVersion, incoming, lsp};
        use std::time::{Duration, Instant};

        let language = LanguageRegistry::by_id(language_id).unwrap();
        let ServerAvailability::Available { executable } =
            LocalLanguageServer.availability(language)
        else {
            panic!("{} not on PATH", language.label);
        };
        let root = env::temp_dir().join(format!(
            "lumen-live-{}-{}",
            language_id.as_str(),
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        for (name, contents) in project_files {
            std::fs::write(root.join(name), contents).unwrap();
        }
        let file = root.join(source_name);
        std::fs::write(&file, text).unwrap();
        let root_uri = format!("file://{}", root.display());
        let file_uri = DocumentUri::new(format!("file://{}", file.display())).unwrap();

        let mut server = ServerProcess::start(&executable, &root, Arc::new(|| {})).unwrap();
        server
            .send(&lsp::initialize_request_with_options(
                0,
                &root_uri,
                language
                    .server
                    .and_then(|server| server.initialization_options),
            ))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut initialized = false;
        let mut legend = None;
        let mut diagnostics = None;
        let mut server_requests = Vec::new();
        while Instant::now() < deadline && diagnostics.is_none() {
            for event in server.drain_events() {
                let ServerEvent::Message(message) = event else {
                    continue;
                };
                let method = message.get("method").and_then(Value::as_str);
                if message.get("id") == Some(&serde_json::json!(0)) && !initialized {
                    initialized = true;
                    legend = lumen_language_services::capabilities::ServerCapabilities::from_initialize_response(&message)
                        .ok()
                        .and_then(|capabilities| capabilities.semantic_legend);
                    server.send(&lsp::initialized_notification()).unwrap();
                    server
                        .send(&lsp::did_open(
                            &file_uri,
                            language_id,
                            ServerDocumentVersion(1),
                            text,
                        ))
                        .unwrap();
                } else if method == Some("textDocument/publishDiagnostics")
                    && incoming::parse_publish_diagnostics(&message, text)
                        .is_ok_and(|published| !published.diagnostics.is_empty())
                {
                    diagnostics = Some(message);
                } else if let Some(reply) = lsp::server_request_response(&message) {
                    server_requests.push(method.unwrap_or_default().to_owned());
                    server.send(&reply).unwrap();
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        // Semantic tokens: the legend must be advertised and a full reply must decode.
        let legend = legend.expect("server did not advertise full semantic tokens");
        let request = lsp::semantic_tokens_request(7, &file_uri);
        server.send(&request).unwrap();
        let mut tokens = None;
        while Instant::now() < deadline && tokens.is_none() {
            for event in server.drain_events() {
                let ServerEvent::Message(message) = event else {
                    continue;
                };
                if message.get("id") == Some(&serde_json::json!(7)) {
                    let data = incoming::parse_semantic_tokens_response(&message).unwrap();
                    tokens = Some(
                        lumen_language_services::decode_semantic_tokens(text, &legend, &data)
                            .unwrap(),
                    );
                } else if let Some(reply) = lsp::server_request_response(&message) {
                    server.send(&reply).unwrap();
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        println!("semantic tokens: {tokens:?}");
        assert!(
            tokens.is_some_and(|tokens| !tokens.is_empty()),
            "no semantic tokens decoded"
        );
        println!("server-initiated requests: {server_requests:?}");
        println!("diagnostics: {diagnostics:?}");
        let stderr = server.stderr_text();
        let _ = server.begin_shutdown();
        let _ = std::fs::remove_dir_all(&root);
        assert!(initialized, "initialize never answered");
        assert!(diagnostics.is_some(), "no diagnostics: {stderr}");
    }

    #[test]
    #[cfg(feature = "language-server")]
    #[ignore = "requires rust-analyzer and a Rust toolchain on PATH"]
    fn live_rust_analyzer_completes_and_shows_signature_help() {
        live_completion(
            lumen_language_services::LanguageId::RUST,
            &[(
                "Cargo.toml",
                "[package]\nname = \"live\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )],
            "src/main.rs",
            "fn main() {\n    let mut v: Vec<u32> = Vec::new();\n    v.pu\n    v.push(\n}\n",
            ("v.pu", "push"),
            ("v.push(", "push"),
        );
    }

    #[test]
    #[cfg(feature = "language-server")]
    #[ignore = "requires gopls and a Go toolchain on PATH"]
    fn live_gopls_completes_and_shows_signature_help() {
        live_completion(
            lumen_language_services::LanguageId::GO,
            &[("go.mod", "module live\n\ngo 1.21\n")],
            "main.go",
            "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Prin\n\tfmt.Println(\n}\n",
            ("fmt.Prin", "Println"),
            ("fmt.Println(", "Println"),
        );
    }

    /// `completion` and `signature` are (anchor text whose end is the caret, expected substring).
    #[cfg(feature = "language-server")]
    fn live_completion(
        language_id: lumen_language_services::LanguageId,
        project_files: &[(&str, &str)],
        source_name: &str,
        text: &str,
        completion: (&str, &str),
        signature: (&str, &str),
    ) {
        use lumen_language_services::{
            DocumentUri, ServerDocumentVersion, byte_to_position, incoming, lsp,
        };
        use std::time::{Duration, Instant};

        let language = LanguageRegistry::by_id(language_id).unwrap();
        let ServerAvailability::Available { executable } =
            LocalLanguageServer.availability(language)
        else {
            panic!("{} not on PATH", language.label);
        };
        let root = env::temp_dir().join(format!(
            "lumen-live-complete-{}-{}",
            language_id.as_str(),
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        for (name, contents) in project_files {
            std::fs::write(root.join(name), contents).unwrap();
        }
        let file = root.join(source_name);
        std::fs::write(&file, text).unwrap();
        let root_uri = format!("file://{}", root.display());
        let uri = DocumentUri::new(format!("file://{}", file.display())).unwrap();
        let caret_after = |anchor: &str| {
            let end = text.find(anchor).expect("anchor in source") + anchor.len();
            byte_to_position(text, end).unwrap()
        };

        let mut server = ServerProcess::start(&executable, &root, Arc::new(|| {})).unwrap();
        server
            .send(&lsp::initialize_request_with_options(
                0,
                &root_uri,
                language
                    .server
                    .and_then(|server| server.initialization_options),
            ))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut next_id = 10_u64;
        let mut pending: Option<(u64, &str)> = None;
        let mut initialized = false;
        let (mut completed, mut signed) = (None, None);
        while Instant::now() < deadline && (completed.is_none() || signed.is_none()) {
            for event in server.drain_events() {
                let ServerEvent::Message(message) = event else {
                    continue;
                };
                if message.get("id") == Some(&serde_json::json!(0)) && !initialized {
                    initialized = true;
                    server.send(&lsp::initialized_notification()).unwrap();
                    server
                        .send(&lsp::did_open(
                            &uri,
                            language_id,
                            ServerDocumentVersion(1),
                            text,
                        ))
                        .unwrap();
                } else if let Some((id, kind)) = pending
                    && message.get("id") == Some(&serde_json::json!(id))
                {
                    pending = None;
                    if kind == "completion" {
                        let items = incoming::parse_completion_response(&message, text).unwrap();
                        println!("completion items: {}", items.len());
                        if let Some(item) =
                            items.iter().find(|item| item.label.contains(completion.1))
                        {
                            println!("matched: {item:?}");
                            completed = Some(item.clone());
                        }
                    } else if let Some(help) =
                        incoming::parse_signature_help_response(&message).unwrap()
                    {
                        println!("signature: {help:?}");
                        if help.label.contains(signature.1) {
                            signed = Some(help);
                        }
                    }
                } else if let Some(reply) = lsp::server_request_response(&message) {
                    server.send(&reply).unwrap();
                }
            }
            if initialized && pending.is_none() {
                // Servers answer empty until indexing finishes, so keep asking.
                next_id += 1;
                let (kind, request) = if completed.is_none() {
                    (
                        "completion",
                        lsp::completion_request(next_id, &uri, caret_after(completion.0)),
                    )
                } else {
                    (
                        "signature",
                        lsp::signature_help_request(next_id, &uri, caret_after(signature.0)),
                    )
                };
                server.send(&request).unwrap();
                pending = Some((next_id, kind));
                thread::sleep(Duration::from_millis(300));
            }
            thread::sleep(Duration::from_millis(50));
        }
        let stderr = server.stderr_text();
        let _ = server.begin_shutdown();
        let _ = std::fs::remove_dir_all(&root);
        assert!(completed.is_some(), "no matching completion: {stderr}");
        let help = signed.expect("no signature help");
        assert!(help.active_parameter.is_some() || help.label.contains(signature.1));
    }

    #[test]
    #[ignore = "requires rust-analyzer, rustfmt and a Rust toolchain on PATH"]
    fn live_rust_analyzer_renames_and_formats() {
        live_edits(
            lumen_language_services::LanguageId::RUST,
            &[(
                "Cargo.toml",
                "[package]\nname = \"live\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )],
            "src/main.rs",
            "fn helper() {}\nfn main(){ helper(); helper();let x=1;let _=x; }\n",
            "helper",
        );
    }

    #[test]
    #[cfg(feature = "language-server")]
    #[ignore = "requires gopls and a Go toolchain on PATH"]
    fn live_gopls_renames_and_formats() {
        live_edits(
            lumen_language_services::LanguageId::GO,
            &[("go.mod", "module live\n\ngo 1.21\n")],
            "main.go",
            "package main\n\nfunc helper() {}\nfunc main(){ helper(); helper();x:=1;_=x }\n",
            "helper",
        );
    }

    /// Rename the first `symbol` to `renamed` and format the document, decoding the
    /// replies exactly as the app does.
    fn live_edits(
        language_id: lumen_language_services::LanguageId,
        project_files: &[(&str, &str)],
        source_name: &str,
        text: &str,
        symbol: &str,
    ) {
        use lumen_language_services::{
            DocumentUri, ServerDocumentVersion, byte_to_position, incoming, lsp,
        };
        use std::time::{Duration, Instant};

        let language = LanguageRegistry::by_id(language_id).unwrap();
        let ServerAvailability::Available { executable } =
            LocalLanguageServer.availability(language)
        else {
            panic!("{} not on PATH", language.label);
        };
        let root = env::temp_dir().join(format!(
            "lumen-live-edit-{}-{}",
            language_id.as_str(),
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        for (name, contents) in project_files {
            std::fs::write(root.join(name), contents).unwrap();
        }
        let file = root.join(source_name);
        std::fs::write(&file, text).unwrap();
        let root_uri = format!("file://{}", root.display());
        let uri = DocumentUri::new(format!("file://{}", file.display())).unwrap();
        let symbol_start = text.find(symbol).unwrap() + 1;
        let position = byte_to_position(text, symbol_start).unwrap();

        let mut server = ServerProcess::start(&executable, &root, Arc::new(|| {})).unwrap();
        server
            .send(&lsp::initialize_request_with_options(
                0,
                &root_uri,
                language
                    .server
                    .and_then(|server| server.initialization_options),
            ))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        let (mut initialized, mut rename_plan, mut format_edits) = (false, None, None);
        let mut pending: Option<(u64, &str)> = None;
        let mut next_id = 10_u64;
        loop {
            assert!(
                Instant::now() < deadline,
                "timed out; stderr: {}",
                server.stderr_text()
            );
            let mut found = None;
            for event in server.drain_events() {
                let ServerEvent::Message(message) = event else {
                    continue;
                };
                if message.get("id") == Some(&serde_json::json!(0)) && !initialized {
                    initialized = true;
                    found = Some(
                        lumen_language_services::capabilities::ServerCapabilities::from_initialize_response(&message)
                            .unwrap(),
                    );
                    server.send(&lsp::initialized_notification()).unwrap();
                    server
                        .send(&lsp::did_open(
                            &uri,
                            language_id,
                            ServerDocumentVersion(1),
                            text,
                        ))
                        .unwrap();
                } else if let Some((id, kind)) = pending
                    && message.get("id") == Some(&serde_json::json!(id))
                {
                    pending = None;
                    if kind == "rename" {
                        if let Some(result) =
                            message.get("result").filter(|result| !result.is_null())
                        {
                            let files =
                                incoming::parse_workspace_edit(result, |_| Some(text.to_owned()))
                                    .expect("rename edit decodes");
                            println!("rename: {files:?}");
                            rename_plan = Some(files);
                        }
                    } else {
                        let edits = incoming::parse_text_edits_response(&message, text).unwrap();
                        println!("format: {} edits", edits.len());
                        if !edits.is_empty() {
                            format_edits = Some(edits);
                        }
                    }
                } else if let Some(reply) = lsp::server_request_response(&message) {
                    server.send(&reply).unwrap();
                }
            }
            if let Some(found) = found {
                assert!(
                    found.rename && found.formatting && found.references,
                    "{found:?}"
                );
            }
            if rename_plan.is_some() && format_edits.is_some() {
                break;
            }
            if initialized && pending.is_none() {
                next_id += 1;
                let (kind, request) = if rename_plan.is_none() {
                    (
                        "rename",
                        lsp::rename_request(next_id, &uri, position, "renamed"),
                    )
                } else {
                    ("format", lsp::formatting_request(next_id, &uri, 4, true))
                };
                server.send(&request).unwrap();
                pending = Some((next_id, kind));
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = server.begin_shutdown();
        let _ = std::fs::remove_dir_all(&root);
        let renamed = rename_plan.unwrap();
        let edits = &renamed[0].edits;
        assert!(edits.len() >= 3, "definition and both calls: {edits:?}");
        let applied = crate::app::workspace_edit::apply_to_text(text, edits).unwrap();
        assert_eq!(applied.matches("renamed").count(), 3);
        assert!(!applied.contains("helper"));
        let formatted =
            crate::app::workspace_edit::apply_to_text(text, &format_edits.unwrap()).unwrap();
        assert_ne!(formatted, text);
    }
}
