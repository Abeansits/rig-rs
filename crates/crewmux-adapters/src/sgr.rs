//! Display-sequence removal for classifiers that do not interpret SGR.
//!
//! Claude reads faint suggestions from the raw capture. Codex and the stub
//! strip first, so a bold marker, a dim placeholder, or an OSC 8 hyperlink is
//! the same text tmux would show without `capture-pane -e`.

/// Drop CSI and OSC sequences. Line breaks stay.
///
/// CSI ends on a final byte in `0x40..=0x7E` (`@` through `~`). OSC ends on
/// BEL or ST (`ESC \`). A `~` or `@` final does not swallow the text after it.
#[must_use]
pub fn strip_csi(text: &str) -> String {
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
    if let Some(rest) = rest.strip_prefix('[') {
        return skip_csi(rest);
    }
    if let Some(rest) = rest.strip_prefix(']') {
        return skip_osc(rest);
    }
    None
}

/// CSI: parameters and intermediates, then one final byte `0x40..=0x7E`.
fn skip_csi(rest: &str) -> Option<&str> {
    let end = rest
        .bytes()
        .position(|byte| (0x40..=0x7E).contains(&byte))?;
    rest.get(end.saturating_add(1)..)
}

/// OSC: payload through BEL, or through ST (`ESC \`).
fn skip_osc(rest: &str) -> Option<&str> {
    if let Some(end) = rest.find('\u{7}') {
        return rest.get(end.saturating_add(1)..);
    }
    let mut from = 0usize;
    while let Some(rel) = rest.get(from..).and_then(|tail| tail.find('\u{1b}')) {
        let at = from.saturating_add(rel);
        if rest.as_bytes().get(at.saturating_add(1)) == Some(&b'\\') {
            return rest.get(at.saturating_add(2)..);
        }
        let next = at.saturating_add(1);
        if next <= from {
            break;
        }
        from = next;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::strip_csi;

    #[test]
    fn csi_at_and_tilde_finals_do_not_swallow_text() {
        assert_eq!(strip_csi("\u{1b}[0@kept"), "kept");
        assert_eq!(strip_csi("\u{1b}[?25~kept"), "kept");
        assert_eq!(strip_csi("\u{1b}[1mkept"), "kept");
    }

    #[test]
    fn osc_8_hyperlinks_drop_the_url_and_keep_the_label() {
        let bel = "\u{1b}]8;;https://example.test\u{7}› Ask\u{1b}]8;;\u{7}";
        assert_eq!(strip_csi(bel), "› Ask");
        let st = "\u{1b}]8;;https://example.test\u{1b}\\› Ask\u{1b}]8;;\u{1b}\\";
        assert_eq!(strip_csi(st), "› Ask");
    }
}
