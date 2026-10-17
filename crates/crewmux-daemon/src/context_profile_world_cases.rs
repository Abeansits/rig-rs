//! Synthetic world-graph cases from `context-pack-profile-route.test.ts`.

use serde_json::{Value, json};

use crate::context_route_tests::{boot, call, write_pack};

const SENTINEL: &str = "sentinel-recap-7fce914a8: we chose the atoms graph because drift";
const WRITTEN_AT: &str = "2026-08-26T12:00:00.000Z";
const WRITTEN_SECS: i64 = 1_787_745_600;

const MANIFEST: &str = r#"
name: synthetic-world
version: "1"
taxonomy: world
files:
  - { path: world-from-primitives.md, role: world }
  - { path: permission-self-sleep.md, role: world }
  - { path: what-this-is-for.md, role: world }
  - { path: ontology.md, role: world }
  - { path: harness-power-use.md, role: world }
  - { path: a-competent-turn.md, role: world }
  - { path: what-you-can-do.md, role: reference }
  - { path: reference-material.md, role: reference }
atoms:
  - { id: world-from-primitives, address: world-from-primitives.md, taxonomy: world, situations: [fresh], purpose: depth, order: 1, priority: core }
  - { id: permission-self-sleep, address: permission-self-sleep.md, taxonomy: world, situations: [fresh], purpose: depth, order: 2, priority: core }
  - { id: what-this-is-for, address: what-this-is-for.md, taxonomy: world, situations: [fresh], purpose: depth, order: 3, priority: core }
  - { id: ontology, address: ontology.md, taxonomy: world, situations: [fresh, post-compaction], purpose: depth, order: 4, priority: core }
  - { id: harness-power-use, address: harness-power-use.md, taxonomy: world, situations: [fresh], purpose: depth, order: 5, priority: core }
  - { id: what-you-can-do, address: what-you-can-do.md, taxonomy: skills, situations: [post-compaction], purpose: width, order: 6, priority: core }
  - { id: reference-material, address: reference-material.md, taxonomy: lore, situations: [post-compaction], purpose: width, order: 7, priority: core }
  - { id: a-competent-turn, address: a-competent-turn.md, taxonomy: world, situations: [fresh, post-compaction], purpose: depth, order: 8, priority: core }
  - { id: recap, address: "seat:RECAP.md", taxonomy: lore, situations: [handover, post-compaction], purpose: width, order: 9, priority: core }
"#;

const FILES: [&str; 8] = [
    "world-from-primitives.md",
    "permission-self-sleep.md",
    "what-this-is-for.md",
    "ontology.md",
    "harness-power-use.md",
    "a-competent-turn.md",
    "what-you-can-do.md",
    "reference-material.md",
];

fn enc(text: &str) -> String {
    text.replace('/', "%2F")
}

async fn synthetic(with_recap: bool) -> crate::context_route_tests::Harness {
    let harness = boot().await;
    let seat = harness
        .dir
        .join("topology")
        .join("rigs")
        .join("r1")
        .join("seats")
        .join("s1");
    std::fs::create_dir_all(&seat).expect("seat");
    if with_recap {
        std::fs::write(seat.join("RECAP.md"), format!("## Decisions\n{SENTINEL}")).expect("recap");
    }
    harness
        .app
        .context
        .set_grants(harness.dir.join("topology"), harness.dir.join("missions"));
    let bodies: Vec<String> = FILES
        .iter()
        .map(|name| format!("## Synthetic fixture\n{name}\n"))
        .collect();
    let pairs: Vec<(&str, &str)> = FILES
        .iter()
        .copied()
        .zip(bodies.iter().map(String::as_str))
        .collect();
    write_pack(
        &harness.dir.join("context"),
        "world/install",
        MANIFEST,
        &pairs,
    );
    harness
}

async fn profile(harness: &crate::context_route_tests::Harness, query: &str) -> (u16, Value) {
    call(
        &harness.sock,
        "GET",
        &format!(
            "/api/context-packs/library/by-ref/profile?ref={}&{query}",
            enc("world/install")
        ),
        None,
    )
    .await
}

fn ids(body: &Value) -> Vec<String> {
    body["pieces"]
        .as_array()
        .expect("pieces")
        .iter()
        .map(|piece| piece["atomId"].as_str().unwrap_or("").to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handover_is_the_fresh_walk_plus_the_seat_recap() {
    let harness = synthetic(true).await;
    let (status, body) =
        profile(&harness, "situation=handover&runtime=claude&rig=r1&seat=s1").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        ids(&body),
        [
            "world-from-primitives",
            "permission-self-sleep",
            "what-this-is-for",
            "ontology",
            "harness-power-use",
            "a-competent-turn",
            "recap",
        ]
    );
    let recap = body["pieces"]
        .as_array()
        .expect("pieces")
        .iter()
        .find(|piece| piece["atomId"] == "recap")
        .expect("recap");
    assert_eq!(recap["sourceKind"], "seat");
    assert!(
        recap["text"]
            .as_str()
            .unwrap_or("")
            .contains("sentinel-recap-7fce914a8")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_compaction_is_the_reprime_plus_the_seat_recap() {
    let harness = synthetic(true).await;
    let (status, body) = profile(
        &harness,
        "situation=post-compaction&runtime=claude&rig=r1&seat=s1",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        ids(&body),
        [
            "ontology",
            "what-you-can-do",
            "reference-material",
            "a-competent-turn",
            "recap"
        ]
    );
    let recap = body["pieces"]
        .as_array()
        .expect("pieces")
        .iter()
        .find(|piece| piece["atomId"] == "recap")
        .expect("recap");
    assert_eq!(recap["sourceKind"], "seat");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_seat_recap_fails_loud() {
    let harness = synthetic(false).await;
    let (status, body) =
        profile(&harness, "situation=handover&runtime=claude&rig=r1&seat=s1").await;
    assert_eq!(status, 422, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .contains("recap")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_compaction_without_recap_names_the_gap() {
    let harness = synthetic(false).await;
    let (status, body) = profile(
        &harness,
        "situation=post-compaction&runtime=claude&rig=r1&seat=s1",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        ids(&body),
        [
            "ontology",
            "what-you-can-do",
            "reference-material",
            "a-competent-turn"
        ]
    );
    assert_eq!(
        body["warnings"],
        json!(["no seat recap for s1@r1; read your newest restore map"])
    );
    assert_eq!(
        body["skipped"],
        json!([{"atomId": "recap", "address": "seat:RECAP.md"}])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_compaction_fails_for_dangling_missing_dir_or_no_grant() {
    let harness = synthetic(false).await;
    let recap = harness
        .dir
        .join("topology")
        .join("rigs")
        .join("r1")
        .join("seats")
        .join("s1")
        .join("RECAP.md");
    std::os::unix::fs::symlink(harness.dir.join("nowhere.md"), &recap).expect("link");
    let (dangling, dangling_body) = profile(
        &harness,
        "situation=post-compaction&runtime=claude&rig=r1&seat=s1",
    )
    .await;
    assert_eq!(dangling, 422, "{dangling_body}");
    assert!(
        dangling_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("DANGLING")
    );
    let (missing, missing_body) = profile(
        &harness,
        "situation=post-compaction&runtime=claude&rig=r1&seat=s2",
    )
    .await;
    assert_eq!(missing, 422, "{missing_body}");
    assert!(
        missing_body["message"]
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .contains("recap")
    );
    let (ungranted, ungranted_body) =
        profile(&harness, "situation=post-compaction&runtime=claude").await;
    assert_eq!(ungranted, 422, "{ungranted_body}");
    assert!(
        ungranted_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("not configured")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recap_piece_carries_its_write_time() {
    let harness = synthetic(true).await;
    let recap = harness
        .dir
        .join("topology")
        .join("rigs")
        .join("r1")
        .join("seats")
        .join("s1")
        .join("RECAP.md");
    let ts = rustix::fs::Timespec {
        tv_sec: WRITTEN_SECS,
        tv_nsec: 0,
    };
    rustix::fs::utimensat(
        rustix::fs::CWD,
        &recap,
        &rustix::fs::Timestamps {
            last_access: ts,
            last_modification: ts,
        },
        rustix::fs::AtFlags::empty(),
    )
    .expect("mtime");
    for situation in ["post-compaction", "handover"] {
        let (status, body) = profile(
            &harness,
            &format!("situation={situation}&runtime=claude&rig=r1&seat=s1"),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        let stamped: Vec<(&str, &str)> = body["pieces"]
            .as_array()
            .expect("pieces")
            .iter()
            .filter(|piece| piece.get("writtenAt").is_some())
            .map(|piece| {
                (
                    piece["atomId"].as_str().unwrap_or(""),
                    piece["writtenAt"].as_str().unwrap_or(""),
                )
            })
            .collect();
        assert_eq!(stamped, [("recap", WRITTEN_AT)]);
        assert_eq!(body["warnings"], json!([]));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_stays_the_six_piece_walk() {
    let harness = synthetic(true).await;
    let (status, body) = profile(&harness, "situation=fresh&runtime=claude").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        ids(&body),
        [
            "world-from-primitives",
            "permission-self-sleep",
            "what-this-is-for",
            "ontology",
            "harness-power-use",
            "a-competent-turn",
        ]
    );
}
