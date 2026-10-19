//! CSI removal for classifiers that do not interpret SGR.
//!
//! Claude reads faint suggestions from the raw capture. Codex and the stub
//! strip first, so a bold marker or a dim placeholder is the same text tmux
//! would show without `capture-pane -e`.

/// Drop CSI sequences. Line breaks stay.
#[must_use]
pub(crate) fn strip_csi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(next) = skip_esc(rest) {
            rest = next;
            continue;
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        out.push(ch);
        rest = rest.get(ch.len_utf8()..).unwrap_or("");
    }
    out
}

pub(crate) fn skip_esc(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('\u{1b}')?;
    let rest = rest.strip_prefix('[')?;
    let end = rest.find(|ch: char| ch.is_ascii_alphabetic())?;
    rest.get(end.saturating_add(1)..)
}
