//! Post-paste screen check.
//!
//! The check captures with `capture-pane -J`, so tmux rejoins soft-wrapped rows
//! before we look for the payload. [`crewmux_mux::FakeMux`] stores logical lines
//! and ignores `-J`. The stub may still echo the payload before the check runs.
//! A byte-for-byte `before + payload` compare only matches the fake multiplexer.

use std::time::{Duration, Instant};

use crewmux_mux::{CaptureRange, Key, PaneRef};

use crate::composer::stub_drafting;
use crate::error::Error;
use crate::state::App;

/// What the pane shows after a paste, relative to the payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PasteHit {
    /// The payload is present and no unexplained line was added.
    Clean,
    /// Text landed that is neither the previous screen nor the payload.
    Contaminated,
    /// The payload cannot be found.
    Missing,
}

/// Classify `after` against the screen captured before the paste.
#[must_use]
pub(crate) fn classify_paste(before: &str, after: &str, payload: &str) -> PasteHit {
    let after_flat = flatten(after);
    let payload_flat = flatten(payload);
    if !located(&after_flat, &payload_flat) {
        return PasteHit::Missing;
    }
    if leftover(before, after, &payload_flat) {
        PasteHit::Contaminated
    } else {
        PasteHit::Clean
    }
}

/// `FakeMux` appends the payload and nothing else. A padded pane never does.
#[must_use]
pub(crate) fn exact_append(before: &str, after: &str, payload: &str) -> bool {
    let mut expected = String::with_capacity(before.len().saturating_add(payload.len()));
    expected.push_str(before);
    expected.push_str(payload);
    after == expected
}

/// Recapture until a real pane has had time to echo the paste.
///
/// An exact append is the in-memory multiplexer: the screen will not change.
/// A contaminated screen is already final. Anything else may still be
/// receiving the application's echo, including `EXTRA` from a contaminating stub.
pub(crate) async fn pasted_screen(
    app: &App,
    pane: &PaneRef,
    before: &str,
    payload: &str,
) -> Result<String, Error> {
    let mut after = app.mux.capture(pane, CaptureRange::Joined).await?.text;
    let hit = classify_paste(before, &after, payload);
    if exact_append(before, &after, payload) || hit == PasteHit::Contaminated {
        return Ok(after);
    }
    let started = Instant::now();
    let floor = Duration::from_millis(150);
    let deadline = started + Duration::from_millis(500);
    loop {
        let hit = classify_paste(before, &after, payload);
        if hit == PasteHit::Contaminated {
            return Ok(after);
        }
        let waited = started.elapsed() >= floor;
        if waited && hit != PasteHit::Clean {
            return Ok(after);
        }
        if Instant::now() >= deadline {
            return Ok(after);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
        let next = app.mux.capture(pane, CaptureRange::Joined).await?.text;
        if waited && next == after && hit == PasteHit::Clean {
            return Ok(after);
        }
        after = next;
    }
}

/// Drop a failed paste from the composer.
///
/// `C-u` alone kills the line. Enter is not sent: on a real composer an empty
/// line would submit a turn. The stub redraws `stub>` when it sees `C-u`.
/// An unchanged screen means the multiplexer did not redraw; stop waiting.
pub(crate) async fn clear_composer(app: &App, pane: &PaneRef) -> Result<(), Error> {
    let started = Instant::now();
    let mut last = app.mux.capture(pane, CaptureRange::Visible).await?.text;
    app.mux.send_keys(pane, &[Key::CtrlU]).await?;
    let mut stable = Instant::now();
    loop {
        let text = app.mux.capture(pane, CaptureRange::Visible).await?.text;
        if !stub_drafting(&text) {
            return Ok(());
        }
        if text == last {
            if stable.elapsed() >= Duration::from_millis(80) {
                return Ok(());
            }
        } else {
            last = text;
            stable = Instant::now();
        }
        if started.elapsed() >= Duration::from_millis(500) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn located(after_flat: &str, payload_flat: &str) -> bool {
    let payload_flat = payload_flat.trim();
    if payload_flat.is_empty() {
        return false;
    }
    if after_flat.contains(payload_flat) {
        return true;
    }
    let mut any = false;
    for line in payload_flat.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        any = true;
        if !after_flat.contains(line) {
            return false;
        }
    }
    any
}

fn leftover(before: &str, after: &str, payload_flat: &str) -> bool {
    let previous = logical_lines(before);
    let payload_lines = logical_lines(payload_flat);
    for line in logical_lines(after) {
        if line == "stub>" || previous.iter().any(|old| old == &line) {
            continue;
        }
        if explained(&line, &payload_lines) {
            continue;
        }
        return true;
    }
    false
}

/// The stub echoes newline-terminated payload lines at the cursor, which is
/// still on the unterminated tail, so one screen row can be several payload
/// lines glued together.
fn explained(line: &str, parts: &[String]) -> bool {
    if line.is_empty() {
        return true;
    }
    for part in parts {
        if part.is_empty() {
            continue;
        }
        let Some(rest) = line.strip_prefix(part.as_str()) else {
            continue;
        };
        if rest.is_empty() || explained(rest, parts) {
            return true;
        }
    }
    false
}

fn logical_lines(text: &str) -> Vec<String> {
    flatten(text)
        .split('\n')
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

fn flatten(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "");
    let raw: Vec<&str> = normalized.split('\n').collect();
    let width = grid_width(&raw);
    let mut out = String::new();
    for line in raw {
        let hard = width
            .is_some_and(|wide| line.len() == wide && !line.ends_with(' ') && !line.is_empty());
        if hard {
            out.push_str(line);
            continue;
        }
        out.push_str(line.trim_end_matches(' '));
        out.push('\n');
    }
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Pane width is the longest row when at least two rows are padded out to it.
///
/// Padding is a trailing space. A repeated long composer line (the stub echoing
/// the payload) is not a grid, and one full-width line is not a wrap by itself.
fn grid_width(lines: &[&str]) -> Option<usize> {
    let max = lines.iter().map(|line| line.len()).max()?;
    if max < 2 {
        return None;
    }
    let padded = lines
        .iter()
        .filter(|line| line.len() == max && line.ends_with(' '))
        .count();
    (padded >= 2).then_some(max)
}

#[cfg(test)]
mod tests {
    use super::{PasteHit, classify_paste};

    fn payload() -> &'static str {
        "From: operator@host\nTo: one@pair\n---\nhello\n---\nreply-line"
    }

    fn pad(line: &str, width: usize) -> String {
        let mut owned = line.to_owned();
        while owned.len() < width {
            owned.push(' ');
        }
        owned
    }

    fn grid(lines: &[&str], width: usize) -> String {
        let mut text = String::new();
        for line in lines {
            text.push_str(&pad(line, width));
            text.push('\n');
        }
        text
    }

    #[test]
    fn exact_append_is_clean_and_extra_text_is_contaminated() {
        let before = "stub-agent\nstub> \n";
        let body = payload();
        let mut clean = String::new();
        clean.push_str(before);
        clean.push_str(body);
        assert_eq!(classify_paste(before, &clean, body), PasteHit::Clean);
        let mut dirty = clean.clone();
        dirty.push_str(" EXTRA");
        assert_eq!(classify_paste(before, &dirty, body), PasteHit::Contaminated);
    }

    #[test]
    fn padded_and_wrapped_panes_locate_the_payload() {
        let before = grid(&["stub-agent", "stub> "], 24);
        let mut lines = vec!["stub-agent".to_owned(), "stub> ".to_owned()];
        for line in payload().lines() {
            lines.push(line.to_owned());
        }
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let after = grid(&refs, 24);
        assert_eq!(classify_paste(&before, &after, payload()), PasteHit::Clean);

        let wrapped_before = grid(&["banner", "stub> "], 10);
        let mut wrapped = wrapped_before.clone();
        wrapped.push_str("0123456789\n");
        wrapped.push_str(&pad("abcdef", 10));
        wrapped.push('\n');
        let wide = "0123456789abcdef";
        assert_eq!(
            classify_paste(&wrapped_before, &wrapped, wide),
            PasteHit::Clean
        );
    }

    #[test]
    fn an_extra_padded_line_is_contaminated_and_a_blank_pane_is_missing() {
        let before = grid(&["stub-agent", "stub> "], 24);
        let mut lines = vec!["stub-agent".to_owned(), "stub> ".to_owned()];
        for line in payload().lines() {
            lines.push(line.to_owned());
        }
        lines.push("EXTRA".to_owned());
        lines.push("stub>".to_owned());
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let after = grid(&refs, 24);
        assert_eq!(
            classify_paste(&before, &after, payload()),
            PasteHit::Contaminated
        );
        assert_eq!(
            classify_paste(&before, &before, payload()),
            PasteHit::Missing
        );
    }

    #[test]
    fn echoed_payload_lines_and_a_new_prompt_are_clean() {
        let before = "stub-agent\nstub> \n";
        let body = payload();
        let mut after = String::new();
        after.push_str(before);
        after.push_str(body);
        after.push_str("\nstub>\n");
        after.push_str(body);
        after.push_str("\nstub>\n");
        assert_eq!(classify_paste(before, &after, body), PasteHit::Clean);
        let mut glued = String::new();
        glued.push_str(before);
        glued.push_str(body);
        glued.push_str("From: operator@host\nTo: one@pair\n---\nhello\n---\n");
        assert_eq!(classify_paste(before, &glued, body), PasteHit::Clean);
        let mut extra = glued.clone();
        extra.push_str("EXTRA\n");
        assert_eq!(classify_paste(before, &extra, body), PasteHit::Contaminated);
    }
}
