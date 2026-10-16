//! Bounded in-memory log of editor activity, shown in the Output panel.
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LogEntry {
    /// Monotonic across the process, so viewers can tell what they have seen.
    pub(crate) sequence: u64,
    /// Local time, `YYYY-MM-DD HH:MM:SS.mmm`.
    pub(crate) timestamp: String,
    pub(crate) level: LogLevel,
    pub(crate) source: String,
    pub(crate) message: String,
}

impl LogEntry {
    /// The line users copy: stable, greppable and identical to what is displayed.
    pub(crate) fn line(&self) -> String {
        format!(
            "{} [{}] {}: {}",
            self.timestamp,
            self.level.label(),
            self.source,
            self.message
        )
    }
}

/// Oldest entries are dropped first once `capacity` is reached.
pub(crate) struct LogBuffer {
    capacity: usize,
    next_sequence: u64,
    entries: VecDeque<LogEntry>,
}

impl LogBuffer {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            next_sequence: 1,
            entries: VecDeque::new(),
        }
    }

    pub(crate) fn push(&mut self, timestamp: String, level: LogLevel, source: &str, message: &str) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(LogEntry {
            sequence: self.next_sequence,
            timestamp,
            level,
            source: source.to_owned(),
            message: message.to_owned(),
        });
        self.next_sequence += 1;
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn snapshot(&self) -> Vec<LogEntry> {
        self.entries.iter().cloned().collect()
    }

    /// Sequence of the newest entry ever pushed (0 before any), surviving `clear`.
    pub(crate) fn last_sequence(&self) -> u64 {
        self.next_sequence - 1
    }
}

/// All entries as copyable text, one per line, oldest first.
pub(crate) fn text_of(entries: &[LogEntry]) -> String {
    entries
        .iter()
        .map(LogEntry::line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Status-bar messages are free text; classify them for the log by wording.
pub(crate) fn classify_status(message: &str) -> LogLevel {
    let lower = message.to_lowercase();
    const ERRORS: [&str; 9] = [
        "failed",
        "error",
        "could not",
        "cannot",
        "unable",
        "invalid",
        "refused",
        "denied",
        "not applied",
    ];
    const WARNINGS: [&str; 6] = [
        "cancelled",
        "canceled",
        "discarded",
        "skipped",
        "changed on disk",
        "conflict",
    ];
    if ERRORS.iter().any(|word| lower.contains(word)) {
        LogLevel::Error
    } else if WARNINGS.iter().any(|word| lower.contains(word)) {
        LogLevel::Warn
    } else {
        LogLevel::Info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_keeps_the_newest_entries_in_order_with_stable_sequences() {
        let mut buffer = LogBuffer::new(3);
        for n in 0..5 {
            buffer.push(format!("t{n}"), LogLevel::Info, "src", &format!("m{n}"));
        }
        let entries = buffer.snapshot();
        assert_eq!(
            entries.iter().map(|e| e.sequence).collect::<Vec<_>>(),
            [3, 4, 5]
        );
        assert_eq!(entries[0].message, "m2");
        buffer.clear();
        assert!(buffer.snapshot().is_empty());
        assert_eq!(buffer.last_sequence(), 5, "clearing never reuses sequences");
    }

    #[test]
    fn copied_text_has_timestamp_level_source_and_message_per_line() {
        let mut buffer = LogBuffer::new(8);
        buffer.push(
            "2026-10-16 10:00:01.250".into(),
            LogLevel::Warn,
            "lsp",
            "slow",
        );
        buffer.push(
            "2026-10-16 10:00:02.000".into(),
            LogLevel::Error,
            "file",
            "boom",
        );
        assert_eq!(
            text_of(&buffer.snapshot()),
            "2026-10-16 10:00:01.250 [WARN] lsp: slow\n2026-10-16 10:00:02.000 [ERROR] file: boom"
        );
    }

    #[test]
    fn status_text_is_classified_by_wording() {
        assert_eq!(
            classify_status("Save failed: permission denied"),
            LogLevel::Error
        );
        assert_eq!(
            classify_status("Paste cancelled: target changed"),
            LogLevel::Warn
        );
        assert_eq!(classify_status("Opened src/main.rs"), LogLevel::Info);
        assert_eq!(
            classify_status("Rename not applied: stale"),
            LogLevel::Error
        );
    }
}
