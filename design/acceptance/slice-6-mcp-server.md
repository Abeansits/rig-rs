# Slice 6: MCP server (`rig mcp`, server name `rig`, 15 renamed tools)
Source of truth: design README. ADR-0002 (rmcp + rename table), ADR-0006 (daemon API over UDS), ADR-0008 (no `~/.claude.json` writes), ADR-0010 (dialog tables/gates). PEs (repo numbering): **PE-016** (MCP tool rename; design PE-003), PE-004 (no runtime Claude user-config writes), PE-013 (no blind Enter). Spike S4 (`spikes/6-config-writes.md` + Mac rerun: rmcp **2.2.0**, 18 placeholder tools, both CLIs connected and called `seat_send`). PLAN 2.15. TS sources: `packages/cli/src/mcp-server.ts` (18 registrations), `packages/cli/test/mcp-server.test.ts`, `packages/cli/src/client.ts`.

## Scope
- **`crewmux-mcp`:** an rmcp **3.x** server over **stdio**, launched as `rig mcp` (a subcommand of the one `rig` binary). The server name in `initialize` is `rig`.
- **Tools:** exactly the **15** ADR-0002 names:
  - rig: `rig_start`, `rig_stop`, `rig_list`
  - agent: `agent_list`, `agent_add`, `agent_send`, `agent_read_screen`
  - chat: `chat_send`, `chat_read`
  - snapshot: `snapshot_create`, `snapshot_list`, `snapshot_restore`
  - spec: `spec_validate_rig`, `spec_validate_agent`
  - daemon: `daemon_health`
  No aliases for old names.
- **Every tool is a thin adapter over `crewmux-api::Client` (UDS).** No tool opens SQLite, tmux, or a provider file. The daemon API is the only path for state changes (ADR-0006, CLI linking rule). `spec_validate_*` may call `crewmux-spec` directly because it is pure.
- **Schemas:** argument structs derive `schemars::JsonSchema`. A schema post-processor (`schema::compat`) applies the fallback transform, **off by default** (behind `RIG_MCP_SCHEMA_COMPAT=1` and a per-client config flag):
  - `$defs`/`$ref` inlined;
  - `type: [T, "null"]` rewritten to `T` + not-required;
  - `anyOf` with null collapsed.
  S4 showed both CLIs accept the raw draft-2020-12 output, so the transform exists only for a client that rejects it.
- **Registration per seat** (writes owned by `crewmux-provider`, reused here):
  - **Claude:** `--mcp-config <seat>/mcp.json --strict-mcp-config` with `{"mcpServers":{"rig":{"command":"<abs rig>","args":["mcp"],"env":{RIG_SEAT_ID,RIG_SEAT_TOKEN,RIG_HOME}}}}`, plus `--allowedTools mcp__rig__<tool>` for the 15 names.
  - **Codex:** per launch `-c mcp_servers.rig.command="<abs rig>" -c mcp_servers.rig.args=["mcp"] -c mcp_servers.rig.env.RIG_SEAT_ID=…` (and the token/home keys). **`config.toml` is never edited at launch.**
- **Codex approval dialog:** `CODEX_DIALOGS` row `CodexMcpToolApproval` → `Choose{"Allow for this session", Gate::AutoApproveRigTools}`. `AnswerPolicy.auto_approve_rig_tools` **defaults to `false` until Sebastian decides D1**, so the dialog escalates (`ReadinessCode::McpToolApproval`). When it is enabled, the gate also requires `server == RIG_MCP_NAME` and `tool ∈ TOOL_NAMES`.
- **Caller identity:** tools that act as a seat (`agent_send`, `chat_send`) forward `RIG_SEAT_ID`/`RIG_SEAT_TOKEN` headers (repo PE-011 model). Outside a seat they act as `operator`.

## Non-goals
- MCP over HTTP/SSE.
- Resources, prompts and sampling.
- The 3 dropped tools (discover, bind, bundle_inspect).
- Writing `~/.claude.json` or the user's global Codex `config.toml` (setup-time Codex edits stay in slice 7).
- Deciding D1. This slice only wires the gate.

## Crates touched
`crewmux-mcp` (new: `server`, `tools/*`, `schema`, `names`), `crewmux-api` (client methods the tools need, already used by the CLI), `crewmux-provider::{claude,codex}` (MCP registration fragments), `crewmux-adapters::{claude,codex}` (argv), `crewmux-adapters::dialog` (gate already defined by ADR-0010), `rig` bin (`mcp` subcommand), `xtask` (lints, golden).

## Public types / traits
```rust
// crewmux-mcp::names
pub const RIG_MCP_NAME: &str = "rig";                 // re-exported by crewmux-provider (PR #16 already imports it)
pub const TOOL_NAMES: [&str; 15] = ["rig_start", "rig_stop", "rig_list", "agent_list", "agent_add", "agent_send",
    "agent_read_screen", "chat_send", "chat_read", "snapshot_create", "snapshot_list", "snapshot_restore",
    "spec_validate_rig", "spec_validate_agent", "daemon_health"];
pub const LEGACY_NAMES: [&str; 18] = [/* the 18 OpenRig names, exact */];
pub fn is_rig_tool(name: &str) -> bool;

// crewmux-mcp::server
pub struct RigMcp { api: crewmux_api::Client, caller: Caller }
pub enum Caller { Seat { id: SeatId, token: SeatToken }, Operator }
impl RigMcp { pub fn from_env() -> Result<Self, Error>; }
// #[tool_router] impl RigMcp { #[tool(name = "agent_send", description = "...")] async fn agent_send(&self, Parameters(a): Parameters<AgentSendArgs>) -> Result<CallToolResult, McpError>; … }
pub async fn serve_stdio(server: RigMcp) -> Result<(), Error>;

// crewmux-mcp::schema
pub enum SchemaMode { Native, Compat }
pub fn compat(schema: serde_json::Value) -> serde_json::Value;   // inline $defs/$ref, nullable arrays → optional, anyOf-null collapse

// crewmux-provider
pub fn claude_mcp_json(rig_bin: &Path, env: &SeatEnv) -> serde_json::Value;
pub fn claude_allowed_tools() -> Vec<String>;                     // "mcp__rig__<tool>" × 15
pub fn codex_mcp_overrides(rig_bin: &Path, env: &SeatEnv) -> Vec<String>; // ["-c", "mcp_servers.rig.command=…", …]
```

## Acceptance criteria
**Tool surface**
1. `tools/list` returns exactly the 15 names in `TOOL_NAMES`, with descriptions and input schemas. It **equals** the golden `crates/crewmux-mcp/tests/golden/tools-list.json` (sorted, stable key order). Any change needs a golden update in the same PR.
2. No legacy name is registered. `tools/call` with `rig_send` → MCP "unknown tool" error.
3. Every tool's behaviour (arguments, result shape, error shape) matches the TS assertion it replaces, via the ADR-0002 old→new map. `xtask diff` maps names before comparing (`ExpectedDiff(PE-016)` only on the name field).
4. Each tool calls exactly one `crewmux-api` endpoint, or `crewmux-spec` for validation. A test with a fake UDS server asserts the request list per tool. A dependency lint asserts `crewmux-mcp` does not depend on `crewmux-store`, `crewmux-mux`, `crewmux-adapters` or `crewmux-durability`.
5. Daemon down: tools other than `spec_validate_*` return a structured error (`daemon_unavailable`) and do **not** auto-start the daemon from inside an agent. `daemon_health` reports `running: false`.

**Schemas**
6. Native mode: schemas are schemars draft-2020-12 output, unchanged.
7. Compat mode: `compat()` output contains no `$defs`, `$ref`, `anyOf` with null, or `type` arrays. A round-trip test checks that every golden example argument object still validates against the transformed schema. The transform is idempotent.

**Registration**
8. The Claude seat argv contains `--mcp-config <seat>/mcp.json --strict-mcp-config` and `--allowedTools` with exactly the 15 `mcp__rig__*` names. `mcp.json` has a single server, `rig`.
9. The Codex seat argv contains the `-c mcp_servers.rig.*` overrides (`command`, `args`, `env.*`). The user's `config.toml` is byte-identical before and after launch (checksum test, fake `CODEX_HOME`).
10. No file under the fake HOME's `.claude*` is written (PE-004).

**Codex approval (PE-013 / ADR-0010)**
11. With the default policy (`auto_approve_rig_tools = false`), the real capture `codex-106-s6-approval.txt`, with its tool renamed to `agent_send`, → `Escalated(McpToolApproval)`, and no key is sent.
12. With `auto_approve_rig_tools = true`, the same capture → the driver selects "Allow for this session" with cursor verification. A foreign server (`server ≠ rig`) or a tool not in `TOOL_NAMES` (e.g. the spike's `seat_send`) → escalate. "Always allow" and "Allow" are never chosen.

**Lints**
13. `cargo xtask lint mcp-legacy-names` fails if any of the 18 `LEGACY_NAMES` appears in shipped skills, docs, or `crates/**/src` outside `names.rs` and the diff-harness map.
14. `cargo xtask lint mcp-state-path` fails if `crewmux-mcp` gains a dependency on a state crate (AC4).

## Required tests
`tools_list_golden.rs` (AC1–2), `tool_requests.rs` (AC4, fake UDS), `daemon_down.rs` (AC5), `schema_compat.rs` (AC6–7), `claude_registration.rs` and `codex_registration.rs` (AC8–10), `codex_mcp_approval.rs` (AC11–12, real capture fixture), `xtask/tests/lint_mcp_names.rs` (AC13–14). The ported `mcp-server.test.ts` rows are `diverged` + PE-016.

**Live (gated `RIG_LIVE_CLAUDE=1` / `RIG_LIVE_CODEX=1`; S4 redone on rmcp 3.x):**
- **L1:** Claude `/mcp` lists `rig` with 15 tools, and the model calls `agent_send` (the transcript has `tool_use` for `mcp__rig__agent_send`).
- **L2:** Codex `/mcp` shows `rig: connected (15 tools)`. With the default policy, the call surfaces as `McpToolApproval` attention. With the gate enabled (D1), it completes and the rollout has an `McpToolCall` for `agent_send`.
- **L3:** if either client rejects the native schemas, re-run with compat mode and record which client needed it.

## CI vs gated
CI: everything except L1–L3 (no provider binaries; rmcp in-process client for `tools/list`/`tools/call`). Gated: L1–L3 on Sebastian's Mac. The S4 evidence is from rmcp 2.2.0, so **the tool-list check on 3.x is open until L1/L2 pass**.

## Definition of done
- CI is green and the golden is committed.
- Ledger rows are `ported` or `diverged` (PE-016).
- L1 and L2 have passed once on rmcp 3.x with the default (escalate) policy.
- The D1 outcome is recorded in ADR-0002, and the default flips only if Sebastian approves.

## Dependencies
- Slice 4: `agent_send` → queue/send API.
- Slice 5: chat.
- Slice 8: the snapshot tools return `not_implemented` until it lands, and that is listed in the golden description.
- Slices 2 and 3: argv and the dialog tables.
- Slice 7: provider writers.
- ADR-0010 merge (PR #15/#16).
