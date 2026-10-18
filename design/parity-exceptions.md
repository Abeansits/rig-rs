# Parity exceptions
These are deliberate behavioural differences from OpenRig v0.6.9, where matching TS behaviour would reproduce a bug. Each entry has a required test. The parity ledger (PLAN.md §2.1) marks the affected TS tests `waived` and links them here. The differential harness (`xtask diff`) lists these IDs as expected diffs.

ADR-0010 makes this file the single PE register. Design-workspace ids that collided with repo ids PE-003…PE-012 are renamed here:

| Design id | Topic | Repo id |
| --- | --- | --- |
| PE-001 | Claude first-run is not ready | PE-001 |
| PE-002 | Codex login | PE-002 |
| PE-003 | MCP tool rename | PE-016 |
| PE-004 | no writes to the Claude user config | PE-004 |
| PE-005 | exactly-once direct/startup re-delivery | PE-017 |
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
- **Rust behaviour today:** the help-name projection is `ask`, `attach`, `broadcast`, `capture`, `chatroom`, `checkpoint`, `compact`, `context`, `crash-cart`, `doctor`, `down`, `health`, `help`, `ps`, `queue`, `restore`, `restore-check`, `send`, `setup`, `snapshot`, `stream`, `unclaim`, `up`, `usage`, `whoami`, and `workflow`. `compact` is the slice 9 operator command. `hook` stays hidden: it is the Claude hook entry, not an operator verb. `rig --version` prints `rig 0.0.0`. That set is still a subset of the v0.6.9 command list. The exit status still matches, which is why the `version` case (exit code only) is Equal. The tmux shim records the same `tmux -V` argv on both sides.
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
- MCP `daemon_health` returns this same body. It does not add a version field. A socket that is down is `{"running": false}` rather than an error string (PE-020).

## PE-011: chat sender is a seat token, not `X-OpenRig-Session`

- **TS behaviour:** `requireSenderIdentity` deliver-and-label. No header and no body sender is `400 actor_required`. No header plus `body.sender` stores that claim. A header supersedes a different body sender and still returns 201. The CLI echoes `OPENRIG_SESSION_NAME` or `[you]`.
- **Rust behaviour:** `RIG_SEAT_ID` and `RIG_SEAT_TOKEN` are stamped into the seat process at launch and checked against `seat_tokens`. A matching pair attributes the row to that seat's name, and only when the seat belongs to the rig in the path. No seat headers, and no claim or a claim of `operator`, attributes the row to `operator`. A claim of anyone else, a wrong token, a partial header pair, or a seat from another rig is rejected. There is no spoofable `--sender` on `rig chatroom send`. The human send line is `[operator]` outside a seat and `[name]` for a seat. `POST /api/send` and `POST /api/broadcast` use the same headers and ignore a body `from`. No headers records the non-seat sender `operator@host`. A wrong or partial token is refused with `unknown sender` and nothing is pasted. A valid token is recorded as `name@rig` even when the body names someone else. `rig send --from` is accepted and ignored.
- **Required test:** `send_outside_seat_is_operator`, `send_spoofed_sender_is_rejected`, `send_wrong_token_is_rejected`, `send_matching_token_is_the_seat`, `send_token_mismatch_claim_is_rejected`, `valid_seat_token_is_the_sender`, `bogus_seat_token_is_refused`, `body_sender_does_not_override_the_token`, and `missing_token_is_operator_not_the_body`.

## PE-012: history page and unknown topic

- **TS behaviour:** `history` without a cursor returns the oldest rows. An unknown `--topic` returns an empty list.
- **Rust behaviour:** without a cursor the page is the newest `limit` rows, oldest first. A cursor pages forward (`id` greater than the cursor, ascending). An unknown topic is `UnknownTopic` after `has_topic` returns false, including when `limit` is 0.
- **Required test:** `history_without_cursor_returns_the_newest_limit` and `unknown_topic_is_an_error`.

## PE-013: never send a blind Enter to a provider dialog

The design index calls this PE-006. That id is already the setup/doctor exception in this file, so the blind-Enter rule is PE-013. This is the only blind-Enter rule.

- **TS behaviour:** `claude-code-adapter.ts` `autoDriveProviderPrompts`, when the trust gate is up, sends `Enter` on the default choice. A default of "No, exit" or "Update now" submits that choice. Codex's update dialog runs `brew upgrade` on "Update now".
- **Rust behaviour:** `crewmux_adapters::dialog::DialogDriver::drive` is the only dialog Enter. Each adapter supplies a recognised-dialog table (`CLAUDE_DIALOGS`, `CODEX_DIALOGS`). The driver captures, recognises, and plans before any wait. It then waits until the dialog is unchanged (1s on a live launch; a test budget under 5s skips the wall wait). It moves one `NavKey` at a time, pauses, and re-captures. Enter requires the planned label on two captures. A redraw that puts the cursor back on `No, exit` is not Enter. The two-row menu is treated as wrapping: one arrow, then a fresh read, and the step cap stays 8. `PaneIo::enter` takes a `VerifiedCursor`. Only the driver module can construct that proof, and only when the selected label is the planned label and is not on the deny list (`Update now`, `Always allow`, `No, exit`, `Run without daemon`, `Cancel`, `Allow`). A mismatch escalates and sends no Enter. Trust is answered only when `auto_trust` is true and the folder is the seat cwd, which is not `$HOME`. Bypass consent and a new MCP server escalate. The fullscreen renderer is answered "Not now". Codex update is answered "Skip", never "Update now". Claude Code 2.1.287 starts the trust cursor on `No, exit`; the folder is one line. The driver Downs to Yes and Enters only after that highlight has settled.
- **Lint:** `cargo xtask lint-blind-enter` scans `crates/crewmux-adapters/src`. `Key::Enter`, `"C-m"`, `"Enter"`, and carriage-return literals (`\r`, `\n`, `\x0d`, `\u{d}`, `.into()`, `concat!`, a const holding one of those) are allowed only in `dialog/driver.rs`, `claude/submit.rs`, and `codex/submit.rs`. `Key::CtrlC`, `Key::Escape`, and `Key::Literal` starting with `q`, `y`, `n`, or a digit are forbidden everywhere, including those files. `SubmitMode::Enter` is not a key.
- **Required test:** `never_enter_on_default_no_exit`, `trust_not_answered_without_opt_in`, `codex_update_answers_skip_never_update`, `bypass_always_escalates`, `unknown_menu_escalates`, `cursor_verification_failure_escalates`, `no_quit_keys_on_login_menu`, `fullscreen_answers_not_now`, `transient_yes_is_not_entered`, `a_dialog_that_never_reaches_yes_sends_no_enter`, `one_down_then_two_yes_reads_enter`, and `real_trust_dialog_downs_from_no_then_enters_yes`. `xtask/tests/lint_enter.rs` plants the key forms above.

## PE-014: retired, superseded by PE-013

## PE-015: Codex thread id comes from the rollout file

- **TS behaviour:** `codex-thread-id.ts` and `launchHarness` scrape the live child process, a nested wrapper, the process home, or the versioned logs database, and pick a thread while title generation is the newest log activity.
- **Rust behaviour:** the id is `session_meta.payload.id` in `$CODEX_HOME/sessions/**/rollout-*.jsonl`. Candidates are the before-snapshot difference, the per-cwd launch lock, and first-message correlation. A tie is `ThreadIdError::Ambiguous`. The id is never guessed from a process or a log row.
- **Required tests:** `back_to_back_launches_get_distinct_ids`, `outside_rollout_is_correlated_by_the_first_message`, `unresolvable_tie_is_ambiguous`, and `snapshot_excludes_rollouts_from_before_launch`.

## PE-016: MCP tool names renamed

The design index calls this PE-003. That id is already the slice 1 help exception in this file, so the MCP rename is PE-016.

- **TS behaviour:** `packages/cli/src/mcp-server.ts` registers 18 tools. The names, sorted as `mcp-server.test.ts` lists them, are `rig_add`, `rig_agent_validate`, `rig_bind`, `rig_bundle_inspect`, `rig_capture`, `rig_chatroom_send`, `rig_chatroom_watch`, `rig_discover`, `rig_down`, `rig_ps`, `rig_restore`, `rig_rig_nodes`, `rig_rig_validate`, `rig_send`, `rig_snapshot_create`, `rig_snapshot_list`, `rig_status`, and `rig_up`. `tools/list` length is 18.
- **Rust behaviour:** the stdio server name is `rig` and it registers exactly 15 tools: `rig_start`, `rig_stop`, `rig_list`, `agent_list`, `agent_add`, `agent_send`, `agent_read_screen`, `chat_send`, `chat_read`, `snapshot_create`, `snapshot_list`, `snapshot_restore`, `spec_validate_rig`, `spec_validate_agent`, and `daemon_health`. There are no aliases. `rig_discover`, `rig_bind`, and `rig_bundle_inspect` are not registered; calling one is the protocol error `unknown tool`. The other fifteen names were renamed as above.
- **What this exception covers:** the tool name and the dropped tools. It does not cover a different HTTP path, request body, timeout, or flag. Those differences are PE-020.
- **Lint:** `cargo xtask lint mcp-legacy-names` fails when one of the 18 legacy names appears in `docs/`, `assets/skills/`, or `crates/**/src` outside `crates/crewmux-mcp/src/names.rs`. `cargo xtask lint mcp-state-path` fails when `crewmux-mcp`'s `[dependencies]` names `crewmux-store`, `crewmux-mux`, `crewmux-adapters`, or `crewmux-durability`.
- **Required tests:** `tools_list_is_the_fifteen_names`, `stdio_reconnect_lists_fifteen`, `bundle_inspect_is_unknown_tool`, `schemas_have_required_fields`, and `spec_validate_tools_require_yaml`.

## PE-019: the process-lineage selector is not the pre-send check

- **TS behaviour:** `native-process-lineage.test.ts` selects the agent through a process tree (nix wrappers, the executable path, launcher shims). `native-claude-delivery.test.ts` pairs that selector with the ordinary transport.
- **Rust behaviour:** before a paste (direct send, launch startup, restore re-send, and `POST /api/seats/submit`), the daemon checks the recorded pid, that the foreground is not a shell, and the session. A version-number process title (`2.1.289`) counts as Claude, because Node's `process.title` overwrites the command line. The start time distinguishes a reused pid; it does not prove the session, because `exec` keeps both. A session or resume token in argv must be the id stored at launch, including when the launch record has none (`codex resume <other>`). With no token, an open file has to name that id, or the send is refused. Claude Code 2.1.286 on Linux keeps `--session-id` in `/proc` cmdline (`comm` is `claude`); the fixture `crewmux-daemon/tests/fixtures/claude-code-2.1.286-fds/` shows the transcript fd is closed. The native binary is covered by the token check. The process-tree selector is not ported. Stub seats take the same record at launch. `rig send`, `POST /api/send`, and every compaction paste (the prep prompt, `/compact`, and the restore prompt) go through `submit_text`, which waits on `lineage::ensure`. A shell in the live argv is a bare shell even when tmux still reports the command recorded at launch.
- **Remaining risk:** a titled build with no argv token is refused, even when it is still the launched process. An in-app `/resume` that leaves argv and open files unchanged is only caught after the send. A same-image `exec` into a non-shell that leaves tmux naming the launch command is not distinguished from the launched process.
- **Required test:** `pid_mismatch_refuses_before_paste`, `session_id_mismatch_refuses_before_paste`, `same_start_time_without_a_session_file_is_not_proof`, `codex_resume_token_the_record_lacks_refuses`, `exec_into_another_session_pastes_nothing`, `respawn_under_another_session_pastes_nothing`, `exec_into_bash_pastes_nothing`, `hidden_title_exec_into_another_session_pastes_nothing`, `node_title_before_launch_pastes_nothing`, `node_title_after_launch_pastes_nothing`, `node_argv_session_delivers`, `shell_argv_is_a_bare_shell_when_tmux_still_names_the_agent`, `normal_stub_seat_still_delivers`, `respawn_pane_delivers_nothing`, and `exec_into_bash_delivers_nothing`.

## PE-020: MCP tools call the Rust daemon routes

PE-016 covers the renamed tool list only. These are the route, body, and flag differences that remain after that rename. Matching the TypeScript assertions would call routes this daemon does not have, or would ignore a flag that changes what the tool does.

- **`agent_add`:** there is no `POST /api/rigs/{id}/pods/{ns}/members` route. The tool returns `{"error":"not_implemented","feature":"agent_add"}` and does not open the socket. A 404 with an empty body was the previous result.
- **`rig_start`:** posts the spec file's YAML bytes to `POST /api/rigs/up` (120s). It does not post `{sourceRef, plan, autoApprove, targetRoot}` to `POST /api/up`. `plan: true`, `autoApprove: true`, and any `targetRoot` are refused before HTTP, because each one would change bring-up and the daemon has no such flag. A file that does not parse as a rig spec is rejected locally; the error text does not include the file body, so a non-spec file's keys are not echoed.
- **`rig_stop`:** `snapshot: true` calls `POST /api/snapshot/create` with `{"rig": rigId}` and, only if that succeeds, `POST /api/rigs/down` with `{"rig": rigId}`. A failed snapshot does not stop the rig. `delete: true` and `force: true` are refused. This is not `POST /api/down` with `{rigId, delete, force, snapshot}`.
- **`rig_list`:** `GET /api/rigs/summary` returns `{id, name}` rows. There is no `GET /api/ps`, and the summary has no status, uptime, or node counts.
- **`agent_list`:** `rigId` is required. The tool resolves it through `GET /api/rigs/summary` and returns that rig's seats from `GET /api/seats`. There is no `GET /api/rigs/:id/nodes`, and the seats are not a node inventory (`nodeKind`, attach command, resume command).
- **`agent_read_screen`:** one session is `POST /api/seats/capture` with `{seat, lines}`. `rig` and `pod` are refused. The daemon captures a single seat, not every session in a rig or pod.
- **`chat_send`:** optional `sender` is accepted and not forwarded, which matches the TypeScript tool (the field is deprecated and ignored). The body stays `{body}`. The `X-Rig-Seat-*` headers are forwarded. Chat does not paste into a pane, so `lineage::ensure` does not apply.
- **`daemon_health`:** returns the `/healthz` body from PE-009 (`status` and `pid`, no version). A down socket is `{"running": false}` and does not start the daemon.
- **`waitForIdleSeconds`:** a positive value is refused with `not_implemented` before any HTTP call. It is not sent as `waitForIdleMs`, and the client timeout is not extended. `force` combined with the wait is still the TypeScript error, also before HTTP. `force` alone does not set `dangerously_interact`.
- **`spec_validate_rig` and `spec_validate_agent`:** local `crewmux-spec` checks. Zero HTTP. They do not `POST` the YAML to `/api/rigs/import/validate` or `/api/agents/validate`.
- **`snapshot_create`:** `POST /api/snapshot/create` with `{"rig": rigId}`. The id is not a percent-encoded path segment under `/api/rigs/:id/snapshots`.
- **`agent_send`:** posts `{to, body, verify}` to `POST /api/send` and forwards the `X-Rig-Seat-*` headers. It does not send a body `from`. The daemon attributes the sender from those headers (PE-011): a valid seat token is `name@rig`, and no headers is `operator@host`. Paste, including this tool, runs `lineage::ensure` first. A replaced stub pane is refused and nothing is pasted.
- **Required tests:** `agent_add_is_not_implemented`, `rig_start_posts_spec_yaml`, `rig_start_refuses_flags`, `rig_start_hides_non_spec_keys`, `rig_stop_posts_down`, `rig_stop_snapshots_then_stops`, `rig_stop_does_not_stop_when_snapshot_fails`, `rig_stop_refuses_delete_and_force`, `rig_list_gets_summary`, `agent_list_filters_by_rig`, `agent_read_screen_returns_capture`, `agent_read_screen_refuses_rig_or_pod`, `chat_send_ignores_sender`, `agent_send_wait_is_refused`, `agent_send_returns_daemon_json`, `agent_send_rejects_unknown_arguments`, `replaced_stub_pane_receives_nothing_via_mcp`, `spec_validate_rig_is_local`, `snapshot_create_posts_body`, and `daemon_health_returns_up_body`. `cargo xtask diff` case `mcp-tool-names` maps the legacy names onto the registered Rust names.

## PE-021: `rig context deliver` pastes a pack; startup is not implemented

v0.6.9's `rig context` noun is delivery-free. Its subcommands stop at `rm`, and `context-grammar.test.ts` asserts that list contains no send seam. Slice 19 adds a guarded paste. It does not add a startup writer.

- **TS behaviour:** `rig context` serves bytes to the caller's stdout. There is no `send` subcommand and no daemon route that pastes a pack into a seat.
- **Rust behaviour:** `rig context deliver --ref <ref> --seat <seat> --mode paste` posts `{ref, seat, mode}` to `POST /api/context-packs/deliver`. `paste` calls the slice 4 `deliver` path (`lineage::ensure`, the paste guard, a receipt, and `obligation_fingerprint`). A receipt whose `delivery.status` is `failed` exits 1, and `partial` exits 3, the same codes as `rig send`. `mode startup` returns HTTP 501 `not_implemented` before the pack is looked up or any member is read, and writes nothing. The stub adapter's `deliver_startup` only counts files, so this route does not call it. The other `rig context` verbs stay delivery-free.
- **Verification window:** paste is already chunked. On a real pane the post-paste check captures at most 1024 history rows plus the visible pane. A 64 KiB line on the 90-column stub pane (about 730 wrapped rows) fits and is delivered. A taller paste fails closed: the prompt scrolls out, the receipt is `failed`, and Enter is not sent. `paste_64k_returns_receipt_and_fingerprint` is the in-memory multiplexer. `context_pack_64kib_delivers_on_real_tmux` is the same size on real tmux. Live Claude and Codex delivery stays gated and is not part of CI.
- **Required tests:** `paste_64k_returns_receipt_and_fingerprint`, `startup_delivery_is_not_implemented`, `startup_returns_not_implemented_before_reading_a_symlink`, `a_failed_receipt_exits_nonzero`, `context_pack_64kib_delivers_on_real_tmux`, `context_pack_past_the_history_window_exits_nonzero`, and `recap_write_and_deliver_speak_the_daemon`.

## PE-022: a human workflow destination is the operator surface

- **TS behaviour:** `workflow-human-destination.ts` resolves a human seat through the gateway human registry (`addHumanFragment`, Slack connector bindings, `address: name@external`). Delivery of that destination goes out through the Slack/gateway path.
- **Rust behaviour:** `human_only` exception routing and a human gate use the operator seat `human@kernel` (`OPERATOR_SEAT` in `crewmux-orchestration::dial`). That seat is the operator surface: the TUI attention view and the CLI inbox. There is no Slack connector and no gateway human registry. A stuck-overdue exception item is addressed to that seat.
- **Required test:** `crewmux_orchestration::dial::tests::human_only_is_the_operator_seat`.
- **Unresolved role:** an entry step or a next step with no `preferred_targets` and no explicit owner is HTTP 400. Instantiate reports `entry_owner_unresolved`. Project reports `next_owner_unresolved`. That is the TypeScript status on the unbound no-target path. Matching it is the behaviour; a note that treated 400 as a miss is retired.
- **Required test:** `unresolved_owner_does_not_mutate_and_graph_project_is_refused` asserts status 400.

## PE-023: project, seat, and mission reads refuse every symlink

`profile-source-resolver.ts` follows a symlink and still returns the target's bytes. Rust refuses the link.

- **TS behaviour:** a project, seat, or mission ref is opened with the normal path walk. A symlink is followed. A dangling link is named. `escapes_root` is set when the real path leaves the configured root, and the bytes are still returned. A hard link is an ordinary file.
- **Rust behaviour:** `read_source` opens the configured root, then walks the relative path from that directory fd (`openat2` with `RESOLVE_NO_SYMLINKS | RESOLVE_BENEATH` on Linux when the kernel has it, otherwise `openat` with `O_NOFOLLOW`). Any symlink component fails the read. The target's bytes are not returned, and `escapes_root` is not how a symlink is reported. A final file with `st_nlink > 1` is also refused. TypeScript does not refuse that hard-link case.
- **Required test:** `project_seat_and_mission_reads_refuse_symlinks`.

## PE-024: a long newline-free paste may be a bare `[Pasted text #N]`

v0.6.9 `hasExpectedStagedText` (`packages/daemon/src/domain/session-transport.ts`) accepts `[Pasted text #N +X lines]` when X is the payload's newline count, or the first 24 normalised characters of the payload when there is no placeholder. It does not accept a bare `[Pasted text #N]`.

- **TS behaviour:** the placeholder regex is `\[Pasted text #\d+ \+(\d+) lines\]`. A token with no `+X lines` falls through to the 24-character head check. A long collapsed paste whose composer shows only `[Pasted text #N]` does not match, and the send is treated as missing.
- **Rust behaviour:** Claude Code 2.1.287 collapses a newline-free paste longer than 31 bytes to `❯ [Pasted text #N]` and `paste again to expand`, with no line count. `staged_matches` accepts exactly one such token when the payload contains no newline and is longer than 31 bytes. A 31-byte single line still has to appear inline. A multi-line payload still needs `+X lines` or the residual suffix. The `+11` form is unchanged.
- **Required test:** `bare_pasted_token_matches_a_long_single_line`.

## Status of real-login testing

A Mac rerun on 2026-10-11 with Claude Code 2.1.286 observed multi-line pastes wrapped in a pasted-content tag and tabs expanded to four spaces (ADR-0009 Amendment 2). Live Claude runs are gated on `RIG_LIVE_CLAUDE=1` inside the `tmux-it` feature and are not part of CI. This change did not execute that live suite. Codex real-login coverage is still pending.

## PE-017: direct and startup deliveries are reconciled on restore

The design index calls this PE-005. That id is already the stub-script exception in this file, so the added reconciliation is PE-017. Queue rows are not part of it.

- **TS behaviour:** `restore-orchestrator.ts` does not re-send direct `rig send` messages. Startup replay is emptied when `replayContained = resumeRequested && !!resumeToken`, so an exact resume does not re-paste startup files. A fresh launch replays startup files through the startup orchestrator with no transcript fingerprint check. The guarded Enter retry says it never repastes. Queue rows stay pointer nudges.
- **Rust behaviour:** queue rows stay in their committed state and are re-nudged with the slice 4 pointer only. Direct and startup rows are acked, re-sent, or escalated from the transcript text-hash (ADR-0009). A match acks. No match re-sends. A contaminated turn escalates. Identical bodies are counted by occurrence. This is added reconciliation, not a port of the TypeScript path.
- **Required tests:** `crewmux_durability::reconcile::tests::queue_rows_only_renudged`, `no_duplicates_after_crash_between_paste_and_ack`, `identical_bodies_counted_by_occurrence`, `contaminated_turn_escalates`, `normalisation_matches_crlf_nfc_and_trailing_space_only`, and `startup_channel_uses_the_same_rule`.

## PE-018: Claude compaction completes on both hooks, not on a timer

The design index calls this PE-007. That id is already the slice 1 stub JSON exception in this file, so compaction completion is PE-018.

- **TS behaviour:** `ClaudeCompactionEnforcer` detects completion by waiting, then sends a turn-boundary handshake, a restore prompt, and a compliance audit. `precompact-hook.mjs` writes one marker per session plus an `.expected.json` sentinel. `compaction-restore-bridge.cjs` tells the seat that compaction finished. A stray `PreCompact` overwrites that marker. Elapsed post-compact time can advance the restore stages.
- **Rust behaviour:** an attempt becomes `Completed` only when `SessionStart` with `source=compact` and `PostCompact` both arrive for the same `session_id`, in either order, within 120s. Only the newest open attempt for that session completes. Older open attempts with the same trigger become `Failed("superseded")` and are not restored. The marker is per `(seat_key, attempt_id)` and is acted on only when `confirmed` is true. An unsolicited `PreCompact` opens a separate attempt and does not rewrite a managed marker. The 10s post-compact wait is only a lower bound before the `Ready` check. Time alone never submits a restore. Exactly one restore prompt is submitted, through `SeatIo::submit`, after both signals and `Ready`. `NotVerified` or `Contaminated` after Enter marks that restore attempt `Failed`. The supervisor does not send the restore again. The same two errors after Enter on `/compact` count as sent and are not retried: the transcript skips that command record, so it can never confirm. `NotReady` and `PasteMismatch` are still errors. The sentinel file and the bridge script are not ported. Occupant generation is not ported, so a missing generation does not fail closed and `skip-map` still compacts. Prompt text, thresholds, the 60s dedup, re-arm, the 120s manual and 25-minute automatic prep budgets, and the 10-minute cooldown match v0.6.9, except the restore prompt drops the `claude-compaction-restore` skill sentence and the refocusing `SKILL.md` sentence. Enforcer sample state is still in-memory. Attempt rows are persisted. When `RESTORE-MAP.md.tmp` ends with the completion marker and is not a symlink, the daemon renames it to `RESTORE-MAP.md` and then submits `/compact`. `SessionEnd` with `reason=clear` does not delete the per-run settings directory. Hook commands use the absolute `rig` path. Compact-record skips are structural (`isCompactSummary`, `isMeta`, `subtype` of `compact_boundary` or `local_command`, or a command-tag envelope whose `command-name` is `/compact`). OpenRig does not skip command tags; a message that is only tags for a different command still counts as a user turn. A user message that quotes `<local-command-stdout>` still confirms. The preparation is cleared when its attempt reaches done, aborted, or failed, and on `--cancel`, so a later `rig compact` is not `already_in_progress` or `preparation_stopped`. A prep paste that fails before Enter (`PasteMismatch`, `NotReady`, or any other pre-Enter error) marks that attempt `Failed` (`paste_mismatch` or `send_failed`), sets the manual stage to `failed`, and releases the prep. The next `rig compact` starts `manual-…-N+1` and sends the prep again. It does not return `preparation_pending` for a paste that never landed. `NotVerified` and `Contaminated` are after Enter and do not release the prep. Auto compaction is suppressed only while that manual stage is still non-terminal (preparing, waiting for the map, compacting, or restoring). `rig compact` exits 0 once `/compact` has been submitted. It does not wait out the 120s completion budget, so a later `no_post_compact` abort does not change that exit code. `--status` reports the attempt.
- **Open gated test:** L3, genuine threshold-driven auto-compaction, is not done. `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` did not produce `PostCompact` in spike S2. L1 and L2 (manual `/compact`, then resume) were observed in that spike on Claude Code 2.1.286 and were not re-run in this checkout. AC12 stays gated on `RIG_LIVE_CLAUDE=1`, which CI does not set. AC13 prompt goldens: `restore_prompt_golden` locks the one restore prompt this build sends. The turn-boundary and compliance prompts are still ported and still not sent. The spike archive has `hooks.jsonl` and no Claude transcript jsonl; `s2-result.json` only projects `compact_boundary` rows.
- **Required test:** `enforcer_table`, `hook_parse_golden`, `spool_drain`, `per_run_settings`, `completion_requires_both_signals`, `stray_auto_precompact_does_not_touch_managed_marker`, `unconfirmed_marker_ignored`, `no_timing_triggered_restore`, `compaction_never_submits_when_not_ready`, `no_claude_files_written`, `fire_compaction_recorded_payload`, and `paste_mismatch_fails_the_prep_and_the_next_compact_sends_again`.
