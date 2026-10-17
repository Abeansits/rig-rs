> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# Slice 0: Workspace skeleton, toolchain, CI, daemon stub

Depends on nothing. Merged as PR #1 (scaffolding) and PR #2 (QA CI). The daemon parts landed in slice 1 (PR #9).

## Scope
- A cargo workspace with library crates, all `crewmux-*`: types, spec, core, store, mux, transcript, provider, adapters, health, messaging, orchestration, durability, integrations, api, daemon, mcp, tui, cli.
- The `rig` binary and `xtask`.
- Apache-2.0 headers and the license.
- CI for fmt, clippy, test and lints.
- A daemon `run`/`status`/`stop` stub over the UDS with `/healthz` (ADR-0006).

## Acceptance criteria
- **AC1:** `cargo build --workspace` and `cargo test --workspace` pass on macOS arm64/x86_64 and Linux x86_64 (G6 targets).
- **AC2:** The toolchain is pinned. **The repo pins 1.99.0 (`rust-version = "1.99"`, PR #1). The design said MSRV 1.89, and the repo value governs.**
- **AC3:** `cargo fmt --check` and `cargo clippy --workspace --all-targets -D warnings` are clean in CI.
- **AC4:** Crate dependency direction is enforced by xtask:
  - domain crates (types/spec/core) depend on no I/O crates;
  - `crewmux-mcp` depends only on api/spec/types (lint `mcp-state-path`, added in slice 6).
- **AC5:** `rig --version` and `rig --help` work. Their output subset is compared to OpenRig in 0b (repo PE-003).
- **AC6:** The state dir name is a single constant `STATE_DIR_NAME = ".rig-rs"` (G4), with no other literal (lint).
- **AC7:** The daemon stub:
  - `rig daemon run|status|stop` over `$RIG_HOME/run/daemon.sock` (0600, `run/` 0700);
  - `GET /healthz` returns version and pid (repo PE-009);
  - stop removes the socket and pid file.
  - **Deferred from PR #1 and delivered in slice 1 (PR #9).**
- **AC8:** `cargo xtask check-headers` verifies the SPDX/Apache header in every source file. **Deferred in PR #1. As of main `1a9b193` there is no `check-headers` xtask (subcommands: `diff`, `ledger`, `lint-mux`, `lint-blind-enter`, `lint`, `record-tui`), so AC8 is still open.**
- **AC9:** No network or real-HOME access in tests. Tests use a scratch HOME.
- **AC10:** No import from Node OpenRig state (G7).

## Out of scope
Any provider, tmux or DB behaviour (slices 1/1b+).
