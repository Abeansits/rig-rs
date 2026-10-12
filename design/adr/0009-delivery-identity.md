# ADR-0009: Delivery identity for exactly-once restore: text-hash, not hidden marker
Status: Accepted (Amendment 1: channel scope; Amendment 2: per-provider normalisation from the Mac rerun)

> **Amendment 1 (2026-10-11, PM accepted gap G2): scope by delivery channel.**
> - **Queue rows** carry the obligation in the DB row, exactly once. On restore they get **only the pointer nudge** through the wake ladder (at-least-once, never the content). This matches OpenRig (`queue-wake-ladder.ts`: "the ladder retries the nudge … never the content"), so no fingerprint check applies.
> - **Direct `rig send` messages and startup-file deliveries** get the text-hash check below: re-send only if no matching user turn exists after the delivery cursor.

> **Amendment 2 (2026-10-11, Mac rerun with real logins; `spikes/1-paste-delivery.md` § Mac rerun, `spikes/scratch/mac-rerun/s1-*.json`)**
> - **Transcript fidelity is verified, closing the earlier unverified assumption.** The full delivered text lands in both transcripts, every time:
>   - 25/25 Claude and 25/25 Codex sends (1 line, 50 lines, **64 KB**), each recorded as **exactly one** user message.
>   - 0 early-submit fragments.
>   - Codex stores the text **byte-exact**, including 64 KB.
>   - Claude stores single-line text byte-exact. It stores **multi-line pastes wrapped** in `<pasted_content id="…">…</pasted_content>`, with **each tab expanded to 4 spaces** (sent 2409 B → stored 2614 B; sent 65536 B → stored 65594 B). All 15 multi-line sends match after undoing those two changes.
>   - The large-paste on-screen collapse (`[Pasted text #N …]`) does **not** affect the transcript.
> - **Normalisation is per provider** (spec below). It replaces the single `normalize` from the original decision.
> - The decision (text-hash, no hidden marker) stands. Nothing in the evidence argues for a marker.

## Context
Slice 8 / PE-005 require exactly-once re-delivery on restore for **direct sends and startup files** (Amendment 1). A delivery that was made but not acked is re-sent **only if** the seat's transcript doesn't already contain it. That needs a way to recognise "this queue item" inside each runtime's transcript. Two options were open (S9 open question, README open item 3):
- **Hidden marker**: an id that the transcript stores but that is invisible to, or ignored by, the agent.
- **Text-hash**: a fingerprint of the delivered text, matched against the user turns in the transcript.

The rule given for this decision: choose the hidden marker **only if** the spike evidence shows it survives in Claude, Codex *and* OpenCode transcripts and doesn't confuse the agent. Otherwise choose text-hash.

## Evidence (from `/workspace/openrig-port/spikes/`)
- **S9 / `5-resume-restore.md` + `scratch/5-resume-restore/rig.py`:** the marker used was **visible**, not hidden. It was a plain `[<mid>] ` prefix on the prompt text (`deliver()` sends `f"[{mid}] {body}"`), and transcripts were matched with `mid in text`.
  - For **Claude and Codex**, the "transcripts" were the spike's own **stand-in** `fake_agent.py` files (`convo/<id>.jsonl`), not real Claude `~/.claude/projects/*.jsonl` or Codex rollout files. Real Claude and Codex were **BLOCKED** (no logins). The report lists "that the transcripts contain the marker ids" as still to verify.
  - For **OpenCode** (real 1.19.0), the prefixed text was found through `GET /session/{id}/message`. Visible-text survival in OpenCode is the only real-runtime evidence.
- **S1a / `1-paste-delivery.md`:** Claude collapses large pastes on screen into `[Pasted text #N +M lines]`. Whether the transcript keeps the full text was not tested.
- **No spike tested a *hidden* marker** of any form (zero-width characters, HTML comments, ANSI-invisible text, hook metadata). **No spike tested agent confusion** either: whether a prefix or marker changes the model's behaviour or gets echoed into replies, files or commits.

**Conclusion:** the evidence does not show that a hidden marker survives in Claude or Codex transcripts, and it does not show that a marker leaves agents unaffected. The condition for choosing a hidden marker is **not met**.

## Decision
Use **text-hash** delivery identity. Nothing is injected into prompts.
- `DeliveryFingerprint = sha256(normalize(body))`, stored per direct-send/startup delivery with `delivered_at`, `seat`, and the transcript cursor/position at delivery time (when the runtime exposes one).
- `normalize` is **per provider**: `normalize_for(runtime, text)` = `provider_unwrap(runtime, text)` → `common(text)`.
- **`provider_unwrap(Claude, t)`** (from the real captures):
  1. If, after an optional leading `\n`, `t` starts with `<pasted_content` + `[^>]*>` and ends with `</pasted_content>` (optionally followed by `\n`), remove **only that outermost wrapper** together with one newline directly after the opening tag. The match is anchored, never global, so a body that itself contains the literal string `<pasted_content` keeps it.
  2. Expand every `\t` to **4 spaces**, on **both** the sent body and the transcript text. Claude's conversion is a fixed 4-space replacement, not tab-stop alignment: the spike's `norm()` used `.replace("\t","    ")` and matched 15/15.
  3. If one Claude user turn contains several wrappers (several pastes in one turn), the turn is treated as composite. It is matched by **containment**, never equality, and a contaminated result escalates (ADR-0005 guard, post-paste verification).
- **`provider_unwrap(Codex, t)`** = identity. The capture was byte-exact. Any change found later is a regression to flag, not something to normalise silently.
- **`provider_unwrap(OpenCode, t)`** = identity on the 2.x `content[]` text of the user item, pending a model-turn capture (blocked).
- **`provider_unwrap(Stub, t)`** = identity.
- **`common`** applies to all providers:
  - Unicode NFC.
  - CRLF/CR → LF.
  - Strip the trailing whitespace on each line and collapse runs of trailing blank lines.
  - Strip bracketed-paste control sequences.
  - It does **not** lowercase or collapse internal whitespace, because code and diffs are whitespace-sensitive.
- **Matching:** a `TranscriptProbe` per runtime extracts **user-turn texts** recorded *after* the item's delivery cursor (or, failing a cursor, after `delivered_at − skew`). It normalises and hashes each one, and compares:
  - Claude: session jsonl, user messages.
  - Codex: rollout file under `$CODEX_HOME/sessions`, user items.
  - OpenCode: `GET /session/{id}/message` user parts.
  - Stub: the stub transcript file.
- **Duplicates of identical text** (the same body sent twice to one seat) are disambiguated by **occurrence counting**. Within the window, the number of matching user turns is compared against the number of delivered-not-acked deliveries with that fingerprint, oldest first. Extra items are re-sent.
- **Composite user turns:** if the agent's turn merged our paste with other text (for example a human draft that the guard flagged as `contaminated`), the item is **not** auto-acked or re-sent. It escalates to attention.
- **Acks** come only from a transcript match, never from paste success (unchanged from PE-005).
- **A visible id stays an option per seat**, not the mechanism. A RigSpec may opt in with `delivery_id_prefix: true` (a `[q:<id>] ` prefix, which S9 showed works in real OpenCode), for operators who want ids visible in the conversation. If enabled, matching uses the id and falls back to the hash.

## Consequences
- Prompts are unchanged, so there is no risk of agents echoing or reasoning about markers.
- It depends on each runtime storing the full delivered text. **Verified** for Claude (with the unwrap and tab rule) and Codex (byte-exact) on 64 KB payloads (Amendment 2). Still unverified for OpenCode, which is blocked on a provider.
- **Required fixture tests** (`rig-durability/tests/normalize_real.rs`):
  - For each of the 15 Claude multi-line rows and the 10 single-line rows in `s1-claude.json`, `fp(normalize_for(Claude, sent)) == fp(normalize_for(Claude, stored))`. **The raw (un-unwrapped) fingerprint must differ for the multi-line rows**, which proves the unwrap is needed.
  - For all 25 Codex rows, raw equality.
  - Negative tests: a body containing a literal `<pasted_content>` in the middle is unchanged; a changed internal space does not match; tab vs 4 spaces matches only for Claude.
  - The fixtures are copied into `tests/fixtures/transcripts/{claude,codex}/` with the sent payloads.
- If a runtime turns out to store a transformed or truncated user turn, a per-runtime `normalize` override may hash a stable prefix, for example the first 4 KB after normalisation. This is decided per slice and recorded in parity-exceptions if needed.
- There is a small false-ack risk when an identical text was typed by a human in the same window. Occurrence counting plus the delivery cursor bound it, and the residual risk is accepted.

## Open risks
- Claude and Codex transcript formats are version-specific (tested: Claude Code 2.1.286, codex-cli 0.162.0). The fixture tests pin the versions, and doctor records the installed version so a format change shows up as a fixture failure on upgrade.
- OpenCode 2.x user-item text, not yet captured.
- Transcript discovery after restore (session id → file path), covered by the resume-token work in slices 2 and 3.
