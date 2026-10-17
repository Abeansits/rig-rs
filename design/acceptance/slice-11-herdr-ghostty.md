> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# Slice 11: Herdr and Ghostty integrations

Depends on 7, 13. **D4 (Ghostty strategy) is pending with Sebastian.** Herdr is attach-only (decided).

## Acceptance criteria
### Herdr
- **AC1:** rig-rs never starts, stops or configures a Herdr server. It only attaches through the private launch config from slice 7 (0600).
- **AC2:** `rig attach --herdr <seat>` and TUI `a` on a Herdr pane exec the Herdr attach argv. If Herdr is missing → `OfferInstall` hint, exit 1.
- **AC3:** Herdr panes appear in `rig ps`/TUI as `external`, with no health probing beyond liveness.
### Ghostty
- **AC4:** On macOS, "open in Ghostty" uses the strategy chosen by D4. The default until decided is `osascript` to open a new window running the attach argv.
- **AC5:** On non-macOS → `Unsupported` with a clear message (no panic, exit 1).
- **AC6:** The argv is built with no shell interpolation (escaped once, tested with spaces and quotes in session names).
- **AC7:** Ghostty install is only offered (needs `--yes` plus the flag, ADR-0008).
### Both
- **AC8:** Doctor reports presence and versions without failing when they are absent (optional integrations).
- **AC9:** Tests use fixture probes. Real Herdr and Ghostty are behind feature flags.
