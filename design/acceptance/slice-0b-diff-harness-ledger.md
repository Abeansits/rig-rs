> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# Slice 0b: Differential harness and test-port ledger

Depends on slice 0. Merged as PR #3. OpenRig is pinned to **v0.6.9, commit 4b48ca21**.

## Acceptance criteria
- **AC1 Hermetic:** each case runs with a scratch HOME, a private `tmux -L` socket, and a PATH shim (stub providers, no real claude/codex).
- **AC2 Both sides:** rig-rs is driven over the UDS and OpenRig over TCP (Node 22). If the TS tree is missing, the run **exits 2, not skip**.
- **AC3 UDS client:** reaches the rig-rs `/healthz`. The config `rs_transport: tcp` is rejected (G8).
- **AC4 Normalisation:** ULIDs, timestamps, absolute paths, ports and socket paths become stable placeholders before comparing.
- **AC5 tmux shim:** a Python shim records every tmux argv as JSONL, so mutation sequences compare.
- **AC6 Goldens:**
  - `--update` writes only `*.ts.snap`;
  - `*.rs.snap` is read-only to the tool and is changed only by a reviewed edit.
- **AC7 Initial cases:** `version`, `help-top`, `daemon-health`, `scenario-runner-probe`, `stub-seat`.
- **AC8 Expected diffs:**
  - A diff is "expected" only when `parity-exceptions.md` has a PE heading whose `- **Covers:**` line names the case, **and** a matching rs snapshot exists.
  - Otherwise the result is `Unexpected`, or `Stale` for a PE that matches nothing.
- **AC9 Ledger seed:** every OpenRig test file is listed. Seed counts: 1303 files, 1124 `todo`, 179 `waived`.
- **AC10 Ledger check:**
  - the status is one of `todo|ported|diverged|waived`;
  - `diverged` requires a `pe_id` that exists;
  - `ported` requires a `rust_test` that appears in `cargo test -- --list`.
- **AC11 Report:** counts per status and per slice.
- **AC12 Exit codes:** 0 clean, 1 unexpected diffs or ledger errors, 2 harness or setup error.
- **CI:** `diff-ts.yml` runs nightly and on the `diff-ts` PR label (PR #2/#3).
