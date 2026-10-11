# Parity ledger

One row per OpenRig TS test file that is in scope. Status: `ported` | `partial` | `skipped` | `todo`.
`skipped` needs a reason (out-of-scope feature, TS-internal detail, superseded by fresh schema).
CI (`guards` job) fails if a row says `ported` but the referenced Rust test path does not exist.

| TS test file | Slice | Status | Rust test(s) | Notes |
|---|---|---|---|---|
| packages/daemon/test/tmux-adapter.test.ts | 1b | partial | crates/crewmux-mux/src/paste.rs, crates/crewmux-mux/tests/tmux_it.rs | Paste sequence and session lifecycle. Guard verdict is slice 4. |
| packages/daemon/test/tmux-argv-exec.test.ts | 1b | partial | crates/crewmux-mux/src/exec.rs | Argv-only exec; a shell binary is rejected before spawn. |
| packages/daemon/test/tmux-exact-target.test.ts | 1b | partial | crates/crewmux-mux/src/argv.rs | Exact `=` targets, including a leading `=`. |
| packages/daemon/test/tmux-session-names-native.test.ts | 1b | partial | crates/crewmux-mux/tests/tmux_it.rs | tmux rewrites `.` and `:` to `_`; targets use the stored name. |
| packages/daemon/test/tmux-session-env-native.test.ts | 1b | partial | crates/crewmux-mux/tests/tmux_it.rs | Session env is passed with `-e`. |
| packages/daemon/test/seat-delivery-guard.test.ts | 4 | todo | | tmux-scope inputs land in slice 1b (`guard_inputs`); the verdict is slice 4. |
| packages/daemon/test/seat-delivery-guard-correction.test.ts | 4 | todo | | Verdict deferred to slice 4. |
| packages/daemon/test/terminal-session-broker.test.ts | 1b | partial | crates/crewmux-mux/src/death.rs | Push `%window-close` / pane-died replaces broker polling. The broker itself is later. |
