# Slice 8: snapshots / restore / restore-check / crash-cart (stub runtime)
Source of truth: design README. ADR-0003 (backup API, checkpoints off the writer), ADR-0009 (text-hash identity), PE-005 (conditional), spike S9 (`spikes/5-resume-restore.md`). PLAN 2.9.
TS sources:
- `domain/restore-orchestrator.ts` (1.9k):
  - per-seat outcomes `resumed | fresh | attention_required | failed | operator_recovered`
  - rig rollup with `partially_restored`; **`ready` is forbidden as a post-reconciliation outcome**
  - launch order constrained only by specific edge kinds
  - a four-way active-occupant ladder (`active-occupant.ts`)
  - runtime-truth reconciliation is a **no-op with a reason** unless all four evidence preconditions hold
- `restore-check-service.ts` (1.6k): ready-confidence classes ready / ready_with_caveats / not_ready / attention_required / unknown; each remediation is flagged **execution-safe or not**, defaulting to unsafe.
- `checkpoint-store.ts`: agent checkpoints (summary, current task, next step, blocked on, key artifacts, confidence).
- `crash-cart-conductor.ts` / `-discovery.ts` / `-detect.ts` / `-probes.ts`:
  - fleet restore: kernel rig first, then the others in sequence, best-effort
  - cancel is **stop-before-next-rig**
  - per-rig outcome `fully_restored | partially_restored | failed | not_attempted`
- `cli/src/restore-packet/`, `cli/src/commands/{snapshot,restore,restore-check,crash-cart}.ts`.
- Test-system: `kill-daemon-mid-handoff`, `locked-restore-never-permissive`, `compaction-restore-resumes-role`, `queue-baton-survives-restart`.

## Scope
- **Snapshots**: a rig snapshot captures the DB slice for the rig (seats, sessions, resume tokens, queue items in flight, checkpoints, policies/posture) via the SQLite backup API plus a manifest (sha256), stored under `$RIG_HOME/snapshots/<rig>/<id>/`. Commands: `rig snapshot create|list|show`.
- **Restore**: `rig restore <rig> [--snapshot id]` relaunches seats in dependency order, using each adapter's resume token where one exists (`launch` with `resume`), and reconciles in-flight deliveries (below). It produces per-seat and rollup outcomes with TS semantics.
- **Restore-check**: `rig restore-check [--json]` is a read-only readiness report with confidence classes and remediation entries flagged `execution_safe`.
- **Crash-cart**: discovery of rigs that died (tmux server gone, daemon restarted, seats dead), a report, and a conductor `rig crash-cart restore [--cancel]`. The TUI view for crash-cart is a later TUI slice.
- **Checkpoints**: `rig checkpoint`-style write/read used by restore guidance.
- **In-flight delivery reconciliation**, split by channel (ADR-0009 Amendment 1, gap G2 closed):
  - **Queue obligations** (rows) are **never re-pasted**. On restore, items stay in their DB state. Items that were handed off and unconfirmed are **re-nudged** through slice 4's ladder. A nudge is idempotent (a pointer), so at-least-once is safe. This matches the TS invariant, so it is parity.
  - **Direct `rig send` messages and startup-file deliveries** that were recorded as delivered but have no ack: **re-send only if the seat's transcript has no user turn matching the item's text-hash fingerprint after its delivery cursor** (ADR-0009). If a match is found, ack it. Identical bodies are counted by occurrence, and contaminated turns escalate.

## Non-goals
- Real Claude `--resume` and Codex thread-id resume. These are adapter work in slices 2/3, and this slice's live variants are gated on them (below).
- Bundles, restore packets for cross-host (out of scope), and the crash-cart TUI.
- Compaction-triggered restore (slice 9 consumes this slice).

## Crates touched
`rig-durability` (snapshot, restore orchestrator, restore-check, crash-cart, checkpoints, reconcile), `rig-store` (backup API on the maintenance connection, checkpoint tables), `rig-adapters` (uses `discover_resume_token`, `launch(resume)` and `TranscriptProbe`; stub implementations here), `rig-messaging` (re-nudge entry point), `rig-daemon`, `rig-api`, `rig-cli`.

## Public types
```rust
pub struct SnapshotManifest { id: SnapshotId, rig: RigId, created_at, schema_version: u32, sha256: String, seats: Vec<SeatSnapshot>, in_flight: Vec<InFlightDelivery> }
pub struct SeatSnapshot { seat: SeatId, runtime: RuntimeKind, resume: Option<ResumeToken>, transcript_cursor: Option<TranscriptCursor>, posture: LaunchPosture, startup_status }
pub enum InFlightDelivery { QueueRow { qitem: QItemId, state: QueueState }, Direct { id: DeliveryId, seat: SeatId, fingerprint: DeliveryFingerprint, delivered_at, cursor: Option<TranscriptCursor> } }
pub enum SeatRestoreOutcome { Resumed, Fresh, AttentionRequired(ReadinessCode), Failed(String), OperatorRecovered }   // never "Ready" post-reconcile
pub enum RigRestoreOutcome { FullyRestored, PartiallyRestored, Failed, NotAttempted }
pub struct RestorePlan { order: Vec<Vec<SeatId>> /* waves by constraining edge kinds */, reconcile: Vec<ReconcileAction> }
pub enum ReconcileAction { ReNudge(QItemId), Ack(DeliveryId), ReSend(DeliveryId), Escalate(DeliveryId, Reason) }
pub fn plan_reconcile(inflight: &[InFlightDelivery], probe: &dyn TranscriptProbe) -> Result<Vec<ReconcileAction>>; // pure over probe
pub struct RestoreCheckReport { classes: ConfidenceBreakdown, seats: Vec<SeatCheck>, remediations: Vec<Remediation> }
pub struct Remediation { action: String, execution_safe: bool /* default false */ }
pub struct CrashCartRun { rigs: Vec<(RigId, RigRestoreOutcome)>, cancelled_before: Option<RigId> }
```

## Acceptance criteria
**Snapshots**
1. `snapshot create` runs the backup on the maintenance connection (never the writer; ADR-0003). Writer p99 is unaffected during the snapshot (S5 bench threshold). The manifest's sha256 verifies, and a corrupted snapshot is refused by restore with a clear error.
2. `snapshot list/show --json` goldens. A snapshot of a newer schema version is refused.

**Restore (stub runtime)**
3. Given a 3-seat stub rig with a constraining edge, `tmux kill-server`, and a daemon restart, `rig restore` relaunches the seats in wave order. Seats with a resume token get `launch(resume)` and become `Resumed` (the stub honours resume by replaying its transcript). Seats without one become `Fresh`. The rollup is `FullyRestored`.
4. A seat whose relaunch hits a gate becomes `AttentionRequired(code)`, and the rollup is `PartiallyRestored`. A seat whose reconciliation lacks any of the four evidence preconditions gets a no-op plus a reason (not an error). **No seat is ever reported `Ready` as a restore outcome.**
5. `locked-restore-never-permissive`: given a seat whose posture was floor/locked, restore never relaunches it with a more permissive posture.

**Delivery reconciliation (ADR-0009 text-hash)**
6. **Queue rows.** Given items mid-handoff at the crash, after restore every row is in exactly its pre-crash committed state, and handed-off-unconfirmed items are re-nudged (a pointer). No item body is pasted by restore. `queue-baton-survives-restart` and `kill-daemon-mid-handoff` pass.
7. **Direct deliveries, crash between paste and ack.** Given direct messages delivered to stub seats, with crashes injected (a) before the paste, (b) after the paste and before the ack, (c) after the ack, restore yields exactly once per message in the stub transcripts, across 18 messages on 3 seats (this replaces the S9 setup):
   - (a) → ReSend
   - (b) → Ack, via fingerprint match
   - (c) → nothing
   - The histogram must be `{1: 18}`.
8. Identical bodies: given the same text sent twice to one seat and only one present in the transcript after the cursor, exactly one ReSend is planned.
9. Normalisation: CRLF vs LF, NFC vs NFD and trailing whitespace still match. Internal whitespace changes do **not** match.
10. Contaminated: given a transcript user turn that contains the payload plus extra text, the action is `Escalate`, not Ack or ReSend.

**Restore-check and crash-cart**
11. `restore-check --json` reports the five classes. Every remediation has `execution_safe`, and mutating remediations (daemon start, chmod, create files, snapshot) are `false`. Golden scenarios: all-ready, a dead seat, a missing resume token, a daemon that is down.
12. The crash-cart conductor restores the kernel rig first, then the others in sequence. One rig failing doesn't stop the others. `--cancel` during rig k lets k finish, and rigs after k are `NotAttempted`.

## Required tests
- Pure: `plan_reconcile` table tests (AC7–AC10); normalisation (AC9); wave ordering; outcome rollup (AC4).
- Stub e2e with real tmux: AC3–AC7, AC12.
- test-system on stub: `kill-daemon-mid-handoff`, `locked-restore-never-permissive`, `queue-baton-survives-restart`. `compaction-restore-resumes-role` waits for slice 9.
- **PE-005 (direct sends and startup files only).** Before closing the slice, check TS `restore-orchestrator.ts`, `session-transport.ts` and the startup-delivery paths for whether TS ever re-pastes direct/startup content on restore. Queue rows are parity (pointer nudge only) and are not part of PE-005.
  - If TS **does** re-send direct/startup deliveries without transcript reconciliation: keep PE-005, and ledger the asserting TS tests as `diverged` + PE-005.
  - If TS doesn't re-send direct deliveries at all: our ack/re-send is an addition. Keep it as PE-005 ("added reconciliation").
  - If TS already reconciles: delete PE-005 and mark the rows `ported`.
- Ledger: restore-orchestrator, restore-check, checkpoint-store, crash-cart and snapshot CLI tests are mapped.

## CI vs gated
- **CI:** everything on the stub runtime and real tmux ≥ 3.3.
- **Gated, live (needs logins; runs after slices 2/3 live. The resume mechanics are already proven on the Mac, S9 rerun: Claude `--resume` and Codex `resume <thread-id>` append to the same transcript after kill-server):**
  - real Claude `--resume <minted session-id>` after kill-server;
  - Codex `resume <thread id>` from the rollout file;
  - transcript fingerprint matching against real Claude jsonl (including large-paste fidelity) and Codex rollouts.
- **Gated (OpenCode provider key, after slice 10):** OpenCode resume by `ses_` id, which is already shown in S9 without a model turn.
- None of these block this slice's CI DoD.

## Definition of done
- AC1–AC12 pass on stub. The listed scenarios are green.
- The PE-005 check is done and recorded (kept, re-scoped or removed).
- Notice headers are on derived files.

## Dependencies
- After PR #1. **Needs slices 0, 1, 1b and 4** (queue rows, nudge path, delivery receipts with fingerprints, transcript writer).
- Live variants also need slices 2/3 (and 10 for OpenCode).
- **Does not need slices 2/3 for CI.**
