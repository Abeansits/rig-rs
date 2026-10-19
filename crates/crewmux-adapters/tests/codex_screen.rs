//! Labelled Codex screen corpus.
#![expect(
    clippy::expect_used,
    reason = "a missing screen fixture fails the test"
)]
#![expect(clippy::panic, reason = "a mislabelled capture fails the test")]
#![expect(
    clippy::wildcard_enum_match_arm,
    reason = "a capture must be the labelled screen and nothing else"
)]

#[path = "codex_support.rs"]
mod support;

use std::fs;

use crewmux_adapters::CODEX_DIALOGS;
use crewmux_adapters::classify_runtime;
use crewmux_adapters::codex::{CodexScreen, classify};
use crewmux_adapters::dialog::recognise;
use crewmux_types::DialogId;
use proptest::prelude::*;
use serde_json::Value;

use support::fixtures;

#[test]
fn every_labelled_capture_matches() {
    let dir = fixtures().join("0.162.0");
    let labels = load_labels(&dir);
    let mut files = Vec::new();
    for entry in fs::read_dir(&dir).expect("dir") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("txt") {
            files.push(path);
        }
    }
    assert_eq!(files.len(), labels.len(), "every capture needs a sha row");
    for path in files {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("name");
        let label = labels
            .get(name)
            .unwrap_or_else(|| panic!("unlabelled {name}"));
        let text = fs::read_to_string(&path).expect(name);
        let sha = crewmux_transcript::fingerprint_raw(&text);
        assert_eq!(
            label.get("sha256").and_then(Value::as_str),
            Some(sha.as_str()),
            "{name} sha"
        );
        assert_screen(label["screen"].as_str().expect("screen"), &text, label);
    }
}

#[test]
fn midturn_is_busy_not_ready() {
    let text =
        fs::read_to_string(fixtures().join("0.162.0/codex-040-midturn.txt")).expect("midturn");
    assert!(matches!(classify(&text), CodexScreen::Busy));
    assert!(text.contains("esc to interrupt"));
    assert!(text.contains('◦'));
    assert!(text.contains("? for shortcuts"));
}

#[test]
fn finished_turn_reads_idle() {
    let text = fs::read_to_string(fixtures().join("0.162.0/codex-069-midturn.txt")).expect("done");
    assert!(text.contains("Worked for"));
    assert!(matches!(classify(&text), CodexScreen::Ready(_)));
}

#[test]
fn coloured_codex_097_is_ready_not_unknown() {
    let text =
        fs::read_to_string(fixtures().join("0.162.0/codex-097-seat-ready.txt")).expect("097");
    let coloured = text.replace(
        "› Ask Codex to do anything",
        "\u{1b}[1m›\u{1b}[22m \u{1b}[2mAsk Codex to do anything\u{1b}[22m",
    );
    assert_ne!(coloured, text, "fixture prompt was not coloured");
    match classify(&coloured) {
        CodexScreen::Ready(_) => {}
        other => panic!("codex-097 with a bold › and a dim placeholder was {other:?}"),
    }
    assert_eq!(
        classify_runtime(crewmux_types::RuntimeKind::Codex, &coloured),
        crewmux_adapters::Readiness::Ready
    );
}

#[test]
fn osc_8_on_a_codex_prompt_is_ready() {
    let bel = "\u{1b}]8;;https://example.test\u{7}› Ask Codex to do anything\u{1b}]8;;\u{7}\n  ? for shortcuts\n";
    let st = "\u{1b}]8;;https://example.test\u{1b}\\› Ask Codex to do anything\u{1b}]8;;\u{1b}\\\n  ? for shortcuts\n";
    for screen in [bel, st] {
        match classify(screen) {
            CodexScreen::Ready(_) => {}
            other => panic!("OSC 8 prompt was {other:?}"),
        }
    }
}

#[test]
fn a_coloured_typed_line_is_a_draft() {
    let text = "\u{1b}[1m›\u{1b}[22m hello\n  ? for shortcuts\n";
    match classify(text) {
        CodexScreen::Draft { visible } => assert_eq!(visible, "hello"),
        other => panic!("coloured draft was {other:?}"),
    }
}

#[test]
fn continuation_under_an_empty_prompt_is_a_draft() {
    let text = "› Ask Codex to do anything\nstill typing\n  GPT-6\n  ? for shortcuts\n";
    match classify(text) {
        CodexScreen::Draft { visible } => assert_eq!(visible, "still typing"),
        other => panic!("draft was {other:?}"),
    }
}

#[test]
fn model_footer_without_a_gpt_prefix_is_ready() {
    for footer in ["gpt-5-codex medium · ~/src", "o3 high · ~/src"] {
        let text = format!("› Ask Codex to do anything\n  {footer}\n  ? for shortcuts\n");
        assert!(
            matches!(classify(&text), CodexScreen::Ready(_)),
            "{footer} => {:?}",
            classify(&text)
        );
    }
}

#[test]
fn interrupt_in_scrollback_does_not_hide_a_ready_composer() {
    let text = "\
esc to interrupt
◦ Working
╭──╮
│ ›  │
╰──╯
  ? for shortcuts
";
    assert!(matches!(classify(text), CodexScreen::Ready(_)));
}

#[test]
fn shell_prompt_is_not_ready() {
    assert!(matches!(
        classify("codex exited\n$"),
        CodexScreen::ReturnedToShell
    ));
}

#[test]
fn draft_composer_is_not_ready() {
    let text = "╭──╮\n│ › hello │\n╰──╯\n  ? for shortcuts\n";
    match classify(text) {
        CodexScreen::Draft { visible } => assert_eq!(visible, "hello"),
        other => panic!("draft was {other:?}"),
    }
}

#[test]
fn unparsed_composer_is_not_ready() {
    let text = "╭──╮\n│ │\n╰──╯\n  ? for shortcuts\n";
    assert!(matches!(classify(text), CodexScreen::Inconclusive));
}

#[test]
fn text_above_an_empty_cursor_is_a_draft() {
    let text = "╭──╮\n│ line1│\n│ › │\n╰──╯\n  ? for shortcuts\n";
    match classify(text) {
        CodexScreen::Draft { visible } => assert_eq!(visible, "line1"),
        other => panic!("draft was {other:?}"),
    }
}

#[test]
fn text_in_the_box_without_a_cursor_is_a_draft() {
    let text = "╭──╮\n│ working │\n╰──╯\n  ? for shortcuts\n";
    match classify(text) {
        CodexScreen::Draft { visible } => assert_eq!(visible, "working"),
        other => panic!("draft was {other:?}"),
    }
}

#[test]
fn login_scrollback_does_not_hide_a_ready_composer() {
    let text = "\
Sign in with ChatGPT
Provide your own API key
╭──╮
│ ›  │
╰──╯
  ? for shortcuts
";
    assert!(matches!(classify(text), CodexScreen::Ready(_)));
}

#[test]
fn stale_scrollback_does_not_hide_a_ready_composer() {
    let text = "\
control socket is already in use
╭──╮
│ ›  │
╰──╯
  ? for shortcuts
";
    assert!(matches!(classify(text), CodexScreen::Ready(_)));
}

#[test]
fn hard_failure_scrollback_does_not_hide_a_ready_composer() {
    let text = "\
Error finding codex home config.toml: does not exist
╭──╮
│ ›  │
╰──╯
  ? for shortcuts
";
    assert!(matches!(classify(text), CodexScreen::Ready(_)));
}

#[test]
fn login_on_the_live_screen_stays_login() {
    let text = "\
╭──╮
│ ›  │
╰──╯
Sign in with ChatGPT
Provide your own API key
";
    assert!(matches!(classify(text), CodexScreen::Login(_)));
}

#[test]
fn stale_on_the_live_screen_stays_stale() {
    let text = "\
╭──╮
│ ›  │
╰──╯
control socket is already in use
";
    assert!(matches!(classify(text), CodexScreen::StaleServer));
}

#[test]
fn hard_failure_on_the_live_screen_stays_a_hard_failure() {
    let text = "\
╭──╮
│ ›  │
╰──╯
Error finding codex home config.toml: does not exist
";
    assert!(matches!(classify(text), CodexScreen::HardFailure { .. }));
}

#[test]
fn ready_composer_ignores_scrollback_update_banner() {
    let text = "\
Update available! 0.162.0 → 0.162.1
› 1. Update now (runs brew upgrade --cask codex)
  2. Skip
  3. Skip until next version
╭──╮
│ ›  │
╰──╯
  ? for shortcuts
";
    assert!(matches!(classify(text), CodexScreen::Ready(_)));
}

#[test]
fn update_title_inside_prose_is_not_a_dialog() {
    let text = "\
The notes mention Update available in the changelog.
› 1. Update now (runs brew upgrade --cask codex)
  2. Skip
  3. Skip until next version
";
    assert!(matches!(classify(text), CodexScreen::UnknownMenu { .. }));
}

#[test]
fn dialog_title_after_the_composer_stays_a_dialog() {
    let text = "\
╭──╮
│ ›  │
╰──╯
  ? for shortcuts
Update available! 0.162.0 → 0.162.1
› 1. Update now (runs `brew upgrade --cask codex`)
  2. Skip
  3. Skip until next version
";
    assert!(matches!(
        classify(text),
        CodexScreen::Dialog(DialogId::CodexUpdate)
    ));
}

#[test]
fn numbered_menu_is_not_ready() {
    let text = "Select a model\n› 1. gpt-5.1\n  2. gpt-5.1-codex\n";
    assert!(matches!(classify(text), CodexScreen::UnknownMenu { .. }));
}

#[test]
fn trust_prompt_is_not_ready() {
    let text = "Accessing workspace\n/tmp/work\n› 1. No, exit\n  2. Yes, I trust this folder\n";
    assert!(matches!(
        classify(text),
        CodexScreen::Dialog(DialogId::ClaudeTrust)
    ));
}

proptest::proptest! {
    #[test]
    fn interrupt_marker_is_never_ready(tail in "[a-z ]{0,24}") {
        let text = format!("esc to interrupt\n? for shortcuts\n{tail}");
        let screen = classify(&text);
        prop_assert!(matches!(screen, CodexScreen::Busy));
    }

    #[test]
    fn sign_in_menu_is_never_ready(pad in "[a-z]{0,12}") {
        let text = format!("{pad}\nSign in with ChatGPT\nProvide your own API key\n");
        let screen = classify(&text);
        prop_assert!(matches!(screen, CodexScreen::Login(_)));
    }
}

fn load_labels(dir: &std::path::Path) -> std::collections::BTreeMap<String, Value> {
    let mut labels = std::collections::BTreeMap::new();
    let text = fs::read_to_string(dir.join("labels.jsonl")).expect("labels");
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).expect("row");
        let file = value
            .get("file")
            .and_then(Value::as_str)
            .expect("file")
            .to_owned();
        labels.insert(file, value);
    }
    labels
}

fn label_str<'a>(label: &'a Value, key: &str) -> Option<&'a str> {
    label.get(key).and_then(Value::as_str)
}

fn assert_screen(kind: &str, text: &str, label: &Value) {
    match kind {
        "ready" => assert!(matches!(classify(text), CodexScreen::Ready(_))),
        "busy" => assert!(matches!(classify(text), CodexScreen::Busy)),
        "inconclusive" => assert!(matches!(classify(text), CodexScreen::Inconclusive)),
        "draft" => assert!(matches!(classify(text), CodexScreen::Draft { .. })),
        "stale" => assert!(matches!(classify(text), CodexScreen::StaleServer)),
        "background" => assert_dialog(text, DialogId::CodexBackgroundServer),
        "update" => assert_update(text, label),
        "mcp" => assert_mcp(text, label),
        other => panic!("unknown label {other}"),
    }
}

fn assert_dialog(text: &str, id: DialogId) {
    match classify(text) {
        CodexScreen::Dialog(found) if found == id => {}
        other => panic!("{id:?} was {other:?}"),
    }
}

fn assert_update(text: &str, label: &Value) {
    assert_dialog(text, DialogId::CodexUpdate);
    let found = recognise(CODEX_DIALOGS, text).expect("update");
    assert_eq!(found.params.from.as_deref(), label_str(label, "from"));
    assert_eq!(found.params.to.as_deref(), label_str(label, "to"));
}

fn assert_mcp(text: &str, label: &Value) {
    assert_dialog(text, DialogId::CodexMcpToolApproval);
    let found = recognise(CODEX_DIALOGS, text).expect("mcp");
    assert_eq!(found.params.server.as_deref(), label_str(label, "server"));
    assert_eq!(found.params.tool.as_deref(), label_str(label, "tool"));
}
