//! Per-provider delivery text (ADR-0009 Amendment 2).
//!
//! Codex is identity before the shared cleanup. Claude unwraps one
//! `<pasted_content>` wrapper and expands tabs to four spaces.

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crewmux_types::RuntimeKind;

/// `provider_unwrap` then the shared cleanup.
#[must_use]
pub fn normalize_for(runtime: RuntimeKind, text: &str) -> String {
    let unwrapped = match runtime {
        RuntimeKind::ClaudeCode => unwrap_claude(text),
        RuntimeKind::Codex | RuntimeKind::OpenCode | RuntimeKind::Terminal | RuntimeKind::Stub => {
            text.to_owned()
        }
    };
    common(&unwrapped)
}

/// Lowercase hex sha256 of [`normalize_for`].
#[must_use]
pub fn delivery_fingerprint(runtime: RuntimeKind, text: &str) -> String {
    let digest = Sha256::digest(normalize_for(runtime, text).as_bytes());
    hex(digest.as_slice())
}

/// Fingerprint equality. Codex pastes that differ only by the shared cleanup still match.
#[must_use]
pub fn same_delivery(runtime: RuntimeKind, sent: &str, stored: &str) -> bool {
    delivery_fingerprint(runtime, sent) == delivery_fingerprint(runtime, stored)
}

fn unwrap_claude(text: &str) -> String {
    let body = text.strip_prefix('\n').unwrap_or(text);
    if let Some(inner) = strip_pasted(body) {
        expand_tabs(&inner)
    } else {
        expand_tabs(text)
    }
}

fn strip_pasted(text: &str) -> Option<String> {
    let rest = text.strip_prefix("<pasted_content")?;
    let close = rest.find('>')?;
    let after = rest.get(close.saturating_add(1)..)?;
    let after = after.strip_prefix('\n').unwrap_or(after);
    let inner = after
        .strip_suffix("\n</pasted_content>\n")
        .or_else(|| after.strip_suffix("\n</pasted_content>"))
        .or_else(|| after.strip_suffix("</pasted_content>\n"))
        .or_else(|| after.strip_suffix("</pasted_content>"))?;
    Some(inner.to_owned())
}

fn expand_tabs(text: &str) -> String {
    text.replace('\t', "    ")
}

fn common(text: &str) -> String {
    let nfc: String = text.nfc().collect();
    let lf = nfc.replace("\r\n", "\n").replace('\r', "\n");
    let stripped = lf.replace("\u{1b}[200~", "").replace("\u{1b}[201~", "");
    let mut lines: Vec<String> = stripped
        .split('\n')
        .map(|line| line.trim_end().to_owned())
        .collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::new();
    for byte in bytes {
        let hi = usize::from(byte >> 4);
        let lo = usize::from(byte & 0x0f);
        if let (Some(high), Some(low)) = (HEX.get(hi), HEX.get(lo)) {
            out.push(char::from(*high));
            out.push(char::from(*low));
        }
    }
    out
}
