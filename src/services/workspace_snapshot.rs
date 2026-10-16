//! Point-in-time copy of a folder's text files, used to show what a coding agent
//! changed during one turn.
//!
//! Agents (Codex above all) edit through their own shell commands, so Lumen cannot
//! see writes as they happen. Instead it snapshots before and after a turn and
//! diffs the two. Captures are incremental: a file whose size and modified time
//! are unchanged since the previous snapshot is reused without being read again.
//! Everything is bounded; a folder too large to track is reported as truncated.
use ignore::WalkBuilder;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

#[derive(Clone, Debug)]
pub(crate) struct Limits {
    pub(crate) max_files: usize,
    pub(crate) max_file_bytes: u64,
    pub(crate) max_total_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_files: 50_000,
            max_file_bytes: 1024 * 1024,
            max_total_bytes: 128 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    size: u64,
    modified: Option<SystemTime>,
    /// `None` for binary, oversized or unreadable files: changes are still
    /// detected, but no text diff or revert is possible.
    text: Option<Arc<str>>,
}

#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    root: PathBuf,
    entries: BTreeMap<PathBuf, Entry>,
    /// Files or bytes beyond the limits were not tracked, or the scan was cancelled.
    pub(crate) truncated: bool,
}

impl Snapshot {
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Files modified this recently are re-read even when size and time look equal:
/// coarse file-system clocks can hide a second write (git's "racy clean" rule).
const RACY_WINDOW: Duration = Duration::from_secs(2);

/// Scan `root`. `previous` (same root) makes it incremental. Setting `cancel`
/// stops the walk early and marks the result truncated.
pub(crate) fn capture(
    root: &Path,
    previous: Option<&Snapshot>,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Snapshot {
    let captured_at = SystemTime::now();
    let previous = previous.filter(|snapshot| snapshot.root == root);
    let mut snapshot = Snapshot {
        root: root.to_path_buf(),
        entries: BTreeMap::new(),
        truncated: false,
    };
    let mut total_text = 0_u64;
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_global(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    for result in walker {
        if cancel.load(Ordering::Relaxed) {
            snapshot.truncated = true;
            break;
        }
        let Ok(entry) = result else { continue };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if snapshot.entries.len() >= limits.max_files {
            snapshot.truncated = true;
            break;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let (size, modified) = (metadata.len(), metadata.modified().ok());
        let reuse = previous
            .and_then(|snapshot| snapshot.entries.get(entry.path()))
            .filter(|old| {
                old.size == size
                    && old.modified == modified
                    && modified.is_none_or(|time| {
                        captured_at
                            .duration_since(time)
                            .is_ok_and(|age| age > RACY_WINDOW)
                    })
            });
        let text = match reuse {
            Some(old) => old.text.clone(),
            None => read_text(entry.path(), size, limits),
        };
        if let Some(text) = &text {
            total_text += text.len() as u64;
            if total_text > limits.max_total_bytes {
                snapshot.truncated = true;
                snapshot.entries.insert(
                    entry.path().to_path_buf(),
                    Entry {
                        size,
                        modified,
                        text: None,
                    },
                );
                continue;
            }
        }
        snapshot.entries.insert(
            entry.path().to_path_buf(),
            Entry {
                size,
                modified,
                text,
            },
        );
    }
    snapshot
}

fn read_text(path: &Path, size: u64, limits: &Limits) -> Option<Arc<str>> {
    if size > limits.max_file_bytes {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok().map(Arc::from)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone, Debug)]
pub(crate) struct FileChange {
    pub(crate) path: PathBuf,
    pub(crate) kind: ChangeKind,
    pub(crate) before: Option<Arc<str>>,
    pub(crate) after: Option<Arc<str>>,
    /// Binary, huge or unreadable: no text diff, so it cannot be reverted by Lumen.
    pub(crate) opaque: bool,
}

/// What differs between two snapshots of the same folder, sorted by path.
pub(crate) fn changes(before: &Snapshot, after: &Snapshot) -> Vec<FileChange> {
    let mut result = Vec::new();
    for (path, new) in &after.entries {
        match before.entries.get(path) {
            None => result.push(FileChange {
                path: path.clone(),
                kind: ChangeKind::Added,
                before: None,
                after: new.text.clone(),
                opaque: new.text.is_none(),
            }),
            Some(old) => {
                let differs = match (&old.text, &new.text) {
                    (Some(a), Some(b)) => a != b,
                    _ => old.size != new.size || old.modified != new.modified,
                };
                if differs {
                    result.push(FileChange {
                        path: path.clone(),
                        kind: ChangeKind::Modified,
                        before: old.text.clone(),
                        after: new.text.clone(),
                        opaque: old.text.is_none() || new.text.is_none(),
                    });
                }
            }
        }
    }
    for (path, old) in &before.entries {
        if !after.entries.contains_key(path) {
            result.push(FileChange {
                path: path.clone(),
                kind: ChangeKind::Deleted,
                before: old.text.clone(),
                after: None,
                opaque: old.text.is_none(),
            });
        }
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(root: &Path, previous: Option<&Snapshot>) -> Snapshot {
        capture(root, previous, &Limits::default(), &AtomicBool::new(false))
    }

    fn write(root: &Path, name: &str, text: &str) {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn summary(changes: &[FileChange], root: &Path) -> Vec<String> {
        changes
            .iter()
            .map(|change| {
                format!(
                    "{:?} {}",
                    change.kind,
                    change.path.strip_prefix(root).unwrap().display()
                )
            })
            .collect()
    }

    #[test]
    fn added_modified_and_deleted_files_are_found_with_both_sides_of_the_text() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "keep.txt", "same");
        write(root, "edit.rs", "fn a() {}\n");
        write(root, "gone.txt", "bye");
        let before = scan(root, None);
        write(root, "edit.rs", "fn a() { 1 }\n");
        write(root, "src/new.rs", "new");
        std::fs::remove_file(root.join("gone.txt")).unwrap();
        let after = scan(root, Some(&before));
        let found = changes(&before, &after);
        assert_eq!(
            summary(&found, root),
            ["Modified edit.rs", "Deleted gone.txt", "Added src/new.rs"]
        );
        assert_eq!(found[0].before.as_deref(), Some("fn a() {}\n"));
        assert_eq!(found[0].after.as_deref(), Some("fn a() { 1 }\n"));
        assert_eq!(found[1].before.as_deref(), Some("bye"));
        assert_eq!(found[2].after.as_deref(), Some("new"));
    }

    #[test]
    fn nothing_changed_means_no_changes_and_unchanged_files_are_reused() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.txt", "text");
        let first = scan(dir.path(), None);
        // Wait out the racy window so reuse is allowed for this file.
        std::thread::sleep(RACY_WINDOW + Duration::from_millis(100));
        let first = scan(dir.path(), Some(&first));
        let second = scan(dir.path(), Some(&first));
        assert!(changes(&first, &second).is_empty());
        let (a, b) = (
            first.entries.values().next().unwrap().text.clone().unwrap(),
            second
                .entries
                .values()
                .next()
                .unwrap()
                .text
                .clone()
                .unwrap(),
        );
        assert!(Arc::ptr_eq(&a, &b), "not read a second time");
    }

    #[test]
    fn a_touch_without_a_content_change_is_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.txt", "text");
        let before = scan(dir.path(), None);
        write(dir.path(), "a.txt", "text"); // new mtime, same bytes
        let after = scan(dir.path(), Some(&before));
        assert!(changes(&before, &after).is_empty());
    }

    #[test]
    fn a_rewrite_right_after_the_snapshot_is_still_detected() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.txt", "aaaa");
        let before = scan(dir.path(), None);
        write(dir.path(), "a.txt", "bbbb"); // same size, possibly the same coarse mtime
        let after = scan(dir.path(), Some(&before));
        let found = changes(&before, &after);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].after.as_deref(), Some("bbbb"));
    }

    #[test]
    fn ignored_files_and_git_internals_are_not_tracked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, ".gitignore", "target/\n*.log\n");
        write(root, "src/main.rs", "fn main() {}");
        write(root, "target/debug/out", "artifact");
        write(root, "run.log", "noise");
        write(root, ".git/HEAD", "ref");
        write(root, ".env", "KEY=1");
        let snapshot = scan(root, None);
        let names: Vec<_> = snapshot
            .entries
            .keys()
            .map(|path| path.strip_prefix(root).unwrap().display().to_string())
            .collect();
        assert_eq!(names, [".env", ".gitignore", "src/main.rs"]);
    }

    #[test]
    fn binary_and_oversized_files_are_detected_but_have_no_text() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("image.bin"), [0_u8, 1, 2, 3]).unwrap();
        write(root, "big.txt", &"x".repeat(100));
        let limits = Limits {
            max_file_bytes: 50,
            ..Limits::default()
        };
        let cancel = AtomicBool::new(false);
        let before = capture(root, None, &limits, &cancel);
        std::fs::write(root.join("image.bin"), [0_u8, 9, 9, 9, 9]).unwrap();
        write(root, "big.txt", &"y".repeat(101));
        let after = capture(root, Some(&before), &limits, &cancel);
        let found = changes(&before, &after);
        assert_eq!(found.len(), 2);
        assert!(
            found
                .iter()
                .all(|change| change.opaque && change.kind == ChangeKind::Modified)
        );
    }

    #[test]
    fn limits_and_cancellation_mark_the_snapshot_truncated() {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..5 {
            write(dir.path(), &format!("f{n}.txt"), "x");
        }
        let few = Limits {
            max_files: 3,
            ..Limits::default()
        };
        let capped = capture(dir.path(), None, &few, &AtomicBool::new(false));
        assert!(capped.truncated && capped.len() == 3);
        let tiny = Limits {
            max_total_bytes: 2,
            ..Limits::default()
        };
        assert!(capture(dir.path(), None, &tiny, &AtomicBool::new(false)).truncated);
        let cancelled = capture(dir.path(), None, &Limits::default(), &AtomicBool::new(true));
        assert!(cancelled.truncated && cancelled.len() == 0);
    }

    #[test]
    fn a_snapshot_of_a_different_root_is_never_reused() {
        let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(one.path(), "a.txt", "one");
        write(two.path(), "a.txt", "two");
        let first = scan(one.path(), None);
        let second = scan(two.path(), Some(&first));
        assert_eq!(second.root(), two.path());
        assert_eq!(
            second.entries.values().next().unwrap().text.as_deref(),
            Some("two")
        );
    }
}
