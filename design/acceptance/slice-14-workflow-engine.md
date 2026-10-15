# Slice 14: workflow engine
Source of truth: design README. PLAN 3.1. Porting map: ~13k, "mostly pure logic; good test-port candidate".
TS sources:
- `domain/workflow-runtime.ts`: the facade over the spec cache, validator, instance store, projector and trail log. Operations: `validate`, `instantiate`, `project`, `continue` (idempotent).
- **`workflow-projector.ts`, load-bearing**: closing the current-frontier qitem and creating the next-step qitem **must happen in the same transaction**, so "lost handoffs are impossible by design".
- `workflow-validator`, `-spec-cache`, `-instance-store`, `-step-trail-log`, `-role-resolver`, `-role-context`, `-guidance`, `-deadline`, `-keepalive-arming` (arms or disarms a watchdog job inside the scribe transaction), `-exception*`, `-frontier-guard`, `-reconciliation`, `-boot-sweep`, `-human-destination`, `-planning-context`, `domain/workflow/` (starter-spec-loader, slice bindings).
- Built-in specs in `daemon/src/builtins/workflow-specs/*.yaml`, with shape `workflow: {id, version, objective, target.rig, entry.role, coordination_terminal_turn_rule, roles{skill_refs, preferred_targets}, steps[{id, actor_role, objective, allowed_exits, next_hop{mode, suggested_roles}}]}`.
- Routes `routes/workflow.ts`: validate, compile, instantiate(-lifecycle), project, list, specs, sse/watch, route, resume, abort, trace, continue, guidance, revision, operations. CLI `workflow.ts` (+follow, render, errors).

## Scope
All of the above, in-scope parts: the spec model and validation (serde-saphyr, ADR-0007), the instance store, the projector with the single-transaction contract on top of slice 4's `QueueRepo::update_within_transaction`, role resolution against rig seats, guidance, deadlines, keepalive watchdog arming, exceptions/escalation, the frontier guard, reconciliation, the boot sweep after a daemon restart, trace and trail log, the revision flow, the built-in specs embedded as assets, and the CLI with `--json`.

## Non-goals
- Bundle-carried workflow specs (out of scope).
- Human-destination delivery via Slack (out of scope). Human destinations are resolved to the operator surface: the TUI attention view, the CLI inbox, and optional notify-only ntfy pings (G3 closed).
- UI-only routes.

## Crates touched
`rig-orchestration::workflow`, `rig-spec` (workflow spec types), `rig-messaging` (queue transaction API), `rig-store`, `rig-daemon`, `rig-api`, `rig-cli`.

## Public types
```rust
pub struct WorkflowSpec { id, version: u32, objective: String, target: Target, entry: Entry, coordination_terminal_turn_rule: TurnRule, roles: BTreeMap<RoleId, RoleSpec>, steps: Vec<StepSpec> }
pub struct StepSpec { id: StepId, actor_role: RoleId, objective: String, allowed_exits: Vec<Exit>, next_hop: NextHop }
pub enum Exit { Handoff, Waiting, Done, Failed, .. /* TS enum */ }
pub struct WorkflowInstance { id, spec: (SpecId, u32), current_frontier: QItemId, state, created_by: SessionRef, revision: u32 }
pub struct Projector;  impl Projector { pub fn project(&self, tx: &Transaction, inst: &WorkflowInstance, exit: Exit, ctx: &ProjectCtx) -> Result<Projection>; } // closes + creates in tx
pub fn validate(spec: &WorkflowSpec, rig: Option<&RigView>) -> Vec<ValidationIssue>;
pub trait WorkflowRuntime { fn instantiate(..); fn project(..); fn continue_(..) /* idempotent */; fn trace(..); fn guidance(..); }
```

## Acceptance criteria
1. Every built-in spec and every TS test fixture spec parses and validates with the same issues as TS (codes and paths). Error messages match the TS text where skills or docs quote them.
2. **Atomic projection.** Given an instance at step k, when `project(exit=handoff)` runs, then closing the frontier qitem, creating the next qitem, updating the instance frontier, appending the trail log and arming or disarming the keepalive all commit in **one** transaction. A crash injected between any two of these leaves the before-state. No observer (a concurrent reader) ever sees a closed frontier without its successor.
3. `continue` is idempotent: N calls cause one advance at most per legal exit.
4. Role resolution picks `preferred_targets` in order and then falls back per TS. An unresolved role produces an exception routed through the escalation path, not a silent park.
5. Deadlines and keepalive: a deadline firing (fake clock) yields the TS exception kind. A terminal exit disarms the keepalive in the same transaction.
6. Boot sweep: after a daemon restart, instances whose frontier is inconsistent (e.g. a qitem closed with no successor because of a TS-era bug or manual edit) are reported and reconciled per TS rules.
7. CLI/route goldens for validate, instantiate, project, list, show, trace, guidance, continue and route/resume/abort (normalised).
8. The diff harness runs one built-in spec end to end (instantiate → 3 handoffs → done) on stub seats against TS v0.6.9, and the outputs match after normalisation.

## Required tests
Ported projector/validator/runtime unit tests (the bulk of the TS workflow tests), crash-injection tests (AC2), concurrent-reader test (AC2), fake-clock deadline/keepalive tests (AC5), boot-sweep (AC6), goldens and diff case (AC7–AC8). Ledger: all `workflow-*` tests mapped.

## CI vs gated
All in CI on stub seats. Live smoke of skills that drive workflows with real agents is part of the slice 20 dogfood (gated on logins).

## Definition of done
AC1–AC8 pass, the workflow ledger rows are closed (no `todo`), and notice headers are present.

## Dependencies
After PR #1. **Needs slices 0, 1 and 4** (queue transaction API, nudges, watchdog/events). **Not** 2/3. Slices 15–19 depend on this one (TS order: workflow first).
