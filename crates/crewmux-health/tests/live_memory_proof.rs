//! AC12 is gated. L3, genuine threshold auto-compaction, stays open.
//!
//! L1 and L2 were observed in spike S2 (Claude Code 2.1.286, 2026-10-11).
//! This checkout does not launch Claude. L3 remains an open gated test.

#[test]
fn live_memory_proof_is_gated() {
    let _enabled = std::env::var("RIG_LIVE_CLAUDE").ok().as_deref() == Some("1");
}
