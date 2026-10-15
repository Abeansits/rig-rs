# Design tree recovery (2026-10-15)

The box copy of the design workspace (`/workspace/openrig-port/design`, `PLAN.md`, `qa/`, `spikes/`) was lost around 2026-10-14/15. From now on this repo is the canonical copy of `design/`. Any copy under `/workspace` is a scratch mirror only.

Rule for this recovery: only verbatim text was restored. Nothing here was rewritten or re-created from memory.

## Sources searched

1. `design/` and `docs/qa/` on `main` (f3c2879) and on every branch in this repo (all `cursor/*` and `qa/*` branches for PRs #1-#28).
2. Cloud-agent transcripts on the box (`/workspace/cloud-agent-transcripts/*.jsonl`, 6 files). All six are from an unrelated project dated 2026-10-08. None contain rig-rs design text.
3. Box agent transcripts (`~/agent-data/agent-transcripts`). The newest is from 2026-10-08, and none hold rig-rs spec text.
4. PR bodies for #1-#28. They cite ADR ids (0002, 0003, 0004, 0005, 0008, 0009, 0010) and AC numbers but quote no spec text.
5. Agent stores under `/cursor/stores/*`. No design files there.
6. Every `*.md` file on the box named `slice-*`, `parity-exceptions*`, `PLAN.md`, `STATUS.md` or `0001-*`. The only matches were repo clones under `/tmp` holding the same blobs as `main`.

## Recovered (verbatim, already on `main`, unchanged by this PR)

| File | Source |
| --- | --- |
| `design/acceptance/slice-2-claude-seat.md` | `main` (same blob on every branch) |
| `design/acceptance/slice-3-codex-seat.md` | `main` |
| `design/acceptance/slice-4-queue-ask-wake-guard.md` | `main` |
| `design/acceptance/slice-5-chatroom.md` | `main` |
| `design/adr/0005-tmux-control.md` | `main`. An older revision exists on `cursor/slice-1-stub-seat-79bf` and `cursor/slice-1b-tmux-core-2ef9`; `main` is the newer one. |
| `design/adr/0009-delivery-identity.md` | `main` |
| `design/adr/0010-dialog-driver-and-transcript-ownership.md` | `main` |
| `design/migrations/2026-10-11-slices-2-3.md` | `main` |
| `design/parity-exceptions.md` | `main`. This is the single PE register (ADR-0010), and xtask checks it against the ledger. |
| `docs/qa/PLAN.md` | `main` (the QA parity plan, not the design workspace `PLAN.md`) |

## Parity exceptions held on open PR branches (not merged here)

These entries exist verbatim, but only on their slice branches. Each one lands with its PR. Copying them to `main` early would claim exceptions for code that hasn't merged, would fail the xtask PE/ledger checks, and would make the three PRs conflict.

| PE | Topic | Where it lives |
| --- | --- | --- |
| PE-017 | direct/startup re-delivery | On `main`. The design-id table on `main` still says "reserved"; PR #18 updates it. |
| PE-018 | Claude compaction via both hooks | PR #18 `cursor/claude-compaction-pe018-c6a3` |
| PE-021 | `rig context deliver`; startup not implemented | PR #27 `cursor/slice-19-context-packs-47ff` |
| PE-022 | human workflow destination is the operator surface | PR #28 `cursor/slice-14-workflow-engine-0dd8` |
| PE-023 | project/seat/mission reads refuse every symlink | PR #27 `cursor/slice-19-context-packs-47ff` |

On `main`: PE-001-PE-009, PE-011-PE-017, PE-019 (lineage) and PE-020 (MCP routes); PE-016 is the MCP tool rename. There is no PE-010 heading in any branch.

## Not recovered (no verbatim copy found anywhere)

- `design/README.md` (the original design index). The `README.md` in this directory is a new file index, not the lost original.
- `design/PLAN.md` (the workspace plan)
- `design/adr/0001` - `0004`, `0006` - `0008` (ADR-0002, 0003, 0004 and 0008 are cited in PR bodies, but their text is not there)
- `design/acceptance/` for slices 0, 0b, 1, 1b, 6, 7, 8, 9, 10, 11, 12, 13, 14, 18 and 19
- `qa/STATUS.md` and `spikes/` (QA Bot is to rebuild STATUS)

## Possible further sources

- The cloud agents for slices 9, 14 and 19 (PRs #18, #28, #27) may still have the uploaded specs in their own workspaces.
- Architect, for any spec that has to be rewritten. A rewritten spec must be marked as a rewrite, not a recovery.
