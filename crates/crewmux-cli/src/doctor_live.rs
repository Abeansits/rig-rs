//! Live doctor probe. It reads files and runs version commands. It creates nothing.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crewmux_api::{Client, daemon_socket};
use crewmux_provider::{CodexProviderAuth, global_window_size_manual, select_codex_provider_auth};

use crate::doctor_eval::codex_view_from_text;
use crate::doctor_facts::{DoctorFacts, HerdrView, HomeState, HostOs, ToolFact};
use crate::paths::{Paths, socket_path_fits};
use crate::spawn::{capture, which};

/// Gather facts from the machine. Missing tools become absent facts.
#[must_use]
pub(crate) fn gather(paths: &Paths) -> DoctorFacts {
    let tmux_conf = fs::read_to_string(&paths.tmux_conf).unwrap_or_default();
    let window_size = window_size_from_conf(&tmux_conf);
    DoctorFacts {
        os: host_os(),
        tmux_version_line: tool_version(paths, "tmux", &["-V".to_owned()]).map(|fact| fact.version),
        tmux_conf,
        window_size,
        daemon_up: daemon_healthy(paths),
        claude: claude_fact(paths),
        claude_running: !claude_pids(paths).is_empty(),
        codex: codex_fact(paths),
        codex_config: codex_config(paths),
        opencode: tool_version(paths, "opencode", &["--version".to_owned()])
            .unwrap_or_else(ToolFact::missing),
        herdr: herdr_fact(paths),
        ghostty: tool_version(paths, "ghostty", &["--version".to_owned()])
            .unwrap_or_else(ToolFact::missing),
        ghostty_aqua: false,
        rig_home: rig_home_state(paths),
    }
}

fn host_os() -> HostOs {
    if std::env::consts::OS == "macos" {
        HostOs::Macos
    } else {
        HostOs::Other
    }
}

fn tool_version(paths: &Paths, name: &str, args: &[String]) -> Option<ToolFact> {
    let program = which(&paths.path_env, name)?;
    let captured = capture(&program, args)?;
    if captured.code != 0 && captured.stdout.trim().is_empty() {
        return Some(ToolFact {
            present: true,
            version: "unknown".to_owned(),
            logged_in: false,
        });
    }
    let version = captured
        .stdout
        .lines()
        .next()
        .unwrap_or("unknown")
        .trim()
        .to_owned();
    Some(ToolFact {
        present: true,
        version,
        logged_in: false,
    })
}

fn claude_fact(paths: &Paths) -> ToolFact {
    let Some(mut fact) = tool_version(paths, "claude", &["--version".to_owned()]) else {
        return ToolFact::missing();
    };
    if let Some(program) = which(&paths.path_env, "claude")
        && let Some(captured) = capture(&program, &["auth".to_owned(), "status".to_owned()])
    {
        fact.logged_in = captured.code == 0;
    }
    fact
}

fn codex_fact(paths: &Paths) -> ToolFact {
    let Some(mut fact) = tool_version(paths, "codex", &["--version".to_owned()]) else {
        return ToolFact::missing();
    };
    fact.logged_in = codex_logged_in(paths);
    fact
}

fn codex_logged_in(paths: &Paths) -> bool {
    let text = fs::read_to_string(&paths.codex_config).ok();
    let auth = select_codex_provider_auth(text.as_deref());
    match auth {
        CodexProviderAuth::EnvKey { env_key, .. } => paths.var_set(&env_key),
        CodexProviderAuth::OpenAiLogin { .. } => codex_login_status(paths),
    }
}

fn codex_login_status(paths: &Paths) -> bool {
    let Some(program) = which(&paths.path_env, "codex") else {
        return false;
    };
    capture(&program, &["login".to_owned(), "status".to_owned()])
        .is_some_and(|captured| captured.code == 0)
}

fn codex_config(paths: &Paths) -> crate::doctor_facts::CodexView {
    match fs::read_to_string(&paths.codex_config) {
        Ok(text) => codex_view_from_text(Some(&text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => codex_view_from_text(None),
        Err(error) => crate::doctor_facts::CodexView::Parse(error.to_string()),
    }
}

fn herdr_fact(paths: &Paths) -> HerdrView {
    if which(&paths.path_env, "herdr").is_none() {
        return HerdrView::Absent;
    }
    HerdrView::Unreachable
}

fn daemon_healthy(paths: &Paths) -> bool {
    let socket = daemon_socket(&paths.rig_home);
    if !socket.exists() {
        return false;
    }
    Client::over_uds(socket)
        .get_healthz()
        .is_ok_and(|response| response.status == 200)
}

fn window_size_from_conf(conf: &str) -> Option<String> {
    if global_window_size_manual(conf) {
        Some("manual".to_owned())
    } else {
        None
    }
}

fn rig_home_state(paths: &Paths) -> HomeState {
    let socket = daemon_socket(&paths.rig_home);
    if !socket_path_fits(&socket) {
        return if paths.rig_home_explicit {
            HomeState::TooLong
        } else {
            HomeState::Fallback
        };
    }
    if path_writable(&paths.rig_home, current_uid(paths)) {
        HomeState::Ok
    } else {
        HomeState::Unwritable
    }
}

fn current_uid(paths: &Paths) -> Option<u32> {
    let program = which(&paths.path_env, "id")?;
    let captured = capture(&program, &["-u".to_owned()])?;
    if captured.code != 0 {
        return None;
    }
    captured.stdout.trim().parse().ok()
}

fn path_writable(path: &Path, uid: Option<u32>) -> bool {
    match fs::metadata(path) {
        Ok(meta) => writable_meta(&meta, uid),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => path_writable(parent, uid),
            _ => false,
        },
        Err(_) => false,
    }
}

fn writable_meta(meta: &fs::Metadata, uid: Option<u32>) -> bool {
    let mode = meta.mode();
    match uid {
        Some(uid) if meta.uid() == uid => mode & 0o200 != 0,
        Some(_) => mode & 0o002 != 0,
        None => mode & 0o222 != 0,
    }
}

fn claude_pids(paths: &Paths) -> Vec<u32> {
    let Some(uid) = current_uid(paths) else {
        return Vec::new();
    };
    let Some(program) = which(&paths.path_env, "ps") else {
        return Vec::new();
    };
    let Some(captured) = capture(
        &program,
        &[
            "-u".to_owned(),
            uid.to_string(),
            "-o".to_owned(),
            "pid=,comm=".to_owned(),
        ],
    ) else {
        return Vec::new();
    };
    if captured.code != 0 {
        return Vec::new();
    }
    parse_claude_pids(&captured.stdout)
}

fn parse_claude_pids(text: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(pid) = parts.next().and_then(|text| text.parse().ok()) else {
            continue;
        };
        if parts.next() == Some("claude") {
            pids.push(pid);
        }
    }
    pids
}
