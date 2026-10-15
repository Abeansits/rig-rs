# OpenRig → Rust porting map

## Context & decisions
- **Goal**: a complete Rust rewrite of OpenRig (github.com/mvschwarz/openrig, Apache-2.0, TypeScript/Node, npm `@openrig/cli` v0.6.9). OpenRig runs teams of coding agents (Claude Code, Codex) in tmux sessions, coordinated by a local daemon with SQLite state, a CLI (`rig`), a TUI and an MCP server. Sebastian wants this for daily agent-team work without depending on a third-party Node project. He uses Ghostty + Herdr as his terminal setup.
- **Approach**: complete rewrite rather than incremental migration. No time estimates. Compatibility with Node OpenRig's on-disk/API formats is optional; keep the `rig` command names (skills call them).
- **In scope**: Claude Code + Codex adapters, tmux layer, daemon (HTTP+WS), SQLite, MCP server, queue/ask/send, snapshots/restore/crash-cart, setup/doctor + config writers (~/.claude.json, ~/.codex/config.toml, tmux.conf, hooks), Herdr + Ghostty, TUI, Claude compaction control, workflow engine, scope, review, context packs, health detectors/stream classifier, mission-control, slices, chatroom, and a **new OpenCode runtime adapter**.
- **Out of scope**: legacy React web UI, Slack/human gateway, bundles & publishing, migration helpers, plugins & agent images, cross-host, Docker service rigs & secrets, Pi & Oh My Pi, cmux, legacy compat shims/demos/proof commands, discover/adopt of existing tmux sessions.
- **Size to port**: ~164k LOC of production TypeScript (of 218.5k, excluding the 50k web UI) + ~1.5k new Rust for OpenCode. See "Scope" below.

Source: github.com/mvschwarz/openrig @ HEAD (shallow clone, v0.6.9), Apache-2.0. Measured 2026-10-11 with find/wc/rg (non-test .ts/.tsx, comments included).

## 1. Size
| Package | Prod LOC | Test files | Notes |
|---|---|---|---|
| packages/daemon | 157,827 | 945 | Hono HTTP+WS, SQLite, domain logic, runtime adapters |
| packages/cli | 47,439 | 264 | commander CLI `rig`, MCP server, setup/doctor, cross-host |
| packages/tui | 13,239 | 106 | `openrig-tui`, **hand-rolled ANSI renderer** (no Ink/blessed), only dep `yaml` |
| **Total excl. UI** | **~218,500** | 1,315 | |
| Tests | 367,916 LOC, ~16,200 test cases | | vitest; more test code than prod code |
Other: 374 .md docs (1.7 MB docs/), skills/ 780 KB (shipped in npm package), 124 YAML specs, 93 .mjs build/check scripts.

Counts: 93 CLI command files (~221 `.command()` registrations incl. subcommands); ~328 HTTP route handlers across 69 route files; ~18 MCP tool registrations (cli/src/mcp-server.ts); 99 migration files / ~59 distinct CREATE TABLEs (daemon/src/db).

Daemon breakdown: domain/ 122.9k, routes/ 16.8k, adapters/ 9.3k, db/ 3.0k, middleware 0.6k, terminal 0.6k. CLI: commands/ 31.5k, lib/ 5.7k, rest ~10k at src root.
Largest domain areas: gateway/ (Slack + human gateway) 7.2k, queue-repository 4.1k (+wake-ladder 1.3k), review/ 3.6k, rigspec-instantiator 2.7k, session-transport 2.2k, context-packs 2.2k, mission-control 2.2k, slices 2.1k, scope 2.0k, workspace 2.0k, restore-orchestrator 1.9k (+restore-check 1.6k), policies 1.9k, workflow runtime/projector 3.7k, provider 1.8k, user-settings 1.5k, rigspec-schema 1.3k, skill-catalog 1.2k, seat handover/lifecycle 2.4k, bundle-* (~20 files), claude-compaction-enforcer 1k.

## 2. Subsystems
(LOC = files whose name matches, approx.; keyword-touching code is much wider: "queue" touches 94k lines, "restore" 72k, "hook/trust" ~60k — these are cross-cutting.)

| Subsystem | Key paths | ~LOC | Node deps | Rust crates | Diff. | Gotchas |
|---|---|---|---|---|---|---|
| CLI front door | cli/src/index.ts, commands/, front-door.ts, client.ts, daemon-auto-start/lock | 47k | commander | clap (derive), reqwest/ureq, anyhow, fs2/fd-lock | L (breadth) | 221 cmds; exact flag/JSON output parity matters for skills that shell `rig …` |
| Daemon HTTP+WS | daemon/src/server.ts, routes/, middleware/ | 17k + domain | hono, @hono/node-server, node-ws | axum + tokio + tower, axum ws (tokio-tungstenite) | L | 328 routes; keep URL/JSON contracts identical so TUI/skills/UI interop |
| SQLite state + migrations | daemon/src/db (all-migrations.ts, migrate.ts, migrations/ 99) | 3k + repos | better-sqlite3 (native) | rusqlite (bundled) + rusqlite_migration or own runner; ulid crate | M | Must replay same migration ids against existing ~/.openrig db; WAL/busy_timeout; sync API in TS → keep sync rusqlite on blocking pool |
| Domain core (rigs, seats, sessions, nodes, lifecycle, handover) | domain/rig-*, seat-*, session-*, node-inventory, startup/bootstrap-orchestrator | ~15k | – | serde, thiserror | XL | Largest logic mass; behaviour encoded in 16k tests |
| RigSpec / agent spec schema + validation | rigspec-schema.ts, spec-validation-surface, agent-manifest | ~4k | yaml, zod, ajv(+formats) | serde_yaml (or serde_yml/yaml-rust2 – serde_yaml is archived), schemars + jsonschema, garde/validator | M | zod error messages surface to users; YAML round-trip/comment preservation where specs are rewritten |
| tmux runtime + adapters | adapters/tmux*.ts, terminal-adapter, claude-code-adapter, codex-runtime-adapter, *-resume.ts, non-interruptive.ts, yolo-mode.ts, session-transport | ~12k | child_process | tokio::process, shell-words/shlex; optionally tmux control mode (-CC) parser | XL | send-keys timing, paste-buffer vs literal keys, trust/permission prompt detection at launch, capture-pane scraping, not typing into a pane the human is using, resume ids per provider, Codex sandbox flags |
| Pi / OMP runners | pi-runner*.ts, pi-runtime-adapter, omp-*, stub-runner* | ~2k | node child + JSON-RPC over stdio | tokio + serde_json line-delimited RPC | M | Pi runner may be a Node script itself → spawn `pi` binary, don't embed Node |
| Queue / send / ask / wake / chatroom / claims | queue-repository, queue-wake-ladder, ask-service, chat-repository, claim-service, ask-wake.ts | ~11k | – | rusqlite, tokio timers | L | Wake ladder = timing heuristics; must not race with human typing |
| Workflow runtime / slices / review / mission-control / scope | workflow-*, slices/, review/, mission-control/, scope/ | ~14k | – | – | L | Mostly pure logic; good test-port candidate |
| Snapshots / restore / crash cart / checkpoints | restore-orchestrator, restore-check-service, checkpoint-store, crash-cart-surface, cli restore-packet/ | ~6k core (72k touching) | tar | tar + flate2, sha2 | L | Restore across reboots, partially-dead tmux servers, provider resume semantics |
| Bundles (assembly, archive, integrity, routers for skills/plugins/context packs/agent images) | domain/bundle-*.ts, context-packs/, agent-images/, plugin-discovery-service | ~10k | tar, yaml | tar, flate2, sha2, walkdir, globset | L | Keep archive layout + manifest byte-compatible for interop |
| Provider config/trust/hook writers | provider/, claude-activity-hooks, claude-compaction-enforcer, setup cmds, cmux-config, herdr-launch-config | ~5k core | jsonc-parser, smol-toml | serde_json (+ jsonc-parser crate / json-patch-like minimal edits), toml_edit (format-preserving!) | M-L | ~/.claude.json is huge & concurrently written by Claude Code → atomic write + minimal-edit + backup; ~/.codex/config.toml must preserve comments (toml_edit); .claude/settings.local.json hooks; ~/.tmux.conf managed block markers |
| rig setup / doctor / node-support / ABI check | cli commands setup*, doctor*, node-support.ts, scripts/check-abi.mjs | ~2k named (17k touching setup) | – | which, dialoguer/inquire, semver | M | Node/ABI checks disappear in Rust; doctor gets simpler |
| Herdr / cmux / Ghostty integrations | herdr-launch-config.ts, cmux*.ts, adapters/cmux-transport, ghostty (3 files) | ~3.5k named | – | tokio::process, serde_json, unix sockets (cmux), plist/AppleScript via `osascript` for Ghostty | M | cmux socket protocol; Ghostty has no stable IPC → shell launches; Herdr launch config format |
| MCP server | cli/src/mcp-server.ts (+ uses daemon client) | ~0.5k + 15k touching | @modelcontextprotocol/sdk | rmcp (official Rust SDK) stdio transport | S-M | Tool names/schemas must stay identical (skills reference them) |
| Slack gateway / human registry | domain/gateway/, gateway-slack-surface | ~7k | fetch | reqwest, tokio-tungstenite (Socket Mode), slack-morphism (optional) | M-L | Socket Mode reconnects, signing secrets |
| Cross-host (remote rigs over ssh) | cli/src/cross-host-*, host-registry, remote-host-ops, domain/hosts | ~3k | child_process ssh | openssh crate or spawn `ssh` | M | |
| TUI | packages/tui (hand-rolled ANSI, mouse hit-map, unix socket server) | 13k | yaml | ratatui + crossterm (or keep hand-rolled renderer port; it's already a pure function (state,snapshot)->lines,hitMap) | L | Pixel/line parity with current UI is optional; mouse hit-map semantics |
| Skills / docs / context packs | skills/, docs/, scripts/mirror-skills, generate-context-packs | data | npm `files` | include_dir!/rust-embed or install to ~/.openrig/skills on `rig setup` | S | Skills likely call `rig` CLI → CLI compat is the real contract |

## 3. Node/npm couplings to replace
- **Distribution**: `npm i -g @openrig/cli`, bin-wrapper.ts, `postinstall` check-abi.mjs (better-sqlite3 ABI vs Node 22/24). → single static binary via cargo-dist / Homebrew tap; rusqlite `bundled` removes ABI issue entirely.
- **Bundled assets** in the npm tarball (daemon/, tui/dist, skills/, docs). → rust-embed at build time; `rig setup` extracts skills/context packs into ~/.openrig.
- **Daemon spawned as `node …/daemon/dist/index.js`** by daemon-lifecycle.ts. → same binary, `rig daemon run` subcommand; launchd plist optional.
- **TUI is a separate Node entry (`openrig-tui`)** talking over a socket. → `rig tui` subcommand.
- **Pi/stub runners** spawned as Node scripts with JSON-RPC protocol → reimplement protocol in Rust; stub runner becomes a test fixture binary.
- **Build-time scripts** (93 .mjs: mirror-skills, context-pack generation, docs guards) → build.rs or `xtask`.
- `string-width`, `jsonc-parser`, `smol-toml`, `ulid`, `zod/ajv` → unicode-width, jsonc-parser crate, toml_edit, ulid, schemars+jsonschema.

## 4. Recommended crates
clap 4 (derive) · tokio · axum (+ws) / tower-http · reqwest (rustls) · rusqlite (bundled) + rusqlite_migration · serde/serde_json · serde_yaml-compatible fork (serde_yml or yaml_serde) · toml_edit · jsonc-parser · schemars + jsonschema · tar + flate2 + sha2 · ulid · rmcp · ratatui + crossterm · unicode-width · tracing · thiserror/anyhow · notify (file watching) · tempfile + atomic writes · insta (snapshot tests for CLI/JSON parity) · assert_cmd.

## 5. Web UI
packages/ui (50k LOC React) is out of scope entirely; no HTTP routes need to exist solely for it.

## 6. Format compatibility
Optional. Not required to read Node OpenRig's ~/.openrig db, bundles or API. Reuse RigSpec YAML shape / CLI names only where free (skills shell out to `rig …`, so keeping CLI verbs avoids rewriting skills).

## 7. OpenCode runtime
**Abstraction**: `RuntimeAdapter` in daemon/src/domain/runtime-adapter.ts (181 LOC): `runtime`, `listInstalled`, `project` (skills/files into runtime dirs), `skillTargetPath?`, `deliverStartup`, `launchHarness(binding,{name,resumeToken?,forkSource?})`, `checkReady`. Separate `*ResumeAdapter`s (`adapters/*-resume.ts`). 6 runtime keys wired (claude-code, codex, pi, omp, terminal, stub); see docs/as-built/architecture/adapters-and-runtimes.md.

**Existing adapter sizes (prod LOC)**
| Runtime | Adapter | Resume | Extra domain support | Total |
|---|---|---|---|---|
| Claude Code | claude-code-adapter 1,165 | claude-resume 224 + launch 15 | activity-hooks 89, compaction-enforcer 965, managed-launch 149 | ~2.6k |
| Codex | codex-runtime-adapter 1,543 | codex-resume 225 | thread-id 328, network-default 231, profile-preflight 57, git-add-dirs 48, team-workspace 52, daemon-support 59 | ~2.5k |
| Pi | pi-runtime-adapter 483 | pi-resume 185 | pi-runner 1,028, runner-protocol 323, readiness 88 | ~2.1k |
| OMP | 16 (extends Pi) | 6 | – | 22 |
Shared: tmux.ts 1,321, yolo-mode 104, non-interruptive 43, preflight-exec 46.

**What an adapter must do** and the OpenCode mapping:
| Need | Claude/Codex today | OpenCode |
|---|---|---|
| Launch flags | CLI args in tmux (`--settings`, `-c notice…`, permission modes) | `opencode` TUI in tmux, or `opencode serve` + `opencode attach`/SDK; model/agent via flags or opencode.json |
| Readiness / prompt detection | capture-pane scraping for trust/permission prompts | With `serve`: HTTP health + session API → no scraping (like Pi's structured RPC) |
| Send input | tmux send-keys/paste, non-interruptive guard | POST message to session API (prompt endpoint); keep pane as mirror |
| Resume / fork | claude `--resume <id>`, codex thread id capture | sessions have ids; `--session`/continue flags and session fork via API |
| Hooks / activity relay | Claude hooks in settings.local.json; codex notify | OpenCode plugins (JS/TS event hooks) or SSE event stream from serve → prefer SSE, no plugin needed |
| Usage/context telemetry | transcript/hook parsing | message metadata includes tokens/cost via API |
| Trust/permissions config | ~/.claude.json, ~/.codex/config.toml writes | `permission` block in opencode.json (project or ~/.config/opencode), write via serde_json; per-seat config through env/OPENCODE_CONFIG |
| Skills projection | .claude/skills, AGENTS.md | AGENTS.md + opencode agents/commands dirs |
**Effort**: M. ~1.2–1.8k Rust LOC (adapter ~800, resume ~150, SSE/HTTP client ~400, config writer ~200, readiness ~100). Structured API makes it easier than Claude/Codex (no prompt scraping); model it on the Pi runner pattern (structured events → pane mirror + daemon activity). Verify current opencode server endpoints against its docs/SDK when implementing (API has changed across versions).

## 8. Pi / Oh My Pi
- **Pi** = the `pi` coding agent (pi-coding-agent from Mario Zechner's badlogic/pi-mono; env var `PI_CODING_AGENT_DIR` used). **Oh My Pi (OMP)** = a Pi distribution/fork with its own CLI and state root; `OmpRuntimeAdapter` just extends Pi with different binary, state root, approval semantics, resume-token type.
- **Integration**: daemon launches `node <dist>/adapters/pi-runner.js` in the tmux pane. Runner spawns pi in RPC mode, converts pane stdin → RPC prompt/steer/follow_up, renders typed JSONL events to the pane, POSTs activity/session identity to daemon, writes runner-state.json sidecar; deny-by-default env allowlist. ~2.1k LOC total.
- **Decision**: out of scope. The runner pattern is worth reusing as the design for OpenCode.

## 9. Scope
Section 2's table lists every subsystem as found in the source; the ones marked out of scope here are not ported.

### In scope (prod TS LOC, approx)
| Subsystem | LOC |
|---|---|
| Domain core: rigs, seats, sessions, lifecycle, handover, startup/bootstrap | ~15k |
| tmux layer + Claude Code + Codex adapters, resume, yolo/non-interruptive | ~12k |
| Queue / ask / send / wake ladder / claims | ~11k |
| Chatroom | ~0.8k |
| Snapshots / restore / crash-cart / checkpoints | ~6k core |
| Setup / doctor + provider trust/hook/tmux.conf writers | ~5k core |
| Herdr + Ghostty | ~3k |
| Claude compaction control | ~1k |
| Daemon HTTP + WS routes / middleware | ~17k |
| SQLite db + migrations (fresh schema) | ~3k + repos |
| MCP server | ~0.5k + shared client |
| CLI commands (in-scope verbs) | ~35k |
| TUI | ~13k |
| Workflow engine | ~13k |
| Scope | ~12.5k |
| Review | ~8.1k |
| Context packs | ~5.5k |
| Health detectors / stream classifier | ~5.4k |
| Mission-control | ~4.9k |
| Slices | ~4.5k |
| Stub runtime (test fixture only) | ~1.2k |
| **New**: OpenCode adapter (Rust, see §7) | ~1.2–1.8k |

### Out of scope
| Subsystem | LOC removed |
|---|---|
| Legacy web UI (packages/ui) | 50.4k (not counted in 218.5k) |
| Slack / human gateway | ~20k |
| Bundles & publishing | ~6.7k |
| Migration helpers | ~5.8k |
| Plugins & agent images | ~5.4k |
| Cross-host | ~4.7k |
| Docker service rigs & secrets | ~3.4k |
| Discover / adopt | ~2.8k |
| cmux | ~2.5k |
| Pi & Oh My Pi | ~2.1k |
| Legacy compat shims / demos / proof commands | ~1.5k |
| **Total removed** | **~55k** |

**Total to port**: 218.5k − ~55k ≈ **~164k LOC of production TypeScript** (±15%: subsystem counts overlap and cross-cutting code is shared), plus ~1.5k new Rust for OpenCode. The 368k LOC of tests are reference material for behaviour; port them selectively.

## 10. Build stages (no dates)
1. **Foundation**: Cargo workspace (core, db, specs, runtime-tmux, daemon, cli, mcp, tui, integrations); fresh SQLite schema + migration runner; RigSpec YAML parse/validate; tmux layer.
2. **Runtimes**: Claude Code and Codex adapters (launch, trust/permission prompt handling, send, capture, resume/fork, hooks/activity relay), stub runtime for tests.
3. **Daemon + CLI core**: axum routes + websockets; `rig daemon`, `up/down/ps/send/capture/attach`, queue/ask/broadcast, chatroom; MCP server.
4. **Durability + setup**: snapshots/restore/crash-cart, Claude compaction control, `rig setup` / `rig doctor` with format-preserving config writers, Herdr + Ghostty launch.
5. **Work orchestration**: workflow engine, scope, review, slices, mission-control, context packs.
6. **Observability + UI**: health detectors / stream classifier, TUI (ratatui).
7. **OpenCode**: adapter on `opencode serve` (structured API, event stream, opencode.json permissions).
8. **Full parity pass**: remaining in-scope CLI verbs and routes, selective test port, single-binary distribution (cargo-dist/Homebrew).

## 11. Apache-2.0 obligations for a port
A Rust translation is a derivative work. You must: include the Apache-2.0 LICENSE; carry forward any NOTICE file (repo has no NOTICE at HEAD — still add your own NOTICE crediting "Portions derived from OpenRig, © mvschwarz / contributors, Apache-2.0"); retain copyright/attribution notices from source files you translate; state prominently in modified/derived files that they were changed (a header like "Derived from openrig packages/daemon/src/…; ported to Rust"); don't use "OpenRig" trademarks/name to imply endorsement (§6) — pick a different product name. You may license your own additions differently, but the derived parts stay under Apache-2.0 terms. Not legal advice.
