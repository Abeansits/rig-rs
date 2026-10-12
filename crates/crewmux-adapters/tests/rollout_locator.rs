//! Thread ids from rollout metadata, including the per-cwd launch lock.

#[path = "codex_support.rs"]
mod support;

use std::fs;
use std::io::Write as _;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crewmux_adapters::codex::{RolloutLocator, ThreadIdError, user_turn_texts, user_turns_after};
use crewmux_adapters::{AgentAdapter, LaunchOutcome};
use crewmux_mux::FakeMux;
use crewmux_types::ThreadId;

use support::{LOGGED_IN, STAMP, adapter, append_user, binding, home_with, opts, write_rollout};

#[tokio::test]
async fn back_to_back_launches_get_distinct_ids() {
    let home = home_with(LOGGED_IN);
    let mux = Arc::new(FakeMux::new());
    let adapter = Arc::new(adapter(&mux, &home.bin));
    let first = binding(&home.cwd, "seat-a");
    adapter
        .launch(&first, opts(&home, "seat-a"))
        .await
        .expect("first");
    let second_binding = binding(&home.cwd, "seat-b");
    let seat_b = second_binding.seat;
    let opts_b = opts(&home, "seat-b");
    let waiter = Arc::clone(&adapter);
    let pending = tokio::spawn(async move { waiter.launch(&second_binding, opts_b).await });
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !pending.is_finished(),
        "second launch must wait on the cwd lock"
    );
    write_rollout(&home.codex_dir, "thread-a", &home.cwd, None);
    adapter.check_ready(&first).await.expect("ready");
    assert_eq!(
        adapter.thread_id(first.seat).await.expect("id-a").as_str(),
        "thread-a"
    );
    let outcome = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .expect("second unblocked")
        .expect("join")
        .expect("second");
    assert!(matches!(outcome, LaunchOutcome::Ok { .. }));
    write_rollout(&home.codex_dir, "thread-b", &home.cwd, None);
    let mut follow = binding(&home.cwd, "seat-b");
    follow.seat = seat_b;
    adapter.check_ready(&follow).await.expect("ready-b");
    assert_eq!(
        adapter.thread_id(seat_b).await.expect("id-b").as_str(),
        "thread-b"
    );
}

#[test]
fn outside_rollout_is_correlated_by_the_first_message() {
    let home = home_with(LOGGED_IN);
    let locator = locator_for(&home);
    let before = locator.snapshot();
    write_rollout(&home.codex_dir, "thread-a", &home.cwd, Some("alpha"));
    write_rollout(&home.codex_dir, "thread-b", &home.cwd, Some("beta"));
    let id = locator
        .resolve(&before, &home.cwd, SystemTime::UNIX_EPOCH, Some("beta"))
        .expect("id");
    assert_eq!(id.as_str(), "thread-b");
}

#[test]
fn unresolvable_tie_is_ambiguous() {
    let home = home_with(LOGGED_IN);
    let locator = locator_for(&home);
    let before = locator.snapshot();
    write_rollout(&home.codex_dir, "thread-a", &home.cwd, Some("same"));
    write_rollout(&home.codex_dir, "thread-b", &home.cwd, Some("same"));
    let err = locator
        .resolve(&before, &home.cwd, SystemTime::UNIX_EPOCH, Some("same"))
        .expect_err("tie");
    assert!(matches!(err, ThreadIdError::Ambiguous(paths) if paths.len() == 2));
}

#[test]
fn snapshot_excludes_rollouts_from_before_launch() {
    let home = home_with(LOGGED_IN);
    write_rollout(&home.codex_dir, "thread-old", &home.cwd, None);
    let locator = locator_for(&home);
    let before = locator.snapshot();
    write_rollout(&home.codex_dir, "thread-new", &home.cwd, None);
    let id = locator
        .resolve(&before, &home.cwd, SystemTime::UNIX_EPOCH, None)
        .expect("new");
    assert_eq!(id.as_str(), "thread-new");
    let old = canonical(
        &home
            .codex_dir
            .join("sessions/2099/01/01/rollout-thread-old.jsonl"),
    );
    assert!(before.contains(&old));
}

#[test]
fn two_files_without_user_text_stay_not_yet() {
    let home = home_with(LOGGED_IN);
    let locator = locator_for(&home);
    let before = locator.snapshot();
    write_rollout(&home.codex_dir, "thread-a", &home.cwd, None);
    write_rollout(&home.codex_dir, "thread-b", &home.cwd, None);
    let err = locator
        .resolve(&before, &home.cwd, SystemTime::UNIX_EPOCH, Some("later"))
        .expect_err("waiting");
    assert!(matches!(err, ThreadIdError::NotYet));
}

#[test]
fn incomplete_utf8_tail_is_not_malformed() {
    let home = home_with(LOGGED_IN);
    let path = write_rollout(&home.codex_dir, "thread-a", &home.cwd, Some("kept"));
    let mut bytes = fs::read(&path).expect("bytes");
    bytes.push(0xC3);
    fs::write(&path, &bytes).expect("tail");
    let turns = user_turn_texts(&path).expect("skip tail");
    assert_eq!(turns, ["kept"]);
    bytes.push(b'\n');
    fs::write(&path, &bytes).expect("complete bad line");
    let err = user_turn_texts(&path).expect_err("malformed");
    assert!(matches!(err, ThreadIdError::Malformed(_)));
    fs::write(&path, [0xC3]).expect("only a partial character");
    let err = user_turn_texts(&path).expect_err("pending");
    assert!(matches!(err, ThreadIdError::NotYet));
}

#[test]
fn user_text_before_the_baseline_does_not_count() {
    let home = home_with(LOGGED_IN);
    let path = write_rollout(&home.codex_dir, "thread-a", &home.cwd, Some("yes"));
    let baseline = fs::metadata(&path).expect("meta").len();
    let turns = user_turns_after(&path, baseline).expect("offset");
    assert_eq!(turns, Vec::<String>::new());
    append_user(&path, "yes");
    let turns = user_turns_after(&path, baseline).expect("new");
    assert_eq!(turns, ["yes"]);
}

#[test]
fn partial_last_line_is_skipped_while_polling() {
    let home = home_with(LOGGED_IN);
    let path = write_rollout(&home.codex_dir, "thread-a", &home.cwd, Some("kept"));
    let mut partial = fs::read_to_string(&path).expect("rollout");
    partial.push_str("{\"payload\":{\"text\":\"hel\"");
    fs::write(&path, &partial).expect("partial");
    let turns = user_turn_texts(&path).expect("skip");
    assert_eq!(turns, ["kept"]);
    let mut finished_bad = partial;
    finished_bad.push_str("\n{bad}\n");
    fs::write(&path, finished_bad).expect("bad");
    let err = user_turn_texts(&path).expect_err("malformed");
    assert!(matches!(err, ThreadIdError::Malformed(_)));
}

#[test]
fn resume_appends_to_the_same_rollout() {
    let home = home_with(LOGGED_IN);
    let path = write_rollout(&home.codex_dir, "thread-a", &home.cwd, Some("first"));
    let before = std::fs::metadata(&path).expect("meta").len();
    let id = ThreadId::new("thread-a").expect("id");
    let locator = locator_for(&home);
    assert_eq!(
        locator.verify_resume(&id, &home.cwd).expect("path"),
        canonical(&path)
    );
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open");
    writeln!(
        file,
        "{{\"type\":\"event\",\"payload\":{{\"text\":\"more\"}}}}"
    )
    .expect("append");
    let after = std::fs::metadata(&path).expect("meta").len();
    assert!(after > before, "{STAMP} {before} -> {after}");
    assert_eq!(
        locator.verify_resume(&id, &home.cwd).expect("same"),
        canonical(&path)
    );
}

#[tokio::test]
async fn resume_argv_names_the_existing_thread() {
    let home = home_with(LOGGED_IN);
    write_rollout(&home.codex_dir, "thread-a", &home.cwd, None);
    let mux = Arc::new(FakeMux::new());
    let seat = adapter(&mux, &home.bin);
    let binding = binding(&home.cwd, "seat");
    let mut opts = opts(&home, "seat");
    opts.resume = Some(crewmux_adapters::ResumeToken {
        kind: "codex".to_owned(),
        value: "thread-a".to_owned(),
    });
    seat.launch(&binding, opts).await.expect("resume");
    let calls = mux.recorded().await;
    let flat: Vec<String> = calls.into_iter().flatten().collect();
    let resume_at = flat.iter().position(|arg| arg == "resume").expect("resume");
    assert_eq!(
        flat.get(resume_at + 1).map(String::as_str),
        Some("thread-a")
    );
    assert_eq!(
        seat.thread_id(binding.seat).await.expect("id").as_str(),
        "thread-a"
    );
}

fn locator_for(home: &support::Home) -> RolloutLocator {
    RolloutLocator {
        sessions_root: home.codex_dir.join("sessions"),
    }
}

fn canonical(path: &std::path::Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
