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

## Recovered from cloud-agent transcripts (verbatim tool results of file reads)

| File | Source agent | Original upload | sha256 |
| --- | --- | --- | --- |
| `design/acceptance/slice-19-context-packs.md` | `bc-d347d75b-ce1d-5530-a13a-df8a828847ff` (slice 19). Unpaged `read_file` result, totalLines 49, no range shortening; four identical reads. | `uploads/slice-19-context-packs_16c6.md` | `d458b4d27d4aa793e7e3beecd20288dc98753ae1b698cf94a51a62dac5be99a1` |
| `design/acceptance/slice-12-health.md` | `bc-0f0706c1-914d-5a14-a086-8f3bd8b62` | `uploads/slice-12-health_610e.md` | `2483eb27aca56220ab0860b85172e757d039da84b499e3dbe2063ea8d34abc32` |
| `design/acceptance/slice-14-workflow-engine.md` | `bc-48561a2a-5bbb-55b1-8aa2-d3e45ff80` | `uploads/slice-14-workflow-engine_a6fd.md` | `7a89bf7303e18d0e952bcae7f467e51981e5fc4c4f72819bce33d7d549242266` |
| `design/acceptance/slice-6-mcp-server.md` | `bc-75957a1a-e26e-51c6-916d-4153c4aad` | `uploads/slice-6-mcp-server_ba31.md` | `d00d5f24c281d715462c1f08452d4f428efdd002e8604c61319da1ce2b071a6d` |
| `design/acceptance/slice-8-snapshots-restore.md` | `bc-9009dbde-ae9c-55a0-97f9-3af0f4211` | `uploads/slice-8-snapshots-restore_7354.md` | `40f5d946dae5a5fc5c115a4129102313ce6b7337faead9d73f98dda5d8a9b505` |
| `design/acceptance/slice-9-claude-compaction.md` | `bc-2d249ad1-43bd-59f1-b851-6a24e876c` | `uploads/slice-9-claude-compaction_3059.md` | `3ccba29eca3db4d31cee519f151e257e91ad40e0ed111d57bba20e75e923a990` |
| `design/PORTING-MAP.md` | `bc-7033a4bc-580f-567f-9b09-bc1de4d77` | `uploads/PORTING-MAP_1983.md` | `2f81cdf95ad9ec8a09b442cb430dfb34fb2f02473279bec7295ba37a9b7c9e95` |
| `design/recovery/parity-exceptions-2026-10-11.md` | `bc-a02f7af2-f2d7-5992-a2f6-61129c0dc` | `uploads/parity-exceptions_42b1.md` | `91569313e9019da8760a076cf6b9e2a48a7c64fd30a80c1edef7443a6d12a024` |
| `design/recovery/slice-2-claude-seat-2026-10-11.md` | `bc-761a6396-af76-5ef6-b2b8-e5cd34f7f` | `uploads/slice-2-claude-seat_3cec.md` | `77ebb2f6e478cdd03f038c1cbd1b4210f6d6b4127b0e2eb35d4b4a4feba47195` |
| `design/recovery/slice-3-codex-seat-2026-10-11.md` | `bc-a02f7af2-f2d7-5992-a2f6-61129c0dc` | `uploads/slice-3-codex-seat_8751.md` | `c04f206b2af1405d6265ca33d4eb521fa37a44144aa9411ed5953d539ef48500` |

These are single unpaged `read_file` results with the full line count (`totalLines`) and no shortening flags. Where an agent read a file more than once, every complete read had the same sha256. The PORTING-MAP text is identical in the transcripts of four agents (bc-7033a4bc, bc-30b3f737, bc-07c60799 and bc-7e7e5910).

`design/recovery/` holds older uploaded copies that differ from `main`. `main` stays canonical for these. They are kept for history only.
Agent ids above are cut short to the prefix of the transcript file name.

Uploads that matched `main` byte for byte, so nothing new was added: `0009-delivery-identity` (bc-a02f7af2 and bc-761a6396) and `2026-10-11-slices-2-3` (both agents).

The slice 19 agent was also given `uploads/parity-exceptions.md`, but it never read that upload; it read only the repo copy. So no older copy of the register was recovered.

All rig-rs agent transcripts were dumped on 2026-10-15 and searched.

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

## Not recovered (no complete verbatim copy found anywhere)

- `design/README.md`, the original. Agents bc-a02f7af2 and bc-761a6396 read `uploads/README_*.md`, but their transcripts don't keep those results.
- `design/PLAN.md`, the workspace plan. No upload read found.
- `design/adr/0001` - `0004` and `0006` - `0008`. No upload read found. The `0010` upload reads (5 agents) have no stored results, but `main` already has 0010.
- `design/acceptance/` for slices 0, 0b, 1, 1b, 7, 10, 11, 13 and 18. No upload read found.
- `RUST-AGENT-GUIDE.md`: partial only. The 330-line file survives only as paged reads (bc-07c60799, bc-7033a4bc and bc-30b3f737) with no offsets recorded and overlapping page edges, so it can't be reassembled with confidence.
- The other agents' `parity-exceptions` uploads (bc-761a6396, bc-75957a1a, bc-2d249ad1, bc-0f0706c1, bc-9009dbde and bc-48561a2a) have no stored results.
- `spikes/`.

## Possible further sources

- The cloud agents for slices 9, 14 and 19 (PRs #18, #28, #27) may still have the uploaded specs in their own workspaces.
- Architect, for any spec that has to be rewritten. A rewritten spec must be marked as a rewrite, not a recovery.

## Architect rewrites (added 2026-10-15)

These files are **rewrites, not recoveries**. Each one keeps its `REWRITE` header line. Source: `/workspace/openrig-port/design/rewrites/`.

- ADRs: `0001-async-runtime`, `0002-mcp-crate`, `0003-sqlite-layer`, `0004-tui-crate`, `0006-daemon-ipc`, `0007-yaml`, `0008-provider-config-writes`
- Acceptance specs: `slice-0-skeleton`, `slice-0b-diff-harness-ledger`, `slice-1-stub-seat`, `slice-1b-tmux-core`, `slice-7-provider-setup-doctor`, `slice-10-opencode-seat`, `slice-11-herdr-ghostty`, `slice-13-tui-v1`, `slice-18-mission-control`

Caveats:
- In slice 1b, AC1-AC15 are **renumbered**. The original AC numbers are lost, so AC references in older PRs and QA notes may not match.
- Slice 18's CLI shape is **reconstructed**. No original text or merged code backs it.

### Corrections made against the code on main (`1a9b193`) before committing

- **MSRV:** the rewrite already says 1.99.0 (`rust-version = "1.99"`, `rust-toolchain.toml` channel `1.99.0`). No change.
- **ADR-0003:** the rewrite left the migration runner open. The code uses `rusqlite_migration` (`crewmux-store/src/migrate.rs`), so the text now says so.
- **ADR-0006:** the rewrite said "hyper/axum server, hyper client". The server is axum. The client (`crewmux-api::client`) is a hand-written blocking HTTP/1.1 client over `std::os::unix::net::UnixStream` with no hyper dependency, so the text was fixed to say that.
- **ADR-0008:**
  - The backup was `<file>.rig-rs.bak`; the code writes `$RIG_HOME/backups/<file-id>/first-touch.orig` (`crewmux-provider/src/backup.rs`), so the text was fixed.
  - The marker was `# >>> rig-rs >>>`; the code uses `# >>> rig-rs managed >>>` / `# <<< rig-rs managed <<<` (`codex_block.rs`, `tmux_conf.rs`), so the text was fixed.
- **Slice 0, AC8:** there is no `check-headers` xtask on main, so the text now says AC8 is still open.

## Still missing (as of 2026-10-15)

- `PLAN.md`
- `spikes/`
- `RUST-AGENT-GUIDE.md`
- acceptance specs for slices 15, 16, 17 and 20
