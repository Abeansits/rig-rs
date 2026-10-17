> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# ADR-0003: SQLite layer

Status: Accepted (2026-10-11). Amended after spike S5 (checkpoints off the writer).

## Context
- OpenRig keeps rigs, seats, queue items, chat, snapshots and events in SQLite via better-sqlite3 (synchronous, one connection, WAL).
- The port uses a **fresh schema** (matching on-disk formats is optional, per Sebastian). It needs transactional multi-row updates: the queue state machine, mission-control verbs and the workflow projector.

## Options
1. **rusqlite (bundled SQLite)**, synchronous, run on a dedicated thread.
2. sqlx (async, compile-time-checked queries). It is heavier, its async gains little for SQLite, and offline query data adds build friction.
3. Diesel or SeaORM. ORM overhead, and a poor fit for hand-written projector SQL.

## Decision
- **rusqlite with the `bundled` feature**, so the SQLite version is the same on macOS and Linux.
- **One writer thread (the DB actor)** owns the only read-write connection. Async callers send closures or commands over a channel and await the reply. **A small pool of read-only connections** serves queries.
- **Pragmas:** `journal_mode=WAL`, `foreign_keys=ON`, `busy_timeout=5000`, `synchronous=NORMAL`.
- **Checkpoints (S5 amendment):** `PRAGMA wal_autocheckpoint=0` on the writer, and `wal_checkpoint(PASSIVE)` run by a **separate maintenance connection** on a timer. The spike measured p99 0.35 ms / max 4.5 ms for writes once checkpoints were off the writer thread.
- **Migrations:** an ordered `MIGRATIONS` list applied at `Store::open`, tracked by `PRAGMA user_version`.
  - Reopen applies nothing, and a database whose `user_version` is newer than the binary's is **refused**. Slice 1 (PR #9) implemented exactly this, with tests `empty_database_applies_migrations_and_reopen_is_a_noop` and `newer_schema_is_refused`.
  - The runner is `rusqlite_migration` (`crewmux-store/src/migrate.rs`, `Migrations`/`M`), which tracks the version in `user_version`.
- Owned by **`crewmux-store`**. No other crate opens the database file.

## Consequences
- Domain code receives plain structs. Multi-step operations (queue transitions, mission-control verbs, the workflow projector) run as **one transaction per call** on the writer.
- No import from Node OpenRig's database (G7, Sebastian).

## Open risks
- Writer-queue latency under bursty event logging; covered by the nightly load test.
- APFS fsync behaviour on macOS. An optional re-run of S5 on the Mac is non-blocking.
