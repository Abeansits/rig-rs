//! One case per `context-pack-routes.test.ts` title.

use serde_json::json;

use crate::context_route_tests::{boot, call, sha256_hex, write_pack};

fn enc(text: &str) -> String {
    text.replace('/', "%2F").replace(':', "%3A")
}

fn by_ref(refer: &str) -> String {
    format!("/api/context-packs/library/by-ref?ref={}", enc(refer))
}

const SMOKE: &str = "\
name: smoke
version: 1
taxonomy: world
purpose: Smoke
files:
  - path: notes.md
    role: notes
";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_library_returns_the_indexed_packs() {
    let harness = boot().await;
    write_pack(
        &harness.dir.join("context"),
        "smoke",
        SMOKE,
        &[("notes.md", "Hello")],
    );
    let (status, body) = call(&harness.sock, "GET", "/api/context-packs/library", None).await;
    assert_eq!(status, 200, "{body}");
    let named = body
        .as_array()
        .expect("rows")
        .iter()
        .filter(|row| row["name"] == "smoke")
        .count();
    assert_eq!(named, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_library_sync_reindexes_and_returns_count() {
    let harness = boot().await;
    write_pack(
        &harness.dir.join("context"),
        "p1",
        "name: p1\nversion: 1\ntaxonomy: world\nfiles:\n  - path: a.md\n    role: r\n",
        &[("a.md", "x")],
    );
    let (status, body) = call(
        &harness.sock,
        "POST",
        "/api/context-packs/library/sync",
        Some(b"{}".to_vec()),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let entries = body["entries"].as_array().expect("entries");
    assert_eq!(body["count"], entries.len());
    assert_eq!(entries.iter().filter(|row| row["name"] == "p1").count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_library_compose_writes_an_ordered_durable_ref() {
    let harness = boot().await;
    let a = harness.dir.join("a.md");
    let b = harness.dir.join("b.md");
    std::fs::write(&a, "A\n").expect("a");
    std::fs::write(&b, "B").expect("b");
    let payload = json!({
        "outRef": "packs/qitem-brief",
        "sources": [
            {"path": a, "label": "a.md"},
            {"path": b, "label": "b.md"},
        ],
    });
    let (status, body) = call(
        &harness.sock,
        "POST",
        "/api/context-packs/library/compose",
        Some(serde_json::to_vec(&payload).expect("json")),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["ref"], "packs/qitem-brief");
    assert_eq!(body["entry"]["relativePath"], "packs/qitem-brief");
    assert_eq!(body["text"], "A\n\n\nB");
    assert_eq!(
        sha256_hex(body["text"].as_str().expect("text")),
        "8b79c77a6d657380a5c5c2cde52b9510dc14a20246772119e7112b3ff45daa43"
    );
    let text = body["text"].as_str().expect("text");
    assert!(!text.contains("# OpenRig Context Pack:"));
    assert!(!text.contains("## File:"));
    let (shown, entry) = call(&harness.sock, "GET", &by_ref("packs/qitem-brief"), None).await;
    assert_eq!(shown, 200, "{entry}");
    assert_eq!(entry["relativePath"], "packs/qitem-brief");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_library_compose_returns_unsafe_ref() {
    let harness = boot().await;
    let a = harness.dir.join("a.md");
    std::fs::write(&a, "A").expect("a");
    let payload = json!({
        "outRef": "../escape",
        "sources": [{"path": a, "label": "a.md"}],
    });
    let (status, body) = call(
        &harness.sock,
        "POST",
        "/api/context-packs/library/compose",
        Some(serde_json::to_vec(&payload).expect("json")),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "unsafe_ref");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("unsafe pack ref")
    );
    assert!(!harness.dir.join("escape").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_library_compose_returns_the_exact_missing_files_envelope() {
    let harness = boot().await;
    let missing = harness.dir.join("absent.md");
    let shown = missing.display().to_string();
    let payload = json!({
        "outRef": "packs/missing",
        "sources": [{"path": shown, "label": "absent.md"}],
    });
    let (status, body) = call(
        &harness.sock,
        "POST",
        "/api/context-packs/library/compose",
        Some(serde_json::to_vec(&payload).expect("json")),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body,
        json!({
            "error": "missing_files",
            "message": format!("context composition source file(s) not found: {shown}"),
            "missingFiles": [shown],
        })
    );
    assert!(
        !harness
            .dir
            .join("context")
            .join("packs")
            .join("missing")
            .exists()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_library_compose_returns_missing_files_for_an_empty_source_list() {
    let harness = boot().await;
    let payload = json!({"outRef": "packs/empty", "sources": []});
    let (status, body) = call(
        &harness.sock,
        "POST",
        "/api/context-packs/library/compose",
        Some(serde_json::to_vec(&payload).expect("json")),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body,
        json!({
            "error": "missing_files",
            "message": "context composition requires at least one --from file",
            "missingFiles": [],
        })
    );
    assert!(
        !harness
            .dir
            .join("context")
            .join("packs")
            .join("empty")
            .exists()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_library_compose_rejects_malformed_source_member() {
    let members = [json!(null), json!(7), json!([])];
    for member in members {
        let harness = boot().await;
        let payload = json!({"outRef": "packs/malformed", "sources": [member]});
        let (status, body) = call(
            &harness.sock,
            "POST",
            "/api/context-packs/library/compose",
            Some(serde_json::to_vec(&payload).expect("json")),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(
            body,
            json!({
                "error": "invalid_compose_request",
                "message": "body must include { outRef, sources: [{ path, label }, ...] }",
            })
        );
        assert!(
            !harness
                .dir
                .join("context")
                .join("packs")
                .join("malformed")
                .exists()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn colon_id_routes_are_removed() {
    let harness = boot().await;
    write_pack(
        &harness.dir.join("context"),
        "p1",
        "name: p1\nversion: 1\ntaxonomy: world\nfiles:\n  - path: a.md\n    role: r\n",
        &[("a.md", "content")],
    );
    let cid = enc("context-pack:p1:1");
    let (show, _) = call(
        &harness.sock,
        "GET",
        &format!("/api/context-packs/library/{cid}"),
        None,
    )
    .await;
    let (preview, _) = call(
        &harness.sock,
        "GET",
        &format!("/api/context-packs/library/{cid}/preview"),
        None,
    )
    .await;
    let (send, _) = call(
        &harness.sock,
        "POST",
        &format!("/api/context-packs/library/{cid}/send"),
        Some(br#"{"destinationSession":"x@rig"}"#.to_vec()),
    )
    .await;
    assert_eq!((show, preview, send), (404, 404, 404));
    let (status, _) = call(&harness.sock, "GET", &by_ref("p1"), None).await;
    assert_eq!(status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_returns_the_assembled_bundle() {
    let harness = boot().await;
    write_pack(
        &harness.dir.join("context"),
        "preview-pack",
        "name: preview-pack\nversion: 1\ntaxonomy: world\npurpose: Preview test\nfiles:\n  - path: a.md\n    role: r\n",
        &[("a.md", "BODY-A")],
    );
    let (status, body) = call(
        &harness.sock,
        "GET",
        &format!(
            "/api/context-packs/library/by-ref/preview?ref={}",
            enc("preview-pack")
        ),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], "context-pack:preview-pack");
    let text = body["bundleText"].as_str().expect("bundle");
    assert!(text.contains("# OpenRig Context Pack: preview-pack v1"));
    assert!(text.contains("BODY-A"));
    assert!(body["bundleBytes"].as_u64().unwrap_or(0) > 0);
    assert_eq!(body["missingFiles"], json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_statuses_for_absent_unsafe_and_missing() {
    let harness = boot().await;
    let absent = call(
        &harness.sock,
        "GET",
        &format!(
            "/api/context-packs/library/by-ref/preview?ref={}",
            enc("packs/absent")
        ),
        None,
    )
    .await
    .0;
    let unsafe_ref = call(
        &harness.sock,
        "GET",
        &format!(
            "/api/context-packs/library/by-ref/preview?ref={}",
            enc("../escape")
        ),
        None,
    )
    .await
    .0;
    let missing = call(
        &harness.sock,
        "GET",
        "/api/context-packs/library/by-ref/preview",
        None,
    )
    .await
    .0;
    assert_eq!((absent, unsafe_ref, missing), (404, 400, 400));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn is_delivery_free() {
    let harness = boot().await;
    let (status, _) = call(
        &harness.sock,
        "POST",
        &format!("/api/context-packs/library/by-ref/send?ref={}", enc("dry")),
        Some(br#"{"destinationSession":"test@rig"}"#.to_vec()),
    )
    .await;
    assert_eq!(status, 404);
    let source = include_str!("context_http.rs");
    assert!(!source.contains("SessionTransport"));
    assert!(
        !source
            .lines()
            .any(|line| line.contains("router.post") && line.contains("/send"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn by_ref_resolves_a_path_like_ref() {
    let harness = boot().await;
    write_pack(
        &harness.dir.join("context"),
        "packs/smoke",
        "name: smoke\nversion: 1\ntaxonomy: world\npurpose: Ref pack\nfiles:\n  - path: notes.md\n    role: notes\n",
        &[("notes.md", "Hello")],
    );
    let (status, body) = call(&harness.sock, "GET", &by_ref("packs/smoke"), None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["name"], "smoke");
    assert_eq!(body["relativePath"], "packs/smoke");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn by_ref_returns_unsafe_ref() {
    let harness = boot().await;
    let (status, body) = call(&harness.sock, "GET", &by_ref("../escape"), None).await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "unsafe_ref");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("unsafe pack ref")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn by_ref_returns_404_for_a_safe_absent_ref() {
    let harness = boot().await;
    let (status, _) = call(&harness.sock, "GET", &by_ref("packs/absent"), None).await;
    assert_eq!(status, 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn by_ref_returns_400_when_the_ref_query_is_missing() {
    let harness = boot().await;
    let (status, body) = call(
        &harness.sock,
        "GET",
        "/api/context-packs/library/by-ref",
        None,
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "ref_required");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_removes_a_pack() {
    let harness = boot().await;
    write_pack(
        &harness.dir.join("context"),
        "packs/smoke",
        "name: smoke\nversion: 1\ntaxonomy: world\nfiles:\n  - path: notes.md\n    role: notes\n",
        &[("notes.md", "Hello")],
    );
    let (status, body) = call(&harness.sock, "DELETE", &by_ref("packs/smoke"), None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["removed"], true);
    assert_eq!(body["ref"], "packs/smoke");
    let (gone, _) = call(&harness.sock, "GET", &by_ref("packs/smoke"), None).await;
    assert_eq!(gone, 404);
    assert!(
        !harness
            .dir
            .join("context")
            .join("packs")
            .join("smoke")
            .exists()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_returns_unsafe_ref_with_no_mutation() {
    let harness = boot().await;
    write_pack(
        &harness.dir.join("context"),
        "packs/keep",
        "name: keep\nversion: 1\ntaxonomy: world\nfiles:\n  - path: notes.md\n    role: notes\n",
        &[("notes.md", "x")],
    );
    let (status, body) = call(&harness.sock, "DELETE", &by_ref("../escape"), None).await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "unsafe_ref");
    let (kept, entry) = call(&harness.sock, "GET", &by_ref("packs/keep"), None).await;
    assert_eq!(kept, 200, "{entry}");
    assert!(
        harness
            .dir
            .join("context")
            .join("packs")
            .join("keep")
            .exists()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_returns_404_pack_not_found() {
    let harness = boot().await;
    let (status, body) = call(&harness.sock, "DELETE", &by_ref("packs/absent"), None).await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], "pack_not_found");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_returns_403_for_a_builtin_pack() {
    let harness = boot().await;
    let (listed, rows) = call(&harness.sock, "GET", "/api/context-packs/library", None).await;
    assert_eq!(listed, 200, "{rows}");
    let builtin = rows
        .as_array()
        .expect("rows")
        .iter()
        .find(|row| row["sourceType"] == "builtin")
        .expect("builtin");
    let refer = builtin["relativePath"].as_str().expect("ref");
    let path = builtin["sourcePath"].as_str().expect("path");
    let (status, body) = call(&harness.sock, "DELETE", &by_ref(refer), None).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error"], "pack_not_removable");
    assert!(std::path::Path::new(path).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pieces_returns_ordered_member_contents() {
    let harness = boot().await;
    write_pack(
        &harness.dir.join("context"),
        "packs/walkme",
        "name: walkme\nversion: 1\ntaxonomy: world\nfiles:\n  - path: intro.md\n    role: intro\n  - path: gone.md\n    role: proof\n  - path: steps.md\n    role: steps\n",
        &[("intro.md", "INTRO-BODY"), ("steps.md", "STEPS-BODY")],
    );
    let (status, body) = call(
        &harness.sock,
        "GET",
        &format!(
            "/api/context-packs/library/by-ref/pieces?ref={}",
            enc("packs/walkme")
        ),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ref"], "packs/walkme");
    let pieces = body["pieces"].as_array().expect("pieces");
    let paths: Vec<&str> = pieces
        .iter()
        .map(|piece| piece["path"].as_str().unwrap_or(""))
        .collect();
    let contents: Vec<&str> = pieces
        .iter()
        .map(|piece| piece["content"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(paths, ["intro.md", "steps.md"]);
    assert_eq!(contents, ["INTRO-BODY", "STEPS-BODY"]);
    let missing: Vec<&str> = body["missingFiles"]
        .as_array()
        .expect("missing")
        .iter()
        .map(|row| row["path"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(missing, ["gone.md"]);
    assert_eq!(body["text"], "INTRO-BODY\n\nSTEPS-BODY");
    assert_eq!(body["bytes"], 22);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pieces_statuses_for_absent_unsafe_and_missing() {
    let harness = boot().await;
    let absent = call(
        &harness.sock,
        "GET",
        &format!(
            "/api/context-packs/library/by-ref/pieces?ref={}",
            enc("packs/absent")
        ),
        None,
    )
    .await
    .0;
    let unsafe_ref = call(
        &harness.sock,
        "GET",
        &format!(
            "/api/context-packs/library/by-ref/pieces?ref={}",
            enc("../escape")
        ),
        None,
    )
    .await
    .0;
    let missing = call(
        &harness.sock,
        "GET",
        "/api/context-packs/library/by-ref/pieces",
        None,
    )
    .await
    .0;
    assert_eq!((absent, unsafe_ref, missing), (404, 400, 400));
}
