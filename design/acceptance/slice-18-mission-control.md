> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# Slice 18: Mission control (human-in-the-loop verbs, audit, notifications)

Depends on 0, 4, 14. Port of OpenRig `packages/daemon/src/domain/mission-control/` @ v0.6.9.
**Not yet implemented in the repo. The ACs below are reconstructed from memory and the TS source.**

## Scope
- 8 atomic verbs on queue items. The TS header comment says "7-verb", but the code has 8 cases including `resolve`.
- An audit log, a read layer, a fleet CLI capability check, notifications (G5), and a CLI inbox (G3).

## Acceptance criteria
- **AC1 Verbs:**
  - `approve` → done/no-follow-on;
  - `deny` → done/denied;
  - `route` (needs `destinationSession`);
  - `hold`;
  - `drop`;
  - `handoff` (needs `destinationSession`, and also creates the destination qitem);
  - `annotate` (`annotateOnly`, needs annotation text);
  - `resolve` (non-closure, needs a leg-1 parked qitem with `state=blocked` and `blocked_on` a human seat, plus decision text).
- **AC2 Atomicity:** each verb is one DB transaction with three parts: the queue update, a `mission_control_actions` audit row (before/after state, actor, reason), and the `mission_control.action_executed` event. Failure injection mid-transaction leaves no partial rows.
- **AC3 Validation:** missing required args → typed error, no writes. Invalid state transitions are refused with the current state.
- **AC4 Audit browse:** paginated, filterable by verb, qitem and time. Read-only.
- **AC5 Notifications (G5):**
  - opt-in and off by default;
  - adapters: ntfy and webhook;
  - every target URL goes through the outbound URL validator (scheme allow-list, no private/loopback unless explicitly allowed).
- **AC6 Notify is post-commit and best-effort.** A failure is logged and evented but never rolls back the verb.
- **AC7 CLI inbox (G3):** `rig queue human-updates` lists items blocked on the human seat. Verbs are exposed as `rig mc <verb> …` (**exact CLI shape reconstructed**).
- **AC8:** iMessage and the Grok Bot bridge are deferred (G3). No code paths.
- **AC9:** Fleet CLI capability: verbs are refused with a clear error when the daemon lacks the capability (older schema).
- **AC10:** Ledger rows for the mission-control tests are `ported` or `diverged` with a PE.
