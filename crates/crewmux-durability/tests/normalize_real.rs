//! ADR-0009 Amendment 2: Codex is byte-exact; Claude unwraps one pasted wrapper.
#![expect(
    clippy::expect_used,
    reason = "a missing transcript fixture fails the test"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use crewmux_adapters::codex::{delivery_fingerprint, normalize_for, same_delivery};
use crewmux_durability::CodexRolloutProbe;
use crewmux_types::RuntimeKind;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/transcripts")
}

#[test]
fn claude_rows_match_only_after_unwrap() {
    let rows = load("claude/rows.json");
    assert!(rows.len() >= 2, "need single-line and multi-line rows");
    for row in rows {
        let sent = row["sent"].as_str().expect("sent");
        let stored = row["stored"].as_str().expect("stored");
        assert!(
            same_delivery(RuntimeKind::ClaudeCode, sent, stored),
            "{}",
            row["id"]
        );
        if row["multiline"].as_bool().expect("flag") {
            assert_ne!(raw_fp(sent), raw_fp(stored), "{}", row["id"]);
            assert_ne!(
                delivery_fingerprint(RuntimeKind::Codex, sent),
                delivery_fingerprint(RuntimeKind::Codex, stored),
                "codex must not unwrap {}",
                row["id"]
            );
        } else {
            assert_eq!(sent, stored);
        }
    }
}

#[test]
fn codex_rows_are_byte_exact() {
    let rows = load("codex/rows.json");
    assert_ne!(rows.len(), 0);
    for row in rows {
        let sent = row["sent"].as_str().expect("sent");
        let stored = row["stored"].as_str().expect("stored");
        assert_eq!(sent, stored, "{}", row["id"]);
        assert_eq!(normalize_for(RuntimeKind::Codex, sent), sent);
        assert!(same_delivery(RuntimeKind::Codex, sent, stored));
    }
}

#[test]
fn codex_64kb_is_identity() {
    let body = "y".repeat(64 * 1024);
    assert_eq!(normalize_for(RuntimeKind::Codex, &body), body);
    assert!(same_delivery(RuntimeKind::Codex, &body, &body));
}

#[test]
fn negatives_match_amendment_2() {
    let literal = "keep <pasted_content> in the middle";
    assert_eq!(normalize_for(RuntimeKind::ClaudeCode, literal), literal);
    assert!(!same_delivery(RuntimeKind::Codex, "a b", "a  b"));
    assert!(!same_delivery(RuntimeKind::ClaudeCode, "a b", "a  b"));
    assert!(same_delivery(RuntimeKind::ClaudeCode, "a\tb", "a    b"));
    assert!(!same_delivery(RuntimeKind::Codex, "a\tb", "a    b"));
    let wrapped = "\n<pasted_content id=\"1\">\na\nb\n</pasted_content>\n";
    assert!(same_delivery(RuntimeKind::ClaudeCode, "a\nb", wrapped));
    assert!(!same_delivery(RuntimeKind::Codex, "a\nb", wrapped));
}

#[test]
fn probe_matches_a_byte_exact_user_item() {
    let dir = tempfile::tempdir().expect("temp");
    let cwd = dir.path().join("work");
    fs::create_dir_all(&cwd).expect("cwd");
    let cwd = cwd.canonicalize().expect("canon");
    let path = dir.path().join("rollout-thread-a.jsonl");
    let body = "hello\nworld";
    let line = serde_json::json!({
        "type": "session_meta",
        "timestamp": "2099-01-01T00:00:00Z",
        "payload": {"id": "thread-a", "cwd": cwd, "timestamp": "2099-01-01T00:00:00Z"}
    });
    let user =
        serde_json::json!({"type": "response_item", "payload": {"role": "user", "text": body}});
    fs::write(&path, format!("{line}\n{user}\n")).expect("write");
    let probe = CodexRolloutProbe::open(&path);
    assert_eq!(probe.user_turns().expect("turns"), vec![body.to_owned()]);
    assert!(probe.matches_sent(body).expect("match"));
    assert!(!probe.matches_sent("other").expect("miss"));
}

fn load(rel: &str) -> Vec<Value> {
    serde_json::from_str(&fs::read_to_string(fixtures().join(rel)).expect(rel)).expect("json")
}

fn raw_fp(text: &str) -> String {
    let mut out = String::new();
    for byte in Sha256::digest(text.as_bytes()) {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
