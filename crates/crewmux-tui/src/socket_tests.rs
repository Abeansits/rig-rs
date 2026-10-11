//! Control socket: mode 0600, commands through `update`, unknown verbs error.

use std::os::unix::fs::PermissionsExt;
use std::time::Instant;

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio_util::sync::CancellationToken;

use crate::msg::{Cmd, ControlCmd, Msg};
use crate::state::{Size, State};
use crate::update::update;

#[tokio::test]
async fn control_socket_is_private_and_replies() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("tui.sock");
    let cancel = CancellationToken::new();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(4);
    let (reply_tx, reply_rx) = tokio::sync::mpsc::channel(4);
    let bound = path.clone();
    let serve_cancel = cancel.clone();
    let server = tokio::spawn(async move {
        if let Err(err) = crate::control::serve(&bound, serve_cancel, cmd_tx, reply_rx).await {
            panic!("serve: {err}");
        }
    });
    for _ in 0..50 {
        if path.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let mode = tokio::fs::metadata(&path)
        .await
        .expect("meta")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let mut client = UnixStream::connect(&path).await.expect("connect");
    client
        .write_all(b"{\"cmd\":\"nope\",\"id\":3}\n")
        .await
        .expect("write");
    let incoming = cmd_rx.recv().await.expect("command");
    let ControlCmd::Unknown { reply, verb } = incoming.cmd.clone() else {
        panic!("expected unknown");
    };
    assert_eq!(verb, "nope");
    let state = State::new(Size::new(80, 24), Instant::now());
    let (_next, cmds) = update(state, Msg::Control(incoming.cmd));
    let Cmd::Reply(id, value) = cmds.first().expect("reply").clone() else {
        panic!("expected reply cmd");
    };
    assert_eq!(id, reply);
    reply_tx.send((id, value)).await.expect("reply send");
    let mut line = String::new();
    BufReader::new(&mut client)
        .read_line(&mut line)
        .await
        .expect("read");
    let parsed: serde_json::Value = serde_json::from_str(line.trim()).expect("json");
    assert_eq!(parsed.get("ok"), Some(&json!(false)));
    assert_eq!(parsed.get("id"), Some(&json!(3)));
    cancel.cancel();
    drop(client);
    match server.await {
        Ok(()) => {}
        Err(err) => panic!("server task: {err}"),
    }
}

#[tokio::test]
async fn control_socket_rejects_a_long_sun_path() {
    let path = std::path::PathBuf::from(format!("/{}", "a".repeat(120)));
    let cancel = CancellationToken::new();
    let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(1);
    let (_reply_tx, reply_rx) = tokio::sync::mpsc::channel(1);
    let err = crate::control::serve(&path, cancel, cmd_tx, reply_rx)
        .await
        .expect_err("sun path");
    assert!(err.to_string().contains("sun_path"));
}
