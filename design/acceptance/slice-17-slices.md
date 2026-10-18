> REWRITE (2026-10-18): reconstructed from records, merged code and PRs; not the original.

# Slice 17: slices (Slice Story View indexer)

**Sources of truth:**
- design README.
- PLAN 3.4 (porting map: Slices ~4.5k).
- OpenRig pin: v0.6.9 / 4b48ca21.

**TS sources:**
- `domain/slices/`: `slice-indexer.ts`, `slice-detail-projector.ts`, `qitem-membership.ts`.

**Model (from `slice-indexer.ts`):**
- It reads slice folders from the configured root.
  - The default contract is `workspace/missions/<mission>/slices/<slice>`.
  - An explicitly configured flat root `workspace/slices/<slice>` stays supported, plus additional compatible roots scanned after the primary.
  - Dot-folders are skipped.
- It parses frontmatter with the indexer's own line-based parser (not full YAML), plus acceptance section markers.
- Status maps to `active|done|blocked|draft`. A missing status → `draft`; `draft`/`pending`/`placeholder` → `draft`.
- It stitches records against `queue_items`, `queue_transitions`, `mission_control_actions` and dogfood-evidence dirs (the latest matching proof packet: md, images, video, traces).
- It honours `workflow_spec: <name>@<version>`.
- It is a **read-only projection: no new tables, no new event types, no migrations.** The cache TTL is 60 s, re-walking on the next request after expiry, and a folder unseen at build time forces a rebuild.

**Membership (`qitem-membership.ts` plus the indexer batch):**
- Typed `slice:`/`mission:` tags are authoritative: per JSON element, comma-split legacy form, exact prefix match.
- The display tier may add the gated legacy body-substring fallback, using SQLite LIKE-equivalent matching (ASCII-only folding, `%`/`_` spanning newlines, truncation at NUL).
- Caps: 500 per slice and 500 per term.
- A display-tier match is never promoted to a signal.

**Detail projector:** acceptance rows through the one logical-checkbox grammar (slice 15), docs and proof packets.

**Routes `routes/slices.ts`:** `GET /`, `POST /refresh`, `GET /:name`, `GET /:name/doc/*`, `GET /:name/proof-asset/*` (Range supported). The literal routes are registered before `/:name`.
- **TS traversal guard:** the route rejects `..` in the decoded relative path, and the projector does a **lexical** `path.resolve` prefix check against the slice or proof dir.

## Scope
- The indexer (both folder layouts; `workspace/missions/<m>/slices/<s>` is kept per G4) and frontmatter/acceptance-marker parsing.
- qitem membership (which queue items belong to a slice) and the detail projector.
- Refresh, and safe document and proof-asset serving with correct content types and Range.
- CLI: `rig slices ls|show <name> [--json]` and `rig slices refresh`, so the TUI and skills have a non-UI reader (see Parity exceptions).

## Non-goals
- Writing slice files (scope does that, slice 15).
- The UI.
- Any write to the DB or the work tree.

## Crates touched
- `crewmux-orchestration::slices`.
- `crewmux-store` (read-only connection pool only, ADR-0003).
- `crewmux-daemon`, `crewmux-api`, `crewmux-cli`, `crewmux-tui`.

## Public types
```rust
pub enum SliceStatus { Active, Done, Blocked, Draft }
pub enum Layout { Missions, Flat }
pub struct SliceRecord { name: String, mission: Option<String>, layout: Layout, path: PathBuf, display_name: String, status: SliceStatus, raw_status: Option<String>,
                         frontmatter: BTreeMap<String, FmValue>, acceptance: Vec<AcceptanceItem>, qitems: Vec<QItemId>, actions: Vec<McActionRef>,
                         proof_packet: Option<ProofPacket>, workflow_spec: Option<WorkflowSpecRef>, last_activity: Timestamp }
pub struct SliceIndex { by_name: BTreeMap<String, SliceRecord>, collisions: Vec<NameCollision> }
pub trait SliceDataReader { /* read-only queue_items / transitions / mission_control_actions */ }
pub fn index(roots: &SliceRoots, db: &dyn SliceDataReader) -> Result<SliceIndex>;
pub fn detail(rec: &SliceRecord, db: &dyn SliceDataReader) -> SliceDetail;
pub fn resolve_doc(rec: &SliceRecord, rel: &str) -> Result<OpenedFile, PathError>;        // traversal-safe, beneath-root
pub fn resolve_proof_asset(p: &ProofPacket, rel: &str) -> Result<OpenedFile, PathError>;  // same discipline
```

## Acceptance criteria
1. **Both layouts:**
   - Both are indexed, and a slice's `mission` is set only for the missions layout.
   - A name collision between layouts, or between roots, is reported in `collisions` and on the route, not silently merged.
   - Dot-folders are ignored.
2. Acceptance markers, frontmatter (with the TS line-parser semantics, including quote stripping and `#` comment lines), status mapping and `workflow_spec` parse identically to TS on its fixtures.
3. **Membership:**
   - qitem membership matches TS rules on fixture DB data: typed-authoritative, the legacy fallback only with zero typed rows, LIKE-equivalent matching including the NUL and astral-character cases, and the 500 caps.
   - A missing `queue_items` table → an empty index. A missing `tags` column → body-only fallback.
4. **Read-only:**
   - Indexing, detail and refresh perform zero DB writes (asserted with a writer-actor counter) and zero filesystem writes (fs snapshot).
   - There are no new tables, migrations or event types.
5. **Path traversal:** `/:name/doc/*` and `/proof-asset/*` refuse:
   - `..` segments, raw or percent-encoded (including `%2e%2e` and `%2f`);
   - absolute paths;
   - NUL;
   - backslash tricks;
   - **any symlink component**, resolved beneath the slice/proof root with the slice 19 reader (`openat2` `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS`, or an `O_NOFOLLOW` walk);
   - unknown slice names, which give 404 before any path is built.
   Correct content types are served, and Range requests return 206 with correct bounds.
6. **Cache and refresh:** the cache honours the 60 s TTL (fake clock). `POST /refresh` picks up a new slice folder immediately. A `get` for a folder created after the build forces a rebuild.
7. **Goldens:** route and CLI goldens, plus an `xtask diff` case listing and detailing a fixture tree with both layouts against TS v0.6.9.

## Required tests
- Ported slice tests with fixture workspaces. Ledger rows currently `todo`: `slice-indexer`, `slice-detail-projector-v1`, `slices-routes`, `scope-membership-agreement` (shared with 15).
- Traversal tests (AC5) as a table of hostile paths plus symlink fixtures. The zero-write assertion (AC4). Goldens.

## CI vs gated
All in CI.

## Definition of done
AC1–AC7 pass, the ledger is closed for slices, and notices are present.

## Dependencies
- After PR #1. **Needs slices 0 and 4** (queue tables, merged).
- The mission-control actions join (part of AC3) needs **slice 18**; until it merges, that join returns an empty list.
- Workflow bindings (`domain/workflow/slice-workflow-*`) need **slice 14** (merged).
- Slice 16 consumes this indexer.
- **Already on main:** PE-023's beneath-root, no-symlink reader from slice 19 (`read_source`, test `project_seat_and_mission_reads_refuse_symlinks`), which this slice reuses for AC5. No indexer code exists yet.

## Parity exceptions
New IDs are taken at PR time; the next free ID on main is PE-024.
- **Symlinks:** TS's lexical prefix guard follows a symlink inside the slice dir; Rust refuses any symlink component. This is the same stance as PE-023, so either extend PE-023's covers or add a new PE.
- **CLI:** `rig slices …` is added (TS has routes only).
- **Collisions:** reporting them explicitly is added behaviour if TS silently shadows. This is not verified against TS.
