//! User turns inside a pane transcript.
//!
//! A direct paste is an envelope. The body between the `---` markers is the
//! turn restore fingerprints. The surrounding From/To lines are not extra text.

use crewmux_transcript::{TranscriptCursor, UserTurn};

/// Envelope bodies whose `From:` line starts at or after `after`.
pub(crate) fn envelope_turns(bytes: &[u8], after: u64) -> Vec<UserTurn> {
    let text = String::from_utf8_lossy(bytes);
    let mut turns = Vec::new();
    let mut search = 0usize;
    while let Some(found) = text.get(search..).and_then(|tail| tail.find("From: ")) {
        let at = search.saturating_add(found);
        if !line_start(&text, at) {
            search = at.saturating_add(1);
            continue;
        }
        let Some(chunk) = text.get(at..) else {
            break;
        };
        let Some(extracted) = extract_body(chunk) else {
            search = at.saturating_add("From: ".len());
            continue;
        };
        let byte_at = u64::try_from(at).unwrap_or(u64::MAX);
        if byte_at >= after {
            turns.push(UserTurn {
                text: extracted.body,
                at: TranscriptCursor(byte_at),
            });
        }
        search = at.saturating_add(extracted.consumed.max(1));
    }
    turns
}

struct Extracted {
    body: String,
    consumed: usize,
}

fn line_start(text: &str, at: usize) -> bool {
    at == 0 || text.as_bytes().get(at.saturating_sub(1)) == Some(&b'\n')
}

fn extract_body(chunk: &str) -> Option<Extracted> {
    let rest = chunk.strip_prefix("From: ")?;
    let rest = after_line(rest)?;
    let rest = rest.strip_prefix("To: ")?;
    let rest = after_line(rest)?;
    let rest = rest.strip_prefix("---")?;
    let rest = after_line(rest)?;
    let (body, after_mark) = rest.split_once("\n---")?;
    let after = after_line(after_mark)?;
    let consumed = chunk.len().saturating_sub(after.len());
    Some(Extracted {
        body: normalize_body(body),
        consumed,
    })
}

/// A pty writes `\r\n`. The marker is still `---`, and the body is compared as LF.
fn after_line(text: &str) -> Option<&str> {
    let (_, rest) = text.split_once('\n')?;
    Some(rest)
}

fn normalize_body(body: &str) -> String {
    body.trim_end_matches('\r').replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::envelope_turns;

    #[test]
    fn body_between_markers_is_the_turn() {
        let text = "banner\nFrom: a@h\nTo: b@r\n---\nhello\n---\n↩ Reply: x\n";
        let turns = envelope_turns(text.as_bytes(), 0);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns.first().map(|turn| turn.text.as_str()), Some("hello"));
    }

    #[test]
    fn a_cursor_hides_earlier_envelopes() {
        let text = "From: a@h\nTo: b@r\n---\none\n---\n\nFrom: a@h\nTo: b@r\n---\ntwo\n---\n";
        let turns = envelope_turns(text.as_bytes(), 0);
        assert_eq!(turns.len(), 2);
        let later = turns.get(1).map_or(0, |turn| turn.at.0);
        let after = envelope_turns(text.as_bytes(), later);
        assert_eq!(after.len(), 1);
        assert_eq!(after.first().map(|turn| turn.text.as_str()), Some("two"));
    }

    #[test]
    fn crlf_envelopes_keep_the_body() {
        let text = "From: a@h\r\nTo: b@r\r\n---\r\nm06\r\n---\r\nstub> \r\n";
        let turns = envelope_turns(text.as_bytes(), 0);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns.first().map(|turn| turn.text.as_str()), Some("m06"));
    }
}
