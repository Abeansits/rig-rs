> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# ADR-0002: MCP server crate (rmcp) and tool names

Status: Accepted. Tool names were renamed by Sebastian's decision on 2026-10-11, and the rename is recorded as **repo PE-016** (design PE-003).

## Context
- OpenRig ships an MCP server (`packages/cli/src/mcp-server.ts`) with 18 tools, so that agents in seats can drive rigs.
- The Rust port needs a maintained MCP server SDK with stdio transport and JSON Schema for tool arguments.

## Options
1. **rmcp**, the official Rust SDK (modelcontextprotocol/rust-sdk): stdio transport, `#[tool]` macros, schemars integration.
2. Community crates (e.g. mcp-sdk-rs, rust-mcp-sdk). They are less maintained or lag the spec.
3. Hand-rolled JSON-RPC. It is small, but we would own protocol churn.

## Decision
- **rmcp, pinned to the 3.x line**, stdio transport only. The binary entry is `rig mcp`, intercepted before clap (slice 6, PR #24). The `initialize` server name is **`rig`**.
- Argument structs derive `schemars::JsonSchema`. Native draft-2020-12 output is used as is. A **compat transform** is available behind a flag but off by default: it inlines `$defs`/`$ref`, rewrites nullable `type` arrays to optional, and collapses `anyOf`-null.
- **Evidence:** spike S4, on rmcp 2.2.0. Claude listed and called a tool via `--mcp-config --strict-mcp-config --allowedTools`. Codex connected via `-c mcp_servers.rig.*` and needed an approval answer, while `-a never` refuses MCP calls. Both accepted the native schemas. Slice 6 repeats the check on 3.x.
- **Every tool is a thin client of the daemon API** (`crewmux-api` over the UDS, ADR-0006). The exception is `spec_validate_*`, which calls `crewmux-spec` directly. `crewmux-mcp` must not depend on state crates (`cargo xtask lint mcp-state-path`).

## Tool names (decision 2026-10-11)
The scheme is `<object>_<action>`: snake_case, ASCII, ≤ 32 chars, no `rig_` prefix for non-rig objects. Clients already namespace tools by server (`mcp__rig__agent_send`).

| OpenRig | rig-rs | Purpose |
|---|---|---|
| `rig_up` | `rig_start` | Start a rig from a RigSpec |
| `rig_down` | `rig_stop` | Stop a rig and its agents |
| `rig_ps` | `rig_list` | List rigs with status |
| `rig_rig_nodes` | `agent_list` | List a rig's agents with session, state, attach/resume commands |
| `rig_add` | `agent_add` | Add one agent to a running rig |
| `rig_send` | `agent_send` | Send a message (safe paste, human-typing guard) |
| `rig_capture` | `agent_read_screen` | Return an agent's current screen |
| `rig_chatroom_send` | `chat_send` | Post to the rig chatroom |
| `rig_chatroom_watch` | `chat_read` | Recent chatroom messages (history, not a stream) |
| `rig_snapshot_create` | `snapshot_create` | Create a snapshot |
| `rig_snapshot_list` | `snapshot_list` | List snapshots |
| `rig_restore` | `snapshot_restore` | Restore from a snapshot |
| `rig_rig_validate` | `spec_validate_rig` | Validate RigSpec YAML with line/col |
| `rig_agent_validate` | `spec_validate_agent` | Validate AgentSpec YAML with line/col |
| `rig_status` | `daemon_health` | Daemon running/healthy, version |
| `rig_discover`, `rig_bind`, `rig_bundle_inspect` | not ported | Out of scope (discover/adopt, bundles) |

That leaves **15 tools**.
- There are **no aliases** for old names.
- CLI verbs are unchanged.
- `cargo xtask lint mcp-legacy-names` fails if any of the 18 legacy names appears outside `crewmux-mcp/src/names.rs` and the diff-harness map.
- The ledger marks `mcp-server.test.ts` rows `diverged` + PE-016. The diff harness maps old to new names before comparing.

## Registration and approvals
- **Claude (per seat):** `--mcp-config <seat>/mcp.json --strict-mcp-config` plus `--allowedTools mcp__rig__<tool>` × 15 (ADR-0008, PE-004).
- **Codex (per launch):** `-c mcp_servers.rig.*` overrides. The user's `config.toml` is never edited at launch.
- The Codex "Allow the rig MCP server to run tool …" dialog is answered only by the shared dialog driver (ADR-0010) under `Gate::AutoApproveRigTools` (server `rig` + a name in the 15). **The default is escalate, pending Sebastian's D1.** "Always allow" is never chosen.

## Consequences / open risks
- rmcp is pre-1.0-style and fast-moving, so the minor version is pinned and `tools/list` is golden-tested.
- MCP-related route and flag deviations in the daemon are recorded as repo PE-020 (slice 6).
