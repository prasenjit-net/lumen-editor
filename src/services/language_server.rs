//! Safe discovery boundary for optional language-server executables.
//!
//! This service does not start a child process yet. It resolves only candidates
//! declared by Lumen's built-in language registry; workspace configuration is
//! never interpreted as an executable command.
use lumen_language_services::{
    LanguageDescriptor, ServerDescriptor, lsp::encode_message, protocol::ContentLengthDecoder,
};
use serde_json::Value;
use std::{
    env,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

const MAX_LSP_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ServerAvailability {
    NotSupported,
    Missing { candidates: Vec<String> },
    Available { executable: PathBuf },
}

pub(crate) trait LanguageServerService: Send + Sync {
    fn availability(&self, language: &LanguageDescriptor) -> ServerAvailability;
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
}

impl ServerProcess {
    pub(crate) fn start(executable: &Path, workspace: &Path) -> Result<Self, String> {
        let mut child = Command::new(executable)
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
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
        let (sender, events) = mpsc::channel();
        thread::spawn(move || read_server_output(output, sender));
        Ok(Self {
            child,
            input,
            events,
        })
    }

    pub(crate) fn send(&mut self, message: &Value) -> Result<(), String> {
        let framed = encode_message(message)
            .map_err(|error| format!("Could not encode language-server message: {error}"))?;
        self.input
            .write_all(&framed)
            .and_then(|_| self.input.flush())
            .map_err(|error| format!("Could not send language-server message: {error}"))
    }

    pub(crate) fn drain_events(&self) -> Vec<ServerEvent> {
        self.events.try_iter().collect()
    }

    pub(crate) fn has_exited(&mut self) -> Result<bool, String> {
        self.child
            .try_wait()
            .map(|status| status.is_some())
            .map_err(|error| format!("Could not inspect language server: {error}"))
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn read_server_output(mut output: impl Read, sender: Sender<ServerEvent>) {
    let mut decoder = ContentLengthDecoder::new(MAX_LSP_MESSAGE_BYTES);
    let mut buffer = [0; 4096];
    loop {
        let count = match output.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        let payloads = match decoder.push(&buffer[..count]) {
            Ok(payloads) => payloads,
            Err(error) => {
                let _ = sender.send(ServerEvent::ProtocolError(error.to_string()));
                return;
            }
        };
        for payload in payloads {
            match serde_json::from_slice(&payload) {
                Ok(message) => {
                    if sender.send(ServerEvent::Message(message)).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = sender.send(ServerEvent::ProtocolError(format!(
                        "Invalid language-server JSON: {error}"
                    )));
                    return;
                }
            }
        }
    }
    let _ = sender.send(ServerEvent::Exited);
}

#[derive(Default)]
pub(crate) struct LocalLanguageServer;

impl LanguageServerService for LocalLanguageServer {
    fn availability(&self, language: &LanguageDescriptor) -> ServerAvailability {
        let Some(server) = language.server else {
            return ServerAvailability::NotSupported;
        };
        discover(server, env::var_os("PATH").as_deref())
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
        read_server_output(std::io::Cursor::new(framed), sender);
        assert_eq!(
            receiver.try_iter().collect::<Vec<_>>(),
            vec![ServerEvent::Message(message), ServerEvent::Exited]
        );

        let (sender, receiver) = mpsc::channel();
        read_server_output(std::io::Cursor::new(b"Content-Length: 1\r\n\r\nx"), sender);
        assert!(matches!(
            receiver.try_iter().next(),
            Some(ServerEvent::ProtocolError(_))
        ));
    }
}
