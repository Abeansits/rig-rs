# Slice 3: Codex seat
Source of truth: design README. ADR-0005, ADR-0008, ADR-0009 (Am. 2), PE-002, PE-005, PE-006. Spikes S1a, S3 (answered by S9), S4, S13: Mac rerun 2026-10-11 (`spikes/scratch/mac-rerun/`) and the logged-out corpus (`spikes/scratch/3b-codex-logged-out/`, 54 captures, codex-cli 0.162.1). PLAN 2.6. TS sources: `daemon/src/adapters/codex-adapter.ts`, `resume-metadata-refresher.ts` (thread-id scrape at line 161), `codex-login-preflight` paths, `assessNativeResumeProbe`.

## Scope
- `CodexAdapter: AgentAdapter`:
  - login preflight (PE-002);
  - launch with per-seat MCP registration;
  - classify until `Ready`;
  - submit;
  - thread-id capture from rollout metadata;
  - `resume <thread-id>`.
- Gate classifier v2 (`rig-adapters::codex::screen`), positive matches only.
- Codex `KnownDialog` variants and their allowed answers (PE-006).
- Seat approval posture (decision below, pending Sebastian's confirmation).

## Non-goals
Codex hooks/notify relay (not needed for the thread id), MCP tool bodies (slice 6), editing the user's global `~/.codex/config.toml` at launch (slice 7 owns `rig setup` edits; per-seat registration uses `-c` overrides).

## Crates touched
`rig-adapters` (`codex`, `dialog`), `rig-provider::codex`, `rig-durability` (rollout `TranscriptProbe`), `rig-types`; fixtures `tests/fixtures/screens/codex/0.162.0/` and `tests/fixtures/transcripts/codex/`.

## Public types / traits
```rust
pub enum CodexScreen { Ready(ComposerState), Busy, Dialog(KnownDialog), Login(LoginKind), StaleServer, UnknownMenu { evidence: String }, Inconclusive }
pub enum LoginKind { Menu, ChatGptBrowser, DeviceCode, ApiKeyEntry, PreflightNotLoggedIn }
pub enum Preflight { LoggedIn, NotLoggedIn /* exit 1 "Not logged in" */, CodexHomeMissing /* exit 1 */, Unrecognised { code: i32, out: String } }
pub fn codex_preflight(home: &Path, codex_home: &Path) -> Preflight;   // runs `codex login status`
// SeatState gains LoginRequired { kind: LoginKind, evidence: String } (distinct from AttentionRequired and Inconclusive)
// KnownDialog additions:
//   CodexUpdate { from: String, to: String }                // options: Update now / Skip / Skip until next version
//   CodexMcpToolApproval { server: String, tool: String }   // options: Allow / Allow for this session / Always allow / Cancel
//   CodexBackgroundServer                                   // "Cannot use the background server" (never auto-answered)
pub struct RolloutLocator { pub sessions_root: PathBuf /* $CODEX_HOME/sessions */ }
impl RolloutLocator {
    pub fn snapshot(&self) -> RolloutSet;                                    // before launch
    pub fn resolve(&self, before: &RolloutSet, cwd: &Path, launched_at: SystemTime) -> Result<ThreadId, ThreadIdError>;
}
pub enum ThreadIdError { NotYet, Ambiguous(Vec<PathBuf>), Malformed(PathBuf) }
pub enum CodexApproval { Never, OnRequest, Untrusted }   // maps to `-a`
```

## Thread id (S9 Mac rerun)
- **Source:** `session_meta.payload.id` from the first line of `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`. It also appears in the filename. No pane scraping (replaces `resume-metadata-refresher.ts:161`).
- **Rule:** take the rollout whose `payload.cwd` equals the seat's canonicalised cwd, newest first.
- **Ambiguity risk:** two seats sharing a cwd, or a human running `codex` in the same folder, would both match "newest for cwd". Disambiguation, in order:
  1. **Before-set difference:** `snapshot()` before exec; only files **not in the snapshot** and with `session_meta.timestamp ≥ launched_at` are candidates.
  2. **Per-cwd launch lock:** the daemon serialises Codex launches per `(CODEX_HOME, cwd)` and holds the lock until the id resolves (or the launch times out). Two rig seats therefore can't race each other.
  3. **First-message correlation:** if more than one candidate remains (an outside `codex` started concurrently), wait for the seat's first submitted body and pick the rollout whose first user item matches its ADR-0009 fingerprint.
  4. Still more than one → `ThreadIdError::Ambiguous` → the seat is flagged `AttentionRequired(ThreadIdAmbiguous)`. **Never guess.**
  5. The resolved id is persisted and re-verified on resume (the file still exists, and its cwd matches).
- Per-seat `CODEX_HOME` would remove the ambiguity entirely, but it splits auth and history from the user's install. It is not adopted (noted as an option for Sebastian).

## Approval posture (S4 Mac rerun) — proposed, needs Sebastian
- **Evidence:**
  - `-a never` makes Codex **refuse** MCP calls ("requires approval, but approval policy is never").
  - The default policy shows "Allow the rig MCP server to run tool …" (options 1 Allow / 2 Allow for this session / 3 Always allow / 4 Cancel; cursor on 1).
- **Proposal:**
  - Seats launch with the RigSpec's `approval` field (default `on-request`), never `never`.
  - The daemon auto-answers `CodexMcpToolApproval` **only if** `server == "rig"` **and** `tool` is one of the 15 ADR-0002 names, choosing "**2. Allow for this session**" via cursor-verified selection (PE-006).
  - "Always allow" is never chosen, because it persists a choice into the user's Codex state (ADR-0008 spirit).
  - Any other server or tool → `AttentionRequired(McpToolApproval)` on the human seat.
  - The spec can disable the auto-answer (`auto_approve_rig_tools: false`).
- If a Codex config key for per-MCP-server/tool approval exists on the pinned version (to verify; not tested), prefer passing it as a per-seat `-c` override over answering the dialog.

## Launch timings (Mac, codex-cli 0.162.0) → timeouts
| Observed | Value |
|---|---|
| Update gate appears | ≈0.48 s (every launch until skipped) |
| Esc on the gate → composer | 1.51–1.53 s (5 launches) |
| Harness `new-session` → ready, incl. dwell on the gate | 10.13–10.17 s (`s3-codex-timings.json`) |
| No trust prompt for fresh non-git folders | (0.162.0) |
| Model turn | p50 3.6–4.1 s, max 6.7 s |

`LaunchBudget::codex()`:
- `poll` 100 ms;
- `first_screen` 5 s;
- `ready_after_last_gate` 10 s (≈6× observed);
- `inconclusive_stable` 15 s → attention;
- `overall` 45 s;
- thread-id resolve 10 s after the first submit, else `NotYet` → attention.

## Acceptance criteria
1. Real update-gate captures → `Dialog(CodexUpdate{from:"0.162.0",to:"0.162.1"})`. The answer is **Skip** (cursor-verified) or Esc. A test asserts that no key sequence ever leaves the cursor on, or selects, "Update now".
2. `codex-000…003-firstrun` ("Cannot use the background server") → `Dialog(CodexBackgroundServer)` → attention. It is never answered, and must not be classified `Inconclusive` until timeout (PE-002).
3. **Login-required (PE-002, divergence from TS, where all 46 live screens read as still-waiting):**
   - **(a) Pre-flight:** `codex login status` runs under the seat's HOME/CODEX_HOME before every launch and resume. Exit 1 "Not logged in" → `SeatState::LoginRequired(PreflightNotLoggedIn)`, and **no tmux session is created**. A missing-CODEX_HOME error → `HardFailure(CodexHomeMissing)`.
   - **(b) Screen fallback:** every one of the 46 live captures `cx-lo-*` → `Login(kind)` within one poll, with the kind from `index.json`. The 8 dead-pane captures → `ReturnedToShell`/`HardFailure`.
   - **(c)** No key is ever sent while in `LoginRequired`; `q`/Ctrl-C/digits quit or choose (PE-006 3a).
   - **(d)** The seat is never `Ready` after a not-logged-in pre-flight, even if a composer is visible. This mitigates the unreproduced stale-server false-ready.
   - **(e)** "control socket is already in use" → `StaleServer` → `AttentionRequired(StaleCodexServer)`. Doctor lists stale app-servers; we never kill them.
4. `codex-106-s6-approval` → `Dialog(CodexMcpToolApproval{server:"rig",tool:"seat_send"})`. Under the proposal it is **escalated**, because `seat_send` is not one of our 15 names (the spike server's name). The same capture with the tool name rewritten to `agent_send` → auto-answered "Allow for this session".
5. Ready/after-turn/resize captures → `Ready`. Midturn (`esc to interrupt`, e.g. `codex-040`) → `Busy`, even though `? for shortcuts` is also shown.
6. Thread id resolves from the rollout metadata. Required temp-dir tests:
   - two seats in the same cwd launched back-to-back (the lock serialises them, giving distinct ids);
   - an outside rollout created concurrently (resolved by first-message correlation);
   - an unresolvable tie (`Ambiguous` → attention);
   - a rollout from before the snapshot is never chosen.
7. `resume <thread-id>` appends to the same rollout file (S9 evidence: 1093863 → 1108724 B).
8. Submit verification: rollout `response_item` role=user text equals the sent body byte-for-byte (`normalize_for(Codex)` = identity), including 64 KB.
9. MCP registration per seat uses `-c 'mcp_servers.rig.command=…'` (+ args/env). The user's `config.toml` is unchanged after a launch (checksum).

## Required tests
`codex_screen.rs` (labelled corpus, every file labelled); `codex_gates.rs::{background_server_menu_is_attention, mcp_tool_approval_is_attention, update_answers_skip_never_update}`; `rollout_locator.rs` (AC6); `normalize_real.rs` (Codex rows); `launch_argv.rs`; the PE-006 `dialog_driver.rs` set. `codex_login.rs::{all_live_logged_out_screens_are_login_required, preflight_exit1_short_circuits, codex_home_missing_is_hard_failure, no_keys_sent_while_login_required, preflight_not_logged_in_never_ready_even_with_composer, stale_server_text_is_attention}`, with fixtures from `3b-codex-logged-out/captures` (redacted). **Live (gated `RIG_LIVE_CODEX=1`):** launch + gate skip, MCP call through the auto-answer, kill-server + resume. A logged-out pre-flight against a throwaway CODEX_HOME runs in CI on the box when `codex` is installed (no login needed).

## CI vs gated
CI: all fixture and temp-dir tests. Gated: the live set. Launch, resume and MCP are covered by Mac evidence. The logged-out screens are covered by the box corpus (done).

## Definition of done
CI is green. Ledger rows are `ported`, or `diverged` with PE-002/PE-006. The approval posture is confirmed by Sebastian, and the live set has passed once.

## Dependencies
Slices 1, 1b; the slice 7 codex writer (for `rig setup` only). Live runs need Sebastian's Codex login (available).
