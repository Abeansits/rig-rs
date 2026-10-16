> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# Slice 1: Spec, store, daemon, one stub seat end to end

Depends on 0, 0b, 1b. Merged as PR #9.

## Acceptance criteria
- **AC1 Spec parse:**
  - RigSpec via `crewmux-spec::yaml` (serde-saphyr, ADR-0007);
  - errors carry line/col;
  - seat names match `[A-Za-z0-9_-]`.
- **AC2 Store:**
  - migrations are applied by `user_version` (ADR-0003);
  - reopen is a no-op;
  - a newer schema is refused.
- **AC3 Up:**
  - `rig up` auto-starts the daemon (`flock run/daemon.lock`) and creates the session at 90×27;
  - events `rig_created`, `seat_launched`, `seat_ready` are emitted;
  - parallel `up` is serialised.
- **AC4 Idempotent up:** a second `up` returns `already_running` with no new panes.
- **AC5 ps:**
  - `rig ps` lists rig/seat state with `attached: false` (control clients are filtered, ADR-0005);
  - dirs are 0700 and files 0600.
- **AC6 Capture:** `rig capture <seat>` shows the `stub> ` prompt and supports `--lines` and `-N`.
- **AC7 Attach:** execs `tmux -S <sock> attach -t =<session>` (repo PE-008).
- **AC8 Gate:** stub with `print_gate=login_required` → seat state `Attention` (PE-002 shape).
- **AC9 Unrecognised:** a stable inconclusive screen past budget → `UnrecognisedScreen`.
- **AC10 Death:** stub `exit_after_ms` → `Dead` via the pane-died hook.
- **AC11 Down:** `rig down` writes `Stopped` before killing the session.
- **AC12 Partial failure:** a seat with `invalid_cwd` makes `up` exit 3 with per-seat errors.
- **AC13 Hermetic:** no writes to the real HOME.
- **Stub provider:** script and JSON contract are repo PE-005 and PE-007. Process lineage is PE-019 (PR #25).
