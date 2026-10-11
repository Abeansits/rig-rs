//! Marker block around the Codex tables this crate owns.
//!
//! Removal finds the markers in the on-disk text, deletes that span, and leaves
//! every other line in place. The markers are comments, so Codex still parses
//! the explicit tables inside them.

/// Opens the block this crate appends to `config.toml`.
pub(crate) const BEGIN: &str = "# >>> rig-rs managed >>>";

/// Closes the block this crate appends to `config.toml`.
pub(crate) const END: &str = "# <<< rig-rs managed <<<";

/// Insert `table` inside the managed block, creating the block when it is absent.
///
/// A leading newline is part of a new block so removing the block restores the
/// bytes that were there before the first insertion.
#[must_use]
pub(crate) fn insert_table(rendered: &str, table: &str) -> String {
    let table = with_newline(table);
    if let Some(end_at) = block_end(rendered) {
        let mut out = String::new();
        out.push_str(rendered.get(..end_at).unwrap_or(rendered));
        out.push_str(&table);
        out.push_str(rendered.get(end_at..).unwrap_or(""));
        return out;
    }
    let mut out = rendered.to_owned();
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(BEGIN);
    out.push('\n');
    out.push_str(&table);
    out.push_str(END);
    out.push('\n');
    out
}

/// Delete the managed block, including the separator newline added with it.
#[must_use]
pub(crate) fn strip_block(text: &str) -> String {
    let Some(begin) = text.find(BEGIN) else {
        return text.to_owned();
    };
    let Some(relative) = text.get(begin..).and_then(|tail| tail.find(END)) else {
        return text.to_owned();
    };
    let mut end = begin.saturating_add(relative).saturating_add(END.len());
    if text.as_bytes().get(end) == Some(&b'\n') {
        end = end.saturating_add(1);
    }
    let start = if begin > 0 && text.as_bytes().get(begin.saturating_sub(1)) == Some(&b'\n') {
        begin.saturating_sub(1)
    } else {
        begin
    };
    let mut out = String::new();
    out.push_str(text.get(..start).unwrap_or(""));
    out.push_str(text.get(end..).unwrap_or(""));
    out
}

fn block_end(text: &str) -> Option<usize> {
    let begin = text.find(BEGIN)?;
    let relative = text.get(begin..)?.find(END)?;
    Some(begin.saturating_add(relative))
}

fn with_newline(table: &str) -> String {
    if table.ends_with('\n') {
        table.to_owned()
    } else {
        format!("{table}\n")
    }
}
