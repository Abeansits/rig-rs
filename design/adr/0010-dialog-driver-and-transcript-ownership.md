# ADR-0010: One dialog driver, one transcript/delivery crate

Status: Accepted (Architect, 2026-10-11). Triggered by rig-rs PR #15 (slice 2) and PR #16 (slice 3), which each built their own dialog driver, delivery normaliser and blind-Enter PE.
Crate names below use the repo's current `crewmux-*` prefix. The design name is in brackets (`rig-*`); the rename is mechanical and out of scope here.

## Context (what the two PRs contain)

| Concern | PR #15 (slice 2) | PR #16 (slice 3) |
|---|---|---|
| Dialog driver | `adapters/src/dialog/drive.rs`: `trait DialogDriver::answer(seat, &KnownDialog, &DialogOption)`, `MuxDialogDriver` over `Multiplexer`; identity check before and after; `auto_choice()` hard-coded | `adapters/src/dialog/{driver,known,options,policy}.rs`: `DialogDriver::answer(&dyn PaneIo, DialogChoice)`, step loop with a re-capture per key, deny-list of destructive labels, `AnswerPolicy{auto_trust, auto_approve_rig_tools}`, trust folder must equal seat cwd and not `$HOME` |
| Menu parser | `adapters/src/screen_common.rs::menu_lines` keeps unmarked, indented sibling options (needed: after Down, "No, exit" has no marker or number) | `dialog/options.rs::menu_options`: number-stripped label + line index, but **drops unmarked unnumbered siblings** |
| `KnownDialog` | `crewmux-types::dialog` (serde; Claude variants + `CodexUpdate`) | `adapters::dialog::known` (Codex variants + `ClaudeTrust{folder}`, `ClaudeBypass`) |
| Normaliser (ADR-0009) | `crewmux-durability/src/normalize.rs`: `normalize_for`, `fingerprint`, `match_for → Equal/Different/Composite`, anchored unwrap, tests | `crewmux-adapters/src/codex/normalize.rs`: a second copy (`same_delivery`); Codex submit compares **byte-exact** instead |
| Transcript reading | `crewmux-durability/src/probe.rs` (Claude jsonl) + `check.rs::ClaudeDelivery` implementing `adapters::DeliveryCheck` | `crewmux-durability/src/probe.rs::CodexRolloutProbe`, which **imports `crewmux_adapters::codex`** |
| Dependency direction | durability → adapters (trait to dodge a cycle) | durability → adapters |
| Blind-Enter PE | PE-013 | PE-014 |
| Lint | `xtask/src/lint_enter.rs` (allow-list: `claude/submit.rs`, `dialog/drive.rs`) | — (its PE cites the lint) |

**What "durability" means in both PRs.** It is not snapshots or restore yet. It is the **ADR-0009 delivery-identity layer**: per-provider normalisation, fingerprints, reading user turns from the runtime's own transcript (Claude `~/.claude/projects/<slug>/<session>.jsonl`, Codex rollout `response_item` role=user), and the post-submit "did exactly one matching user turn land" check.

**Problem.** Slice 8 (restore) will put the real durability code (snapshots, restore orchestrator, crash-cart) in `crewmux-durability`, and that code drives adapters. If durability also sits *under* adapters for delivery checks, the graph cycles. Both PRs already dodge this differently.

## Decision 1: dialog driver
- **Owner: `crewmux-adapters` [`rig-adapters`], module `dialog`.** There is no seat/gate crate in the layout and none is added. Dialogs are adapter knowledge, and the driver is ~300 lines.
- **Shared, provider-neutral parts** live in `dialog`: the menu parser, recognition, policy and driver. **Each adapter supplies only a table.**
- **`DialogId` moves to `crewmux-types::dialog`** (serde, used by `ReadinessCode`/API). Per-dialog parameters (folder, server, tool, versions) stay in adapters.

```rust
// crewmux-types::dialog
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DialogId {
    ClaudeTrust, ClaudeBypassConsent, ClaudeNewMcpServer, ClaudeFullscreenRenderer,
    CodexUpdate, CodexMcpToolApproval, CodexBackgroundServer,
}

// crewmux-adapters::dialog::table
pub struct DialogSpec {
    pub id: DialogId,
    pub title: TitleMatch,                 // positive match on the title line(s)
    pub options: &'static [&'static str],  // FULL option list, exact labels (number prefix stripped), screen order
    pub params: fn(&str) -> DialogParams,  // extracts folder / server+tool / from+to / server name; never decides
    pub answer: Answer,
}
pub enum TitleMatch { Contains(&'static str), AllOf(&'static [&'static str]) }
pub enum Answer { Escalate, Choose { label: &'static str, gate: Gate } }
pub enum Gate {
    Always,               // e.g. fullscreen → "Not now", Codex update → "Skip"
    AutoTrust,            // RigSpec auto_trust && params.folder canonicalises to seat cwd && != $HOME
    AutoApproveRigTools,  // policy flag && server == RIG_MCP_NAME && tool ∈ ADR-0002 list (D1 pending)
}
#[derive(Default)] pub struct DialogParams { pub folder: Option<PathBuf>, pub server: Option<String>, pub tool: Option<String>, pub from: Option<String>, pub to: Option<String> }
pub trait DialogTable { fn dialogs(&self) -> &'static [DialogSpec]; }   // impl'd by ClaudeAdapter, CodexAdapter (consts CLAUDE_DIALOGS / CODEX_DIALOGS)

// crewmux-adapters::dialog::menu  (one parser; PR #15 sibling rule + PR #16 label/line fields)
pub struct MenuOption { pub label: String, pub number: Option<u8>, pub selected: bool, pub line: usize }
pub fn parse_menu(screen: &str) -> Vec<MenuOption>;
pub fn is_option_menu(opts: &[MenuOption]) -> bool;   // ≥2 options or any numbered → UnknownMenu if unrecognised

// crewmux-adapters::dialog::recognise
pub struct Recognised { pub spec: &'static DialogSpec, pub params: DialogParams, pub menu: Vec<MenuOption> }
/// Title matches AND parsed labels == spec.options (same set, same order). Anything less is not recognised.
pub fn recognise(table: &'static [DialogSpec], screen: &str) -> Option<Recognised>;

// crewmux-adapters::dialog::policy
pub struct AnswerPolicy { pub auto_trust: bool /* RigSpec, default false */, pub auto_approve_rig_tools: bool /* D1 */ }
pub struct SeatContext<'a> { pub cwd: &'a Path, pub home: &'a Path }
pub enum Plan { Answer(&'static str), Escalate(EscalateReason) }
pub fn plan(r: &Recognised, policy: &AnswerPolicy, seat: &SeatContext) -> Plan;  // pure
const NEVER_CHOOSE: &[&str] = &["Update now", "Always allow", "No, exit", "Run without daemon", "Cancel", "Allow"]; // defence in depth; exact or prefix as in PR #16

// crewmux-adapters::dialog::driver
pub enum NavKey { Up, Down }                    // the only keys the driver can move with
pub struct VerifiedCursor { line: usize, label: String, _priv: () }  // constructible only inside driver.rs
#[async_trait] pub trait PaneIo: Send + Sync {
    async fn capture_text(&self) -> Result<String, Error>;
    async fn nav(&self, key: NavKey) -> Result<(), Error>;
    async fn enter(&self, proof: VerifiedCursor) -> Result<(), Error>;   // Enter is impossible without a proof
}
pub struct MuxPane { mux: Arc<dyn Multiplexer>, pane: PaneRef }          // PaneIo over the real Multiplexer
pub struct DialogDriver { pub max_steps: u8 /* default 8 */ }
pub enum DriveOutcome { Answered { id: DialogId, label: &'static str }, Escalated { id: Option<DialogId>, reason: EscalateReason }, NotADialog }
impl DialogDriver {
    /// capture → recognise → plan → loop { nav one step; re-capture; re-recognise (same id) } →
    /// VerifiedCursor only if the selected line's label == planned label and ∉ NEVER_CHOOSE →
    /// enter(proof) → re-capture → expect that dialog id gone. Any mismatch → Escalated, no Enter.
    pub async fn drive(&self, io: &dyn PaneIo, table: &'static [DialogSpec], policy: &AnswerPolicy, seat: &SeatContext<'_>) -> Result<DriveOutcome, Error>;
}
```

- **Tables at merge time** (labels from the real captures):
  - `CLAUDE_DIALOGS`:
    - ClaudeTrust → Choose "Yes, I trust this folder" / AutoTrust
    - ClaudeBypassConsent → Escalate
    - ClaudeNewMcpServer → Escalate
    - ClaudeFullscreenRenderer → Choose "Not now" / Always
  - `CODEX_DIALOGS`:
    - CodexUpdate → Choose "Skip" / Always (must not match "Skip until next version")
    - CodexMcpToolApproval → Choose "Allow for this session" / AutoApproveRigTools
    - CodexBackgroundServer → Escalate
- **Enforcement of PE-006/3a:** no blind Enter, `q`, Ctrl-C, Esc, digits or y/n.
  - **By type:** the driver can only emit `NavKey` and a proof-gated Enter.
  - **By lint:** `xtask/src/lint_enter.rs` (PR #15's file is kept as the one lint; command `cargo xtask lint blind-enter`; CI step in `.github/workflows/ci.yml`).
    - Scan root: `crates/crewmux-adapters/src/**`.
    - `Key::Enter` / `"\r"` allowed **only** in `dialog/driver.rs`, `claude/submit.rs`, `codex/submit.rs`.
    - `Key::CtrlC`, `Key::Escape`, and `Key::Literal` starting with `q`/`y`/`n`/a digit are forbidden everywhere in adapters.
    - Its tests are `xtask/tests/lint_enter.rs`.
- Submit paths remain the only other Enter sites. Each requires `Ready` (empty composer), re-captures to see the paste as a non-empty prefix of the body (PR #16's `draft_is_prefix`), and then sends Enter.
- The stub runtime also sends Enter at `crates/crewmux-daemon/src/deliver.rs:257`, in the `PasteHit::Clean` arm (`app.mux.send_keys(pane, &[Key::Enter])`). That site is outside the adapters scan. It is the stub delivery submit, not a Claude or Codex seat.

## Decision 2: delivery identity / "durability" code
- **Owner: a new leaf crate `crewmux-transcript` [`rig-transcript`].** It depends only on `crewmux-types`, `sha2`, `unicode-normalization`, `serde_json` and `uuid`. **Adapters and durability both depend on it. Durability does not depend on adapters** for delivery, so the slice 8 restore orchestrator can depend on adapters without a cycle.
- `crewmux-durability` goes back to its layout role (snapshots, restore, crash-cart, compaction state) and is empty of slice 2/3 code. Its later restore reconciliation (PE-005) calls `crewmux-transcript`.

```rust
// crewmux-transcript (normalize.rs is PR #15's file, moved verbatim, re-keyed on RuntimeKind)
pub fn normalize_for(rt: RuntimeKind, text: &str) -> String;           // Claude: anchored unwrap + tab→4; Codex/OpenCode/Stub: identity; then common()
pub fn unwrap_outermost(text: &str) -> Option<String>;
pub fn pasted_openings(text: &str) -> usize;
#[derive(Clone, Copy, PartialEq, Eq, Hash)] pub struct Fingerprint(pub [u8; 32]);
pub fn fingerprint(rt: RuntimeKind, text: &str) -> Fingerprint;         // hex via Display
pub enum TurnMatch { Equal, Different, Composite }
pub fn match_turn(rt: RuntimeKind, sent: &str, stored: &str) -> TurnMatch;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] pub struct TranscriptCursor(pub u64);  // byte length before the paste
pub struct UserTurn { pub text: String, pub at: TranscriptCursor }
pub trait TranscriptSource: Send + Sync {
    fn runtime(&self) -> RuntimeKind;
    fn cursor(&self) -> Result<TranscriptCursor, Error>;                  // missing file = 0
    fn user_turns_after(&self, c: TranscriptCursor) -> Result<Vec<UserTurn>, Error>;
}
pub struct ClaudeTranscript { /* home, cwd, session_id */ }  impl ClaudeTranscript { pub fn open(home: &Path, cwd: &Path, session_id: &str) -> Result<Self, Error>; } // uuid-checked
pub fn project_slug(cwd: &Path) -> String;
pub struct CodexRollout { /* path */ }  impl CodexRollout { pub fn open(path: PathBuf) -> Self; }
pub struct SessionMeta { pub id: String, pub cwd: PathBuf, pub timestamp: SystemTime }
pub fn read_session_meta(rollout: &Path) -> Result<SessionMeta, Error>;  // used by adapters' RolloutLocator
pub enum Delivery { Landed, Missing, Composite, Duplicate(usize) }
pub fn verify_delivery(src: &dyn TranscriptSource, since: TranscriptCursor, sent: &str) -> Result<Delivery, Error>; // exactly one Equal ⇒ Landed
```

- `RolloutLocator` (thread-id resolution: before-set, per-cwd lock, first-message correlation) **stays in `adapters::codex`**. It is launch logic, and it calls `read_session_meta` and `match_turn`.
- The adapters' submit paths take `Arc<dyn TranscriptSource>` (FakeTranscript in tests). PR #15's `DeliveryCheck` trait and `ProbeQuery` are replaced by `TranscriptSource`.
- **Codex compares through `match_turn(Codex, …)`**, not raw `==`. The result is the same for real captures (identity + the shared trailing-whitespace/CRLF/NFC cleanup), and there is one rule.

## Decision 3: PE ids
The repo's `design/parity-exceptions.md` is now the **single PE register**. Its ids PE-003…PE-012 were allocated during slices 0–5 and **collide with the Architect workspace ids** (where PE-006 meant blind-Enter and PE-007 compaction). Resolution:
- **Blind Enter = repo PE-013** (first to merge). Its text is replaced by the design rule (old design PE-006, incl. 3a). **PE-014 is retired: superseded by PE-013.** The PE-014 heading stays as a one-line tombstone so ledger rows citing it still resolve; the ledger rows are rewritten to PE-013.
- PE-015 (Codex thread id from rollout, PR #16) is kept.
- Ids reserved for design PEs that have no repo entry yet:

  | Design id | Topic | Repo id |
  |---|---|---|
  | PE-003 | MCP tool rename | PE-016 |
  | PE-005 | exactly-once direct/startup re-delivery | PE-017 |
  | PE-007 | compaction completion via hooks | PE-018 |

  Design PE-001, PE-002 and PE-004 keep the same ids in both registers.

## Consequences
- About 21 conflicting files collapse to one owner each (see `design/migrations/2026-10-11-slices-2-3.md`).
- Adding a provider dialog means one table row plus a fixture; no driver change.
- A new crate (`crewmux-transcript`) is added to the workspace, the dependency graph and the NOTICE header check.
- Risk: `recognise()` requires the **full** option list, so a CLI that adds one option makes the dialog unrecognised. That is intentional: it escalates, the fixture corpus catches it, and the table is updated deliberately.
