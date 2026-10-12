# Parity exceptions
These are deliberate behavioural differences from OpenRig v0.6.9, where matching TS behaviour would reproduce a bug. Each entry has a required test. The parity ledger (PLAN.md §2.1) marks the affected TS tests `waived` and links them here. The differential harness (`xtask diff`) lists these IDs as expected diffs.

ADR-0010 makes this file the single PE register. Design-workspace ids that collided with repo ids PE-003…PE-012 are renamed here:

| Design id | Topic | Repo id |
| --- | --- | --- |
| PE-001 | Claude first-run is not ready | PE-001 |
| PE-002 | Codex login | PE-002 |
| PE-003 | MCP tool rename | PE-016 (reserved; no heading yet) |
| PE-004 | no writes to the Claude user config | PE-004 |
| PE-005 | exactly-once direct/startup re-delivery | PE-017 (reserved; no heading yet) |
| PE-006 | blind Enter | PE-013 |
| PE-007 | compaction completion via hooks | PE-018 |

Repo PE-006 stays the setup/doctor exception below. It is not the blind-Enter rule.

## PE-001: Claude first-run / login screens must not be classified as ready
- **TS behaviour:** `packages/daemon/src/domain/native-resume-probe.ts` `assessNativeResumeProbe`, used by `claude-code-adapter.ts` `checkReady` (l.362). `looksLikeClaudeTui` matches `/(^|\n)\s*❯/` plus `"Claude Code v"`. The first-run banner "Welcome to Claude Code v2.1.296" and the menu cursor `❯` satisfy both, so the theme picker and "Select login method:" are classified as **`resumed / active_runtime` (ready)**. `looksLikeClaudeLoginPrompt` only knows the in-session "Not logged in" + "Run /login" text. Spike S13 (report: spikes/3-gate-detection.md) reproduced this by running the verbatim TS probe on fixtures cl-0, cl-1 and cl-2.
- **Impact:** on a fresh or logged-out machine, startup context is pasted into a menu. Digits in the payload can even select a menu option.
- **Rust behaviour:**
  - "Ready" requires a **positive composer match**: the Claude input box border plus the mode/footer line. It is never banner + `❯`.
  - New gate codes: `OnboardingTheme`, and `LoginRequired` for "Select login method".
  - **Any numbered-menu screen (`❯ 1.` / `  2.` pattern) that matches no known gate is `AttentionRequired(UnknownMenu)`, never ready.**
- **Required test:** `crewmux-adapters/tests/claude_gates.rs::first_run_screens_are_not_ready`. It loads `2.1.286/claude-000-firstrun.txt` and two inline login screens. It asserts `OnboardingTheme` and `LoginRequired`, with `requires_attention() == true` and `!is_ready()`. The same test asserts zero paste and zero Enter on those screens. `splash_without_footer_is_not_ready` covers the splash. `draft_composer_is_attention`, `spinner_without_interrupt_is_busy`, `working_on_it_is_not_ready`, and `live_tui_with_draft_is_not_ready` cover the draft and spinner cases. All are CI-blocking.
- **Mac rerun (2026-10-11, Claude Code 2.1.286):** a splash with the banner and a bare `❯` but no `? for shortcuts` footer is inconclusive, not ready. TS `assessNativeResumeProbe` classifies that splash as resumed. The Rust classifier is a positive match (footer, border, empty prompt, no menu, no interrupt), not a port of that probe.
- A non-empty composer is `PromptDraft` attention. That includes `│ > half typed draft │`, a multi-line draft, and a live TUI whose `❯` line already holds text (`Working on a task.`). TS treats some of those as resumed or as unknown. Rust fails closed and does not paste.
- A spinner is `Busy` even without `esc to interrupt`: braille anywhere, or a line that starts with `✶` `✽` `✢` `✻` `✳`, or a line that starts with `· ` and contains `…`. A completed turn is a spinner glyph, one or more words (Unicode letters included), ` for `, and a duration of hours, minutes, and/or seconds (`2m` and `1h 2m` included). A line containing `· done` is not a spinner. A live row that still has `…` stays busy. A newer unindented line ends an older live row, so `✻ Onioning…` above `● Ready.` with an empty composer is ready. A mid-line `·` in a footer (`· /effort`, `· ← for agents`) is not a spinner. `Working on it…` with no footer is busy, not resumed.
- An empty composer between box-drawing rules is ready, including an accept-edits or edit-approval footer, only when every line between the prompt and the closing rule is blank. A continuation under an empty `❯` is `PromptDraft`. `edit_approval_footer_is_ready` and `empty_accept_edits_composers_are_ready` assert the empty frames. A non-empty composer on that footer stays `PromptDraft`.
- The 2.1.286 fixtures are byte copies of the Mac rerun captures. The 2.1.289 fixtures are the OpenRig pane strings. `labels.jsonl` records the sha256 of each file. They are not rebuilt screens.

## PE-002: Codex login screens must be detected explicitly (TS hangs until timeout)
- **TS behaviour:** `codex-runtime-adapter.ts` `checkReady` (l.483) has no login matcher apart from token-refresh failure text. "Sign in with ChatGPT" / "Provide your own API key" (fixtures cx-1/2/3) classify as `inconclusive / awaiting_runtime`, and the launch waits until the readiness timeout. A missing CODEX_HOME (cx-0) is also inconclusive, though it should be a hard failure.
- **Rust behaviour** (the Codex login detection design, implemented in `adapters::codex::screen` and `adapters::codex::preflight`):
  - `LoginRequired` when the screen matches the sign-in menu ("Sign in with ChatGPT", "Provide your own API key", or the API-key entry field).
  - `HardFailure(CodexHomeMissing)` for the CODEX_HOME error, which is not retried.
  - **Pre-flight before launch:** `codex login status` (exit code / output) runs under the seat's CODEX_HOME. If it reports not logged in, the launch is short-circuited to `LoginRequired` without starting the TUI. The screen matcher is the second line of defence.
  - **Inconclusive budget:** an `Inconclusive` verdict that persists for `inconclusive_budget` (default 15 s, configurable) while the screen is *stable* (same hash across 3 captures) escalates to `AttentionRequired(UnrecognisedScreen)` and attaches the capture as evidence. It does not wait for the full launch timeout. This applies to all runtimes.
- **Required tests:** `codex_login.rs::all_live_logged_out_screens_are_login_required` (live captures are `LoginRequired(kind)`; dead panes are `ReturnedToShell` or `HardFailure`); `codex_login.rs::preflight_exit1_short_circuits` (stub `codex` printing "Not logged in" on exit 1 creates no tmux session); `codex_login.rs::codex_home_missing_is_hard_failure`; `readiness.rs::stable_inconclusive_escalates_before_timeout` (FakeMux plus a fake Clock).
- **Logged-out corpus:** exit 1 with "Not logged in" is `SeatState::LoginRequired { kind: PreflightNotLoggedIn }`. A missing Codex home is `HardFailure(CodexHomeMissing)`. A not-logged-in pre-flight is never `Ready`, even when a composer is on screen. "control socket is already in use" is `AttentionRequired(StaleCodexServer)`. The seat is not driven: no Enter, `q`, Ctrl-C, or digits.

## PE-003: slice 1 `rig` help and version are a subset of v0.6.9

- **Covers:** stdout
- **TS behaviour:** `rig --help` lists the v0.6.9 command names (`add`, `adopt`, `agent`, … `workspace`). `rig --version` prints `0.6.9` and exits 0.
- **Rust behaviour today:** the merged CLI prints clap help for `attach`, `capture`, `chatroom`, `doctor`, `down`, `help`, `ps`, `setup`, and `up`, and `rig --version` prints `rig 0.0.0`. The help-name projection is those names sorted. The exit status still matches, which is why the `version` case (exit code only) is Equal. The tmux shim records the same `tmux -V` argv on both sides.
- **Snapshot:** `tests/diff/snapshots/help-top.rs.snap` and `tests/diff/snapshots/scenario-runner-probe.rs.snap` record that Rust side. `ExpectedDiff(PE-003)` applies only while the live Rust observation still matches that snapshot. Exit status, stderr, JSON, files, and tmux argv are compared as ordinary fields. A different exit code, a stderr panic, or an extra or missing tmux call is `Unexpected`.
- **Why this is recorded:** these are real diffs from a local `cargo xtask diff` against OpenRig v0.6.9 (`4b48ca21`). They are not a behaviour we intend to keep, and they are not hidden. Delete this heading, and the two `.rs.snap` files, when the CLI prints the same command names and `0.6.9` version text. The harness then reports `stale exception` because the Rust output no longer matches the snapshot (or the two sides are equal).
- **Required test:** `cargo xtask diff` against the pinned tree reports `ExpectedDiff(PE-003)` for `help-top` and `scenario-runner-probe`, and `Equal` for `version`. `excepted_exit_code_is_unexpected`, `excepted_stderr_panic_is_unexpected`, and `excepted_extra_tmux_call_is_unexpected` each return `Unexpected`. `changed_rust_snapshot_is_stale` reports `stale exception`.

## PE-004: No runtime writes to the Claude user config while Claude is running

- **TS behaviour:** `provisionClaudeState` (`claude-code-adapter.ts` around the trust rewrite) rewrites the Claude user config for trust and MCP whenever the trust flag is missing, including while Claude is running.
- **Rust behaviour:** per ADR-0008. Per-seat MCP goes through `--mcp-config` and `--strict-mcp-config`. Settings and hooks go through `--settings`. Trust is pre-accepted only when the process probe reports no Claude pid. Otherwise `pre_accept` returns `SkippedClaudeRunning` and does not open the file.
- **Evidence:** the config-writes spike. Compare-and-swap with verify still dropped a fraction of a concurrent writer's updates, which is why the guard exists.
- **Required test:** `crewmux-provider/tests/claude_guard.rs::refuses_while_claude_running`. `cargo xtask lint claude-json` fails when any non-test Rust file outside `crates/crewmux-provider/src/claude/trust.rs` names that user-config basename.

## PE-005: the stub script is `RIG_STUB_SCRIPT` key=value, not a JSON step list

- **TS behaviour:** `packages/daemon/src/adapters/stub-script.ts` runs a JSON list of steps.
- **Rust behaviour:** the slice 1 acceptance script is comma-separated `ready_after_ms`, `exit_after_ms`, and `print_gate=<code>`. The fixture still prints `stub> ` and echoes submitted lines.
- **Required test:** `crewmux_adapters::screen_tests::script_parses_gate_and_timers` and `gate_surfaces_as_attention`.

## PE-006: setup and doctor reports differ from v0.6.9

- **Covers:** stdout
- **TS behaviour:** `rig setup --dry-run --json` prints a step list whose ids include install and auth steps (`brew`, `tmux_install`, `herdr_install`, `claude_auth`, and the rest of `setup.ts`). `rig doctor --help` documents the v0.6.9 doctor, including node, ui, and cmux checks.
- **Rust behaviour:** `rig setup` prints a `SetupPlan` (`write`, `extract`, `offer_install`, `skip`). Node, ABI, ui_dist, cmux, and daemon_dist checks are not part of `rig doctor`. Install steps stay offers unless `--yes` is passed. Herdr stays attach-only.
- **Required tests:** `crates/rig/tests/setup_flow.rs` and `crates/rig/tests/doctor_json.rs`.
- **Snapshot:** `tests/diff/snapshots/setup-dry-run.rs.snap` and `tests/diff/snapshots/doctor-help.rs.snap`. `ExpectedDiff(PE-006)` applies only while those Rust observations still match.

## PE-007: slice 1 stub up, ps, capture, and down speak the acceptance JSON

- **Covers:** stdout, exit
- **TS behaviour:** `rig up` of the slice 1 document (`name` plus a `seats` list) exits 1. v0.6.9 reports `seats` as an unknown key and refuses the file. `rig ps --json` then prints `[]`. `rig capture <seat> --json` reports the session missing or tmux unreachable. `rig down --json` without a rig argument is `commander.missingArgument`. stderr stays empty. The cases record no tmux argv, and that list is empty on both sides.
- **Rust behaviour:** the same document is the slice 1 spec. `rig up --json` auto-starts the daemon and prints `{status, rig, seats, events}` with both seats `ready`. `rig ps --json` lists rig, seat, runtime, state, tmux session, and `attached: false`. `rig capture --json` returns the screen text, including `stub> `. `rig down --json` prints `{stopped, sessions_killed, events}`. The process exits 0.
- **Snapshot:** `tests/diff/snapshots/stub-seat.rs.snap`, `tests/diff/snapshots/up-ps-down-stub.rs.snap`, and `tests/diff/snapshots/capture-stub.rs.snap` record that Rust side. `ExpectedDiff(PE-007)` applies only while the live Rust observation still matches the snapshot for that case. stderr, json, files, and tmux argv stay compared as ordinary fields.
- **Why this is recorded:** this is the stdout and exit from a cold `cargo xtask diff` against OpenRig v0.6.9 (`4b48ca21`). The acceptance spec requires this document and this JSON. Delete this heading and the three `.rs.snap` files when those two fields match v0.6.9. The harness then reports `stale exception`. The `capture-stub` and `stub-seat` TypeScript goldens use the cold-start capture error (`tmux_unavailable`). A leftover OpenRig daemon from an earlier run answers the same capture with `session_missing`, and the golden check fails. dash's `getcwd() failed` line is stripped before compare. CI runs one cold diff, which is the snapshot that is committed.
- **Required test:** `cargo xtask diff` against the pinned tree reports `ExpectedDiff(PE-007)` for `stub-seat`, `up-ps-down-stub`, and `capture-stub`.
- **A v0.6.9-valid spec is not a comparable stub case.** Slice 1's parser accepts `name` plus `seats` and rejects the unknown keys `version`, `pods`, and `members`. A v0.6.9 document (`version: "0.2"`, pods, members, `agent_ref`) is rejected by Rust before any seat exists. OpenRig accepts that file and then resolves agent files and a runtime this slice does not launch. The two sides cannot both produce a stub seat from that file, so the comparison would be a spec-format mismatch rather than stub behaviour. No extra stub case was added.

## PE-008: `rig attach` execs tmux rather than `rig attach --self`

- **TS behaviour:** `packages/cli` `rig attach --self` attaches the operator's terminal through the TypeScript client.
- **Rust behaviour:** acceptance criterion 7 execs the daemon's resolved tmux binary with `-S <socket path> attach -t =<session>` (exact-match target). The path is the one that server reported (`display-message -p '#{socket_path}'`). `-L` plus the caller's `TMUX_TMPDIR` addresses a different server. An unknown seat exits 1.
- **Required test:** `attach_execs_exact_tmux_target`, `attach_unknown_seat_exits_1`, and `attach_uses_the_server_socket_under_tmux_tmpdir`.

## PE-009: `/healthz` is status and pid

- **Covers:** stdout
- **TS behaviour:** `GET /healthz` in `packages/daemon/src/server.ts` returns `status: "ok"` plus production fields: `pid`, `selfHostId`, `selfHostIdSource`, `eventLoop`, `routeTimings`, `bind`, `stuckSweep`, `wakeLadder`, and `slowOperations`.
- **Rust behaviour:** slice 1 returns `{status, pid}`. The health-shape projection is `status=ok` and `keys=pid,status`. Exit status matches.
- **Snapshot:** `tests/diff/snapshots/daemon-health.rs.snap` records that Rust side. `ExpectedDiff(PE-009)` applies only while the live Rust observation still matches it. stderr, exit, json, files, and tmux argv stay compared as ordinary fields.
- **Why this is recorded:** a cold `cargo xtask diff` against OpenRig v0.6.9 (`4b48ca21`) shows the same `status=ok` and a longer key list on the TypeScript side. Delete this heading and the `.rs.snap` when `/healthz` carries the same top-level keys.
- **Required test:** `daemon_health_against_product_daemon`. `cargo xtask diff` against the pinned tree reports `ExpectedDiff(PE-009)` for `daemon-health`.

## PE-011: chat sender is a seat token, not `X-OpenRig-Session`

- **TS behaviour:** `requireSenderIdentity` deliver-and-label. No header and no body sender is `400 actor_required`. No header plus `body.sender` stores that claim. A header supersedes a different body sender and still returns 201. The CLI echoes `OPENRIG_SESSION_NAME` or `[you]`.
- **Rust behaviour:** `RIG_SEAT_ID` and `RIG_SEAT_TOKEN` are stamped into the seat process at launch and checked against `seat_tokens`. A matching pair attributes the row to that seat's name, and only when the seat belongs to the rig in the path. No seat headers, and no claim or a claim of `operator`, attributes the row to `operator`. A claim of anyone else, a wrong token, a partial header pair, or a seat from another rig is rejected. There is no spoofable `--sender` on `rig chatroom send`. The human send line is `[operator]` outside a seat and `[name]` for a seat.
- **Required test:** `send_outside_seat_is_operator`, `send_spoofed_sender_is_rejected`, `send_wrong_token_is_rejected`, `send_matching_token_is_the_seat`, and `send_token_mismatch_claim_is_rejected`.

## PE-012: history page and unknown topic

- **TS behaviour:** `history` without a cursor returns the oldest rows. An unknown `--topic` returns an empty list.
- **Rust behaviour:** without a cursor the page is the newest `limit` rows, oldest first. A cursor pages forward (`id` greater than the cursor, ascending). An unknown topic is `UnknownTopic` after `has_topic` returns false, including when `limit` is 0.
- **Required test:** `history_without_cursor_returns_the_newest_limit` and `unknown_topic_is_an_error`.

## PE-013: never send a blind Enter to a provider dialog

The design index calls this PE-006. That id is already the setup/doctor exception in this file, so the blind-Enter rule is PE-013. This is the only blind-Enter rule.

- **TS behaviour:** `claude-code-adapter.ts` `autoDriveProviderPrompts`, when the trust gate is up, sends `Enter` on the default choice. A default of "No, exit" or "Update now" submits that choice. Codex's update dialog runs `brew upgrade` on "Update now".
- **Rust behaviour:** `crewmux_adapters::dialog::DialogDriver::drive` is the only dialog Enter. Each adapter supplies a recognised-dialog table (`CLAUDE_DIALOGS`, `CODEX_DIALOGS`). The driver captures, recognises, and plans, then moves one `NavKey` at a time and re-captures. `PaneIo::enter` takes a `VerifiedCursor`. Only the driver module can construct that proof, and only when the selected label is the planned label and is not on the deny list (`Update now`, `Always allow`, `No, exit`, `Run without daemon`, `Cancel`, `Allow`). A mismatch escalates and sends no Enter. Trust is answered only when `auto_trust` is true and the folder is the seat cwd, which is not `$HOME`. Bypass consent and a new MCP server escalate. The fullscreen renderer is answered "Not now". Codex update is answered "Skip", never "Update now".
- **Lint:** `cargo xtask lint-blind-enter` scans `crates/crewmux-adapters/src`. `Key::Enter`, `"C-m"`, `"Enter"`, and carriage-return literals (`\r`, `\n`, `\x0d`, `\u{d}`, `.into()`, `concat!`, a const holding one of those) are allowed only in `dialog/driver.rs`, `claude/submit.rs`, and `codex/submit.rs`. `Key::CtrlC`, `Key::Escape`, and `Key::Literal` starting with `q`, `y`, `n`, or a digit are forbidden everywhere, including those files. `SubmitMode::Enter` is not a key.
- **Required test:** `never_enter_on_default_no_exit`, `trust_not_answered_without_opt_in`, `codex_update_answers_skip_never_update`, `bypass_always_escalates`, `unknown_menu_escalates`, `cursor_verification_failure_escalates`, `no_quit_keys_on_login_menu`, and `fullscreen_answers_not_now`. `xtask/tests/lint_enter.rs` plants the key forms above.

## PE-014: retired, superseded by PE-013

## PE-015: Codex thread id comes from the rollout file

- **TS behaviour:** `codex-thread-id.ts` and `launchHarness` scrape the live child process, a nested wrapper, the process home, or the versioned logs database, and pick a thread while title generation is the newest log activity.
- **Rust behaviour:** the id is `session_meta.payload.id` in `$CODEX_HOME/sessions/**/rollout-*.jsonl`. Candidates are the before-snapshot difference, the per-cwd launch lock, and first-message correlation. A tie is `ThreadIdError::Ambiguous`. The id is never guessed from a process or a log row.
- **Required tests:** `back_to_back_launches_get_distinct_ids`, `outside_rollout_is_correlated_by_the_first_message`, `unresolvable_tie_is_ambiguous`, and `snapshot_excludes_rollouts_from_before_launch`.

## Status of real-login testing

A Mac rerun on 2026-10-11 with Claude Code 2.1.286 observed multi-line pastes wrapped in a pasted-content tag and tabs expanded to four spaces (ADR-0009 Amendment 2). Live Claude runs are gated on `RIG_LIVE_CLAUDE=1` inside the `tmux-it` feature and are not part of CI. This change did not execute that live suite. Codex real-login coverage is still pending.

## PE-018: Claude compaction completes on both hooks, not on a timer

The design index calls this PE-007. That id is already the slice 1 stub JSON exception in this file, so compaction completion is PE-018.

- **TS behaviour:** `ClaudeCompactionEnforcer` detects completion by waiting, then sends a turn-boundary handshake, a restore prompt, and a compliance audit. `precompact-hook.mjs` writes one marker per session plus an `.expected.json` sentinel. `compaction-restore-bridge.cjs` tells the seat that compaction finished. A stray `PreCompact` overwrites that marker. Elapsed post-compact time can advance the restore stages.
- **Rust behaviour:** an attempt becomes `Completed` only when `SessionStart` with `source=compact` and `PostCompact` both arrive for the same `session_id`, in either order, within 120s. Only the newest open attempt for that session completes. Older open attempts with the same trigger become `Failed("superseded")` and are not restored. The marker is per `(seat_key, attempt_id)` and is acted on only when `confirmed` is true. An unsolicited `PreCompact` opens a separate attempt and does not rewrite a managed marker. The 10s post-compact wait is only a lower bound before the `Ready` check. Time alone never submits a restore. Exactly one restore prompt is submitted, through `SeatIo::submit`, after both signals and `Ready`. `NotVerified` or `Contaminated` after Enter marks that attempt `Failed`. The supervisor does not send the restore again. The sentinel file and the bridge script are not ported. Prompt text, thresholds, the 60s dedup, re-arm, the 120s manual and 25-minute automatic prep budgets, and the 10-minute cooldown match v0.6.9, except the restore prompt does not name the `claude-compaction-restore` or refocusing plugin skills. Enforcer sample state is still in-memory. Attempt rows are persisted. When `RESTORE-MAP.md.tmp` ends with the completion marker, the daemon renames it to `RESTORE-MAP.md` and then submits `/compact`. `SessionEnd` with `reason=clear` does not delete the per-run settings directory. Hook commands use the absolute `rig` path.
- **Open gated test:** L3, genuine threshold-driven auto-compaction, is not done. `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` did not produce `PostCompact` in spike S2. L1 and L2 (manual `/compact`, then resume) were observed in that spike on Claude Code 2.1.286 and were not re-run in this checkout. AC12 stays gated on `RIG_LIVE_CLAUDE=1`, which CI does not set. AC13 prompt goldens: `restore_prompt_golden` locks the one restore prompt this build sends. The turn-boundary and compliance prompts are still ported and still not sent. The spike archive has `hooks.jsonl` and no Claude transcript jsonl; `s2-result.json` only projects `compact_boundary` rows.
- **Required test:** `enforcer_table`, `hook_parse_golden`, `spool_drain`, `per_run_settings`, `completion_requires_both_signals`, `stray_auto_precompact_does_not_touch_managed_marker`, `unconfirmed_marker_ignored`, `no_timing_triggered_restore`, `compaction_never_submits_when_not_ready`, `no_claude_files_written`, and `fire_compaction_recorded_payload`.
