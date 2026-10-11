//! The temporary fan-out is SSE on `GET /api/events` and nothing else.

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use crewmux_api::SeatId;
use crewmux_core::DomainEvent;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio_util::sync::CancellationToken;

use super::{EventHub, serve};

async fn started() -> Option<(
    tempfile::TempDir,
    std::path::PathBuf,
    EventHub,
    CancellationToken,
    tokio::task::JoinHandle<Result<(), super::Error>>,
)> {
    let dir = tempfile::tempdir().ok()?;
    let path = dir.path().join("daemon.sock");
    let hub = EventHub::new();
    let cancel = CancellationToken::new();
    let bound = path.clone();
    let hub_task = hub.clone();
    let serve_cancel = cancel.clone();
    let server = tokio::spawn(async move { serve(&bound, hub_task, serve_cancel).await });
    for _ in 0..50 {
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Some((dir, path, hub, cancel, server))
}

fn data_json(body: &str) -> serde_json::Value {
    let data = body
        .lines()
        .find_map(|line| line.strip_prefix("data:"))
        .unwrap_or("");
    serde_json::from_str(data.trim()).unwrap_or(serde_json::Value::Null)
}

#[tokio::test]
async fn publishes_a_seat_death_as_sse() {
    let (_dir, path, hub, cancel, server) = started().await.expect("start");
    let mode = tokio::fs::metadata(&path)
        .await
        .expect("meta")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let mut stream = UnixStream::connect(&path).await.expect("connect");
    stream
        .write_all(
            b"GET /api/events HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n",
        )
        .await
        .expect("request");
    let mut buf = Vec::new();
    let mut tmp = [0_u8; 1024];
    let header_at = loop {
        let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut tmp))
            .await
            .expect("timeout")
            .expect("read");
        buf.extend_from_slice(tmp.get(..n).unwrap_or(&[]));
        if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(buf.get(..header_at).unwrap_or(&[]));
    assert!(head.contains("200"));
    assert!(head.to_ascii_lowercase().contains("text/event-stream"));
    hub.publish(&DomainEvent::SeatDied {
        seat: SeatId::parse("seat-1").expect("seat"),
    });
    let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut tmp))
        .await
        .expect("event timeout")
        .expect("event read");
    let body = String::from_utf8_lossy(tmp.get(..n).unwrap_or(&[])).into_owned();
    let value = data_json(&body);
    assert_eq!(
        value.get("type").and_then(|item| item.as_str()),
        Some("seat_died")
    );
    assert!(
        value
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .is_some()
    );
    assert_eq!(
        value
            .get("payload")
            .and_then(|item| item.get("seat"))
            .and_then(|item| item.as_str()),
        Some("seat-1")
    );
    assert!(value.get("seat").is_none());
    assert!(body.lines().any(|line| line.starts_with("id:")));
    cancel.cancel();
    match tokio::time::timeout(Duration::from_secs(2), server).await {
        Ok(Ok(_) | Err(_)) | Err(_) => {}
    }
}

#[tokio::test]
async fn resume_without_a_backlog_emits_a_gap() {
    let (_dir, path, hub, cancel, server) = started().await.expect("start");
    hub.publish(&DomainEvent::SeatDied {
        seat: SeatId::parse("seat-1").expect("seat"),
    });
    let mut stream = UnixStream::connect(&path).await.expect("connect");
    stream
        .write_all(
            b"GET /api/events?last_event_id=1 HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\nLast-Event-ID: 1\r\n\r\n",
        )
        .await
        .expect("request");
    let mut buf = Vec::new();
    let mut tmp = [0_u8; 2048];
    let body = loop {
        let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut tmp))
            .await
            .expect("timeout")
            .expect("read");
        buf.extend_from_slice(tmp.get(..n).unwrap_or(&[]));
        let text = String::from_utf8_lossy(&buf);
        if text.contains("data:") {
            break text.into_owned();
        }
    };
    let value = data_json(&body);
    assert_eq!(
        value.get("type").and_then(|item| item.as_str()),
        Some("gap")
    );
    assert!(value.get("payload").is_none());
    assert!(!body.contains("seat_died"));
    cancel.cancel();
    match tokio::time::timeout(Duration::from_secs(2), server).await {
        Ok(Ok(_) | Err(_)) | Err(_) => {}
    }
}

#[tokio::test]
async fn rejects_a_sun_path_that_does_not_fit() {
    let path = std::path::PathBuf::from(format!("/{}", "b".repeat(120)));
    let err = serve(&path, EventHub::new(), CancellationToken::new())
        .await
        .expect_err("guard");
    assert!(err.to_string().contains("sun_path"));
}
