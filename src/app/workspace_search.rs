//! Search lifecycle and guarded workspace replacement.
use super::LumenApp;
use crate::{
    model::search::SearchOptions,
    services::workspace_search::{self, SearchRequest},
};
use eframe::egui;
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) struct SearchJob {
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<SearchEvent>,
    request: SearchRequest,
}

enum SearchEvent {
    Progress(workspace_search::SearchProgress),
    Finished(workspace_search::SearchResult),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplaceScope {
    File(PathBuf),
    Folder(PathBuf),
}

impl LumenApp {
    pub(crate) fn open_workspace_hit(&mut self, path: PathBuf, range: Range<usize>) {
        self.open_file(path.clone());
        let Some(file) = self
            .editor
            .active()
            .filter(|file| file.path.as_ref() == Some(&path))
        else {
            return;
        };
        let Some(request) = &self.view.workspace_search.completed_request else {
            return;
        };
        let state = self.view.find.entry(file.id).or_default();
        state.query.clone_from(&request.query);
        state.options = request.options;
        state.refresh(file);
        if let Some(index) = state.matches.iter().position(|found| *found == range) {
            state.current = index;
            state.reveal = true;
            state.open = false;
            self.view.focus_editor = true;
        } else {
            self.view.status = "Search result is stale; search again".into();
        }
    }

    pub(crate) fn start_workspace_search(&mut self, ctx: &egui::Context) {
        self.cancel_workspace_search();
        let exclude = self.combined_workspace_exclude();
        let view = &mut self.view.workspace_search;
        view.results = None;
        view.completed_request = None;
        view.selected.clear();
        view.preview_open = false;
        view.confirmation_open = false;
        view.preview_replacement = None;
        view.preview_selected.clear();
        view.preview_page = 0;
        view.report.clear();
        view.replaced_files = 0;
        view.progress = None;
        let Some(root) = self.editor.workspace.clone() else {
            self.view.status = "Open a folder to search its files".into();
            return;
        };
        if view.query.is_empty() {
            return;
        }
        let request = SearchRequest {
            root,
            query: view.query.clone(),
            include: view.include.clone(),
            exclude,
            options: SearchOptions {
                match_case: view.match_case,
                whole_word: view.whole_word,
            },
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        // Drop intermediate snapshots if the UI is busy; the final result is never dropped.
        let (sender, receiver) = mpsc::sync_channel(8);
        let worker_request = request.clone();
        std::thread::spawn(move || {
            let result = workspace_search::search_with_progress(
                &worker_request,
                &worker_cancelled,
                |progress| {
                    let _ = sender.try_send(SearchEvent::Progress(progress));
                },
            );
            let _ = sender.send(SearchEvent::Finished(result));
        });
        self.search_job = Some(SearchJob {
            cancelled,
            receiver,
            request,
        });
        view.running = true;
        self.view.status = "Searching workspace…".into();
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    pub(crate) fn cancel_workspace_search(&mut self) {
        if let Some(job) = self.search_job.take() {
            job.cancelled.store(true, Ordering::Relaxed);
            self.view.status = "Workspace search cancelled".into();
        }
        self.view.workspace_search.running = false;
        self.view.workspace_search.progress = None;
    }

    pub(crate) fn poll_workspace_search(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.search_job else { return };
        let mut finished = None;
        let mut disconnected = false;
        loop {
            match job.receiver.try_recv() {
                Ok(SearchEvent::Progress(progress)) => {
                    self.view.workspace_search.progress = Some(progress);
                }
                Ok(SearchEvent::Finished(result)) => {
                    finished = Some((job.request.clone(), result));
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if let Some((request, result)) = finished {
            let matches: usize = result.files.iter().map(|file| file.hits.len()).sum();
            let view = &mut self.view.workspace_search;
            view.completed_request = Some(request.clone());
            view.selected = result.files.iter().map(|file| file.path.clone()).collect();
            view.expanded_files = view.selected.clone();
            view.expanded_folders = result
                .files
                .iter()
                .filter_map(|file| file.path.parent())
                .flat_map(|folder| {
                    folder
                        .ancestors()
                        .take_while(|ancestor| ancestor.starts_with(&request.root))
                        .map(Path::to_path_buf)
                })
                .collect();
            view.running = false;
            view.progress = None;
            view.results = Some(result);
            self.search_job = None;
            self.view.status = format!("{matches} workspace match(es)");
        } else if disconnected {
            self.search_job = None;
            self.view.workspace_search.running = false;
            self.view.workspace_search.progress = None;
            self.view.status = "Workspace search stopped unexpectedly".into();
        } else {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    pub(crate) fn preview_workspace_replace(&mut self) {
        self.view.workspace_search.selected = self
            .view
            .workspace_search
            .results
            .as_ref()
            .map(|result| result.files.iter().map(|file| file.path.clone()).collect())
            .unwrap_or_default();
        self.open_workspace_replace_preview();
    }

    pub(crate) fn preview_workspace_replace_scope(&mut self, scope: ReplaceScope) {
        self.view.workspace_search.selected = self
            .view
            .workspace_search
            .results
            .as_ref()
            .map(|result| {
                result
                    .files
                    .iter()
                    .filter(|file| match &scope {
                        ReplaceScope::File(path) => &file.path == path,
                        ReplaceScope::Folder(folder) => file.path.starts_with(folder),
                    })
                    .map(|file| file.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.open_workspace_replace_preview();
    }

    fn open_workspace_replace_preview(&mut self) {
        let current = self.workspace_search_current();
        let view = &mut self.view.workspace_search;
        view.preview_open = view.results.is_some() && !view.running && current;
        view.confirmation_open = false;
        view.preview_replacement = view.preview_open.then(|| view.replacement.clone());
        view.preview_selected = view.selected.clone();
        view.preview_page = 0;
        if !view.preview_open {
            self.view.status = "Search inputs changed; run Search again before replacing".into();
        }
    }

    pub(crate) fn apply_workspace_replace(&mut self) {
        if !self.view.workspace_search.confirmation_open {
            return;
        }
        if !self.workspace_search_current()
            || self.view.workspace_search.preview_replacement.as_ref()
                != Some(&self.view.workspace_search.replacement)
            || self.view.workspace_search.preview_selected != self.view.workspace_search.selected
        {
            self.view.workspace_search.confirmation_open = false;
            self.view.status =
                "Search or replacement choices changed; preview again before replacing".into();
            return;
        }
        let view = &mut self.view.workspace_search;
        view.confirmation_open = false;
        let Some(results) = &view.results else { return };
        let mut report = Vec::new();
        let mut changed = 0;
        for file in &results.files {
            if !view.selected.contains(&file.path) {
                continue;
            }
            if self
                .editor
                .files
                .iter()
                .any(|open| open.path.as_deref() == Some(file.path.as_path()))
            {
                report.push(format!(
                    "{}: open editor tab; close or save it before workspace replacement",
                    file.path.display()
                ));
                continue;
            }
            let ranges: Vec<_> = file.hits.iter().map(|hit| hit.range.clone()).collect();
            if !grapheme_safe(&file.original, &ranges) {
                report.push(format!(
                    "{}: a match splits a combined character",
                    file.path.display()
                ));
                continue;
            }
            let Some(replacement) =
                crate::model::search::replace_matches(&file.original, &ranges, &view.replacement)
            else {
                report.push(format!("{}: invalid match range", file.path.display()));
                continue;
            };
            let root = self
                .editor
                .workspace
                .as_deref()
                .expect("search root was validated");
            match self.file_system.write_workspace_replace(
                root,
                &file.path,
                &file.original,
                &replacement,
            ) {
                Ok(()) => changed += 1,
                Err(error) => report.push(format!("{}: {error}", file.path.display())),
            }
        }
        view.report = report;
        view.replaced_files = changed;
        view.preview_open = false;
        self.view.status = if view.report.is_empty() {
            format!("Replaced matches in {changed} file(s)")
        } else {
            format!(
                "Replaced {changed} file(s); {} skipped or failed — review Search",
                view.report.len()
            )
        };
    }

    fn workspace_search_current(&self) -> bool {
        let view = &self.view.workspace_search;
        view.completed_request.as_ref().is_some_and(|request| {
            self.editor.workspace.as_ref() == Some(&request.root)
                && request.query == view.query
                && request.include == view.include
                && request.exclude == self.combined_workspace_exclude()
                && request.options.match_case == view.match_case
                && request.options.whole_word == view.whole_word
        })
    }

    pub(crate) fn combined_workspace_exclude(&self) -> String {
        let configured = self.view.settings.search_exclude();
        [configured.trim(), self.view.workspace_search.exclude.trim()]
            .into_iter()
            .filter(|patterns| !patterns.is_empty())
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;

    #[test]
    fn progress_updates_until_final_result_and_cancel_drops_old_events() {
        let mut app = LumenApp::default();
        let request = SearchRequest {
            root: PathBuf::from("/workspace"),
            query: "term".into(),
            include: String::new(),
            exclude: String::new(),
            options: SearchOptions::default(),
        };
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        app.search_job = Some(SearchJob {
            cancelled: cancelled.clone(),
            receiver,
            request,
        });
        app.view.workspace_search.running = true;
        sender
            .send(SearchEvent::Progress(workspace_search::SearchProgress {
                scanned: 64,
                skipped: 2,
                matches: 5,
            }))
            .unwrap();
        let ctx = egui::Context::default();
        app.poll_workspace_search(&ctx);
        assert_eq!(app.view.workspace_search.progress.unwrap().scanned, 64);
        assert!(app.view.workspace_search.running);

        sender
            .send(SearchEvent::Finished(
                workspace_search::SearchResult::default(),
            ))
            .unwrap();
        app.poll_workspace_search(&ctx);
        assert!(!app.view.workspace_search.running);
        assert!(app.view.workspace_search.progress.is_none());
        assert!(app.search_job.is_none());

        let (sender, receiver) = mpsc::channel();
        app.search_job = Some(SearchJob {
            cancelled: cancelled.clone(),
            receiver,
            request: SearchRequest {
                root: PathBuf::from("/workspace"),
                query: "term".into(),
                include: String::new(),
                exclude: String::new(),
                options: SearchOptions::default(),
            },
        });
        app.view.workspace_search.running = true;
        app.cancel_workspace_search();
        assert!(cancelled.load(Ordering::Relaxed));
        assert!(app.view.workspace_search.progress.is_none());
        assert!(!app.view.workspace_search.running);
        assert!(
            sender
                .send(SearchEvent::Progress(Default::default()))
                .is_err()
        );
    }
}

fn grapheme_safe(text: &str, ranges: &[std::ops::Range<usize>]) -> bool {
    let mut boundaries: Vec<_> = text
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .collect();
    boundaries.push(text.len());
    ranges.iter().all(|range| {
        boundaries.binary_search(&range.start).is_ok()
            && boundaries.binary_search(&range.end).is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_partial_combined_characters() {
        assert!(!grapheme_safe("e\u{301}", std::slice::from_ref(&(0..1))));
        assert!(grapheme_safe("e\u{301}", std::slice::from_ref(&(0..3))));
    }
}
