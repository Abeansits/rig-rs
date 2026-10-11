# AGENTS.md: rig-rs (Rust port of OpenRig, working name)

## Before you finish ANY task, run and pass:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
Do not report done with failing checks. Do not silence lints with `#[allow]`; if unavoidable
use `#[expect(lint, reason = "...")]`.

## Hard rules
- No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `dbg!`, slice indexing in non-test code.
  Use `?`, `.get()`, `.ok_or(Error::...)`, pattern matching.
- No `unsafe` (forbidden workspace-wide).
- No new dependency without stating why in the PR. Add it with `cargo add`, never type a version
  from memory, then hoist it into `[workspace.dependencies]`. Check the API on docs.rs for the
  version in `Cargo.lock`. Commit `Cargo.lock`.
- Errors: each lib crate gets an `error.rs` with a `thiserror` enum. `anyhow` only in binaries
  (`crates/rig`, `xtask`). No `String` errors, no `Box<dyn Error>` in lib APIs.
- IDs and names are newtypes from `crewmux-types`. Statuses are enums. Never pass raw `&str` IDs across crates.
- Async: never call blocking code (std::fs, rusqlite, std::process, thread::sleep) on a tokio worker.
  SQLite goes through `crewmux-store` (single writer). Never hold a lock guard across `.await`.
  Every spawned task is owned (JoinSet or TaskTracker) and takes a CancellationToken.
- Don't `.clone()` to satisfy the borrow checker without thinking.
- Prefer concrete types. Add a trait only for a real seam (runtime adapters, multiplexer, clock, fs-for-tests).
- Files <= 400 lines (CI will hard-fail at 800). One concept per module.
- User-facing output only through `crewmux-cli`'s output module. Logging via `tracing` with structured fields.
- Crate names use the working name `crewmux-*` (per workspace-layout.md); expect a rename.
- Do not use "OpenRig" in crate names, binary names or branding; only in attribution text.

## Attribution (Apache-2.0)
Every file translated from an OpenRig TypeScript source starts with:
    // Derived from openrig packages/<path>.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
Keep any copyright lines from the source file. See NOTICE.

## Layout (see the design's workspace-layout.md)
- `crewmux-types`: ids, enums, DTOs. `crewmux-spec`: RigSpec YAML. `crewmux-core`: pure domain + ports.
- `crewmux-store` (SQLite), `crewmux-mux` (Multiplexer/tmux), `crewmux-adapters` (AgentAdapter),
  `crewmux-provider` (config writers), `crewmux-integrations` (Herdr/Ghostty).
- `crewmux-messaging`, `crewmux-durability`, `crewmux-orchestration`, `crewmux-health`: domain services.
- `crewmux-api`: typed daemon contract + client. `crewmux-daemon`: the only composition root.
- Clients `crewmux-cli`, `crewmux-tui`, `crewmux-mcp` talk to the daemon through `crewmux-api` only
  (`crewmux-cli` may also use `crewmux-spec`, `crewmux-provider`, `crewmux-integrations`).
- `crates/rig`: the single `rig` binary. `xtask`: repo automation (`cargo xtask`).
- `crewmux-core`, `crewmux-messaging`, `crewmux-orchestration`, `crewmux-health`, `crewmux-spec`: no tokio runtime,
  no rusqlite, no process spawning. Lower crates never depend on higher ones.

## Tests
- Behavior spec = ported OpenRig tests in `crates/*/tests/`. When porting a TS module, port its tests first.
- Snapshots: `insta`; review with `cargo insta review`; never blindly accept.
- CLI: `assert_cmd` against a temp `RIG_HOME`. Never touch the real `~` in tests.
- tmux: unit tests use the fake multiplexer; integration tests use a private `-L` socket.
- Parsers (YAML specs, stream classifier, prompt detection): add `proptest`.

## When stuck
Read the original TS at the pinned OpenRig v0.6.9 source. Ask rather than invent behavior.
