> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# ADR-0001: Async runtime: tokio

Status: Accepted (2026-10-11).

## Context
- The daemon serves HTTP over a Unix socket (ADR-0006) and streams events (SSE/WebSocket).
- It owns a long-lived tmux control-mode client (ADR-0005) and spawns and reaps provider and tmux subprocesses.
- It runs timers: the wake ladder, inconclusive budgets, health detectors and compaction waits.
- The CLI, TUI and MCP server are API clients.
- OpenRig (TS) does all of this on the Node event loop.

## Options
1. **tokio, multi-thread runtime.** The de-facto standard; hyper/axum, rmcp (ADR-0002) and reqwest all assume it.
2. async-std or smol. Smaller, but the ecosystem we need is tokio-first, and async-std is discontinued.
3. Threads only, no async. Domain code stays simple, but we would hand-roll an event loop for sockets, SSE, control-mode streams and subprocess I/O.

## Decision
- **tokio with the multi-thread scheduler** in `crewmux-daemon`, `crewmux-api`, `crewmux-mux` (control-mode reader, exec), `crewmux-mcp`, `crewmux-tui` (event pump), `crewmux-cli` and the `rig` binary.
- **Domain crates stay synchronous and pure:** `crewmux-types`, `crewmux-spec`, `crewmux-core`, and the decision functions in `crewmux-health`, `crewmux-orchestration` and `crewmux-transcript`. TS tests port as plain `#[test]`s with an injected clock.
- **Blocking work** runs on dedicated threads or `spawn_blocking`, never on async workers. This covers SQLite on the DB actor thread (ADR-0003) and transcript/rollout file reads.
- **Diagnostics:** `tracing`.
- **Shutdown:** a `tokio_util::sync::CancellationToken` tree from the daemon root. SIGTERM/SIGINT come in via tokio's `signal` feature (slice 1, PR #9). The daemon removes `daemon.sock` and `daemon.pid` on exit.
- **Time is injected** through a `Clock` trait, with a fake clock in tests.

## Consequences
- One runtime across all binaries.
- Each crate declares only the tokio features it needs.
- Async seams are tested with `#[tokio::test]`, FakeMux and the fake clock. Real tmux tests sit behind the `tmux-it` feature.

## Open risks (as recorded)
- **Child reaping:** libutempter-linked tmux on Linux can swallow `SIGCHLD`. Resolved in slice 1b with a rustix `SIGCHLD` nudge sent only to our own `rig-rs*` tmux server (ADR-0005).
- **Timer ordering under load** (wake ladder): fake-clock tests, plus nightly load tests.
