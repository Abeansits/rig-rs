# Slice 4: queue / send / ask / wake ladder + human-typing guard verdict

Source of truth: design README. ADR-0005 (guard), ADR-0009 (delivery identity), PE-005. PLAN 2.4/2.7.

TS sources:

- `domain/queue-repository.ts` (4.1k). `QUEUE_STATES` = pending, in-progress, done, blocked, failed, denied, canceled, handed-off; claim, handoff, handoff-and-complete, fallback, inbox/outbox.
- `domain/queue-wake-ladder.ts` (1.3k). Invariant: "the row carries the obligation exactly-once; the wake is at-least-once. The ladder retries the nudge (an envelope pointer at the row), never the content." Bounded retries, then escalation rungs (destination orchestrator, aggregated, then operator), every step a recorded transition.
- `ask-service.ts` (L1 seat / L2 session transcript archaeology plus chat history), `claim-service.ts`, `inbox-handler.ts`, `session-transport.ts` (`rig send`: refuses interactive prompts unless `--dangerously-interact --reason`; multi-recipient `--to a,b`), `non-interruptive.ts`.
- `routes/queue.ts` (create/claim/unclaim/update/handoff/handoff-and-complete/fallback/list/overdue/undelivered/recent-transitions/watch/sse/inbox/outbox), `routes/ask.ts`, `routes/events.ts`.

## Scope

- Queue: items with the TS state machine, transitions log, claim/unclaim, handoff (incl. hot-potato closure validation), handoff-and-complete, fallback, block/park with a human-seat blocker, inbox drop/absorb/deny, outbox record/list, overdue/undelivered views, retention sweep.
- Wake ladder: the nudge is a short pointer text ("you have queue item X — run `rig queue show X`" style) delivered via the guard. It is at-least-once, with bounded retries, then escalation rungs and the `queue.wake_human_rung` setting (`always` | `explicit-only`), all as transitions. Ladder state is derived from the transition log, as in TS.
- `rig send` direct message delivery (single, `--to` list, pod/rig fan-out with per-seat `To:` header), with the interactive-prompt refusal.
- Guard verdict: `guard::evaluate(GuardInputs) -> GuardVerdict` (ADR-0005 rules 1–6), daemon input taps (`last_human_keystroke_tap` from terminal WS / attach proxies), and delivery receipts (`delivered | deferred | escalated | contaminated`, with a `guard_degraded` flag).
- `rig ask`: L1/L2 transcript search plus chat history (chat history is read via slice 5's repository when present, otherwise empty).
- Events: `GET /api/events` (WS/SSE over UDS) fan-out of `DomainEvent`s plus `/api/queue/watch`. This slice's `/api/events` is canonical (event envelope, `last_event_id` resume, bounded backlog, `gap` event). Slice 13's minimal version must be reconciled toward this one: same path, a `StreamEvent` envelope in the api crate, and resume semantics. A slice 4 AC18 test runs the TUI's event client against the canonical endpoint.
- Seat transcripts writer: the daemon persists per-seat pane output from control-mode `%output` to rotated transcript files, which `rig ask` L1 reads.

## Non-goals

- Composer parsers for Claude/Codex (slices 2/3). Here only the stub runtime's composer parser exists, plus `ComposerState::Unknown`, which gives a degraded guard.
- Restore re-delivery (slice 8).
- Slack/human gateway delivery of human rungs. The operator rung lands in the attention surface/TUI only.
- Mission-control verbs (slice 18), which reuse `QueueRepository::update_within_transaction` from here.
- Workflow projector (slice 14).

## Crates touched

messaging (queue domain, wake ladder, claims, inbox/outbox, ask), mux::guard (verdict, pure), store (queue tables + transitions), daemon (routes, events, input taps, transcript writer), api, cli (`rig queue …`, `rig send`, `rig ask`, `rig broadcast`, `rig unclaim`, `rig whoami`).

Crate names in this tree are `crewmux-*`. The binary is `rig`.

## Public types / traits

```rust
pub enum QueueState { Pending, InProgress, Done, Blocked, Failed, Denied, Canceled, HandedOff }
impl QueueState { pub fn is_terminal(self) -> bool; }   // one predicate, shared by archiver + guards (TS FS-1)
pub struct QueueItem { id: QItemId, source: SessionRef, destination: SessionRef, state: QueueState, priority, tier, tags, blocked_on: Option<SessionRef>,
                       handed_off_to/from, expires_at, closure: Option<Closure>, claimed_at, last_nudge: Option<NudgeRecord>, body: String, fingerprint: DeliveryFingerprint }
pub struct Transition { id: u64, qitem: QItemId, from: QueueState, to: QueueState, note: String, at: Timestamp, actor: Actor }
pub trait QueueRepo { fn create(&self, tx, NewItem) -> Result<QItemId>; fn update_within_transaction(&self, tx, QItemId, Update) -> Result<Transition>; .. }
pub struct WakeLadder { cfg: WakeConfig }   impl WakeLadder { pub fn tick(&self, view: &LadderView, now: Timestamp) -> Vec<WakeAction>; } // pure, view derived from transitions
pub enum WakeAction { Nudge { qitem, target }, Escalate { rung: Rung, aggregate_key }, Suspend, Exhausted }
pub enum Rung { Retry(u8), DestinationOrchestrator, Operator }
pub enum WakeHumanRung { Always, ExplicitOnly }
pub fn evaluate(i: &GuardInputs, composer: ComposerState, cfg: &GuardConfig, now: Instant) -> GuardVerdict;  // mux::guard
pub enum GuardVerdict { Deliver, Defer { retry_after: Duration, reason: DeferReason }, Escalate(DeferReason) }
pub enum DeferReason { NotAtPrompt, InCopyMode, HumanTyping, HumanDraft, Interactive(ReadinessCode) }
pub struct DeliveryReceipt { outcome: Delivered|Deferred|Escalated|Contaminated, guard_degraded: bool, fingerprint: DeliveryFingerprint, at: Timestamp }
pub trait SeatInputTap { fn record_keystroke(&self, seat: &SeatId, at: Instant); }
pub struct AskQuery { rig, seat: Option<SeatId>, session_token: Option<String>, text: String }  pub struct AskResult { l1: Option<Evidence>, l2: Option<Evidence>, chat: Vec<ChatHit>, guidance: Vec<String> }
```

`evaluate` returns `Evaluated { verdict, guard_degraded }` so rule 5 can set the flag without a `Deliver` variant that carries it. `DeliveryReceipt` also stores the monotonic delivery `cursor` for slice 8.

The canonical stream envelope lives in `crewmux-api`:

```rust
pub struct StreamEvent { id: u64, kind: StreamKind, payload: Option<StreamPayload> } // serde name `type`
```

`StreamKind` is `#[non_exhaustive]` and includes the slice 13 names (`seat_state_changed`, `seat_died`, `activity`, `gap`) plus `queue_changed`, `delivery_recorded`, and `attention_raised`. Unknown kinds deserialize as `Other`. `DomainEvent` stays `#[non_exhaustive]`.

## Acceptance criteria

Queue state machine

1. Every TS-legal transition is accepted, and every illegal one is rejected with a typed error. This is a table-driven test generated from the TS transition tables. Terminal states never transition, except via the explicit TS exceptions (e.g. `blocked` → resolve, from slice 18).
2. Exactly-once obligation. Given an item, when create, claim, handoff and complete happen, then each is one transaction. A crash injected at any point (test hook) leaves either the before or the after state, never a half-applied handoff. `queue-baton-survives-restart` (test-system) passes on the stub runtime.
3. Hot-potato closure validation: a handoff must carry a closure (target/reason/required_at) where TS requires one. A missing closure is rejected with the TS message text.
4. Claim is exclusive: a second claimant gets `AlreadyClaimed`; unclaim releases it; claims by a different session are refused unless `--force` is given with a reason (TS parity).
5. Retention sweep archives only `is_terminal()` items older than the configured age. Unit test with a fake clock.

Wake ladder

6. Given a handed-off item whose first nudge failed, `tick` yields `Nudge` retries up to N (config), then `Escalate(DestinationOrchestrator)` once per destination (aggregated, never one row per item), then `Escalate(Operator)`. Each step appends a transition with the TS-compatible note prefix, so derivation is stable across restarts.
7. The ladder never re-sends content. A nudge is the pointer text only. A test asserts that the nudge body never contains the item body.
8. Given `wake_human_rung = explicit-only`, a human-seat destination is not nudged by the ladder unless the item was explicitly addressed to the human. Given `always`, it is nudged (the operator surface records it).
9. A "delivered-but-unconfirmed" item is never re-nudged before the confirmation window (TS F1 gate). Fake clock.

Guard verdict (ADR-0005)

10. `evaluate` implements rules 1–6 in order. Table test over `GuardInputs` × `ComposerState`:
    - not-at-prompt → Defer(NotAtPrompt)
    - copy mode → Defer
    - keystroke within the quiet window (3 s default) → Defer(HumanTyping), from either the tmux or the tap source
    - a human draft → Defer, and after `max_defer` → Escalate(HumanDraft); it never overwrites
    - `Unknown` composer → rules 2–3 only, with `guard_degraded = true`
    - otherwise Deliver
11. Pre-paste re-check: given the human types between the verdict and the paste (FakeMux scripts a keystroke at that instant), the paste is not issued and the verdict re-runs.
12. Post-paste verification: given the composer after paste contains text beyond our payload, Enter is not sent, the receipt is `Contaminated`, and attention is raised. With the stub composer, Enter is sent only on a clean match.
13. Daemon taps: a keystroke arriving over the terminal WS for seat S defers deliveries to S for the quiet window. This works even with no tmux human client.

rig send

14. Given a seat at an interactive gate (Readiness ≠ Ready), `rig send` refuses with the TS-equivalent message and sends nothing. With `--dangerously-interact --reason "<why>"`, it sends unbracketed (`bracketed:false`) and records the reason.
15. `--to a,b` and pod/rig fan-out deliver to each seat with its own `To:` header; per-seat receipts are returned; one seat failing does not block others (exit 3 on partial).
16. The `send-renders-in-addressed-pane` and `send-verify-means-rendered` test-system scenarios pass on the stub runtime.

Ask / transcripts / events

17. The transcript writer appends each seat's `%output` (ANSI kept, plus a stripped text twin) to `$RIG_HOME/transcripts/<seat>/<generation>.log`, rotated at a size cap. `rig ask --seat S "<text>"` searches only S's generations (L1). `--session <token>` searches only that provider session's JSONL (L2, read-only). The two are never mixed with a whole-rig grep. When transcripts are disabled or missing, the result carries honest `guidance`, not an empty success.
18. `GET /api/events` streams `DomainEvent`s over UDS (SSE and WS), with resume from a `last_event_id`. A reconnecting client gets missed events from a bounded backlog, or a `gap` event if the backlog was exceeded.

## Required tests

- messaging: state-machine table (AC1), crash-injection transactions (AC2), closure validation (AC3), claims (AC4), retention (AC5), ladder ticks with a fake clock incl. transition-log derivation after a "restart" (AC6–AC9).
- mux::guard: verdict table (AC10). Property test: no input combination yields Deliver while `last_human_keystroke` is within the quiet window.
- FakeMux integration: pre-paste race (AC11), contamination (AC12), tap path (AC13).
- CLI goldens for `rig queue list/show --json`, `rig send` refusal text, and `rig ask` guidance.
- test-system scenarios on stub: `queue-baton-survives-restart`, `send-renders-in-addressed-pane`, `send-verify-means-rendered`, `kill-daemon-mid-handoff` (wake/obligation part).
- PE: none new. Queue obligations follow the TS row model, which is parity. PE-005 covers only direct sends and startup files. This slice must record the fingerprint and delivery cursor on every direct `rig send` receipt for slice 8.
- Ledger: queue-repository, wake-ladder, ask, claim, inbox, session-transport and seat-delivery-guard tests are mapped. Guard-verdict TS tests become ported, or diverged + a PE where the composite guard intentionally defers more than TS did. The pinned `seat-delivery-guard` file is a lease guard, not ADR-0005, so those titles stay unported and no PE is added.

## CI vs gated

CI: everything above, with real tmux ≥ 3.3 and the stub runtime for e2e and scenarios. Gated (needs logins, slices 2/3): guard and post-paste verification against real Claude/Codex composers; doesn't block this slice.

## Definition of done

AC1–AC18 pass; scenarios green on stub; ledger rows mapped; ADR-0005 rules and ADR-0009 Amendment 1 match; notice headers on derived files.

## Dependencies added for this slice

- `axum` feature `ws` (daemon), which pulls `tokio-tungstenite`, so `GET /api/events` can upgrade to a WebSocket on the same path as SSE.
- `futures-core`, hoisted to `[workspace.dependencies]`, for the hand-written SSE `Stream`.
- `serde` as a direct dependency of `crewmux-daemon` and `crewmux-store` (already in the workspace).
