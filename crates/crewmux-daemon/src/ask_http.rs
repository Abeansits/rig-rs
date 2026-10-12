//! `rig ask`. L1 reads seat transcripts. L2 reads one session JSONL. They are not mixed.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use crewmux_messaging::{AskCorpus, AskQuery, PasteDisplay, search};
use crewmux_types::SeatId;
use serde::Deserialize;
use serde_json::json;

use crate::error::Error;
use crate::http::{fail, ok};
use crate::state::App;

pub(crate) async fn ask(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed: AskBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let home = app.home.clone();
    let enabled = app.transcripts;
    let seat_name = parsed.seat.clone();
    let rig_name = parsed.rig.clone();
    let token = parsed.session.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        load(
            &home,
            rig_name.as_deref(),
            seat_name.as_deref(),
            token.as_deref(),
        )
    })
    .await;
    let corpus = match loaded {
        Ok(corpus) => corpus,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let display = match parsed.seat.as_deref() {
        Some(name) => match crate::query::find_seat(&app, name).await {
            Ok(seat) => PasteDisplay::for_runtime(seat.runtime),
            Err(_) => PasteDisplay::Exact,
        },
        None => PasteDisplay::Exact,
    };
    let query = AskQuery {
        rig: parsed.rig.unwrap_or_default(),
        seat: parsed
            .seat
            .as_deref()
            .and_then(|name| name.parse::<SeatId>().ok()),
        seat_name: parsed.seat,
        session_token: parsed.session,
        text: parsed.text,
        transcripts_enabled: enabled,
        display,
    };
    let result = search(&query, &corpus);
    match serde_json::to_value(&result) {
        Ok(value) => ok(value),
        Err(err) => fail(Error::Message(err.to_string())),
    }
}

fn load(
    home: &std::path::Path,
    rig: Option<&str>,
    seat: Option<&str>,
    token: Option<&str>,
) -> AskCorpus {
    if token.is_some() {
        return AskCorpus {
            session_jsonl: token.and_then(|token| read_session(home, token)),
            ..AskCorpus::default()
        };
    }
    AskCorpus {
        seat_generations: seat
            .map(|seat| read_seat(home, rig, seat))
            .unwrap_or_default(),
        ..AskCorpus::default()
    }
}

fn read_seat(home: &std::path::Path, rig: Option<&str>, seat: &str) -> Vec<(u64, String)> {
    let mut rows = Vec::new();
    for dir in seat_dirs(home, rig, seat) {
        rows.extend(read_generations(&dir));
    }
    rows.sort_by_key(|(generation, _)| *generation);
    rows
}

fn seat_dirs(home: &std::path::Path, rig: Option<&str>, seat: &str) -> Vec<std::path::PathBuf> {
    let root = home.join("transcripts");
    if let Some(rig) = rig.filter(|rig| !rig.is_empty()) {
        return vec![root.join(rig).join(seat)];
    }
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path().join(seat);
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    dirs
}

fn read_generations(dir: &std::path::Path) -> Vec<(u64, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(text) = name.to_str() else {
            continue;
        };
        let Some(stem) = text.strip_suffix(".txt") else {
            continue;
        };
        let Ok(generation) = stem.parse::<u64>() else {
            continue;
        };
        if let Ok(body) = std::fs::read_to_string(entry.path()) {
            rows.push((generation, body));
        }
    }
    rows
}

fn read_session(home: &std::path::Path, token: &str) -> Option<String> {
    let path = home.join("sessions").join(format!("{token}.jsonl"));
    std::fs::read_to_string(path).ok()
}

pub(crate) async fn whoami(State(app): State<App>) -> Response {
    app.touch();
    let session = std::env::var("RIG_SESSION").unwrap_or_else(|_| "operator@host".to_owned());
    ok(json!({"session": session}))
}

#[derive(Deserialize)]
struct AskBody {
    #[serde(default)]
    rig: Option<String>,
    #[serde(default)]
    seat: Option<String>,
    #[serde(default)]
    session: Option<String>,
    text: String,
}
