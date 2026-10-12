//! Human-input guard (ADR-0005 rules 1–6). Pure: no tmux and no paste.

use std::time::{Duration, Instant, SystemTime};

use crewmux_types::ReadinessCode;

use crate::types::GuardInputs;

/// What the composer parser reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposerState {
    /// A prompt is waiting, with no draft after it.
    AtPrompt,
    /// The runtime is in a menu or other non-prompt screen.
    NotAtPrompt,
    /// A human draft is sitting in the composer. `age` is how long it has waited.
    Draft {
        /// Time since the draft was first observed.
        age: Duration,
    },
    /// No parser matched. Rules 2–3 still run, and the receipt is degraded.
    Unknown,
}

/// Quiet window and how long a draft may defer before escalation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuardConfig {
    /// Keystrokes inside this window defer. Default 3s.
    pub quiet: Duration,
    /// A draft older than this escalates instead of deferring.
    pub max_defer: Duration,
    /// Wall clock used to age tmux `client_activity`. Tests inject it.
    pub wall: SystemTime,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self {
            quiet: Duration::from_secs(3),
            max_defer: Duration::from_secs(30),
            wall: SystemTime::UNIX_EPOCH,
        }
    }
}

/// Why a paste is not issued.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeferReason {
    /// Rule 1.
    NotAtPrompt,
    /// Rule 2. Copy mode or another pane mode.
    InCopyMode,
    /// Rule 3. A human keystroke inside the quiet window.
    HumanTyping,
    /// Rule 4. Text the human typed. Never overwritten.
    HumanDraft,
    /// An interactive gate. `rig send` refuses these unless overridden.
    Interactive(ReadinessCode),
}

/// What the caller may do with the paste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardVerdict {
    /// Paste is allowed.
    Deliver,
    /// Wait, then ask again.
    Defer {
        /// Suggested wait.
        retry_after: Duration,
        /// Which rule fired.
        reason: DeferReason,
    },
    /// Stop deferring. Do not paste over the human.
    Escalate(DeferReason),
}

/// Verdict plus the degraded flag the receipt copies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluated {
    /// Rules 1–6.
    pub verdict: GuardVerdict,
    /// True when the composer was [`ComposerState::Unknown`].
    pub guard_degraded: bool,
}

/// Apply ADR-0005 rules 1–6 in order.
#[must_use]
pub fn evaluate(
    inputs: &GuardInputs,
    composer: ComposerState,
    cfg: &GuardConfig,
    now: Instant,
) -> Evaluated {
    let degraded = matches!(composer, ComposerState::Unknown);
    if !degraded && matches!(composer, ComposerState::NotAtPrompt) {
        return defer(DeferReason::NotAtPrompt, cfg.quiet, false);
    }
    if inputs.pane_in_mode {
        return defer(DeferReason::InCopyMode, cfg.quiet, degraded);
    }
    if typing(inputs, cfg, now) {
        return defer(DeferReason::HumanTyping, cfg.quiet, degraded);
    }
    if degraded {
        return Evaluated {
            verdict: GuardVerdict::Deliver,
            guard_degraded: true,
        };
    }
    if let ComposerState::Draft { age } = composer {
        return draft_verdict(age, cfg);
    }
    Evaluated {
        verdict: GuardVerdict::Deliver,
        guard_degraded: false,
    }
}

fn draft_verdict(age: Duration, cfg: &GuardConfig) -> Evaluated {
    if age >= cfg.max_defer {
        return Evaluated {
            verdict: GuardVerdict::Escalate(DeferReason::HumanDraft),
            guard_degraded: false,
        };
    }
    defer(
        DeferReason::HumanDraft,
        cfg.max_defer.saturating_sub(age),
        false,
    )
}

fn defer(reason: DeferReason, retry_after: Duration, guard_degraded: bool) -> Evaluated {
    Evaluated {
        verdict: GuardVerdict::Defer {
            retry_after,
            reason,
        },
        guard_degraded,
    }
}

fn typing(inputs: &GuardInputs, cfg: &GuardConfig, now: Instant) -> bool {
    let tap = inputs
        .last_human_keystroke_tap
        .is_some_and(|at| now.saturating_duration_since(at) < cfg.quiet);
    let tmux = inputs
        .last_human_keystroke_tmux
        .is_some_and(|at| wall_age(cfg.wall, at) < cfg.quiet);
    tap || tmux
}

fn wall_age(wall: SystemTime, stroke: SystemTime) -> Duration {
    wall.duration_since(stroke).unwrap_or(Duration::ZERO)
}
