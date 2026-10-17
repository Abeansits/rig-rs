// Derived from openrig packages/daemon/src/domain/claude-compaction-enforcer.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Pure compaction policy. Sample state lives in memory and is not persisted.
//!
//! `decide` never returns [`EnforcerOutcome::SendRestore`]. A restore is queued
//! by the attempt book after both hook signals, then gated on `Ready`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use crewmux_types::{RuntimeKind, SeatId};

use super::prompt::{
    PrepPrompt, build_compact_command, build_pre_compact_prep_prompt, completion_marker, map_path,
};
use super::threshold::Threshold;
use super::{AUTO_PREP_WAIT, DEDUP_WINDOW, MANUAL_PREP_WAIT, POST_COMPACT_RESTORE_COOLDOWN};

/// Why `decide` did not send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnforcerSkipReason {
    /// A typing guard blocked delivery.
    TypingGuardEnabled,
    /// The delivery guard could not name the target.
    GuardTargetUnknown,
    /// The occupant changed under the guard.
    GuardTargetChanged,
    /// The seat is not Claude Code.
    RuntimeFilter,
    /// No context-usage sample.
    NoUsageData,
    /// Policy `enabled` is false.
    Disabled,
    /// Usage is under the threshold. This also re-arms.
    BelowThreshold,
    /// A compact already fired and usage has not dropped back under the threshold.
    AlreadyTriggeredAboveThreshold,
    /// Inside the 60s dedup window.
    DedupWindow,
    /// Inside the post-restore cooldown.
    PostRestoreCooldown,
    /// The seat was not `Ready`, or the send did not land.
    SendFailed,
    /// Threshold was 0, 101, NaN, or not an integer.
    InvalidPolicy,
    /// The occupant generation changed.
    StaleGeneration,
    /// The restore map is not published yet.
    PreparationPending,
    /// The preparation deadline passed.
    PreparationIncomplete,
    /// Preparation was stopped and must not continue.
    PreparationStopped,
    /// No occupant generation was available for a map.
    OccupantGenerationUnavailable,
    /// Nothing is waiting to be drained.
    NoPendingStage,
    /// A manual attempt is already running.
    AlreadyInProgress,
}

impl EnforcerSkipReason {
    /// Stable token for status JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TypingGuardEnabled => "typing_guard_enabled",
            Self::GuardTargetUnknown => "guard_target_unknown",
            Self::GuardTargetChanged => "guard_target_changed",
            Self::RuntimeFilter => "runtime_filter",
            Self::NoUsageData => "no_usage_data",
            Self::Disabled => "disabled",
            Self::BelowThreshold => "below_threshold",
            Self::AlreadyTriggeredAboveThreshold => "already_triggered_above_threshold",
            Self::DedupWindow => "dedup_window",
            Self::PostRestoreCooldown => "post_restore_cooldown",
            Self::SendFailed => "send_failed",
            Self::InvalidPolicy => "invalid_policy",
            Self::StaleGeneration => "stale_generation",
            Self::PreparationPending => "preparation_pending",
            Self::PreparationIncomplete => "preparation_incomplete",
            Self::PreparationStopped => "preparation_stopped",
            Self::OccupantGenerationUnavailable => "occupant_generation_unavailable",
            Self::NoPendingStage => "no_pending_stage",
            Self::AlreadyInProgress => "already_in_progress",
        }
    }
}

/// What the caller may submit, and only through `SeatIo` when the seat is `Ready`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnforcerOutcome {
    /// Do not send.
    Skip(EnforcerSkipReason),
    /// Preparation prompt.
    SendPrep(String),
    /// `/compact` command.
    SendCompact(String),
    /// Restore prompt. Produced by the attempt book, not by a timer inside `decide`.
    SendRestore(String),
}

/// Surfaced manual-compaction stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManualCompactionStage {
    /// The prep prompt is being sent.
    Preparing,
    /// Waiting for the restore map.
    WaitingMap,
    /// `/compact` was submitted. Hooks have not both arrived.
    Compacting,
    /// Hooks completed and the restore prompt is owed.
    Restoring,
    /// The restore was submitted.
    Done,
    /// The operator cancelled.
    Cancelled,
    /// Preparation or delivery failed.
    Failed,
}

impl ManualCompactionStage {
    /// Status token for `rig compact --status`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::WaitingMap => "waiting_map",
            Self::Compacting => "compacting",
            Self::Restoring => "restoring",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    /// Preparing, waiting for the map, compacting, or restoring.
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::WaitingMap | Self::Compacting | Self::Restoring
        )
    }
}

/// Automatic or manual preparation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepMode {
    /// Threshold-driven.
    Automatic,
    /// `rig compact`.
    Manual,
}

/// Where a preparation attempt is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepStatus {
    /// The prep prompt has not landed.
    Sending,
    /// Waiting for the map, or ready to compact.
    Waiting,
    /// Stopped. A later tick must not send `/compact`.
    Stopped,
    /// `/compact` was sent.
    CompactSent,
}

/// Caller-owned preparation. The enforcer does not persist it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparationState {
    /// False when no attempt is open.
    pub active: bool,
    /// Attempt id quoted in the prompt.
    pub attempt_id: String,
    /// Who started it.
    pub mode: PrepMode,
    /// Progress.
    pub status: PrepStatus,
    /// The map file ends with this attempt's marker.
    pub map_complete: bool,
    /// Deadline in Unix milliseconds.
    pub deadline_ms: Option<u64>,
    /// Why it stopped.
    pub reason: Option<String>,
}

impl PreparationState {
    /// No preparation in flight.
    #[must_use]
    pub fn idle() -> Self {
        Self {
            active: false,
            attempt_id: String::new(),
            mode: PrepMode::Automatic,
            status: PrepStatus::Sending,
            map_complete: false,
            deadline_ms: None,
            reason: None,
        }
    }
}

impl Default for PreparationState {
    fn default() -> Self {
        Self::idle()
    }
}

/// One observation for [`Enforcer::decide`].
pub struct EnforcerInput {
    /// Seat id. Prompts use [`Self::session`].
    pub seat: SeatId,
    /// Session name, the dedup key.
    pub session: String,
    /// Runtime filter. Only Claude Code proceeds.
    pub runtime: RuntimeKind,
    /// Context used, as a percent.
    pub used_percent: Option<u8>,
    /// When the sample was taken, Unix milliseconds.
    pub sample_at_ms: Option<u64>,
    /// Operator threshold.
    pub threshold: Threshold,
    /// `policies.claude_compaction.enabled`.
    pub enabled: bool,
    /// Classifier says the composer is ready. Sends wait while this is false.
    pub ready: bool,
    /// Launch workspace. Absolute paths host the restore map.
    pub cwd: Option<PathBuf>,
    /// Transcript quoted in the compact command's map note.
    pub transcript_path: Option<PathBuf>,
    /// Claude session id, when known.
    pub session_id: Option<String>,
    /// Operator `/compact` instruction.
    pub compact_instruction: String,
    /// Operator prep instruction.
    pub pre_compact_instruction: Option<String>,
    /// Rig home, for the legacy map location.
    pub rig_home: PathBuf,
    /// Id to use when this call starts a preparation.
    pub attempt_id: String,
    /// `--skip-map`: compact without a published map.
    pub skip_map: bool,
}

/// In-memory dedup, re-arm, and cooldown. Not written to sqlite.
#[derive(Debug)]
pub struct Enforcer {
    tracks: BTreeMap<String, Track>,
    dedup_ms: u64,
    cooldown_ms: u64,
    manual_wait_ms: u64,
    auto_wait_ms: u64,
}

#[derive(Debug, Default)]
struct Track {
    last_compact_ms: Option<u64>,
    cooldown_until_ms: Option<u64>,
    armed: bool,
    stage: Option<ManualCompactionStage>,
}

impl Default for Enforcer {
    fn default() -> Self {
        Self::new()
    }
}

impl Enforcer {
    /// Windows from the acceptance spec.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tracks: BTreeMap::new(),
            dedup_ms: duration_ms(DEDUP_WINDOW),
            cooldown_ms: duration_ms(POST_COMPACT_RESTORE_COOLDOWN),
            manual_wait_ms: duration_ms(MANUAL_PREP_WAIT),
            auto_wait_ms: duration_ms(AUTO_PREP_WAIT),
        }
    }

    /// Replace the dedup and cooldown windows. Tests inject a clock beside this.
    #[must_use]
    pub fn with_windows(mut self, dedup: Duration, cooldown: Duration) -> Self {
        self.dedup_ms = duration_ms(dedup);
        self.cooldown_ms = duration_ms(cooldown);
        self
    }

    /// Decide the next auto action. Does not send a restore because of elapsed time.
    pub fn decide(
        &mut self,
        input: &EnforcerInput,
        prep: &PreparationState,
        now_ms: u64,
    ) -> EnforcerOutcome {
        let Some(gate) = self.gate(input, now_ms, true) else {
            return self.after_gate(input, prep, now_ms);
        };
        if matches!(gate, EnforcerSkipReason::BelowThreshold) {
            self.rearm(&input.session);
        }
        EnforcerOutcome::Skip(gate)
    }

    /// Operator `rig compact`. Skips the threshold and the enabled gate.
    pub fn decide_manual(
        &mut self,
        input: &EnforcerInput,
        prep: &PreparationState,
        now_ms: u64,
    ) -> EnforcerOutcome {
        if self.settled_compact(&input.session, prep) {
            let idle = PreparationState::idle();
            return self.finish_manual(input, &idle, now_ms);
        }
        self.finish_manual(input, prep, now_ms)
    }

    fn finish_manual(
        &mut self,
        input: &EnforcerInput,
        prep: &PreparationState,
        now_ms: u64,
    ) -> EnforcerOutcome {
        if manual_blocked(prep, input) {
            return EnforcerOutcome::Skip(EnforcerSkipReason::AlreadyInProgress);
        }
        if let Some(reason) = runtime_and_usage(input) {
            return self.fail_manual(&input.session, reason);
        }
        let outcome = self.prepare(input, prep, now_ms, PrepMode::Manual);
        self.note_manual(&input.session, &outcome, prep);
        outcome
    }

    /// Manual stage, when this session has one.
    #[must_use]
    pub fn manual_stage(&self, session: &str) -> Option<ManualCompactionStage> {
        self.tracks.get(session).and_then(|track| track.stage)
    }

    /// `rig compact <seat> --cancel`.
    pub fn cancel(&mut self, session: &str) {
        let track = self.tracks.entry(session.to_owned()).or_default();
        track.stage = Some(ManualCompactionStage::Cancelled);
        track.armed = false;
    }

    /// The attempt is done, aborted, or failed. A later compact may start.
    ///
    /// Does not start the post-restore cooldown. That stays on
    /// [`Self::note_restore_finished`].
    pub fn note_settled(&mut self, session: &str, stage: ManualCompactionStage) {
        if stage.is_open() {
            return;
        }
        let track = self.tracks.entry(session.to_owned()).or_default();
        track.armed = false;
        if track.stage.is_none_or(ManualCompactionStage::is_open) {
            track.stage = Some(stage);
        }
    }

    /// Record that the restore prompt landed, and start the cooldown.
    pub fn note_restore_finished(&mut self, session: &str, now_ms: u64) {
        let track = self.tracks.entry(session.to_owned()).or_default();
        track.armed = false;
        track.cooldown_until_ms = Some(now_ms.saturating_add(self.cooldown_ms));
        if track.stage == Some(ManualCompactionStage::Restoring) {
            track.stage = Some(ManualCompactionStage::Done);
        }
    }

    /// Hooks completed. The manual stage becomes `Restoring`. Time alone does not do this.
    pub fn note_hooks_completed(&mut self, session: &str) {
        let track = self.tracks.entry(session.to_owned()).or_default();
        if track.stage == Some(ManualCompactionStage::Compacting) {
            track.stage = Some(ManualCompactionStage::Restoring);
        }
    }

    fn gate(
        &mut self,
        input: &EnforcerInput,
        now_ms: u64,
        honor_enabled: bool,
    ) -> Option<EnforcerSkipReason> {
        if let Some(reason) = runtime_and_usage(input) {
            return Some(reason);
        }
        let Some(threshold) = input.threshold.get() else {
            return Some(EnforcerSkipReason::InvalidPolicy);
        };
        let used = input.used_percent.unwrap_or(0);
        if used < threshold {
            return Some(EnforcerSkipReason::BelowThreshold);
        }
        if honor_enabled && !input.enabled {
            return Some(EnforcerSkipReason::Disabled);
        }
        self.cooldown_and_dedup(&input.session, now_ms)
    }

    fn cooldown_and_dedup(&mut self, session: &str, now_ms: u64) -> Option<EnforcerSkipReason> {
        let track = self.tracks.entry(session.to_owned()).or_default();
        if let Some(until) = track.cooldown_until_ms {
            if now_ms < until {
                return Some(EnforcerSkipReason::PostRestoreCooldown);
            }
            track.cooldown_until_ms = None;
        }
        if let Some(last) = track.last_compact_ms
            && now_ms.saturating_sub(last) < self.dedup_ms
        {
            return Some(EnforcerSkipReason::DedupWindow);
        }
        if track.armed {
            return Some(EnforcerSkipReason::AlreadyTriggeredAboveThreshold);
        }
        None
    }

    fn after_gate(
        &mut self,
        input: &EnforcerInput,
        prep: &PreparationState,
        now_ms: u64,
    ) -> EnforcerOutcome {
        // A finished manual attempt must not keep auto compaction parked.
        if self.settled_compact(&input.session, prep) {
            let idle = PreparationState::idle();
            return self.prepare(input, &idle, now_ms, PrepMode::Automatic);
        }
        if prep.active && prep.mode == PrepMode::Manual && prep.status != PrepStatus::Stopped {
            return EnforcerOutcome::Skip(EnforcerSkipReason::PreparationPending);
        }
        self.prepare(input, prep, now_ms, PrepMode::Automatic)
    }

    fn prepare(
        &mut self,
        input: &EnforcerInput,
        prep: &PreparationState,
        now_ms: u64,
        mode: PrepMode,
    ) -> EnforcerOutcome {
        if prep.active && prep.status == PrepStatus::Stopped {
            return EnforcerOutcome::Skip(EnforcerSkipReason::PreparationStopped);
        }
        if prep.active && prep.status == PrepStatus::CompactSent {
            return EnforcerOutcome::Skip(EnforcerSkipReason::AlreadyTriggeredAboveThreshold);
        }
        if let Some(deadline) = prep.deadline_ms
            && prep.active
            && now_ms >= deadline
            && !prep.map_complete
            && !input.skip_map
        {
            return EnforcerOutcome::Skip(EnforcerSkipReason::PreparationIncomplete);
        }
        if !prep.active || prep.status == PrepStatus::Sending {
            return self.send_prep(input, now_ms, mode);
        }
        if !input.skip_map && !prep.map_complete {
            return EnforcerOutcome::Skip(EnforcerSkipReason::PreparationPending);
        }
        self.send_compact(input, prep, now_ms)
    }

    fn send_prep(&mut self, input: &EnforcerInput, now_ms: u64, mode: PrepMode) -> EnforcerOutcome {
        self.clear_settled(&input.session);
        if !input.ready {
            return EnforcerOutcome::Skip(EnforcerSkipReason::SendFailed);
        }
        let attempt = if input.attempt_id.is_empty() {
            "attempt".to_owned()
        } else {
            input.attempt_id.clone()
        };
        let path = map_path(
            &input.rig_home,
            input.cwd.as_deref(),
            &input.session,
            &attempt,
        );
        let deadline = match mode {
            PrepMode::Manual => Some(now_ms.saturating_add(self.manual_wait_ms)),
            PrepMode::Automatic => Some(now_ms.saturating_add(self.auto_wait_ms)),
        };
        let prompt = build_pre_compact_prep_prompt(&PrepPrompt {
            mode: mode_name(mode),
            used_percent: input.used_percent.unwrap_or(0),
            threshold_percent: input.threshold.get().unwrap_or(0),
            pre_compact_instruction: input.pre_compact_instruction.as_deref(),
            map_path: &path,
            marker: &completion_marker(&attempt, &input.session),
            deadline_ms: deadline,
            now_ms,
        });
        EnforcerOutcome::SendPrep(prompt)
    }

    fn send_compact(
        &mut self,
        input: &EnforcerInput,
        prep: &PreparationState,
        now_ms: u64,
    ) -> EnforcerOutcome {
        if !input.ready {
            return EnforcerOutcome::Skip(EnforcerSkipReason::SendFailed);
        }
        let map = if input.skip_map {
            None
        } else {
            Some(map_path(
                &input.rig_home,
                input.cwd.as_deref(),
                &input.session,
                &prep.attempt_id,
            ))
        };
        let command = build_compact_command(&input.compact_instruction, map.as_deref());
        let track = self.tracks.entry(input.session.clone()).or_default();
        track.armed = true;
        track.last_compact_ms = Some(now_ms);
        EnforcerOutcome::SendCompact(command)
    }

    fn rearm(&mut self, session: &str) {
        if let Some(track) = self.tracks.get_mut(session) {
            track.armed = false;
        }
    }

    fn clear_settled(&mut self, session: &str) {
        let Some(track) = self.tracks.get_mut(session) else {
            return;
        };
        if track.stage.is_some_and(|stage| !stage.is_open()) {
            track.stage = None;
        }
    }

    fn settled_compact(&self, session: &str, prep: &PreparationState) -> bool {
        self.stage_terminal(session) && prep.active && prep.status == PrepStatus::CompactSent
    }

    fn stage_terminal(&self, session: &str) -> bool {
        self.manual_stage(session)
            .is_some_and(|stage| !stage.is_open())
    }

    fn fail_manual(&mut self, session: &str, reason: EnforcerSkipReason) -> EnforcerOutcome {
        self.tracks.entry(session.to_owned()).or_default().stage =
            Some(ManualCompactionStage::Failed);
        EnforcerOutcome::Skip(reason)
    }

    fn note_manual(&mut self, session: &str, outcome: &EnforcerOutcome, prep: &PreparationState) {
        let stage = match outcome {
            EnforcerOutcome::SendPrep(_) => ManualCompactionStage::Preparing,
            EnforcerOutcome::SendCompact(_) => ManualCompactionStage::Compacting,
            EnforcerOutcome::Skip(EnforcerSkipReason::PreparationPending) => {
                ManualCompactionStage::WaitingMap
            }
            EnforcerOutcome::Skip(EnforcerSkipReason::SendFailed) => {
                if prep.active {
                    ManualCompactionStage::WaitingMap
                } else {
                    ManualCompactionStage::Preparing
                }
            }
            EnforcerOutcome::Skip(_) => ManualCompactionStage::Failed,
            EnforcerOutcome::SendRestore(_) => ManualCompactionStage::Restoring,
        };
        self.tracks.entry(session.to_owned()).or_default().stage = Some(stage);
    }
}

/// Prompt text when the seat is `Ready`. `None` when it is not, including for a restore.
#[must_use]
pub fn gated_submit(ready: bool, outcome: &EnforcerOutcome) -> Option<String> {
    if !ready {
        return None;
    }
    match outcome {
        EnforcerOutcome::SendPrep(text)
        | EnforcerOutcome::SendCompact(text)
        | EnforcerOutcome::SendRestore(text) => Some(text.clone()),
        EnforcerOutcome::Skip(_) => None,
    }
}

fn manual_blocked(prep: &PreparationState, input: &EnforcerInput) -> bool {
    if !prep.active || prep.mode != PrepMode::Manual || prep.status == PrepStatus::Stopped {
        return false;
    }
    if prep.attempt_id != input.attempt_id || prep.status == PrepStatus::CompactSent {
        return true;
    }
    input.used_percent.is_none()
}

fn runtime_and_usage(input: &EnforcerInput) -> Option<EnforcerSkipReason> {
    if input.runtime != RuntimeKind::ClaudeCode {
        return Some(EnforcerSkipReason::RuntimeFilter);
    }
    if input.used_percent.is_none() {
        return Some(EnforcerSkipReason::NoUsageData);
    }
    None
}

fn mode_name(mode: PrepMode) -> &'static str {
    match mode {
        PrepMode::Automatic => "automatic",
        PrepMode::Manual => "manual",
    }
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(0)
}
