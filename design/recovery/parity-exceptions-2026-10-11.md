# Parity exceptions
These are deliberate behavioural differences from OpenRig v0.6.9, where matching TS behaviour would reproduce a bug. Each entry has a required test. The parity ledger (PLAN.md §2.1) marks the affected TS tests `waived` and links them here. The differential harness (`xtask diff`) lists these IDs as expected diffs.

## PE-001: Claude first-run / login screens must not be classified as ready
- **TS behaviour:** `packages/daemon/src/domain/native-resume-probe.ts` `assessNativeResumeProbe`, used by `claude-code-adapter.ts` `checkReady` (l.362). `looksLikeClaudeTui` matches `/(^|\n)\s*❯/` plus `"Claude Code v"`. The first-run banner "Welcome to Claude Code v2.1.296" and the menu cursor `❯` satisfy both, so the theme picker and "Select login method:" are classified as **`resumed / active_runtime` (ready)**. `looksLikeClaudeLoginPrompt` only knows the in-session "Not logged in" + "Run /login" text. Spike S13 (report: spikes/3-gate-detection.md) reproduced this by running the verbatim TS probe on fixtures cl-0, cl-1 and cl-2.
- **Impact:** on a fresh or logged-out machine, startup context is pasted into a menu. Digits in the payload can even select a menu option.
- **Rust behaviour:**
  - "Ready" requires a **positive composer match**: the Claude input box border plus the mode/footer line. It is never banner + `❯`.
  - New gate codes: `OnboardingTheme`, and `LoginRequired` for "Select login method".
  - **Any numbered-menu screen (`❯ 1.` / `  2.` pattern) that matches no known gate is `AttentionRequired(UnknownMenu)`, never ready.**
- **Required test:** `rig-adapters/tests/claude_gates.rs::first_run_screens_are_not_ready`. It loads fixtures `claude/cl-0-theme.txt`, `cl-1-login-method.txt` and `cl-2-login-method-opt2.txt` (copied from spikes/scratch/3-gate-detection/fixtures). It asserts that `classify(..)` returns `OnboardingTheme`, `LoginRequired` and `LoginRequired` respectively, with `requires_attention() == true` and `!is_ready()`. A second test asserts that no delivery (paste) is issued to a seat in these states, using FakeMux to assert zero paste calls. Both are CI-blocking.
- **Mac rerun (2026-10-11), confirmed on real screens:**
  - Claude Code 2.1.286, theme picker with an empty `CLAUDE_CONFIG_DIR`: 4/4 captures were classified **ready** by the verbatim TS probe (`spikes/scratch/mac-rerun/classify.txt`).
  - Two more false-ready screens were found:
    - **"New MCP server found in this project: …"** (TS only knows the old text "new MCP servers found in .mcp.json"): `captures/claude-055/056-mcpgate-after-trust.txt`.
    - **"Try the new fullscreen renderer?"** (a new popup, absent from TS): `captures/claude-007…010`, 7 captures.
  - Positive ready fixtures now exist (`claude-020-ready.txt`, resize and after-turn sets).
  - **Rust rule:** ready = composer plus a footer containing `? for shortcuts`, with **no** `esc to interrupt` and **no** option menu (`❯`/`›` cursor lines or a numbered list) on screen.
- **Additional required tests:** `claude_gates.rs::{mcp_server_found_is_attention, fullscreen_renderer_popup_is_attention, ready_requires_footer_and_no_menu, midturn_is_busy_not_ready}` over the real Mac captures listed above (copied to `tests/fixtures/screens/claude/2.1.286/`).

## PE-002: Codex login screens must be detected explicitly (TS hangs until timeout)
- **TS behaviour:** `codex-runtime-adapter.ts` `checkReady` (l.483) has no login matcher apart from token-refresh failure text. "Sign in with ChatGPT" / "Provide your own API key" (fixtures cx-1/2/3) classify as `inconclusive / awaiting_runtime`, and the launch waits until the readiness timeout. A missing CODEX_HOME (cx-0) is also inconclusive, though it should be a hard failure.
- **Rust behaviour** (the Codex login detection design, implemented in `adapters::codex::screen` and `adapters::codex::preflight`):
  - `LoginRequired` when the screen matches the sign-in menu ("Sign in with ChatGPT", "Provide your own API key", or the API-key entry field).
  - `HardFailure(CodexHomeMissing)` for the CODEX_HOME error, which is not retried.
  - **Pre-flight before launch:** `codex login status` (exit code / output) runs under the seat's CODEX_HOME. If it reports not logged in, the launch is short-circuited to `LoginRequired` without starting the TUI. The screen matcher is the second line of defence.
  - **Inconclusive budget:** an `Inconclusive` verdict that persists for `inconclusive_budget` (default 15 s, configurable) while the screen is *stable* (same hash across 3 captures) escalates to `AttentionRequired(UnrecognisedScreen)` and attaches the capture as evidence. It does not wait for the full launch timeout. This applies to all runtimes.
- **Required tests:** `codex_gates.rs::login_screens_are_login_required` (cx-1, cx-2, cx-3 give `LoginRequired`; cx-0 gives `HardFailure`); `codex_launch.rs::not_logged_in_preflight_short_circuits` (stub `codex` binary on PATH printing not-logged-in; asserts no tmux session is created and the seat state is `attention_required/login_required`); `readiness.rs::stable_inconclusive_escalates_before_timeout` (FakeMux plus a fake Clock).
- **Mac rerun (2026-10-11), corrected reading of the evidence:**
  - The spike report lists Codex "Sign in with ChatGPT (4)". The captures it cites (`captures/codex-000…003-firstrun.txt`) actually show a different first-run menu: **"Cannot use the background server / Experimental feature request failed"**, with options "1. Run without daemon this time" and "› 2. Cancel" (the default).
  - The TS probe classified these `inconclusive/awaiting_runtime`, which would hang until timeout. The *hang* finding is confirmed. The **sign-in screens themselves were not captured on the Mac**, so the box fixtures cx-1/2/3 (codex-cli 0.162.1) remain the evidence for login detection.
  - The background-server menu is a new screen, which Rust classifies as `UnknownMenu` → attention (fixture test `codex_gates.rs::background_server_menu_is_attention`).
  - Also found on the Mac: **"Allow the rig MCP server to run tool …"** (`codex-106-s6-approval.txt`) was classified `awaiting_runtime`. It must be `AttentionRequired(McpToolApproval)`, or handled under PE-006 (`codex_gates.rs::mcp_tool_approval_is_attention`).
  - The `codex login status` output and exit codes were unverified at this point, because the Mac CLI was logged in throughout. **Resolved by the logged-out corpus below.**
- **Logged-out corpus (box, codex-cli 0.162.1, 2026-10-11; `spikes/scratch/3b-codex-logged-out/captures/cx-lo-00…53`, `classify.txt`):**
  - **Parity divergence, recorded:** OpenRig's verbatim checker labelled **all 46 live logged-out screens** `inconclusive / awaiting_runtime` (still waiting): the sign-in menu at 4 sizes, option highlights, the ChatGPT browser flow, the device-code flow and API-key entry. **None** were labelled login-required. The 8 dead panes (Ctrl-C, `q`, `login status`, `exec`, missing CODEX_HOME) were correctly `failed / returned_to_shell`. Rust deliberately diverges: these are a distinct **`LoginRequired(kind)`** state, and the ledger marks the TS readiness tests that assume "waiting" as `diverged` + PE-002.
  - **Explicit state:** `SeatState::LoginRequired { kind: LoginKind, evidence }`, with `LoginKind = Menu | ChatGptBrowser | DeviceCode | ApiKeyEntry | PreflightNotLoggedIn`. It is a terminal launch outcome, separate from `AttentionRequired(UnknownMenu)` and from `Inconclusive`. It surfaces in `rig ps`, the TUI attention view and `rig doctor` with the remedy "run `codex login` in a terminal". **We never drive the login.**
  - **Pre-flight (primary):** `codex login status` under the seat's `HOME`/`CODEX_HOME`. **Exit 1 with "Not logged in" → `LoginRequired(PreflightNotLoggedIn)`, and no tmux session is created.** "Error finding codex home… does not exist" (exit 1) → `HardFailure(CodexHomeMissing)`. Exit 0 → launch. Any other exit/output → launch but flag `preflight_unrecognised` in doctor.
  - **Screen matchers (second line, all positive):**
    - Menu = "Sign in with ChatGPT" **and** "Provide your own API key";
    - ChatGptBrowser = "Finish signing in via your browser";
    - DeviceCode = "Enter this one-time code";
    - ApiKeyEntry = "Paste or type your API key".
    There is **no trust prompt before login** (git repo, `resume` and `--no-daemon` all go straight to the menu), so the login check runs first.
  - **Required fixture tests** (`tests/fixtures/screens/codex/0.162.1-logged-out/`, OAuth query and device code already redacted):
    - `codex_login.rs::all_live_logged_out_screens_are_login_required` iterates `index.json`. The 46 live captures → `LoginRequired(expected kind)`; the 8 dead panes → `ReturnedToShell`/`HardFailure`. **None** may be `Ready`, `Inconclusive` or `UnknownMenu`.
    - `::preflight_exit1_short_circuits`, using the real captured `cx-lo-45-login-status.txt` text and exit 1 through a stub binary.
    - `::codex_home_missing_is_hard_failure` (`cx-lo-47`).
    - `::no_keys_sent_while_login_required` (FakeMux: zero `send_keys`/paste).
  - **Risk, unreproduced false-ready with stale background servers:** in a discarded first attempt (shared HOME, stale app-server daemons), a relaunch after "app-server control socket is already in use" showed the **normal composer while logged out**. drive3.py's stale-daemon relaunches (`cx-lo-49…53`) did not reproduce it.
    **Mitigation:**
    1. The pre-flight is authoritative. A seat whose pre-flight said not-logged-in is never marked `Ready`, whatever the screen shows.
    2. Pre-flight also runs on every relaunch/resume, not just first launch.
    3. If the screen shows the "control socket is already in use" text, or the app-server crash text, the seat → `AttentionRequired(StaleCodexServer)` and doctor lists stale codex app-server processes for the seat's `CODEX_HOME`. **We never kill them automatically.**
    4. After `Ready`, the first submit's transcript verification (ADR-0009) must see a rollout user item. A composer that accepts input but writes no rollout within the submit timeout → `AttentionRequired(NotVerified)`.
    5. A targeted repro is tracked in spike-list (S13 follow-up).

## PE-003: MCP tool names renamed (deliberate, decided by Sebastian 2026-10-11)
- **TS behaviour:** 18 tools named `rig_*` (`cli/src/mcp-server.ts`), including the awkward `rig_rig_nodes` / `rig_rig_validate`.
- **Rust behaviour:** the 15 in-scope tools use the `<object>_<action>` scheme and the rename table in [ADR-0002](adr/0002-mcp-crate.md#rename-table). The 3 out-of-scope tools (`rig_discover`, `rig_bind`, `rig_bundle_inspect`) are not registered, and no aliases are kept for the old names. Arguments, results and error shapes keep TS parity, except where a later PE says otherwise.
- **Ledger:** rows for `cli/test/mcp-server.test.ts` are `diverged` + `PE-003` for name assertions; behavioural cases are `ported`.
- **Required tests:**
  - `rig-mcp/tests/tools_list.rs::names_match_adr_0002`: an insta golden of `tools/list`. It asserts the exact 15 names, that every name matches `^(rig|agent|chat|snapshot|spec|daemon)_(list|start|stop|add|send|read|create|restore|validate|health)(_[a-z]+)?$`, and that none of the 18 legacy names in the ADR-0002 table is present. Note that the new `rig_start`/`rig_stop`/`rig_list` legitimately share the `rig_` prefix.
  - `rig-mcp/tests/rename_parity.rs`: for each table row, the diff harness calls TS's old name and Rust's new name with the same arguments, and the normalised results are equal. The verdict is `ExpectedDiff(PE-003)` only on the name.
  - The `xtask` lint that none of the 18 legacy names (an exact list taken from the ADR-0002 table) appears in shipped skills/docs.

## PE-004: No runtime writes to `~/.claude.json` (deliberate)
- **TS behaviour:** `provisionClaudeState` (claude-code-adapter.ts l.747+) rewrites `~/.claude.json` for trust and MCP whenever the trust flag is missing, even while Claude is running.
- **Rust behaviour:** per ADR-0008. Per-seat MCP via `--mcp-config --strict-mcp-config`; settings and hooks via `--settings`; trust is pre-accepted only when no Claude process runs, and otherwise surfaces as `Attention(TrustGate)`.
- **Evidence:** S6 concurrency test. Even compare-and-swap with verify erased ~18% of the concurrent writer's updates.
- **Required tests:** `rig-provider/tests/claude_guard.rs::refuses_while_claude_running` (a fake process list); `rig-adapters/tests/claude_launch_argv.rs::uses_flags_not_global_config` (the launch argv contains `--mcp-config`, `--strict-mcp-config` and `--settings`, and `~/.claude.json`'s mtime and inode are unchanged under HermeticHome); and a lint that no code path outside `rig-provider::claude::trust` opens `.claude.json` for writing.

## PE-005: Exactly-once re-delivery on restore for direct sends and startup files
- **Scope:** direct `rig send` messages and startup-file deliveries **only** (ADR-0009 Amendment 1). **Queue rows are parity, not part of this PE.** On restore they get only the pointer nudge, as in OpenRig's wake ladder.
- **TS behaviour:** to be confirmed in slice 8, using `restore-orchestrator.ts`, `session-transport.ts` and the startup-delivery paths. S9 (`spikes/5-resume-restore.md`) showed that naive "re-send un-acked" duplicates content (3 of 18) when a delivery lands before its ack is recorded.
- **Rust behaviour:** each direct/startup delivery stores a text-hash fingerprint (`sha256(normalize(body))`), and nothing is injected into prompts. On restore, a delivery with `delivered_at` set and no ack is re-sent **only if** the seat's transcript has no matching user turn after the delivery cursor; if a match is found, it is acked. Identical bodies are counted by occurrence, and contaminated turns escalate. Acks come only from transcript evidence.
- **Outcome of the slice 8 check:**
  - If TS re-sends direct/startup deliveries without reconciliation, keep this PE and ledger the asserting tests as `diverged`.
  - If TS never re-sends them, keep it as an "added reconciliation" PE.
  - If TS already reconciles, delete this PE and mark those rows `ported`.
- **Required tests:** `rig-durability/tests/restore_reconcile.rs::{no_duplicates_after_crash_between_paste_and_ack, resends_when_fingerprint_absent, identical_bodies_counted_by_occurrence, contaminated_turn_escalates}` (direct sends and startup files); `normalize.rs`; and `queue_rows_only_renudged` (asserts that restore never pastes a queue item body: **parity, not part of this PE**).

## PE-006: Never send a blind Enter to a provider dialog (TS auto-drives trust with Enter)
- **TS behaviour, verified:** `packages/daemon/src/adapters/claude-code-adapter.ts` lines 463–470. When `autoDriveProviderPrompts && probe.code === "trust_gate"`, it calls `this.tmux.sendKeys(tmuxSession, ["Enter"])`, sleeps 200 ms and loops. It does not move the cursor and does not check which option is selected.
- **Evidence (Mac rerun, Claude Code 2.1.286 / codex-cli 0.162.0):** default options on real dialogs:
  | Dialog | Default (cursor) option | Effect of a blind Enter | Capture |
  |---|---|---|---|
  | Claude trust "Accessing workspace…" | `❯ No, exit` | **exits Claude** (observed in run 1) | `claude-006-trust.txt`, `captures-run1-enter-declined-trust/` |
  | Claude bypass "WARNING … Bypass Permissions" | `❯ No, exit` | exits Claude | `claude-050/051-bypass-after-trust.txt` |
  | Claude "New MCP server found" | `❯ Continue without using this MCP server` | silently runs without the server | `claude-055/056` |
  | Claude "Try the new fullscreen renderer?" | `❯ 1. Yes, try it` | **switches the renderer** (changes the UI we scrape) | `claude-007…010` |
  | Codex "Update available 0.162.0 → 0.162.1" | `› 1. Update now (runs brew upgrade --cask codex)` | **runs a software update** | `codex-005…` (e.g. resize sets) |
  | Codex "Cannot use the background server" | `› 2. Cancel` | cancels the launch | `codex-000…003-firstrun.txt` |
  | Codex "Allow the rig MCP server to run tool …" | `› 1. Allow` | approves a tool call without review | `codex-106-s6-approval.txt` |
  So TS's trust auto-drive exits Claude on current versions (on these defaults), and a blind Enter on the Codex update gate would run `brew upgrade`.
- **Rust behaviour:**
  1. **No code path sends `Enter` (or `y`, or a digit) to a pane whose classifier result is not `Ready`**, except through `DialogDriver::answer`. A lint and a unit test enforce this: `send_keys([Enter])` from adapters goes through `SeatIo::submit`, which requires `Ready`.
  2. **Prefer avoiding the dialog** (ADR-0008):
     - trust is pre-accepted only when no Claude process runs;
     - per-seat `--mcp-config --strict-mcp-config` (no project `.mcp.json` prompt);
     - the bypass posture is chosen explicitly via flags and settings;
     - the fullscreen-renderer and update prompts are suppressed via per-seat settings or config **if** the CLI offers a setting (to verify per version; record which keys work in the slice 2/3 specs);
     - otherwise they are answered as below.
  3. **`DialogDriver::answer(dialog, choice)` only for known dialogs with a positively identified option.** The classifier must return a specific `KnownDialog` variant (matched on its title text **and** the full option list). The driver then:
     - locates the target option line by its exact text;
     - moves the cursor with Up/Down;
     - re-captures and verifies that the cursor marker (`❯`/`›`) is on that exact line;
     - only then sends Enter;
     - re-classifies, and expects the dialog gone.
     If any step fails, it escalates. Digit shortcuts are used only where the option is numbered **and** the number-to-text mapping was verified in the same capture.
  3a. **Forbidden keystrokes outside `Ready`:** besides Enter, never send `q`, `Ctrl-C`, `Esc`, digits or `y`/`n` to a non-`Ready` pane, except as the explicit, verified action of a known dialog. Evidence: on the Codex sign-in menu, **`q` and Ctrl-C quit Codex (exit 0)** (`cx-lo-35/36/38`), and digits pick a login method (`cx-lo-39/40`). Daemon shutdown and seat stop use the tmux kill path, never Ctrl-C or `q` into the pane. `lint-blind-enter` also covers `C-c`, `q` and digit literals in adapter `send_keys` calls. Test: `dialog_driver.rs::no_quit_keys_on_login_menu`.
  4. **Allowed automatic answers** (everything else escalates to `AttentionRequired`):
     - Claude trust → "Yes, I trust this folder", **only** when the RigSpec opts in (`auto_trust: true`) and ADR-0008 pre-accept was skipped because Claude was running.
     - Claude fullscreen renderer → "Not now".
     - Codex update → "Skip" (or Esc, which the spike used), **never "Update now"**.
     - Codex MCP approval → see the slice 3 / ADR-0002 decision (only server `rig` + a tool in our 15-tool list → "Allow for this session"; otherwise escalate). This is pending Sebastian.
     - **Never auto-answered:** bypass consent, Claude "New MCP server found" (should not appear, given `--strict-mcp-config`), login screens, the background-server menu, and any unknown menu.
- **Required tests** (fixtures from `spikes/scratch/mac-rerun/captures/`, copied per CLI version):
  - `dialog_driver.rs::never_enter_on_default_no_exit`: FakeMux replays `claude-006-trust.txt`; asserts that the first key sent is `Down` (not `Enter`), and that `Enter` is sent only after a re-capture shows `❯ Yes, I trust this folder`.
  - `::trust_not_answered_without_opt_in`.
  - `::codex_update_answers_skip_never_update` (asserts no keystroke ever selects option 1).
  - `::bypass_always_escalates`.
  - `::unknown_menu_escalates`.
  - `::cursor_verification_failure_escalates` (FakeMux moves the cursor to the wrong line).
  - `lint-blind-enter` (adapters may not call `send_keys` with Enter outside `SeatIo::submit` and `DialogDriver`).
- **Ledger:** the TS tests asserting trust auto-drive via Enter are `diverged` + PE-006.

## PE-007: Compaction completion is confirmed by hooks, not inferred by timing
- **TS behaviour:** `precompact-hook.mjs` writes a seat-keyed marker (`restore-pending/<sessionKey>.json`, last writer wins) on PreCompact, and the daemon/bridge (`.expected.json` sentinel, `compaction-restore-bridge.cjs`) infers completion from the marker plus waits (post-compact send wait 10 s).
- **Evidence (S2, Claude Code 2.1.286):**
  - Claude now emits `SessionStart{source=compact}` and `PostCompact{compact_summary}` on completion.
  - A `PreCompact{trigger=auto}` fired **without any compaction following**, and it overwrote the managed marker (`managedRefocusPending: true → false`).
- **Rust behaviour:**
  - The restore trigger is **PostCompact + SessionStart:compact** (same session_id), then `Ready`.
  - Markers are per attempt, with a `confirmed` flag set only on completion.
  - An unsolicited PreCompact never touches a pending managed marker. An unconfirmed marker is never acted on.
  - Time alone never triggers a restore.
  - The `.expected.json` sentinel and the bridge script are not ported.
- **Required tests:** slice 9 AC6–AC10, incl. `stray_auto_precompact_does_not_touch_managed_marker.rs` built from `hooks.jsonl`.
- **Ledger:** TS tests asserting sentinel/bridge or timing-based restore → `diverged` + PE-007. Prompt text and threshold tests remain `ported`.

## Status of real-login testing (updated 2026-10-11, Mac rerun)
**Done** (Sebastian's Mac, Claude Code 2.1.286 + codex-cli 0.162.0, real logins):
- S1a composer delivery (25 + 25 sends, transcript-verified).
- S13 gate corpus (67 Claude + 108 Codex captures) and launch-to-ready timings.
- S9 resume after kill-server (Claude `--resume`, Codex `resume <thread id>`).
- S4 MCP acceptance in both clients.
- **Codex logged-out screens and `codex login status`** (54 captures, codex-cli 0.162.1, box; PE-002).
- **S2 Claude hooks and manual compaction, plus resume** (slice 9 L1/L2; PE-007).

Also pending: **genuine threshold-driven auto-compaction** (slice 9 L3, gated) and a repro of the stale-server false-ready (PE-002 risk).

**Still pending:**
- The guard with a human typing in a real pane on the Mac (the box result stands).
- OpenCode model turns (no provider; slice 10).
- The Ghostty launch (the window opens but `command` doesn't run on 1.3.1; slice 11).
