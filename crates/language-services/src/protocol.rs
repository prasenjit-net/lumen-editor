//! Pure, bounded JSON-RPC `Content-Length` framing for LSP stdio transport.
//!
//! This module deliberately does not spawn processes or parse JSON. The service
//! adapter owns I/O and message serialization; it feeds bytes here and receives
//! complete payloads without allowing a server to allocate an unbounded buffer.

const HEADER_LIMIT: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    HeaderTooLarge,
    InvalidHeaderEncoding,
    MissingContentLength,
    DuplicateContentLength,
    InvalidContentLength,
    MessageTooLarge,
}

/// Incremental decoder for LSP's `Content-Length: <bytes>\r\n\r\n<payload>`
/// framing. A single append may yield zero, one or many complete payloads.
#[derive(Debug)]
pub struct ContentLengthDecoder {
    buffer: Vec<u8>,
    expected_payload: Option<usize>,
    max_message_bytes: usize,
}

impl ContentLengthDecoder {
    #[must_use]
    pub fn new(max_message_bytes: usize) -> Self {
        Self {
            buffer: Vec::new(),
            expected_payload: None,
            max_message_bytes,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        self.buffer.extend_from_slice(bytes);
        let mut payloads = Vec::new();
        loop {
            if let Some(expected) = self.expected_payload {
                if self.buffer.len() < expected {
                    break;
                }
                payloads.push(self.buffer.drain(..expected).collect());
                self.expected_payload = None;
                continue;
            }

            let Some(header_end) = self
                .buffer
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
            else {
                if self.buffer.len() > HEADER_LIMIT {
                    return Err(FrameError::HeaderTooLarge);
                }
                break;
            };
            let header = std::str::from_utf8(&self.buffer[..header_end])
                .map_err(|_| FrameError::InvalidHeaderEncoding)?;
            let length = content_length(header)?;
            if length > self.max_message_bytes {
                return Err(FrameError::MessageTooLarge);
            }
            self.buffer.drain(..header_end + 4);
            self.expected_payload = Some(length);
        }
        Ok(payloads)
    }
}

fn content_length(header: &str) -> Result<usize, FrameError> {
    let mut length = None;
    for line in header.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            return Err(FrameError::InvalidHeaderEncoding);
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(FrameError::DuplicateContentLength);
            }
            length = Some(
                value
                    .trim()
                    .parse()
                    .map_err(|_| FrameError::InvalidContentLength)?,
            );
        }
    }
    length.ok_or(FrameError::MissingContentLength)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_and_multiple_frames_preserve_exact_payloads() {
        let mut decoder = ContentLengthDecoder::new(1024);
        assert!(decoder.push(b"Content-Len").unwrap().is_empty());
        assert!(decoder.push(b"gth: 2\r\n\r").unwrap().is_empty());
        assert_eq!(
            decoder.push(b"\n{}Content-Length: 4\r\n\r\nnull").unwrap(),
            vec![b"{}".to_vec(), b"null".to_vec()]
        );
    }

    #[test]
    fn malformed_and_oversized_frames_are_rejected_before_delivery() {
        let mut decoder = ContentLengthDecoder::new(3);
        assert_eq!(
            decoder.push(b"Content-Length: 4\r\n\r\nnull"),
            Err(FrameError::MessageTooLarge)
        );
        let mut decoder = ContentLengthDecoder::new(10);
        assert_eq!(
            decoder.push(b"Content-Length: two\r\n\r\n"),
            Err(FrameError::InvalidContentLength)
        );
        let mut decoder = ContentLengthDecoder::new(10);
        assert_eq!(
            decoder.push(b"Content-Length: 1\r\nContent-Length: 1\r\n\r\na"),
            Err(FrameError::DuplicateContentLength)
        );
    }
}
