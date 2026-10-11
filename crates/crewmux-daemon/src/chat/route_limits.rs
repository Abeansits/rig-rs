//! Body cap, seat membership, and `Last-Event-ID` checks.

use crewmux_api::MAX_CHAT_BODY;

use super::route_support::{exchange, json, rig, seat_headers, seated};
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

fn oversized_body() -> String {
    let mut body = String::from("{\"body\":\"");
    body.push_str(&"a".repeat(MAX_CHAT_BODY.saturating_add(1)));
    body.push_str("\"}");
    body
}
