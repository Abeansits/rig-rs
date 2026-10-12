//! AC12 is gated on `RIG_LIVE_CLAUDE=1`. This checkout does not launch Claude.
//!
//! L1 and L2 (manual `/compact`, then resume) were observed in spike S2
//! (Claude Code 2.1.286, 2026-10-11) and were not re-run here. L3, genuine
//! threshold auto-compaction, stays open: `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`
//! did not produce `PostCompact` in that spike.
//!
//! AC13: `restore_prompt_golden` locks the restore prompt. The turn-boundary
//! and compliance prompts are not sent (PE-018). There is no further AC13
//! snapshot to close.

#[test]
fn live_memory_proof_is_gated() {
    let enabled = std::env::var("RIG_LIVE_CLAUDE").ok().as_deref() == Some("1");
    assert!(!enabled, "this suite must not launch Claude");
}
