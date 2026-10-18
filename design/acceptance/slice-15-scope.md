> REWRITE (2026-10-18): reconstructed from records, merged code and PRs; not the original.

# Slice 15: scope

**Sources of truth:**
- design README.
- PLAN 3.2 (porting map: Scope ~12.5k including cross-cutting code; "mostly pure logic; good test-port candidate").
- OpenRig pin: v0.6.9 / 4b48ca21. The `domain/scope/`, `lib/scope/` and `commands/scope.ts` files are unchanged at the box checkout bed4d45.

**TS sources:**
- `domain/scope/`:
  - `types.ts`, `dot-id.ts`, `logical-checkbox.ts`, `node-file.ts`, `plan-lock-artifacts.ts`, `scaffold-placeholder.ts`, `scope-approve.ts`, `scope-audit.ts`, `scope-view-projection.ts`, `attestation-lineage.generated.ts`.
- CLI `lib/scope/`:
  - `scope-fs.ts` (frontmatter split/update, missions-root resolution, discovery, `git mv`), `dot-id.ts`, `logical-checkbox.ts` (byte-twin of the daemon copy), `progress-edit.ts`, `scope-audit.ts`, `templates.ts`, `trust.ts`, `capability-delta.ts`, `mission-composition.ts`, `attestation-lineage.ts`.
- Routes:
  - `routes/scopes.ts`: `GET /projects`, `/`, `/slice`, `/narrative`.
  - `routes/scope-approve.ts`: `POST /`.
  - `routes/scope-audit.ts`.
- CLI `commands/scope.ts`:
  - `rig scope [--workspace <path>]`.
  - `slice`: `ls|show|create|ship|close|move|progress|repair|stage|verified|approve`.
  - `mission`: `ls|show|create|graph|progress|repair|stage|verified|approve`.
  - top level: `resolve-notes`, `audit`.

## Model
- **Tiers:** `mission` and `slice` have verbs. `project` and `sub-slice` exist in the grammar but have no verbs in v0.
- **Stable dot-IDs** (convention §1):
  - project `OPR`; mission `<PFX>.<ver>`; slice `<PFX>.<ver>.<n>`; sub-slice `<PFX>.<ver>.<n>.<m>`.
  - A `release-X.Y[.Z]` folder → its semver.
  - Any other mission folder → the escape band `<PFX>.99.0.<m>`, with `m` = max+1 over peer non-release missions. The `0` is fixed, so `99.7.8` is invalid.
  - Tier is resolved by depth.
  - IDs are **minted into mission/slice frontmatter** on the filesystem.
- **Layout (G4, kept as-is):**
  - `<missions-root>/<mission>/slices/<NN-slug>`.
  - Node file precedence: `SPEC.md` then `README.md`. README is legacy and stays valid with no migration.
  - Notes file precedence: `NOTES.md` then `MISSION_NOTES.md`.
- **Stage enum:** `wip|provisional|established|canonical|superseded|retired`. `superseded` names its successor, and invented values are rejected.
- **Errors:** every CLI error is the 3-part `{fact, consequence, action}` shape.
- **Approve (daemon-side, `scope-approve.ts`):**
  - The frontmatter stamp is written first, then the `mission_control_actions` audit row. If the audit write fails, the call fails loudly and the prior frontmatter bytes are restored.
  - No audit row is ever deleted (append-only).
  - Staged scopes:
    - `spec` (the plan-lock) writes `approved-spec-by|at|priors`.
    - `delivery` (the freeze trigger for slice 16) writes `approved-by|at|priors`.
  - `--re-approve` requires a reason and supersedes the prior stamp; both stay in the audit log.
- **Plan-lock (`plan-lock-artifacts.ts`):**
  - A slice `spec` approval co-writes `locked-artifacts` (`{name, path, kind}`) in the same frontmatter write.
  - The derived order is: node pin → selected proof-contract plannedRefs → intent visuals. Paths are normalised and deduped, and the first source wins.
  - A contentless derived set (a scaffold-only node) is refused unless the stamper names the set explicitly with `--locked-artifacts`. An explicit set must be slice-relative, existing, scheme-free and non-escaping.
- **Logical checkboxes:** one grammar shared by scope, review (slice 16) and the slice-detail projector (slice 17).

## Scope
- The dot-ID grammar (parse, format, mint, infer) and mission/slice discovery from the workspace filesystem.
- **Frontmatter read and write that leaves the body unchanged:**
  - A writer owns only the keys it names. They are located via `crewmux-spec::yaml` and spliced by source range.
  - Everything after the closing `---`, and every unowned frontmatter byte (comments, quoting, folded scalars, order), is preserved byte-for-byte.
  - YAML that cannot be spliced falls back to the TS line-based update with a warning.
  - This is the "targeted text edits" path that ADR-0007 anticipates. The YAML is never re-serialised.
- Logical-checkbox parsing; plan-lock derivation; the approve flow (`rig scope slice|mission approve`, recording who, when, which scope and the locked set); re-approve.
- Audit (backstops plus advisories; low and info findings never flip the exit code), view projections, narrative, and every TS `mission` and `slice` verb listed above, each with `--json`.
- Missions-root resolution: `--workspace`, then the env override, then the typed `workspace.slices_root` setting. There is no cwd walk, and an explicit override that names no tree is refused (TS #995).

## Non-goals
- New tiers.
- Changing the on-disk workspace convention. **G4 closed: `workspace/missions/<mission>/slices/<slice>` and the frontmatter keys are kept as-is.**
- Bundles.
- UI-only scope views (the TUI scopes view is the consumer).
- Any import of Node OpenRig state (G7). The work tree is plain files and is read where it is.
- **Plan-lock content hashing and drift detection.** Plan-lock follows v0.6.9: `locked-artifacts` stores `{name, path, kind}` (paths only), with no hash and no drift check on approve. A per-artifact sha256 plus drift reporting is an optional later enhancement. It is not part of this slice and has no PE.

## Crates touched
- `crewmux-orchestration::scope`.
- `crewmux-spec`: `frontmatter` (a markdown splitter plus source-range splice over `crewmux-spec::yaml`) and `checkbox` (the one logical-checkbox grammar).
- `crewmux-store`: the approval audit in `mission_control_actions` (schema owned by slice 18).
- `crewmux-daemon`, `crewmux-api`, `crewmux-cli`, `crewmux-tui` (scopes view).

## Public types
```rust
pub struct DotId { project: String, version: String, n: Option<u32>, m: Option<u32> }   // FromStr / Display
pub enum ScopeTier { Mission, Slice }
pub fn infer_mission_dot_id(folder: &str, peers: &MissionIndex) -> DotId;               // release semver or 99.0.<max+1>
pub struct Mission { id: Option<DotId>, folder: PathBuf, node_file: Option<PathBuf>, slices: Vec<SliceRef> }
pub fn update_frontmatter(md: &str, owned: &[(Key, Value)]) -> Result<String, FrontmatterError>; // body + unowned bytes untouched
pub struct LockedArtifact { name: String, path: RelPath, kind: String }
pub enum ApprovalScope { Spec, Delivery }
pub struct Approval { scope_path: RelPath, scope: ApprovalScope, by: TransportActor, at: Timestamp,
                      locked: Option<Vec<LockedArtifact>>, supersedes: Option<Stamp> }
pub struct ScopeCliError { fact: String, consequence: String, action: String }
pub struct AuditFinding { .. }  pub fn project_view(..) -> ScopeView;
```

## Acceptance criteria
1. **Dot-IDs:**
   - The round trip holds for every TS fixture, including escape-band IDs.
   - Invalid IDs (e.g. `OPR.99.7.8`) are rejected with the TS messages.
   - Minting is monotonic and collision-free under concurrent requests, serialised by the daemon writer.
2. **Frontmatter edits change only the frontmatter block:**
   - Body bytes are identical, verified by a byte-diff test and a body-SHA-256 property test across every editing verb: create, stage, verified, repair, ship, close, move, approve.
   - Unowned frontmatter keys, comments and order are preserved.
   - Where a splice is impossible, the TS line-based fallback runs with a warning. Any comment loss beyond TS goes in a PE.
3. Logical-checkbox parsing matches TS on its fixtures (continuations, nesting, code blocks ignored). There is one Rust implementation.
4. **Plan-lock:**
   - A slice `spec` approval writes the derived `locked-artifacts` set in TS order, in the same write as the stamp.
   - Mission and delivery approvals never write it.
   - A contentless set is refused with the TS remedy text.
   - Explicit sets with a scheme, an absolute or `..` path, or a missing file are refused.
5. **Approve ordering:**
   - With the audit insert forced to fail, the call exits non-zero and the node file is byte-identical to before.
   - A stamp never exists without its audit row, and no audit row is ever deleted.
   - A repeat stamp without `--re-approve` is refused, naming `--re-approve`. `--re-approve` without a reason is refused.
   - After a re-stamp, both attestations are in the log, and the scope is never observed unapproved.
6. **Containment and provenance:**
   - A scope path, named root or locked artifact resolving outside the missions root (including via a symlink) is refused before any write.
   - The actor is transport-derived (`X-Rig-Seat-*`, slice 6), never taken from the body.
7. **Projections:** audit and view projections (`/api/scopes`, `/slice`, `/narrative`, `/projects`) match the TS JSON for the in-scope fields on fixture workspaces.
8. **Goldens:** CLI goldens for every TS scope verb in scope. The `xtask diff` case `scope-create-approve` (create mission → create slice → spec approve → delivery approve on a scratch tree) matches TS after normalisation.

## Required tests
- Ported scope unit tests. Ledger rows currently `todo`:
  - `cli/test/scope-*.test.ts` (dot-id, frontmatter, fs, fs-spec-md, progress, commands, convention-scaffold, intent-visual-template, trust, reapprove-cli, approve-workspace, audit, audit-shadowed-node);
  - `daemon/test/scope-*.test.ts` (approve, approve-append-only, approve-named-root, reapprove, audit-parity, audit-routes, lineage-parity, membership-agreement, node-file-spec-md, view-projection);
  - `scopes-routes`, `workspace-frontmatter-validator`, `tui/test/scopes-view`.
- Fixture workspaces copied from the TS tests.
- Byte-preservation tests (AC2), failure-injection restore (AC5), concurrency on minting (AC1), goldens and the diff case (AC8).
- `scope-audit-parity` (the TS twin byte-equality check) is `diverged` under a new PE, because there is one Rust grammar instead of twins.

## CI vs gated
All in CI. No logins.

## Definition of done
AC1–AC8 pass, the scope ledger rows are closed, and notice headers are present.

## Dependencies
- After PR #1. **Needs slice 0** and **slice 14** (TS scope consumes workflow/slice bindings for admission and narrative). The filesystem and dot-ID core (AC1–AC3) can start after slice 0 alone. AC7 needs slice 14 data.
- **What already exists on main:**
  - Slice 14 merged (`crewmux-spec::workflow`).
  - PE-023's beneath-root, no-symlink reader (slice 19) is available for containment.
- **Missing on main:**
  - Any scope code, and the `mission_control_actions` table.
  - The approve audit (AC5) therefore needs slice 18's table. If 15 merges first, it lands that migration exactly as slice 18 specifies, and 18 reuses it.

## Parity exceptions
New IDs are taken at PR time. The next free ID on main is PE-024; PE-018 is held by open PR #18, and there is no PE-010.
- Single checkbox grammar instead of the TS twin-plus-parity arrangement.
- Env override: `RIG_WORK_ROOT`, with `OPENRIG_WORK_ROOT` honoured as a fallback, if renamed.
