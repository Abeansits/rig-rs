// Derived from openrig packages/daemon/src/domain/terminal/herdr-adapter.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Herdr attach-only client.
//!
//! One JSON line per connection. We ping, create a workspace, and `layout.apply`
//! a single pane. We never start or stop the Herdr server.

use std::path::Path;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::error::Error;

/// Open one pane that runs `command`. `socket` is Herdr's control socket.
///
/// # Errors
///
/// Returns [`Error`] when Herdr is not listening or rejects the layout.
pub async fn open_pane(socket: &Path, label: &str, command: &[String]) -> Result<(), Error> {
    let _ping = request(socket, "ping", json!({})).await?;
    let created = request(socket, "workspace.create", json!({ "name": label })).await?;
    let workspace = workspace_id(&created).ok_or_else(|| {
        Error::Protocol("workspace.create did not return a workspace id".to_owned())
    })?;
    let _applied = request(
        socket,
        "layout.apply",
        json!({
            "workspace_id": workspace,
            "tab_label": label,
            "focus": true,
            "root": { "type": "pane", "label": label, "command": command }
        }),
    )
    .await?;
    Ok(())
}

fn workspace_id(value: &Value) -> Option<String> {
    value
        .pointer("/result/workspace_id")
        .or_else(|| value.pointer("/result/id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

async fn request(socket: &Path, method: &str, params: Value) -> Result<Value, Error> {
    let mut stream = UnixStream::connect(socket).await.map_err(Error::Io)?;
    let mut line = serde_json::to_string(&json!({ "id": 1, "method": method, "params": params }))
        .map_err(|err| Error::Protocol(err.to_string()))?;
    line.push('\n');
    stream.write_all(line.as_bytes()).await.map_err(Error::Io)?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response).await.map_err(Error::Io)?;
    if response.trim().is_empty() {
        return Err(Error::Protocol("empty response".to_owned()));
    }
    let value: Value =
        serde_json::from_str(response.trim()).map_err(|err| Error::Protocol(err.to_string()))?;
    if value.get("error").is_some() {
        return Err(Error::Protocol(value.to_string()));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::open_pane;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn open_pane_pings_then_applies_one_layout() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("herdr.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let replies = [
            "{\"id\":1,\"result\":{\"protocol\":22}}\n",
            "{\"id\":1,\"result\":{\"workspace_id\":\"w1\"}}\n",
            "{\"id\":1,\"result\":{\"type\":\"layout_apply\"}}\n",
        ];
        let server = tokio::spawn(async move {
            for reply in replies {
                let (mut stream, _) = listener.accept().await.expect("accept");
                let mut buf = [0_u8; 2048];
                let read = stream.read(&mut buf).await.expect("read");
                assert!(read > 0);
                stream.write_all(reply.as_bytes()).await.expect("write");
            }
        });
        open_pane(&path, "alpha", &["tmux".to_owned(), "attach".to_owned()])
            .await
            .expect("open");
        server.await.expect("server");
    }
}
