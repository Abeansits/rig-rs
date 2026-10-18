> REWRITE (2026-10-18): reconstructed from records, merged code and PRs; not the original.

# Slice 16: review

**Sources of truth:**
- design README.
- PLAN 3.3 (porting map: Review ~8.1k).
- OpenRig pin: v0.6.9 / 4b48ca21.

**TS sources:**
- `domain/review/`: `types.ts`, `compose.ts`, `gather.ts`, `proof-io.ts`, `freeze.ts`, `brief-spine.ts`, `fleet-compose.ts`.
- The contract (`review/types.ts`, "Living Notes", corrective redesign):
  - **One read contract for all consumers**, with the agents projection scope-parameterised (`slice:<id> | mission:<id> | rig`). There is never a second endpoint.
  - A slice review is **one vertical stack, INTENT → PLAN → DELIVERED**. The old parallel structures (sections/acceptance/join/compare) and the separate `green` field were removed.
  - The per-deliverable `verified` signal comes **only from recorded verdicts**.
- `compose.ts` is **pure**: the same inputs, including the caller-supplied view-time facts `nowIso` and `mainTip`, give byte-identical output. Each section names its source of truth and degrades to a declared value, never invented content.
- `gather.ts` is the impure shell:
  - it reads slice docs and proof artifacts (`proof-io.ts`, `proof/*.md`);
  - attention and agent rows from SQLite;
  - approval stamps from frontmatter, cross-checked against the slice 15 audit row;
  - git facts from the workspace default repo.
  - Each unreadable source degrades to null/"unknown".
- `freeze.ts`:
  - One synchronous compose-and-freeze, invoked **after** the slice 15 `delivery` stamp and its audit row commit.
  - Output is one self-contained HTML file, `REVIEW-<scope-id>-<approved-date>.html`. CSS is inline, images are `data:` URIs, video is a link with an inline poster, and there are zero external fetches.
  - It is written **exclusive-create** through the daemon's allowlisted atomic writer with the actor audited.
  - An existing export makes re-invocation an idempotent no-op (`alreadyFrozen`), and frozen exports are never rewritten.
  - A failed render never un-approves.
  - Media inlining is slice-contained: a prefix check first, then a realpath check, so a symlink cannot escape.
- `brief-spine.ts`: the four spine sections of `MISSION_BRIEF.md` (Building / Progress / Proven / Needs you).
  - They are served fresh with no writes, and written **only** at freeze moments.
  - The write is section-scoped: hand-authored sections and the H1/frontmatter stay byte-untouched. A brief without the pinned H2 schema is never rewritten.
- `fleet-compose.ts` fans out each **registered host's** rig root and unions them.
- Routes `routes/review.ts`: `/agents`, `/rig`, `/fleet`, `/slice/:name`, `/mission/:name`, `POST /freeze`.

## Model (pinned from v0.6.9)
- **Proof artifacts:**
  - Artifact types: `guard|qa|rev1-r1|rev1-r2|adjudication`.
  - Recorded verdict tokens: `CLEAR|BLOCKING|CONCERNING|PASS|NOT-CLEAR`. A missing or out-of-set token is MISSING.
  - Chip text is always the recorded token verbatim; tone is derived separately.
- **Pass mapping:**
  - `qa` passes on `PASS`;
  - `adjudication` passes on `CLEAR` or `PASS`;
  - gates (`guard`, `rev1-*`) pass on `CLEAR`.
- **Selection:** latest wins per `(candidate_sha, artifact_type)` by mtime. A non-passing verdict is superseded only by a later artifact.
- **`verified` per deliverable:**
  - `verified` means a covering `qa|adjudication` artifact that records the comparison (`self_check`) **and** carries a passing recorded verdict;
  - `unverified` means some covering artifact exists without one;
  - `missing` means no covering artifact.
  - It is **never** inferred from artifact presence, CI or "tests green". Unverified and missing render as visibly incomplete and never block.
- **Lineage:** `candidateSha`, `mergeSha` (null = UNMERGED) and `mainTip` give `fresh|stale|unknown` freshness, plus commits behind.
- **Phase:** `locked > review > building > spec > intent`, with top-down precedence.
- **Locks:**
  - The plan-lock is the slice 15 `spec` stamp plus `locked-artifacts`.
  - The proof-lock is the `delivery` stamp.
  - `auditVerified: false` (a stamp with no matching audit row) renders loudly as UNVERIFIED.

## Scope
- Gather (inputs from slices (17), scope (15), queue/workflow trails (14) and proof assets), and pure compose (slice, mission, rig).
- Freeze: the immutable, hash-stable frozen export.
- Proof I/O and the brief spine.
- The routes above. `/fleet` composes the local host only.
- CLI verbs that render reviews: `rig review slice|mission <name> [--json]` and `rig review freeze <slice>`. These are new, because TS reviews were UI-only (see Parity exceptions).

## Non-goals
- The web UI renderings.
- Bundle-carried proof.
- **Multi-host fleet fan-out.** v0.6.9's `fleet-compose.ts` fans out across registered hosts. Cross-host is out of scope per the porting map, so fleet is local host only.
  - **Tracking:** the `fleet-compose` ledger rows are `diverged` under the local-host-only PE below, with a note naming multi-host fan-out as deferred. If cross-host is brought into scope, reopen it as a new slice.
- Computing "green" anywhere (removed in TS).

## Crates touched
- `crewmux-orchestration::review`, plus readers from `::scope`, `::slices` and `::workflow`.
- `crewmux-spec::checkbox` (the slice 15 grammar).
- `crewmux-provider::atomic` (the existing atomic writer, extended with exclusive-create and the root allowlist).
- `crewmux-daemon`, `crewmux-api`, `crewmux-cli`, `crewmux-tui`.

## Public types
```rust
pub enum ReviewScope { Slice(String), Mission(String), Rig }
pub struct SliceReview { intent: IntentSection, plan: PlanSection /* lock + locked_artifacts */, delivered: Delivered, phase: ReviewPhase, lineage: VerifyLineage }
pub struct DeliveredItem { id: String, text: String, proof: Vec<ReviewMedia>, verified: Verified /* Verified|Unverified|Missing — recorded verdicts only */ }
pub enum C1Verdict { Clear, Blocking, Concerning, Pass, NotClear }   pub fn is_passing(t: C1ArtifactType, v: Option<C1Verdict>) -> bool;
pub trait ReviewSources { .. }                                        // fixture-injectable
pub fn gather(scope: &ReviewScope, src: &dyn ReviewSources, clock: &dyn Clock) -> Result<Gathered>;
pub fn compose(g: &Gathered) -> Review;                              // pure, byte-stable
pub enum FreezeOutcome { Frozen { path: PathBuf, sha256: String }, AlreadyFrozen { path: PathBuf, sha256: String } }
pub fn freeze(r: &SliceReview, slice_dir: &Path, writer: &AllowlistedWriter, actor: &TransportActor) -> Result<FreezeOutcome, FreezeError>;
```

## Acceptance criteria
1. **Pure compose:**
   - `compose` is pure and deterministic. For TS fixtures, the INTENT/PLAN/DELIVERED output equals TS's JSON (normalised).
   - A property test confirms that identical `Gathered` input (including `now` and `mainTip`) gives byte-identical serialised output.
2. **`verified` only from recorded verdicts:**
   - `verified` is set **only** from a recorded verdict under the pinned pass mapping, plus `self_check`.
   - No verdict means `unverified` or `missing`, never inferred from "tests green", CI status or artifact presence.
   - The verdict chip text equals the recorded token byte-for-byte.
3. **One contract:** `/agents?scope=slice:x|mission:y|rig` returns the same shape for every scope. A test asserts a single DTO type.
4. **Hash-stable freeze:**
   - Freeze yields an immutable snapshot with a sha256.
   - The same composed review and slice directory give the same bytes and hash.
   - A re-freeze returns `AlreadyFrozen` with the original hash and writes nothing.
   - A frozen export never changes after a source edit.
   - Freeze runs only after a `delivery` stamp with its audit row. Without one it returns `stamp_missing`.
   - A render failure leaves the stamp and audit row intact.
5. **Freeze containment:**
   - A media ref using `..`, an absolute path, or a symlink that resolves outside the slice dir is never inlined; it renders the muted unavailable branch.
   - The export references no external URL (asserted by a scan of the output).
6. **Brief spine:** section-scoped writes only at freeze. Hand-authored sections, the H1 and the frontmatter are byte-identical. A brief without the pinned schema is left untouched and reported.
7. **Degrades:** each unreadable source (git missing, DB row absent, proof dir unreadable) degrades to the declared value without failing the whole review.
8. **Fleet:** the fleet route composes the **local host only**, with the same shape as TS and one member. If remote hosts are registered, they are reported as unsupported, not fatal.
9. **Goldens:** route and CLI goldens, plus an `xtask diff` case on a fixture slice: compose, then freeze, comparing the composed JSON and the export bytes after normalisation.

## Required tests
- Ported `review/*` tests with fixtures. Ledger rows currently `todo`: `review-composer`, `review-freeze`, `review-brief-spine`, `review-read-indexes`, `review-routes`, `review-import-audit`, `progress-review-done-coherence`.
- The purity property test (AC1), freeze hash stability and idempotence (AC4), containment (AC5), brief-spine byte tests (AC6), goldens and the diff case.
- `spec-review-service` and `spec-review-routes` are mapped here too.

## CI vs gated
All in CI.

## Definition of done
AC1–AC9 pass, the review ledger rows are closed, and notices are present.

## Dependencies
- After PR #1.
- **Needs slices 14 (workflow trails, merged), 15 (scope stamps and plan-lock) and 17 (slices indexer)**. Gather reads all three. Compose and freeze can be built against fixture `ReviewSources` before then.
- The UNVERIFIED-stamp cross-check needs `mission_control_actions` (slices 15/18).
- **What already exists on main:** the atomic writer in `crewmux-provider::atomic` (temp, fsync, rename). Exclusive-create and the root allowlist are new.

## Parity exceptions
New IDs are taken at PR time; the next free ID on main is PE-024.
- `rig review …` CLI verbs are added (TS has routes only).
- Fleet is local-host only.
- The freeze outcome carries the sha256 of the written export (TS returns the path only).
