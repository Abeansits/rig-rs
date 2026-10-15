# Slice 12: health detectors, health projection/diagnosis, stream + classifier
Source of truth: design README. PLAN 3.7. Porting map: "Health detectors / stream classifier", ~5.4k.
TS sources:
- `domain/health-detectors.ts` (591): observation kinds `coordination-lineage`, `wake-lineage`, `directive`, `scope-admission`, `context-pressure`. Each source reports **coverage** (what it evaluated); an omitted item is "not evaluated", never "healthy". A policy failure fails the query, and a limit is never fabricated as a zero count. **"The live v1 adapter intentionally supplies only context observations"**; the other detectors need replay sources.
- `health-projection.ts`, `health-diagnosis.ts`, `health-policy.ts`, `health-passive-ceremony.ts`, `health-checkpoints.ts`, `health-context.ts`, `context-monitor.ts`, `capture-observer.ts`.
- Stream: `stream-store`, `stream-classification-worker.ts` (labels type/urgency/maturity/confidence/destination/area/scopeRef, needsHuman, duplicateOf with positive evidence only, `abstain | classify` decisions), `classification-attempts.ts`, `classifier-lease-manager.ts`. There is a **single active lease**, enforced by a partial unique index, with TTL and heartbeat; it is marked expired when stale, and **reclaim is operator-verb only**.
- Routes: `health.ts` (`/`, `/:findingId`), `health-diagnosis.ts` (`/policy`, `/checkpoints`, `/`, `/:id`), `health-summary.ts` (`/nodes`, `/context`, `/version`; `/gateway` is out of scope), `stream.ts` (`emit`, `list`, `watch/sse`, `/:id`, `archive`).
- CLI: `health`, `stream`, `usage`, `telemetry`, `watchdog`.

## Scope
- The health detector framework with coverage semantics, the policy, the projection into findings, the diagnosis records, and the health checkpoints.
- **Live sources in v1** (TS parity): context-pressure from seat context usage, through the adapter `usage_telemetry` capability. The stub reports synthetic usage; Claude/Codex/OpenCode wiring lands in their slices.
- **Replay sources**: coordination/wake lineage from queue transitions (slice 4 data), plus directive and scope-admission where data exists. Each is behind the same `HealthSource` trait. Sources whose input tables don't exist yet report `coverage: none`, not "healthy".
- The stream: emit, list, watch, archive. The classifier lease (acquire, heartbeat, expire, operator reclaim) and the attempt ledger. Classification itself is done **by an agent seat holding the lease**; the daemon stores the decisions and never calls a model itself.
- CLI `rig health [--json]`, `rig stream …`, `rig usage`. Watchdog jobs needed by the lease and keepalive.

## Non-goals
- The gateway health summary (Slack, out of scope).
- Designing new detectors.
- The UI beyond the TUI's health summary (slice 13 reads this API).

## Crates touched
`rig-health` (detectors, policy, projection, diagnosis, stream classification domain), `rig-store`, `rig-daemon` (routes, watchdog jobs), `rig-api`, `rig-cli`.

## Public types
```rust
pub enum HealthObservation { CoordinationLineage{..}, WakeLineage{..}, Directive{..}, ScopeAdmission{..}, ContextPressure{ seat, used_tokens, limit, at } }
pub struct Coverage { evaluated: u32, omitted: u32, limit_hit: bool }
pub trait HealthSource { fn detectors(&self) -> &[DetectorId]; fn read(&self, q: &HealthQuery) -> Result<(Vec<HealthObservation>, Option<Coverage>), SourceError>; }
pub struct HealthPolicy { enabled: BTreeSet<DetectorId>, thresholds: .. }
pub struct HealthRecord { id: FindingId, detector: DetectorId, severity, subject, evidence: Vec<EvidenceRef>, coverage: Option<Coverage> }
pub fn project(policy: &HealthPolicy, reads: Vec<SourceRead>) -> Result<HealthProjection, HealthError>; // pure
pub struct StreamItem { id, kind, body, labels: WorkerLabels, archived: bool }
pub enum ClassificationDecision { Abstain{reason}, Classify{labels: WorkerLabels} }
pub struct ClassifierLease { id, holder: SessionRef, acquired_at, expires_at, last_heartbeat, state: Active|Expired|Released }
```

## Acceptance criteria
1. Coverage honesty: given a source that evaluated 40 of 100 items (limit), the projection reports `limit_hit` and the omitted count. It never reports the omitted items as healthy. Given a failing source, its read is discarded and the failure is named in the result.
2. A policy failure fails the whole query (TS rule). A disabled detector is skipped without a read.
3. Context-pressure: given stub seats reporting 50%, 85% and 97% of the context limit, findings appear at the TS policy thresholds, and the primary signal stays visible under the query cap (it is "reserved visibility", ahead of a hundred context samples).
4. Lineage replay: given queue transitions with a failed wake that was never escalated, a wake-lineage finding is produced. Given a correctly escalated ladder, none is produced.
5. `rig health --json` and the `/api/health*` route goldens match the TS shapes for the in-scope fields.
6. Classifier lease: there is at most one active lease, enforced at DB level (a second acquire returns `LeaseHeld`). A stale heartbeat past the TTL is marked `Expired` by evaluation, never reclaimed automatically. `rig stream --reclaim-classifier` (an operator verb) reclaims it.
7. Classification decisions: `Classify` labels must come from the candidate set's version. Values outside it are rejected. `duplicateOf` requires an evidence ref from the supplied candidates. `Abstain` is recorded with a reason. Attempts are ledgered and capped (`MAX_ELIGIBLE_PAGE`).
8. Stream emit, list, watch and archive work over UDS, including watch resume.

## Required tests
- Pure projection/policy tests (AC1–AC4), ported from the TS health tests.
- Lease DB tests, including the partial-unique-index race (AC6).
- Classification validation (AC7).
- Route and CLI goldens (AC5, AC8).
- Ledger: health-*, stream-*, classifier-* and context-monitor tests are mapped. Gateway health is `waived` (out of scope).

## CI vs gated
- **CI:** everything, using stub usage telemetry and synthetic queue transitions.
- **Gated (logins / provider key):** real context usage from Claude, Codex and OpenCode. This is verified in slices 2, 3 and 10 against `usage_telemetry` and does not block this slice.

## Definition of done
AC1–AC8 pass, ledger rows are mapped, notice headers are on derived files.

## Dependencies
- After PR #1. **Needs slices 0, 1 and 1b** (seats, capture observer input), and **slice 4** for lineage replay sources and the events stream. Without slice 4, the lineage detectors report `coverage: none`, and AC4 lands when slice 4 merges.
- **Not** slice 2/3/10 for CI.
