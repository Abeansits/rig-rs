//! Byte-preserving edits of one project entry in a Claude user-config object.

use serde_json::Value;

/// Result of setting `hasTrustDialogAccepted` on one project.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TrustEdit {
    /// The project entry already has the flag set. The caller must not write.
    Already,
    /// A new document with every other byte kept.
    Changed(String),
}

/// Set `projects."<project>".hasTrustDialogAccepted` to `true`.
///
/// Keys outside that project entry stay byte-for-byte. The project entry itself
/// may gain the flag or flip it to `true`.
///
/// # Errors
///
/// Returns a message when the document is not a JSON object, or when `projects`
/// or the project entry is present but not an object.
pub(crate) fn set_project_trust(original: &str, project: &str) -> Result<TrustEdit, String> {
    if original.trim().is_empty() {
        let key = serde_json::to_string(project).map_err(|err| err.to_string())?;
        return Ok(TrustEdit::Changed(format!(
            "{{\"projects\":{{{key}:{{\"hasTrustDialogAccepted\":true}}}}}}"
        )));
    }
    let parsed: Value = serde_json::from_str(original).map_err(|err| err.to_string())?;
    let Some(root) = parsed.as_object() else {
        return Err("Claude user config must be a JSON object".to_owned());
    };
    if project_already_trusted(root, project) {
        return Ok(TrustEdit::Already);
    }
    let bytes = original.as_bytes();
    let mut parser = Parser { bytes, index: 0 };
    parser.skip_ws();
    let entries = parser.object_entries()?;
    let close = parser
        .index
        .checked_sub(1)
        .ok_or_else(|| "object is empty".to_owned())?;
    let Some(projects) = entries.iter().find(|entry| entry.key == "projects") else {
        let key = serde_json::to_string(project).map_err(|err| err.to_string())?;
        let insertion = format!("\"projects\":{{{key}:{{\"hasTrustDialogAccepted\":true}}}}");
        return Ok(TrustEdit::Changed(insert_before(
            original,
            close,
            !entries.is_empty(),
            &insertion,
        )?));
    };
    let project_entries = entries_of(original, projects.value_start, projects.value_end)?;
    edit_projects(original, project, projects, &project_entries)
}

fn project_already_trusted(root: &serde_json::Map<String, Value>, project: &str) -> bool {
    root.get("projects")
        .and_then(Value::as_object)
        .and_then(|projects| projects.get(project))
        .and_then(Value::as_object)
        .and_then(|entry| entry.get("hasTrustDialogAccepted"))
        .and_then(Value::as_bool)
        == Some(true)
}

fn edit_projects(
    original: &str,
    project: &str,
    projects: &Entry,
    project_entries: &[Entry],
) -> Result<TrustEdit, String> {
    let Some(found) = project_entries.iter().find(|entry| entry.key == project) else {
        let key = serde_json::to_string(project).map_err(|err| err.to_string())?;
        let insertion = format!("{key}:{{\"hasTrustDialogAccepted\":true}}");
        let close = projects
            .value_end
            .checked_sub(1)
            .ok_or_else(|| "projects object is empty".to_owned())?;
        return Ok(TrustEdit::Changed(insert_before(
            original,
            close,
            !project_entries.is_empty(),
            &insertion,
        )?));
    };
    let inner = entries_of(original, found.value_start, found.value_end)?;
    if let Some(flag) = inner
        .iter()
        .find(|entry| entry.key == "hasTrustDialogAccepted")
    {
        return Ok(TrustEdit::Changed(replace_span(
            original,
            flag.value_start,
            flag.value_end,
            "true",
        )?));
    }
    let close = found
        .value_end
        .checked_sub(1)
        .ok_or_else(|| "project object is empty".to_owned())?;
    Ok(TrustEdit::Changed(insert_before(
        original,
        close,
        !inner.is_empty(),
        "\"hasTrustDialogAccepted\":true",
    )?))
}

fn entries_of(original: &str, start: usize, end: usize) -> Result<Vec<Entry>, String> {
    let bytes = original.as_bytes();
    let slice = bytes
        .get(start..end)
        .ok_or_else(|| "project span is out of range".to_owned())?;
    let mut parser = Parser {
        bytes: slice,
        index: 0,
    };
    parser.skip_ws();
    if parser.peek() != Some(b'{') {
        return Err("project entry must be a JSON object".to_owned());
    }
    let mut entries = parser.object_entries()?;
    for entry in &mut entries {
        entry.value_start = entry
            .value_start
            .checked_add(start)
            .ok_or_else(|| "span overflow".to_owned())?;
        entry.value_end = entry
            .value_end
            .checked_add(start)
            .ok_or_else(|| "span overflow".to_owned())?;
    }
    Ok(entries)
}

fn insert_before(
    original: &str,
    close: usize,
    needs_comma: bool,
    insertion: &str,
) -> Result<String, String> {
    let prefix = original
        .get(..close)
        .ok_or_else(|| "insert point is out of range".to_owned())?;
    let suffix = original
        .get(close..)
        .ok_or_else(|| "insert point is out of range".to_owned())?;
    let mut out = String::new();
    out.push_str(prefix);
    if needs_comma {
        out.push(',');
    }
    out.push_str(insertion);
    out.push_str(suffix);
    Ok(out)
}

fn replace_span(original: &str, start: usize, end: usize, text: &str) -> Result<String, String> {
    let prefix = original
        .get(..start)
        .ok_or_else(|| "replace start is out of range".to_owned())?;
    let suffix = original
        .get(end..)
        .ok_or_else(|| "replace end is out of range".to_owned())?;
    let mut out = String::new();
    out.push_str(prefix);
    out.push_str(text);
    out.push_str(suffix);
    Ok(out)
}

#[derive(Debug)]
struct Entry {
    key: String,
    value_start: usize,
    value_end: usize,
}

struct Parser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.index = self.index.saturating_add(1);
        Some(byte)
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.index = self.index.saturating_add(1);
        }
    }

    fn object_entries(&mut self) -> Result<Vec<Entry>, String> {
        if self.bump() != Some(b'{') {
            return Err("expected object".to_owned());
        }
        let mut entries = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.bump();
                return Ok(entries);
            }
            let key_start = self.index;
            self.skip_string()?;
            let key_end = self.index;
            let key_raw = self.slice(key_start, key_end)?;
            let key: String = serde_json::from_str(key_raw).map_err(|err| err.to_string())?;
            self.skip_ws();
            if self.bump() != Some(b':') {
                return Err("expected colon".to_owned());
            }
            self.skip_ws();
            let value_start = self.index;
            self.skip_value()?;
            entries.push(Entry {
                key,
                value_start,
                value_end: self.index,
            });
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.bump();
                }
                Some(b'}') => {
                    self.bump();
                    return Ok(entries);
                }
                _ => return Err("expected comma or end of object".to_owned()),
            }
        }
    }

    fn skip_value(&mut self) -> Result<(), String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => {
                self.object_entries()?;
                Ok(())
            }
            Some(b'[') => self.skip_array(),
            Some(b'"') => self.skip_string(),
            Some(b't') => self.consume(b"true"),
            Some(b'f') => self.consume(b"false"),
            Some(b'n') => self.consume(b"null"),
            Some(b'-' | b'0'..=b'9') => self.skip_number(),
            _ => Err("expected JSON value".to_owned()),
        }
    }

    fn skip_array(&mut self) -> Result<(), String> {
        if self.bump() != Some(b'[') {
            return Err("expected array".to_owned());
        }
        loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.bump();
                return Ok(());
            }
            self.skip_value()?;
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.bump();
                }
                Some(b']') => {
                    self.bump();
                    return Ok(());
                }
                _ => return Err("expected comma or end of array".to_owned()),
            }
        }
    }

    fn skip_string(&mut self) -> Result<(), String> {
        if self.bump() != Some(b'"') {
            return Err("expected string".to_owned());
        }
        while let Some(byte) = self.bump() {
            if byte == b'\\' {
                if self.bump().is_none() {
                    return Err("truncated escape".to_owned());
                }
                continue;
            }
            if byte == b'"' {
                return Ok(());
            }
        }
        Err("unterminated string".to_owned())
    }

    fn skip_number(&mut self) -> Result<(), String> {
        let start = self.index;
        while self
            .peek()
            .is_some_and(|byte| matches!(byte, b'+' | b'-' | b'.' | b'e' | b'E' | b'0'..=b'9'))
        {
            self.index = self.index.saturating_add(1);
        }
        if self.index == start {
            return Err("expected number".to_owned());
        }
        Ok(())
    }

    fn consume(&mut self, literal: &[u8]) -> Result<(), String> {
        for expected in literal {
            if self.bump() != Some(*expected) {
                return Err("unexpected literal".to_owned());
            }
        }
        Ok(())
    }

    fn slice(&self, start: usize, end: usize) -> Result<&str, String> {
        let bytes = self
            .bytes
            .get(start..end)
            .ok_or_else(|| "slice is out of range".to_owned())?;
        std::str::from_utf8(bytes).map_err(|err| err.to_string())
    }
}
