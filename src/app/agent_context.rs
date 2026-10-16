//! Context the user attaches to an agent message: selections, files, @-mentions
//! and slash commands. The helpers here are pure; the controller methods at the
//! bottom connect them to the editor.
use super::{LumenApp, agent::AgentKind, language_services::file_uri};
use lumen_agent_protocol::{requests, types::AvailableCommand};
use serde_json::Value;
use std::{
    collections::HashSet,
    ops::Range,
    path::{Path, PathBuf},
};

/// Selections longer than this are refused instead of silently truncated.
pub(crate) const MAX_SELECTION_BYTES: usize = 100 * 1024;

/// Config values that let the agent act without asking; changing to one confirms first.
pub(crate) const RISKY_VALUES: [&str; 2] = ["bypassPermissions", "agent-full-access"];

/// A chip in the composer: shown as `label`, sent as `block`.
#[derive(Clone, Debug)]
pub(crate) struct ComposerAttachment {
    pub(crate) label: String,
    pub(crate) block: Value,
}

// ------------------------------------------------------------------ mentions

/// The `@query` the caret is in (we only look at the end of the draft): the byte
/// index of the `@` and the text typed after it. `None` when the last word is not
/// a mention in progress.
pub(crate) fn active_mention(draft: &str) -> Option<(usize, &str)> {
    let start = draft
        .rfind(|c: char| c.is_whitespace())
        .map_or(0, |i| i + 1);
    let word = &draft[start..];
    word.strip_prefix('@').map(|query| (start, query))
}

/// Up to `limit` paths whose relative form matches `query`, best first: file
/// name prefix, then name substring, then path substring, then subsequence.
pub(crate) fn mention_candidates(
    paths: &[PathBuf],
    root: &Path,
    query: &str,
    limit: usize,
) -> Vec<String> {
    let query = query.to_lowercase();
    let mut scored: Vec<(u8, usize, String)> = paths
        .iter()
        .filter_map(|path| {
            let relative = path
                .strip_prefix(root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            let lower = relative.to_lowercase();
            let name = lower.rsplit('/').next().unwrap_or(&lower).to_owned();
            let rank = if query.is_empty() {
                3
            } else if name.starts_with(&query) {
                0
            } else if name.contains(&query) {
                1
            } else if lower.contains(&query) {
                2
            } else if is_subsequence(&query, &lower) {
                4
            } else {
                return None;
            };
            Some((rank, relative.len(), relative))
        })
        .collect();
    scored.sort();
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, relative)| relative)
        .collect()
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle.chars().all(|n| chars.any(|h| h == n))
}

/// Replace the in-progress `@query` with `@relative ` (a space ends the mention).
pub(crate) fn complete_mention(draft: &str, relative: &str) -> String {
    match active_mention(draft) {
        Some((start, _)) => format!("{}@{relative} ", &draft[..start]),
        None => format!("{draft}@{relative} "),
    }
}

/// `resource_link` blocks for every `@path` in `text` naming an existing file in
/// the folder. Unknown mentions stay plain text (they may be handles or emails).
pub(crate) fn mention_links(
    text: &str,
    root: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Vec<(String, Value)> {
    let mut seen = HashSet::new();
    let mut links = Vec::new();
    for word in text.split_whitespace() {
        let Some(relative) = word.strip_prefix('@') else {
            continue;
        };
        let relative = relative.trim_end_matches(|c: char| ",.;:)!?".contains(c));
        if relative.is_empty() || relative.starts_with('/') || relative.contains("..") {
            continue;
        }
        let path = root.join(relative);
        if exists(&path) && seen.insert(path.clone()) {
            links.push((
                relative.to_owned(),
                requests::resource_link_block(&file_uri(&path), relative),
            ));
        }
    }
    links
}

// ------------------------------------------------------------ slash commands

/// Commands matching a draft that is just `/partial` (no space yet).
pub(crate) fn slash_matches<'a>(
    commands: &'a [AvailableCommand],
    draft: &str,
) -> Vec<&'a AvailableCommand> {
    let Some(partial) = draft.strip_prefix('/') else {
        return Vec::new();
    };
    if partial.contains(char::is_whitespace) {
        return Vec::new();
    }
    let partial = partial.to_lowercase();
    let mut matches: Vec<_> = commands
        .iter()
        .filter(|command| command.name.to_lowercase().contains(&partial))
        .collect();
    matches.sort_by_key(|command| {
        (
            !command.name.to_lowercase().starts_with(&partial),
            command.name.clone(),
        )
    });
    matches
}

// -------------------------------------------------------- selection context

/// 1-based line range covered by `range` in `text`.
pub(crate) fn line_span(text: &str, range: &Range<usize>) -> (usize, usize) {
    let line_of = |byte: usize| text[..byte.min(text.len())].matches('\n').count() + 1;
    let start = line_of(range.start);
    // A selection ending right after a newline does not include the next line.
    let end_byte = if range.end > range.start && text[..range.end].ends_with('\n') {
        range.end - 1
    } else {
        range.end
    };
    (start, line_of(end_byte).max(start))
}

pub(crate) fn selection_attachment(
    path: Option<&Path>,
    name: &str,
    text: &str,
    range: &Range<usize>,
) -> Result<ComposerAttachment, String> {
    let selected = text
        .get(range.clone())
        .filter(|selected| !selected.is_empty())
        .ok_or("Select some text first")?;
    if selected.len() > MAX_SELECTION_BYTES {
        return Err("The selection is too large to attach (over 100 KiB)".into());
    }
    let (first, last) = line_span(text, range);
    let base = match path {
        Some(path) => file_uri(path),
        None => format!("untitled:{name}"),
    };
    let lines = if first == last {
        format!("{first}")
    } else {
        format!("{first}-{last}")
    };
    Ok(ComposerAttachment {
        label: format!("{name}:{lines}"),
        block: requests::resource_block(&format!("{base}#L{first}-L{last}"), selected, None),
    })
}

pub(crate) fn file_attachment(path: &Path, root: Option<&Path>) -> ComposerAttachment {
    let label = root
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    ComposerAttachment {
        block: requests::resource_link_block(&file_uri(path), &label),
        label,
    }
}

// --------------------------------------------------------------- controller

impl LumenApp {
    fn open_agent_composer(&mut self) -> AgentKind {
        let kind = self.view.agents.active;
        if !self.view.settings.boolean("agent.enabled") {
            self.view.status = "Coding agents are turned off in Settings (agent.enabled)".into();
            return kind;
        }
        self.view.agents.open = true;
        self.view.agents.session_mut(kind).focus_composer = true;
        kind
    }

    pub(crate) fn add_attachment(&mut self, attachment: ComposerAttachment) {
        let kind = self.open_agent_composer();
        let session = self.view.agents.session_mut(kind);
        if !session
            .attachments
            .iter()
            .any(|a| a.label == attachment.label)
        {
            session.attachments.push(attachment);
        }
    }

    /// Attach the active editor's selection (or, without one, the whole file as a link).
    pub(crate) fn ask_agent_about_selection(&mut self) {
        let Some(document) = self.editor.active() else {
            self.view.status = "Open a file to ask about it".into();
            return;
        };
        let (path, name) = (document.path.clone(), document.name.clone());
        #[cfg(feature = "shared-editor")]
        let selection = document.editing.as_ref().map(|buffer| {
            let set = buffer.selections();
            set.selections()[set.primary_index()].range()
        });
        #[cfg(not(feature = "shared-editor"))]
        let selection: Option<Range<usize>> = None;
        match selection.filter(|range| !range.is_empty()) {
            Some(range) => {
                match selection_attachment(path.as_deref(), &name, &document.text, &range) {
                    Ok(attachment) => self.add_attachment(attachment),
                    Err(error) => self.view.status = error,
                }
            }
            None => {
                match path {
                    Some(path) => {
                        let root = self.editor.workspace.clone();
                        self.add_attachment(file_attachment(&path, root.as_deref()));
                    }
                    None => self.view.status =
                        "Select some text first (untitled files can only be attached by selection)"
                            .into(),
                }
            }
        }
    }

    pub(crate) fn add_file_to_agent(&mut self, path: PathBuf) {
        let root = self.editor.workspace.clone();
        self.add_attachment(file_attachment(&path, root.as_deref()));
    }

    pub(crate) fn remove_agent_attachment(&mut self, kind: AgentKind, index: usize) {
        let attachments = &mut self.view.agents.session_mut(kind).attachments;
        if index < attachments.len() {
            attachments.remove(index);
        }
    }

    /// Prefill the composer with a request to fix one diagnostic, with the file attached.
    pub(crate) fn fix_problem_with_agent(
        &mut self,
        document: crate::model::DocumentId,
        range: Range<usize>,
    ) {
        let Some(file) = self.editor.files.iter().find(|file| file.id == document) else {
            return;
        };
        let (path, name, text) = (file.path.clone(), file.name.clone(), file.text.clone());
        let message = self
            .view
            .language_server_diagnostics
            .get(&document)
            .and_then(|list| list.iter().find(|d| d.range == range))
            .map(|diagnostic| diagnostic.message.clone())
            .unwrap_or_default();
        let (line, _) = line_span(&text, &range);
        let kind = self.open_agent_composer();
        let root = self.editor.workspace.clone();
        if let Some(path) = path {
            self.add_attachment(file_attachment(&path, root.as_deref()));
        }
        let session = self.view.agents.session_mut(kind);
        session.draft = format!("Fix this problem in {name} at line {line}:\n{message}\n");
        session.focus_composer = true;
    }

    /// Replace the `@query` being typed with the chosen path.
    pub(crate) fn pick_agent_mention(&mut self, kind: AgentKind, relative: &str) {
        let session = self.view.agents.session_mut(kind);
        session.draft = complete_mention(&session.draft, relative);
        session.mention_selected = 0;
        session.focus_composer = true;
    }

    /// Replace the partial `/command` with the chosen one, ready for arguments.
    pub(crate) fn pick_agent_command(&mut self, kind: AgentKind, name: &str) {
        let session = self.view.agents.session_mut(kind);
        session.draft = format!("/{name} ");
        session.slash_selected = 0;
        session.focus_composer = true;
    }

    /// Change a session option. Values that remove the approval step ask first.
    pub(crate) fn set_agent_config(&mut self, kind: AgentKind, id: &str, value: &str) {
        let session = self.view.agents.session_mut(kind);
        let Some(option) = session
            .conversation
            .config_options
            .iter()
            .find(|o| o.id == id)
        else {
            return;
        };
        if option.current == value {
            return;
        }
        let label = option
            .values
            .iter()
            .find(|v| v.value == value)
            .map_or_else(|| value.to_owned(), |v| v.name.clone());
        if RISKY_VALUES.contains(&value) {
            session.pending_config = Some((id.to_owned(), value.to_owned(), label));
            return;
        }
        self.send_agent_config(kind, id, value);
    }

    pub(crate) fn confirm_agent_config(&mut self, accepted: bool) {
        let kind = self.view.agents.active;
        let Some((id, value, _)) = self.view.agents.session_mut(kind).pending_config.take() else {
            return;
        };
        if accepted {
            self.send_agent_config(kind, &id, &value);
        }
    }

    fn send_agent_config(&mut self, kind: AgentKind, id: &str, value: &str) {
        let Some(session_id) = self.agent_session_id(kind) else {
            self.view.status = "Start the agent before changing its options".into();
            return;
        };
        let request = self.agent_register_request(kind, "session/set_config_option");
        self.agent_send(
            kind,
            &requests::set_config_option(request, &session_id, id, value),
        );
        Self::agent_log(
            kind,
            crate::model::output_log::LogLevel::Info,
            &format!("Set {id} to {value}"),
        );
    }

    /// Make sure file paths are available for `@` completion.
    pub(crate) fn ensure_mention_index(&mut self, ctx: &eframe::egui::Context) {
        self.ensure_file_index(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(name: &str) -> AvailableCommand {
        AvailableCommand {
            name: name.into(),
            description: String::new(),
            input_hint: None,
        }
    }

    #[test]
    fn a_mention_in_progress_is_the_last_word_starting_with_at() {
        assert_eq!(active_mention("look at @src/ma"), Some((8, "src/ma")));
        assert_eq!(active_mention("@"), Some((0, "")));
        assert_eq!(active_mention("done @a.rs and"), None);
        assert_eq!(
            active_mention("mail me@x.com"),
            None,
            "an @ inside a word is not a mention"
        );
        assert_eq!(active_mention(""), None);
        assert_eq!(
            complete_mention("look at @src/ma", "src/main.rs"),
            "look at @src/main.rs "
        );
        assert_eq!(complete_mention("plain", "a.rs"), "plain@a.rs ");
    }

    #[test]
    fn candidates_rank_name_matches_first_and_stay_inside_the_folder() {
        let root = Path::new("/w");
        let paths: Vec<PathBuf> = [
            "/w/src/main.rs",
            "/w/docs/main_notes.md",
            "/w/src/lib.rs",
            "/other/main.rs",
            "/w/tests/domain.rs",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        let found = mention_candidates(&paths, root, "main", 10);
        assert_eq!(
            found,
            ["src/main.rs", "docs/main_notes.md", "tests/domain.rs"]
        );
        assert_eq!(mention_candidates(&paths, root, "", 2).len(), 2);
        assert_eq!(
            mention_candidates(&paths, root, "sl", 10),
            ["src/lib.rs"],
            "subsequence matches count last"
        );
        assert!(mention_candidates(&paths, root, "zzz", 10).is_empty());
    }

    #[test]
    fn only_mentions_of_real_files_inside_the_folder_become_links() {
        let root = Path::new("/w");
        let exists =
            |path: &Path| path == Path::new("/w/src/a.rs") || path == Path::new("/w/b.txt");
        let links = mention_links(
            "see @src/a.rs, @b.txt. also @nope.rs @../secret @/etc/passwd mail@x.com @src/a.rs",
            root,
            exists,
        );
        let labels: Vec<_> = links.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(
            labels,
            ["src/a.rs", "b.txt"],
            "trailing punctuation trimmed, duplicates dropped"
        );
        assert_eq!(links[0].1["type"], "resource_link");
        assert!(links[0].1["uri"].as_str().unwrap().ends_with("/w/src/a.rs"));
    }

    #[test]
    fn slash_commands_filter_by_the_partial_name_until_a_space() {
        let commands = vec![
            command("review"),
            command("init"),
            command("compact"),
            command("review-branch"),
        ];
        let names = |draft: &str| {
            slash_matches(&commands, draft)
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names("/"), ["compact", "init", "review", "review-branch"]);
        assert_eq!(names("/rev"), ["review", "review-branch"]);
        assert_eq!(
            names("/it"),
            ["init"],
            "substring matches too, prefixes first"
        );
        assert!(
            names("/review now").is_empty(),
            "once arguments start, the popup closes"
        );
        assert!(names("review").is_empty());
    }

    #[test]
    fn line_spans_are_one_based_and_ignore_a_trailing_newline() {
        let text = "one\ntwo\nthree\n";
        assert_eq!(line_span(text, &(0..3)), (1, 1));
        assert_eq!(line_span(text, &(4..13)), (2, 3));
        assert_eq!(
            line_span(text, &(0..8)),
            (1, 2),
            "ending after the newline stays on line 2"
        );
        assert_eq!(line_span(text, &(4..4)), (2, 2));
    }

    #[test]
    fn selections_become_labelled_resource_blocks_with_line_anchors() {
        let text = "fn a() {}\nfn b() {}\n";
        let attachment =
            selection_attachment(Some(Path::new("/w/lib.rs")), "lib.rs", text, &(10..19)).unwrap();
        assert_eq!(attachment.label, "lib.rs:2");
        assert_eq!(attachment.block["resource"]["text"], "fn b() {}");
        assert!(
            attachment.block["resource"]["uri"]
                .as_str()
                .unwrap()
                .ends_with("/w/lib.rs#L2-L2")
        );
        let untitled = selection_attachment(None, "Untitled-1", text, &(0..19)).unwrap();
        assert_eq!(untitled.label, "Untitled-1:1-2");
        assert!(
            untitled.block["resource"]["uri"]
                .as_str()
                .unwrap()
                .starts_with("untitled:Untitled-1#L1-L2")
        );
        assert!(selection_attachment(None, "x", text, &(3..3)).is_err());
        let big = "x".repeat(MAX_SELECTION_BYTES + 1);
        assert!(
            selection_attachment(None, "x", &big, &(0..big.len()))
                .unwrap_err()
                .contains("too large")
        );
    }

    #[test]
    fn file_chips_show_the_folder_relative_path() {
        let chip = file_attachment(Path::new("/w/src/a.rs"), Some(Path::new("/w")));
        assert_eq!(chip.label, "src/a.rs");
        assert_eq!(chip.block["name"], "src/a.rs");
        assert_eq!(
            file_attachment(Path::new("/x/a.rs"), Some(Path::new("/w"))).label,
            "/x/a.rs"
        );
    }
}
