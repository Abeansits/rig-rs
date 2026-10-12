# Parity exceptions
These are deliberate behavioural differences from OpenRig v0.6.9, where matching TS behaviour would reproduce a bug. Each entry has a required test. The parity ledger (PLAN.md §2.1) marks the affected TS tests `waived` and links them here. The differential harness (`xtask diff`) lists these IDs as expected diffs.

## PE-001: Claude first-run / login screens must not be classified as ready
- **TS behaviour:** `packages/daemon/src/domain/native-resume-probe.ts` `assessNativeResumeProbe`, used by `claude-code-adapter.ts` `checkReady` (l.362). `looksLikeClaudeTui` matches `/(^|\n)\s*❯/` plus `"Claude Code v"`. The first-run banner "Welcome to Claude Code v2.1.296" and the menu cursor `❯` satisfy both, so the theme picker and "Select login method:" are classified as **`resumed / active_runtime` (ready)**. `looksLikeClaudeLoginPrompt` only knows the in-session "Not logged in" + "Run /login" text. Spike S13 (report: spikes/3-gate-detection.md) reproduced this by running the verbatim TS probe on fixtures cl-0, cl-1 and cl-2.
- **Impact:** on a fresh or logged-out machine, startup context is pasted into a menu. Digits in the payload can even select a menu option.
- **Rust behaviour:**
  - "Ready" requires a **positive composer match**: the Claude input box border plus the mode/footer line. It is never banner + `❯`.
  - New gate codes: `OnboardingTheme`, and `LoginRequired` for "Select login method".
  - **Any numbered-menu screen (`❯ 1.` / `  2.` pattern) that matches no known gate is `AttentionRequired(UnknownMenu)`, never ready.**
- **Required test:** `crewmux-adapters/tests/claude_gates.rs::first_run_screens_are_not_ready`. It loads fixtures `claude/cl-0-theme.txt`, `cl-1-login-method.txt` and `cl-2-login-method-opt2.txt`. It asserts `OnboardingTheme`, `LoginRequired` and `LoginRequired`, with `requires_attention() == true` and `!is_ready()`. The same test asserts zero paste and zero Enter on those screens. `splash_without_footer_is_not_ready` covers the splash case below. Both are CI-blocking.
- **Mac rerun (2026-10-11, Claude Code 2.1.286):** a splash with the banner and a bare `❯` but no `? for shortcuts` footer is inconclusive, not ready. TS `assessNativeResumeProbe` classifies that splash as resumed. The Rust classifier is a positive match (footer, border, prompt, no menu, no interrupt), not a port of that probe.
- **Pending:** the 2.1.286 corpus under `tests/fixtures/screens/claude/2.1.286/` is reconstructed from the dialog titles and this rerun. It is not a byte copy of the spike captures.

## PE-002: Codex login screens must be detected explicitly (TS hangs until timeout)
- **TS behaviour:** `codex-runtime-adapter.ts` `checkReady` (l.483) has no login matcher apart from token-refresh failure text. "Sign in with ChatGPT" / "Provide your own API key" (fixtures cx-1/2/3) classify as `inconclusive / awaiting_runtime`, and the launch waits until the readiness timeout. A missing CODEX_HOME (cx-0) is also inconclusive, though it should be a hard failure.
- **Rust behaviour** (the Codex login detection design, implemented in `adapters::codex::screen` and `adapters::codex::preflight`):
  - `LoginRequired` when the screen matches the sign-in menu ("Sign in with ChatGPT", "Provide your own API key", or the API-key entry field).
  - `HardFailure(CodexHomeMissing)` for the CODEX_HOME error, which is not retried.
  - **Pre-flight before launch:** `codex login status` (exit code / output) runs under the seat's CODEX_HOME. If it reports not logged in, the launch is short-circuited to `LoginRequired` without starting the TUI. The screen matcher is the second line of defence.
  - **Inconclusive budget:** an `Inconclusive` verdict that persists for `inconclusive_budget` (default 15 s, configurable) while the screen is *stable* (same hash across 3 captures) escalates to `AttentionRequired(UnrecognisedScreen)` and attaches the capture as evidence. It does not wait for the full launch timeout. This applies to all runtimes.
- **Required tests:** `codex_gates.rs::login_screens_are_login_required` (cx-1, cx-2, cx-3 give `LoginRequired`; cx-0 gives `HardFailure`); `codex_launch.rs::not_logged_in_preflight_short_circuits` (stub `codex` binary on PATH printing not-logged-in; asserts no tmux session is created and the seat state is `attention_required/login_required`); `readiness.rs::stable_inconclusive_escalates_before_timeout` (FakeMux plus a fake Clock).
- **Pending:** confirm the current `codex login status` output and exit codes against a real logged-in and logged-out codex. The fixtures come from codex-cli 0.162.1.

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

The design index calls this PE-006. That id is already the setup/doctor exception in this file, so the blind-Enter rule is PE-013.

- **TS behaviour:** `claude-code-adapter.ts` `autoDriveProviderPrompts`, when the trust gate is up, sends `Enter` on the default choice. A default of "No, exit" or "Update now" submits that choice. Codex's update dialog runs `brew upgrade` on "Update now".
- **Rust behaviour:** `DialogDriver` locates the choice by exact text, sends Up or Down, captures again, and sends Enter only when the cursor (`❯` or `›`) is on that exact line. A mismatch escalates and sends no Enter. Trust is answered only when `auto_trust` is true, and then only "Yes, I trust this folder". Bypass consent and the MCP dialog have no automatic choice. The fullscreen renderer is answered "2. Not now". Codex update is answered "2. Skip". Outside a verified dialog action and `SeatIo::submit`, adapter code does not send Enter, `q`, Ctrl-C, Escape, or a digit. Daemon stop kills the tmux session.
- **Required test:** `never_enter_on_default_no_exit`, `trust_not_answered_without_opt_in`, `codex_update_answers_skip_never_update`, `bypass_always_escalates`, `unknown_menu_escalates`, `cursor_verification_failure_escalates`, and `no_quit_keys_on_login_menu`. `cargo xtask lint-blind-enter` fails when adapter source sends Enter outside `claude/submit.rs` and `dialog/drive.rs`.

## Status of real-login testing

A Mac rerun on 2026-10-11 with Claude Code 2.1.286 observed multi-line pastes wrapped in a pasted-content tag and tabs expanded to four spaces (ADR-0009 Amendment 2). Live Claude runs are gated on `RIG_LIVE_CLAUDE=1` inside the `tmux-it` feature and are not part of CI. This change did not execute that live suite. Codex real-login coverage is still pending.
