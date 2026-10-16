//! Opening what was named on the command line.
use super::LumenApp;
use crate::{cli::Target, services::logger};

impl LumenApp {
    /// Open command-line targets: the first folder becomes the workspace (a
    /// workspace holds one folder, so further ones are reported and skipped), then
    /// every file opens as a tab in order, so the last one named is the one shown.
    /// Problems never stop the rest from opening; they are logged and summarised
    /// in the status bar.
    pub(crate) fn open_command_line_targets(&mut self, targets: Vec<Target>) {
        let (folders, files): (Vec<Target>, Vec<Target>) = targets
            .into_iter()
            .partition(|target| self.file_system.is_directory(&target.path));
        let mut problems: Vec<String> = Vec::new();
        let mut opened = 0;
        if let Some(first) = folders.first() {
            self.open_folder_path(first.path.clone());
            opened += 1;
        }
        for extra in folders.iter().skip(1) {
            problems.push(format!(
                "only one folder can be open, skipped {}",
                extra.path.display()
            ));
        }
        for target in files {
            self.open_file(target.path.clone());
            let open = self
                .editor
                .files
                .iter()
                .find(|file| file.path.as_deref() == Some(target.path.as_path()));
            let Some(document) = open else {
                // `open_file` has already put the reason in the status bar.
                problems.push(format!(
                    "{}: {}",
                    target.path.display(),
                    self.view.status.trim_start_matches("Could not open file: ")
                ));
                continue;
            };
            opened += 1;
            if let Some(line) = target.line {
                let byte = byte_at(&document.text, line, target.column);
                self.view.pending_goto = Some((document.id, byte));
            }
        }
        for problem in &problems {
            logger::warn("cli", problem);
        }
        if !problems.is_empty() {
            self.view.status = format!("Could not open {}", problems.join("; "));
        } else if opened > 1 {
            self.view.status = format!("Opened {opened} items from the command line");
        }
    }
}

/// Byte offset of a 1-based `line` and optional 1-based `column` (in characters).
/// A line past the end lands at the end of the text; a column past the end of its
/// line lands at the end of that line (before its line break).
pub(crate) fn byte_at(text: &str, line: usize, column: Option<usize>) -> usize {
    let mut start = 0;
    for _ in 1..line.max(1) {
        match text[start..].find('\n') {
            Some(newline) => start += newline + 1,
            None => return text.len(),
        }
    }
    let rest = &text[start..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let line_text = rest[..line_end]
        .strip_suffix('\r')
        .unwrap_or(&rest[..line_end]);
    let skip = column.unwrap_or(1).saturating_sub(1);
    let within: usize = line_text.chars().take(skip).map(char::len_utf8).sum();
    start + within
}

#[cfg(test)]
mod tests {
    use super::byte_at;

    #[test]
    fn positions_map_to_byte_offsets_on_char_boundaries() {
        let text = "ab\nçd\r\nlast";
        assert_eq!(byte_at(text, 1, None), 0);
        assert_eq!(byte_at(text, 1, Some(2)), 1);
        assert_eq!(byte_at(text, 2, None), 3);
        assert_eq!(byte_at(text, 2, Some(2)), 5, "after the two-byte ç");
        assert_eq!(byte_at(text, 3, Some(3)), 10);
        assert_eq!(
            byte_at(text, 2, Some(99)),
            6,
            "clamped before the line break"
        );
        assert_eq!(byte_at(text, 99, None), text.len(), "past the end");
        assert_eq!(byte_at("", 4, Some(4)), 0);
        assert_eq!(byte_at(text, 0, None), 0, "line 0 behaves like line 1");
    }
}
