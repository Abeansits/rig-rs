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
    Some(redact_capture(shot.text))
}

/// Cap the stored pane and replace obvious credentials.
///
/// The tail is what was on screen when the process died, which can include a
/// pasted key. Values are replaced and the row is marked `[redacted]`. This is
/// not a general secret scanner: it catches common token shapes and assignments.
fn redact_capture(text: String) -> String {
    let capped = bound_capture(text);
    let (redacted, changed) = redact_tokens(&capped);
    if !changed {
        return redacted;
    }
    let mut out = redacted;
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
    PREFIXES.iter().fold(None, |best, prefix| {
        let Some(at) = text.find(prefix) else {
            return best;
        };
        match best {
            Some((earlier, _)) if earlier <= at => best,
            _ => Some((at, *prefix)),
        }
    })
}

fn redact_assignments(text: &str) -> (String, bool) {
    let mut changed = false;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let Some(at) = assignment_at(rest) else {
            out.push_str(rest);
            break;
        };
        out.push_str(rest.get(..at).unwrap_or(""));
        let tail = rest.get(at..).unwrap_or("");
        let eq = tail.find(['=', ':']).unwrap_or(0);
        let mut split = eq.saturating_add(1);
        if tail.as_bytes().get(split) == Some(&b' ') {
            split = split.saturating_add(1);
        }
        out.push_str(tail.get(..split).unwrap_or(""));
        let value = tail.get(split..).unwrap_or("");
        let len = value
            .chars()
            .take_while(|ch| !ch.is_whitespace())
            .map(char::len_utf8)
            .sum::<usize>();
        if len > 0 {
            out.push_str("[redacted]");
            changed = true;
        }
        rest = value.get(len..).unwrap_or("");
    }
    (out, changed)
}

fn assignment_at(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let mut found = None;
    for key in KEYS {
        let Some(at) = lower.find(key) else {
            continue;
        };
        let after = text.get(at.saturating_add(key.len())..).unwrap_or("");
        let trimmed = after.trim_start();
        if !(trimmed.starts_with('=') || trimmed.starts_with(':')) {
            continue;
        }
        if found.is_none_or(|earlier| at < earlier) {
            found = Some(at);
        }
    }
    found
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
        let redacted = redact_capture(raw.to_owned());
        assert!(!redacted.contains("supersecretvalue"), "{redacted}");
        assert!(!redacted.contains("sk-abcDEF"), "{redacted}");
        assert!(!redacted.contains("ghp_abcdefghijklmnop"), "{redacted}");
        assert!(redacted.contains("hello"), "{redacted}");
        assert!(redacted.contains("still here"), "{redacted}");
        assert!(redacted.contains("[redacted]"), "{redacted}");
    }
}
