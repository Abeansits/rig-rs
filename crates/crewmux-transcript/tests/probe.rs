//! Claude jsonl cursor and user-turn reads.
#![expect(
    clippy::expect_used,
    reason = "temp-dir setup failures should fail the test"
)]

use std::path::{Path, PathBuf};

use crewmux_transcript::{project_slug, transcript_path, user_turns_after};

const SESSION: &str = "11111111-1111-4111-8111-111111111111";

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
    assert_eq!(missing.len(), 0);
    std::fs::write(&path, "{\"type\":\"user\"\n").expect("bad");
    assert!(user_turns_after(&path, 0).is_err());
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
    assert_eq!(empty.len(), 0);
    assert!(transcript_path(&root, &cwd, "sid").is_err());
    let braced =
        transcript_path(&root, &cwd, "{11111111-1111-4111-8111-111111111111}").expect("brace");
    assert_eq!(braced, path);
    drop(std::fs::remove_dir_all(&root));
}

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("rig-transcript-{label}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&root));
    std::fs::create_dir_all(&root).expect("scratch");
    root
}
