//! Bounded splitting of a byte stream into text lines, for the newline-delimited
//! JSON both providers speak. Oversized lines are discarded (and reported), not
//! buffered without limit; a bad line never loses the lines around it.

/// Generous but bounded: agents send whole files in single messages.
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    Text(String),
    /// A line over the limit; its bytes were dropped up to the next newline.
    TooLong,
    InvalidUtf8,
}

#[derive(Debug)]
pub struct LineBuffer {
    buffer: Vec<u8>,
    discarding: bool,
    limit: usize,
}

impl Default for LineBuffer {
    fn default() -> Self {
        Self::with_limit(MAX_LINE_BYTES)
    }
}

impl LineBuffer {
    pub fn with_limit(limit: usize) -> Self {
        Self {
            buffer: Vec::new(),
            discarding: false,
            limit,
        }
    }

    /// Feed bytes; returns the lines they completed (empty lines are skipped).
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Line> {
        let mut lines = Vec::new();
        let mut rest = bytes;
        while let Some(newline) = rest.iter().position(|byte| *byte == b'\n') {
            let (chunk, tail) = rest.split_at(newline);
            rest = &tail[1..];
            if self.discarding {
                self.discarding = false;
                self.buffer.clear();
                continue;
            }
            if self.buffer.len() + chunk.len() > self.limit {
                self.buffer.clear();
                lines.push(Line::TooLong);
                continue;
            }
            self.buffer.extend_from_slice(chunk);
            let mut raw = std::mem::take(&mut self.buffer);
            if raw.last() == Some(&b'\r') {
                raw.pop();
            }
            if raw.is_empty() {
                continue;
            }
            lines.push(match String::from_utf8(raw) {
                Ok(text) => Line::Text(text),
                Err(_) => Line::InvalidUtf8,
            });
        }
        if !self.discarding {
            if self.buffer.len() + rest.len() > self.limit {
                self.buffer.clear();
                self.discarding = true;
                lines.push(Line::TooLong);
            } else {
                self.buffer.extend_from_slice(rest);
            }
        }
        lines
    }

    /// Bytes buffered for an unfinished line.
    pub fn pending(&self) -> usize {
        self.buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: Vec<Line>) -> Vec<String> {
        lines
            .into_iter()
            .map(|line| match line {
                Line::Text(text) => text,
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn lines_split_across_chunks_and_crlf_and_blank_lines_are_handled() {
        let mut buffer = LineBuffer::default();
        assert!(buffer.push(b"{\"a\"").is_empty());
        assert_eq!(
            texts(buffer.push(b":1}\r\n\n{\"b\":2}\n{\"c\"")),
            ["{\"a\":1}", "{\"b\":2}"]
        );
        assert_eq!(texts(buffer.push(b":3}\n")), ["{\"c\":3}"]);
        assert_eq!(buffer.pending(), 0);
    }

    #[test]
    fn multibyte_text_split_across_chunks_survives() {
        let mut buffer = LineBuffer::default();
        let bytes = "héllo ✓\n".as_bytes();
        let mut out = Vec::new();
        for chunk in bytes.chunks(1) {
            out.extend(buffer.push(chunk));
        }
        assert_eq!(texts(out), ["héllo ✓"]);
    }

    #[test]
    fn an_oversized_line_is_reported_once_dropped_and_the_next_line_still_arrives() {
        let mut buffer = LineBuffer::with_limit(8);
        let mut out = buffer.push(b"0123456789");
        assert_eq!(out, [Line::TooLong]);
        out = buffer.push(b"more of the same line\nok\n");
        assert_eq!(texts(out), ["ok"]);
        assert!(buffer.pending() == 0);
        // Oversized in one chunk with its newline.
        let mut buffer = LineBuffer::with_limit(4);
        assert_eq!(texts(buffer.push(b"toolong\nok\n")), ["TooLong", "ok"]);
    }

    #[test]
    fn invalid_utf8_is_reported_without_losing_neighbours() {
        let mut buffer = LineBuffer::default();
        let out = buffer.push(b"a\n\xff\xfe\nb\n");
        assert_eq!(
            out,
            [
                Line::Text("a".into()),
                Line::InvalidUtf8,
                Line::Text("b".into())
            ]
        );
    }
}
