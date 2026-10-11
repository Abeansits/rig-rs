# rig-rs QA plan: CI gates

Status: prepared by QA Bot on 2026-10-11 against `Abeansits/rig-rs` main @ `37677ee`. Updated with Dev Bot's decisions (crewmux-* names, MSRV from Cargo.toml, SHA-pinned actions including `check`, features per crate, deny.toml from slice 0). Nothing has been pushed yet.
The ready-to-PR files are in `qa/pr/`.

## Baseline (main @ 37677ee, read via the GitHub-xai connector)
- `Cargo.toml`: `resolver = "3"`, `members = []`, edition 2024, Apache-2.0, `publish = false`, `[workspace.lints.rust] unsafe_code = "forbid"`. **No `rust-version`, no `Cargo.lock`, no `rust-toolchain.toml`, no `clippy.toml`/`rustfmt.toml` yet.**
- `.github/workflows/ci.yml`: one job, `check`, on an ubuntu + macos matrix. It runs fmt, then clippy `-D warnings`, then `cargo test`, and each step is skipped while the workspace has 0 members.

## Gates per merge (new jobs; Dev Bot's `check` job logic kept, actions now SHA-pinned)
| Job (check name) | Runs | Required? | What it does |
|---|---|---|---|
| `fmt, clippy, test (ubuntu-latest)` / `(macos-latest)` | Dev Bot's | yes (via ci-pass) | Logic unchanged; actions pinned |
| `nextest (ubuntu-latest)` / `(macos-latest)` | every PR + main | yes | `cargo nextest run --workspace --all-targets --all-features --profile ci` with tmux installed, `INSTA_UPDATE=no`, a temp `RIG_HOME`, and doctests via `cargo test --doc --all-features`. Features (`tmux-it`, `differential`, …) are declared per crate; the `ci` profile filters out `::differential::` |
| `cargo-deny (bans licenses sources)` | every PR + main | yes | Uses `deny.toml` **shipped by slice 0** (RUST-AGENT-GUIDE §2.5 + bans). This PR does not include `deny.toml`; rebase onto slice 0 |
| `cargo-deny (advisories)` | every PR + main | **on main only** | `continue-on-error` on PRs, so a new RUSTSEC entry can't block unrelated work; a red result on main gets triaged |
| `msrv` | every PR + main | yes | Reads `workspace.package.rust-version` from `Cargo.toml` (slice 0 sets a real stable). If absent, the job **skips with a notice** and succeeds — no hardcoded fallback |
| `guards` | every PR + main | yes | 800-line cap on non-test `.rs` files, no bare `#[ignore]`, no committed `.snap.new`, and every `ported` row in the parity ledger must point at a Rust test that exists |
| `coverage (informational)` | every PR + main | **no** | `cargo llvm-cov nextest --all-features` produces an lcov artifact plus a summary in the job summary |
| `ci-pass` | always | **the only required check** | Fails unless check, nextest, deny, msrv and guards all succeeded |

Branch protection: make **`ci-pass`** the only required status check. Matrix job names change as the matrix changes; an aggregator doesn't, so branch protection never has to be updated. This needs Sebastian (repo admin).

### Why these choices
- **Empty-workspace guards.** `cargo deny` exits with an error on a virtual manifest with no members (verified locally), so nextest, deny and coverage reuse Dev Bot's member-count guard and stay green until slice 0 lands.
- **nextest next to `cargo test`.** Dev Bot's `cargo test` step stays for now (no collision). **Follow-up (once `ci-pass` is the required check):** drop `cargo test` from `check` so tests don't run twice. That edit is Dev Bot's to make after branch protection is set.
- **Coverage is cheap enough to have, but not to gate on.** On ubuntu only, cargo-llvm-cov roughly doubles one build because of instrumentation, and there's no macOS leg. It's informational because porting PRs add tests faster than coverage moves, and a percentage gate would invite hollow tests. Mutation testing (cargo-mutants, already in the guide) is the better hollowness check; add a weekly job after slice 4.
- **Pinning.** Every `uses:` in this workflow (including Dev Bot's `check` job) is pinned to a full commit SHA with a `# vX.Y.Z` (or `# master` for dtolnay/rust-toolchain) comment. Resolved via `gh api repos/<owner>/<repo>/git/refs/tags/<tag>` (peeled) / commits API. dtolnay keeps `@stable` semantics: pinned action SHA + `toolchain: stable` input. cargo-deny is pinned to `0.20.2`.
- **Caching.** Swatinem/rust-cache; coverage uses a separate `key` so instrumented artifacts don't overwrite the normal cache.
- **Toolchain.** Jobs use `toolchain: stable` (action input). When Dev Bot adds `rust-toolchain.toml` in slice 0, rustup will follow that file. MSRV is whatever `rust-version` slice 0 writes — not a hardcoded number in this workflow.
- **Crate names.** Architect's design wins: all crates are `crewmux-*` (e.g. `crewmux-testkit`, `crewmux-api`). See `design/workspace-layout.md`.
- **Features per crate.** Optional features (`tmux-it`, `differential`, …) are declared in each crate's own `[features]` table, never in the workspace root. CI uses `--all-features`; nextest profiles filter suites that shouldn't run per PR.

## Not included (deliberately)
cargo-audit (overlaps cargo-deny advisories), cargo-machete (add once crates have deps), `cargo doc -D warnings` (Dev Bot's lint lane), mutants, fuzz. These go in a follow-up PR after slice 0. `deny.toml` is also not in this PR — slice 0 owns it.

## Local validation done
- `actionlint` 1.7.x: clean, including its shellcheck integration.
- YAML parse (PyYAML): jobs = check, nextest, deny, msrv, guards, coverage, ci-pass.
- Diff against the original ci.yml (1140 bytes, blob `9eddaad`): the `check` job **logic** is identical; only `uses:` lines changed to SHA pins (+ `toolchain: stable` on dtolnay).
- `cargo-deny 0.20.2` on a scratch workspace replicating Cargo.toml: errors on 0 members (hence the guard). With one probe crate: `advisories ok, bans ok, licenses ok, sources ok`. Adding `serde_yaml` makes it fail with `banned` as intended.
- `cargo-nextest` (latest): the `.config/nextest.toml` profiles default, ci and slow all parse and run, and the tmux test-group registers.
- The guard scripts were smoke-tested: an 800-line violation and a missing ledger path were both caught.
