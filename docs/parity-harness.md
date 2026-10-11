# Parity harness

`cargo xtask diff` runs one case against TypeScript OpenRig v0.6.9 and against the `rig` binary. Each side gets its own scratch `HOME`, a private `tmux -L` socket, and a `tmux` shim that records argv. `cargo xtask ledger` keeps `parity/ledger.csv`, one row per upstream test file.

The TypeScript pin is tag `v0.6.9`, commit `4b48ca21a9bd072aa05a08b3da6d9c0708e093c5`. rig-rs talks to its daemon on `$RIG_HOME/run/daemon.sock`. TypeScript uses TCP `127.0.0.1`. A case that sets `rs_transport` to `tcp` fails while the file is loaded, before any dial.

## Add a case

1. Add `tests/diff/cases/<id>.json`. The id is `[A-Za-z0-9._-]+` and matches the file stem.
2. List `steps`. Each step is `cli` (`argv`), `http` (`method`, `path`, optional `body`), `read_file` (path relative to the hermetic `HOME`), or `tmux_calls`.
3. Set `normalize` in this order when the defaults are enough: `ulid`, `timestamp`, `socketpath`, `abspath`, `port`. The same raw value becomes the same placeholder inside one observation.
4. Set `project` to `full`, `exit` (status only), `help-names` (sorted command names), or `health-shape` (`status=` plus sorted top-level keys).
5. Set `needs` to skip the case and list it. The diff report prints those rows under `ignored`.
6. Set `scenario` when the case stands in for a file under `packages/test-system/scenarios/`.

`daemon-health` runs `GET /healthz` and compares the health-shape projection (`status=` plus sorted top-level keys). `stub-seat` runs the slice 1 acceptance document.

`scenario-runner-probe` names `packages/test-system/scenarios/clean-lifecycle-no-residue.yaml` and runs `rig --version` plus the tmux shim. Spike S11 is not a YAML interpreter yet, so this probe shows the runner can target either side. It does not execute the scenario file.

`help-top` and `scenario-runner-probe` set `expected_diff` to `PE-003`, which covers `stdout` only. Against v0.6.9, TypeScript prints the command list and `0.6.9`; the scaffold `rig` binary prints nothing and still exits 0. The committed `*.rs.snap` files record that Rust side (empty stdout, exit 0, the same tmux argv). `version` compares the exit code only and names no exception, so it stays Equal. `--self` does not apply `expected_diff`: both sides are rig-rs, and a match is Equal.

## Mark an expected diff

Put `expected_diff` on the case. The value must be a `## PE-…` heading in `design/parity-exceptions.md`. Under that heading, `- **Covers:**` lists the observation fields the exception excuses (`stdout`, `stderr`, `exit`, `json`, `files`, `tmux_argv`). Any other id fails at load time. A heading without a covers line fails at load time.

Commit `tests/diff/snapshots/<id>.rs.snap` (insta front matter plus the canonical Rust observation). `--update` writes TypeScript goldens only and does not accept a rig-rs snapshot.

`ExpectedDiff` applies when the live Rust observation matches that snapshot and every difference from TypeScript is in a covered field. A difference in any other field — a different exit code, stderr, tmux argv, JSON, or read file — is `Unexpected`, with a unified diff. When the Rust observation no longer matches the snapshot, the run fails and the report contains `stale exception`. The same stale result is reported when the two sides are equal. Process exit is 1.

Tests use a fixture copy of the exceptions file with a synthetic `## PE-TEST` heading. Do not add `PE-TEST` to the real file.

## Update TypeScript goldens

```sh
cargo xtask diff --update
cargo xtask diff --update --case version
```

`--update` rewrites only `tests/diff/snapshots/<id>.ts.snap` (insta front matter plus the canonical JSON). The rig-rs side is compared and never written. There is no flag that accepts a rig-rs snapshot. `--update` with `--self` is a usage error.

A missing TypeScript golden, a missing Rust snapshot on an expected-diff case, a missing TypeScript tree, or a bad PE id is a harness error (exit 2). The missing-tree message names v0.6.9 and the pinned commit and tells you to clone into `$HOME/.cache/rig-rs/openrig-v0.6.9` (or set `OPENRIG_DIR`) and run `npm ci && npm run build -w @openrig/daemon && npm run build -w @openrig/cli`. The harness does not skip that case.

`cargo xtask diff --self` runs every case twice against the `rig` binary (or `--bin`). It does not need Node. Ignored cases stay in the report and do not fail the run.

`.github/workflows/diff-ts.yml` runs `cargo xtask ledger check` and `cargo xtask diff` against the pinned tree. It runs nightly, on `workflow_dispatch`, and on pull requests labeled `diff-ts`. Node >= 22 is required for that job. Per-PR `ci.yml` does not run the TypeScript side.

Exit codes: 0 when every verdict is Equal or ExpectedDiff, 1 on Unexpected or a stale exception, 2 on a harness error. Unexpected rows include a unified diff.

## Ledger

```sh
OPENRIG_DIR=/path/to/openrig cargo xtask ledger seed
cargo xtask ledger check
cargo xtask ledger report
```

`seed` lists every `*.test.ts`, `*.test.tsx`, `*.spec.ts`, and `*.spec.tsx` under `packages/{daemon,cli,tui}/test` and `packages/test-system`. Helpers are not rows. Out-of-scope paths (gateway, bundle, cross-host, cmux, pi/omp, ui, plugins, docker secrets, adopt/discovery, migrations, proof/demo/compat) are `waived` with a note. Everything else is `todo`. The patterns are narrower than a raw search for `discover`, `compose`, or `proof`, so in-scope tests that only share those words stay `todo`.

`check` fails on an unknown status, a `diverged` row without a known `pe_id`, a `ported` or `reused` row without `rust_test`, a `waived` row with an empty note, a duplicate `ts_path`, or a `rust_test` that `cargo test --workspace --locked -- --list` does not print. `check` spawns that cargo command only when some row sets `rust_test`. The seeded ledger does not, so check does not take the target-directory lock.

`report` prints markdown counts per status and per slice.

`parity/ledger.csv` is the single parity ledger (the old `tests/parity/LEDGER.md` QA note was retired).
