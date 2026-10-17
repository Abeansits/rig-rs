> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# Slice 7: Provider config writes, `rig setup`, `rig doctor`

Depends on 0, 1b. Merged as PR #8. Implements ADR-0008. Doctor and setup output differences are repo PE-006.

## Acceptance criteria
- **AC1:** Atomic write (temp → fsync → rename → dir fsync). A crash-injection test leaves either old or new bytes.
- **AC2:** First-touch backup at 0600. A second write never overwrites the backup.
- **AC3:** Symlinks: a resolved target is written in place. Dangling or looping links are refused with an error.
- **AC4:** Claude guard: if a `claude` process runs → `SkippedClaudeRunning`, with no bytes changed.
- **AC5:** Only `claude/trust.rs` names `~/.claude.json` (lint `claude-json`, PE-004).
- **AC6:** Trust is written only when idle, with CAS re-verify. `auto_trust` stays off by default (D2 pending).
- **AC7:** Per-seat `mcp.json`/`settings.json` at 0600. Launch argv contains `--mcp-config`, `--strict-mcp-config` and `--settings`.
- **AC8:** Codex toml_edit:
  - comments are preserved;
  - the marker block is idempotent;
  - `remove_ours` restores the original bytes;
  - invalid TOML gives line/col without echoing values.
- **AC9:** After a Codex registration, `codex mcp list` (stub or real, feature-gated) shows `rig`.
- **AC10:** OpenCode per-seat json at 0600. `~/.config/opencode` is never touched.
- **AC11:** tmux.conf managed block with no global `window-size` and no global `remain-on-exit`. The load check passes on a private server.
- **AC12:** Herdr private config at 0600. No Herdr server is started.
- **AC13:** `rig setup --dry-run --json` prints a `SetupPlan` and makes zero writes (fs snapshot equality).
- **AC14:** `rig setup` apply:
  - is idempotent (second run = no changes);
  - never trusts `$HOME` (uses resolved home + scratch in tests);
  - holds `setup.lock`.
- **AC15:** Herdr and Ghostty installs appear only as `OfferInstall` and run only with `--yes` plus the explicit flag.
- **AC16:** `rig doctor` uses a fixture-driven probe with golden output.
  - Checks include: tmux ≥ 3.3, global window-size (any flag group containing `g` counts as global), provider binaries and versions, the `.openrig`/`.rig-rs` state dir (G4: compat symlink never clobbers a real dir), socket permissions, and MCP registration.
  - **The exact check list is reconstructed.**
- **AC17:** Doctor exits 0 when healthy and 1 on any failure with a fix hint. It never creates `$RIG_HOME` or starts the daemon.
