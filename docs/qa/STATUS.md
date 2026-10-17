# rig-rs QA ledger (QA Bot)

Rebuilt 2026-10-15 from QA Bot's memory after a box reset lost the original. Earlier entries are summaries and leave out the per-run detail. Times are PT. "Both" means real tmux 3.3a and 3.8 (3.8 still latest upstream; no newer tmux/tmux release).

## Standing rules
- A PR merges once CI is green and QA has passed. Dev Bot merges. QA Bot never merges, pushes or comments.
- Slice PRs need ported TS parity tests plus PE-* tests.
- Every merge gets a post-merge smoke on main on both tmux versions.
- Known issue already on main: on 3.3a, PE-007 padding fails (capture-stub/stub-seat) vs TS — not a new red. `xtask diff --self` is Equal on both.

## Slice map (main tip 1a9b193, 2026-10-15 02:58 PT)
Merged on main:
- 1 / 1b stub+tmux · 2 Claude · 3 Codex · 4 queue/send/ask/wake · 5 chatroom · 6 MCP (#24 f3c2879) · 8 snapshots (#19) · 12 health (#20) · 13 TUI (#10) · 14 workflow engine (#28 781e0bd) · stub lineage (#25)
- Docs: #29 restore design tree (59091ef), merged just before #28 · #31 slice-14 AC note naming timer-firing owner (1a9b193, merged 02:47 PT; firing tracked as slice 4 follow-up #30)
In review / open:
- #18 slice 9 Claude compaction (head 9faa125; was 6e08b0a) — prior gate PASS WITH NITS; Spike L1/AC12 still needed
- #27 slice 19 context packs (head e7cd079; was 8d00611) — last recorded gate FAIL (1 blocker: pack-root symlink after sync); head unchanged at 02:58 PT, not re-gated

## Parity ledger (main tip 1a9b193, parity/ledger.csv)
- ledger check: ok (1819 rows) — unchanged since 781e0bd (#31 touched only design/acceptance/slice-14-workflow-engine.md); no TS checkout on box, so check ran without OPENRIG_DIR
- by status: ported 245 · reused 17 · waived 461 · diverged 65 · todo 1031
- by slice (assigned): 1 (ported4/reused3/waived11/diverged2/todo10) · 1b (5/0/0/0/6) · 2 (38/0/97/8/6) · 3 (25/0/106/34/0) · 4 (18/0/0/0/7) · 5 (38/0/0/0/2) · 6 (8/0/0/17/0) · 7 (11/0/0/1/15) · 8 (16/0/0/0/16) · 12 (26/0/11/2/30) · 13 (21/14/4/0/98) · **14 (35/0/56/1/2)** · unassigned (todo839/waived176)

## Post-merge smokes on main
- 2026-10-11 11:37 · #10 slice 13 TUI · main 1cb1ef1 · nextest 286/1 skip on both, rr13 TUI smoke PASS
- 2026-10-11 19:40 · #14 slice 5 chatroom · main 24c7b2b · 366/1 skip, rr14 chat smoke PASS
- 2026-10-12 00:08 · #13 slice 4 queue/send/ask/wake · main 4d31368 · 459/1 skip
- 2026-10-12 03:10 · #15 slice 2 Claude seat · main aa61e0a · 575/1 skip, ledger 1554 rows OK
- 2026-10-12 05:19 · #16 slice 3 Codex seat · main 0d2b8bc · 648/1 skip, ledger 1712 rows OK
- 2026-10-12 10:25 · #19 slice 8 snapshots/restore/crash-cart · main 634c61b · 710/1 skip, s8 smoke 24/24, RIG_TMUX_SOCKET isolation nit cleared
- 2026-10-12 16:38 · #20 slice 12 health + #25 stub lineage · main 89908bb · 784/1 skip, ledger 1760 rows OK, nightly diff-ts runs on schedule
- #24 merged (f3c2879). No smoke recorded (box reset gap).
- **2026-10-15 02:22–02:41 · #29 docs + #28 slice 14 · main 781e0bd · GREEN**
  - Merged since last smoked tip (89908bb / f3c2879): #24 (already on tip, unscanned), #29 (59091ef), #28 (781e0bd tip).
  - tmux: 3.8 still latest upstream (tmux/tmux releases); used /opt/tmux-3.3a and /opt/tmux-latest→3.8.
  - nextest `--workspace --all-features --profile ci --locked`: **876 passed / 1 skipped** on 3.8 and on 3.3a.
  - smoke `qa/scripts/rr28m-smoke.sh` (rebuilt): **27/27 PASS** both — binary help/version, stub up/ps/capture/send, queue list, ask help, chatroom, TUI q, down; slice-14 validate/instantiate/show/continue/list/specs + daemon kill→relist (boot sweep) + show-after-restart; no orphan smoke sockets.
  - xtask: ledger check ok (1819); lint-mux ok; `diff --self` no failures on 3.8 and 3.3a (capture-stub Equal; workflow-linear-build ignored in self-mode as expected).
  - main CI on tip: run **38712474485** success, 13/13 (incl. ci-pass, real tmux 3.3a/3.5a/3.8).
  - nightly diff-ts: workflow active; latest scheduled run **38553694857** success (2026-10-14 04:01 PT, head f3c2879). Prior scheduled: 38404525555 success, 38261231833 success. No failed scheduled run.
  - Verdict: **GREEN**. No new reds. Pre-existing carried nits unchanged. PE-007 3.3a-vs-TS padding remains known-on-main (not hit by `--self`).
- **2026-10-15 02:50–02:58 · #31 docs(slice-14) AC note · main 1a9b193 · GREEN**
  - Delta 781e0bd..1a9b193: commit 61d01c7 + merge; 1 file, design/acceptance/slice-14-workflow-engine.md +4 (AC note: slice 14 arms but doesn't fire deadline/keepalive/owner-wake timers; firing owned by slice 4 wake ladder, tracked in #30). Docs-only.
  - Tarball of 1a9b193 → /tmp/rr31m/src; `cargo build --locked` on 1.99.0 ok.
  - tmux: 3.8 still latest upstream; /opt/tmux-3.3a and /opt/tmux-latest→3.8.
  - nextest `--workspace --all-features --profile ci --locked`: **876 passed / 1 skipped** on 3.8 and on 3.3a (= #28 baseline).
  - smoke `qa/scripts/rr31m-smoke.sh` (rr28m with paths moved to /tmp/rr31m): **27/27 PASS** both; no live servers left on smoke sockets.
  - xtask: ledger check ok (1819); lint-mux, lint-blind-enter, lint claude-json/window-size/mcp-legacy-names/mcp-state-path ok; `diff --self` no failures on 3.8 and 3.3a.
  - main CI on tip: run **38715551395** (push) success, 13/13, done 02:57 PT.
  - nightly diff-ts: latest scheduled still **38553694857** success (2026-10-14 04:01 PT, head f3c2879); next due ~04:00 PT 10-15. No failed scheduled run.
  - Verdict: **GREEN**. No new reds.

## Red items
- (none on main tip 1a9b193 after #31 post-merge smoke, 2026-10-15 02:58 PT)
- Pre-existing known (not new): 3.3a PE-007 padding vs TS on capture-stub/stub-seat.
- Open-PR blockers (not main): #27 pack-root symlink leak (see gate notes below).

## Carried nits
- #13: a kill window when two events land on the same tick.
- #20: a fd that isn't CLOEXEC could leak during a swap.
- TUI misses resize when it isn't the process-group leader.
- #28 nits (accepted into main with PASS WITH NITS): absorb drops pending doc on newer doc-less write (latent/test-only); no daemon-level concurrent HTTP reader test; inspect `waiting` null; exceptionObligations/exceptionReadiness/reconciliation absent; snapshot restore replaces queue book without workflow lock (pre-existing); runtime deadline/keepalive firing + owner wake deferred (see #31 AC note).

## Open PR gates (stale heads may have moved — re-gate before merge)
Prior detailed gate writeups below (heads may have advanced since).

## #27 slice 19 re-gate @ 8d00611 (2026-10-15 ~02:05 PT, QA) — FAIL (1 blocker)
- Gated 0326319 first, then moved to 8d00611 after Dev pushed it. nofollow.rs is the same in both.
- BLOCKER: a pack root swapped for a symlink after sync leaks. Static test (real dir at sync, then `mv` plus `ln -s` to an outside dir): `rig context get` rc0 returns the secret, and `deliver` rc0 pastes it. Both tmux versions.
  - Live root-swap race: get leaks 199/400 (3.3a) and 193/400 (3.8); every deliver succeeded and pasted the secret.
  - Cause: `open_root` opens the root with openat(CWD, root, O_DIRECTORY) and follows symlinks. Only the components below the root are protected.
- FIXED: the component swap race leaks 0/400 gets and 0 pastes on both versions, including with openat2 forced to ENOSYS through a seccomp wrapper (the walk fallback). The race test fails when reverted to read_to_string (55 leaks), so it is real.
- Static cases pass: hardlink refused; symlink refused for get, preview and deliver; HTTP 400; startup 501 before lookup (a nonexistent ref also gives 501).
- Ledger is closed honestly. Slice 19 is 174 ported, 6 waived, 4 diverged, 0 todo. All TS it() titles in the 11 files map to a row, and every rust_test exists. Four random cases and the PE-023 case are faithful.
- Green: nextest 1047/1047 on 3.8 and 3.3a; fmt, clippy, deny, the 4 xtask lints, generate --check, ledger check (1938 rows), diff --self ×2 and diff vs the TS pin; decoys survived; no orphans.
- Smokes not run (scripts lost in the box reset). Earlier nits stand.

## #18 @6e08b0a re-gate (2026-10-15, /tmp/rr18f, rebuilt box) — PASS WITH NITS
- Head 6e08b0a unchanged since 10-12; main f3c2879; MERGEABLE/CLEAN. CI 38337695382 13/13 ✓, diff-ts 38337695373 ✓.
- Delta 65de628..6e08b0a: Book::cancel also cancels a Completed (restoring) attempt; PE-017 row no longer "reserved"; claude_registration asserts MCP env + run-settings hooks; settle test cancel_during_restore_drops_the_pending_prompt. (PE-016/020 and migration 8 were already right at 65de628.)
- Mutation: old cancel() → cancel_drops_a_due_restore and cancel_during_restore_drops_the_pending_prompt both FAIL; restored → pass.
- Cancel during restoring, live 6/6 on 3.8 and 6/6 on 3.3a: cancelled restore never pasted, next compact rc 0, its restore delivered → FIXED.
- launch.rs merge: live seat on PR = main's MCP entry/env/allowedTools + 5702648's per-run hook settings; env identical. Scratch test (65de628, file unchanged since): permission mode borrowed, lands in seat and run settings.
- fake_api.rs: PR's until_ready helper kept (also retries exchange errors; test-only nit). TUI smoke ✓.
- PE: PE-016 MCP rename, PE-020 MCP routes as on main; no dups/stale; ledger ok (1784).
- Lineage live, 3 stages × respawn/exec-other/exec-bash, both tmux: refused (pid/session/bare_shell), 0 B, no Enter; restore control (exec same session) delivers. No lineage row → resume_unproven, 0 B.
- Migration: main f3c2879 DB v7 → v8 ✓. 5702648 v5 DB refuses (documented in PR body; nit).
- Round 3 both tmux ✓. Help union ✓ (hook hidden).
- nextest ci 882/1 skip both tmux, decoys up, no orphans. Gates ✓, lint_enter 20/20. xtask diff 3.8 none; 3.3a capture-stub PE-007 stale (known padding). Smokes (reconstructed) ✓ both.
- Outstanding: live L1/AC12 on pinned Claude (Spike).

## PR #29 docs check (head 6bbb42e, was 23d5adb) — PASS WITH NITS
- Docs only: design/README.md, design/RECOVERY.md, docs/qa/STATUS.md, plus design/acceptance/slice-19-context-packs.md added in 6bbb42e (sha256 matches RECOVERY's d458b4d2…; not on #27's branch).
- All paths cited in RECOVERY exist on main; no markdown links; diff --check clean. docs/qa/STATUS.md was identical to this file before this append.
- CI on 6bbb42e still running at check time; diff-ts skipped.

## rig-rs #28 re-gate @ 1aec47b3d22462a93c8f845cd683dad1d651d7ca: PASS WITH NITS (PM ruling needed on runtime-firing todos) (2026-10-15, /tmp/rr28c)
- Head is 1aec47b, 1 commit ahead of 85a090f. 11 files: persist.rs, queue_http.rs, workflow.rs, workflow_http.rs, workflow_tests.rs, new orchestration inspect.rs, PE-022 text, 6 ledger rows. OPEN, MERGEABLE/CLEAN.
- CI verified independently: run 38707731991 (CI) and run 38707731957 (diff-ts) both ran on head_sha 1aec47b and concluded success; all 14 checks pass.
- Blocker 1 (out-of-order commits) is fixed.
  - Injected build /tmp/rr28c-inj adds an env-driven random delay before the commit and env toggles that revert each fix.
  - 60 instances, 6 drivers, 3 readers, kill -9 mid-run, then the store is checked.
  - Head: 0 mismatches and 0 acked-but-lost in 12 runs over 3.8 and 3.3a.
  - Reverting BOTH fixes (lock dropped before commit + old absorb) is caught: persisted mismatch "bad 2" and acked-but-lost 1-2, in 4 of 9 runs.
  - Reverting either fix alone wasn't caught in 5 runs each, so each fix alone holds under this load.
- Blocker 2 (reader half state) is fixed.
  - Readers hammer GET show and POST continue during mutations and flag any frontier packet that isn't pending, in-progress or blocked.
  - Head: 0 half states on both tmux versions.
  - Reverting read_pair to separate locks is caught: 3-7 handed-off frontier rows per run (3.8 and 3.3a).
- inspect parity vs live TS v0.6.9 (ad-hoc diff case; instantiate, then show/continue): frontierPackets/frontier entries are equal to TS on every field except `waiting` (TS returns a WaitingView, Rust null). failures, unknowns and boundaryObligations are equal. TS show also has exceptionObligations and exceptionReadiness, and continue has reconciliation; Rust has none of these.
- guard_close now runs under the pair lock (mutate_open). Live: every out-of-band close is refused (5 update states, handoff, handoff-and-complete); fallback retargets without closing. Boot sweep now uses world(app). PE-022 records the 400 and cites a test that exists.
- Gates: fmt, clippy, lint-mux, lint blind-enter, ledger (1819 rows) and deny all ok. nextest ci 876/876 on 3.8 and on 3.3a. diff --self: no failures; diff_harness 5/5. Real diff vs v0.6.9: no failures, linear-build Equal. Issue matrix 106/106 against live TS.
- Live on 3.8 and 3.3a: race 1 win / 9 refused; continue changes nothing; unresolved owner causes no mutation; kill -9 loop 0 bad; boot sweep hash identical; 0 pane bytes.
- Decoys untouched. A shared rig-rs server with session "decoy" (not mine; likely the #18 run's own) disappeared during the 3.8 nextest. No test uses a real tmux on "rig-rs" (only FakeMux), and my named decoys survived.
- Orphans: 6 control clients left by my injected daemons after kill -9 + pkill (harness artifact, cleaned up). 0 left from nextest.
- Runtime-firing todos: deadline-policy and keepalive-policy are now `todo`, pointing to the watchdog outbox (slice-12 watchdog rows are also todo). Slice-14 scope lists "deadlines, keepalive watchdog arming, exceptions/escalation". AC5 asks for a fake-clock deadline firing (met at the evaluator level) and a terminal disarm in the same tx (met). The DoD/required tests say "all workflow-* tests mapped". No runtime timer fires and owners are never woken: PM MUST RULE whether that is accepted for slice 14 or needs a PE/AC note naming the slice that delivers it. The two remaining todos are slice-13 TUI rows.
- NITS:
  - The new absorb drops a pending doc when a newer doc-less write arrives (its own test asserts this). Unreachable today because every prod mutation holds the pair lock through commit and submit_queue is test-only. Latent.
  - There is no daemon-level concurrent HTTP reader test, only the store-level one.
  - inspect `waiting` is null; exceptionObligations, exceptionReadiness and reconciliation are absent.
  - snapshot.rs restore replaces the queue book without the workflow lock (pre-existing).

## State as of 2026-10-15 02:45 PT (post-merge smoke for #29) — superseded
- Intermediate note while #28 was still merging. Superseded by the 2026-10-15 02:22–02:41 post-merge smoke on tip **781e0bd** (#29 + #28) above. Ledger on tip is 1819 rows (slice 14 closed most of its todos). #27 head has since moved to e7cd079.

## #27 re-gate @ e7cd079 (2026-10-15 02:46 PT, QA). CHECKPOINT, not a full gate (time box)
- Pack root swapped for a symlink is FIXED. Static test is refused (rc1, no paste) on 3.8, 3.3a and with openat2 forced to ENOSYS. Root race: 0/400 leaks and 0 pastes on all three.
- Race inside the pack: 0/400 leaks and 0 pastes on all three.
- OPEN (PM to decide): swapping the library folder `$RIG_HOME/context` itself for a symlink after sync still leaks. get rc0 shows the secret and deliver rc0 pastes it, on all three. An intermediate parent was not tested.
- NOT RUN: the revert check on the new test, full nextest on both versions, gates, ledger, diff, decoys. CI 38714440654 / 38714440385 was still in progress at 02:41.
- target/ deleted. The clone is kept in /tmp/rr27d/src for the resume.

## #18 6e08b0a → 9faa125 delta (2026-10-15 02:31–02:48, /tmp/rr18g) — PARTIAL, no blockers found; stopped at the time limit
- Head 9faa125 (2d4c624 trust and compact fixes, cb2bd8f main merge of 781e0bd, 9faa125 snapshot fix). CI 38713365645 13/13 ✓, diff-ts 38713365685 ✓. MERGEABLE.
- Trust: revert dialogs.rs and policy.rs → wrapped_padded… and tilde_folder… FAIL (prefix/different-folder refusal passes either way, as it should).
- Trust adversarial (scratch test). Accepted: wrapped mid-component, trailing U+FFFD, trailing slash, ~/proj. Refused: U+FFFD inside a name, Cyrillic lookalike, prefix, different folder, ~/other, ~u/proj, cwd==$HOME (~, abs, trailing /), symlinked home in either direction (lexical, fails closed), .., NBSP inside.
  - Note: two absolute rows "/home/u" + "/proj" are joined into a match (accepted).
  - Note: a non-blank word row right under the path is joined and refuses (fails closed).
- 2.1.287 trust fixture is a reconstructed shape, not a byte capture. Spike's live run is what proves it.
- Migration: 781e0bd DB v8 → v9 ✓, f3c2879 DB v7 → v9 ✓ (workflow + compaction_attempts present), fresh = v9. One list, len 9: workflow 8, compaction 9, no collision.
- 3.8 with decoys: nextest ci 917 pass / 1 skip, xtask diff ×2 none, decoys up, no orphans. fmt, clippy, ledger, lint-mux, blind-enter ✓.
- NOT DONE: compact-fix live repro on both tmux and mutation of compact_after_send_failed_then_incomplete_sends_again; 3.3a suite; lineage and smokes re-run; cargo deny.
