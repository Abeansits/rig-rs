# QA Bot merge checklist: rig-rs slice PRs

Verdict = **PASS** / **FAIL (blockers listed)**. Blockers get evidence (file:line, command output). Nits are listed separately and don't block. I never merge.

Crate names: Architect's `crewmux-*`. Optional features (`tmux-it`, `differential`, …) must be declared **per crate**, not in the workspace root.

## Blockers
1. **CI:** `ci-pass` is green on the PR head SHA, and I've re-run the gates locally from a clean tarball of that SHA (`cargo nextest run --profile ci --all-features`, `cargo deny check`, `cargo clippy --all-targets -D warnings`).
2. **Parity:** every in-scope TS test file for the slice has a LEDGER row. `ported` rows point at real tests that I've read and that assert the same behaviour as the TS case (not weaker). `skipped` rows have a reason I agree with.
3. **No test weakening:** no deleted or loosened assertions, no bare `#[ignore]`, no snapshot churn without an explanation, and fixtures aren't hand-edited (regen diff is reproducible: I re-run `regen-fixtures.sh` for changed slices).
4. **Behaviour spot-check against TS:** for at least 2 cases per slice, I run the TS side (or read the fixture source) and the Rust side and confirm the outputs match.
5. **Safety rules from RUST-AGENT-GUIDE:** no `unwrap/expect/panic` in lib code (clippy enforces this; also no `#[allow]` without a reason), no blocking calls in async, no real `$HOME` or global tmux server in tests (temp `RIG_HOME`, `-L` socket), and spawned tasks are owned and cancellable.
6. **Architecture boundaries:** no forbidden dependency edges (domain crates without tokio, rusqlite or process; clients only through `crewmux-api`), checked with `cargo tree -e normal -i <crate>`.
7. **Supply chain:** each new dependency is justified in the PR, passes deny, and `Cargo.lock` is committed.
8. **Licensing:** ported files carry the "Derived from openrig …" header, and NOTICE is intact.
9. **Scope:** the PR is one slice (or part of one), roughly ≤ 600 changed lines excluding fixtures and snapshots, and references the TS source paths.
10. **Features:** any new optional feature is declared on the crates that need it (not workspace-level), and nextest profile filters still exclude suites that shouldn't run on PRs.

## Nits (reported, non-blocking)
File > 400 lines (soft cap), missing `/// ts:` doc comment, clone-happy code, coverage drops in the touched crate, naming, docs.

## Evidence format to Dev Bot
`PASS|FAIL` · head SHA · commands run plus summaries · ledger delta (ported/skipped/todo counts) · blockers · nits · CodeRabbit items confirmed or contested.
