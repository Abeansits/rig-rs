//! PE-004: refuse the Claude user config while a Claude process is running.
#![expect(
    clippy::unwrap_used,
    reason = "guard tests assert temp-file results directly"
)]

use std::fs;
use std::os::unix::fs::MetadataExt;

use crewmux_provider::{
    Backups, ClaudeTrust, FileStamp, ProcessProbe, TrustOutcome, user_config_basename,
};

struct Running;

impl ProcessProbe for Running {
    fn claude_pids(&self) -> Vec<u32> {
        vec![4242]
    }
}

struct Idle;

impl ProcessProbe for Idle {
    fn claude_pids(&self) -> Vec<u32> {
        Vec::new()
    }
}

fn stamp(path: &std::path::Path) -> FileStamp {
    let meta = fs::metadata(path).unwrap();
    FileStamp {
        inode: meta.ino(),
        modified: meta.modified().ok(),
        len: meta.len(),
    }
}

#[test]
fn refuses_while_claude_running() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(user_config_basename());
    let original = b"{\"theme\":\"dark\",\"projects\":{}}\n";
    fs::write(&path, original).unwrap();
    let before = stamp(&path);
    let trust = ClaudeTrust::new(&Running, &path, Backups::new(&dir.path().join("backups")));
    let outcome = trust.pre_accept(dir.path()).unwrap();
    assert_eq!(outcome, TrustOutcome::SkippedClaudeRunning);
    assert_eq!(stamp(&path), before);
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(trust.stamp().unwrap().is_some());
}

#[test]
fn writes_trust_when_idle_and_keeps_other_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(user_config_basename());
    let original =
        "{\n  \"theme\": \"dark\",\n  \"projects\": {\n    \"/elsewhere\": {\"ok\":true}\n  }\n}\n";
    fs::write(&path, original).unwrap();
    let trust = ClaudeTrust::new(&Idle, &path, Backups::new(&dir.path().join("backups")));
    let project = dir.path().join("seat");
    assert_eq!(trust.pre_accept(&project).unwrap(), TrustOutcome::Written);
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"theme\": \"dark\""));
    assert!(text.contains("\"/elsewhere\""));
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        parsed["projects"][project.to_str().unwrap()]["hasTrustDialogAccepted"],
        true
    );
    assert_eq!(
        trust.pre_accept(&project).unwrap(),
        TrustOutcome::AlreadyTrusted
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn concurrent_writer_loss_rate_is_reported() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::thread;
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(user_config_basename());
    fs::write(&path, "{\"projects\":{}}").unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let writes = Arc::new(AtomicU64::new(0));
    let losses = Arc::new(AtomicU64::new(0));
    let worker = {
        let path = path.clone();
        let stop = Arc::clone(&stop);
        let writes = Arc::clone(&writes);
        let losses = Arc::clone(&losses);
        thread::spawn(move || {
            let mut generation = 0u64;
            while !stop.load(Ordering::Relaxed) {
                generation = generation.saturating_add(1);
                let current = fs::read_to_string(&path).unwrap_or_default();
                if current.contains("hasTrustDialogAccepted") {
                    losses.fetch_add(1, Ordering::Relaxed);
                }
                let body = format!("{{\"other\":{generation},\"projects\":{{}}}}");
                fs::write(&path, body).ok();
                writes.fetch_add(1, Ordering::Relaxed);
                thread::park_timeout(Duration::from_millis(10));
            }
        })
    };
    let trust = ClaudeTrust::new(&Idle, &path, Backups::new(&dir.path().join("backups")));
    let project = dir.path().join("proj");
    let mut wrote = false;
    for _ in 0..20 {
        if trust.pre_accept(&project).is_ok() {
            wrote = true;
            break;
        }
        thread::park_timeout(Duration::from_millis(5));
    }
    stop.store(true, Ordering::Relaxed);
    worker.thread().unpark();
    worker.join().unwrap();
    assert!(wrote, "pre_accept never observed a quiet window");
    let again = trust.pre_accept(&project).unwrap();
    assert!(matches!(
        again,
        TrustOutcome::Written | TrustOutcome::AlreadyTrusted
    ));
    let final_text = fs::read_to_string(&path).unwrap();
    assert!(
        final_text.contains("hasTrustDialogAccepted"),
        "{final_text}"
    );
    let write_count = writes.load(Ordering::Relaxed);
    let loss_count = losses.load(Ordering::Relaxed);
    println!("claude_writer_loss_rate losses={loss_count} writes={write_count}");
}
