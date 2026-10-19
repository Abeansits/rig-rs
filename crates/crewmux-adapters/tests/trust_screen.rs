//! Trust Enter only when the on-screen folder is the seat cwd.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a mismatched fixture should fail the test"
)]

use std::path::PathBuf;
use std::sync::Arc;

use crewmux_adapters::{AgentAdapter, ClaudeAdapter, LaunchOpts, Readiness};
use crewmux_mux::{FakeMux, Multiplexer};
use crewmux_types::{ReadinessCode, SeatId};

#[path = "common/mod.rs"]
mod support;

const TRUST_SHA: &str = "aba7d2dd9c877940d91879169099b512625956f36086be82cb4bd75fc2b40052";
const FOLDER: &str = "/tmp/rig-trust/project-with-a-long-directory-name/src/app";

#[tokio::test]
async fn real_trust_dialog_downs_from_no_then_enters_yes() {
    let no = screen("2.1.287/claude-2.1.287-trust-navprobe-initial-no.txt");
    let yes = screen("2.1.287/claude-2.1.287-trust-after-down-yes.txt");
    let ready = screen("2.1.287/claude-2.1.287-ready-after-trust.txt");
    let mux = Arc::new(FakeMux::new());
    mux.replay_screens(vec![
        support::capture(&no),
        support::capture(&yes),
        support::capture(&ready),
    ])
    .await;
    let adapter = adapter(Arc::clone(&mux));
    let cwd = PathBuf::from("/tmp/rig-qa1/openrig-spikes-scratch/rig-rs/w18probe032913");
    let binding = bound(cwd);
    drop(adapter.launch(&binding, opts(true)).await.expect("launch"));
    let calls = mux.recorded().await;
    assert!(sent_down(&calls), "{calls:?}");
    assert!(support::sent_enter(&calls), "{calls:?}");
    assert!(down_before_enter(&calls), "{calls:?}");
    assert_eq!(
        adapter.check_ready(&binding).await.expect("ready"),
        Readiness::Ready
    );
}

#[tokio::test]
async fn synthetic_wrapped_trust_enters_when_the_cursor_is_already_on_yes() {
    let trust = trust_screen();
    assert_eq!(crewmux_transcript::fingerprint_raw(&trust), TRUST_SHA);
    let ready = support::read(&support::fixtures().join("2.1.286/claude-020-ready.txt"));
    let mux = Arc::new(FakeMux::new());
    mux.replay_screens(vec![support::capture(&trust), support::capture(&ready)])
        .await;
    let adapter = adapter(Arc::clone(&mux));
    let binding = bound(PathBuf::from(FOLDER));
    drop(adapter.launch(&binding, opts(true)).await.expect("launch"));
    let calls = mux.recorded().await;
    assert!(support::sent_enter(&calls), "{calls:?}");
    assert!(!sent_down(&calls), "{calls:?}");
    assert_eq!(
        adapter.check_ready(&binding).await.expect("ready"),
        Readiness::Ready
    );
}

#[tokio::test]
async fn real_trust_dialog_refuses_a_prefix_and_a_different_folder() {
    let trust = screen("2.1.287/claude-2.1.287-trust-cursor-no.txt");
    for cwd in [
        "/tmp/rig-qa1/openrig-spikes-scratch/rig-rs",
        "/tmp/not-the-folder",
    ] {
        let mux = replay(&trust).await;
        let adapter = adapter(Arc::clone(&mux));
        let binding = bound(PathBuf::from(cwd));
        drop(adapter.launch(&binding, opts(true)).await.expect("launch"));
        assert!(!support::sent_enter(&mux.recorded().await), "{cwd}");
        assert_eq!(
            adapter.check_ready(&binding).await.expect("ready"),
            Readiness::Gate(ReadinessCode::TrustGate),
            "{cwd}"
        );
    }
}

#[tokio::test]
async fn tilde_folder_matches_only_under_the_seat_home() {
    let shown = tilde_screen("~/proj");
    let home = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/claude-tilde-home");
    let cwd = home.join("proj");
    let ready = support::read(&support::fixtures().join("2.1.286/claude-020-ready.txt"));
    let mux = Arc::new(FakeMux::new());
    mux.replay_screens(vec![support::capture(&shown), support::capture(&ready)])
        .await;
    let adapter = adapter_home(Arc::clone(&mux), home.clone());
    let binding = bound(cwd);
    drop(adapter.launch(&binding, opts(true)).await.expect("launch"));
    assert!(support::sent_enter(&mux.recorded().await));

    let other = tilde_screen("~/other");
    let mux = replay(&other).await;
    let adapter = adapter_home(Arc::clone(&mux), home.clone());
    let binding = bound(home.join("proj"));
    drop(adapter.launch(&binding, opts(true)).await.expect("launch"));
    assert!(!support::sent_enter(&mux.recorded().await));
    assert_eq!(
        adapter.check_ready(&binding).await.expect("ready"),
        Readiness::Gate(ReadinessCode::TrustGate)
    );
}

#[tokio::test]
async fn settled_yes_enters_when_the_folder_matches() {
    let yes = screen("2.1.287/claude-2.1.287-trust-settled-yes.txt");
    let ready = screen("2.1.287/claude-2.1.287-ready-after-trust.txt");
    let mux = Arc::new(FakeMux::new());
    mux.replay_screens(vec![support::capture(&yes), support::capture(&ready)])
        .await;
    let adapter = adapter(Arc::clone(&mux));
    let cwd = PathBuf::from("/tmp/rig-qa1/openrig-spikes-scratch/rig-rs/w18r2b032943");
    let binding = bound(cwd);
    drop(adapter.launch(&binding, opts(true)).await.expect("launch"));
    assert!(support::sent_enter(&mux.recorded().await));
    assert_eq!(
        adapter.check_ready(&binding).await.expect("ready"),
        Readiness::Ready
    );
}

#[tokio::test]
async fn cursor_no_folder_matches_the_sanitized_trust_pane() {
    let no = screen("2.1.287/claude-2.1.287-trust-cursor-no.txt");
    let yes = screen("2.1.287/claude-2.1.287-trust-settled-yes.txt");
    assert!(
        !no.contains("/Users/"),
        "sanitized fixtures must not contain a home path"
    );
    assert!(!yes.contains("/Users/"));
}

fn screen(name: &str) -> String {
    support::read(&support::fixtures().join(name))
}

fn trust_screen() -> String {
    support::read(&support::fixtures().join("2.1.287/claude-trust.txt"))
}

fn sent_down(calls: &[Vec<String>]) -> bool {
    calls.iter().flatten().any(|arg| arg == "Down")
}

fn down_before_enter(calls: &[Vec<String>]) -> bool {
    let mut saw_down = false;
    for arg in calls.iter().flatten() {
        if arg == "Down" {
            saw_down = true;
        }
        if arg == "Enter" {
            return saw_down;
        }
    }
    false
}

fn tilde_screen(folder: &str) -> String {
    support::read(&support::fixtures().join("2.1.286/claude-006-trust.txt"))
        .replace("/Users/sebas/openrig-spikes-scratch/proj/claude-f1", folder)
        .replace(
            " ❯ No, exit\n   Yes, I trust this folder",
            "   No, exit\n ❯ Yes, I trust this folder",
        )
}

async fn replay(screen: &str) -> Arc<FakeMux> {
    let mux = Arc::new(FakeMux::new());
    mux.replay_screens(vec![support::capture(screen)]).await;
    mux
}

fn adapter(mux: Arc<FakeMux>) -> ClaudeAdapter {
    let home = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/claude-trust-home");
    adapter_home(mux, home)
}

fn adapter_home(mux: Arc<FakeMux>, home: PathBuf) -> ClaudeAdapter {
    let seats = home.join(format!("seats-{}", std::process::id()));
    let mux: Arc<dyn Multiplexer> = mux;
    ClaudeAdapter::new(mux, seats, home)
}

fn bound(cwd: PathBuf) -> crewmux_adapters::NodeBinding {
    std::fs::create_dir_all(&cwd).expect("cwd");
    crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig")
}

fn opts(auto_trust: bool) -> LaunchOpts {
    LaunchOpts {
        session_name: "seat__rig".to_owned(),
        env: std::collections::BTreeMap::new(),
        resume: None,
        fork: false,
        auto_trust,
        budget: Some(support::fast_budget()),
        claude_bin: None,
    }
}
