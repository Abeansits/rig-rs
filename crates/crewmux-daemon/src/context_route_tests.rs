//! Context-pack HTTP routes against a temp library.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_api::Client;
use serde_json::{Value, json};

use crate::flow::harness_with;
use crate::state::App;

pub(crate) struct Harness {
    pub(crate) app: App,
    pub(crate) sock: PathBuf,
    pub(crate) dir: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.app.cancel.cancel();
        drop(std::fs::remove_file(&self.sock));
        drop(std::fs::remove_dir_all(&self.dir));
    }
}

pub(crate) async fn boot() -> Harness {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rig-ctx-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let (app, _mux) = harness_with(None, &dir, None).await;
    let sock = dir.join("d.sock");
    let listener = crate::runtime::bind_socket(&sock).expect("bind");
    let served = app.clone();
    let cancel = app.cancel.clone();
    tokio::spawn(async move {
        drop(
            axum::serve(listener, crate::http::router(served))
                .with_graceful_shutdown(async move { cancel.cancelled().await })
                .await,
        );
    });
    Harness { app, sock, dir }
}

pub(crate) async fn call(
    sock: &Path,
    method: &str,
    path: &str,
    body: Option<Vec<u8>>,
) -> (u16, Value) {
    let sock = sock.to_path_buf();
    let method = method.to_owned();
    let path = path.to_owned();
    let response = tokio::task::spawn_blocking(move || {
        Client::over_uds(sock)
            .request(&method, &path, body.as_deref())
            .expect("http")
    })
    .await
    .expect("join");
    let value = serde_json::from_str(&response.body).unwrap_or(Value::Null);
    (response.status, value)
}

pub(crate) fn write_pack(root: &Path, refer: &str, manifest: &str, files: &[(&str, &str)]) {
    let dir = root.join(refer);
    std::fs::create_dir_all(&dir).expect("pack");
    std::fs::write(dir.join("manifest.yaml"), manifest).expect("manifest");
    for (name, body) in files {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(path, body).expect("file");
    }
}

pub(crate) const WORLD: &str = "
name: world-install
version: \"1\"
taxonomy: world
files:
  - { path: walk.md, role: world }
atoms:
  - id: welcome
    address: \"walk.md#welcome\"
    taxonomy: world
    situations: [fresh]
    purpose: depth
    order: 1
    priority: core
  - id: recap
    address: \"seat:RECAP.md#recent-decisions\"
    taxonomy: lore
    situations: [handover, post-compaction]
    purpose: width
    order: 9
    priority: core
";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn library_returns_503_when_unwired() {
    let harness = boot().await;
    harness.app.context.set_available(false);
    let (status, body) = call(&harness.sock, "GET", "/api/context-packs/library", None).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"], "context_pack_library_unavailable");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workspace_pack_appears_after_sync_once() {
    let harness = boot().await;
    let workspace = harness.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let packs = workspace.join(".rig-rs").join("context-packs");
    std::os::unix::fs::symlink(".rig-rs", workspace.join(".openrig")).expect("link");
    harness.app.context.set_workspace(Some(workspace.clone()));
    let (missing, _) = call(
        &harness.sock,
        "GET",
        "/api/context-packs/library/by-ref?ref=late-workspace-pack",
        None,
    )
    .await;
    assert_eq!(missing, 404);
    write_pack(
        &packs,
        "late-workspace-pack",
        "name: late-workspace-pack\nversion: \"1\"\ntaxonomy: world\npurpose: Late\nfiles:\n  - path: world.md\n    role: world\n",
        &[("world.md", "# Late workspace pack\n")],
    );
    let (synced, report) = call(
        &harness.sock,
        "POST",
        "/api/context-packs/library/sync",
        Some(b"{}".to_vec()),
    )
    .await;
    assert_eq!(synced, 200, "{report}");
    let (status, entry) = call(
        &harness.sock,
        "GET",
        "/api/context-packs/library/by-ref?ref=late-workspace-pack",
        None,
    )
    .await;
    assert_eq!(status, 200, "{entry}");
    assert_eq!(entry["sourceType"], "workspace");
    assert_eq!(entry["name"], "late-workspace-pack");
    let (listed, rows) = call(&harness.sock, "GET", "/api/context-packs/library", None).await;
    assert_eq!(listed, 200, "{rows}");
    let hits = rows
        .as_array()
        .expect("array")
        .iter()
        .filter(|row| row["relativePath"] == "late-workspace-pack")
        .count();
    assert_eq!(hits, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_handover_reports_budget_and_hash() {
    let harness = boot().await;
    let topology = harness.dir.join("topology");
    let seat = topology.join("rigs").join("r1").join("seats").join("s1");
    std::fs::create_dir_all(&seat).expect("seat");
    std::fs::write(
        seat.join("RECAP.md"),
        "## Recent Decisions\nwe chose X because Y",
    )
    .expect("recap");
    harness
        .app
        .context
        .set_grants(topology, harness.dir.join("missions"));
    write_pack(
        &harness.dir.join("context"),
        "packs/world",
        WORLD,
        &[("walk.md", "## Welcome\nhello world")],
    );
    let fresh = profile(&harness, "situation=fresh&runtime=claude").await;
    assert_eq!(fresh.0, 200, "{}", fresh.1);
    assert_eq!(ids(&fresh.1), vec!["welcome".to_owned()]);
    let bare = profile(&harness, "situation=handover&runtime=claude").await;
    assert_eq!(bare.0, 422, "{}", bare.1);
    assert!(
        bare.1["message"]
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .contains("seat")
    );
    let full = profile(
        &harness,
        "situation=handover&runtime=claude&rig=r1&seat=s1&budget=1",
    )
    .await;
    assert_eq!(full.0, 200, "{}", full.1);
    assert_eq!(ids(&full.1), vec!["welcome".to_owned(), "recap".to_owned()]);
    let pieces = full.1["pieces"].as_array().expect("pieces");
    assert_eq!(pieces.len(), 2);
    let recap = pieces
        .iter()
        .find(|piece| piece["atomId"] == "recap")
        .expect("recap");
    assert_eq!(recap["sourceKind"], "seat");
    assert!(
        recap["text"]
            .as_str()
            .unwrap_or("")
            .contains("we chose X because Y")
    );
    for piece in pieces {
        let text = piece["text"].as_str().expect("text");
        assert_eq!(piece["sha256"], sha256_hex(text));
    }
    assert!(full.1["budget"]["overageTokens"].as_u64().unwrap_or(0) > 0);
}

async fn profile(harness: &Harness, query: &str) -> (u16, Value) {
    call(
        &harness.sock,
        "GET",
        &format!("/api/context-packs/library/by-ref/profile?ref=packs/world&{query}"),
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

pub(crate) fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::new();
    for byte in digest {
        out.push(char::from(HEX[usize::from(byte / 16)]));
        out.push(char::from(HEX[usize::from(byte % 16)]));
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolve_address_serves_the_header_span() {
    let harness = boot().await;
    write_pack(
        &harness.dir.join("context"),
        "packs/span",
        "name: span\nversion: \"1\"\ntaxonomy: world\nfiles:\n  - path: walk.md\n    role: world\n",
        &[("walk.md", "## Welcome\nhello address\n")],
    );
    let (status, body) = call(
        &harness.sock,
        "GET",
        "/api/context-packs/library/resolve-address?address=packs/span/walk.md%23welcome",
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["text"]
            .as_str()
            .unwrap_or("")
            .contains("hello address")
    );
    assert_eq!(body["packRef"], "packs/span");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recap_write_lands_under_the_topology_rig() {
    let harness = boot().await;
    let topology = harness.dir.join("topology");
    std::fs::create_dir_all(topology.join("rigs").join("r1")).expect("rig");
    harness
        .app
        .context
        .set_grants(topology.clone(), harness.dir.join("missions"));
    let body = serde_json::to_vec(&json!({
        "rig": "r1",
        "seat": "s1",
        "content": "conclusions only\n",
    }))
    .expect("json");
    let (status, value) = call(
        &harness.sock,
        "POST",
        "/api/context-packs/recap",
        Some(body),
    )
    .await;
    assert_eq!(status, 200, "{value}");
    let path = topology
        .join("rigs")
        .join("r1")
        .join("seats")
        .join("s1")
        .join("RECAP.md");
    assert_eq!(
        std::fs::read_to_string(&path).expect("recap"),
        "conclusions only\n"
    );
    assert!(
        value["findings"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["kind"] == "no-decisions-section"))
    );
}
