//! Body cap, seat membership, and `Last-Event-ID` checks.

use crewmux_api::MAX_CHAT_BODY;

use super::route_support::{enc, exchange, json, rig, seat_headers, seated};
use super::route_tests::Reader;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_body_is_413_and_a_small_one_is_stored() {
    let harness = super::route_support::boot().await;
    let rig = rig(&harness.app, "pair").await;
    let big = oversized_body();
    let response = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(big.into_bytes()),
        Vec::new(),
    )
    .await;
    assert_eq!(response.status, 413, "{}", response.body);
    assert_eq!(json(&response.body)["error"], "message is too large");
    let small = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(br#"{"body":"kept"}"#.to_vec()),
        Vec::new(),
    )
    .await;
    assert_eq!(small.status, 201, "{}", small.body);
    let history = exchange(
        &harness.sock,
        "GET",
        &format!("/api/rigs/{rig}/chat/history"),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(history.status, 200);
    let rows = json(&history.body);
    let list = rows.as_array().expect("array");
    assert_eq!(list.len(), 1);
    assert_eq!(
        list.first().and_then(|row| row.get("body")),
        Some(&serde_json::json!("kept"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_prints_a_small_message_after_an_oversized_send() {
    let harness = super::route_support::boot().await;
    let rig = rig(&harness.app, "pair").await;
    let reader = Reader::open(
        harness.sock.clone(),
        format!("/api/rigs/{rig}/chat/watch"),
        Vec::new(),
    );
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let big = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(oversized_body().into_bytes()),
        Vec::new(),
    )
    .await;
    assert_eq!(big.status, 413, "{}", big.body);
    exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(br#"{"body":"after-large"}"#.to_vec()),
        Vec::new(),
    )
    .await;
    let text = reader.until("after-large").await;
    assert!(text.contains("after-large"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seat_cannot_post_into_another_rig() {
    let harness = super::route_support::boot().await;
    let home = rig(&harness.app, "home").await;
    let other = rig(&harness.app, "other").await;
    let (seat, token) = seated(&harness.app, home).await;
    let rejected = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{other}/chat/send"),
        Some(br#"{"body":"nope"}"#.to_vec()),
        seat_headers(&seat, &token),
    )
    .await;
    assert_eq!(rejected.status, 403, "{}", rejected.body);
    assert_eq!(json(&rejected.body)["error"], "seat is not in this rig");
    let allowed = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{home}/chat/send"),
        Some(br#"{"body":"yes"}"#.to_vec()),
        seat_headers(&seat, &token),
    )
    .await;
    assert_eq!(allowed.status, 201, "{}", allowed.body);
    assert_eq!(json(&allowed.body)["sender"], "one");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn garbage_last_event_id_is_400() {
    let harness = super::route_support::boot().await;
    let rig = rig(&harness.app, "pair").await;
    let response = exchange(
        &harness.sock,
        "GET",
        &format!("/api/rigs/{rig}/chat/watch"),
        None,
        vec![("Last-Event-ID".to_owned(), "garbage".to_owned())],
    )
    .await;
    assert_eq!(response.status, 400, "{}", response.body);
    assert_eq!(json(&response.body)["error"], "invalid cursor");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn topic_body_is_capped_at_max_chat_body() {
    let harness = super::route_support::boot().await;
    let rig = rig(&harness.app, "pair").await;
    let path = format!("/api/rigs/{rig}/chat/topic");
    let over = topic_json("plan", &"b".repeat(MAX_CHAT_BODY.saturating_add(1)));
    let rejected = exchange(
        &harness.sock,
        "POST",
        &path,
        Some(over.into_bytes()),
        Vec::new(),
    )
    .await;
    assert_eq!(rejected.status, 413, "{}", rejected.body);
    assert_eq!(json(&rejected.body)["error"], "message is too large");
    let at_cap = topic_json("plan", &"c".repeat(MAX_CHAT_BODY));
    let stored = exchange(
        &harness.sock,
        "POST",
        &path,
        Some(at_cap.into_bytes()),
        Vec::new(),
    )
    .await;
    assert_eq!(stored.status, 201, "len {}", stored.body.len());
    let empty = topic_json("other", "");
    let accepted = exchange(
        &harness.sock,
        "POST",
        &path,
        Some(empty.into_bytes()),
        Vec::new(),
    )
    .await;
    assert_eq!(accepted.status, 201, "{}", accepted.body);
    let history = exchange(
        &harness.sock,
        "GET",
        &format!("/api/rigs/{rig}/chat/history"),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(history.status, 200);
    let rows = json(&history.body);
    let list = rows.as_array().expect("array");
    assert_eq!(list.len(), 2);
    let capped = list
        .iter()
        .find(|row| row.get("topic").and_then(|v| v.as_str()) == Some("plan"));
    let body = capped
        .and_then(|row| row.get("body"))
        .and_then(|v| v.as_str());
    assert_eq!(body.map(str::len), Some(MAX_CHAT_BODY));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn topic_longer_than_256_bytes_is_400() {
    let harness = super::route_support::boot().await;
    let rig = rig(&harness.app, "pair").await;
    let name = "\"".repeat(257);
    let topic = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/topic"),
        Some(topic_json(&name, "hi").into_bytes()),
        Vec::new(),
    )
    .await;
    assert_eq!(topic.status, 400, "{}", topic.body);
    assert_eq!(json(&topic.body)["error"], "invalid topic");
    let history = exchange(
        &harness.sock,
        "GET",
        &format!("/api/rigs/{rig}/chat/history?topic={}", enc(&name)),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(history.status, 400, "{}", history.body);
    assert_eq!(json(&history.body)["error"], "invalid topic");
    let send = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(
            serde_json::json!({"body": "hi", "topic": name})
                .to_string()
                .into_bytes(),
        ),
        Vec::new(),
    )
    .await;
    assert_eq!(send.status, 400, "{}", send.body);
    assert_eq!(json(&send.body)["error"], "invalid topic");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overlong_sender_cursor_and_since_are_400() {
    let harness = super::route_support::boot().await;
    let rig = rig(&harness.app, "pair").await;
    let long = "a".repeat(257);
    let send = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/send"),
        Some(
            serde_json::json!({"body": "hi", "sender": long})
                .to_string()
                .into_bytes(),
        ),
        Vec::new(),
    )
    .await;
    assert_eq!(send.status, 400, "{}", send.body);
    assert_eq!(json(&send.body)["error"], "invalid sender");
    let history = format!("/api/rigs/{rig}/chat/history");
    expect_400(
        &harness.sock,
        &format!("{history}?sender={}", enc(&long)),
        "invalid sender",
    )
    .await;
    expect_400(
        &harness.sock,
        &format!("{history}?after={}", enc(&long)),
        "invalid cursor",
    )
    .await;
    let since = "c".repeat(65);
    expect_400(
        &harness.sock,
        &format!("{history}?since={}", enc(&since)),
        "invalid since",
    )
    .await;
    let topic = exchange(
        &harness.sock,
        "POST",
        &format!("/api/rigs/{rig}/chat/topic"),
        Some(
            serde_json::json!({"topic": "plan", "sender": long})
                .to_string()
                .into_bytes(),
        ),
        Vec::new(),
    )
    .await;
    assert_eq!(topic.status, 400, "{}", topic.body);
    assert_eq!(json(&topic.body)["error"], "invalid sender");
}

async fn expect_400(sock: &std::path::Path, path: &str, error: &str) {
    let response = exchange(sock, "GET", path, None, Vec::new()).await;
    assert_eq!(response.status, 400, "{}", response.body);
    assert_eq!(json(&response.body)["error"], error);
    assert!(response.body.len() < 200, "len {}", response.body.len());
}

fn topic_json(topic: &str, body: &str) -> String {
    serde_json::json!({"topic": topic, "body": body}).to_string()
}

fn oversized_body() -> String {
    let mut body = String::from("{\"body\":\"");
    body.push_str(&"a".repeat(MAX_CHAT_BODY.saturating_add(1)));
    body.push_str("\"}");
    body
}
