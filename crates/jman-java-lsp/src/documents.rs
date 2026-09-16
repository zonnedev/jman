use std::collections::HashMap;

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Document {
    pub version: i64,
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Documents {
    open: HashMap<String, Document>,
}

impl Documents {
    pub fn open(&mut self, uri: String, version: i64, text: String) {
        self.open.insert(uri, Document { version, text });
    }

    pub fn change(
        &mut self,
        uri: &str,
        version: i64,
        changes: &[Value],
    ) -> Result<&Document, &'static str> {
        let document = self.open.get_mut(uri).ok_or("document is not open")?;
        if version <= document.version {
            return Err("document version is not newer");
        }
        for change in changes {
            let replacement = change
                .get("text")
                .and_then(Value::as_str)
                .ok_or("change has no text")?;
            if let Some(range) = change.get("range") {
                let start = position_to_byte(&document.text, &range["start"])?;
                let end = position_to_byte(&document.text, &range["end"])?;
                if start > end {
                    return Err("change range is reversed");
                }
                document.text.replace_range(start..end, replacement);
            } else {
                document.text.clear();
                document.text.push_str(replacement);
            }
        }
        document.version = version;
        Ok(document)
    }

    pub fn get(&self, uri: &str) -> Option<&Document> {
        self.open.get(uri)
    }

    pub fn close(&mut self, uri: &str) -> Option<Document> {
        self.open.remove(uri)
    }

    pub fn uris(&self) -> impl Iterator<Item = &str> {
        self.open.keys().map(String::as_str)
    }
}

pub fn position_to_byte(text: &str, position: &Value) -> Result<usize, &'static str> {
    let line = position
        .get("line")
        .and_then(Value::as_u64)
        .ok_or("position has no line")?;
    let character = position
        .get("character")
        .and_then(Value::as_u64)
        .ok_or("position has no character")?;
    let line_start = if line == 0 {
        0
    } else {
        text.match_indices('\n')
            .nth(line as usize - 1)
            .map(|(index, _)| index + 1)
            .ok_or("line is outside document")?
    };
    let line_end = text[line_start..]
        .find('\n')
        .map(|offset| line_start + offset)
        .unwrap_or(text.len());
    let mut utf16 = 0_u64;
    for (offset, value) in text[line_start..line_end].char_indices() {
        if utf16 == character {
            return Ok(line_start + offset);
        }
        utf16 += value.len_utf16() as u64;
        if utf16 > character {
            return Err("position splits a UTF-16 surrogate pair");
        }
    }
    if utf16 == character {
        Ok(line_end)
    } else {
        Err("character is outside line")
    }
}

pub fn offset_to_position(text: &str, utf16_offset: u64) -> Value {
    let mut consumed = 0_u64;
    let mut line = 0_u64;
    let mut character = 0_u64;
    for value in text.chars() {
        if consumed >= utf16_offset {
            break;
        }
        let width = value.len_utf16() as u64;
        if consumed + width > utf16_offset {
            break;
        }
        consumed += width;
        if value == '\n' {
            line += 1;
            character = 0;
        } else {
            character += width;
        }
    }
    serde_json::json!({"line": line, "character": character})
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn applies_incremental_edits_using_utf16_positions() {
        let mut documents = Documents::default();
        documents.open("file:///Test.java".to_owned(), 1, "a😀b\n".to_owned());
        documents
            .change(
                "file:///Test.java",
                2,
                &[json!({
                    "range": {
                        "start": {"line": 0, "character": 1},
                        "end": {"line": 0, "character": 3}
                    },
                    "text": "X"
                })],
            )
            .unwrap();
        assert_eq!(documents.get("file:///Test.java").unwrap().text, "aXb\n");
    }

    #[test]
    fn converts_javac_utf16_offsets_to_lsp_positions() {
        assert_eq!(
            offset_to_position("a😀b\nnext", 3),
            json!({"line":0,"character":3})
        );
        assert_eq!(
            offset_to_position("a😀b\nnext", 5),
            json!({"line":1,"character":0})
        );
    }
}
