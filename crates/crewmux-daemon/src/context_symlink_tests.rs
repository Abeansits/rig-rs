//! Symlinks under a pack must not be followed by deliver, get, or preview.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_api::Client;
use crewmux_mux::FakeMux;
use serde_json::Value;

use crate::flow::harness_with;
use crate::state::App;

const SECRET: &str = "SYMLINK-SECRET-BYTES";

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
        "rig-symlink-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let (app, mux) = harness_with(None, &dir, None).await;
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

async fn call(sock: &Path, method: &str, path: &str, body: Option<Vec<u8>>) -> (u16, String) {
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
    (response.status, response.body)
}

fn manifest(member: &str) -> String {
    format!(
        "name: leakpack\nversion: \"1\"\ntaxonomy: mission\nfiles:\n  - path: {member}\n    role: source\n"
    )
}

fn install_real(home: &Path) -> PathBuf {
    let pack = home.join("context").join("leakpack");
    std::fs::create_dir_all(&pack).expect("pack");
    std::fs::write(pack.join("manifest.yaml"), manifest("notes.md")).expect("manifest");
    std::fs::write(pack.join("notes.md"), "inside").expect("notes");
    pack
}

async fn sync(sock: &Path) {
    let (status, body) = call(
        sock,
        "POST",
        "/api/context-packs/library/sync",
        Some(b"{}".to_vec()),
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

fn outside(home: &Path) -> PathBuf {
    let path = home.join("secret.txt");
    std::fs::write(&path, SECRET).expect("secret");
    path
}

async fn refuse_all(harness: &Harness) {
    let before = pastes(&harness.mux.recorded().await);
    let preview = format!(
        "/api/context-packs/library/by-ref/preview?ref={}",
        "leakpack"
    );
    let pieces = format!(
        "/api/context-packs/library/by-ref/pieces?ref={}",
        "leakpack"
    );
    assert_refused(&harness.sock, "GET", &preview, None).await;
    assert_refused(&harness.sock, "GET", &pieces, None).await;
    let deliver = serde_json::to_vec(&serde_json::json!({
        "ref": "leakpack",
        "seat": "one",
        "mode": "paste",
    }))
    .expect("json");
    assert_refused(
        &harness.sock,
        "POST",
        "/api/context-packs/deliver",
        Some(deliver),
    )
    .await;
    assert_eq!(pastes(&harness.mux.recorded().await), before);
}

async fn assert_refused(sock: &Path, method: &str, path: &str, body: Option<Vec<u8>>) {
    let (status, text) = call(sock, method, path, body).await;
    assert_ne!(status, 200, "{path} {text}");
    assert!(
        !text.contains(SECRET),
        "{path} leaked {SECRET} bytes: {text}"
    );
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    assert_eq!(value["error"], "file_outside_pack", "{path} {text}");
}

fn pastes(calls: &[Vec<String>]) -> usize {
    calls
        .iter()
        .filter(|argv| argv.iter().any(|arg| arg == "paste-buffer"))
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deliver_get_and_preview_refuse_a_symlinked_file() {
    let harness = boot().await;
    let pack = install_real(&harness.dir);
    let secret = outside(&harness.dir);
    std::fs::remove_file(pack.join("notes.md")).expect("remove");
    std::os::unix::fs::symlink(&secret, pack.join("notes.md")).expect("link");
    sync(&harness.sock).await;
    refuse_all(&harness).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deliver_get_and_preview_refuse_a_symlinked_directory() {
    let harness = boot().await;
    let pack = harness.dir.join("context").join("leakpack");
    std::fs::create_dir_all(&pack).expect("pack");
    std::fs::write(pack.join("manifest.yaml"), manifest("nested/notes.md")).expect("manifest");
    let outside_dir = harness.dir.join("outside-dir");
    std::fs::create_dir_all(&outside_dir).expect("outside");
    std::fs::write(outside_dir.join("notes.md"), SECRET).expect("secret");
    std::os::unix::fs::symlink(&outside_dir, pack.join("nested")).expect("link");
    sync(&harness.sock).await;
    refuse_all(&harness).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deliver_get_and_preview_refuse_a_swap_after_install() {
    let harness = boot().await;
    let pack = install_real(&harness.dir);
    sync(&harness.sock).await;
    let secret = outside(&harness.dir);
    std::fs::remove_file(pack.join("notes.md")).expect("remove");
    std::os::unix::fs::symlink(&secret, pack.join("notes.md")).expect("swap");
    refuse_all(&harness).await;
}
