//! Execute a case against one side and compare two sides.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use super::compare::project;
use super::normalize::normalize_observation;
use super::reap::{DaemonGuard, free_port};
use super::types::HermeticHome;
use super::types::{LoadedCase, Observation, Side, Step, Transport, canonical};
use crate::error::{Error, io_err};

/// One finished side, after normalisation and projection.
pub struct Executed {
    /// Canonical JSON, including the trailing newline.
    pub canonical: String,
    /// Scratch `HOME` that was used. The directory is already removed.
    pub home: PathBuf,
    /// Private `tmux -L` name.
    pub tmux_socket: String,
}

/// Run `case` once as the rig-rs binary, in its own hermetic home.
///
/// # Errors
///
/// Returns [`Error`] when the binary cannot be spawned or normalisation fails.
pub fn execute_rs(case: &LoadedCase, bin: &Path) -> Result<Executed, Error> {
    let side = Side::Rs {
        bin: bin.to_path_buf(),
    };
    execute(case, &side, None)
}

/// Run `case` against `side`. `ts_port` is the TCP port for a TypeScript daemon step.
///
/// # Errors
///
/// Returns [`Error`] on spawn failure, a missing TypeScript build, or bad JSON.
pub fn execute(case: &LoadedCase, side: &Side, ts_port: Option<u16>) -> Result<Executed, Error> {
    let home = HermeticHome::create()?;
    let home_path = home.home.clone();
    let socket = home.tmux_socket.clone();
    let rig_home = home.rig_home();
    let http = case
        .case
        .steps
        .iter()
        .any(|step| matches!(step, Step::Http { .. } | Step::EnsureRig { .. }));
    let port = match (side, ts_port, http) {
        (Side::Ts { .. }, Some(port), _) => Some(port),
        (Side::Ts { .. }, None, true) => Some(free_port()?),
        (Side::Ts { .. } | Side::Rs { .. }, _, _) => None,
    };
    let transport = match side {
        Side::Rs { .. } => Transport::Uds(crewmux_api::daemon_socket(&rig_home)),
        Side::Ts { .. } => {
            let port = port.unwrap_or(9);
            let addr = format!("127.0.0.1:{port}")
                .parse()
                .map_err(|_| Error::Message(format!("bad port {port}")))?;
            Transport::Tcp(addr)
        }
    };
    super::bind::bind_transport(side, &transport)?;
    let guard = DaemonGuard::new(&home, side, port);
    if http {
        guard.ensure()?;
    }
    let mut obs = Observation::empty();
    let stepped = (|| {
        for step in &case.case.steps {
            apply_step(step, side, &transport, &home, &mut obs)?;
        }
        Ok(())
    })();
    guard.finish()?;
    stepped?;
    let roots = vec![home.root.path().to_path_buf(), home_path.clone(), rig_home];
    let normalized = normalize_observation(&obs, &case.case.normalize, &roots)?;
    let projected = project(normalized, case.project);
    let text = canonical(&projected)?;
    Ok(Executed {
        canonical: text,
        home: home_path,
        tmux_socket: socket,
    })
}

fn apply_step(
    step: &Step,
    side: &Side,
    transport: &Transport,
    home: &HermeticHome,
    obs: &mut Observation,
) -> Result<(), Error> {
    match step {
        Step::Cli { argv } => {
            let argv = substitute(argv, obs.json.as_ref())?;
            run_cli(side, &argv, home, obs)
        }
        Step::Http {
            method,
            path,
            body,
            record,
        } => run_http(
            transport,
            method.as_str(),
            path,
            body.as_ref(),
            *record,
            obs,
        ),
        Step::ReadFile(path) => read_home_file(home, path, obs),
        Step::WriteFile { path, body } => write_home_file(home, path, body),
        Step::TmuxCalls => collect_tmux(home, obs),
        Step::McpTools => super::mcp_list::collect(side, home, obs),
        Step::EnsureRig { name } => ensure_rig(side, transport, home, name),
    }
}

fn run_cli(
    side: &Side,
    argv: &[String],
    home: &HermeticHome,
    obs: &mut Observation,
) -> Result<(), Error> {
    let (program, prefix) = program_for(side)?;
    let mut command = Command::new(&program);
    command
        .args(&prefix)
        .args(argv)
        .current_dir(&home.home)
        .env_clear()
        .envs(home.child_env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|source| io_err(format!("spawn {}", program.display()), source))?;
    let output = child
        .wait_with_output()
        .map_err(|source| io_err("wait for rig", source))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    push_text(&mut obs.stdout, &stdout);
    push_text(&mut obs.stderr, &String::from_utf8_lossy(&output.stderr));
    obs.exit = output.status.code().unwrap_or(1);
    if let Some(value) = json_body(&stdout) {
        obs.json = Some(value);
    }
    Ok(())
}

fn substitute(argv: &[String], json: Option<&Value>) -> Result<Vec<String>, Error> {
    let mut out = Vec::with_capacity(argv.len());
    for arg in argv {
        out.push(fill_arg(arg, json)?);
    }
    Ok(out)
}

fn fill_arg(arg: &str, json: Option<&Value>) -> Result<String, Error> {
    let Some(start) = arg.find("{{") else {
        return Ok(arg.to_owned());
    };
    let after = arg.get(start..).unwrap_or("");
    let Some(end) = after.find("}}") else {
        return Err(Error::Message(format!("unclosed placeholder in {arg}")));
    };
    let key = after.get(2..end).unwrap_or("");
    let value = json.ok_or_else(|| Error::Message(format!("no JSON yet for {arg}")))?;
    let found = lookup(value, key)
        .ok_or_else(|| Error::Message(format!("placeholder {key} is missing")))?;
    let text = found
        .as_str()
        .map_or_else(|| found.to_string(), str::to_owned);
    let mut rendered = String::new();
    rendered.push_str(arg.get(..start).unwrap_or(""));
    rendered.push_str(&text);
    let rest = after.get(end.saturating_add(2)..).unwrap_or("");
    rendered.push_str(&fill_arg(rest, Some(value))?);
    Ok(rendered)
}

fn lookup<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in key.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn json_body(stdout: &str) -> Option<Value> {
    let trimmed = stdout.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    trimmed
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line).ok())
}

fn program_for(side: &Side) -> Result<(PathBuf, Vec<String>), Error> {
    match side {
        Side::Rs { bin } => Ok((bin.clone(), Vec::new())),
        Side::Ts { node, pkg } => {
            let wrapper = pkg.join("packages/cli/dist/bin-wrapper.js");
            if !wrapper.is_file() {
                return Err(Error::TsMissing {
                    detail: ts_build_hint(pkg),
                });
            }
            Ok((node.clone(), vec![wrapper.display().to_string()]))
        }
    }
}

fn run_http(
    transport: &Transport,
    method: &str,
    path: &str,
    body: Option<&Value>,
    record: bool,
    obs: &mut Observation,
) -> Result<(), Error> {
    let payload = match body {
        Some(value) => Some(serde_json::to_vec(value).map_err(|source| Error::Json {
            context: "encode HTTP body".to_owned(),
            source,
        })?),
        None => None,
    };
    let client = match transport {
        Transport::Uds(path) => crewmux_api::Client::over_uds(path.clone()),
        Transport::Tcp(addr) => crewmux_api::Client::over_tcp(*addr),
    };
    let response = client
        .request(method, path, payload.as_deref())
        .map_err(|err| Error::Message(format!("HTTP {method} {path} failed: {err}")))?;
    if !record {
        if response.status >= 400 {
            return Err(Error::Message(format!(
                "HTTP {method} {path} returned {}",
                response.status
            )));
        }
        return Ok(());
    }
    push_text(&mut obs.stdout, &response.body);
    if let Ok(parsed) = serde_json::from_str::<Value>(&response.body) {
        obs.json = Some(parsed);
    }
    obs.exit = if response.status == 200 {
        0
    } else {
        i32::from(response.status)
    };
    Ok(())
}

fn write_home_file(home: &HermeticHome, relative: &Path, body: &str) -> Result<(), Error> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(Error::Message(
            "refusing to write outside the hermetic HOME".to_owned(),
        ));
    }
    let full = home.home.join(relative);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent)
            .map_err(|source| io_err(format!("create {}", parent.display()), source))?;
    }
    fs::write(&full, body).map_err(|source| io_err(format!("write {}", full.display()), source))
}

fn read_home_file(
    home: &HermeticHome,
    relative: &Path,
    obs: &mut Observation,
) -> Result<(), Error> {
    if relative.is_absolute() {
        return Err(Error::Message(
            "refusing to read an absolute path".to_owned(),
        ));
    }
    let full = home.home.join(relative);
    let text = fs::read_to_string(&full)
        .map_err(|source| io_err(format!("read {}", full.display()), source))?;
    obs.files.insert(relative.to_path_buf(), text);
    Ok(())
}

fn collect_tmux(home: &HermeticHome, obs: &mut Observation) -> Result<(), Error> {
    if home.can_probe_tmux() {
        let code = home.invoke_tmux(&["-V".to_owned()])?;
        if code != 0 {
            return Err(Error::Message(format!(
                "tmux -V via the shim exited {code}"
            )));
        }
    }
    obs.tmux_argv = home.tmux_argv()?;
    Ok(())
}

fn push_text(slot: &mut String, extra: &str) {
    if extra.is_empty() {
        return;
    }
    if !slot.is_empty() && !slot.ends_with('\n') {
        slot.push('\n');
    }
    slot.push_str(extra);
}

fn ensure_rig(
    side: &Side,
    transport: &Transport,
    home: &HermeticHome,
    name: &str,
) -> Result<(), Error> {
    match side {
        Side::Ts { .. } => {
            let body = serde_json::json!({ "name": name });
            run_http(
                transport,
                "POST",
                "/api/rigs",
                Some(&body),
                false,
                &mut Observation::empty(),
            )
        }
        Side::Rs { .. } => super::rig::seed_witness(home, name),
    }
}

fn ts_build_hint(pkg: &Path) -> String {
    format!(
        "The TypeScript CLI wrapper is missing at {}.\n\
Build the pinned tree with: npm ci && npm run build -w @openrig/cli\n\
The checkout must be OpenRig v0.6.9 ({})",
        pkg.join("packages/cli/dist/bin-wrapper.js").display(),
        super::types::OPENRIG_COMMIT
    )
}
