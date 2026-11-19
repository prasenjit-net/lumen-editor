//! Transient notices: what the last action reported, and for how long it stays.
//!
//! Information fades from the status bar after a few seconds. Warnings pop up as
//! a toast for a while, and errors stay in a toast until they are dismissed.
//! No egui and no clock here: callers pass the current time, so the lifetime
//! rules are tested with synthetic time.
//!
//! Most code still reports through `view.status`. [`Notices::observe_status`]
//! turns each change of that text into a notice, so call sites can move to
//! [`Notices::post`] one at a time.
use crate::model::output_log::LogLevel;

/// Seconds an information message stays fully visible in the status bar.
pub(crate) const STATUS_SECS: f64 = 5.0;
/// Seconds the message then takes to fade out.
pub(crate) const FADE_SECS: f64 = 0.4;
/// Seconds a warning toast stays on screen.
pub(crate) const WARNING_TOAST_SECS: f64 = 10.0;
/// Most toasts shown at once; older ones stay dismissible only after these go.
pub(crate) const MAX_TOASTS: usize = 3;
const MAX_HISTORY: usize = 100;

/// Free-text status messages that report a finished action by its past tense.
fn reports_completion(status: &str) -> bool {
    const DONE: [&str; 12] = [
        "Saved",
        "Committed",
        "Pushed",
        "Pulled",
        "Fetched",
        "Published",
        "Staged",
        "Unstaged",
        "Created",
        "Stashed",
        "Dropped",
        "Resolved",
    ];
    DONE.iter().any(|word| status.starts_with(word))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Severity {
    Info,
    Success,
    Warning,
    Error,
}

impl From<LogLevel> for Severity {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Info => Self::Info,
            LogLevel::Warn => Self::Warning,
            LogLevel::Error => Self::Error,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Notice {
    pub(crate) id: u64,
    pub(crate) severity: Severity,
    pub(crate) text: String,
    pub(crate) posted_at: f64,
    /// How many times the same message arrived in a row.
    pub(crate) count: u32,
    pub(crate) dismissed: bool,
}

#[derive(Default)]
pub(crate) struct Notices {
    history: Vec<Notice>,
    next_id: u64,
    /// The last `view.status` text seen; `None` until the first observation, so
    /// the text the app starts with is not announced.
    seen_status: Option<String>,
}

impl Notices {
    pub(crate) fn post(&mut self, severity: Severity, text: impl Into<String>, now: f64) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.history.last_mut()
            && !last.dismissed
            && last.severity == severity
            && last.text == text
        {
            last.count += 1;
            last.posted_at = now;
            return;
        }
        self.next_id += 1;
        self.history.push(Notice {
            id: self.next_id,
            severity,
            text,
            posted_at: now,
            count: 1,
            dismissed: false,
        });
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
    }

    /// Posts `status` when it differs from the last text seen.
    pub(crate) fn observe_status(&mut self, status: &str, now: f64) {
        let Some(seen) = &self.seen_status else {
            self.seen_status = Some(status.to_owned());
            return;
        };
        if seen == status {
            return;
        }
        self.seen_status = Some(status.to_owned());
        let mut severity: Severity = crate::model::output_log::classify_status(status).into();
        if severity == Severity::Info && reports_completion(status) {
            severity = Severity::Success;
        }
        self.post(severity, status, now);
    }

    /// The newest notice still worth showing in the status bar, with its opacity.
    pub(crate) fn status_line(&self, now: f64) -> Option<(&Notice, f32)> {
        let notice = self.history.last()?;
        let age = now - notice.posted_at;
        if age >= STATUS_SECS + FADE_SECS {
            return None;
        }
        let opacity = if age <= STATUS_SECS {
            1.0
        } else {
            (1.0 - (age - STATUS_SECS) / FADE_SECS) as f32
        };
        Some((notice, opacity.clamp(0.0, 1.0)))
    }

    /// Toasts to draw, oldest first: errors until dismissed, warnings for a while.
    pub(crate) fn toasts(&self, now: f64) -> Vec<&Notice> {
        let mut shown: Vec<&Notice> = self
            .history
            .iter()
            .filter(|notice| !notice.dismissed)
            .filter(|notice| match notice.severity {
                Severity::Error => true,
                Severity::Warning => now - notice.posted_at < WARNING_TOAST_SECS,
                Severity::Info | Severity::Success => false,
            })
            .collect();
        let extra = shown.len().saturating_sub(MAX_TOASTS);
        shown.drain(..extra);
        shown
    }

    pub(crate) fn dismiss(&mut self, id: u64) {
        if let Some(notice) = self.history.iter_mut().find(|notice| notice.id == id) {
            notice.dismissed = true;
        }
    }

    /// Seconds until something visible changes, so idle frames stay idle.
    pub(crate) fn next_change(&self, now: f64) -> Option<f64> {
        let mut soonest: Option<f64> = None;
        let mut consider = |at: f64| {
            let wait = (at - now).max(0.0);
            soonest = Some(soonest.map_or(wait, |current| current.min(wait)));
        };
        if let Some(last) = self.history.last() {
            let end = last.posted_at + STATUS_SECS + FADE_SECS;
            if now < end {
                // Fading needs frames; before that one wake-up at the fade start.
                consider(if now < last.posted_at + STATUS_SECS {
                    last.posted_at + STATUS_SECS
                } else {
                    now + 0.03
                });
            }
        }
        for notice in self.history.iter().filter(|n| !n.dismissed) {
            if notice.severity == Severity::Warning {
                let end = notice.posted_at + WARNING_TOAST_SECS;
                if now < end {
                    consider(end);
                }
            }
        }
        soonest
    }

    #[cfg(test)]
    pub(crate) fn history(&self) -> &[Notice] {
        &self.history
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_text_the_app_starts_with_is_not_announced() {
        let mut notices = Notices::default();
        notices.observe_status("Ready", 0.0);
        assert!(notices.history().is_empty());
        notices.observe_status("Ready", 1.0);
        assert!(notices.history().is_empty());
        notices.observe_status("Opened folder", 2.0);
        assert_eq!(notices.history().len(), 1);
    }

    #[test]
    fn information_fades_after_five_seconds_and_is_gone_after_the_fade() {
        let mut notices = Notices::default();
        notices.post(Severity::Info, "Explorer refreshed", 10.0);
        assert_eq!(notices.status_line(14.9).map(|(_, o)| o), Some(1.0));
        let fading = notices.status_line(15.2).map(|(_, o)| o).unwrap();
        assert!(fading > 0.0 && fading < 1.0, "{fading}");
        assert!(notices.status_line(15.5).is_none());
        assert!(notices.toasts(10.1).is_empty(), "information never pops up");
    }

    #[test]
    fn an_error_toast_survives_until_it_is_dismissed() {
        let mut notices = Notices::default();
        notices.observe_status("Ready", 0.0);
        notices.observe_status("Could not pull: network unreachable", 1.0);
        assert_eq!(notices.toasts(1.0).len(), 1);
        assert_eq!(notices.toasts(61.0).len(), 1, "still there after a minute");
        let id = notices.toasts(61.0)[0].id;
        notices.dismiss(id);
        assert!(notices.toasts(61.0).is_empty());
    }

    #[test]
    fn completed_actions_are_successes_and_other_text_stays_information() {
        let mut notices = Notices::default();
        notices.observe_status("Ready", 0.0);
        notices.observe_status("Pushed local commits", 1.0);
        notices.observe_status("Explorer refreshed", 2.0);
        let severities: Vec<_> = notices.history().iter().map(|n| n.severity).collect();
        assert_eq!(severities, [Severity::Success, Severity::Info]);
    }

    #[test]
    fn warnings_pop_up_for_ten_seconds() {
        let mut notices = Notices::default();
        notices.post(Severity::Warning, "Save cancelled", 0.0);
        assert_eq!(notices.toasts(9.9).len(), 1);
        assert!(notices.toasts(10.1).is_empty());
    }

    #[test]
    fn repeats_coalesce_and_only_three_toasts_show() {
        let mut notices = Notices::default();
        notices.post(Severity::Error, "Failed to save", 0.0);
        notices.post(Severity::Error, "Failed to save", 1.0);
        assert_eq!(notices.history().len(), 1);
        assert_eq!(notices.history()[0].count, 2);
        for n in 0..4 {
            notices.post(Severity::Error, format!("Error {n}"), 2.0);
        }
        let shown = notices.toasts(3.0);
        assert_eq!(shown.len(), MAX_TOASTS);
        assert_eq!(shown.last().unwrap().text, "Error 3");
    }

    #[test]
    fn an_idle_notice_center_asks_for_no_repaint() {
        let mut notices = Notices::default();
        assert_eq!(notices.next_change(0.0), None);
        notices.post(Severity::Error, "Failed", 0.0);
        assert_eq!(notices.next_change(1000.0), None, "errors are static");
        notices.post(Severity::Info, "Saved", 1000.0);
        let wait = notices.next_change(1000.0).unwrap();
        assert!((wait - STATUS_SECS).abs() < 1e-9);
    }
}
