# Slice 9: Claude compaction control — READY FOR DEV BOT
Source of truth: design README. ADR-0008 (per-run `--settings`), ADR-0009, PE-004, PE-006, **PE-007**. Spike **S2 done** (`spikes/10-claude-compaction.md`, Claude Code 2.1.286, evidence `spikes/scratch/10-claude-compaction/`: `hooks.jsonl`, `s2-result.json`, `captures/claude-067…073`, `s2-openrig-home/`). PLAN 2.10. TS sources: `daemon/src/domain/claude-compaction-enforcer.ts` (thresholds, dedup 60 s, re-arm, prep waits 120 s manual / 25 min auto, post-compact send wait 10 s, restore cooldown 10 min, prompt builders), `adapters/stub-compaction.ts`, `assets/plugins/openrig-core/skills/claude-compaction-restore/scripts/precompact-hook.mjs` (marker `${OPENRIG_HOME}/compaction/restore-pending/<sessionKey>.json`, `.expected.json` sentinel), `hooks/scripts/compaction-restore-bridge.cjs`. TS tests: `claude-compaction-enforcer`, `compaction-*`, `precompact-hook`, `stub-compaction`, `stub-runner-compaction-e2e`, `restore-check-compact`, `queue-list-compact`.

## Evidence summary (S2, what the design relies on)
- **Hooks in a per-run `--settings` file fire.** No user, global or project file is needed: `~/.claude/settings.json` had the same sha before and after, and no `.claude/settings*.json` was created.
- **Observed order for a managed `/compact`:** `PreCompact(trigger=manual, custom_instructions=<text after /compact>)` → `SessionStart(source=compact)` → `PostCompact(trigger=manual, compact_summary=<full summary>)`.
  - The transcript gains `system/compact_boundary{trigger, preTokens, postTokens, durationMs}`.
  - Compaction took 16.1 s wall (`durationMs` 12993).
- **Stray PreCompact:** with `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=1` after resume, `PreCompact(trigger=auto, custom_instructions=null)` fired, **but no compaction followed**: no SessionStart:compact, no PostCompact, no second boundary. TS's last-writer-wins marker was then **overwritten**, from `managedRefocusPending:true` to `false, trigger:auto`.
- **Resume:** `--resume` keeps the session id, and SessionStart reports `source=resume`.
- **Memory:** Claude auto-memory wrote `~/.claude/projects/<slug>/memory/codeword.md` before compacting, so recalling the codeword does **not** prove the summary carried it.
- **Hook env:** `OPENRIG_SESSION_NAME` (ours: `RIG_SEAT_KEY`) and `CLAUDE_PROJECT_DIR`.

## Scope
- **Enforcer (pure):** `rig-health::compaction::Enforcer::decide(EnforcerInput, &PreparationState, now) -> EnforcerOutcome`. It is ported 1:1 from TS (skip reasons, dedup, re-arm, prep waits, cooldown, invalid-threshold rule), **except** for how completion is detected (PE-007).
- **Per-run settings:** each Claude launch (fresh and resume) writes `<seat>/run/<run-id>/settings.json`, passed via `--settings`. It registers `rig hook <event>` for SessionStart, UserPromptSubmit, PreCompact, PostCompact, Stop and SessionEnd, with env `RIG_SEAT_KEY`, `RIG_HOME` and `RIG_RUN_ID`. It is written atomically before exec and never edited while Claude runs (PE-004). Old runs are garbage-collected after SessionEnd.
- **`rig hook <event>`:** a Rust subcommand with no Node. It reads hook JSON from stdin and forwards a typed `HookEvent` to the daemon over the UDS (`POST /api/seats/{key}/hooks`). If the daemon is down, it appends to `<seat>/hooks.spool.jsonl`, which the daemon drains on start. It always exits 0 quickly, so it can never block Claude. The parser is golden-tested against `hooks.jsonl`.
- **Compaction state machine per seat** (in `rig-health::compaction::Attempt`, persisted in `rig-store`):
  1. `Requested`: the enforcer decided and `/compact <instruction>` was submitted via `SeatIo::submit` (requires `Ready`, PE-006).
  2. `PreCompactSeen{trigger, instructions_hash}`: the marker is written as **unconfirmed**.
  3. `Completed`: requires **both** `SessionStart{source=compact}` **and** `PostCompact{trigger}` with the same `session_id`, in either order, within `COMPLETION_WAIT` (default 120 s; ≈7× the observed 16.1 s). The `compact_summary` sha256 and length are stored. `compact_boundary` in the transcript is optional corroboration.
  4. `RestoreSent`: sent once Completed **and** the seat is `Ready`. The restore prompt goes through ADR-0009 verification (exactly one user turn).
  5. `Done`.
  Failure exits: `Aborted{reason}` (PreCompact without completion before the timeout), `Cancelled`, `Failed`.
  **Timing never triggers the restore.** TS's post-compact send wait (10 s) remains only as a lower bound before the readiness check.
- **Marker rules (fixes the S2 overwrite):**
  - Markers are keyed by `(seat_key, attempt_id)`. The file is `${RIG_HOME}/compaction/restore-pending/<seat_key>/<attempt_id>.json`, with the TS key sanitiser.
  - A marker carries `confirmed: bool` and is acted on **only** when `confirmed == true`, which is set by the daemon on `Completed`.
  - An **unsolicited** PreCompact (no `Requested` attempt, e.g. `trigger=auto`) creates a separate `auto` attempt. It never modifies, replaces or confirms a pending managed attempt's marker.
  - An auto attempt that reaches `Completed` gets the TS auto-restore behaviour. One that never completes becomes `Aborted(no_post_compact)` with no restore and no prompt.
- **Not ported:** the `.expected.json` sentinel and `compaction-restore-bridge.cjs`. Their job (knowing that compaction finished) is done by PostCompact plus SessionStart:compact. Recorded in PE-007.
- **Stub seam:** `fire_compaction(seat, trigger, complete: bool)` runs the **real** `rig hook` binary with recorded payloads from `hooks.jsonl`, so CI exercises the real parser and spool path.

## Non-goals
- Codex/OpenCode compaction.
- Persisting *enforcer* sample state across daemon restarts (TS deliberately doesn't). Attempt rows **are** persisted, so a restart mid-attempt resumes from the spool.
- A new compaction policy: thresholds and prompts are parity.
- Suppressing Claude auto-memory. It is not touched; the continuity proof is designed around it (AC12).

## Crates touched
`rig-health` (`compaction`), `rig-adapters::claude` (submit only), `rig-provider::claude` (per-run settings writer), `rig-cli` (`compact`, `hook <event>`), `rig-api` (compaction routes), `rig-store` (attempt rows).

## Public types / traits
```rust
pub enum HookEvent {
    SessionStart { session_id: String, source: SessionSource /* Startup|Compact|Resume|Other(String) */, context_tokens: Option<u64> },
    UserPromptSubmit { session_id: String },
    PreCompact { session_id: String, trigger: CompactTrigger /* Manual|Auto */, custom_instructions: Option<String>, transcript_path: PathBuf },
    PostCompact { session_id: String, trigger: CompactTrigger, compact_summary: String },
    Stop { session_id: String }, SessionEnd { session_id: String, reason: String },
    Unknown { name: String, raw: serde_json::Value },   // forward-compatible: logged, never fails the hook
}
pub enum AttemptState { Requested, PreCompactSeen, Completed { summary_sha: [u8;32] }, RestoreSent, Done, Aborted(AbortReason), Cancelled, Failed(String) }
pub struct RestoreMarker { pub seat_key: String, pub attempt_id: AttemptId, pub trigger: CompactTrigger, pub managed: bool, pub confirmed: bool, pub session_id: String, pub transcript_path: PathBuf, pub cwd: PathBuf }
pub const COMPLETION_WAIT: Duration = 120s;
```

```rust
pub struct EnforcerInput { pub seat: SeatId, pub used_percent: Option<u8>, pub sample_at: Option<Instant>, pub threshold: Threshold, pub ready: bool, .. }
pub enum EnforcerOutcome { Skip(EnforcerSkipReason), SendPrep(String), SendCompact(String), SendRestore(String) }
pub enum ManualCompactionStage { Preparing, WaitingMap, Compacting, Restoring, Done, Cancelled, Failed }
pub struct Threshold(Option<u8>); impl Threshold { pub fn parse(raw: &serde_json::Value) -> Self; }
pub const DEDUP_WINDOW: Duration = 60s; pub const POST_COMPACT_RESTORE_COOLDOWN: Duration = 600s;
pub const MANUAL_PREP_WAIT: Duration = 120s; pub const AUTO_PREP_WAIT: Duration = 1500s; pub const POST_COMPACT_SEND_WAIT: Duration = 10s;
```

## Acceptance criteria
**Enforcer parity**
1. Every TS enforcer test case is ported as a table test against `decide()` with an injected clock. Prompt strings are byte-equal to TS (goldens via `xtask diff`). The invalid-threshold rule (0, 101, NaN, non-integer → disabled) is covered.
2. A threshold crossing → `SendPrep` first. `/compact` is sent only after the completed map exists, or once with `--skip-map`. Re-arm requires dropping below the threshold. `rig compact <seat> --cancel` ends the attempt in `Cancelled`.
3. `/compact`, the prep and the restore prompts go only through `SeatIo::submit`. A non-`Ready` seat makes the attempt wait within its budget, then fail visibly. It never sends keys to a dialog (PE-006).

**Hooks and settings**
4. Every launch (fresh and `--resume`) gets a fresh per-run `settings.json` registering the six hooks, and the launch argv contains `--settings <that path>`. No user, global or project Claude file is created or modified (checksum of `~/.claude/settings.json`, `~/.claude.json`, and absence of `<cwd>/.claude/settings*.json`), per PE-004.
5. `rig hook` parses every line of S2's `hooks.jsonl` into the right `HookEvent` variant (golden). An unknown event or extra keys → `Unknown`/ignored, exit 0. A malformed stdin → logged, exit 0. With the daemon down, the event lands in the spool and is drained in order on daemon start.

**Completion and restore (PE-007)**
6. Given the S2 manual sequence replayed (`PreCompact:manual` → `SessionStart:compact` → `PostCompact:manual`), then the attempt goes `Requested → PreCompactSeen → Completed`, the marker gets `confirmed=true`, and exactly one restore prompt is submitted after the seat is `Ready`.
7. Completion requires **both** `SessionStart{source=compact}` **and** `PostCompact`, with the same `session_id`, in either order. Either one alone → no restore. A `session_id` mismatch → ignored and logged.
8. **Stray PreCompact (from S2 evidence):** given a managed attempt with a pending marker, when `PreCompact{trigger=auto, custom_instructions=null}` arrives with no following SessionStart:compact/PostCompact (the exact S2 post-resume payload), then:
   - the managed marker file is **byte-unchanged**;
   - a separate auto attempt is created;
   - after `COMPLETION_WAIT` it becomes `Aborted(no_post_compact)`;
   - **no** restore prompt is sent for either attempt by this event.
9. A marker with `confirmed=false` is never acted on, by the enforcer, restore-check or crash-cart.
10. No restore is ever sent because of elapsed time alone. A test advances the clock past every TS wait with no PostCompact and asserts that no restore happens.
11. The restore prompt passes ADR-0009 verification (exactly one user turn, `normalize_for(Claude)`). `compact_summary` sha/len are stored on the attempt and shown by `rig compact <seat> --status`.

**Continuity proof (memory-proof)**
12. The live proof uses a check that Claude auto-memory cannot satisfy:
    - (a) **Fresh secret per run:** a 128-bit random nonce `N`, never reused, so memory from earlier runs can't contain it.
    - (b) **Mechanical primary check:** `N` (planted in turn 1 with the instruction "include this token in any summary") must appear in the **PostCompact `compact_summary`** captured by the hook. This proves the summary carried it, independently of what the model later says.
    - (c) **Memory exclusion:** before and after the run, scan `~/.claude/projects/<cwd-slug>/memory/` (read-only). If any memory file contains `N`, the recall check is marked **inconclusive (memory-contaminated)**, and the test passes only on (b).
    - (d) **Restore delivery:** the restore prompt carries a second fresh nonce `R` that the model must echo. This proves the post-compact prompt landed once and was read.
    The PELICAN-style recall alone is explicitly **not** accepted as proof.
13. Restore-check and `queue list` show compaction state as in TS (`restore-check-compact`, `queue-list-compact` goldens), including the new `Aborted` state.

## Required tests
- **Ported TS tests:** enforcer, preparation, generation, clock-injection, drain-readiness, routes, precompact-hook (as `rig hook` tests), stub-compaction, stub-runner e2e.
- **New tests, all CI with FakeMux and the stub seam using recorded payloads from `spikes/scratch/10-claude-compaction/hooks.jsonl` (copied to `tests/fixtures/hooks/claude/2.1.286/`):**
  - `hook_parse_golden.rs` (AC5)
  - `spool_drain.rs`
  - `per_run_settings.rs` (AC4)
  - `completion_requires_both_signals.rs` (AC7)
  - **`stray_auto_precompact_does_not_touch_managed_marker.rs`** (AC8, replaying the S2 post-resume PreCompact:auto payload)
  - `unconfirmed_marker_ignored.rs` (AC9)
  - `no_timing_triggered_restore.rs` (AC10)
  - `compaction_never_submits_when_not_ready.rs` (PE-006)
  - `no_claude_files_written.rs` (PE-004)
- **Live (gated `RIG_LIVE_CLAUDE=1`, Sebastian's Mac):**
  - **L1, manual compaction end to end:** done once by S2 (2026-10-11, 2.1.286). It is re-run as a regression with the AC12 memory-proof check.
  - **L2, `--resume` keeps the session id and per-run settings:** done once by S2.
  - **L3, genuine threshold-driven auto-compaction: UNTESTED, gated.** Fill the context to a real threshold, then assert PreCompact:auto → SessionStart:compact → PostCompact:auto → the TS auto-restore path. `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` did **not** produce a compaction in S2, so this needs a long context. It does not block DoD; it is tracked in the README.

## CI vs gated
CI: everything except L1–L3. Gated: L1–L3. L1/L2 have passing spike evidence; L3 is open.

## Definition of done
- All CI tests green.
- Ledger rows `ported`, or `diverged` + PE-007 (completion detection, sentinel/bridge dropped).
- L1 with the AC12 proof passed once on the pinned Claude version.
- L3 listed as an open gated test (non-blocking).

## Dependencies
- Slice 2: Claude seat, `SeatIo`, transcript probe, `--settings` launch.
- Slice 4: `queue list` interplay.
- Slice 7: the atomic per-seat writer, reused for per-run settings.
- Slice 0: UDS route for hook events.
No Sebastian blockers for CI work.
