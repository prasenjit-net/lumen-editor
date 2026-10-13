//! Quick Open indexing/ranking and document-safe line navigation.
use super::{
    LumenApp,
    view_state::{NavigationMode, NavigationView},
};
use crate::{
    model::DocumentId,
    services::file_index::{self, FileIndex},
    services::recent_files::{LocalRecentFilesStore, RecentFilesStore},
};
use eframe::egui;
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};
use unicode_segmentation::UnicodeSegmentation;

const MAX_RESULTS: usize = 60;
const MAX_RECENT: usize = 30;

pub(crate) struct FileIndexJob {
    root: PathBuf,
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<FileIndex>,
}

impl LumenApp {
    pub(crate) fn initialize_recent_files(&mut self, settings_path: Option<PathBuf>) {
        let Some(settings_path) = settings_path else {
            return;
        };
        let store = Box::new(LocalRecentFilesStore::new(&settings_path));
        match store.load() {
            Ok(paths) => {
                for path in paths.into_iter().rev() {
                    if path.is_absolute() {
                        self.view.navigation.remember(&path);
                    }
                }
            }
            Err(error) => {
                self.view.navigation.recent_warning =
                    Some(format!("Could not load recent files: {error}"));
            }
        }
        self.recent_files_store = Some(store);
    }

    pub(crate) fn remember_recent_file(&mut self, path: &Path) {
        self.view.navigation.remember(path);
        self.save_recent_files();
    }

    pub(crate) fn save_recent_files(&mut self) {
        if let Some(store) = &self.recent_files_store {
            match store.save(&self.view.navigation.recent) {
                Ok(()) => self.view.navigation.recent_warning = None,
                Err(error) => {
                    self.view.navigation.recent_warning =
                        Some(format!("Could not save recent files: {error}"));
                }
            }
        }
    }

    pub(crate) fn open_quick_open(&mut self, ctx: &egui::Context) {
        self.view.command_palette = false;
        let navigation = &mut self.view.navigation;
        navigation.mode = Some(NavigationMode::QuickOpen);
        navigation.query.clear();
        navigation.error = None;
        navigation.selected = 0;
        navigation.focus = true;
        navigation.matched_query = None;
        let root = self.editor.workspace.clone();
        if navigation.indexed_root != root {
            self.cancel_file_index();
            self.view.navigation.indexed_root = root.clone();
            self.view.navigation.paths.clear();
            self.view.navigation.running = root.is_some();
            if let Some(root) = root {
                let cancelled = Arc::new(AtomicBool::new(false));
                let worker_cancelled = Arc::clone(&cancelled);
                let (sender, receiver) = mpsc::channel();
                let scan_root = root.clone();
                std::thread::spawn(move || {
                    let result = file_index::scan(&scan_root, &worker_cancelled);
                    let _ = sender.send(result);
                });
                self.file_index_job = Some(FileIndexJob {
                    root,
                    cancelled,
                    receiver,
                });
                ctx.request_repaint();
            }
        }
        self.view
            .navigation
            .refresh(self.editor.workspace.as_deref());
    }

    pub(crate) fn open_go_to_line(&mut self) {
        if self.editor.active().is_none() || self.view.settings.open {
            self.view.status = "Open an editor to go to a line".into();
            return;
        }
        self.view.command_palette = false;
        self.view.navigation.mode = Some(NavigationMode::GoToLine);
        self.view.navigation.query.clear();
        self.view.navigation.error = None;
        self.view.navigation.focus = true;
    }

    pub(crate) fn go_to_line(&mut self, document: DocumentId, input: &str) {
        let Some(file) = self.editor.active().filter(|file| file.id == document) else {
            self.view.navigation.error = Some("Editor changed; try again".into());
            return;
        };
        match parse_location(input) {
            Ok((line, column)) => {
                let byte = location_byte(&file.text, line, column);
                self.view.pending_goto = Some((document, byte));
                self.view.navigation.mode = None;
                self.view.focus_editor = true;
                self.view.status = "Caret moved".into();
            }
            Err(error) => {
                self.view.navigation.error = Some(error.into());
                self.view.navigation.focus = true;
            }
        }
    }

    pub(crate) fn open_quick_path(&mut self, path: PathBuf) {
        self.open_file(path.clone());
        if self
            .editor
            .active()
            .is_some_and(|file| file.path.as_ref() == Some(&path))
        {
            self.view.navigation.mode = None;
            self.view.navigation.error = None;
            self.view.focus_editor = true;
        } else {
            self.view.navigation.error = Some(self.view.status.clone());
            self.view.navigation.focus = true;
            if !self.file_system.entry_exists(&path) {
                self.view.navigation.forget_deleted(&path);
                self.save_recent_files();
            }
        }
    }

    pub(crate) fn cancel_file_index(&mut self) {
        if let Some(job) = self.file_index_job.take() {
            job.cancelled.store(true, Ordering::Relaxed);
        }
        self.view.navigation.running = false;
    }

    pub(crate) fn poll_file_index(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.file_index_job else {
            return;
        };
        match job.receiver.try_recv() {
            Ok(index) => {
                let root = job.root.clone();
                self.file_index_job = None;
                if self.editor.workspace.as_ref() == Some(&root) {
                    let navigation = &mut self.view.navigation;
                    navigation.paths = index.paths;
                    navigation.limited = index.limited;
                    navigation.errors = index.errors;
                    navigation.running = false;
                    navigation.matched_query = None;
                    navigation.refresh(Some(&root));
                    ctx.request_repaint();
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.file_index_job = None;
                self.view.navigation.running = false;
            }
            Err(mpsc::TryRecvError::Empty) => {
                if self.view.navigation.mode == Some(NavigationMode::QuickOpen) {
                    ctx.request_repaint_after(std::time::Duration::from_millis(75));
                }
            }
        }
    }
}

impl NavigationView {
    pub(crate) fn remember(&mut self, path: &Path) {
        self.recent.retain(|item| item != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(MAX_RECENT);
        self.matched_query = None;
    }

    pub(crate) fn add_workspace_path(&mut self, path: &Path) {
        if self
            .indexed_root
            .as_ref()
            .is_some_and(|root| path.starts_with(root))
            && !self.paths.iter().any(|candidate| candidate == path)
        {
            self.paths.push(path.to_path_buf());
            self.matched_query = None;
        }
    }

    pub(crate) fn relocate(&mut self, source: &Path, target: &Path) {
        for path in self.recent.iter_mut().chain(self.paths.iter_mut()) {
            if let Ok(suffix) = path.strip_prefix(source) {
                *path = target.join(suffix);
            }
        }
        self.matched_query = None;
    }

    pub(crate) fn forget_deleted(&mut self, path: &Path) {
        self.recent.retain(|candidate| !candidate.starts_with(path));
        self.paths.retain(|candidate| !candidate.starts_with(path));
        self.matched_query = None;
    }

    pub(crate) fn refresh(&mut self, root: Option<&Path>) {
        if self.matched_query.as_deref() == Some(&self.query) {
            return;
        }
        let mut seen = HashSet::new();
        let recent: HashMap<&Path, usize> = self
            .recent
            .iter()
            .enumerate()
            .map(|(index, path)| (path.as_path(), index))
            .collect();
        let mut best = BinaryHeap::new();
        for path in self.recent.iter().chain(self.paths.iter()) {
            let path = path.as_path();
            if !seen.insert(path) {
                continue;
            }
            let relative = root
                .and_then(|root| path.strip_prefix(root).ok())
                .unwrap_or(path);
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if let Some(score) = fuzzy_score(name, &relative.to_string_lossy(), &self.query) {
                let workspace_bonus = if !self.query.trim().is_empty()
                    && root.is_some_and(|root| path.starts_with(root))
                {
                    600
                } else {
                    0
                };
                let rank = (
                    score
                        + workspace_bonus
                        + recent.get(path).map_or(0, |index| 500 - *index as i64),
                    Reverse(path),
                );
                if best.len() < MAX_RESULTS {
                    best.push(Reverse(rank));
                } else if best.peek().is_some_and(|worst| rank > worst.0) {
                    best.pop();
                    best.push(Reverse(rank));
                }
            }
        }
        let mut ranked: Vec<_> = best
            .into_iter()
            .map(|Reverse((score, Reverse(path)))| (score, path))
            .collect();
        ranked.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1)));
        self.matches = ranked
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, path)| path.to_path_buf())
            .collect();
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
        self.matched_query = Some(self.query.clone());
    }
}

fn fuzzy_score(name: &str, path: &str, query: &str) -> Option<i64> {
    if query.trim().is_empty() {
        return Some(0);
    }
    let query = query.to_lowercase();
    let name = name.to_lowercase();
    let path = path.to_lowercase().replace('\\', "/");
    let haystack = if is_subsequence(&name, &query) {
        &name
    } else {
        &path
    };
    let mut score = if haystack == &name { 200 } else { 0 };
    let mut chars = haystack.chars().enumerate();
    let mut previous = None;
    for needle in query.chars().filter(|ch| !ch.is_whitespace()) {
        let (index, _) = chars.find(|(_, ch)| *ch == needle)?;
        score += if index == 0 { 25 } else { 0 };
        score += if previous.is_some_and(|previous| index == previous + 1) {
            12
        } else {
            0
        };
        if index > 0 && matches!(haystack.chars().nth(index - 1), Some('/' | '_' | '-' | '.')) {
            score += 20;
        }
        previous = Some(index);
    }
    Some(score - haystack.chars().count() as i64)
}

fn is_subsequence(haystack: &str, query: &str) -> bool {
    let mut chars = haystack.chars();
    query
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .all(|needle| chars.by_ref().any(|ch| ch == needle))
}

fn parse_location(input: &str) -> Result<(usize, usize), &'static str> {
    let mut parts = input.trim().split(':');
    let line = parts
        .next()
        .unwrap_or("")
        .parse::<usize>()
        .map_err(|_| "Enter a line number, optionally followed by :column")?;
    let column = match parts.next() {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| "Column must be a positive number")?,
        None => 1,
    };
    if line == 0 || column == 0 || parts.next().is_some() {
        return Err("Use a positive line[:column]");
    }
    Ok((line, column))
}

fn location_byte(text: &str, line: usize, column: usize) -> usize {
    let starts = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(index, _)| index + 1))
        .collect::<Vec<_>>();
    let start = starts[line.saturating_sub(1).min(starts.len() - 1)];
    let end = starts
        .iter()
        .copied()
        .find(|next| *next > start)
        .map_or(text.len(), |next| next - 1);
    let content = text[start..end]
        .strip_suffix('\r')
        .unwrap_or(&text[start..end]);
    let offset = content
        .grapheme_indices(true)
        .nth(column - 1)
        .map_or(content.len(), |(index, _)| index);
    start + offset
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranking_prefers_filename_and_recent_paths() {
        assert!(
            fuzzy_score("main.rs", "src/main.rs", "main").unwrap()
                > fuzzy_score("other.rs", "main/other.rs", "main").unwrap()
        );
        assert!(fuzzy_score("main.rs", "src/main.rs", "mn").is_some());
        assert!(fuzzy_score("main.rs", "src/main.rs", "xyz").is_none());
        let mut view = NavigationView {
            paths: vec!["/project/src/main.rs".into(), "/project/test.rs".into()],
            ..Default::default()
        };
        view.remember(Path::new("/project/test.rs"));
        view.refresh(Some(Path::new("/project")));
        assert_eq!(view.matches[0], PathBuf::from("/project/test.rs"));
    }
    #[test]
    fn matching_workspace_files_rank_above_external_recents() {
        let mut view = NavigationView {
            query: "main".into(),
            paths: vec!["/project/src/main.rs".into()],
            ..Default::default()
        };
        view.remember(Path::new("/elsewhere/main.rs"));
        view.refresh(Some(Path::new("/project")));
        assert_eq!(view.matches[0], PathBuf::from("/project/src/main.rs"));
    }
    #[test]
    fn location_is_one_based_clamped_and_grapheme_safe() {
        assert_eq!(parse_location("2:3"), Ok((2, 3)));
        assert!(parse_location("0").is_err());
        assert!(parse_location("2:0").is_err());
        assert_eq!(
            location_byte("a\r\ne\u{301}x\n", 2, 2),
            "a\r\ne\u{301}".len()
        );
        assert_eq!(location_byte("a\nlast", 99, 99), "a\nlast".len());
    }

    #[test]
    fn large_index_keeps_only_top_bounded_results() {
        let mut view = NavigationView {
            query: "main".into(),
            paths: (0..20_000)
                .map(|number| PathBuf::from(format!("/project/module_{number}/main.rs")))
                .collect(),
            ..Default::default()
        };
        view.refresh(Some(Path::new("/project")));
        assert_eq!(view.matches.len(), MAX_RESULTS);
        assert!(
            view.matches
                .iter()
                .all(|path| path.file_name().unwrap() == "main.rs")
        );
    }

    #[test]
    fn explorer_moves_and_deletes_update_recent_and_indexed_paths() {
        let mut view = NavigationView {
            indexed_root: Some("/project".into()),
            paths: vec!["/project/src/main.rs".into()],
            recent: vec!["/project/src/main.rs".into()],
            ..Default::default()
        };
        view.relocate(Path::new("/project/src"), Path::new("/project/code"));
        assert_eq!(view.recent, vec![PathBuf::from("/project/code/main.rs")]);
        assert_eq!(view.paths, view.recent);
        view.forget_deleted(Path::new("/project/code"));
        assert!(view.paths.is_empty() && view.recent.is_empty());
    }
}
