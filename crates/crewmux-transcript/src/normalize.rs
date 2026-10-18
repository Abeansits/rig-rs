//! Per-provider delivery text, ADR-0009 Amendment 2.
//!
//! Claude unwraps one outermost `<pasted_content>` wrapper and expands tabs to
//! four spaces. Codex, `OpenCode`, the stub, and a plain terminal stay
//! byte-oriented aside from the shared whitespace and NFC rules.

use crewmux_types::RuntimeKind;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

/// `sha256(normalize_for(runtime, text))`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub [u8; 32]);

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for Fingerprint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Fingerprint({self})")
    }
}

/// How a stored turn relates to the text we sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnMatch {
    /// Normalised texts are equal.
    Equal,
    /// Normalised texts differ.
    Different,
    /// The turn merged our paste with other text. Do not ack or resend.
    Composite,
}

/// `sha256(normalize_for(runtime, text))`.
#[must_use]
pub fn fingerprint(runtime: RuntimeKind, text: &str) -> Fingerprint {
    let digest = Sha256::digest(normalize_for(runtime, text).as_bytes());
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&digest);
    Fingerprint(bytes)
}

/// `sha256` of the raw bytes, with no unwrap and no shared cleanup.
#[must_use]
pub fn fingerprint_raw(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// Provider unwrap, then the shared cleanup.
#[must_use]
pub fn normalize_for(runtime: RuntimeKind, text: &str) -> String {
    let unwrapped = match runtime {
        RuntimeKind::ClaudeCode => unwrap_outermost(text).unwrap_or_else(|| text.to_owned()),
        RuntimeKind::Codex | RuntimeKind::OpenCode | RuntimeKind::Stub | RuntimeKind::Terminal => {
            text.to_owned()
        }
    };
    let expanded = match runtime {
        RuntimeKind::ClaudeCode => unwrapped.replace('\t', "    "),
        RuntimeKind::Codex | RuntimeKind::OpenCode | RuntimeKind::Stub | RuntimeKind::Terminal => {
            unwrapped
        }
    };
    common(&expanded)
}

/// Compare using the runtime that stored the turn.
#[must_use]
pub fn match_turn(runtime: RuntimeKind, sent: &str, stored: &str) -> TurnMatch {
    let sent_norm = normalize_for(runtime, sent);
    let stored_norm = normalize_for(runtime, stored);
    if sent_norm == stored_norm {
        return TurnMatch::Equal;
    }
    if runtime == RuntimeKind::ClaudeCode
        && wrapper_lines(stored) > 1
        && !sent_norm.is_empty()
        && stored_norm.contains(&sent_norm)
    {
        return TurnMatch::Composite;
    }
    TurnMatch::Different
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
/// Leading and trailing whitespace may surround the tag. Claude Code 2.1.287
/// writes the close tag as `</pasted_content id="…">` and may put two newlines
/// before the open tag. The id is not a content hash: the same id is reused
/// for later pastes, so equality is the inner text. A `<pasted_content`
/// literal in the middle stays. Two wrappers stay wrapped.
#[must_use]
pub fn unwrap_outermost(text: &str) -> Option<String> {
    let rest = text.trim_matches(|ch: char| matches!(ch, '\n' | '\r' | ' ' | '\t'));
    let after_name = rest.strip_prefix("<pasted_content")?;
    let end = after_name.find('>')?;
    let after_tag = after_name.get(end.saturating_add(1)..)?;
    let body_and_close = after_tag.strip_prefix('\n').unwrap_or(after_tag);
    let close_at = body_and_close.rfind("</pasted_content")?;
    let inner = body_and_close.get(..close_at)?;
    if inner.contains("</pasted_content>") {
        return None;
    }
    let after_close = body_and_close.get(close_at..)?;
    let tag_end = after_close.find('>')?;
    let tail = after_close.get(tag_end.saturating_add(1)..)?;
    if !tail.trim().is_empty() {
        return None;
    }
    Some(inner.strip_suffix('\n').unwrap_or(inner).to_owned())
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
    use crewmux_types::RuntimeKind;

    use super::{TurnMatch, fingerprint, match_turn, normalize_for};

    #[test]
    fn literal_tag_in_the_middle_is_unchanged() {
        let body = "keep <pasted_content id=\"x\"> this";
        assert_eq!(normalize_for(RuntimeKind::ClaudeCode, body), body);
    }

    #[test]
    fn internal_space_does_not_match() {
        let sent = "a b";
        let stored = "ab";
        assert_ne!(
            fingerprint(RuntimeKind::ClaudeCode, sent),
            fingerprint(RuntimeKind::ClaudeCode, stored)
        );
    }

    #[test]
    fn literal_pasted_content_inside_one_wrapper_is_equal() {
        let sent = "see <pasted_content in the docs";
        let stored = "<pasted_content>\nsee <pasted_content in the docs\n</pasted_content>";
        assert_eq!(super::pasted_openings(stored), 1);
        assert_eq!(
            match_turn(RuntimeKind::ClaudeCode, sent, stored),
            TurnMatch::Equal
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
            match_turn(RuntimeKind::ClaudeCode, "hello", composite),
            TurnMatch::Composite
        );
    }

    #[test]
    fn tabs_match_spaces_only_for_claude() {
        let tabs = "a\tb";
        let spaces = "a    b";
        assert_eq!(
            fingerprint(RuntimeKind::ClaudeCode, tabs),
            fingerprint(RuntimeKind::ClaudeCode, spaces)
        );
        assert_ne!(
            fingerprint(RuntimeKind::Codex, tabs),
            fingerprint(RuntimeKind::Codex, spaces)
        );
        assert_eq!(
            match_turn(RuntimeKind::ClaudeCode, tabs, spaces),
            TurnMatch::Equal
        );
        assert_eq!(
            match_turn(RuntimeKind::Codex, tabs, spaces),
            TurnMatch::Different
        );
    }
}
