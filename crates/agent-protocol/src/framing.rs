//! ACP stdio framing: UTF-8 JSON-RPC messages separated by `\n`.
use serde_json::Value;

/// Agents send whole-file contents in single messages; stay generous but bounded.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// A line exceeded the limit; its bytes were discarded up to the next newline.
    MessageTooLarge,
    InvalidUtf8,
    InvalidJson(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MessageTooLarge => formatter.write_str("agent message exceeds the size limit"),
            Self::InvalidUtf8 => formatter.write_str("agent message is not valid UTF-8"),
            Self::InvalidJson(error) => write!(formatter, "agent message is not JSON: {error}"),
        }
    }
}

/// Incremental decoder. One `push` may yield zero, one or many messages, and a
/// bad line is reported without losing the lines around it.
#[derive(Debug)]
pub struct LineDecoder {
    buffer: Vec<u8>,
    discarding: bool,
    limit: usize,
}

impl Default for LineDecoder {
    fn default() -> Self {
        Self::with_limit(MAX_MESSAGE_BYTES)
    }
}

impl LineDecoder {
    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            buffer: Vec::new(),
            discarding: false,
            limit,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<Value, FrameError>> {
        let mut results = Vec::new();
        let mut rest = bytes;
        while let Some(newline) = rest.iter().position(|byte| *byte == b'\n') {
            let (line, tail) = rest.split_at(newline);
            rest = &tail[1..];
            if self.discarding {
                self.discarding = false;
                self.buffer.clear();
                continue;
            }
            if self.buffer.len() + line.len() > self.limit {
                self.buffer.clear();
                results.push(Err(FrameError::MessageTooLarge));
                continue;
            }
            self.buffer.extend_from_slice(line);
            let complete = std::mem::take(&mut self.buffer);
            if let Some(result) = decode_line(&complete) {
                results.push(result);
            }
        }
        if !self.discarding {
            if self.buffer.len() + rest.len() > self.limit {
                self.buffer.clear();
                self.discarding = true;
                results.push(Err(FrameError::MessageTooLarge));
            } else {
                self.buffer.extend_from_slice(rest);
            }
        }
        results
    }

    /// Bytes buffered for an unfinished line.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.buffer.len()
    }
}

fn decode_line(line: &[u8]) -> Option<Result<Value, FrameError>> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let Ok(text) = std::str::from_utf8(line) else {
        return Some(Err(FrameError::InvalidUtf8));
    };
    Some(serde_json::from_str(text).map_err(|error| FrameError::InvalidJson(error.to_string())))
}

/// One message as a compact line. `serde_json` never emits raw newlines.
#[must_use]
pub fn encode(message: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(message).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn values(results: Vec<Result<Value, FrameError>>) -> Vec<Value> {
        results.into_iter().map(Result::unwrap).collect()
    }

    #[test]
    fn fragments_and_batches_decode_to_the_same_messages() {
        let first = encode(&json!({"jsonrpc":"2.0","id":1,"result":{"text":"a\nb"}}));
        let second = encode(&json!({"jsonrpc":"2.0","method":"x"}));
        let all = [first.clone(), second.clone()].concat();
        let mut whole = LineDecoder::default();
        let expected = values(whole.push(&all));
        assert_eq!(expected.len(), 2);
        // Newlines inside strings are escaped, so every byte split is safe.
        for size in 1..all.len() {
            let mut decoder = LineDecoder::default();
            let mut got = Vec::new();
            for chunk in all.chunks(size) {
                got.extend(values(decoder.push(chunk)));
            }
            assert_eq!(got, expected, "chunk size {size}");
            assert_eq!(decoder.pending_len(), 0);
        }
    }

    #[test]
    fn blank_lines_and_crlf_are_tolerated() {
        let mut decoder = LineDecoder::default();
        let got = values(decoder.push(b"\n  \r\n{\"a\":1}\r\n"));
        assert_eq!(got, vec![json!({"a":1})]);
    }

    #[test]
    fn bad_lines_are_reported_without_losing_neighbours() {
        let mut decoder = LineDecoder::default();
        let results = decoder.push(b"{\"a\":1}\nnot json\n\xff\xfe\n{\"b\":2}\n");
        assert_eq!(results.len(), 4);
        assert_eq!(results[0], Ok(json!({"a":1})));
        assert!(matches!(results[1], Err(FrameError::InvalidJson(_))));
        assert_eq!(results[2], Err(FrameError::InvalidUtf8));
        assert_eq!(results[3], Ok(json!({"b":2})));
    }

    #[test]
    fn oversized_lines_are_discarded_across_chunks_then_decoding_resumes() {
        let mut decoder = LineDecoder::with_limit(16);
        let mut results = decoder.push(b"{\"padding\":\"0123456789");
        assert_eq!(results, vec![Err(FrameError::MessageTooLarge)]);
        results = decoder.push(b"more bytes of the same line");
        assert!(results.is_empty());
        assert_eq!(decoder.pending_len(), 0, "memory stays bounded");
        results = decoder.push(b"\n{\"ok\":1}\n");
        assert_eq!(results, vec![Ok(json!({"ok":1}))]);
        // A complete oversized line in one push is also rejected.
        let results = decoder.push(b"{\"padding\":\"0123456789012345\"}\n");
        assert_eq!(results, vec![Err(FrameError::MessageTooLarge)]);
    }
}
