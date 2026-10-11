# QA Bot merge checklist: rig-rs slice PRs

Verdict = **PASS** / **FAIL (blockers listed)**. Blockers get evidence (file:line, command output). Nits are listed separately and don't block. Merge rule (Sebastian, 2026-10-11): a rig-rs PR merges once `ci-pass` is green and QA has passed it; Dev Bot does the merging, including QA Bot's own CI PRs (second pair of eyes). QA Bot gates, never merges.

Crate names: Architect's `crewmux-*`. Optional features (`tmux-it`, `differential`, …) must be declared **per crate**, not in the workspace root.

TS reference: OpenRig v0.6.9 pinned at `4b48ca21`.

## Blockers
1. **CI:** `ci-pass` is green on the PR head SHA, and I've re-run the gates locally from a clean tarball of that SHA (`cargo nextest run --profile ci --all-features`, `cargo deny check`, `cargo clippy --all-targets -D warnings`).
2. **Parity:** every in-scope TS test file for the slice has a row in `parity/ledger.csv`, and `cargo xtask ledger check` passes. `ported`/`reused` rows name real tests (`rust_test`) that I've read and that assert the same behaviour as the TS case (not weaker). `waived` rows have a note I agree with; `diverged` rows have a PE id. No row is left `todo` without a stated reason in the PR.
3. **No test weakening:** no deleted or loosened assertions, no bare `#[ignore]`, no snapshot churn without an explanation, and fixtures aren't hand-edited (regen diff is reproducible: I re-run `regen-fixtures.sh` for changed slices).
4. **Behaviour spot-check against TS:** for at least 2 cases per slice, I run the TS side (or read the fixture source) and the Rust side and confirm the outputs match.
5. **Safety rules from RUST-AGENT-GUIDE:** no `unwrap/expect/panic` in lib code (clippy enforces this; also no `#[allow]` without a reason), no blocking calls in async, no real `$HOME` or global tmux server in tests (temp `RIG_HOME`, `-L` socket), and spawned tasks are owned and cancellable.
6. **Architecture boundaries:** no forbidden dependency edges (domain crates without tokio, rusqlite or process; clients only through `crewmux-api`), checked with `cargo tree -e normal -i <crate>`.
7. **Supply chain:** each new dependency is justified in the PR, passes deny, and `Cargo.lock` is committed.
8. **Licensing:** ported files carry the "Derived from openrig …" header, and NOTICE is intact.
9. **Scope:** the PR is one slice (or part of one), roughly ≤ 600 changed lines excluding fixtures and snapshots, and references the TS source paths.
10. **Features:** any new optional feature is declared on the crates that need it (not workspace-level), and nextest profile filters still exclude suites that shouldn't run on PRs.

11. **Parity tests required (unit-only PRs FAIL):** the PR must include ported TS parity tests for its slice: `parity/ledger.csv` rows moved to `ported`/`reused` with real Rust tests, covering the TS files mapped to the slice in `qa/ts-test-map.csv` (or `waived` with a note / `diverged` with a PE id). A PR that only adds unit tests does not pass, even if all of them are green.
12. **PE-* exception tests required:** every PE-id that the slice's acceptance spec (`design/acceptance/slice-*.md`) or `design/parity-exceptions.md` assigns to it must ship its named required test(s), CI-blocking. Today: PE-001 → slice 2 (`claude_gates.rs::first_run_screens_are_not_ready` + zero-paste FakeMux test); PE-002 → slice 3 (`codex_gates.rs::login_screens_are_login_required`, `codex_launch.rs::not_logged_in_preflight_short_circuits`, `readiness.rs::stable_inconclusive_escalates_before_timeout`); slice 1 ships the shared ready-requires-positive-match rule (AC8/AC9) and PE-008 (`attach_execs_exact_tmux_target`, `attach_unknown_seat_exits_1`); slice 0b tests AC8 with a synthetic `PE-TEST`. Every `diverged` ledger row needs a PE-id with a test.
13. **Post-merge gates are blockers:** after each merge to main, QA runs `qa/scripts/tmux-smoke.sh` on merged main under **tmux 3.3a and latest** (`/opt/tmux-3.3a`, `/opt/tmux-latest`) and logs the result in `qa/STATUS.md`. A failed smoke on either version, or a red nightly diff-harness run, is a **blocker**: no further slice PR passes until it is fixed or reverted.

## Nits (reported, non-blocking)
File > 400 lines (soft cap), missing `/// ts:` doc comment (pointer only; the ledger's `rust_test` is authoritative), clone-happy code, coverage drops in the touched crate, naming, docs.

## Evidence format to Dev Bot
`PASS|FAIL` · head SHA · commands run plus summaries · ledger delta from `cargo xtask ledger report` (todo/ported/reused/waived/diverged counts) · blockers · nits · CodeRabbit items confirmed or contested.
