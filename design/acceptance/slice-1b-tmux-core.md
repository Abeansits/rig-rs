> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# Slice 1b: tmux core (crewmux-mux)

Depends on slice 0. Merged as PR #4. Implements ADR-0005.
**The AC numbering below is reconstructed. The original had about 25 ACs, with AC24–25 being the window-size rules.**

## Acceptance criteria
- **AC1:** Every mutation is an argv `exec` on our private socket (`-L rig-rs…`/`-S`). There is no shell string interpolation.
- **AC2:** `crewmux_types::tmux_version` is the only tmux version parser. Below 3.3 → `TmuxTooOld` (minimum 3.3 is final, Sebastian).
- **AC3:** Exactly one control-mode client per server, attached to the hidden `crewmux-control` session.
- **AC4:** Observation (output, pane exits, window changes) comes only from the control stream.
- **AC5:** The `list-clients` format is `#{client_control_mode}|#{client_activity}|#{client_tty}|#{client_session}`. Control clients are excluded from "human attached".
- **AC6:** Paste goes through a private buffer written from a 0600 temp file, then `paste-buffer -p -r -d`. The buffer is deleted after use.
- **AC7:** Submit is a separate `send-keys Enter` per provider policy. Blind Enter is repo PE-013, which supersedes PE-014.
- **AC8:** A bell-window doorbell signals buffer and paste completion, because tmux 3.3a lacks `%paste-buffer-changed`.
- **AC9:** A `pane-died` hook plus `remain-on-exit` (per window, never global) detects death with the exit status.
- **AC10:** Windows are unlinked from `crewmux-control` before `kill-session`, so the control session survives.
- **AC11:** On Linux only, after child exits on `rig-rs*` sockets, the server gets a SIGCHLD nudge (rustix). This works around libutempter.
- **AC12:** The control client reconnects after a server restart without duplicating events.
- **AC13:** `FakeMux` implements the same `Multiplexer` trait for unit tests.
- **AC14:** Real-tmux tests sit behind the `tmux-it` feature. The CI matrix is tmux 3.3a, 3.5a and 3.8.
- **AC15:** `cargo xtask lint-mux`: no `Command::new("tmux")` outside `crewmux-mux`.
- **AC24:** `set -g window-size` (or any global window-size) → `MuxError::Forbidden`. It crashes 3.5a.
- **AC25:** `cargo xtask lint window-size` fails on a global window-size anywhere in the repo or managed configs.
