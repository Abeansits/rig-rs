> REWRITE (2026-10-15): reconstructed from records, merged code and PRs; not the original.

# ADR-0008: Rules for writing provider and tool config

Status: Accepted (2026-10-11). Implemented in slice 7 (PR #8). Rules 1–5 are the decision.

## Context
- OpenRig writes into users' provider configs: `~/.claude.json` trust and MCP entries, Codex `config.toml`, tmux config.
- Claude Code rewrites `~/.claude.json` while running, so concurrent writes corrupt it or are lost (repo **PE-004**).
- The port must never damage a user's config and must be reversible.

## Decision

### Common write discipline (all rules)
- **Atomic write:** temp file in the same dir, `fsync`, `rename`, `fsync(dir)`.
- **First-touch backup** at 0600 (`$RIG_HOME/backups/<file-id>/first-touch.orig`, where `<file-id>` is a hex hash of the path; directory 0700), created once and never overwritten.
- **Symlink-aware:** a symlinked target is resolved and written in place. Dangling or looping links are refused.
- **Never trusts `$HOME` blindly:** paths come from the resolved user home with `RIG_HOME` isolation. Tests use a scratch HOME.
- **Lock:** a `setup.lock` serialises apply.

### Rule 1: Claude
- **Never write `~/.claude.json` while any Claude process runs.** If one does, the result is `SkippedClaudeRunning` with a hint.
- Seats get MCP and settings via **flags**: `--mcp-config <seat>/mcp.json --strict-mcp-config --settings <seat>/settings.json`. Seat files are 0600.
- Only `crewmux-provider` `claude/trust.rs` may name the user-config file, enforced by `cargo xtask lint claude-json`.

### Rule 2: Trust pre-accept
- Only when Claude is not running, by compare-and-swap: re-read, verify unchanged, write, verify.
- **auto_trust defaults off** (pending Sebastian's D2). The trust dialog otherwise goes to the dialog driver (ADR-0010).

### Rule 3: Codex
- Edits use **toml_edit**: comments and ordering preserved, our keys inside a **marker block** (`# >>> rig-rs managed >>>` … `# <<< rig-rs managed <<<`).
- Idempotent. `remove_ours` restores the original bytes exactly.
- Invalid TOML reports line/col without echoing values (no secrets).
- **Seats use per-launch `-c` overrides** (`-c mcp_servers.rig.*`). `config.toml` is only touched by explicit `rig setup`.

### Rule 4: tmux and OpenCode
- **tmux.conf:** a managed block only. **No global `window-size`** (it crashes tmux 3.5a) and no global `remain-on-exit`. A load check (`tmux -f … start-server` on a private socket) runs after the write.
- **OpenCode:** a per-seat JSON config at 0600 via `OPENCODE_CONFIG`. **Never `~/.config/opencode`.**

### Rule 5: Herdr
- **Attach-only**: a private launch config at 0600. rig-rs never starts or manages a Herdr server.
- Herdr and Ghostty installs are **offered only** (`OfferInstall`), and execute only with `--yes` plus the explicit flag.

## Consequences
- `rig setup --dry-run --json` prints a `SetupPlan` and writes nothing. `rig doctor` checks without mutating, never creates `$RIG_HOME`, and never starts the daemon.
- Lints `window-size` and `claude-json` guard rules 1 and 4. The window-size scan treats any flag group containing `g` as global.

## Open (pending Sebastian)
- D1 (Codex MCP auto-approve), D2 (auto_trust default), D3 (OpenCode version and model source), D4 (Ghostty strategy). Defaults stay conservative until decided.
