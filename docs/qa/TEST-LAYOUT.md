# Test layout convention (rig-rs)

Crate names follow Architect's design: `crewmux-*` (see `design/workspace-layout.md`). Features are declared **per crate**, never in the workspace root `Cargo.toml`.

| What | Where | Notes |
|---|---|---|
| Unit tests | in-module `#[cfg(test)] mod tests` at the bottom of the file | Pure logic, no I/O. If tests push the file past 400 lines, move them to `src/<mod>/tests.rs` |
| Integration tests (crate API) | `crates/<crate>/tests/<topic>.rs` | One binary per topic; shared helpers in `crates/<crate>/tests/common/mod.rs` |
| Parity tests | `crates/<crate>/tests/parity_sNN_<name>.rs` | Driven by `tests/fixtures/sNN-<name>/` through the testkit |
| Golden fixtures | `tests/fixtures/sNN-<name>/*.json` (workspace root) | Captured from TS v0.6.9 (`4b48ca21`); see PARITY.md |
| insta snapshots | `crates/<crate>/tests/snapshots/` (insta default) | `.snap` committed, `.snap.new` never |
| Real-tmux tests | `mod tmux_it` inside the integration file, `#[cfg(feature = "tmux-it")]` | Private `-L` socket per test; nextest test-group `tmux` (serial) |
| Differential tests | `mod differential`, `#[cfg(feature = "differential")]` | Need `OPENRIG_DIR` and node; excluded from the `ci` profile |
| End-to-end (binary) | `tests/e2e/` crate (design) using `assert_cmd` | Temp `RIG_HOME`; never the real `~` |
| Fakes and test helpers | `crewmux-testkit` crate (dev-dependency only) | FakeMux/FakeTmux, temp home, fixture loader, clock |

## Naming
- Test fns: `<behaviour>_<condition>` in snake_case, e.g. `wake_ladder_escalates_after_three_nudges`. No `test_` prefix.
- Ported tests are registered in `parity/ledger.csv`: `ts_path` → `rust_test` (the name as shown by `cargo test -- --list`), checked by `cargo xtask ledger check`. Optionally add a `/// ts: <ts_path> > "<case>"` doc comment as a human pointer; the ledger is authoritative.
- Feature names: `tmux-it` (real tmux), `differential` (TS side-by-side). Both are off by default. **Each crate that uses them declares them in its own `[features]` table** (Dev Bot decision); the workspace Cargo.toml is not touched. CI runs with `--all-features`; nextest profiles filter suites that shouldn't run.

## nextest profiles (`.config/nextest.toml`)
- `default`: local work, fail-fast, and filters out `::tmux_it::` and `::differential::` even under `--all-features`.
- `ci`: no retries (fix flakes, don't retry them), no fail-fast, JUnit output, runs tmux_it, excludes differential.
- `slow`: everything, including differential; nightly or on demand: `cargo nextest run --all-features --profile slow`.

## Snapshots
Use insta (guide §1: `insta` 1.49 with the yaml, json and redactions features) for CLI output (`assert_cmd` + `assert_snapshot!`), JSON API responses, the MCP tools/list, and ratatui `TestBackend` buffers. Redact ids and timestamps with insta redactions. CI sets `INSTA_UPDATE=no`. Review snapshots locally with `cargo insta review`, never a blanket `accept`.

## Time and async
Use `#[tokio::test(start_paused = true)]` for anything timer-driven (wake ladder, health detectors). No real sleeps in tests.
