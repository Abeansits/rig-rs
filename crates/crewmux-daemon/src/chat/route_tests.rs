//! Chat routes over a private Unix socket.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crewmux_api::Client;
use crewmux_store::seat_token;
use crewmux_types::SeatId;
use serde_json::Value;

use super::route_support::{
    boot, enc, exchange, history_path, json, rig, seat_headers, seated, seed, stamp,
};

struct Reader {
    stop: Arc<AtomicBool>,
    rx: std::sync::mpsc::Receiver<String>,
}

impl Reader {
    fn open(sock: PathBuf, path: String, headers: Vec<(String, String)>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        tokio::task::spawn_blocking(move || {
            let client = Client::over_uds(sock);
            let view: Vec<(&str, &str)> = headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect();
            let mut stream = None;
            let mut last = String::new();
            for _ in 0..8 {
                match client.open("GET", &path, &view) {
                    Ok(opened) => {
                        stream = Some(opened);
                        break;
                    }
                    Err(err) => last = err.to_string(),
                }
            }
            let mut stream = stream.unwrap_or_else(|| panic!("open: {last}"));
            assert_eq!(stream.status, 200, "{}", stream.content_type);
            assert!(stream.content_type.contains("text/event-stream"));
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .expect("timeout");
            let mut raw = Vec::new();
            let mut buf = [0_u8; 2048];
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(6) && !flag.load(Ordering::Relaxed) {
                match std::io::Read::read(&mut stream, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Some(chunk) = buf.get(..n) {
                            raw.extend_from_slice(chunk);
                        }
                        drop(tx.send(String::from_utf8_lossy(&raw).into_owned()));
                    }
                    Err(err)
                        if matches!(
                            err.kind(),
                            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                        ) => {}
                    Err(err) => panic!("watch read: {err}"),
                }
            }
        });
        Self { stop, rx }
    }

    async fn until(&self, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut last = String::new();
        while Instant::now() < deadline {
            while let Ok(text) = self.rx.try_recv() {
                last = text;
            }
            if last.contains(needle) {
                return last;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        last
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_returns_same_day_rows_after_iso_cutoff() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    let id = seed(&harness.app, rig, "alice", "same day").await;
    stamp(&harness.app, &id, "2026-09-30 12:00:00").await;
    let path = history_path(rig, &format!("since={}", enc("2026-09-30T11:00:00Z")));
    let response = exchange(&harness.sock, "GET", &path, None, Vec::new()).await;
    assert_eq!(response.status, 200);
    let rows = json(&response.body);
    assert_eq!(rows.as_array().expect("array").len(), 1);
    assert_eq!(rows[0]["id"], id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_returns_messages() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    seed(&harness.app, rig, "alice", "msg1").await;
    seed(&harness.app, rig, "bob", "msg2").await;
    let response = exchange(
        &harness.sock,
        "GET",
        &history_path(rig, ""),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 200);
    let rows = json(&response.body);
    assert_eq!(rows[0]["body"], "msg1");
    assert_eq!(rows[1]["body"], "msg2");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_topic_filters() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    seed(&harness.app, rig, "alice", "before topic").await;
    let response = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/topic"),
        Some(br#"{"topic":"deploy"}"#.to_vec()),
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 201);
    seed(&harness.app, rig, "bob", "deploy msg").await;
    let response = exchange(
        &harness.sock,
        "GET",
        &history_path(rig, "topic=deploy"),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 200);
    let text = response.body;
    assert!(text.contains("deploy msg"), "{text}");
    assert!(!text.contains("before topic"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn topic_creates_marker() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    let response = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/topic"),
        Some(br#"{"topic":"standup","body":"daily"}"#.to_vec()),
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 201);
    let body = json(&response.body);
    assert_eq!(body["kind"], "topic");
    assert_eq!(body["topic"], "standup");
    assert_eq!(body["sender"], "operator");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_sender_filters() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    seed(&harness.app, rig, "alice", "alice msg").await;
    seed(&harness.app, rig, "bob", "bob msg").await;
    let response = exchange(
        &harness.sock,
        "GET",
        &history_path(rig, "sender=alice"),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 200);
    let rows = json(&response.body);
    assert_eq!(rows.as_array().expect("array").len(), 1);
    assert_eq!(rows[0]["sender"], "alice");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_since_filters_by_timestamp() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    seed(&harness.app, rig, "alice", "msg1").await;
    seed(&harness.app, rig, "bob", "msg2").await;
    let future = exchange(
        &harness.sock,
        "GET",
        &history_path(rig, &format!("since={}", enc("2099-01-01T00:00:00Z"))),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(future.status, 200);
    assert_eq!(json(&future.body).as_array().expect("array").len(), 0);
    let past = exchange(
        &harness.sock,
        "GET",
        &history_path(rig, &format!("since={}", enc("2020-01-01T00:00:00Z"))),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(json(&past.body).as_array().expect("array").len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_since_refuses_unparseable() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    seed(&harness.app, rig, "alice", "msg1").await;
    let response = exchange(
        &harness.sock,
        "GET",
        &history_path(rig, &format!("since={}", enc("garbage"))),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 400);
    assert!(
        json(&response.body)["error"]
            .as_str()
            .unwrap_or("")
            .contains("since")
    );
    let spaced = exchange(
        &harness.sock,
        "GET",
        &history_path(rig, &format!("since={}", enc("not a timestamp"))),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(spaced.status, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_since_empty_is_unfiltered_whitespace_refused() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    seed(&harness.app, rig, "alice", "msg1").await;
    seed(&harness.app, rig, "bob", "msg2").await;
    let empty = exchange(
        &harness.sock,
        "GET",
        &history_path(rig, "since="),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(empty.status, 200);
    assert_eq!(json(&empty.body).as_array().expect("array").len(), 2);
    let blank = exchange(
        &harness.sock,
        "GET",
        &history_path(rig, &format!("since={}", enc("   "))),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(blank.status, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_since_escapes_controls() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    let response = exchange(
        &harness.sock,
        "GET",
        &history_path(rig, &format!("since={}", enc("a\nb\x1bc"))),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 400);
    let err = json(&response.body)["error"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(err.contains("a\\x0ab\\x1bc"), "{err}");
    assert!(!err.contains('\n'), "{err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_since_escapes_c1() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    let response = exchange(
        &harness.sock,
        "GET",
        &history_path(rig, &format!("since={}", enc("a\u{009b}béc"))),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 400);
    let err = json(&response.body)["error"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(err.contains("a\\x9bbéc"), "{err}");
    assert!(!err.contains('\u{009b}'));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_since_accepts_sqlite_formats() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    seed(&harness.app, rig, "alice", "msg1").await;
    for since in ["2020-01-01", "2020-01-01 00:00:00"] {
        let response = exchange(
            &harness.sock,
            "GET",
            &history_path(rig, &format!("since={}", enc(since))),
            None,
            Vec::new(),
        )
        .await;
        assert_eq!(response.status, 200, "{since}");
        assert!(
            !json(&response.body).as_array().expect("array").is_empty(),
            "{since}"
        );
    }
    let now = exchange(
        &harness.sock,
        "GET",
        &history_path(rig, "since=now"),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(now.status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_removes_and_counts() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    seed(&harness.app, rig, "alice", "msg1").await;
    seed(&harness.app, rig, "bob", "msg2").await;
    let response = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/clear"),
        Some(b"{}".to_vec()),
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 200);
    let body = json(&response.body);
    assert_eq!(body["ok"], true);
    assert_eq!(body["deleted"], 2);
    let history = exchange(
        &harness.sock,
        "GET",
        &history_path(rig, ""),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(json(&history.body).as_array().expect("array").len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_empty_is_zero() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    let response = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/clear"),
        Some(b"{}".to_vec()),
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 200);
    let body = json(&response.body);
    assert_eq!(body["ok"], true);
    assert_eq!(body["deleted"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_leaves_other_rigs() {
    let harness = boot().await;
    let rig_id = rig(&harness.app, "pair").await;
    let other = rig(&harness.app, "other").await;
    seed(&harness.app, rig_id, "alice", "target msg").await;
    seed(&harness.app, other, "bob", "other msg").await;
    exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig_id}/chat/clear"),
        Some(b"{}".to_vec()),
        Vec::new(),
    )
    .await;
    let target = exchange(
        &harness.sock,
        "GET",
        &history_path(rig_id, ""),
        None,
        Vec::new(),
    )
    .await;
    let kept = exchange(
        &harness.sock,
        "GET",
        &history_path(other, ""),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(json(&target.body).as_array().expect("array").len(), 0);
    assert_eq!(json(&kept.body)[0]["body"], "other msg");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_outside_seat_is_operator() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    let response = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(br#"{"body":"hi"}"#.to_vec()),
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 201);
    assert_eq!(json(&response.body)["sender"], "operator");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_spoofed_sender_is_rejected() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    let response = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(br#"{"sender":"alice","body":"hi"}"#.to_vec()),
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(json(&response.body)["error"], "unknown sender");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_wrong_token_is_rejected() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    let (seat, _) = seated(&harness.app, rig).await;
    let response = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(br#"{"body":"hi"}"#.to_vec()),
        seat_headers(&seat, "nope"),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(json(&response.body)["error"], "unknown sender");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_matching_token_is_the_seat() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    let (seat, token) = seated(&harness.app, rig).await;
    let response = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(br#"{"body":"hi"}"#.to_vec()),
        seat_headers(&seat, &token),
    )
    .await;
    assert_eq!(response.status, 201);
    assert_eq!(json(&response.body)["sender"], seat.to_string());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_token_mismatch_claim_is_rejected() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    let (seat, token) = seated(&harness.app, rig).await;
    let response = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(br#"{"sender":"alice","body":"hi"}"#.to_vec()),
        seat_headers(&seat, &token),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(json(&response.body)["error"], "unknown sender");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_body_before_identity() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    let (seat, _) = seated(&harness.app, rig).await;
    let response = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(b"{}".to_vec()),
        seat_headers(&seat, "nope"),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(json(&response.body)["error"], "Missing body");
    let garbage = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(b"not-json".to_vec()),
        Vec::new(),
    )
    .await;
    assert_eq!(garbage.status, 400);
    assert_eq!(json(&garbage.body)["error"], "Missing body");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_delivers_initial_batch() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    seed(&harness.app, rig, "alice", "msg1").await;
    seed(&harness.app, rig, "bob", "msg2").await;
    let reader = Reader::open(
        harness.sock.clone(),
        format!("/api/rigs/{rig}/chat/watch"),
        Vec::new(),
    );
    let text = reader.until("msg2").await;
    assert!(
        text.contains("text/event-stream") || text.contains("msg1"),
        "{text}"
    );
    let msg1 = text.find("msg1").expect("msg1");
    let msg2 = text.find("msg2").expect("msg2");
    assert!(msg1 < msg2, "{text}");
    assert!(text.contains("chat.message"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_streams_new_messages_in_order() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    let reader = Reader::open(
        harness.sock.clone(),
        format!("/api/rigs/{rig}/chat/watch"),
        Vec::new(),
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    let response = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(br#"{"body":"live-one"}"#.to_vec()),
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 201, "{}", response.body);
    exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(br#"{"body":"live-two"}"#.to_vec()),
        Vec::new(),
    )
    .await;
    let text = reader.until("live-two").await;
    let first = text.find("live-one").expect(&text);
    let second = text.find("live-two").expect(&text);
    assert!(first < second, "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_resumes_from_last_event_id() {
    let harness = boot().await;
    let rig = rig(&harness.app, "pair").await;
    let first = seed(&harness.app, rig, "alice", "old").await;
    seed(&harness.app, rig, "bob", "kept").await;
    seed(&harness.app, rig, "alice", "later").await;
    let reader = Reader::open(
        harness.sock.clone(),
        format!("/api/rigs/{rig}/chat/watch"),
        vec![("Last-Event-ID".to_owned(), first)],
    );
    let text = reader.until("later").await;
    assert!(!text.contains("\"body\":\"old\""), "{text}");
    let kept = text.find("kept").expect(&text);
    let later = text.find("later").expect(&text);
    assert!(kept < later, "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seat_token_is_stored_at_launch() {
    let harness = boot().await;
    let body = crate::launch::bring_up(
        &harness.app,
        "name: pair\nseats:\n  - name: one\n    runtime: stub\n    cwd: /tmp\n",
    )
    .await
    .expect("up");
    assert_eq!(body.get("status").and_then(Value::as_str), Some("ready"));
    let calls = harness.mux.recorded().await;
    let flat: Vec<&str> = calls.iter().flatten().map(String::as_str).collect();
    let token = flat
        .iter()
        .find_map(|arg| arg.strip_prefix("RIG_SEAT_TOKEN="))
        .expect("token env")
        .to_owned();
    let id = flat
        .iter()
        .find_map(|arg| arg.strip_prefix("RIG_SEAT_ID="))
        .expect("seat env")
        .to_owned();
    assert_ne!(token, id);
    let seat = SeatId::from_str(&id).expect("seat id");
    let store = Arc::clone(&harness.app.store);
    let stored = tokio::task::spawn_blocking(move || {
        store.read(move |conn| seat_token(conn, &seat).map_err(crewmux_store::Error::from))
    })
    .await
    .expect("join")
    .expect("read")
    .expect("token");
    assert_eq!(stored, token);
}
