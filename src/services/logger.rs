//! Process-wide editor log. Any module, including background threads, can record
//! an entry; the Output panel reads snapshots on the UI thread.
use crate::model::output_log::{LogBuffer, LogEntry, LogLevel};
use std::sync::{Arc, Mutex, OnceLock};

const CAPACITY: usize = 2_000;

struct Logger {
    buffer: Mutex<LogBuffer>,
    waker: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

fn logger() -> &'static Logger {
    static LOGGER: OnceLock<Logger> = OnceLock::new();
    LOGGER.get_or_init(|| Logger {
        buffer: Mutex::new(LogBuffer::new(CAPACITY)),
        waker: Mutex::new(None),
    })
}

pub(crate) fn timestamp_now() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S%.3f")
        .to_string()
}

/// Wake the UI when a background thread logs while the window is idle.
pub(crate) fn set_waker(waker: Arc<dyn Fn() + Send + Sync>) {
    if let Ok(mut slot) = logger().waker.lock() {
        *slot = Some(waker);
    }
}

pub(crate) fn log(level: LogLevel, source: &str, message: &str) {
    let message = message.trim_end();
    if message.is_empty() {
        return;
    }
    if let Ok(mut buffer) = logger().buffer.lock() {
        buffer.push(timestamp_now(), level, source, message);
    }
    let waker = logger().waker.lock().ok().and_then(|slot| slot.clone());
    if let Some(waker) = waker {
        waker();
    }
}

pub(crate) fn info(source: &str, message: &str) {
    log(LogLevel::Info, source, message);
}

pub(crate) fn warn(source: &str, message: &str) {
    log(LogLevel::Warn, source, message);
}

pub(crate) fn error(source: &str, message: &str) {
    log(LogLevel::Error, source, message);
}

pub(crate) fn last_sequence() -> u64 {
    logger()
        .buffer
        .lock()
        .map_or(0, |buffer| buffer.last_sequence())
}

pub(crate) fn snapshot() -> Vec<LogEntry> {
    logger()
        .buffer
        .lock()
        .map(|buffer| buffer.snapshot())
        .unwrap_or_default()
}

pub(crate) fn clear() {
    if let Ok(mut buffer) = logger().buffer.lock() {
        buffer.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_timestamped_trimmed_and_visible_to_snapshots() {
        let marker = "logger-test-unique-marker";
        error("test", &format!("{marker}\n"));
        log(LogLevel::Info, "test", "   ");
        let entry = snapshot()
            .into_iter()
            .rev()
            .find(|entry| entry.message == marker)
            .expect("logged entry");
        assert_eq!(
            (entry.level, entry.source.as_str()),
            (LogLevel::Error, "test")
        );
        // 2026-10-16 10:00:01.250
        assert_eq!(entry.timestamp.len(), 23);
        assert_eq!(&entry.timestamp[4..5], "-");
        assert_eq!(&entry.timestamp[19..20], ".");
        assert!(last_sequence() >= entry.sequence);
    }
}
