// Derived from openrig packages/daemon/src/domain/ask-service.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! L1 seat transcripts, L2 session JSONL, and chat hits.
//!
//! The two transcript scopes are never mixed with a whole-rig search.

use crewmux_types::SeatId;
use serde::{Deserialize, Serialize};

/// What `rig ask` searched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskQuery {
    /// Rig name.
    pub rig: String,
    /// L1 seat. Absent for an unscoped or L2 query.
    pub seat: Option<SeatId>,
    /// Seat name used in transcript paths. Preferred over the id when set.
    pub seat_name: Option<String>,
    /// L2 provider session token.
    pub session_token: Option<String>,
    /// Search text.
    pub text: String,
    /// When false, L1 returns guidance instead of an empty success.
    pub transcripts_enabled: bool,
}

/// One hit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// Where it was found.
    pub source: String,
    /// Matching line.
    pub text: String,
}

/// A chat-history hit. Empty until slice 5's repository is wired.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatHit {
    /// Matching line.
    pub text: String,
}

/// Ask result. `guidance` is set when the search could not honestly succeed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskResult {
    /// L1 hits. `None` when the query was not seat-scoped.
    pub l1: Option<Evidence>,
    /// L2 hits. `None` when the query was not session-scoped.
    pub l2: Option<Evidence>,
    /// Chat hits. Empty when slice 5 is absent.
    pub chat: Vec<ChatHit>,
    /// Honest notes. Never a silent empty success.
    pub guidance: Vec<String>,
}

/// Files the daemon already loaded. The caller passes only the scope it intends.
#[derive(Clone, Debug, Default)]
pub struct AskCorpus {
    /// `(generation, stripped text)` for one seat. Not a whole rig.
    pub seat_generations: Vec<(u64, String)>,
    /// One session JSONL, read-only. `None` when the file is missing.
    pub session_jsonl: Option<String>,
    /// Chat lines from slice 5. Empty by default.
    pub chat_lines: Vec<String>,
}

/// Search `corpus` for `query`. Seat files and the session file are not combined.
#[must_use]
pub fn search(query: &AskQuery, corpus: &AskCorpus) -> AskResult {
    if query.session_token.is_some() {
        return search_session(query, corpus);
    }
    if query.seat.is_some() || query.seat_name.is_some() {
        return search_seat(query, corpus);
    }
    AskResult {
        l1: None,
        l2: None,
        chat: chat_hits(corpus, &query.text),
        guidance: vec![
            "Name a seat with --seat or a provider session with --session. A whole-rig grep is not used.".to_owned(),
        ],
    }
}

fn search_seat(query: &AskQuery, corpus: &AskCorpus) -> AskResult {
    let name = query
        .seat_name
        .clone()
        .or_else(|| query.seat.as_ref().map(ToString::to_string))
        .unwrap_or_else(|| "seat".to_owned());
    if !query.transcripts_enabled {
        return AskResult {
            l1: None,
            l2: None,
            chat: Vec::new(),
            guidance: vec![
                "Transcripts are disabled. Enable with: rig config set transcripts.enabled true"
                    .to_owned(),
            ],
        };
    }
    if corpus.seat_generations.is_empty() {
        return AskResult {
            l1: None,
            l2: None,
            chat: Vec::new(),
            guidance: vec![format!(
                "No transcript files for seat '{name}'. The daemon writes them from pane output when transcripts are enabled."
            )],
        };
    }
    let hit = corpus
        .seat_generations
        .iter()
        .find_map(|(generation, text)| {
            line_hit(text, &query.text).map(|line| Evidence {
                source: format!("seat {name} gen {generation}"),
                text: line,
            })
        });
    let guidance = if hit.is_none() {
        vec![format!(
            "No matching evidence in seat '{name}' across {} generation(s). Try different search terms.",
            corpus.seat_generations.len()
        )]
    } else {
        Vec::new()
    };
    AskResult {
        l1: hit,
        l2: None,
        chat: Vec::new(),
        guidance,
    }
}

fn search_session(query: &AskQuery, corpus: &AskCorpus) -> AskResult {
    let token = query.session_token.clone().unwrap_or_default();
    let Some(text) = corpus.session_jsonl.as_deref() else {
        return AskResult {
            l1: None,
            l2: None,
            chat: Vec::new(),
            guidance: vec![format!(
                "No provider session JSONL for '{token}'. Session search is read-only and does not fall back to seat transcripts."
            )],
        };
    };
    let hit = line_hit(text, &query.text).map(|line| Evidence {
        source: format!("session {token}"),
        text: line,
    });
    let guidance = if hit.is_none() {
        vec![format!(
            "No matching content in session '{token}'. Try different search terms."
        )]
    } else {
        Vec::new()
    };
    AskResult {
        l1: None,
        l2: hit,
        chat: Vec::new(),
        guidance,
    }
}

fn chat_hits(corpus: &AskCorpus, needle: &str) -> Vec<ChatHit> {
    corpus
        .chat_lines
        .iter()
        .filter(|line| line.contains(needle))
        .map(|line| ChatHit { text: line.clone() })
        .collect()
}

fn line_hit(text: &str, needle: &str) -> Option<String> {
    if needle.is_empty() {
        return None;
    }
    text.lines()
        .find(|line| line.contains(needle))
        .map(ToOwned::to_owned)
}
