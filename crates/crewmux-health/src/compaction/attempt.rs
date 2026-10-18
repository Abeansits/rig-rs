//! Per-seat compaction attempts. Completion requires both hook signals.
//!
//! Elapsed time aborts an unfinished attempt. It never confirms a marker and
//! never produces a restore.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::COMPLETION_WAIT;
use super::POST_COMPACT_SEND_WAIT;
use super::hash::{hex, sha256, sha256_hex};
use super::hook::{CompactTrigger, HookEvent, SessionSource};
use super::marker::RestoreMarker;

/// Attempt identifier. The marker file is named with the sanitised form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AttemptId(pub String);

impl AttemptId {
    /// Borrow the id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why an attempt stopped without a restore.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbortReason {
    /// `PreCompact` was not followed by both completion signals before the wait.
    NoPostCompact,
}

/// Lifecycle of one attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", content = "data", rename_all = "snake_case")]
pub enum AttemptState {
    /// The enforcer decided and `/compact` was submitted.
    Requested,
    /// `PreCompact` arrived. The marker is unconfirmed.
    PreCompactSeen {
        /// Manual or automatic.
        trigger: CompactTrigger,
        /// sha256 hex of the custom instructions, or of an empty string.
        instructions_hash: String,
    },
    /// Both completion signals arrived for this session.
    Completed {
        /// sha256 of `compact_summary`.
        summary_sha: [u8; 32],
    },
    /// The restore prompt was submitted once.
    RestoreSent,
    /// The restore turn was verified.
    Done,
    /// Timed out or abandoned without a restore.
    Aborted(AbortReason),
    /// `rig compact <seat> --cancel`.
    Cancelled,
    /// A visible failure.
    Failed(String),
}

impl AttemptState {
    /// Status token for `rig compact --status` and `queue list`.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::PreCompactSeen { .. } => "pre_compact_seen",
            Self::Completed { .. } => "completed",
            Self::RestoreSent => "restore_sent",
            Self::Done => "done",
            Self::Aborted(_) => "aborted",
            Self::Cancelled => "cancelled",
            Self::Failed(_) => "failed",
        }
    }
}

/// One persisted attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    /// Id.
    pub id: AttemptId,
    /// Seat key.
    pub seat_key: String,
    /// Trigger captured at `PreCompact`, or the requested trigger.
    pub trigger: CompactTrigger,
    /// True when the enforcer requested this `/compact`.
    pub managed: bool,
    /// Lifecycle.
    pub state: AttemptState,
    /// Claude session id, once `PreCompact` names it.
    pub session_id: Option<String>,
    /// `SessionStart { source: compact }` seen for `session_id`.
    pub saw_compact_start: bool,
    /// `PostCompact` seen for `session_id`.
    pub saw_post_compact: bool,
    /// sha256 of the summary, once `PostCompact` arrives.
    pub summary_sha: Option<[u8; 32]>,
    /// Byte length of `compact_summary`.
    pub summary_len: Option<u64>,
    /// When the attempt was opened, Unix milliseconds.
    pub opened_ms: u64,
    /// When `PreCompact` arrived.
    pub precompact_ms: Option<u64>,
    /// When both signals had arrived.
    pub completed_ms: Option<u64>,
    /// Transcript from `PreCompact`.
    pub transcript_path: PathBuf,
    /// Working directory from the hook payload, when the caller supplies it.
    pub cwd: PathBuf,
}

/// What one hook changed. Marker bytes are written by the caller.
#[derive(Clone, Debug, Default)]
pub struct ApplyEffect {
    /// Marker to write. Absent when this event must not touch disk.
    pub marker: Option<RestoreMarker>,
    /// A completion signal named a different session than the open attempt.
    pub ignored_session_mismatch: bool,
    /// Human-readable notes for the daemon log.
    pub log: Vec<String>,
}

/// Every attempt the daemon has loaded.
#[derive(Clone, Debug, Default)]
pub struct Book {
    attempts: Vec<Attempt>,
    seq: u64,
    completion_wait_ms: u64,
    send_wait_ms: u64,
}

impl Book {
    /// Waits from the acceptance spec.
    #[must_use]
    pub fn new() -> Self {
        Self {
            attempts: Vec::new(),
            seq: 1,
            completion_wait_ms: duration_ms(COMPLETION_WAIT),
            send_wait_ms: duration_ms(POST_COMPACT_SEND_WAIT),
        }
    }

    /// Open a managed attempt in `Requested`.
    pub fn request(&mut self, seat_key: &str, id: AttemptId, now_ms: u64) {
        self.attempts.push(Attempt {
            id,
            seat_key: seat_key.to_owned(),
            trigger: CompactTrigger::Manual,
            managed: true,
            state: AttemptState::Requested,
            session_id: None,
            saw_compact_start: false,
            saw_post_compact: false,
            summary_sha: None,
            summary_len: None,
            opened_ms: now_ms,
            precompact_ms: None,
            completed_ms: None,
            transcript_path: PathBuf::new(),
            cwd: PathBuf::new(),
        });
    }

    /// Apply one hook. An unsolicited `PreCompact` never rewrites another attempt's marker.
    pub fn on_hook(&mut self, seat_key: &str, event: &HookEvent, now_ms: u64) -> ApplyEffect {
        match event {
            HookEvent::PreCompact {
                session_id,
                trigger,
                custom_instructions,
                transcript_path,
                cwd,
            } => self.on_precompact(
                seat_key,
                Seen {
                    session_id,
                    trigger: *trigger,
                    instructions: custom_instructions.as_deref(),
                    transcript: transcript_path,
                    cwd,
                    now_ms,
                },
            ),
            HookEvent::SessionStart {
                session_id, source, ..
            } => self.on_session_start(seat_key, session_id, source, now_ms),
            HookEvent::PostCompact {
                session_id,
                trigger,
                compact_summary,
            } => self.on_post(seat_key, session_id, *trigger, compact_summary, now_ms),
            HookEvent::UserPromptSubmit { .. }
            | HookEvent::Stop { .. }
            | HookEvent::SessionEnd { .. }
            | HookEvent::Unknown { .. } => ApplyEffect::default(),
        }
    }

    /// Abort attempts that saw `PreCompact` and did not complete. Does not restore.
    ///
    /// Returns true when an attempt changed.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let mut changed = false;
        for attempt in &mut self.attempts {
            if !attempt.seat_open() {
                continue;
            }
            let started = attempt.precompact_ms.unwrap_or(attempt.opened_ms);
            let expired = now_ms.saturating_sub(started) >= self.completion_wait_ms;
            let done = attempt.saw_compact_start && attempt.saw_post_compact;
            if expired && !done {
                attempt.state = AttemptState::Aborted(AbortReason::NoPostCompact);
                changed = true;
            }
        }
        changed
    }

    /// Cancel managed attempts that are still open, and a completed attempt
    /// whose restore has not been submitted.
    ///
    /// A `Completed` attempt is the restoring stage. Leaving it in place lets
    /// the next `Stop` submit that restore.
    pub fn cancel(&mut self, seat_key: &str) -> bool {
        let mut changed = false;
        for attempt in &mut self.attempts {
            if attempt.seat_key != seat_key || !attempt.managed {
                continue;
            }
            if !matches!(
                attempt.state,
                AttemptState::Requested
                    | AttemptState::PreCompactSeen { .. }
                    | AttemptState::Completed { .. }
            ) {
                continue;
            }
            attempt.state = AttemptState::Cancelled;
            changed = true;
        }
        changed
    }

    /// The attempt whose restore may be submitted.
    ///
    /// Requires `Completed`, `ready`, and the post-compact lower bound. Time
    /// without both signals returns `None`.
    #[must_use]
    pub fn due_restore(&self, seat_key: &str, ready: bool, now_ms: u64) -> Option<&Attempt> {
        if !ready {
            return None;
        }
        self.attempts.iter().rev().find(|attempt| {
            attempt.seat_key == seat_key
                && matches!(attempt.state, AttemptState::Completed { .. })
                && attempt
                    .completed_ms
                    .is_some_and(|done| now_ms.saturating_sub(done) >= self.send_wait_ms)
        })
    }

    /// Put a claimed restore back to `Completed` when nothing was pasted.
    ///
    /// `NotReady` is the only caller. A missing summary hash stays
    /// `RestoreSent`, so the attempt is not due again.
    pub fn release_restore_claim(&mut self, id: &AttemptId) -> bool {
        let Some(attempt) = self.by_id_mut(id) else {
            return false;
        };
        if !matches!(attempt.state, AttemptState::RestoreSent) {
            return false;
        }
        let Some(sha) = attempt.summary_sha else {
            return false;
        };
        attempt.state = AttemptState::Completed { summary_sha: sha };
        true
    }

    /// Record that the restore prompt was submitted.
    pub fn mark_restore_sent(&mut self, id: &AttemptId) {
        if let Some(attempt) = self.by_id_mut(id)
            && matches!(attempt.state, AttemptState::Completed { .. })
        {
            attempt.state = AttemptState::RestoreSent;
        }
    }

    /// Record that the restore turn was verified.
    pub fn mark_done(&mut self, id: &AttemptId) {
        if let Some(attempt) = self.by_id_mut(id)
            && matches!(attempt.state, AttemptState::RestoreSent)
        {
            attempt.state = AttemptState::Done;
        }
    }

    /// A prep paste failed before `/compact`. The row is `Failed` even when
    /// [`Self::request`] never opened it.
    pub fn fail_unsent(&mut self, seat_key: &str, id: AttemptId, reason: &str, now_ms: u64) {
        if let Some(attempt) = self.by_id_mut(&id) {
            attempt.state = AttemptState::Failed(reason.to_owned());
            return;
        }
        self.attempts.push(Attempt {
            id,
            seat_key: seat_key.to_owned(),
            trigger: CompactTrigger::Manual,
            managed: true,
            state: AttemptState::Failed(reason.to_owned()),
            session_id: None,
            saw_compact_start: false,
            saw_post_compact: false,
            summary_sha: None,
            summary_len: None,
            opened_ms: now_ms,
            precompact_ms: None,
            completed_ms: None,
            transcript_path: PathBuf::new(),
            cwd: PathBuf::new(),
        });
    }

    /// Enter already happened and the transcript check failed. Do not resend.
    pub fn mark_failed(&mut self, id: &AttemptId, reason: &str) {
        if let Some(attempt) = self.by_id_mut(id)
            && matches!(
                attempt.state,
                AttemptState::Completed { .. } | AttemptState::RestoreSent
            )
        {
            attempt.state = AttemptState::Failed(reason.to_owned());
        }
    }

    /// Attempts for one seat, oldest first.
    #[must_use]
    pub fn for_seat(&self, seat_key: &str) -> Vec<&Attempt> {
        self.attempts
            .iter()
            .filter(|attempt| attempt.seat_key == seat_key)
            .collect()
    }

    /// Every attempt, oldest first.
    #[must_use]
    pub fn attempts(&self) -> &[Attempt] {
        &self.attempts
    }

    /// Replace the book with persisted rows.
    pub fn load(&mut self, attempts: Vec<Attempt>) {
        let mut seq = 1u64;
        for attempt in &attempts {
            seq = seq.max(seq_after(attempt.id.as_str()));
        }
        self.seq = seq;
        self.attempts = attempts;
    }

    /// Id for a managed attempt. Distinct from ids minted for unsolicited `PreCompact`.
    pub fn next_manual_id(&mut self, seat_key: &str) -> AttemptId {
        self.mint_prefixed("manual", seat_key)
    }

    fn on_precompact(&mut self, seat_key: &str, seen: Seen<'_>) -> ApplyEffect {
        let hash = sha256_hex(seen.instructions.unwrap_or("").as_bytes());
        if seen.trigger == CompactTrigger::Manual
            && let Some(attempt) = self.managed_requested_mut(seat_key)
        {
            bind_precompact(attempt, &seen, hash);
            return effect_for(attempt, false);
        }
        let id = self.mint(seat_key);
        let attempt = Attempt {
            id,
            seat_key: seat_key.to_owned(),
            trigger: seen.trigger,
            managed: false,
            state: AttemptState::PreCompactSeen {
                trigger: seen.trigger,
                instructions_hash: hash,
            },
            session_id: Some(seen.session_id.to_owned()),
            saw_compact_start: false,
            saw_post_compact: false,
            summary_sha: None,
            summary_len: None,
            opened_ms: seen.now_ms,
            precompact_ms: Some(seen.now_ms),
            completed_ms: None,
            transcript_path: seen.transcript.to_path_buf(),
            cwd: seen.cwd.to_path_buf(),
        };
        let effect = effect_for(&attempt, false);
        self.attempts.push(attempt);
        effect
    }

    fn on_session_start(
        &mut self,
        seat_key: &str,
        session_id: &str,
        source: &SessionSource,
        now_ms: u64,
    ) -> ApplyEffect {
        if !matches!(source, SessionSource::Compact) {
            return ApplyEffect::default();
        }
        self.note_signal(seat_key, session_id, Signal::Start, now_ms)
    }

    fn on_post(
        &mut self,
        seat_key: &str,
        session_id: &str,
        trigger: CompactTrigger,
        summary: &str,
        now_ms: u64,
    ) -> ApplyEffect {
        let sha = sha256(summary.as_bytes());
        let len = u64::try_from(summary.len()).unwrap_or(0);
        self.note_signal(
            seat_key,
            session_id,
            Signal::Post { trigger, sha, len },
            now_ms,
        )
    }

    fn note_signal(
        &mut self,
        seat_key: &str,
        session_id: &str,
        signal: Signal,
        now_ms: u64,
    ) -> ApplyEffect {
        let mut effect = ApplyEffect::default();
        let Some(index) = newest_match(&self.attempts, seat_key, session_id, signal) else {
            log_unmatched(&self.attempts, seat_key, session_id, signal, &mut effect);
            return effect;
        };
        apply_signal(&mut self.attempts, index, signal, now_ms, &mut effect);
        if self
            .attempts
            .get(index)
            .is_some_and(|attempt| matches!(attempt.state, AttemptState::Completed { .. }))
        {
            supersede_older(&mut self.attempts, index);
        }
        effect
    }

    fn managed_requested_mut(&mut self, seat_key: &str) -> Option<&mut Attempt> {
        self.attempts.iter_mut().rev().find(|attempt| {
            attempt.seat_key == seat_key
                && attempt.managed
                && matches!(attempt.state, AttemptState::Requested)
        })
    }

    fn by_id_mut(&mut self, id: &AttemptId) -> Option<&mut Attempt> {
        self.attempts.iter_mut().find(|attempt| &attempt.id == id)
    }

    fn mint(&mut self, seat_key: &str) -> AttemptId {
        self.mint_prefixed("auto", seat_key)
    }

    fn mint_prefixed(&mut self, prefix: &str, seat_key: &str) -> AttemptId {
        let id = AttemptId(format!("{prefix}-{seat_key}-{}", self.seq));
        self.seq = self.seq.saturating_add(1);
        id
    }
}

impl Attempt {
    fn seat_open(&self) -> bool {
        matches!(
            self.state,
            AttemptState::Requested | AttemptState::PreCompactSeen { .. }
        )
    }

    /// Hex sha256 of the summary, when `PostCompact` stored one.
    #[must_use]
    pub fn summary_sha_hex(&self) -> Option<String> {
        self.summary_sha.map(|sha| hex(&sha))
    }
}

impl AbortReason {
    /// Status token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoPostCompact => "no_post_compact",
        }
    }
}

/// JSON row for `rig compact --status` and `queue list`.
#[must_use]
pub fn summary_value(attempt: &Attempt) -> serde_json::Value {
    let abort_reason = match attempt.state {
        AttemptState::Aborted(reason) => Some(reason.as_str()),
        AttemptState::Failed(ref reason) => Some(reason.as_str()),
        AttemptState::Requested
        | AttemptState::PreCompactSeen { .. }
        | AttemptState::Completed { .. }
        | AttemptState::RestoreSent
        | AttemptState::Done
        | AttemptState::Cancelled => None,
    };
    let confirmed = matches!(
        attempt.state,
        AttemptState::Completed { .. } | AttemptState::RestoreSent | AttemptState::Done
    );
    serde_json::json!({
        "attempt_id": attempt.id.as_str(),
        "seat_key": attempt.seat_key,
        "trigger": attempt.trigger,
        "managed": attempt.managed,
        "state": attempt.state.label(),
        "abort_reason": abort_reason,
        "summary_sha": attempt.summary_sha_hex(),
        "summary_len": attempt.summary_len,
        "confirmed": confirmed,
    })
}

#[derive(Clone, Copy)]
struct Seen<'a> {
    session_id: &'a str,
    trigger: CompactTrigger,
    instructions: Option<&'a str>,
    transcript: &'a std::path::Path,
    cwd: &'a std::path::Path,
    now_ms: u64,
}

#[derive(Clone, Copy)]
enum Signal {
    Start,
    Post {
        trigger: CompactTrigger,
        sha: [u8; 32],
        len: u64,
    },
}

fn bind_precompact(attempt: &mut Attempt, seen: &Seen<'_>, hash: String) {
    attempt.trigger = seen.trigger;
    attempt.session_id = Some(seen.session_id.to_owned());
    attempt.transcript_path = seen.transcript.to_path_buf();
    attempt.cwd = seen.cwd.to_path_buf();
    attempt.precompact_ms = Some(seen.now_ms);
    attempt.state = AttemptState::PreCompactSeen {
        trigger: seen.trigger,
        instructions_hash: hash,
    };
}

fn effect_for(attempt: &Attempt, confirmed: bool) -> ApplyEffect {
    ApplyEffect {
        marker: Some(marker_of(attempt, confirmed)),
        ignored_session_mismatch: false,
        log: Vec::new(),
    }
}

fn marker_of(attempt: &Attempt, confirmed: bool) -> RestoreMarker {
    RestoreMarker {
        seat_key: attempt.seat_key.clone(),
        attempt_id: attempt.id.as_str().to_owned(),
        trigger: attempt.trigger,
        managed: attempt.managed,
        confirmed,
        session_id: attempt.session_id.clone().unwrap_or_default(),
        transcript_path: attempt.transcript_path.clone(),
        cwd: attempt.cwd.clone(),
    }
}

fn newest_match(
    attempts: &[Attempt],
    seat_key: &str,
    session_id: &str,
    signal: Signal,
) -> Option<usize> {
    attempts
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, attempt)| {
            accepts(attempt, seat_key, session_id, signal).then_some(index)
        })
}

fn accepts(attempt: &Attempt, seat_key: &str, session_id: &str, signal: Signal) -> bool {
    if attempt.seat_key != seat_key || !attempt.seat_open() {
        return false;
    }
    let Some(known) = attempt.session_id.as_deref() else {
        return false;
    };
    if known != session_id {
        return false;
    }
    if let Signal::Post { trigger, .. } = signal
        && trigger != attempt.trigger
    {
        return false;
    }
    true
}

fn apply_signal(
    attempts: &mut [Attempt],
    index: usize,
    signal: Signal,
    now_ms: u64,
    effect: &mut ApplyEffect,
) {
    let Some(attempt) = attempts.get_mut(index) else {
        return;
    };
    match signal {
        Signal::Start => attempt.saw_compact_start = true,
        Signal::Post { sha, len, .. } => {
            attempt.saw_post_compact = true;
            attempt.summary_sha = Some(sha);
            attempt.summary_len = Some(len);
        }
    }
    if !(attempt.saw_compact_start && attempt.saw_post_compact) {
        return;
    }
    let Some(sha) = attempt.summary_sha else {
        return;
    };
    attempt.state = AttemptState::Completed { summary_sha: sha };
    attempt.completed_ms = Some(now_ms);
    let len = attempt.summary_len.unwrap_or(0);
    effect.marker = Some(marker_of(attempt, true));
    effect.log.push(format!(
        "attempt {} completed summary {} len {len}",
        attempt.id.as_str(),
        hex(&sha)
    ));
}

fn supersede_older(attempts: &mut [Attempt], newest: usize) {
    let Some(keeper) = attempts.get(newest) else {
        return;
    };
    let seat = keeper.seat_key.clone();
    let session = keeper.session_id.clone();
    let trigger = keeper.trigger;
    for (index, attempt) in attempts.iter_mut().enumerate() {
        if index >= newest
            || attempt.seat_key != seat
            || attempt.trigger != trigger
            || attempt.session_id != session
            || !attempt.seat_open()
        {
            continue;
        }
        attempt.state = AttemptState::Failed("superseded".to_owned());
    }
}

fn log_unmatched(
    attempts: &[Attempt],
    seat_key: &str,
    session_id: &str,
    signal: Signal,
    effect: &mut ApplyEffect,
) {
    for attempt in attempts {
        if attempt.seat_key != seat_key || !attempt.seat_open() {
            continue;
        }
        let Some(known) = attempt.session_id.as_deref() else {
            continue;
        };
        if known != session_id {
            effect.ignored_session_mismatch = true;
            effect.log.push(format!(
                "ignored compaction signal for session {session_id}; attempt {} is {known}",
                attempt.id.as_str()
            ));
            continue;
        }
        if let Signal::Post { trigger, .. } = signal
            && trigger != attempt.trigger
        {
            effect.log.push(format!(
                "ignored post-compact trigger mismatch on {}",
                attempt.id.as_str()
            ));
        }
    }
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(0)
}

fn seq_after(id: &str) -> u64 {
    let Some(rest) = id
        .strip_prefix("auto-")
        .or_else(|| id.strip_prefix("manual-"))
    else {
        return 1;
    };
    let Some((_, digits)) = rest.rsplit_once('-') else {
        return 1;
    };
    digits.parse::<u64>().map_or(1, |n| n.saturating_add(1))
}
