//! 64 KB guarded paste and startup delivery of a composed pack.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_api::Client;
use crewmux_messaging::obligation_fingerprint;
use crewmux_mux::FakeMux;
use serde_json::{Value, json};

use crate::flow::{harness_with, two_stub};
use crate::launch::bring_up;
use crate::state::App;

struct Harness {
    app: App,
    mux: std::sync::Arc<FakeMux>,
    sock: PathBuf,
    dir: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.app.cancel.cancel();
        drop(std::fs::remove_file(&self.sock));
        drop(std::fs::remove_dir_all(&self.dir));
    }
}

async fn boot() -> Harness {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rig-deliver-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let (app, mux) = harness_with(None, &dir, None).await;
    bring_up(&app, &two_stub(dir.to_str().unwrap_or("/tmp")))
        .await
        .expect("up");
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
    Harness {
        app,
        mux,
        sock,
        dir,
    }
}

async fn post(sock: &Path, body: Value) -> (u16, Value) {
    let sock = sock.to_path_buf();
    let bytes = serde_json::to_vec(&body).expect("json");
    let response = tokio::task::spawn_blocking(move || {
        Client::over_uds(sock)
            .request("POST", "/api/context-packs/deliver", Some(&bytes))
            .expect("http")
    })
    .await
    .expect("join");
    let value = serde_json::from_str(&response.body).unwrap_or(Value::Null);
    (response.status, value)
}

fn bulk(home: &Path) -> String {
    let dir = home.join("context").join("bulk");
    std::fs::create_dir_all(&dir).expect("pack");
    let mut bytes = vec![b'x'; 64 * 1024];
    bytes[..7].copy_from_slice(b"PACK64-");
    std::fs::write(dir.join("body.txt"), &bytes).expect("body");
    std::fs::write(
        dir.join("manifest.yaml"),
        "name: bulk\nversion: \"1\"\ntaxonomy: mission\nfiles:\n  - path: body.txt\n    role: source\n",
    )
    .expect("manifest");
    String::from_utf8(bytes).expect("utf8")
}

fn pastes(calls: &[Vec<String>]) -> usize {
    calls
        .iter()
        .filter(|argv| argv.iter().any(|arg| arg == "paste-buffer"))
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn paste_64k_returns_receipt_and_fingerprint() {
    let harness = boot().await;
    let text = bulk(&harness.dir);
    assert_eq!(text.len(), 64 * 1024);
    let before = pastes(&harness.mux.recorded().await);
    let (status, body) = post(
        &harness.sock,
        json!({"ref": "bulk", "seat": "one", "mode": "paste"}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["mode"], "paste");
    assert_eq!(body["tokensEst"], 16_384);
    let receipt = &body["delivery"]["receipts"][0];
    assert_eq!(receipt["outcome"], "delivered");
    assert_eq!(receipt["seat"], "one@pair");
    let fingerprint = obligation_fingerprint("context", "one@pair", &text);
    assert_eq!(receipt["fingerprint"], fingerprint);
    assert!(pastes(&harness.mux.recorded().await) > before);
    let shot = crate::query::capture(&harness.app, "one", None)
        .await
        .expect("capture");
    let screen = shot["text"].as_str().unwrap_or("");
    assert!(screen.contains("PACK64-"), "{screen}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_delivery_is_not_implemented() {
    let harness = boot().await;
    drop(bulk(&harness.dir));
    let before = pastes(&harness.mux.recorded().await);
    let (status, body) = post(
        &harness.sock,
        json!({"ref": "bulk", "seat": "one", "mode": "startup"}),
    )
    .await;
    assert_eq!(status, 501, "{body}");
    assert_eq!(body["error"], "not_implemented");
    assert_eq!(pastes(&harness.mux.recorded().await), before);
    let shot = crate::query::capture(&harness.app, "one", None)
        .await
        .expect("capture");
    let screen = shot["text"].as_str().unwrap_or("");
    assert!(!screen.contains("PACK64-"), "{screen}");
}
