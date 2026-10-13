//! `tools/list` over MCP stdio, and the legacy-name projection for `xtask diff`.

use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use super::types::{HermeticHome, Observation, Side};
use crate::error::{Error, io_err};

const RENAMES: &[(&str, &str)] = &[
    ("rig_add", "agent_add"),
    ("rig_agent_validate", "spec_validate_agent"),
    ("rig_capture", "agent_read_screen"),
    ("rig_chatroom_send", "chat_send"),
    ("rig_chatroom_watch", "chat_read"),
    ("rig_down", "rig_stop"),
    ("rig_ps", "rig_list"),
    ("rig_restore", "snapshot_restore"),
    ("rig_rig_nodes", "agent_list"),
    ("rig_rig_validate", "spec_validate_rig"),
    ("rig_send", "agent_send"),
    ("rig_snapshot_create", "snapshot_create"),
    ("rig_snapshot_list", "snapshot_list"),
    ("rig_status", "daemon_health"),
    ("rig_up", "rig_start"),
];

const DROPPED: &[&str] = &["rig_bind", "rig_bundle_inspect", "rig_discover"];

const INIT: &str = concat!(
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","#,
    r#""capabilities":{},"clientInfo":{"name":"xtask-diff","version":"0.0.0"}}}"#
);

/// Map one legacy or Rust tool name. Dropped tools are `None`.
#[must_use]
pub fn map_tool(name: &str) -> Option<String> {
    if DROPPED.contains(&name) {
        return None;
    }
    if let Some((_, rust_name)) = RENAMES.iter().find(|(legacy, _)| *legacy == name) {
        return Some((*rust_name).to_owned());
    }
    Some(name.to_owned())
}

/// Sorted Rust names. Legacy names are renamed. Dropped names are omitted.
#[must_use]
pub(crate) fn project_names(stdout: &str) -> String {
    let mut names = Vec::new();
    for line in stdout.lines() {
        let name = line.trim();
        if name.is_empty() {
            continue;
        }
        if let Some(mapped) = map_tool(name) {
            names.push(mapped);
        }
    }
    names.sort();
    names.dedup();
    names.join("\n")
}

/// Run `tools/list` and store one raw name per line.
///
/// # Errors
///
/// Returns [`Error`] when the server cannot be spawned or does not list tools.
pub(crate) fn collect(
    side: &Side,
    home: &HermeticHome,
    obs: &mut Observation,
) -> Result<(), Error> {
    let names = list_names(side, home)?;
    obs.stdout = names.join("\n");
    obs.exit = 0;
    Ok(())
}

fn list_names(side: &Side, home: &HermeticHome) -> Result<Vec<String>, Error> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|err| Error::Message(err.to_string()))?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(20), handshake(side, home))
            .await
            .map_err(|_| Error::Message("mcp tools/list timed out".to_owned()))?
    })
}

async fn handshake(side: &Side, home: &HermeticHome) -> Result<Vec<String>, Error> {
    let mut child = spawn(side, home)?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Message("mcp server stdin was not piped".to_owned()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Message("mcp server stdout was not piped".to_owned()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Message("mcp server stderr was not piped".to_owned()))?;
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let read = stderr.read_to_end(&mut buf).await;
        drop(read);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let listed = talk(&mut stdin, stdout).await;
    drop(stdin);
    let waited = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    drop(waited);
    let stderr_text = stderr_task.await.unwrap_or_default();
    match listed {
        Ok(names) => Ok(names),
        Err(err) => Err(Error::Message(format!("{err}\n{stderr_text}"))),
    }
}

async fn talk<W, R>(stdin: &mut W, stdout: R) -> Result<Vec<String>, Error>
where
    W: AsyncWriteExt + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    writeln(stdin, INIT).await?;
    let mut lines = BufReader::new(stdout).lines();
    let init = next_id(&mut lines, 1).await?;
    if init.get("error").is_some() {
        return Err(Error::Message(format!("mcp initialize failed: {init}")));
    }
    writeln(
        stdin,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    )
    .await?;
    writeln(stdin, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).await?;
    let listed = next_id(&mut lines, 2).await?;
    tool_names(&listed)
}

async fn writeln<W: AsyncWriteExt + Unpin>(stdin: &mut W, line: &str) -> Result<(), Error> {
    stdin.write_all(line.as_bytes()).await.map_err(io)?;
    stdin.write_all(b"\n").await.map_err(io)?;
    stdin.flush().await.map_err(io)?;
    Ok(())
}

async fn next_id<R>(lines: &mut tokio::io::Lines<BufReader<R>>, id: i64) -> Result<Value, Error>
where
    R: tokio::io::AsyncRead + Unpin,
{
    while let Some(line) = lines.next_line().await.map_err(io)? {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            return Ok(value);
        }
    }
    Err(Error::Message(format!(
        "mcp server closed before a reply with id {id}"
    )))
}

fn tool_names(message: &Value) -> Result<Vec<String>, Error> {
    let Some(tools) = message
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(Value::as_array)
    else {
        return Err(Error::Message(format!(
            "mcp tools/list had no tools array: {message}"
        )));
    };
    let mut names = Vec::new();
    for tool in tools {
        let Some(name) = tool.get("name").and_then(Value::as_str) else {
            continue;
        };
        names.push(name.to_owned());
    }
    if names.is_empty() {
        return Err(Error::Message(
            "mcp tools/list returned no names".to_owned(),
        ));
    }
    Ok(names)
}

fn spawn(side: &Side, home: &HermeticHome) -> Result<tokio::process::Child, Error> {
    let mut command = command_for(side)?;
    command
        .current_dir(&home.home)
        .env_clear()
        .envs(home.child_env())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    command
        .spawn()
        .map_err(|source| io_err("spawn mcp server", source))
}

fn command_for(side: &Side) -> Result<Command, Error> {
    match side {
        Side::Rs { bin } => {
            let mut command = Command::new(bin);
            command.arg("mcp");
            Ok(command)
        }
        Side::Ts { node, pkg } => {
            let wrapper = pkg.join("packages/cli/dist/bin-wrapper.js");
            if !wrapper.is_file() {
                return Err(Error::TsMissing {
                    detail: format!("missing {}", wrapper.display()),
                });
            }
            let mut command = Command::new(node);
            // `mcp serve` exits before stdio when no daemon is up. `--port`
            // skips that check. tools/list does not open the port.
            command
                .arg(wrapper)
                .arg("mcp")
                .arg("serve")
                .arg("--port")
                .arg("9");
            Ok(command)
        }
    }
}

fn io(source: std::io::Error) -> Error {
    io_err("mcp stdio", source)
}
