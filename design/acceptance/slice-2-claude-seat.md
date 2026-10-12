# Slice 2: Claude Code seat
Source of truth: design README. ADR-0005, ADR-0008, ADR-0009 (Am. 2), PE-001, PE-004, PE-005, PE-006. Spikes S1a, S9, S4, S13 (Mac rerun 2026-10-11; evidence `spikes/scratch/mac-rerun/`). PLAN 1.4, 2.5. TS sources: `daemon/src/adapters/claude-code-adapter.ts` (launch, `assessNativeResumeProbe`, trust auto-drive at lines 463–470), `claude-resume.ts`, `claude-managed-launch.ts`, `claude-activity-hooks.ts`.

## Scope
- `ClaudeAdapter: AgentAdapter`. Lifecycle:
  - mint a `--session-id`;
  - launch in the seat's tmux pane with per-seat `--mcp-config <seat>/mcp.json --strict-mcp-config --settings <seat>/settings.json` (ADR-0008);
  - classify the screen until `Ready`;
  - deliver with `SeatIo::submit` (paste sequence from slice 1b);
  - resume with `--resume <session-id>`.
- **Gate classifier v2 (`rig-adapters::claude::screen`)**: a pure function `classify(&Capture) -> ClaudeScreen`, built from positive matches only. It is **not** a port of `assessNativeResumeProbe`.
- **`DialogDriver`** (shared with slice 3, lives in `rig-adapters::dialog`): explicit, cursor-verified option selection (PE-006).
- **Transcript probe**: locates `~/.claude/projects/<cwd-slug>/<session-id>.jsonl` and extracts user turns. Feeds ADR-0009 `normalize_for(Claude, …)`.
- Launch timeouts derived from the Mac timings (below).

## Non-goals
- Compaction (slice 9).
- MCP tool implementation (slice 6). This slice only passes the per-seat config.
- Snapshot/restore orchestration (slice 8). This slice provides `resume` plus the probe.
- Writing `~/.claude.json` while Claude runs (PE-004: never).

## Crates touched
`rig-adapters` (`claude`, `dialog`, `screen_common`), `rig-provider::claude` (per-seat files from slice 7; the write path is reused), `rig-durability` (`TranscriptProbe` impl, normaliser), `rig-types` (`ReadinessCode`, `ComposerState`, `KnownDialog`), fixtures under `rig-adapters/tests/fixtures/screens/claude/2.1.286/`.

## Public types / traits
```rust
pub enum ClaudeScreen {
    Ready(ComposerState),            // composer + footer "? for shortcuts", no "esc to interrupt", no option menu
    Busy,                            // "esc to interrupt" present
    Dialog(KnownDialog),             // positively identified title + full option list
    Login(LoginKind),                // theme picker / login method / OAuth paste (PE-001)
    UnknownMenu { evidence: String },// any ❯/› cursor line or numbered list not matched above
    Inconclusive,                    // nothing matched (startup splash, blank)
}
pub enum KnownDialog { ClaudeTrust, ClaudeBypassConsent, ClaudeNewMcpServer { name: String },
                       ClaudeFullscreenRenderer, /* Codex variants in slice 3 */ }
pub struct DialogOption { pub text: &'static str, pub index: Option<u8> }
pub trait DialogDriver { fn answer(&self, seat: &SeatId, d: &KnownDialog, choose: &DialogOption) -> Result<Answered, Escalate>; }
pub struct LaunchBudget { pub first_screen: Duration, pub ready_after_last_gate: Duration,
                          pub inconclusive_stable: Duration, pub overall: Duration, pub poll: Duration }
```

## Launch timings (Mac, Claude Code 2.1.286 haiku) → timeouts
| Observed | Value |
|---|---|
| Already-trusted folder, `new-session` → composer | 0.55–0.82 s (5 launches) |
| Fresh folder: trust screen appears | 0.27–0.28 s |
| Fresh folder: accept → composer | 2.08–2.10 s (3 launches); one outlier at 6.42 s (`s3-timings.json`) |
| One model turn (S1a) | p50 3.0–3.6 s, max 4.6 s (64 KB) |

**Defaults** (`LaunchBudget::claude()`, all overridable in the RigSpec):
- `poll` 100 ms;
- `first_screen` 5 s (≈6× the trusted worst case);
- `ready_after_last_gate` 15 s (≈2× the 6.4 s outlier);
- `inconclusive_stable` 15 s (then `AttentionRequired(Inconclusive)`, never ready);
- `overall` 45 s.

The submit-to-transcript wait for delivery verification is 30 s.

## Acceptance criteria
**Classifier (pure, over real captures)**
1. The 4 theme-picker first-run captures (`claude-000…003-firstrun`) → `Login(ThemePicker)`, never `Ready` (PE-001).
2. Trust captures (`claude-006/018/030/039-trust`, plus the `startup` ones showing "Accessing workspace") → `Dialog(ClaudeTrust)`, with options `["No, exit", "Yes, I trust this folder"]` and the cursor on "No, exit".
3. Bypass captures (`claude-050/051`) → `Dialog(ClaudeBypassConsent)`.
4. `claude-055/056` → `Dialog(ClaudeNewMcpServer{name:"dummy"})`. The "mcpgate" captures taken before trust → `Dialog(ClaudeTrust)`.
5. The fullscreen-renderer captures (`claude-007…010` and the others matching "Try the new fullscreen renderer?") → `Dialog(ClaudeFullscreenRenderer)`.
6. Ready, after-turn, resize (80x24, 200x50, 60x20, 120x40) and seat-ready captures → `Ready`. Midturn captures containing `esc to interrupt` → `Busy`.
7. Any capture with a `❯`/`›` cursor line or a numbered list that matches no `KnownDialog` → `UnknownMenu`. This is proven with a synthetic variant (a trust dialog with a reworded title).

**Dialog handling (PE-006)**
8. Given the trust dialog and `auto_trust: false` (the default), when the launch loop sees it, then the seat goes to `AttentionRequired(ClaudeTrust)` and **no key is sent**.
9. Given `auto_trust: true`, when it is answered, then the keys are `Down` → re-capture showing `❯ Yes, I trust this folder` → `Enter`. If the re-capture shows the cursor elsewhere, it escalates without Enter.
10. Bypass consent and New MCP server are always escalated. With `--strict-mcp-config` the MCP dialog is expected not to appear; if it does, doctor reports drift.
11. Fullscreen renderer is answered "2. Not now", with cursor verification. If a per-seat `settings.json` key that suppresses it is found for the pinned version, it is written instead, and the test asserts that the dialog doesn't appear (live, gated).
12. A lint (`xtask lint-blind-enter`) fails if `rig-adapters` sends `Enter` outside `SeatIo::submit` / `DialogDriver`.

**Launch, delivery, resume**
13. Launch argv contains `--session-id <uuid>` (minted, stored before exec), `--mcp-config`, `--strict-mcp-config` and `--settings`. No file under `~/.claude*` is modified during or after the launch (checksum test against a fake HOME).
14. `SeatIo::submit` refuses unless the classifier says `Ready`. After the paste, the transcript probe confirms exactly one new user turn whose `normalize_for(Claude, …)` fingerprint equals the sent body's (ADR-0009 Am. 2).
15. `resume` relaunches with `--resume <stored id>`, and the probe finds the **same** transcript file growing (S9 evidence: 742980 → 752666 B).

## Required tests
- `claude_screen.rs`: one test per AC1–7, iterating over `tests/fixtures/screens/claude/2.1.286/*.txt` with the expected label from a checked-in `labels.jsonl` (built from `captures/index.jsonl` with the Rust label added). It also checks that **every** corpus file is labelled, so a new capture without a label fails.
- PE-001: `theme_picker_is_not_ready`, `mcp_server_found_is_attention`, `fullscreen_renderer_popup_is_attention`, `ready_requires_footer_and_no_menu`.
- PE-006: the `dialog_driver.rs` set listed in parity-exceptions, run against FakeMux replaying real captures.
- ADR-0009: `normalize_real.rs` (Claude rows of `s1-claude.json`).
- `launch_argv.rs`, `no_claude_json_writes.rs` (PE-004).
- **Live (gated, `RIG_LIVE_CLAUDE=1`, needs Sebastian's login):**
  - launch in trusted and fresh folders;
  - fullscreen suppression;
  - 3 submits including 64 KB + tabs;
  - kill-server + resume;
  - timing log compared against the budget.

## CI vs gated
- CI: everything except the live set. It runs on the fixture corpus with no Claude binary.
- Gated: the live set. **Status: the evidence it re-checks already exists from the Mac rerun** (S1a, S9, S13, S4). The live test is a regression guard, not an open blocker.

## Definition of done
- All CI tests are green, and the ledger rows for `claude-code-adapter*.test.ts` are `ported`, or `diverged` with PE-001/PE-006.
- The live set has passed once on Sebastian's Mac, with the version recorded.

## Dependencies
Slices 1, 1b; the slice 7 writers (`rig-provider::claude`). Live runs need Sebastian's Claude login (available as of the Mac rerun).

The design index calls the blind-Enter rule PE-006. In this repo that id is the setup/doctor exception, so the rule is PE-013.

## AC-to-test

| AC | What it gates | Rust test |
| --- | --- | --- |
| 1 | Theme picker is not ready | `ac1_firstrun_is_theme_picker`, `first_run_screens_are_not_ready` |
| 2 | Trust dialog, cursor on "No, exit" | `ac2_trust_dialog_defaults_to_no_exit` |
| 3 | Bypass dialog | `ac3_bypass_dialog_defaults_to_no_exit` |
| 4 | MCP dialog names the server; pre-trust stays trust | `ac4_mcp_dialog_names_the_server`, `mcp_checkbox_is_a_gate` |
| 5 | Fullscreen renderer dialog | `ac5_fullscreen_renderer_dialog` |
| 6 | Ready captures and mid-turn busy | `ac6_ready_and_midturn` |
| 7 | Reworded menu is unknown, not ready | `ac7_reworded_trust_is_an_unknown_menu`, `prose_does_not_open_a_dialog` |
| 8 | No keys when auto-trust is off | `never_enter_on_default_no_exit`, `trust_not_answered_without_opt_in` |
| 9 | Cursor verified before Enter; folder must be the seat cwd | `cursor_verification_failure_escalates`, `trust_folder_must_match_the_seat_cwd` |
| 10 | Bypass and new MCP are not auto-answered | `bypass_always_escalates`, `mcp_server_found_is_attention` |
| 11 | Fullscreen answers "2. Not now" | `fullscreen_answers_not_now` |
| 12 | Blind Enter lint, including `Key::Literal("\r")` | `planted_enter_outside_the_driver_fails`, `planted_carriage_return_literal_fails`, `carriage_return_inside_submit_fails` |
| 13 | Session id, per-seat flags, no `~/.claude` writes; refuse cwd `$HOME` | `uses_flags_not_global_config`, `home_cwd_does_not_launch` |
| 14 | Submit only when ready; paste must land before Enter; one matching turn | `submit_matches_one_turn_and_rejects_a_composite`, `submit_refuses_a_draft`, `paste_mismatch_sends_no_enter`, `draft_composer_is_attention` |
| 15 | Resume uses the stored id; the id is a UUID | `resume_uses_stored_id_flag`, `resume_rejects_a_non_uuid_token`, `resume_emits_model` |

QA additions that sit beside those criteria:

| Check | Rust test |
| --- | --- |
| Non-empty composer is attention, not ready | `draft_composer_is_attention`, `multiline_draft_is_not_ready` |
| Spinner without "esc to interrupt" is busy | `spinner_without_interrupt_is_busy`, `hook_spinner_is_busy` |
| Dialog titles are line-anchored | `prose_does_not_open_a_dialog` |
| A literal `<pasted_content` in a sentence is not a second wrapper | `literal_pasted_content_inside_one_wrapper_is_equal` |
| A torn jsonl tail is skipped and polling continues | `incomplete_trailing_jsonl_is_skipped` |
| Trust folder equals the seat cwd | `trust_folder_must_match_the_seat_cwd` |
