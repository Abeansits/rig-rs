//! A directory swapped for a symlink must not leak the symlink target.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crewmux_api::Client;
use crewmux_messaging::obligation_fingerprint;
use crewmux_mux::FakeMux;

use crate::flow::{harness_with, two_stub};
use crate::launch::bring_up;
use crate::state::App;

const TRIALS: u32 = 60;
const SECRET: &str = "OUTSIDE-SECRET-BYTES";
const INSIDE: &str = "INSIDE-PACK-BYTES";

struct Harness {
    app: App,
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
        "rig-race-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let (app, _mux): (App, Arc<FakeMux>) = harness_with(None, &dir, None).await;
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
    Harness { app, sock, dir }
}

fn install(home: &Path) -> PathBuf {
    let pack = home.join("context").join("racepack");
    let sub = pack.join("sub");
    std::fs::create_dir_all(&sub).expect("sub");
    std::fs::write(sub.join("notes.md"), INSIDE).expect("inside");
    std::fs::write(
        pack.join("manifest.yaml"),
        "name: racepack\nversion: \"1\"\ntaxonomy: mission\nfiles:\n  - path: sub/notes.md\n    role: source\n",
    )
    .expect("manifest");
    let outside = home.join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::write(outside.join("notes.md"), SECRET).expect("secret");
    std::os::unix::fs::symlink(&outside, pack.join("alias")).expect("alias");
    pack
}

async fn call(sock: &Path, method: &str, path: &str, body: Option<Vec<u8>>) -> String {
    let sock = sock.to_path_buf();
    let method = method.to_owned();
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        Client::over_uds(sock)
            .request(&method, &path, body.as_deref())
            .expect("http")
            .body
    })
    .await
    .expect("join")
}

fn exchange(sub: &Path, alias: &Path) -> bool {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        sub,
        rustix::fs::CWD,
        alias,
        rustix::fs::RenameFlags::EXCHANGE,
    )
    .is_ok()
}

fn swap_until(sub: &Path, alias: &Path, stop: &AtomicBool, swaps: &AtomicU32) {
    while !stop.load(Ordering::Relaxed) {
        if exchange(sub, alias) {
            swaps.fetch_add(1, Ordering::Relaxed);
        }
        std::thread::yield_now();
    }
}

fn leaked(body: &str, fingerprint: &str) -> bool {
    body.contains(SECRET) || body.contains(fingerprint)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn swapping_a_directory_for_a_symlink_never_returns_outside_bytes() {
    let harness = boot().await;
    let pack = install(&harness.dir);
    let synced = call(
        &harness.sock,
        "POST",
        "/api/context-packs/library/sync",
        Some(b"{}".to_vec()),
    )
    .await;
    assert!(!synced.contains(SECRET), "{synced}");
    let stop = Arc::new(AtomicBool::new(false));
    let swaps = Arc::new(AtomicU32::new(0));
    let worker = {
        let stop = Arc::clone(&stop);
        let swaps = Arc::clone(&swaps);
        let sub = pack.join("sub");
        let alias = pack.join("alias");
        std::thread::spawn(move || swap_until(&sub, &alias, &stop, &swaps))
    };
    let fingerprint = obligation_fingerprint("context", "one@pair", SECRET);
    let mut leaks = 0u32;
    let mut inside = 0u32;
    let pieces = "/api/context-packs/library/by-ref/pieces?ref=racepack";
    let deliver = serde_json::to_vec(&serde_json::json!({
        "ref": "racepack",
        "seat": "one",
        "mode": "paste",
    }))
    .expect("json");
    for _ in 0..TRIALS {
        let got = call(&harness.sock, "GET", pieces, None).await;
        leaks += u32::from(leaked(&got, &fingerprint));
        inside += u32::from(got.contains(INSIDE));
        let posted = call(
            &harness.sock,
            "POST",
            "/api/context-packs/deliver",
            Some(deliver.clone()),
        )
        .await;
        leaks += u32::from(leaked(&posted, &fingerprint));
    }
    stop.store(true, Ordering::Relaxed);
    worker.join().expect("swap thread");
    let shot = crate::query::capture(&harness.app, "one", None)
        .await
        .expect("capture");
    let screen = shot["text"].as_str().unwrap_or("");
    leaks += u32::from(screen.contains(SECRET));
    let swaps = swaps.load(Ordering::Relaxed);
    println!(
        "race: leaks={leaks} gets={TRIALS} delivers={TRIALS} swaps={swaps} inside_gets={inside}"
    );
    assert!(
        swaps > 0,
        "the directory was never exchanged with the symlink"
    );
    assert_eq!(
        leaks, 0,
        "outside bytes appeared {leaks} times across {swaps} swaps"
    );
}

fn install_pack_root(home: &Path) -> (PathBuf, PathBuf) {
    let context = home.join("context");
    let pack = context.join("racepack");
    std::fs::create_dir_all(&pack).expect("pack");
    let manifest = "name: racepack\nversion: \"1\"\ntaxonomy: mission\nfiles:\n  - path: notes.md\n    role: source\n";
    std::fs::write(pack.join("manifest.yaml"), manifest).expect("manifest");
    std::fs::write(pack.join("notes.md"), INSIDE).expect("inside");
    let outside = home.join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::write(outside.join("manifest.yaml"), manifest).expect("outside manifest");
    std::fs::write(outside.join("notes.md"), SECRET).expect("secret");
    let alias = context.join("alias");
    std::os::unix::fs::symlink(&outside, &alias).expect("alias");
    (pack, alias)
}

fn leave_pack_symlinked(pack: &Path, alias: &Path) {
    let linked = std::fs::symlink_metadata(pack).is_ok_and(|meta| meta.file_type().is_symlink());
    if !linked {
        assert!(
            exchange(pack, alias),
            "could not leave the pack root as a symlink"
        );
    }
    let linked = std::fs::symlink_metadata(pack).is_ok_and(|meta| meta.file_type().is_symlink());
    assert!(
        linked,
        "pack root is a directory, so a followed open would not be caught"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn swapping_the_pack_root_for_a_symlink_never_returns_outside_bytes() {
    let harness = boot().await;
    let (pack, alias) = install_pack_root(&harness.dir);
    let synced = call(
        &harness.sock,
        "POST",
        "/api/context-packs/library/sync",
        Some(b"{}".to_vec()),
    )
    .await;
    assert!(!synced.contains(SECRET), "{synced}");
    let stop = Arc::new(AtomicBool::new(false));
    let swaps = Arc::new(AtomicU32::new(0));
    let worker = {
        let stop = Arc::clone(&stop);
        let swaps = Arc::clone(&swaps);
        let pack = pack.clone();
        let alias = alias.clone();
        std::thread::spawn(move || swap_until(&pack, &alias, &stop, &swaps))
    };
    let fingerprint = obligation_fingerprint("context", "one@pair", SECRET);
    let mut leaks = 0u32;
    let pieces = "/api/context-packs/library/by-ref/pieces?ref=racepack";
    let deliver = serde_json::to_vec(&serde_json::json!({
        "ref": "racepack",
        "seat": "one",
        "mode": "paste",
    }))
    .expect("json");
    for _ in 0..TRIALS {
        let got = call(&harness.sock, "GET", pieces, None).await;
        leaks += u32::from(leaked(&got, &fingerprint));
        let posted = call(
            &harness.sock,
            "POST",
            "/api/context-packs/deliver",
            Some(deliver.clone()),
        )
        .await;
        leaks += u32::from(leaked(&posted, &fingerprint));
    }
    stop.store(true, Ordering::Relaxed);
    worker.join().expect("swap thread");
    leave_pack_symlinked(&pack, &alias);
    let settled = call(&harness.sock, "GET", pieces, None).await;
    leaks += u32::from(leaked(&settled, &fingerprint));
    assert!(
        settled.contains("file_outside_pack"),
        "a symlinked pack root was not refused, so reverting O_NOFOLLOW would pass: {settled}"
    );
    let shot = crate::query::capture(&harness.app, "one", None)
        .await
        .expect("capture");
    let screen = shot["text"].as_str().unwrap_or("");
    leaks += u32::from(screen.contains(SECRET));
    let swaps = swaps.load(Ordering::Relaxed);
    println!("pack-root race: leaks={leaks} swaps={swaps}");
    assert!(
        swaps > 0,
        "the pack root was never exchanged with the symlink"
    );
    assert_eq!(
        leaks, 0,
        "outside bytes appeared {leaks} times across {swaps} swaps"
    );
}
