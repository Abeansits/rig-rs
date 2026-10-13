//! Apply a [`SetupPlan`](crate::setup_plan::SetupPlan). Installs run only when confirmed.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

use crewmux_provider::{
    AtomicWrite, Backups, ClaudeSeatFiles, ClaudeSeatSettings, CodexConfig, HerdrLaunchConfig,
    ManagedBlock, McpServerEntry, OpenCodePermission, OpenCodeSeatConfig, RIG_MCP_NAME,
    rig_mcp_command,
};

use crate::error::Error;
use crate::output::print_stderr;
use crate::paths::Paths;
use crate::setup_plan::{GHOSTTY_INSTALL, SetupPlan, SetupStep};

const SKILL: &str = include_str!("../../../assets/skills/using-rig/SKILL.md");

/// Confirmation and host facts needed to apply a plan.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each field is a separate setup flag"
)]
pub(crate) struct ApplyCtx<'a> {
    /// Paths captured at startup.
    pub paths: &'a Paths,
    /// `--yes`, or an interactive yes. Never true for `--dry-run`.
    pub confirm_installs: bool,
    /// `--herdr` without `--no-herdr`.
    pub herdr: bool,
    /// `--ghostty` without `--no-ghostty`.
    pub ghostty: bool,
    /// True on macOS. Ghostty's Homebrew command runs only then.
    pub macos: bool,
}

/// Apply every step.
///
/// # Errors
///
/// Returns [`Error`] when a writer or a confirmed install fails.
pub(crate) fn apply(plan: &SetupPlan, ctx: &ApplyCtx<'_>) -> Result<(), Error> {
    let _lock = acquire_lock(ctx.paths)?;
    for step in &plan.steps {
        apply_step(step, ctx)?;
    }
    Ok(())
}

fn apply_step(step: &SetupStep, ctx: &ApplyCtx<'_>) -> Result<(), Error> {
    match step {
        SetupStep::Write { id, change, .. } if change == "unchanged" => Ok(()),
        SetupStep::Write { id, .. } => apply_write(id, ctx),
        SetupStep::Extract { .. } => extract_skills(&ctx.paths.skills_dir()),
        SetupStep::OfferInstall { tool, cmd } => offer(tool, cmd, ctx),
        SetupStep::Skip { .. } => Ok(()),
    }
}

fn apply_write(id: &str, ctx: &ApplyCtx<'_>) -> Result<(), Error> {
    match id {
        "claude_mcp" => write_claude_mcp(ctx),
        "claude_settings" => write_claude_settings(ctx),
        "codex" => write_codex(ctx),
        "opencode" => write_opencode(ctx),
        "tmux_conf" => write_tmux(ctx),
        "herdr_launch" => write_herdr(ctx),
        other => Err(Error::Usage(format!("unknown setup write {other}"))),
    }
}

fn write_claude_mcp(ctx: &ApplyCtx<'_>) -> Result<(), Error> {
    let files = ClaudeSeatFiles::new(ctx.paths.seat_dir());
    let cmd = rig_mcp_command();
    let command = cmd.first().cloned().unwrap_or_else(|| "rig".to_owned());
    let args = cmd.get(1..).unwrap_or(&[]).to_vec();
    files.write_mcp(&[McpServerEntry {
        name: RIG_MCP_NAME.to_owned(),
        command,
        args,
        env: std::collections::BTreeMap::new(),
    }])?;
    Ok(())
}

fn write_claude_settings(ctx: &ApplyCtx<'_>) -> Result<(), Error> {
    let files = ClaudeSeatFiles::new(ctx.paths.seat_dir());
    files.write_settings(&ClaudeSeatSettings {
        permission_mode: Some("default".to_owned()),
        hooks: serde_json::json!({}),
    })?;
    Ok(())
}

fn write_codex(ctx: &ApplyCtx<'_>) -> Result<(), Error> {
    let mut config = CodexConfig::load(&ctx.paths.codex_config)?;
    let before = config.rendered().to_owned();
    // Trust is for a seat's managed cwd, which setup does not have yet.
    config.ensure_mcp_server(RIG_MCP_NAME, &rig_mcp_command())?;
    if config.rendered() == before {
        return Ok(());
    }
    let backups = Backups::new(&ctx.paths.rig_home);
    config.save(&backups)?;
    Ok(())
}

fn write_opencode(ctx: &ApplyCtx<'_>) -> Result<(), Error> {
    let config = OpenCodeSeatConfig {
        permission: OpenCodePermission {
            edit: "allow".to_owned(),
            bash: "ask".to_owned(),
            webfetch: "ask".to_owned(),
        },
    };
    config.write(&ctx.paths.seat_dir())?;
    Ok(())
}

fn write_tmux(ctx: &ApplyCtx<'_>) -> Result<(), Error> {
    let existing = match std::fs::read_to_string(&ctx.paths.tmux_conf) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let next = ManagedBlock::TMUX.upsert(&existing, crewmux_provider::TMUX_BLOCK_BODY)?;
    if next == existing {
        return Ok(());
    }
    if is_read_only(&ctx.paths.tmux_conf) {
        return Err(Error::Usage(format!(
            "{} is read-only",
            ctx.paths.tmux_conf.display()
        )));
    }
    let backups = Backups::new(&ctx.paths.rig_home);
    backups.capture_once(&ctx.paths.tmux_conf)?;
    if let Some(parent) = ctx.paths.tmux_conf.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    AtomicWrite::write(&ctx.paths.tmux_conf, next.as_bytes(), None)?;
    Ok(())
}

fn write_herdr(ctx: &ApplyCtx<'_>) -> Result<(), Error> {
    let launch = HerdrLaunchConfig {
        env_home: &ctx.paths.home,
        xdg_config_home: ctx.paths.xdg_config_home.as_deref(),
        config_path: ctx.paths.herdr_config.as_deref(),
        socket_path: "rig-rs",
        columns: ctx.paths.columns,
    };
    launch.prepare()?;
    Ok(())
}

fn extract_skills(dir: &Path) -> Result<(), Error> {
    let path = dir.join("using-rig").join("SKILL.md");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    AtomicWrite::write(&path, SKILL.as_bytes(), Some(0o600))?;
    Ok(())
}

fn offer(tool: &str, cmd: &str, ctx: &ApplyCtx<'_>) -> Result<(), Error> {
    if !ctx.confirm_installs {
        return Ok(());
    }
    if tool == "herdr" && !ctx.herdr {
        return Ok(());
    }
    if tool == "ghostty" && (!ctx.ghostty || !ctx.macos || cmd != GHOSTTY_INSTALL) {
        print_stderr("skipped Ghostty install: Homebrew cask install runs on macOS only");
        return Ok(());
    }
    run_shell(cmd)
}

fn is_read_only(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    meta.permissions().readonly()
}

struct SetupLock {
    path: std::path::PathBuf,
}

fn acquire_lock(paths: &Paths) -> Result<SetupLock, Error> {
    std::fs::create_dir_all(&paths.rig_home)?;
    let path = paths.rig_home.join("setup.lock");
    for _ in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                let pid = std::process::id().to_string();
                file.write_all(pid.as_bytes())?;
                return Ok(SetupLock { path });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if lock_is_stale(&path, &paths.path_env) {
                    std::fs::remove_file(&path)?;
                    continue;
                }
                return Err(Error::Usage("another setup is running".to_owned()));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::Usage("another setup is running".to_owned()))
}

fn lock_is_stale(path: &Path, path_env: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return true;
    };
    let Ok(pid) = text.trim().parse::<u32>() else {
        return true;
    };
    let Some(kill) = crate::spawn::which(path_env, "kill") else {
        return false;
    };
    crate::spawn::capture(&kill, &["-0".to_owned(), pid.to_string()])
        .is_none_or(|captured| captured.code != 0)
}

impl Drop for SetupLock {
    fn drop(&mut self) {
        drop(std::fs::remove_file(&self.path));
    }
}

fn run_shell(cmd: &str) -> Result<(), Error> {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn()?;
    let output = child.wait_with_output()?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Error::Usage(format!(
            "install command failed: {}",
            stderr.trim()
        )))
    }
}

/// True when stdin looks like an affirmative answer.
#[must_use]
pub(crate) fn stdin_yes() -> bool {
    let stdin = std::io::stdin();
    let mut lines = BufReader::new(stdin.lock()).lines();
    match lines.next() {
        Some(Ok(line)) => matches!(line.trim(), "y" | "Y" | "yes" | "YES"),
        _ => false,
    }
}
