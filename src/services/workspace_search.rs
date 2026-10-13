//! Bounded, cancellable filesystem search. No editor or egui state lives here.
use crate::model::search::{SearchOptions, find_matches};
use std::{
    fs, io,
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MATCHES: usize = 10_000;
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SearchRequest {
    pub(crate) root: PathBuf,
    pub(crate) query: String,
    pub(crate) include: String,
    pub(crate) exclude: String,
    pub(crate) options: SearchOptions,
}

#[derive(Clone)]
pub(crate) struct SearchHit {
    pub(crate) range: Range<usize>,
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) snippet: String,
}

#[derive(Clone)]
pub(crate) struct SearchFile {
    pub(crate) path: PathBuf,
    pub(crate) original: String,
    pub(crate) hits: Vec<SearchHit>,
}

#[derive(Default)]
pub(crate) struct SearchResult {
    pub(crate) files: Vec<SearchFile>,
    pub(crate) scanned: usize,
    pub(crate) skipped: usize,
    pub(crate) errors: Vec<String>,
    pub(crate) limited: bool,
    pub(crate) cancelled: bool,
}

#[derive(Clone)]
struct IgnoreRule {
    directory: PathBuf,
    pattern: String,
    negated: bool,
    directory_only: bool,
}

pub(crate) fn search(request: &SearchRequest, cancel: &AtomicBool) -> SearchResult {
    let mut result = SearchResult::default();
    if request.query.is_empty() {
        return result;
    }
    let mut stack = vec![(request.root.clone(), Vec::<IgnoreRule>::new())];
    let mut retained_bytes = 0;
    let mut retained_matches = 0;
    while let Some((directory, mut rules)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            result.cancelled = true;
            break;
        }
        read_ignore_rules(&directory, &mut rules, &mut result.errors);
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                result
                    .errors
                    .push(format!("{}: {error}", directory.display()));
                continue;
            }
        };
        for entry in entries {
            if cancel.load(Ordering::Relaxed) {
                result.cancelled = true;
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    result.errors.push(error.to_string());
                    continue;
                }
            };
            let path = entry.path();
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) => {
                    result.errors.push(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            // Do not recurse through links, or inspect generated/VCS internals.
            if kind.is_symlink() || matches!(entry.file_name().to_str(), Some(".git" | "target")) {
                result.skipped += 1;
                continue;
            }
            let relative = path.strip_prefix(&request.root).unwrap_or(&path);
            if ignored(&path, kind.is_dir(), &rules) || matches_patterns(relative, &request.exclude)
            {
                result.skipped += 1;
                continue;
            }
            if kind.is_dir() {
                stack.push((path, rules.clone()));
                continue;
            }
            if !kind.is_file()
                || (!request.include.trim().is_empty()
                    && !matches_patterns(relative, &request.include))
            {
                continue;
            }
            result.scanned += 1;
            match read_candidate(&path) {
                Ok(Some(text)) => {
                    let ranges = find_matches(&text, &request.query, request.options);
                    if ranges.is_empty() {
                        continue;
                    }
                    if retained_bytes + text.len() > MAX_SNAPSHOT_BYTES
                        || retained_matches + ranges.len() > MAX_MATCHES
                    {
                        result.limited = true;
                        break;
                    }
                    retained_bytes += text.len();
                    retained_matches += ranges.len();
                    result.files.push(SearchFile {
                        path,
                        hits: hits(&text, ranges),
                        original: text,
                    });
                }
                Ok(None) => result.skipped += 1,
                Err(error) => result.errors.push(format!("{}: {error}", path.display())),
            }
        }
        if result.cancelled || result.limited {
            break;
        }
    }
    result.files.sort_by(|a, b| a.path.cmp(&b.path));
    result
}

fn read_candidate(path: &Path) -> io::Result<Option<String>> {
    if fs::metadata(path)?.len() > MAX_FILE_BYTES {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    if bytes.len() as u64 > MAX_FILE_BYTES || bytes.contains(&0) {
        return Ok(None);
    }
    Ok(String::from_utf8(bytes).ok())
}

fn hits(text: &str, ranges: Vec<Range<usize>>) -> Vec<SearchHit> {
    let mut line_starts = vec![0];
    line_starts.extend(
        text.bytes()
            .enumerate()
            .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
    );
    ranges
        .into_iter()
        .map(|range| {
            let line = line_starts.partition_point(|start| *start <= range.start);
            let start = line_starts[line - 1];
            let end = text[range.end..]
                .find('\n')
                .map_or(text.len(), |offset| range.end + offset);
            let snippet = text[start..end]
                .trim_end_matches('\r')
                .chars()
                .take(180)
                .collect();
            let column = text[start..range.start].chars().count() + 1;
            SearchHit {
                range,
                line,
                column,
                snippet,
            }
        })
        .collect()
}

fn read_ignore_rules(directory: &Path, rules: &mut Vec<IgnoreRule>, errors: &mut Vec<String>) {
    let path = directory.join(".gitignore");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            errors.push(format!("{}: {error}", path.display()));
            return;
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let negated = line.starts_with('!');
        let pattern = line.strip_prefix('!').unwrap_or(line);
        rules.push(IgnoreRule {
            directory: directory.to_path_buf(),
            pattern: pattern.trim_end_matches('/').trim_start_matches('/').into(),
            negated,
            directory_only: pattern.ends_with('/'),
        });
    }
}

fn ignored(path: &Path, is_dir: bool, rules: &[IgnoreRule]) -> bool {
    let mut ignored = false;
    for rule in rules {
        if rule.directory_only && !is_dir {
            continue;
        }
        let Ok(relative) = path.strip_prefix(&rule.directory) else {
            continue;
        };
        if pattern_matches(relative, &rule.pattern) {
            ignored = !rule.negated;
        }
    }
    ignored
}

fn matches_patterns(path: &Path, patterns: &str) -> bool {
    patterns
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .any(|pattern| pattern_matches(path, pattern))
}

fn pattern_matches(path: &Path, pattern: &str) -> bool {
    let path = path.to_string_lossy().replace('\\', "/");
    let pattern = pattern.trim_start_matches('/');
    if !pattern.contains('/') {
        return path.split('/').any(|part| glob_match(pattern, part));
    }
    glob_match(pattern, &path)
}

// Small glob matcher: * stays in one component, ** crosses directories.
fn glob_match(pattern: &str, path: &str) -> bool {
    let p = pattern.as_bytes();
    let s = path.as_bytes();
    let mut memo = vec![vec![None; s.len() + 1]; p.len() + 1];
    fn match_at(p: &[u8], s: &[u8], i: usize, j: usize, memo: &mut [Vec<Option<bool>>]) -> bool {
        if let Some(value) = memo[i][j] {
            return value;
        }
        let value = if i == p.len() {
            j == s.len()
        } else if p[i] == b'*' {
            let double = p.get(i + 1) == Some(&b'*');
            let next = i + if double { 2 } else { 1 };
            match_at(p, s, next, j, memo)
                || (j < s.len() && (double || s[j] != b'/') && match_at(p, s, i, j + 1, memo))
        } else if j < s.len() && (p[i] == s[j] || (p[i] == b'?' && s[j] != b'/')) {
            match_at(p, s, i + 1, j + 1, memo)
        } else {
            false
        };
        memo[i][j] = Some(value);
        value
    }
    match_at(p, s, 0, 0, &mut memo)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_crlf_globs_and_nested_ignores() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        fs::write(root.path().join(".gitignore"), "*.tmp\n").unwrap();
        fs::write(root.path().join("src/.gitignore"), "hidden.rs\n").unwrap();
        fs::write(root.path().join("src/main.rs"), "é\r\né").unwrap();
        fs::write(root.path().join("src/hidden.rs"), "é").unwrap();
        fs::write(root.path().join("src/other.tmp"), "é").unwrap();
        let result = search(
            &SearchRequest {
                root: root.path().into(),
                query: "é".into(),
                include: "*.rs".into(),
                exclude: String::new(),
                options: SearchOptions::default(),
            },
            &AtomicBool::new(false),
        );
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].hits.len(), 2);
        assert_eq!(result.files[0].hits[1].line, 2);
        assert_eq!(result.files[0].hits[1].range, 4..6);
    }
    #[test]
    fn cancellation_binary_size_and_errors_are_reported() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("binary.txt"), b"term\0term").unwrap();
        fs::write(
            root.path().join("large.txt"),
            vec![b'a'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        fs::write(root.path().join("small.txt"), "term").unwrap();
        let request = SearchRequest {
            root: root.path().into(),
            query: "term".into(),
            include: String::new(),
            exclude: String::new(),
            options: SearchOptions::default(),
        };
        let result = search(&request, &AtomicBool::new(false));
        assert_eq!(result.files.len(), 1);
        assert!(result.skipped >= 2);
        assert!(search(&request, &AtomicBool::new(true)).cancelled);
    }
}
