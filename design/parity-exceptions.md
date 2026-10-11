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
- **Pending:** a positive "ready" fixture from a **logged-in** Claude Code, needed to prove the composer matcher doesn't produce false negatives.

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
- **Rust behaviour today:** slice 1 prints clap help for `up`, `ps`, `capture`, `attach`, `down`, and `help`, and `rig --version` prints `rig 0.0.0`. The help-name projection is those names sorted. The exit status still matches, which is why the `version` case (exit code only) is Equal. The tmux shim records the same `tmux -V` argv on both sides.
- **Snapshot:** `tests/diff/snapshots/help-top.rs.snap` and `tests/diff/snapshots/scenario-runner-probe.rs.snap` record that Rust side. `ExpectedDiff(PE-003)` applies only while the live Rust observation still matches that snapshot. Exit status, stderr, JSON, files, and tmux argv are compared as ordinary fields. A different exit code, a stderr panic, or an extra or missing tmux call is `Unexpected`.
- **Why this is recorded:** these are real diffs from a local `cargo xtask diff` against OpenRig v0.6.9 (`4b48ca21`). They are not a behaviour we intend to keep, and they are not hidden. Delete this heading, and the two `.rs.snap` files, when the CLI prints the same command names and `0.6.9` version text. The harness then reports `stale exception` because the Rust output no longer matches the snapshot (or the two sides are equal).
- **Required test:** `cargo xtask diff` against the pinned tree reports `ExpectedDiff(PE-003)` for `help-top` and `scenario-runner-probe`, and `Equal` for `version`. `excepted_exit_code_is_unexpected`, `excepted_stderr_panic_is_unexpected`, and `excepted_extra_tmux_call_is_unexpected` each return `Unexpected`. `changed_rust_snapshot_is_stale` reports `stale exception`.

## PE-004: `rig attach` execs tmux rather than `rig attach --self`

- **TS behaviour:** `packages/cli` `rig attach --self` attaches the operator's terminal through the TypeScript client.
- **Rust behaviour:** acceptance criterion 7 execs `tmux -L rig-rs attach -t =<session>` (exact-match target). An unknown seat exits 1.
- **Required test:** `attach_execs_exact_tmux_target` and `attach_unknown_seat_exits_1`.

## PE-005: the stub script is `RIG_STUB_SCRIPT` key=value, not a JSON step list

- **TS behaviour:** `packages/daemon/src/stub-script.ts` runs a JSON list of steps.
- **Rust behaviour:** the slice 1 acceptance script is comma-separated `ready_after_ms`, `exit_after_ms`, and `print_gate=<code>`. The fixture still prints `stub> ` and echoes submitted lines.
- **Required test:** `crewmux_adapters::screen_tests::script_parses_gate_and_timers` and `gate_surfaces_as_attention`.

## Status of real-login testing
All tests against **real logged-in Claude Code and Codex** (S1a composer delivery, S13 trust/MCP/bypass/update/ready screens, launch-to-ready end to end) are **pending**. The spike box has no logged-in CLIs, and none were logged in. This needs a box or account designated by Sebastian.
