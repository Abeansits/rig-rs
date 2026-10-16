> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# Slice 10: OpenCode seat

Depends on 1, 2, 3, 7. **D3 (OpenCode version and model source) is pending with Sebastian.** The criteria target opencode 2.x behind a version probe and don't hard-code a model.

## Acceptance criteria
- **AC1:** Version probe of `opencode --version`. Versions outside the supported range → seat `Attention` with a reason and no launch. The range is set by D3 when decided.
- **AC2:** Launch uses a per-seat config through `OPENCODE_CONFIG` (0600, ADR-0008 rule 4). `~/.config/opencode` is untouched (fs snapshot).
- **AC3:** The model comes from the AgentSpec if set, else OpenCode's own default. rig-rs does not choose one (D3).
- **AC4:** The rig MCP server is registered in the seat config with the 15 tool names (ADR-0002, PE-016).
- **AC5:** Readiness is detected from the recorded opencode 2.x screen corpus. A false-ready guard mirrors Claude's (PE-001 style).
- **AC6:** Delivery uses the slice 2/3 paths:
  - queue rows → pointer nudge (G2);
  - direct and startup → text-hash check with OpenCode normalisation (ADR-0009).
- **AC7:** Dialogs (permission and trust prompts) go only through the shared dialog driver (ADR-0010). Unknown dialog → escalate.
- **AC8:** Session id and resume come from OpenCode's session store and are read-only. A missing id → resume unsupported, reported honestly.
- **AC9:** Death and restart follow slice 1 semantics (pane-died → Dead).
- **AC10:** The ledger rows for OpenCode adapter tests are `ported` or `diverged` with a PE.
