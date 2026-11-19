//! Links clicked in agent replies. Agents name files relative to the session's
//! folder, often with a position (`src/main.rs:42`, `src/main.rs#L42`), so those
//! open in the editor at that line; web links go to the browser.
use super::LumenApp;
use crate::{
    cli::split_position,
    model::markdown::LinkTarget,
    services::{logger, open_external},
};
use lumen_agent_core::SessionId;
use std::path::{Path, PathBuf};

/// What a chat link leads to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChatLink {
    External(String),
    /// A file or folder with an optional 1-based position.
    Path {
        path: PathBuf,
        line: Option<usize>,
        column: Option<usize>,
    },
    /// An in-page anchor or an empty link: nothing to open.
    Nothing,
}

/// Interpret `destination` from a reply in a session rooted at `workspace`.
/// `exists` decides `file:10` versus a file really named `file:10`.
pub(crate) fn chat_link(
    destination: &str,
    workspace: &Path,
    exists: impl Fn(&Path) -> bool,
) -> ChatLink {
    let destination = destination.trim();
    let lower = destination.to_ascii_lowercase();
    if ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
    {
        return ChatLink::External(destination.to_owned());
    }
    // `file:///abs/path` is a local path, not something for the browser.
    let local = if lower.starts_with("file://") {
        &destination["file://".len()..]
    } else {
        destination
    };
    // Anything else is a path: `main.rs:42` would otherwise read as a URL scheme.
    let relative = match LinkTarget::classify(local) {
        LinkTarget::Anchor(_) => return ChatLink::Nothing,
        LinkTarget::External(_) => percent_decoded_path(local),
        LinkTarget::File(path) => path,
    };
    if relative.is_empty() {
        return ChatLink::Nothing;
    }
    let resolve = |text: &str| {
        let path = Path::new(text);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace.join(path)
        }
    };
    let (line, column) = fragment_position(destination);
    let whole = resolve(&relative);
    if line.is_some() || exists(&whole) {
        return ChatLink::Path {
            path: whole,
            line,
            column,
        };
    }
    let (base, line, column) = split_position(&relative);
    ChatLink::Path {
        path: resolve(base),
        line,
        column,
    }
}

/// `local` without its `#fragment` or `?query`, as `LinkTarget` reads a path.
fn percent_decoded_path(local: &str) -> String {
    let path = local.split(['#', '?']).next().unwrap_or(local);
    match LinkTarget::classify(&format!("./{path}")) {
        LinkTarget::File(decoded) => decoded.trim_start_matches("./").to_owned(),
        _ => path.to_owned(),
    }
}

/// `#L42`, `#L42C7` or `#L42-L50` (the range's start) after the path.
fn fragment_position(destination: &str) -> (Option<usize>, Option<usize>) {
    let Some((_, fragment)) = destination.split_once('#') else {
        return (None, None);
    };
    let Some(rest) = fragment.strip_prefix('L') else {
        return (None, None);
    };
    let number = |text: &str| {
        let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<usize>().ok().filter(|value| *value > 0)
    };
    let line = number(rest);
    let column = rest.split_once('C').and_then(|(_, column)| number(column));
    (line, line.and(column))
}

impl LumenApp {
    pub(crate) fn open_chat_link(&mut self, session: SessionId, destination: &str) {
        let Some(workspace) = self
            .view
            .native_agents
            .as_ref()
            .and_then(|hub| hub.registry.get(session))
            .map(|record| record.workspace.clone())
        else {
            self.view.status = "This chat no longer exists".into();
            return;
        };
        let file_system = self.file_system.as_ref();
        match chat_link(destination, &workspace, |path| {
            file_system.entry_exists(path)
        }) {
            ChatLink::Nothing => {}
            ChatLink::External(url) => {
                self.view.status = match open_external(&url) {
                    Ok(()) => format!("Opened {url}"),
                    Err(error) => format!("Could not open {url}: {error}"),
                };
            }
            ChatLink::Path { path, .. } if self.file_system.is_directory(&path) => {
                self.reveal_in_explorer(path);
            }
            ChatLink::Path { path, .. } if !self.file_system.entry_exists(&path) => {
                let message = format!("{} does not exist", path.display());
                logger::info("agent", &format!("Chat link: {message}"));
                self.view.status = message;
            }
            ChatLink::Path { path, line, column } => {
                self.open_file(path.clone());
                let Some(line) = line else {
                    return;
                };
                if let Some(document) = self
                    .editor
                    .files
                    .iter()
                    .find(|file| file.path.as_deref() == Some(path.as_path()))
                {
                    let byte = super::cli_open::byte_at(&document.text, line, column);
                    self.view.pending_goto = Some((document.id, byte));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(destination: &str, existing: &[&str]) -> ChatLink {
        let workspace = Path::new("/work");
        let existing: Vec<PathBuf> = existing.iter().map(|p| workspace.join(p)).collect();
        chat_link(destination, workspace, |path| {
            existing.iter().any(|known| known == path)
        })
    }

    fn file(path: &str, line: Option<usize>, column: Option<usize>) -> ChatLink {
        ChatLink::Path {
            path: path.into(),
            line,
            column,
        }
    }

    #[test]
    fn relative_paths_resolve_against_the_session_folder() {
        assert_eq!(
            link("src/main.rs", &[]),
            file("/work/src/main.rs", None, None)
        );
        assert_eq!(
            link("docs/My%20Notes.md", &[]),
            file("/work/docs/My Notes.md", None, None)
        );
    }

    #[test]
    fn positions_come_from_a_suffix_or_a_line_fragment() {
        assert_eq!(
            link("src/main.rs:42", &[]),
            file("/work/src/main.rs", Some(42), None)
        );
        assert_eq!(
            link("src/main.rs:42:7", &[]),
            file("/work/src/main.rs", Some(42), Some(7))
        );
        assert_eq!(
            link("src/main.rs#L42", &[]),
            file("/work/src/main.rs", Some(42), None)
        );
        assert_eq!(
            link("src/main.rs#L42C7", &[]),
            file("/work/src/main.rs", Some(42), Some(7))
        );
        assert_eq!(
            link("src/main.rs#L10-L20", &[]),
            file("/work/src/main.rs", Some(10), None)
        );
        // A file that really has the suffix in its name is opened as named.
        assert_eq!(link("odd:5", &["odd:5"]), file("/work/odd:5", None, None));
        // A bare name with a position is a file, not a URL with a `main.rs:` scheme.
        assert_eq!(
            link("main.rs:42", &[]),
            file("/work/main.rs", Some(42), None)
        );
    }

    #[test]
    fn web_links_anchors_and_file_urls() {
        assert_eq!(
            link("https://example.com/a", &[]),
            ChatLink::External("https://example.com/a".into())
        );
        assert_eq!(link("#usage", &[]), ChatLink::Nothing);
        assert_eq!(link("", &[]), ChatLink::Nothing);
    }

    #[cfg(unix)]
    #[test]
    fn absolute_paths_and_file_urls_are_used_as_given() {
        assert_eq!(link("/etc/hosts", &[]), file("/etc/hosts", None, None));
        assert_eq!(
            link("file:///tmp/a.txt", &[]),
            file("/tmp/a.txt", None, None)
        );
    }
}
