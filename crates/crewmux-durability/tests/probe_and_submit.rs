//! Transcript probe and a submit that confirms through [`crewmux_durability::ClaudeDelivery`].
#![expect(
    clippy::expect_used,
    reason = "temp-dir setup failures should fail the test"
)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crewmux_adapters::{ClaudeAdapter, Error, SeatIo, SubmitMode};
use crewmux_durability::{ClaudeDelivery, project_slug, transcript_path, user_turns_after};
use crewmux_mux::{FakeMux, Multiplexer};
use crewmux_types::SeatId;

#[test]
fn slug_and_user_turns() {
    assert_eq!(project_slug(Path::new("/tmp/work")), "-tmp-work");
    let root = scratch("probe");
    let cwd = PathBuf::from("/tmp/work");
    let path = transcript_path(&root, &cwd, SESSION).expect("uuid");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    let body = "\
{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}
{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"nope\"}}
{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"content\":\"skip\"},{\"type\":\"text\",\"text\":\"kept\"}]}}
";
    std::fs::write(&path, body).expect("write");
    let turns = user_turns_after(&path, 0).expect("turns");
    let texts: Vec<&str> = turns.iter().map(|turn| turn.text.as_str()).collect();
    assert_eq!(texts, ["hello", "kept"]);
    let mid = u64::try_from(body.find("assistant").expect("assistant")).expect("offset");
    let after = user_turns_after(&path, mid).expect("after");
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].text, "kept");
    let missing = user_turns_after(&path.with_file_name("missing.jsonl"), 0).expect("missing");
    assert_eq!(missing, Vec::new());
    std::fs::write(&path, "{\"type\":\"user\"\n").expect("bad");
    assert!(user_turns_after(&path, 0).is_err());
    drop(std::fs::remove_dir_all(&root));
}

#[tokio::test]
async fn submit_confirms_against_the_session_file() {
    let root = scratch("submit");
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let home = root.join("home");
    let session = SESSION;
    let path = transcript_path(&home, &cwd, session).expect("uuid");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(
        &path,
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"old\"}}\n",
    )
    .expect("seed");
    let mux = Arc::new(FakeMux::new());
    let ready = "\
Claude Code v2.1.286

╭────────────────────────────────────────╮
│ >                                      │
╰────────────────────────────────────────╯
  ? for shortcuts
";
    mux.set_capture(crewmux_mux::Capture::new(ready.to_owned(), (0, 0)))
        .await;
    let watch = Arc::clone(&mux);
    let append = path.clone();
    tokio::spawn(async move {
        loop {
            let calls = watch.recorded().await;
            if calls.iter().flatten().any(|arg| arg == "paste-buffer") {
                let line = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\\nworld\"}}\n";
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&append)
                    .expect("open");
                file.write_all(line.as_bytes()).expect("append");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let seats = root.join("seats");
    let cloned = Arc::clone(&mux);
    let shared: Arc<dyn Multiplexer> = cloned;
    let adapter = ClaudeAdapter::new(shared, Arc::new(ClaudeDelivery), seats, home)
        .with_submit_wait(Duration::from_secs(2));
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
    std::fs::create_dir_all(adapter.seat_dir(&binding.seat)).expect("seat");
    std::fs::write(
        adapter.seat_dir(&binding.seat).join("claude-session-id"),
        session,
    )
    .expect("id");
    let io = adapter.io_for(&binding).await.expect("io");
    let receipt = io
        .submit("hello\nworld", SubmitMode::Enter)
        .await
        .expect("confirmed");
    assert_eq!(receipt.bytes, "hello\nworld".len());
    drop(std::fs::remove_dir_all(&root));
}

#[tokio::test]
async fn submit_reports_not_verified_when_the_turn_never_lands() {
    let root = scratch("miss");
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let mux = Arc::new(FakeMux::new());
    mux.set_capture(crewmux_mux::Capture::new(
        "╭────╮\n│ >  │\n╰────╯\n  ? for shortcuts\n".to_owned(),
        (0, 0),
    ))
    .await;
    let shared: Arc<dyn Multiplexer> = mux;
    let adapter = ClaudeAdapter::new(
        shared,
        Arc::new(ClaudeDelivery),
        root.join("seats"),
        root.join("home"),
    )
    .with_submit_wait(Duration::from_millis(40));
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
    std::fs::create_dir_all(adapter.seat_dir(&binding.seat)).expect("seat");
    std::fs::write(
        adapter.seat_dir(&binding.seat).join("claude-session-id"),
        SESSION,
    )
    .expect("id");
    let io = adapter.io_for(&binding).await.expect("io");
    let err = io
        .submit("hello", SubmitMode::Enter)
        .await
        .expect_err("wait");
    assert!(matches!(err, Error::NotVerified));
    drop(std::fs::remove_dir_all(&root));
}

#[test]
fn incomplete_trailing_jsonl_is_skipped() {
    let root = scratch("tail");
    let cwd = PathBuf::from("/tmp/work");
    let path = transcript_path(&root, &cwd, SESSION).expect("uuid");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    let complete = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n";
    let torn =
        format!("{complete}{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"tor");
    std::fs::write(&path, torn).expect("write");
    let turns = user_turns_after(&path, 0).expect("skip tail");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].text, "hello");
    std::fs::write(&path, "{\"type\":\"user\"").expect("partial");
    let empty = user_turns_after(&path, 0).expect("unterminated");
    assert_eq!(empty, Vec::new());
    assert!(transcript_path(&root, &cwd, "sid").is_err());
    drop(std::fs::remove_dir_all(&root));
}

const SESSION: &str = "11111111-1111-4111-8111-111111111111";

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("rig-dur-{label}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&root));
    std::fs::create_dir_all(&root).expect("scratch");
    root
}
