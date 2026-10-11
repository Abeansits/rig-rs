# Parity ledger

The acceptance ledger is [`parity/ledger.csv`](../../parity/ledger.csv). `cargo xtask ledger check` is the gate. This note only lists the slice 1b mapping.

| TS test file | Slice | Status | Rust test(s) | Notes |
|---|---|---|---|---|
| packages/daemon/test/tmux-adapter.test.ts | 1b | ported | crates/crewmux-mux/tests/parity_tmux.rs | Paste goes through a buffer file. Live delivery is `tmux_it`. |
| packages/daemon/test/tmux-argv-exec.test.ts | 1b | ported | crates/crewmux-mux/tests/parity_tmux.rs | Argv units for create and send-keys. Pipe-pane and respawn stay later. |
| packages/daemon/test/tmux-exact-target.test.ts | 1b | ported | crates/crewmux-mux/tests/parity_tmux.rs | A prefix of a live session name is not that session. |
| packages/daemon/test/tmux-session-names-native.test.ts | 1b | ported | crates/crewmux-mux/tests/parity_tmux.rs | tmux rewrites `.` and `:` to `_`. |
| packages/daemon/test/tmux-session-env-native.test.ts | 1b | ported | crates/crewmux-mux/tests/parity_tmux.rs | Session env is a verbatim `-e` unit. |
| packages/daemon/test/seat-delivery-guard.test.ts | 4 | todo | | Verdict is slice 4. `guard_inputs` is the tmux-side input. |
| packages/daemon/test/seat-delivery-guard-correction.test.ts | 4 | todo | | Verdict deferred to slice 4. |
| packages/daemon/test/terminal-session-broker.test.ts | 1b | todo | | Pipe-pane broker is out of this slice. Pane death is push-based in `crewmux-mux`. |
