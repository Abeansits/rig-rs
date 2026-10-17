> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# Slice 13: TUI v1

Depends on 1, 1b, 4. Merged as PR #10. Implements ADR-0004.

## Acceptance criteria
- **AC1:** A pure `update(Model, Msg)`. All I/O is via `Cmd`, unit-tested without a terminal.
- **AC2:** TestBackend goldens at 120×40 and 80×24.
- **AC3:** An honest `LoadState`: loading, empty, error and stale are rendered distinctly, never as blank data.
- **AC4:** `RowFlash` on changed rows, with fake-clock tests.
- **AC5:** Hit-test rule: the disclosure cell is registered before its row, and a click maps to exactly one action.
- **AC6:** `cargo xtask record-tui` records OpenRig screens. Comparison is by action identity, not coordinates.
- **AC7:** Wheel scrolls the focused list.
- **AC8:** `a` attaches via the daemon-provided argv. Inside `$TMUX` on the same server it uses `switch-client`.
- **AC9:** 50 real pty attach/detach cycles restore raw mode, mouse and paste each time. A panic hook restores the terminal.
- **AC10:** Herdr seats are attach-only. Ghostty uses osascript on macOS, `Unsupported` elsewhere (finalised in slice 11).
- **AC11:** Control socket at 0600 with a `sun_path` guard. Unknown commands → error reply.
- **AC12:** A 20-seat frame renders in under 5 ms (release, benchmark).
- **AC13:** Routes used: `GET /api/seats`, `POST /api/seats/attach`, `GET /api/events` (with cursor and gap). **Slice 4's events endpoint is canonical.**
- **AC14:** The human seat attention view (G3) lists items needing the human. It is the TUI half of G3; the CLI inbox is in slice 18.
