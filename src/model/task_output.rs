//! Bounded, read-only log of one task run. Bytes arrive in arbitrary chunks from
//! one or two streams; this turns them into display lines with defined behavior
//! for split UTF-8, invalid bytes, `\r`, terminal escape sequences and very long
//! lines, and never grows past its byte budget (old lines are dropped and counted).
// Wired to the task runner in the next step.
#![allow(dead_code)]
use std::collections::VecDeque;

/// Longest line kept whole; longer ones are split so one line cannot defeat the budget.
pub(crate) const MAX_LINE_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LogLine {
    pub(crate) stream: Stream,
    pub(crate) text: String,
}

/// Per-stream decoder state, so a sequence split across chunks is not corrupted
/// and stdout/stderr do not interleave inside one line.
#[derive(Default)]
struct StreamState {
    /// Bytes of an incomplete UTF-8 sequence from the previous chunk.
    pending: Vec<u8>,
    line: String,
    escape: Escape,
    /// A `\r` was just seen; `\n` completes the line, anything else rewinds it.
    carriage_return: bool,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Escape {
    #[default]
    None,
    /// After ESC, before the introducer.
    Start,
    /// Inside CSI (`ESC [`), until a final byte 0x40..=0x7E.
    Csi,
    /// Inside OSC (`ESC ]`), until BEL or ST.
    Osc,
    /// ESC seen inside OSC (maybe the `ESC \` terminator).
    OscTerminator,
}

pub(crate) struct RunLog {
    lines: VecDeque<LogLine>,
    retained_bytes: usize,
    limit_bytes: usize,
    dropped_lines: u64,
    dropped_bytes: u64,
    stdout: StreamState,
    stderr: StreamState,
    finished: bool,
}

impl RunLog {
    pub(crate) fn new(limit_bytes: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            retained_bytes: 0,
            limit_bytes: limit_bytes.max(MAX_LINE_BYTES),
            dropped_lines: 0,
            dropped_bytes: 0,
            stdout: StreamState::default(),
            stderr: StreamState::default(),
            finished: false,
        }
    }

    /// Complete lines, oldest first.
    pub(crate) fn lines(&self) -> &VecDeque<LogLine> {
        &self.lines
    }

    /// The unfinished line of each stream (a prompt, a progress bar), shown live.
    pub(crate) fn partial(&self) -> Vec<LogLine> {
        [
            (Stream::Stdout, &self.stdout),
            (Stream::Stderr, &self.stderr),
        ]
        .into_iter()
        .filter(|(_, state)| !state.line.is_empty())
        .map(|(stream, state)| LogLine {
            stream,
            text: state.line.clone(),
        })
        .collect()
    }

    /// The most recent text (about `max_bytes`), including unfinished lines, for
    /// literal pattern matching such as readiness. Lines are joined with `\n`.
    pub(crate) fn tail_text(&self, max_bytes: usize) -> String {
        let mut parts: Vec<&str> = Vec::new();
        let mut bytes = 0;
        for line in self.stdout_stderr_partials().into_iter().rev() {
            bytes += line.len() + 1;
            parts.push(line);
        }
        for line in self.lines.iter().rev() {
            if bytes >= max_bytes {
                break;
            }
            bytes += line.text.len() + 1;
            parts.push(&line.text);
        }
        parts.reverse();
        parts.join("\n")
    }

    fn stdout_stderr_partials(&self) -> Vec<&str> {
        [&self.stdout, &self.stderr]
            .into_iter()
            .filter(|state| !state.line.is_empty())
            .map(|state| state.line.as_str())
            .collect()
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub(crate) fn dropped_lines(&self) -> u64 {
        self.dropped_lines
    }

    pub(crate) fn dropped_bytes(&self) -> u64 {
        self.dropped_bytes
    }

    /// Shown above the retained lines whenever output was discarded.
    pub(crate) fn truncation_notice(&self) -> Option<String> {
        (self.dropped_lines > 0).then(|| {
            format!(
                "… {} earlier line(s), {} KiB, discarded to stay within the output limit",
                self.dropped_lines,
                self.dropped_bytes.div_ceil(1024)
            )
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.stdout.line.is_empty() && self.stderr.line.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.lines.clear();
        self.retained_bytes = 0;
        self.dropped_lines = 0;
        self.dropped_bytes = 0;
        self.stdout.line.clear();
        self.stderr.line.clear();
    }

    /// Feed bytes from one stream.
    pub(crate) fn push(&mut self, stream: Stream, bytes: &[u8]) {
        self.push_lines(stream, bytes);
    }

    /// Feed bytes from one stream and return the lines this completed, so a caller
    /// can scan each exactly once (problem matching).
    pub(crate) fn push_lines(&mut self, stream: Stream, bytes: &[u8]) -> Vec<String> {
        let mut state = std::mem::take(match stream {
            Stream::Stdout => &mut self.stdout,
            Stream::Stderr => &mut self.stderr,
        });
        let mut completed = Vec::new();
        decode(&mut state, bytes, &mut completed);
        *match stream {
            Stream::Stdout => &mut self.stdout,
            Stream::Stderr => &mut self.stderr,
        } = state;
        let mut lines = Vec::with_capacity(completed.len());
        for text in completed {
            lines.push(text.clone());
            self.add_line(stream, text);
        }
        lines
    }

    /// Flush unfinished lines and incomplete UTF-8 once both streams have closed.
    pub(crate) fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        for stream in [Stream::Stdout, Stream::Stderr] {
            let state = match stream {
                Stream::Stdout => &mut self.stdout,
                Stream::Stderr => &mut self.stderr,
            };
            if !state.pending.is_empty() {
                state.line.push('\u{FFFD}');
                state.pending.clear();
            }
            let line = std::mem::take(&mut state.line);
            if !line.is_empty() {
                self.add_line(stream, line);
            }
        }
    }

    fn add_line(&mut self, stream: Stream, text: String) {
        self.retained_bytes += text.len();
        self.lines.push_back(LogLine { stream, text });
        while self.retained_bytes > self.limit_bytes && self.lines.len() > 1 {
            if let Some(old) = self.lines.pop_front() {
                self.retained_bytes -= old.text.len();
                self.dropped_lines += 1;
                self.dropped_bytes += old.text.len() as u64;
            }
        }
    }

    /// Retained text as shown, one line each, for Copy and Save Output. It carries
    /// the truncation notice so an export never implies a full log.
    pub(crate) fn text(&self) -> String {
        let mut out = String::new();
        if let Some(notice) = self.truncation_notice() {
            out.push_str(&notice);
            out.push('\n');
        }
        for line in &self.lines {
            out.push_str(&line.text);
            out.push('\n');
        }
        for line in self.partial() {
            out.push_str(&line.text);
            out.push('\n');
        }
        out
    }
}

fn decode(state: &mut StreamState, bytes: &[u8], completed: &mut Vec<String>) {
    let mut input = std::mem::take(&mut state.pending);
    input.extend_from_slice(bytes);
    let mut rest = input.as_slice();
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                feed(state, text, completed);
                rest = &[];
            }
            Err(error) => {
                let (valid, after) = rest.split_at(error.valid_up_to());
                feed(
                    state,
                    std::str::from_utf8(valid).unwrap_or_default(),
                    completed,
                );
                match error.error_len() {
                    // An invalid sequence: replace it and continue after it.
                    Some(len) => {
                        feed(state, "\u{FFFD}", completed);
                        rest = &after[len..];
                    }
                    // An incomplete sequence at the end: wait for the next chunk.
                    None => {
                        state.pending = after.to_vec();
                        rest = &[];
                    }
                }
            }
        }
    }
}

fn feed(state: &mut StreamState, text: &str, completed: &mut Vec<String>) {
    for c in text.chars() {
        match state.escape {
            Escape::Start => {
                state.escape = match c {
                    '[' => Escape::Csi,
                    ']' => Escape::Osc,
                    // A two-character escape such as ESC c or ESC M.
                    _ => Escape::None,
                };
                continue;
            }
            Escape::Csi => {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    state.escape = Escape::None;
                }
                continue;
            }
            Escape::Osc => {
                state.escape = match c {
                    '\u{07}' => Escape::None,
                    '\u{1b}' => Escape::OscTerminator,
                    _ => Escape::Osc,
                };
                continue;
            }
            Escape::OscTerminator => {
                state.escape = if c == '\\' { Escape::None } else { Escape::Osc };
                continue;
            }
            Escape::None => {}
        }
        if state.carriage_return {
            state.carriage_return = false;
            if c == '\n' {
                completed.push(std::mem::take(&mut state.line));
                continue;
            }
            // A lone carriage return: the cursor went back to column 0, so what
            // follows replaces the line (progress bars).
            state.line.clear();
        }
        match c {
            '\u{1b}' => state.escape = Escape::Start,
            '\r' => state.carriage_return = true,
            '\n' => completed.push(std::mem::take(&mut state.line)),
            '\t' => state.line.push('\t'),
            c if c.is_control() => {}
            c => {
                state.line.push(c);
                if state.line.len() >= MAX_LINE_BYTES {
                    completed.push(std::mem::take(&mut state.line));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log() -> RunLog {
        RunLog::new(MAX_LINE_BYTES * 4)
    }

    fn texts(log: &RunLog) -> Vec<&str> {
        log.lines().iter().map(|line| line.text.as_str()).collect()
    }

    #[test]
    fn lines_split_across_chunks_and_partial_lines_show_live() {
        let mut log = log();
        log.push(Stream::Stdout, b"hel");
        assert!(log.lines().is_empty());
        assert_eq!(log.partial()[0].text, "hel");
        log.push(Stream::Stdout, b"lo\nwor");
        log.push(Stream::Stdout, b"ld\r\nlast");
        assert_eq!(texts(&log), ["hello", "world"]);
        assert_eq!(log.partial()[0].text, "last");
        log.finish();
        assert_eq!(texts(&log), ["hello", "world", "last"]);
        assert!(log.partial().is_empty());
    }

    #[test]
    fn multibyte_characters_split_across_chunks_survive() {
        let mut log = log();
        let bytes = "héllo ✓ 日本\n".as_bytes();
        for chunk in bytes.chunks(1) {
            log.push(Stream::Stdout, chunk);
        }
        assert_eq!(texts(&log), ["héllo ✓ 日本"]);
    }

    #[test]
    fn invalid_bytes_become_replacement_characters_and_truncated_tail_is_flushed() {
        let mut log = log();
        log.push(Stream::Stdout, b"a\xffb\xc3\x28c\n");
        assert_eq!(texts(&log), ["a\u{FFFD}b\u{FFFD}(c"]);
        log.push(Stream::Stderr, &"é".as_bytes()[..1]);
        log.finish();
        assert_eq!(log.lines().back().unwrap().text, "\u{FFFD}");
        assert_eq!(log.lines().back().unwrap().stream, Stream::Stderr);
    }

    #[test]
    fn carriage_return_rewinds_progress_lines_but_crlf_is_a_newline() {
        let mut log = log();
        log.push(Stream::Stdout, b"10%\r50%\r100%\ndone\r\n");
        assert_eq!(texts(&log), ["100%", "done"]);
        // A split CRLF is still one newline.
        log.push(Stream::Stdout, b"x\r");
        log.push(Stream::Stdout, b"\ny\n");
        assert_eq!(texts(&log)[2..], ["x", "y"]);
    }

    #[test]
    fn escape_sequences_are_stripped_even_when_split_across_chunks() {
        let mut log = log();
        log.push(Stream::Stdout, b"\x1b[31mred\x1b[0m plain \x1b[1");
        log.push(Stream::Stdout, b";32mgreen\x1b[0m\n");
        log.push(
            Stream::Stdout,
            b"\x1b]0;window title\x07title-set\n\x1b]8;;http://x\x1b\\link\n",
        );
        log.push(Stream::Stdout, b"\x1bcreset\n");
        assert_eq!(
            texts(&log),
            ["red plain green", "title-set", "link", "reset"]
        );
    }

    #[test]
    fn control_characters_are_dropped_but_tabs_stay() {
        let mut log = log();
        log.push(Stream::Stdout, b"a\x00b\x07c\td\n");
        assert_eq!(texts(&log), ["abc\td"]);
    }

    #[test]
    fn streams_do_not_interleave_inside_a_line() {
        let mut log = log();
        log.push(Stream::Stdout, b"out-1 ");
        log.push(Stream::Stderr, b"err-1\n");
        log.push(Stream::Stdout, b"out-2\n");
        let lines: Vec<_> = log
            .lines()
            .iter()
            .map(|l| (l.stream, l.text.as_str()))
            .collect();
        assert_eq!(
            lines,
            [(Stream::Stderr, "err-1"), (Stream::Stdout, "out-1 out-2")]
        );
    }

    #[test]
    fn very_long_lines_are_split_and_the_budget_drops_oldest_with_a_notice() {
        let mut log = RunLog::new(MAX_LINE_BYTES * 2);
        log.push(Stream::Stdout, &vec![b'x'; MAX_LINE_BYTES * 3 + 10]);
        assert!(
            log.lines()
                .iter()
                .all(|line| line.text.len() <= MAX_LINE_BYTES)
        );
        assert_eq!(
            log.lines().len(),
            2,
            "two of three full chunks fit; the oldest was dropped"
        );
        assert!(log.retained_bytes() <= MAX_LINE_BYTES * 2);
        assert_eq!(log.dropped_lines(), 1);
        let notice = log.truncation_notice().unwrap();
        assert!(notice.contains("1 earlier line"), "{notice}");
        assert!(log.text().starts_with("… 1 earlier line"));
        for n in 0..10_000 {
            log.push(Stream::Stdout, format!("line {n}\n").as_bytes());
        }
        assert!(log.retained_bytes() <= MAX_LINE_BYTES * 2);
        assert!(log.text().contains("line 9999"));
        assert!(!log.text().contains("line 0\n"));
        assert!(log.dropped_bytes() > 0);
    }

    #[test]
    fn clear_resets_everything_and_the_newest_line_is_never_dropped() {
        let mut log = RunLog::new(MAX_LINE_BYTES);
        log.push(Stream::Stdout, b"keep\n");
        log.clear();
        assert!(log.is_empty() && log.truncation_notice().is_none());
        log.push(Stream::Stdout, &vec![b'y'; MAX_LINE_BYTES]);
        assert_eq!(log.lines().len(), 1);
    }

    #[test]
    fn the_tail_includes_unfinished_lines_so_a_pattern_split_across_chunks_matches() {
        let mut log = log();
        log.push(Stream::Stdout, b"starting\nready on http://loc");
        assert!(!log.tail_text(1024).contains("Local:"));
        log.push(Stream::Stdout, b"alhost\n  Local: http://x");
        let tail = log.tail_text(1024);
        assert!(tail.contains("ready on http://localhost"), "{tail}");
        assert!(
            tail.contains("Local: http://x"),
            "unfinished lines count: {tail}"
        );
        // Only recent text is considered.
        let mut log = RunLog::new(MAX_LINE_BYTES * 4);
        for n in 0..2000 {
            log.push(Stream::Stdout, format!("noise {n}\n").as_bytes());
        }
        let tail = log.tail_text(256);
        assert!(tail.contains("noise 1999") && !tail.contains("noise 0\n") && tail.len() < 600);
    }

    #[test]
    fn push_lines_returns_each_completed_line_once() {
        let mut log = log();
        assert!(log.push_lines(Stream::Stdout, b"par").is_empty());
        assert_eq!(
            log.push_lines(Stream::Stdout, b"tial\nnext\r\nrest"),
            ["partial", "next"]
        );
        assert_eq!(log.push_lines(Stream::Stdout, b"\n"), ["rest"]);
        assert_eq!(log.lines().len(), 3, "the lines are retained as before");
    }
}
