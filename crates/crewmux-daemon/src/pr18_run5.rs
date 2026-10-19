//! Send-path paste check and captured `rig compact` JSON from runs 2–4.
//!
//! The panes are the same sanitized set the adapter tests classify. `classify_for`
//! is the send matcher. The JSON is what those runs printed; e04–e06 are the
//! pre-fix `send_failed` captures and are not what a refused compact returns now.

use std::path::PathBuf;

use crewmux_health::compaction::{AttemptState, EnforcerSkipReason, ManualCompactionStage};
use crewmux_messaging::PasteDisplay;
use crewmux_transcript::unwrap_outermost;
use serde_json::Value;

use crate::paste_check::{PasteHit, classify_for};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/claude/2.1.287/pr18-run5")
}

fn read(name: &str) -> String {
    let path = dir().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

#[test]
fn send_path_accepts_the_same_paste_markers() {
    let plus11 = read("a02p-paste-payload-12-lines.txt");
    expect_clean("a01-paste-plus11-lines.txt", &plus11);
    expect_clean("a02-paste-plus11-lines-1800ms.txt", &plus11);
    let literal = read("a03p-paste-payload-1-line.txt");
    expect_clean("a03-paste-single-line-literal.txt", &literal);
    let bare = pasted_inner("g-run3-L005-prep.jsonl");
    expect_clean("a04-paste-bare-no-linecount-run2.txt", &bare);
    expect_clean("a05-paste-bare-no-linecount-run4.txt", &bare);
    let plus5 = pasted_inner("g-run4A-L005-send-envelope.jsonl");
    expect_clean("a06-paste-envelope-plus5-lines.txt", &plus5);
    expect_clean("a07-paste-envelope-plus5-lines-run3.txt", &plus5);
    let plus10 = pasted_inner("g-run4A-L060-restore.jsonl");
    expect_clean("c01-restore-pasted-plus10-lines.txt", &plus10);
    let screen = read("a06-paste-envelope-plus5-lines.txt");
    assert_ne!(
        classify_for(PasteDisplay::Claude, "", &screen, "not the envelope"),
        PasteHit::Clean
    );
}

#[test]
fn captured_compact_json_parses_to_the_recorded_stage() {
    let preparing = stage(
        "e01-compact-preparing.rig.json",
        ManualCompactionStage::Preparing,
    );
    assert_eq!(preparing["compact_sent"], false);
    let first = &preparing["attempts"][0];
    assert_eq!(
        first["state"].as_str(),
        Some(ManualCompactionStage::Preparing.as_str())
    );
    assert!(first["attempt_id"].as_str().unwrap_or("").ends_with("-1"));

    let done = stage("e02-compact-done.status.json", ManualCompactionStage::Done);
    let row = &done["attempts"][0];
    assert_eq!(row["state"].as_str(), Some(AttemptState::Done.label()));
    assert_eq!(row["confirmed"], true);
    assert_eq!(row["summary_len"], 4078);

    let restoring = stage(
        "e03-compact-restoring.status.json",
        ManualCompactionStage::Restoring,
    );
    assert_eq!(
        restoring["attempts"][0]["state"].as_str(),
        Some(
            AttemptState::Completed {
                summary_sha: [0; 32]
            }
            .label()
        )
    );
    assert_eq!(restoring["attempts"][0]["confirmed"], true);

    let failed = stage(
        "e07-compact-failed.status.json",
        ManualCompactionStage::Failed,
    );
    assert_eq!(
        failed["attempts"][0]["state"].as_str(),
        Some(AttemptState::Done.label())
    );
    let empty = stage(
        "e08-compact-failed-no-attempts-run2.status.json",
        ManualCompactionStage::Failed,
    );
    assert_eq!(empty["attempts"].as_array().map(Vec::len), Some(0));
}

#[test]
fn captured_send_failed_status_is_the_pre_fix_hang() {
    let refused = stage(
        "e04-compact-send-failed.rig.json",
        ManualCompactionStage::Preparing,
    );
    assert_eq!(
        refused["reason"].as_str(),
        Some(EnforcerSkipReason::SendFailed.as_str())
    );
    assert_eq!(
        refused["attempts"][0]["state"].as_str(),
        Some(AttemptState::Done.label())
    );
    assert!(
        refused["attempts"][0]["attempt_id"]
            .as_str()
            .unwrap_or("")
            .ends_with("-4")
    );

    let stuck = stage(
        "e05-compact-after-send-failed.status.json",
        ManualCompactionStage::Preparing,
    );
    assert!(stuck.get("reason").is_none());
    assert_eq!(stuck["attempts"].as_array().map(Vec::len), Some(1));

    let pending = stage(
        "e06-compact-preparation-pending.rig.json",
        ManualCompactionStage::WaitingMap,
    );
    assert_eq!(
        pending["reason"].as_str(),
        Some(EnforcerSkipReason::PreparationPending.as_str())
    );
}

#[test]
fn captured_compact_errors_use_the_real_tokens() {
    let mismatch = read("e09-compact-paste-mismatch-run2.rig.txt");
    assert_eq!(
        mismatch,
        format!("{}\n", crewmux_adapters::Error::PasteMismatch)
    );
    let busy = read("e10-compact-already-in-progress.rig.txt");
    assert_eq!(
        busy,
        format!("{}\n", EnforcerSkipReason::AlreadyInProgress.as_str())
    );
}

fn expect_clean(name: &str, payload: &str) {
    let screen = read(name);
    assert_eq!(
        classify_for(PasteDisplay::Claude, "", &screen, payload),
        PasteHit::Clean,
        "{name}"
    );
}

fn stage(name: &str, want: ManualCompactionStage) -> Value {
    let value: Value =
        serde_json::from_str(&read(name)).unwrap_or_else(|err| panic!("{name}: {err}"));
    assert_eq!(value["stage"].as_str(), Some(want.as_str()), "{name}");
    value
}

fn pasted_inner(name: &str) -> String {
    let value: Value =
        serde_json::from_str(&read(name)).unwrap_or_else(|err| panic!("{name}: {err}"));
    let content = value
        .pointer("/message/content")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{name} content"));
    unwrap_outermost(content).unwrap_or_else(|| panic!("{name} wrapper"))
}
