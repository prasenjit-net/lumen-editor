//! Bounded, read-only workspace file discovery for Quick Open.
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_FILES: usize = 100_000;

#[derive(Default)]
pub(crate) struct FileIndex {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) limited: bool,
    pub(crate) errors: usize,
}

pub(crate) fn scan(root: &Path, cancelled: &AtomicBool) -> FileIndex {
    let mut index = FileIndex::default();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .require_git(false)
        .git_global(false)
        .git_exclude(false)
        .follow_links(false)
        .filter_entry(|entry| {
            entry.depth() == 0 || !matches!(entry.file_name().to_str(), Some(".git" | "target"))
        })
        .build();
    for entry in walker {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        match entry {
            Ok(entry) if entry.file_type().is_some_and(|kind| kind.is_file()) => {
                index.paths.push(entry.into_path());
                if index.paths.len() >= MAX_FILES {
                    index.limited = true;
                    break;
                }
            }
            Ok(_) => (),
            Err(_) => index.errors += 1,
        }
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn indexes_files_without_generated_or_ignored_entries() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::create_dir(temp.path().join("target")).unwrap();
        fs::write(temp.path().join(".gitignore"), "ignored.rs\n").unwrap();
        fs::write(temp.path().join("src/main.rs"), "").unwrap();
        fs::write(temp.path().join("ignored.rs"), "").unwrap();
        fs::write(temp.path().join("target/output.rs"), "").unwrap();
        let index = scan(temp.path(), &AtomicBool::new(false));
        assert!(index.paths.contains(&temp.path().join("src/main.rs")));
        assert!(!index.paths.contains(&temp.path().join("ignored.rs")));
        assert!(!index.paths.contains(&temp.path().join("target/output.rs")));
    }

    #[test]
    fn cancelled_scan_keeps_no_paths() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("file.rs"), "").unwrap();
        assert!(scan(temp.path(), &AtomicBool::new(true)).paths.is_empty());
    }
}
