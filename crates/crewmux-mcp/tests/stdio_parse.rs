//! A line that is not JSON gets a JSON-RPC parse error, and the session continues.

use crewmux_api::Client;
use crewmux_mcp::{Caller, RigMcp, SchemaMode, serve_io};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

#[tokio::test]
async fn non_json_input_is_parse_error() {
    let server = RigMcp::new(
        Client::over_uds("/tmp/rig-mcp-parse.sock"),
        Caller::Operator,
        SchemaMode::Native,
    );
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server_io);
    let (client_read, mut client_write) = tokio::io::split(client_io);
    let serve = tokio::spawn(async move {
        serve_io(server, server_read, server_write)
            .await
            .expect("serve");
    });
    client_write.write_all(b"not json\n").await.expect("write");
    client_write.flush().await.expect("flush");
    let mut lines = tokio::io::BufReader::new(client_read).lines();
    let line = lines.next_line().await.expect("read").expect("line");
    let parsed: serde_json::Value = serde_json::from_str(&line).expect("json");
    assert_eq!(parsed["error"]["code"], -32700);
    assert_eq!(parsed["id"], serde_json::Value::Null);

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"qa","version":"0"}}}"#;
    client_write
        .write_all(format!("{init}\n").as_bytes())
        .await
        .expect("init");
    client_write.flush().await.expect("flush");
    let reply = lines.next_line().await.expect("read").expect("init reply");
    let init_reply: serde_json::Value = serde_json::from_str(&reply).expect("init json");
    assert_eq!(init_reply["id"], 1, "{reply}");
    assert!(init_reply.get("result").is_some(), "{reply}");

    drop(client_write);
    drop(lines);
    serve.await.expect("join");
}
