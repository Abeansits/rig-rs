# rig-rs QA ledger (QA Bot)

Rebuilt 2026-10-15 from QA Bot's memory after a box reset lost the original. Earlier entries are summaries and leave out the per-run detail. Times are PT. "Both" means real tmux 3.3a and 3.8.

## Standing rules
- A PR merges once CI is green and QA has passed. Dev Bot merges. QA Bot never merges, pushes or comments.
- Slice PRs need ported TS parity tests plus PE-* tests.
- Every merge gets a post-merge smoke on main on both tmux versions.
- Known issue already on main: on 3.3a, PE-007 padding fails (capture-stub/stub-seat).

## Post-merge smokes on main (all green)
- 2026-10-11 11:37 · #10 slice 13 TUI · main 1cb1ef1 · nextest 286/1 skip on both, rr13 TUI smoke PASS
- 2026-10-11 19:40 · #14 slice 5 chatroom · main 24c7b2b · 366/1 skip, rr14 chat smoke PASS
- 2026-10-12 00:08 · #13 slice 4 queue/send/ask/wake · main 4d31368 · 459/1 skip
- 2026-10-12 03:10 · #15 slice 2 Claude seat · main aa61e0a · 575/1 skip, ledger 1554 rows OK
- 2026-10-12 05:19 · #16 slice 3 Codex seat · main 0d2b8bc · 648/1 skip, ledger 1712 rows OK
- 2026-10-12 10:25 · #19 slice 8 snapshots/restore/crash-cart · main 634c61b · 710/1 skip, s8 smoke 24/24, RIG_TMUX_SOCKET isolation nit cleared
- 2026-10-12 16:38 · #20 slice 12 health + #25 stub lineage · main 89908bb · 784/1 skip, ledger 1760 rows OK, nightly diff-ts runs on schedule
- #24 merged (f3c2879). No smoke is recorded after the reset.

## Open PR gates
- #18 (current head 6e08b0a): 5702648 PASS WITH NITS. f2b0027 FAIL (conflicted with main). Re-gate of 6e08b0a in progress. Before it merges, Spike's live L1/AC12 run is still needed.
- #27 slice 19:
  - b1fd95b FAIL (4 blockers).
  - 74af399 FAIL: a directory swapped for a symlink mid-read leaked (about 36% of gets, 21 of 60 delivers).
  - Re-gate of 0326319 in progress. The 11 core slice-19 todo ledger rows must be ported (PM's decision).
- #28 slice 14 workflow engine, 85a090f: FAIL (2026-10-15 01:11).
  - Blocker 1: persist absorb()/mutate_pair can persist book N+1 with doc N.
  - Blocker 2: snapshot() takes the two locks separately, so a reader can see a half state. Also, continue/show return an empty frontier, failures, unknowns and boundaryObligations.
  - Nits: no runtime deadline/keepalive firing; nudges don't wake the owner; the boot sweep uses World::default(); AC4 vs TS 400; guard_close race; 2 todo ledger rows.
  - Green: nextest 874/874 on both, diff vs v0.6.9 Equal, issue matrix 106/106.

## Carried nits
- #13: a kill window when two events land on the same tick.
- #20: a fd that isn't CLOEXEC could leak during a swap.
- TUI misses resize when it isn't the process-group leader.
