//! Per-provider delivery text, ADR-0009 Amendment 2.
//!
//! Claude unwraps one outermost `<pasted_content>` wrapper and expands tabs to
//! four spaces. Codex, `OpenCode`, and the stub stay byte-oriented aside from
//! the shared whitespace and NFC rules.

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

/// Which runtime stored the user turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryRuntime {
    /// Claude Code.
    Claude,
    /// Codex.
    Codex,
    /// `OpenCode`.
    OpenCode,
    /// The in-tree stub.
    Stub,
}

/// How a stored turn relates to the text we sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryMatch {
    /// Normalised texts are equal.
    Equal,
    /// Normalised texts differ.
    Different,
    /// The turn merged our paste with other text. Do not ack or resend.
    Composite,
}

/// `sha256(normalize_for(runtime, text))` as lowercase hex.
#[must_use]
pub fn fingerprint(runtime: DeliveryRuntime, text: &str) -> String {
    hex(&Sha256::digest(normalize_for(runtime, text).as_bytes()))
}

/// `sha256` of the raw bytes, with no unwrap and no shared cleanup.
#[must_use]
pub fn fingerprint_raw(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// Provider unwrap, then the shared cleanup.
#[must_use]
pub fn normalize_for(runtime: DeliveryRuntime, text: &str) -> String {
    let unwrapped = match runtime {
        DeliveryRuntime::Claude => unwrap_outermost(text).unwrap_or_else(|| text.to_owned()),
        DeliveryRuntime::Codex | DeliveryRuntime::OpenCode | DeliveryRuntime::Stub => {
            text.to_owned()
        }
    };
    let expanded = match runtime {
        DeliveryRuntime::Claude => unwrapped.replace('\t', "    "),
        DeliveryRuntime::Codex | DeliveryRuntime::OpenCode | DeliveryRuntime::Stub => unwrapped,
    };
    common(&expanded)
}

/// Compare using the runtime that stored the turn.
#[must_use]
pub fn match_for(runtime: DeliveryRuntime, sent: &str, stored: &str) -> DeliveryMatch {
    let sent_norm = normalize_for(runtime, sent);
    let stored_norm = normalize_for(runtime, stored);
    if sent_norm == stored_norm {
        return DeliveryMatch::Equal;
    }
    if runtime == DeliveryRuntime::Claude
        && wrapper_lines(stored) > 1
        && !sent_norm.is_empty()
        && stored_norm.contains(&sent_norm)
    {
        return DeliveryMatch::Composite;
    }
    DeliveryMatch::Different
}

/// Opening wrapper lines. A literal `<pasted_content` inside a sentence is not one.
#[must_use]
pub fn pasted_openings(text: &str) -> usize {
    wrapper_lines(text)
}

fn wrapper_lines(text: &str) -> usize {
    text.lines()
        .filter(|line| is_wrapper_line(line.trim()))
        .count()
}

fn is_wrapper_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("<pasted_content") else {
        return false;
    };
    rest.contains('>')
}

/// Remove one outermost wrapper when the whole string is that wrapper.
///
/// The match is anchored. A `<pasted_content` literal in the middle stays.
#[must_use]
pub fn unwrap_outermost(text: &str) -> Option<String> {
    let rest = text.strip_prefix('\n').unwrap_or(text);
    let after_name = rest.strip_prefix("<pasted_content")?;
    let end = after_name.find('>')?;
    let after_tag = after_name.get(end.saturating_add(1)..)?;
    let body_and_close = after_tag.strip_prefix('\n')?;
    let trimmed = body_and_close.strip_suffix('\n').unwrap_or(body_and_close);
    let inner = trimmed.strip_suffix("</pasted_content>")?;
    if inner.contains("</pasted_content>") {
        return None;
    }
    Some(inner.to_owned())
}

fn common(text: &str) -> String {
    let nfc: String = text.nfc().collect();
    let lf = nfc.replace("\r\n", "\n").replace('\r', "\n");
    let stripped = strip_bracketed_paste(&lf);
    let mut lines: Vec<String> = stripped
        .lines()
        .map(|line| line.trim_end().to_owned())
        .collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

fn strip_bracketed_paste(text: &str) -> String {
    text.replace("\u{1b}[200~", "").replace("\u{1b}[201~", "")
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(nibble: u8) -> char {
    char::from(match nibble {
        0..=9 => b'0'.saturating_add(nibble),
        _ => b'a'.saturating_add(nibble.saturating_sub(10)),
    })
}

#[cfg(test)]
mod tests {
    use super::{DeliveryMatch, DeliveryRuntime, fingerprint, match_for, normalize_for};

    #[test]
    fn literal_tag_in_the_middle_is_unchanged() {
        let body = "keep <pasted_content id=\"x\"> this";
        assert_eq!(normalize_for(DeliveryRuntime::Claude, body), body);
    }

    #[test]
    fn internal_space_does_not_match() {
        let sent = "a b";
        let stored = "ab";
        assert_ne!(
            fingerprint(DeliveryRuntime::Claude, sent),
            fingerprint(DeliveryRuntime::Claude, stored)
        );
    }

    #[test]
    fn literal_pasted_content_inside_one_wrapper_is_equal() {
        let sent = "see <pasted_content in the docs";
        let stored = "<pasted_content>\nsee <pasted_content in the docs\n</pasted_content>";
        assert_eq!(super::pasted_openings(stored), 1);
        assert_eq!(
            match_for(DeliveryRuntime::Claude, sent, stored),
            DeliveryMatch::Equal
        );
        let composite = "\
<pasted_content>
hello
</pasted_content>
<pasted_content>
world
</pasted_content>
";
        assert!(super::pasted_openings(composite) > 1);
        assert_eq!(
            match_for(DeliveryRuntime::Claude, "hello", composite),
            DeliveryMatch::Composite
        );
    }

    #[test]
    fn tabs_match_spaces_only_for_claude() {
        let tabs = "a\tb";
        let spaces = "a    b";
        assert_eq!(
            fingerprint(DeliveryRuntime::Claude, tabs),
            fingerprint(DeliveryRuntime::Claude, spaces)
        );
        assert_ne!(
            fingerprint(DeliveryRuntime::Codex, tabs),
            fingerprint(DeliveryRuntime::Codex, spaces)
        );
        assert_eq!(
            match_for(DeliveryRuntime::Claude, tabs, spaces),
            DeliveryMatch::Equal
        );
        assert_eq!(
            match_for(DeliveryRuntime::Codex, tabs, spaces),
            DeliveryMatch::Different
        );
    }
}
