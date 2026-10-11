// Derived from openrig packages/tui/src/socket-server.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! JSON-lines control socket. Commands are parsed here and applied only in `update`.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio_util::sync::CancellationToken;

use crewmux_api::{MAX_SUN_PATH_BYTES, SeatId, path_bytes};

use crate::error::Error;
use crate::msg::{ControlCmd, ControlReplyId};
use crate::state::Section;

/// One accepted command, in the order it was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Incoming {
    /// Parsed command.
    pub cmd: ControlCmd,
}

/// Bind `path` mode `0600` and serve clients until `cancel` fires.
///
/// One client is read at a time so each command's reply stays on that connection.
/// The caller spawns this future on its `TaskTracker`.
///
/// # Errors
///
/// Returns [`Error::Socket`] when the path is too long or the bind fails.
pub(crate) async fn serve(
    path: &Path,
    cancel: CancellationToken,
    cmds: tokio::sync::mpsc::Sender<Incoming>,
    mut replies: tokio::sync::mpsc::Receiver<(ControlReplyId, Value)>,
) -> Result<(), Error> {
    if path_bytes(path) > MAX_SUN_PATH_BYTES {
        return Err(Error::socket(format!(
            "socket path is {} bytes; the sun_path guard is {MAX_SUN_PATH_BYTES}",
            path_bytes(path)
        )));
    }
    prepare(path).await?;
    let listener = UnixListener::bind(path).map_err(Error::Socket)?;
    set_mode(path, 0o600).await?;
    loop {
        let stream = tokio::select! {
            () = cancel.cancelled() => return Ok(()),
            accepted = listener.accept() => accepted.map_err(Error::Socket)?.0,
        };
        client(stream, cancel.clone(), &cmds, &mut replies).await;
    }
}

/// Parse one JSON line into a control command. Unknown verbs stay commands so `update` replies.
#[must_use]
pub(crate) fn parse_line(line: &str) -> ControlCmd {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return ControlCmd::Unknown {
            verb: "malformed".to_owned(),
            reply: ControlReplyId(0),
        };
    };
    let reply = ControlReplyId(value.get("id").and_then(Value::as_u64).unwrap_or(0));
    let verb = value.get("cmd").and_then(Value::as_str).unwrap_or("");
    match verb {
        "select" => seat_cmd(&value, reply, true),
        "attach" => seat_cmd(&value, reply, false),
        "section" => section_cmd(&value, reply),
        "refresh" => ControlCmd::Refresh { reply },
        other => ControlCmd::Unknown {
            verb: other.to_owned(),
            reply,
        },
    }
}

fn seat_cmd(value: &Value, reply: ControlReplyId, select: bool) -> ControlCmd {
    let Some(seat) = value
        .get("seat")
        .and_then(Value::as_str)
        .and_then(|raw| SeatId::parse(raw).ok())
    else {
        return ControlCmd::Unknown {
            verb: if select { "select" } else { "attach" }.to_owned(),
            reply,
        };
    };
    if select {
        ControlCmd::Select { seat, reply }
    } else {
        ControlCmd::Attach { seat, reply }
    }
}

fn section_cmd(value: &Value, reply: ControlReplyId) -> ControlCmd {
    match value
        .get("name")
        .and_then(Value::as_str)
        .and_then(Section::parse)
    {
        Some(section) => ControlCmd::Section { section, reply },
        None => ControlCmd::Unknown {
            verb: "section".to_owned(),
            reply,
        },
    }
}

/// Add the reply id and a trailing newline.
#[must_use]
pub(crate) fn frame_reply(id: ControlReplyId, mut value: Value) -> String {
    if let Some(object) = value.as_object_mut() {
        object.insert("id".to_owned(), Value::from(id.0));
    }
    let mut line = serde_json::to_string(&value).unwrap_or_else(|_| "{\"ok\":false}".to_owned());
    line.push('\n');
    line
}

async fn client(
    stream: tokio::net::UnixStream,
    cancel: CancellationToken,
    cmds: &tokio::sync::mpsc::Sender<Incoming>,
    replies: &mut tokio::sync::mpsc::Receiver<(ControlReplyId, Value)>,
) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    loop {
        let line = tokio::select! {
            () = cancel.cancelled() => return,
            next = lines.next_line() => match next {
                Ok(Some(line)) => line,
                _ => return,
            },
        };
        let cmd = parse_line(&line);
        if cmds.send(Incoming { cmd }).await.is_err() {
            return;
        }
        let Some((id, value)) = replies.recv().await else {
            return;
        };
        let frame = frame_reply(id, value);
        if write.write_all(frame.as_bytes()).await.is_err() {
            return;
        }
    }
}

async fn prepare(path: &Path) -> Result<(), Error> {
    let Some(parent) = path.parent() else {
        return Err(Error::socket("socket path has no directory"));
    };
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(Error::Socket)?;
    set_mode(parent, 0o700).await?;
    if tokio::fs::try_exists(path).await.map_err(Error::Socket)? {
        tokio::fs::remove_file(path).await.map_err(Error::Socket)?;
    }
    Ok(())
}

async fn set_mode(path: &Path, mode: u32) -> Result<(), Error> {
    let meta = tokio::fs::metadata(path).await.map_err(Error::Socket)?;
    let mut perms = meta.permissions();
    perms.set_mode((meta.permissions().mode() & !0o777) | (mode & 0o777));
    tokio::fs::set_permissions(path, perms)
        .await
        .map_err(Error::Socket)
}
