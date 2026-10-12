//! Post-paste check against the composer region, not the whole screen.
//!
//! The capture is `capture-pane -J -S -<n>`, so a payload taller than the
//! visible pane is still one logical block. [`ComposerParser`] isolates the
//! text after the stub prompt. Rows above that prompt are earlier output and
//! are not contamination. [`crewmux_mux::FakeMux`] stores logical lines and
//! ignores `-J`.

use std::time::{Duration, Instant};

use crewmux_mux::{CaptureRange, PaneRef};

use crate::composer::{ComposerParser, anchored_region, composer_empty};
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

/// Classify the composer region of `after` against the region at verdict time.
///
/// `before` and `after` are pane captures. Only the parser's composer region
/// is compared, so output above the prompt cannot look like a dirty paste.
#[must_use]
pub(crate) fn classify_paste(before: &str, after: &str, payload: &str) -> PasteHit {
    let parser = ComposerParser::Stub;
    // Width comes from the whole pane. The composer region starts mid-row, so
    // it no longer contains the padded rows that show where a wrap joins.
    let width = pane_width(before).or_else(|| pane_width(after));
    let before_flat = flat_region(parser, before, width);
    // The after-region starts at the verdict-time prompt (the last `stub> ` in
    // `before`), not at a later copy inside the payload or an earlier one.
    let after_flat = flatten_at(&composer_text(before, after), width);
    let payload_flat = flatten(payload);
    if !located(&after_flat, &payload_flat) {
        return PasteHit::Missing;
    }
    if leftover(&before_flat, &after_flat, &payload_flat) {
        PasteHit::Contaminated
    } else {
        PasteHit::Clean
    }
}

fn composer_text(before: &str, screen: &str) -> String {
    anchored_region(before, screen).unwrap_or_else(|| screen.to_owned())
}

fn flat_region(parser: ComposerParser, screen: &str, width: Option<usize>) -> String {
    let text = parser.region(screen).unwrap_or_else(|| screen.to_owned());
    flatten_at(&text, width)
}

/// Scrollback for `capture-pane -J -S -<n>`.
///
/// `-S -<n>` is `n` raw grid rows of history plus the visible pane. Wrapped
/// rows count toward `n`. `len / 20` overestimates a 90-column wrap; it is a
/// window, not a paste limit. The window is clamped to 500 history rows. A
/// paste whose prompt no longer fits fails closed: the payload is not found
/// and Enter is not sent. A 20_000-character line on a 90-column pane is about
/// 230 wrapped rows and still fits.
#[must_use]
pub(crate) fn history_range(payload: &str) -> CaptureRange {
    let wraps = payload.len() / 20;
    let breaks = payload.lines().count();
    let rows = wraps.saturating_add(breaks).saturating_add(32);
    let rows = u32::try_from(rows).unwrap_or(400).clamp(48, 500);
    CaptureRange::JoinedHistory(rows)
}

/// `FakeMux` appends the payload and nothing else. A padded pane never does.
#[must_use]
pub(crate) fn exact_append(before: &str, after: &str, payload: &str) -> bool {
    let mut expected = String::with_capacity(before.len().saturating_add(payload.len()));
    expected.push_str(before);
    expected.push_str(payload);
    after == expected
}

/// Recapture until the composer text matches across two captures.
///
/// An exact append is the in-memory multiplexer and is returned immediately.
/// A clean or contaminated screen is final only after 150ms, so a frame torn
/// mid-paste is not the verdict and Enter arrives after the stub's pause.
/// A missing payload keeps polling until the deadline.
pub(crate) async fn pasted_screen(
    app: &App,
    pane: &PaneRef,
    before: &str,
    payload: &str,
    range: CaptureRange,
) -> Result<String, Error> {
    let mut after = app.mux.capture(pane, range).await?.text;
    if exact_append(before, &after, payload) {
        return Ok(after);
    }
    let started = Instant::now();
    let deadline = started + Duration::from_millis(500);
    let mut seen = composer_text(before, &after);
    loop {
        if Instant::now() >= deadline {
            return Ok(after);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
        after = app.mux.capture(pane, range).await?.text;
        let text = composer_text(before, &after);
        let stable = text == seen;
        seen = text;
        if stable && settled(started.elapsed(), before, &after, payload) {
            return Ok(after);
        }
    }
}

/// The stub treats a newline as Enter only after a 40ms gap. Returning sooner
/// delivers the key while the paste is still one burst, and the prompt is not
/// redrawn. Missing means the paste may still be landing.
fn settled(elapsed: Duration, before: &str, after: &str, payload: &str) -> bool {
    if elapsed < Duration::from_millis(150) {
        return false;
    }
    match classify_paste(before, after, payload) {
        PasteHit::Clean | PasteHit::Contaminated => true,
        PasteHit::Missing => false,
    }
}

/// Drop a failed paste. Enter is never sent.
///
/// The stub's clear is `C-u`, once per pasted line and at least 8 times.
/// A tall buffer paste can still fill the pane after that budget: the prompt
/// scrolls off and the seat looks like `unrecognised_screen`. One `C-g` drops
/// the rest of the draft and redraws the prompt. Returns whether a recapture
/// shows an empty composer. A composer that stays dirty is reported by the
/// caller; this does not submit.
pub(crate) async fn clear_composer(
    app: &App,
    pane: &PaneRef,
    range: CaptureRange,
    lines: usize,
) -> Result<bool, Error> {
    let parser = ComposerParser::Stub;
    let budget = clear_passes(lines);
    let deadline = Instant::now() + Duration::from_millis(40).saturating_mul(budget);
    let mut sent = 0u32;
    loop {
        if sent > 0 {
            let text = app.mux.capture(pane, range).await?.text;
            if composer_empty(parser, &text) {
                return Ok(true);
            }
        }
        if sent >= budget || (sent > 0 && Instant::now() >= deadline) {
            return reset_composer(app, pane, range, parser).await;
        }
        app.mux.send_keys(pane, parser.clear_keys()).await?;
        sent = sent.saturating_add(1);
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn reset_composer(
    app: &App,
    pane: &PaneRef,
    range: CaptureRange,
    parser: ComposerParser,
) -> Result<bool, Error> {
    app.mux.send_keys(pane, parser.reset_keys()).await?;
    tokio::time::sleep(Duration::from_millis(25)).await;
    let text = app.mux.capture(pane, range).await?.text;
    Ok(composer_empty(parser, &text))
}

/// `C-u` passes for a paste of `lines` lines. Short pastes stay at 8.
fn clear_passes(lines: usize) -> u32 {
    u32::try_from(lines.saturating_add(1))
        .unwrap_or(80)
        .clamp(8, 80)
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
    flatten_at(text, None)
}

fn flatten_at(text: &str, width: Option<usize>) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "");
    let raw: Vec<&str> = normalized.split('\n').collect();
    flatten_rows(&raw, width.or_else(|| grid_width(&raw)))
}

fn flatten_rows(raw: &[&str], width: Option<usize>) -> String {
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
fn pane_width(screen: &str) -> Option<usize> {
    let normalized = screen.replace("\r\n", "\n").replace('\r', "");
    let raw: Vec<&str> = normalized.split('\n').collect();
    grid_width(&raw)
}

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
    use crewmux_mux::CaptureRange;

    use super::{PasteHit, classify_paste, clear_passes, history_range};

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
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        // A padded `stub>` would grow a trailing space and look like a new prompt,
        // hiding the payload above it. The redraw the stub actually writes has none.
        let mut after = grid(&refs, 24);
        after.push_str("stub>\n");
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

    #[test]
    fn earlier_output_above_the_prompt_is_not_contamination() {
        let before = "stub-agent\nstub> \nold payload that scrolled\nstub> \n";
        let body = payload();
        let mut after = before.to_owned();
        after.push_str(body);
        assert_eq!(classify_paste(before, &after, body), PasteHit::Clean);
        let mut dirty = after.clone();
        dirty.push_str("\nEXTRA\n");
        assert_eq!(classify_paste(before, &dirty, body), PasteHit::Contaminated);
    }

    #[test]
    fn payload_containing_the_prompt_is_anchored_to_the_verdict_prompt() {
        let before = "stub-agent\nstub> \n";
        let body = "see stub> inside";
        let mut after = String::new();
        after.push_str(before);
        after.push_str(body);
        assert_eq!(classify_paste(before, &after, body), PasteHit::Clean);
    }

    #[test]
    fn a_repeated_reply_line_stays_above_the_verdict_prompt() {
        let reply = "↩ Reply: rig send operator@host \"...\"";
        let first = format!("{}\n{reply}\n{reply}\nstub> \n", "A".repeat(20));
        let second = format!("{}\n{reply}\n{reply}\nstub> \n", "B".repeat(20));
        let before = format!("stub-agent\nstub> \n{first}{second}");
        let body = format!("From: operator@host\n{}", "C".repeat(20));
        let after = format!("{before}{body}");
        assert_eq!(classify_paste(&before, &after, &body), PasteHit::Clean);
    }

    #[test]
    fn history_range_clamps_at_500_rows() {
        let wide = "W".repeat(20_000);
        assert_eq!(history_range(&wide), CaptureRange::JoinedHistory(500));
        let modest = "W".repeat(100);
        assert!(matches!(
            history_range(&modest),
            CaptureRange::JoinedHistory(rows) if (48..500).contains(&rows)
        ));
    }

    #[test]
    fn clear_budget_tracks_the_pasted_line_count() {
        assert_eq!(clear_passes(1), 8);
        assert_eq!(clear_passes(62), 63);
        assert_eq!(clear_passes(500), 80);
    }
}
