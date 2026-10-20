//! Sidebar lists that need their own Git reads: the selected repository's
//! stashes and the commits that differ from its upstream. They load in the
//! background whenever the repository's state changes, so the section headers
//! can show counts without the user opening anything.
use super::LumenApp;
use crate::services::source_control::{GitSnapshot, GitStash, SyncCommits};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::Duration,
};

/// What the loaded lists were read for; any difference means they are stale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExtrasKey {
    root: PathBuf,
    ahead: u32,
    behind: u32,
    /// The latest commit's subject, which changes with every commit.
    head: String,
    /// Bumped after every completed operation and manual refresh.
    generation: u64,
}

pub(crate) struct RepoExtras {
    key: ExtrasKey,
    pub(crate) stashes: Vec<GitStash>,
    pub(crate) sync: SyncCommits,
    pub(crate) error: Option<String>,
}

impl RepoExtras {
    #[cfg(test)]
    pub(crate) fn for_test(root: &str, stashes: Vec<GitStash>, sync: SyncCommits) -> Self {
        Self {
            key: ExtrasKey {
                root: PathBuf::from(root),
                ahead: 0,
                behind: 0,
                head: String::new(),
                generation: 0,
            },
            stashes,
            sync,
            error: None,
        }
    }

    pub(crate) fn root(&self) -> &std::path::Path {
        &self.key.root
    }
}

type ExtrasResult = (ExtrasKey, Result<(Vec<GitStash>, SyncCommits), String>);

#[derive(Default)]
pub(crate) struct ExtrasState {
    pub(crate) loaded: Option<RepoExtras>,
    /// See `ExtrasKey::generation`.
    pub(crate) generation: u64,
    requested: Option<ExtrasKey>,
    receiver: Option<Receiver<ExtrasResult>>,
}

fn key_for(snapshot: &GitSnapshot, generation: u64) -> ExtrasKey {
    let (ahead, behind) = snapshot
        .upstream
        .as_ref()
        .and_then(|upstream| upstream.ahead_behind)
        .unwrap_or((0, 0));
    ExtrasKey {
        root: snapshot.root.clone(),
        ahead,
        behind,
        head: snapshot.last_commit.clone().unwrap_or_default(),
        generation,
    }
}

impl LumenApp {
    /// Starts a read when the selected repository's state differs from what
    /// the lists were loaded for. Only called while Source Control is visible.
    pub(crate) fn ensure_repo_extras(&mut self, ctx: &egui::Context) {
        let extras = &mut self.view.source_control.extras;
        let Some(snapshot) = &self.view.source_control.snapshot else {
            extras.loaded = None;
            extras.requested = None;
            extras.receiver = None;
            return;
        };
        let key = key_for(snapshot, extras.generation);
        let current = extras.loaded.as_ref().map(|loaded| &loaded.key) == Some(&key)
            || extras.requested.as_ref() == Some(&key);
        if current {
            return;
        }
        let root = snapshot.root.clone();
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        let worker_key = key.clone();
        std::thread::spawn(move || {
            let result = service
                .stashes(&root)
                .and_then(|stashes| Ok((stashes, service.sync_commits(&root)?)));
            let _ = sender.send((worker_key, result));
            worker_ctx.request_repaint();
        });
        // Replacing the receiver drops any older, slower answer.
        extras.receiver = Some(receiver);
        extras.requested = Some(key);
    }

    pub(crate) fn poll_repo_extras(&mut self, ctx: &egui::Context) {
        let extras = &mut self.view.source_control.extras;
        let Some(receiver) = &extras.receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok((key, result)) => {
                extras.receiver = None;
                if extras.requested.as_ref() == Some(&key) {
                    extras.requested = None;
                    extras.loaded = Some(match result {
                        Ok((stashes, sync)) => RepoExtras {
                            key,
                            stashes,
                            sync,
                            error: None,
                        },
                        Err(error) => RepoExtras {
                            key,
                            stashes: Vec::new(),
                            sync: SyncCommits::default(),
                            error: Some(error),
                        },
                    });
                }
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(mpsc::TryRecvError::Disconnected) => {
                extras.receiver = None;
                extras.requested = None;
            }
        }
    }

    /// The lists are stale after any completed Git change or manual refresh.
    pub(crate) fn invalidate_repo_extras(&mut self) {
        self.view.source_control.extras.generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::source_control::GitUpstream;

    fn snapshot(ahead: u32, behind: u32, head: &str) -> GitSnapshot {
        GitSnapshot {
            root: PathBuf::from("/w/a"),
            upstream: Some(GitUpstream {
                name: "origin/main".into(),
                ahead_behind: Some((ahead, behind)),
            }),
            last_commit: Some(head.into()),
            ..Default::default()
        }
    }

    #[test]
    fn the_key_changes_with_counts_head_repository_and_generation() {
        let base = key_for(&snapshot(1, 0, "a"), 0);
        assert_eq!(base, key_for(&snapshot(1, 0, "a"), 0));
        assert_ne!(base, key_for(&snapshot(2, 0, "a"), 0));
        assert_ne!(base, key_for(&snapshot(1, 1, "a"), 0));
        assert_ne!(base, key_for(&snapshot(1, 0, "b"), 0));
        assert_ne!(base, key_for(&snapshot(1, 0, "a"), 1));
        let mut other = snapshot(1, 0, "a");
        other.root = PathBuf::from("/w/b");
        assert_ne!(base, key_for(&other, 0));
    }

    #[test]
    fn extras_load_once_per_state_and_reload_after_an_invalidation_or_a_change() {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        app.view.source_control.snapshot = Some(snapshot(1, 0, "a"));
        app.ensure_repo_extras(&ctx);
        assert!(app.view.source_control.extras.requested.is_some());
        let first = app.view.source_control.extras.requested.clone();
        app.ensure_repo_extras(&ctx);
        assert_eq!(
            app.view.source_control.extras.requested, first,
            "the same state is not requested twice"
        );

        // Wait for the (failing, nonexistent repository) read to be answered.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while app.view.source_control.extras.loaded.is_none()
            && std::time::Instant::now() < deadline
        {
            app.poll_repo_extras(&ctx);
            std::thread::sleep(Duration::from_millis(10));
        }
        let loaded = app.view.source_control.extras.loaded.as_ref().unwrap();
        assert_eq!(loaded.root(), std::path::Path::new("/w/a"));
        assert!(
            loaded.error.is_some(),
            "a failed read is reported, not hidden"
        );

        app.ensure_repo_extras(&ctx);
        assert!(app.view.source_control.extras.requested.is_none());
        app.invalidate_repo_extras();
        app.ensure_repo_extras(&ctx);
        assert!(app.view.source_control.extras.requested.is_some());

        app.view.source_control.snapshot = None;
        app.ensure_repo_extras(&ctx);
        assert!(app.view.source_control.extras.loaded.is_none());
    }
}
