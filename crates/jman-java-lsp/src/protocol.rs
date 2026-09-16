use std::fmt;
use std::io::{BufRead, Write};

use serde_json::Value;

const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum ProtocolError {
    Io(std::io::Error),
    MissingContentLength,
    InvalidHeader,
    MessageTooLarge,
    InvalidJson(serde_json::Error),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::MissingContentLength => formatter.write_str("missing Content-Length header"),
            Self::InvalidHeader => formatter.write_str("invalid LSP header"),
            Self::MessageTooLarge => formatter.write_str("LSP message exceeds 16 MiB"),
            Self::InvalidJson(error) => write!(formatter, "invalid JSON: {error}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<std::io::Error> for ProtocolError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn read_message(reader: &mut impl BufRead) -> Result<Option<Value>, ProtocolError> {
    let mut content_length = None;
    let mut saw_header = false;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            return if saw_header {
                Err(ProtocolError::InvalidHeader)
            } else {
                Ok(None)
            };
        }
        saw_header = true;
        if header == "\r\n" || header == "\n" {
            break;
        }
        let Some((name, value)) = header.trim_end().split_once(':') else {
            return Err(ProtocolError::InvalidHeader);
        };
        if name.eq_ignore_ascii_case("content-length") {
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| ProtocolError::InvalidHeader)?;
            if content_length.is_some_and(|previous| previous != parsed) {
                return Err(ProtocolError::InvalidHeader);
            }
            content_length = Some(parsed);
        }
    }
    let length = content_length.ok_or(ProtocolError::MissingContentLength)?;
    if length > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::MessageTooLarge);
    }
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(ProtocolError::InvalidJson)
}

pub fn write_message(writer: &mut impl Write, message: &Value) -> Result<(), ProtocolError> {
    let body = serde_json::to_vec(message).map_err(ProtocolError::InvalidJson)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_case_insensitive_headers_and_exact_utf8_lengths() {
        let body = r#"{"jsonrpc":"2.0","method":"example","params":{"text":"café"}}"#;
        let framed = format!(
            "content-length: {}\r\nX-Test: yes\r\n\r\n{body}",
            body.len()
        );
        let message = read_message(&mut framed.as_bytes()).unwrap().unwrap();
        assert_eq!(message["params"]["text"], "café");
    }

    #[test]
    fn writes_an_lsp_frame_that_round_trips() {
        let message = serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"ok":true}});
        let mut output = Vec::new();
        write_message(&mut output, &message).unwrap();
        assert_eq!(read_message(&mut output.as_slice()).unwrap(), Some(message));
    }

    #[test]
    fn rejects_missing_conflicting_and_oversized_content_lengths() {
        assert!(matches!(
            read_message(&mut "X-Test: yes\r\n\r\n{}".as_bytes()),
            Err(ProtocolError::MissingContentLength)
        ));
        assert!(matches!(
            read_message(&mut "Content-Length: 2\r\ncontent-length: 3\r\n\r\n{}".as_bytes()),
            Err(ProtocolError::InvalidHeader)
        ));
        let oversized = format!("Content-Length: {}\r\n\r\n", MAX_MESSAGE_BYTES + 1);
        assert!(matches!(
            read_message(&mut oversized.as_bytes()),
            Err(ProtocolError::MessageTooLarge)
        ));
    }

    #[test]
    fn reports_truncated_frames_without_reusing_partial_json() {
        let result = read_message(&mut "Content-Length: 10\r\n\r\n{}".as_bytes());
        assert!(matches!(result, Err(ProtocolError::Io(_))));
    }
}
