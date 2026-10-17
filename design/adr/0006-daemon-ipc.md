> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# ADR-0006: Daemon IPC: HTTP/1.1 + JSON over a Unix domain socket

Status: Accepted (2026-10-11). G8 closed by Sebastian.

## Context
- OpenRig's CLI, TUI and MCP server talk to the daemon over HTTP on localhost TCP.
- The port wants the same request/response shape, so the diff harness can compare both sides. It also wants filesystem-permission security, so a token is not needed by default.

## Options
1. **HTTP/1.1 + JSON over a UDS** (axum server; `crewmux-api::client` is a small blocking HTTP/1.1 client over `std::os::unix::net::UnixStream`, with no hyper dependency).
2. gRPC/tonic over a UDS. A schema toolchain, and harder to diff against OpenRig.
3. Custom length-prefixed JSON-RPC. Small, but no streaming conventions and no curl debuggability.

## Decision
- **Transport:** HTTP/1.1 + JSON at **`$RIG_HOME/run/daemon.sock`**, mode 0600. `run/` is 0700.
  - The bind path is checked against `sun_path` length, with a clear error that suggests a shorter `RIG_HOME`.
  - A stale socket is removed only when no live daemon answers.
- **Client:** `crewmux-api::client` is the only client, used by the CLI, TUI and MCP.
- **Auto-start:** commands that need the daemon start it under `flock(run/daemon.lock)` and then poll `/healthz`. This makes parallel `rig up` safe (slice 1, PR #9).
- **Health:** `GET /healthz` returns version and pid. Its shape differs from OpenRig's, as recorded in repo **PE-009**.
- **Streams:** `GET /api/events` with cursor and gap semantics (SSE/WebSocket). **Slice 4's event endpoint is canonical**; slice 13's minimal one was reconciled into it. Chat history is a page, not a stream (repo PE-012).
- **Errors:** JSON `{ "error": { "code", "message" } }` with stable snake_case codes, e.g. `invalid_cwd`, `already_running`, `not_implemented`.
- **G8 (Sebastian):**
  - **No token on the UDS.** Filesystem permissions are the boundary.
  - **Opt-in TCP** (for remote or harness use) **requires a bearer token** and binds loopback by default.
  - The 0b harness talks to rig-rs over the UDS only (`rs_transport: tcp` is rejected) and to OpenRig over TCP.
- **Seat identity on MCP-originated calls:** `X-Rig-Seat-*` headers (slice 6). The chat seat token deviation is repo PE-011. MCP route and flag deviations are repo PE-020.

## Consequences
- Routes are versionless under `/api/…`, and the diff harness normalises transport details.
- Daemon-side MCP route and flag differences are recorded as PE-020, not silently.

## Open risks
- `sun_path` limits with deep `RIG_HOME`s (guarded).
- If the server framework is swapped, the error envelope must stay byte-stable for the harness.
