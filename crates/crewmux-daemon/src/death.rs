//! Mark seats dead when the multiplexer reports a pane or window death.
//!
//! The clear path sends Ctrl-U only. Claude does not get Escape, Ctrl-C,
//! Ctrl-D, or Ctrl-G, and a compact timeout does not kill the pane. `seat_died`
//! still records the exit status and the last capture so the next unexplained
//! exit can be read from the event.

use crewmux_core::{LifecycleService, Seat, SeatRepo, SeatState};
use crewmux_mux::{CaptureRange, MuxEvent};

use crate::db::{read, write};
use crate::error::Error;
use crate::launch::now_ts;
use crate::state::App;

pub(crate) async fn supervise(app: App) {
    let mut deaths = app.mux.death_events();
    loop {
        tokio::select! {
            () = app.cancel.cancelled() => break,
            event = deaths.recv() => {
                let Some(event) = event else { break };
                if let Err(err) = apply(&app, event).await {
                    tracing::warn!(error = %err, "seat death");
                }
            }
        }
    }
}

async fn apply(app: &App, event: MuxEvent) -> Result<(), Error> {
    match event {
        MuxEvent::PaneDied {
            pane_id,
            status,
            signal,
        } => {
            if let Some(seat) = read(&app.store, move |conn| conn.get_by_pane(&pane_id)).await? {
                let capture = last_capture(app, &seat).await;
                mark_dead(app, &seat, status, signal, capture).await?;
            }
        }
        MuxEvent::WindowClosed { session, .. } => {
            if !session.is_empty() {
                let seats = read(&app.store, |conn| conn.list_all()).await?;
                for seat in seats {
                    if seat.tmux_session.as_deref() == Some(session.as_str()) {
                        let capture = last_capture(app, &seat).await;
                        mark_dead(app, &seat, None, None, capture).await?;
                    }
                }
            }
        }
        MuxEvent::Output { .. }
        | MuxEvent::SessionsChanged
        | MuxEvent::ServerExited
        | MuxEvent::Reconnected
        | MuxEvent::Gap { .. } => {}
    }
    Ok(())
}

pub(crate) async fn mark_dead(
    app: &App,
    seat: &Seat,
    exit_status: Option<i32>,
    signal: Option<String>,
    capture: Option<String>,
) -> Result<(), Error> {
    let id = seat.id;
    let logged = capture.clone();
    tracing::info!(
        seat = %id,
        exit_status = ?exit_status,
        signal = signal.as_deref().unwrap_or(""),
        capture = logged.as_deref().unwrap_or(""),
        "seat_died"
    );
    write(&app.store, move |conn| {
        let current = match SeatRepo::get(conn, &id) {
            Ok(seat) => seat,
            Err(crewmux_core::Error::SeatNotFound(_)) => return Ok(()),
            Err(err) => return Err(err),
        };
        if matches!(current.state, SeatState::Dead | SeatState::Stopped) {
            return Ok(());
        }
        let mut updated = current;
        let event =
            LifecycleService::mark_dead(&mut updated, now_ts(), exit_status, signal, capture);
        SeatRepo::upsert(conn, &updated)?;
        crewmux_core::EventLog::append(conn, &event)?;
        Ok(())
    })
    .await?;
    Ok(())
}

pub(crate) async fn last_capture(app: &App, seat: &Seat) -> Option<String> {
    let pane = crate::query::pane_of(seat).ok()?;
    let shot = app.mux.capture(&pane, CaptureRange::Visible).await.ok()?;
    Some(redact_capture(&shot.text))
}

/// Cap the stored pane and replace obvious credentials.
///
/// The tail is what was on screen when the process died, which can include a
/// pasted key. Values are replaced and the row is marked `[redacted]`. This is
/// not a general secret scanner: it catches common token shapes and assignments.
fn redact_capture(text: &str) -> String {
    // Redact the whole pane first. Capping first can keep the tail of a token
    // whose prefix fell outside the window.
    let (redacted, changed) = redact_tokens(text);
    let capped = bound_capture(redacted);
    if !changed {
        return capped;
    }
    let mut out = capped;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("[redacted]");
    bound_capture(out)
}

fn redact_tokens(text: &str) -> (String, bool) {
    let (prefixed, prefix_hit) = redact_prefixes(text);
    let (assigned, assign_hit) = redact_assignments(&prefixed);
    (assigned, prefix_hit || assign_hit)
}

const KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "token",
    "secret",
    "password",
    "authorization",
];

const PREFIXES: &[&str] = &[
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxs-",
    "ghp_",
    "sk-",
    "AKIA",
    "Bearer ",
];

fn redact_prefixes(text: &str) -> (String, bool) {
    let mut changed = false;
    let mut rest = text;
    let mut out = String::with_capacity(text.len());
    while !rest.is_empty() {
        let Some((at, prefix)) = earliest_prefix(rest) else {
            out.push_str(rest);
            break;
        };
        out.push_str(rest.get(..at).unwrap_or(""));
        let after = rest.get(at.saturating_add(prefix.len())..).unwrap_or("");
        let token_len = after
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
            .map(char::len_utf8)
            .sum::<usize>();
        if token_len >= 8 {
            out.push_str("[redacted]");
            changed = true;
            rest = after.get(token_len..).unwrap_or("");
        } else {
            out.push_str(prefix);
            rest = after;
        }
    }
    (out, changed)
}

fn earliest_prefix(text: &str) -> Option<(usize, &'static str)> {
    let mut best: Option<(usize, &'static str)> = None;
    for prefix in PREFIXES {
        let mut from = 0usize;
        while let Some(rel) = text.get(from..).and_then(|rest| rest.find(prefix)) {
            let at = from.saturating_add(rel);
            let next = at.saturating_add(prefix.len());
            if at_boundary(text, at) {
                best = match best {
                    Some((earlier, _)) if earlier <= at => best,
                    _ => Some((at, *prefix)),
                };
                break;
            }
            if next <= from {
                break;
            }
            from = next;
        }
    }
    best
}

fn at_boundary(text: &str, at: usize) -> bool {
    let Some(before) = text.get(..at).and_then(|head| head.chars().next_back()) else {
        return true;
    };
    !before.is_ascii_alphanumeric() && before != '_'
}

struct Assignment {
    key_at: usize,
    value_at: usize,
    value_end: usize,
}

fn redact_assignments(text: &str) -> (String, bool) {
    let mut changed = false;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut base = 0usize;
    while !rest.is_empty() {
        let Some(hit) = next_assignment(text, base) else {
            out.push_str(rest);
            break;
        };
        let local = hit.key_at.saturating_sub(base);
        out.push_str(rest.get(..local).unwrap_or(""));
        out.push_str(text.get(hit.key_at..hit.value_at).unwrap_or(""));
        out.push_str("[redacted]");
        changed = true;
        base = hit.value_end;
        rest = text.get(base..).unwrap_or("");
    }
    (out, changed)
}

/// Earliest `key` that is an assignment, including a later one after a bare mention.
fn next_assignment(text: &str, from: usize) -> Option<Assignment> {
    let lower = text.get(from..)?.to_ascii_lowercase();
    let mut best: Option<Assignment> = None;
    for key in KEYS {
        let mut cursor = 0usize;
        while let Some(rel) = lower.get(cursor..).and_then(|rest| rest.find(*key)) {
            let at = from.saturating_add(cursor).saturating_add(rel);
            let next = at.saturating_add(key.len());
            if at_boundary(text, at)
                && let Some(hit) = assignment_after(text, at, key.len())
            {
                if best.as_ref().is_none_or(|cur| hit.key_at < cur.key_at) {
                    best = Some(hit);
                }
                break;
            }
            if next <= from.saturating_add(cursor) {
                break;
            }
            cursor = next.saturating_sub(from);
        }
    }
    best
}

fn assignment_after(text: &str, at: usize, key_len: usize) -> Option<Assignment> {
    let after = text.get(at.saturating_add(key_len)..)?;
    let mut idx = 0usize;
    if after
        .as_bytes()
        .first()
        .is_some_and(|byte| *byte == b'"' || *byte == b'\'')
    {
        idx = idx.saturating_add(1);
    }
    while after
        .as_bytes()
        .get(idx)
        .is_some_and(u8::is_ascii_whitespace)
    {
        idx = idx.saturating_add(1);
    }
    let sep = *after.as_bytes().get(idx)?;
    if sep != b'=' && sep != b':' {
        return None;
    }
    idx = idx.saturating_add(1);
    while after
        .as_bytes()
        .get(idx)
        .is_some_and(u8::is_ascii_whitespace)
    {
        idx = idx.saturating_add(1);
    }
    let value_at = at.saturating_add(key_len).saturating_add(idx);
    let value = text.get(value_at..)?;
    let value_len = quoted_or_bare(value)?;
    Some(Assignment {
        key_at: at,
        value_at,
        value_end: value_at.saturating_add(value_len),
    })
}

fn quoted_or_bare(value: &str) -> Option<usize> {
    let quote = value.as_bytes().first().copied()?;
    if quote == b'"' || quote == b'\'' {
        let rest = value.get(1..)?;
        let end = rest.find(char::from(quote))?;
        return Some(1usize.saturating_add(end).saturating_add(1));
    }
    let len = value
        .chars()
        .take_while(|ch| !ch.is_whitespace())
        .map(char::len_utf8)
        .sum::<usize>();
    (len > 0).then_some(len)
}

fn bound_capture(text: String) -> String {
    const LIMIT: usize = 8_000;
    if text.len() <= LIMIT {
        return text;
    }
    let mut start = text.len().saturating_sub(LIMIT);
    while start < text.len() && !text.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    text.get(start..).unwrap_or("").to_owned()
}

#[cfg(test)]
mod tests {
    use super::redact_capture;

    #[test]
    fn seat_died_capture_drops_obvious_tokens() {
        let raw = "\
hello
token=supersecretvalue
sk-abcDEF1234567890xyz
Bearer ghp_abcdefghijklmnop
still here
";
        let redacted = redact_capture(raw);
        assert!(!redacted.contains("supersecretvalue"), "{redacted}");
        assert!(!redacted.contains("sk-abcDEF"), "{redacted}");
        assert!(!redacted.contains("ghp_abcdefghijklmnop"), "{redacted}");
        assert!(redacted.contains("hello"), "{redacted}");
        assert!(redacted.contains("still here"), "{redacted}");
        assert!(redacted.contains("[redacted]"), "{redacted}");
    }

    #[test]
    fn redaction_covers_later_keys_json_and_a_split_token() {
        let later = redact_capture("see the token bucket\ntoken=supersecretvalue\n");
        assert!(!later.contains("supersecretvalue"), "{later}");
        let json = redact_capture("{\"token\": \"supersecretvalue\"}\n");
        assert!(!json.contains("supersecretvalue"), "{json}");
        let task = redact_capture("task-abcdefghijklmnop\n");
        assert!(task.contains("task-abcdefghijklmnop"), "{task}");
        assert!(!task.contains("[redacted]"), "{task}");
        // The secret sits on a token boundary and straddles the 8000-byte window.
        // Capping first would keep `4567890xyz`; redacting first removes it.
        let secret = "sk-abcDEF1234567890xyz";
        let raw = format!("{}\n{secret}{}", "z".repeat(90), "y".repeat(7_990));
        let cut = redact_capture(&raw);
        assert!(!cut.contains("abcDEF1234567890"), "{cut}");
        assert!(!cut.contains("4567890xyz"), "{cut}");
        assert!(cut.contains("[redacted]"), "{cut}");
    }
}
