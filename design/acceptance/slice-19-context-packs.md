# Slice 19: context packs
Source of truth: design README. PLAN 3.6 (~5.5k, excluding bundle routing).
TS sources:
- `domain/context-packs/` (types, manifest-parser, context-pack-library-service, profile-composer, profile-source-resolver, ref-safety, token-estimate, seat-recap-store, probe-eval-bridge; `bundle-assembler` is out of scope as bundles).
- Model: a pack is a directory with `manifest.yaml` plus markdown/yaml/txt files. Storage is **filesystem-canonical**: `<context.root>/<ref>/` (default `~/<STATE_DIR_NAME>/context`) and workspace-local `.openrig/context-packs/<name>/`. In rig-rs this becomes `<workspace>/<STATE_DIR_NAME>/context-packs/<name>/` (G4 closed). A `.openrig` compat symlink created by `rig setup` keeps old paths resolving. **No new SQLite tables**; the library cache is in memory.
- Routes `routes/context-packs.ts`: `/library`, `POST /library/sync`, `POST /library/compose`, `/library/by-ref`, ….
- CLI `context.ts`: work-install, trace, compose, list, show, preview, get, profile, recap-write, sync, add, source, rm.
- The `context-pack-taxonomy-surface.ts` wiring.

## Scope
Manifest parsing and validation, the library (discovery across both roots, sync, by-ref lookup), ref safety (no traversal), the profile composer and source resolver, token estimation, compose and preview, **send/deliver a composed pack to a seat** (through slice 4's guarded paste, or as a startup file through the adapter's `deliver_startup`), seat recaps, and the CLI verbs above.

## Non-goals
Bundle-carried packs; the probe-eval bridge if it depends on out-of-scope eval tooling (confirm while porting, and ledger it as waived with a reason); the UI.

## Crates touched
`rig-orchestration::context_packs`, `rig-spec` (manifest), `rig-messaging` (delivery), `rig-adapters` (`deliver_startup` usage), `rig-daemon`, `rig-api`, `rig-cli`. Shipped packs are embedded as assets (`xtask` generation replaces `scripts/generate-context-packs`).

## Public types
```rust
pub struct PackManifest { name, version, description, files: Vec<PackFile>, profiles: Vec<Profile> }   pub struct PackFile { path: RelPath, role: Option<String> }
pub struct PackRef(String);  pub fn resolve_ref(roots: &[PathBuf], r: &PackRef) -> Result<PathBuf, RefError>;   // ref-safety
pub struct Library { packs: BTreeMap<PackRef, LoadedPack> }  impl Library { pub fn sync(&mut self) -> SyncReport; pub fn by_ref(&self, r: &PackRef) -> Option<&LoadedPack>; }
pub fn compose(lib: &Library, req: &ComposeRequest) -> Result<Composed>;   pub struct Composed { text: String, tokens_est: u32, sources: Vec<PackRef> }
pub fn estimate_tokens(s: &str) -> u32;   // TS-compatible heuristic
```

## Acceptance criteria
1. The manifest parser accepts every TS fixture and rejects invalid ones with TS messages (serde-saphyr, merge keys allowed).
2. Ref safety: `..`, absolute paths, and symlinks escaping the pack root are refused.
3. Library sync discovers both roots, and a pack reachable via both `<STATE_DIR_NAME>/` and the `.openrig` symlink is listed once (canonicalised path). A workspace-local pack overrides a global one with the same name, per TS precedence.
4. Compose output is deterministic and byte-equal to TS for fixture profiles. `estimate_tokens` equals TS's estimate on the fixtures.
5. Delivery: given a composed pack of 64 KB, `rig context …send`-style delivery to a stub seat goes through the slice 4 guard and paste path, with a receipt and fingerprint. As a startup file, it goes through `deliver_startup`.
6. No DB writes for the library (in-memory cache only), apart from seat recaps if TS stores them in the DB.
7. CLI and route goldens for each verb.

## Required tests
Ported context-pack tests with fixtures, traversal tests, a compose byte-equality diff case against TS, delivery e2e on stub (AC5), goldens. Ledger mapped. Bundle-related tests are `waived`.

## CI vs gated
- **CI:** everything on stub.
- **Gated (logins):** delivery of a large pack into real Claude/Codex composers, covered by the slice 2/3 live tests.

## Definition of done
AC1–AC7 pass, ledger closed, notices present, workspace-local dir is `<STATE_DIR_NAME>/context-packs` via the constant (G4), and the library resolves through the `.openrig` symlink when present, without double-counting packs.

## Dependencies
After PR #1. **Needs slices 0 and 1** (seats), **4** (guarded delivery), and **1b** (paste). Compose and library (AC1–AC4) can start after slice 0 alone. **Not** 14–18.
