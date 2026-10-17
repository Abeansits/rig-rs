> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# ADR-0004: TUI crate: ratatui + crossterm; agent terminals attach externally

Status: Accepted. The external-attach-only decision is Sebastian's (2026-10-11).

## Context
- OpenRig's TUI (`packages/tui`) shows rigs, seats, attention and chat. It has mouse hit-targets and a control socket.
- The Rust TUI must be fast with 20+ seats and must restore the terminal reliably across attach/detach.

## Options
1. **ratatui + crossterm.** Maintained, immediate-mode, works with `TestBackend` for golden frames.
2. cursive (retained mode). Less control over custom hit-maps.
3. A custom renderer on termion/crossterm. Too much to own.

## Decision
- **ratatui with the crossterm backend**, in `crewmux-tui`. Architecture is **Elm-style**: `Model`, pure `update(Model, Msg) -> (Model, Cmd)` and `view(&Model, &mut Frame)`. All I/O goes through `Cmd`.
- **Data** comes only from `crewmux-api`: hydrate (`GET /api/seats`), then the event stream (`GET /api/events`). The canonical event stream is slice 4's; slice 13 shipped a minimal version that slice 4 reconciled.
- **Mouse:** a hit-map of `(row, x1..=x2) → Action`, with disclosure cells registered before rows. Recorded OpenRig screens are compared by action identity, not coordinates.
- **Agent terminals: external attach only (Sebastian, 2026-10-11).**
  - `a` suspends the TUI, runs the daemon-provided attach argv, and restores raw mode, mouse and bracketed paste.
  - Inside `$TMUX` on the same server it uses `switch-client`.
  - Herdr panes are attach-only (ADR-0008 rule 5). Ghostty goes through the slice 11 launcher.
  - **Embedded live terminals are out of scope.** Spike S7 showed an embedded read-only preview is viable (about 1.3% CPU, row-exact), so it is recorded as a possible later enhancement only.
- **Terminal safety:** a panic hook is installed before entering raw mode. Ctrl-C quits.
- A **control socket**, private and mode 0600, guarded against `sun_path` overflow, replies with an error to unknown commands.

## Consequences
- Rendering is testable with `TestBackend` goldens (120×40, 80×24).
- Perf budget: a 20-seat frame in under 5 ms in release.
- The attach round-trip is tested with 50 real pty attach/detach cycles (slice 13).

## Open risks
- Terminal restoration on unusual terminals; mitigated by the 50-cycle test.
- Hit-map drift from OpenRig layout; accepted, because identity is compared, not coordinates.
