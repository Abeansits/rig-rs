//! Apply a hook, write the marker it names, and submit a due restore.

use std::path::{Path, PathBuf};

use crewmux_adapters::Readiness;
use crewmux_health::compaction::{
    Attempt, AttemptState, HookEvent, RestorePrompt, build_post_compact_restore_prompt,
    marker_bytes, marker_path, parse_hook_value,
};
use crewmux_provider::AtomicWrite;
use serde_json::{Value, json};

use crate::error::Error;
use crate::query::find_seat_id;
use crate::state::{App, join_err, now_ms};
use crate::submit::{binding_for, submit_text};

use super::persist::save;

pub(crate) async fn ingest(app: &App, seat_key: &str, body: &[u8]) -> Result<Value, Error> {
    let (run_id, payload) = decode(body)?;
    let event = parse_hook_value(&payload);
    if event_name(&event).is_empty() || matches!(event, HookEvent::Unknown { .. }) {
        tracing::info!(seat_key, "ignored compaction hook");
        return Ok(json!({"status": "ignored"}));
    }
    let session = session_name(app, seat_key).await;
    let marker = apply_event(app, seat_key, &event, session.as_deref()).await;
    if let Some(marker) = marker {
        write_marker(&app.home, &marker).await?;
    }
    save(app).await?;
    if let (HookEvent::SessionEnd { reason, .. }, Some(run_id)) = (&event, run_id.as_deref())
        && reason != "clear"
    {
        gc_run(&app.home, seat_key, run_id).await?;
    }
    deliver_seat(app, seat_key).await?;
    Ok(json!({"status": "ok"}))
}

pub(crate) async fn deliver_due(app: &App) -> Result<(), Error> {
    let keys = {
        let guard = app.compaction.lock().await;
        guard
            .book
            .attempts()
            .iter()
            .filter(|attempt| matches!(attempt.state, AttemptState::Completed { .. }))
            .map(|attempt| attempt.seat_key.clone())
            .collect::<Vec<_>>()
    };
    let mut seen = std::collections::BTreeSet::new();
    for key in keys {
        if seen.insert(key.clone()) {
            deliver_seat(app, &key).await?;
        }
    }
    Ok(())
}

async fn apply_event(
    app: &App,
    seat_key: &str,
    event: &HookEvent,
    session: Option<&str>,
) -> Option<crewmux_health::compaction::RestoreMarker> {
    let mut guard = app.compaction.lock().await;
    let effect = guard.book.on_hook(seat_key, event, now_ms());
    guard.book.tick(now_ms());
    if let Some(session) = session
        && guard
            .book
            .for_seat(seat_key)
            .iter()
            .any(|attempt| matches!(attempt.state, AttemptState::Completed { .. }))
    {
        guard.enforcer.note_hooks_completed(session);
    }
    for note in &effect.log {
        tracing::info!(seat_key, note, "compaction");
    }
    effect.marker
}

pub(super) async fn deliver_seat(app: &App, seat_key: &str) -> Result<(), Error> {
    let Ok(seat) = find_seat_id(app, seat_key).await else {
        return Ok(());
    };
    if !seat_is_ready(app, &seat).await {
        return Ok(());
    }
    let Some(attempt) = due_attempt(app, seat_key).await else {
        return Ok(());
    };
    let session = seat.name.as_str().to_owned();
    let text = restore_text(&app.home, &attempt, &session);
    match submit_text(app, &session, &text).await {
        Ok(_) => mark_restored(app, &attempt, &session).await,
        Err(Error::Adapter(err)) if after_enter(&err) => {
            fail_restore(app, &attempt, &err.to_string()).await
        }
        Err(Error::Adapter(
            crewmux_adapters::Error::NotReady | crewmux_adapters::Error::PasteMismatch,
        )) => Ok(()),
        Err(err) => {
            tracing::warn!(seat_key, %err, "restore submit failed");
            Ok(())
        }
    }
}

fn after_enter(err: &crewmux_adapters::Error) -> bool {
    matches!(
        err,
        crewmux_adapters::Error::NotVerified | crewmux_adapters::Error::Contaminated
    )
}

async fn due_attempt(app: &App, seat_key: &str) -> Option<Attempt> {
    let guard = app.compaction.lock().await;
    guard.book.due_restore(seat_key, true, now_ms()).cloned()
}

async fn fail_restore(app: &App, attempt: &Attempt, reason: &str) -> Result<(), Error> {
    {
        let mut guard = app.compaction.lock().await;
        guard.book.mark_failed(&attempt.id, reason);
    }
    save(app).await
}

async fn mark_restored(app: &App, attempt: &Attempt, session: &str) -> Result<(), Error> {
    {
        let mut guard = app.compaction.lock().await;
        guard.book.mark_restore_sent(&attempt.id);
        guard.book.mark_done(&attempt.id);
        guard.enforcer.note_restore_finished(session, now_ms());
    }
    save(app).await
}

pub(super) async fn seat_is_ready(app: &App, seat: &crewmux_core::Seat) -> bool {
    let Ok(binding) = binding_for(seat) else {
        return false;
    };
    let Some(adapter) = app.registry.get(seat.runtime) else {
        return false;
    };
    matches!(adapter.check_ready(&binding).await, Ok(Readiness::Ready))
}

fn restore_text(home: &Path, attempt: &Attempt, session: &str) -> String {
    let path = marker_path(home, &attempt.seat_key, attempt.id.as_str());
    let transcript = nonempty(&attempt.transcript_path);
    build_post_compact_restore_prompt(&RestorePrompt {
        session_name: session,
        rig_home: home,
        marker_path: Some(&path),
        transcript_path: transcript,
        session_id: attempt.session_id.as_deref(),
        post_compact_instruction: None,
        post_compact_instruction_file: None,
        ignored_wrong_seat_extra: false,
    })
}

fn nonempty(path: &Path) -> Option<&Path> {
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

fn decode(body: &[u8]) -> Result<(Option<String>, Value), Error> {
    let value: Value =
        serde_json::from_slice(body).map_err(|err| Error::Message(err.to_string()))?;
    if let Some(payload) = value.get("payload") {
        let run_id = value
            .get("run_id")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        return Ok((run_id, payload.clone()));
    }
    Ok((None, value))
}

fn event_name(event: &HookEvent) -> &str {
    match event {
        HookEvent::Unknown { name, .. } => name,
        HookEvent::SessionStart { .. } => "SessionStart",
        HookEvent::UserPromptSubmit { .. } => "UserPromptSubmit",
        HookEvent::PreCompact { .. } => "PreCompact",
        HookEvent::PostCompact { .. } => "PostCompact",
        HookEvent::Stop { .. } => "Stop",
        HookEvent::SessionEnd { .. } => "SessionEnd",
    }
}

async fn session_name(app: &App, seat_key: &str) -> Option<String> {
    find_seat_id(app, seat_key)
        .await
        .ok()
        .map(|seat| seat.name.as_str().to_owned())
}

async fn write_marker(
    home: &Path,
    marker: &crewmux_health::compaction::RestoreMarker,
) -> Result<(), Error> {
    let path = marker_path(home, &marker.seat_key, &marker.attempt_id);
    let bytes = marker_bytes(marker).map_err(|err| Error::Message(err.to_string()))?;
    tokio::task::spawn_blocking(move || write_bytes(&path, &bytes))
        .await
        .map_err(join_err)?
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    AtomicWrite::write(path, bytes, Some(0o600)).map_err(|err| Error::Message(err.to_string()))
}

async fn gc_run(home: &Path, seat_key: &str, run_id: &str) -> Result<(), Error> {
    if !safe_component(seat_key) || !safe_component(run_id) {
        tracing::warn!(seat_key, run_id, "refusing to gc compaction run");
        return Ok(());
    }
    let path = home.join("seats").join(seat_key).join("run").join(run_id);
    tokio::task::spawn_blocking(move || remove_run(&path))
        .await
        .map_err(join_err)?
}

fn remove_run(path: &PathBuf) -> Result<(), Error> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains('\0')
}
