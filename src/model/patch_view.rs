//! Reads the patch text a tool call carries into per-file, per-line changes so
//! the chat can colour additions and deletions. Tolerant by design: providers
//! send unified diffs with or without file headers, and anything that has no
//! changed line is left to the plain-text view.
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PatchLineKind {
    Hunk,
    Added,
    Removed,
    Context,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PatchLine {
    pub(crate) kind: PatchLineKind,
    pub(crate) text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PatchFile {
    /// The path as the patch names it; empty when it names none.
    pub(crate) path: String,
    pub(crate) added: usize,
    pub(crate) removed: usize,
    pub(crate) lines: Vec<PatchLine>,
}

impl PatchFile {
    fn new(path: String) -> Self {
        Self {
            path,
            added: 0,
            removed: 0,
            lines: Vec::new(),
        }
    }

    /// File name to show; falls back to a generic label for header-less patches.
    pub(crate) fn display_name(&self) -> &str {
        let name = self.path.rsplit('/').next().unwrap_or_default();
        if name.is_empty() { "Changes" } else { name }
    }
}

fn strip_side(path: &str) -> String {
    let path = path.split('\t').next().unwrap_or(path).trim();
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_string()
}

fn git_header_path(rest: &str) -> String {
    // "a/x b/x": the new side is the part after the last " b/".
    rest.rsplit_once(" b/")
        .map(|(_, new)| new.to_string())
        .unwrap_or_else(|| strip_side(rest))
}

/// Split a unified diff into files. `locations` names the file of a patch that
/// carries no header of its own (only when it changes exactly one file).
/// Returns `None` when the text has no added or removed line.
pub(crate) fn parse(patch: &str, locations: &[PathBuf]) -> Option<Vec<PatchFile>> {
    let mut files: Vec<PatchFile> = Vec::new();
    let mut in_hunk = false;
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            files.push(PatchFile::new(git_header_path(rest)));
            in_hunk = false;
        } else if !in_hunk && line.starts_with("--- ") {
            // A header pair without "diff --git" starts a file of its own.
            if files.last().is_none_or(|f| !f.lines.is_empty()) {
                files.push(PatchFile::new(String::new()));
            }
            let path = strip_side(&line[4..]);
            if let Some(file) = files.last_mut().filter(|f| f.path.is_empty())
                && path != "/dev/null"
            {
                file.path = path;
            }
        } else if !in_hunk && line.starts_with("+++ ") {
            let path = strip_side(&line[4..]);
            if let Some(file) = files.last_mut()
                && path != "/dev/null"
            {
                file.path = path;
            }
        } else if line.starts_with("@@") {
            in_hunk = true;
            file_mut(&mut files).lines.push(PatchLine {
                kind: PatchLineKind::Hunk,
                text: line.to_string(),
            });
        } else if in_hunk || line.starts_with(['+', '-']) {
            let (kind, text) = match line.as_bytes().first() {
                Some(b'+') => (PatchLineKind::Added, &line[1..]),
                Some(b'-') => (PatchLineKind::Removed, &line[1..]),
                Some(b' ') => (PatchLineKind::Context, &line[1..]),
                _ => (PatchLineKind::Context, line),
            };
            let file = file_mut(&mut files);
            match kind {
                PatchLineKind::Added => file.added += 1,
                PatchLineKind::Removed => file.removed += 1,
                _ => {}
            }
            file.lines.push(PatchLine {
                kind,
                text: text.to_string(),
            });
        }
        // Other lines ("index ...", "new file mode ...") are header noise.
    }
    files.retain(|f| f.added + f.removed > 0);
    if files.is_empty() {
        return None;
    }
    if let ([file], [location]) = (files.as_mut_slice(), locations)
        && file.path.is_empty()
    {
        file.path = location.to_string_lossy().into_owned();
    }
    Some(files)
}

fn file_mut(files: &mut Vec<PatchFile>) -> &mut PatchFile {
    if files.is_empty() {
        files.push(PatchFile::new(String::new()));
    }
    files.last_mut().expect("just ensured")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_diffs_split_into_files_with_counts() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@\n keep\n-old\n+new\ndiff --git a/b.txt b/b.txt\n--- a/b.txt\n+++ b/b.txt\n@@ -0,0 +1 @@\n+hello\n";
        let files = parse(patch, &[]).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!((files[0].added, files[0].removed), (1, 1));
        assert_eq!(files[0].display_name(), "a.rs");
        assert_eq!((files[1].path.as_str(), files[1].added), ("b.txt", 1));
    }

    #[test]
    fn dashes_inside_a_hunk_are_deletions_not_headers() {
        let patch = "--- a/x\n+++ b/x\n@@ -1,2 +1 @@\n--- removed text\n keep\n";
        let files = parse(patch, &[]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!((files[0].added, files[0].removed), (0, 1));
    }

    #[test]
    fn headerless_patch_takes_the_single_location() {
        let files = parse("@@ -1 +1 @@\n-a\n+b", &[PathBuf::from("/w/f.rs")]).unwrap();
        assert_eq!(files[0].path, "/w/f.rs");
        assert_eq!(files[0].display_name(), "f.rs");
        let two = parse(
            "@@ -1 +1 @@\n-a\n+b",
            &[PathBuf::from("/w/f.rs"), PathBuf::from("/w/g.rs")],
        )
        .unwrap();
        assert_eq!(two[0].display_name(), "Changes");
    }

    #[test]
    fn deleted_files_keep_their_old_name() {
        let patch = "diff --git a/gone.txt b/gone.txt\n--- a/gone.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";
        assert_eq!(parse(patch, &[]).unwrap()[0].path, "gone.txt");
    }

    #[test]
    fn text_without_changes_is_not_a_patch() {
        assert_eq!(parse("", &[]), None);
        assert_eq!(parse("just some tool output\nmore", &[]), None);
        assert_eq!(parse("diff --git a/x b/x\nindex 1..2\n", &[]), None);
    }
}
