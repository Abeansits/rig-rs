//! Real idle composers, spinners, and one send into an accept-edits footer.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a wrong classification or a temp-dir failure should fail the test"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crewmux_adapters::claude::{ClaudeScreen, classify_text};
use crewmux_adapters::{ClaudeAdapter, SeatIo, SubmitMode};
use crewmux_mux::FakeMux;
use crewmux_types::{RuntimeKind, SeatId};

#[path = "common/mod.rs"]
mod support;

const BORDER: &str =
    "────────────────────────────────────────────────────────────────────────────────";
const BAR: &str = "⏵⏵ accept edits on (shift+tab to cycle) · ← for agents";

fn pane(before: &str, composer: &str, hint: &str, trailers: &[&str]) -> String {
    let mut lines = vec![
        before.to_owned(),
        String::new(),
        BORDER.to_owned(),
        composer.to_owned(),
        BORDER.to_owned(),
        format!("  {hint}"),
    ];
    for trailer in trailers {
        lines.push(format!("  {trailer}"));
    }
    lines.push(String::new());
    lines.join("\n")
}

#[test]
fn empty_accept_edits_composers_are_ready() {
    let update = "✘ Auto-update failed: no write permission to npm prefix · Run claude doctor";
    let focus = "tmux focus-events off · add 'set -g focus-events on' to ~/.tmux.conf and re…";
    let weekly = "You've used 96% of your weekly limit · resets 12pm (UTC)";
    let hints = [
        BAR,
        "⏵⏵ bypass permissions on (shift+tab to cycle)",
        "⏸ plan mode on (shift+tab to cycle)",
        "? for shortcuts",
        "custom mode hint",
        "⏵⏵ bypass permissions on · 1 shell · ← for agents",
        "⏵⏵ accept edits on · 1 shell · ← for agents",
        "⏵⏵ auto mode on (shift+tab to cycle)",
        "-- INSERT -- ⏵⏵ auto mode on (shift+tab to cycle)",
    ];
    let warnings: &[&[&str]] = &[&[], &[update], &[update, focus], &[update, weekly]];
    for hint in hints {
        for trailers in warnings {
            let onion = "✻ Onioning… (2m 38s · ↓ 10.9k tokens · thought for 8s)\n● Ready.";
            for before in [
                "● Ready.",
                "✻ Crunched for 2s",
                onion,
                "✻ Baked for 1m 24s",
                "✻ Cooked for 9s",
                "✻ Worked for 2s",
                "✻ Brewed for 1s",
                "✻ Sautéed for 1m 24s",
                "✻ Churned for 2s",
            ] {
                let screen = pane(before, "❯\u{a0}", hint, trailers);
                let class = classify_text(&screen);
                assert!(class.is_ready(), "{hint} {before} {class:?}");
            }
        }
    }
}

#[test]
fn two_history_prompts_leave_the_composer_ready() {
    let screen = pane(
        "❯ first request\n❯ second request\n● Completed response.",
        "❯\u{a0}",
        BAR,
        &[],
    );
    let class = classify_text(&screen);
    assert!(class.is_ready(), "{class:?}");
    assert!(!matches!(class, ClaudeScreen::UnknownMenu { .. }));
}

#[test]
fn bare_angle_above_a_draft_is_not_ready() {
    let screen = pane("> previous shell command", "❯ actual draft", BAR, &[]);
    let class = classify_text(&screen);
    assert!(matches!(class, ClaudeScreen::Draft), "{class:?}");
    assert!(!class.is_ready());
    let idle = pane("> previous shell command", "❯\u{a0}", BAR, &[]);
    assert!(classify_text(&idle).is_ready(), "{idle}");
}

#[test]
fn stale_trust_above_the_composer_is_not_the_dialog() {
    let screen = format!(
        "\
Accessing workspace:
/some/workspace

Quick safety check: Is this a project you created or one you trust?
  ❯ No, exit
    Yes, I trust this folder

{BORDER}
❯\u{a0}
{BORDER}
  {BAR}
"
    );
    let class = classify_text(&screen);
    assert!(class.is_ready(), "{class:?}");
    assert!(!matches!(class, ClaudeScreen::Dialog(_)));
}

#[test]
fn leading_spinner_glyphs_are_busy() {
    for body in [
        "✢ Cogitating…",
        "✻ Thinking…",
        "✳ Working…",
        "· Running PreToolUse hook…",
    ] {
        let screen = pane(body, "❯\u{a0}", BAR, &[]);
        let class = classify_text(&screen);
        assert!(matches!(class, ClaudeScreen::Busy), "{body} {class:?}");
    }
    let done = pane(
        "✻ Crunched for 2s · done",
        "❯\u{a0}",
        "● high · /effort",
        &[],
    );
    assert!(
        classify_text(&done).is_ready(),
        "a mid-line dot and a done marker are not spinners"
    );
}

#[tokio::test]
async fn explicit_wait_sends_once() {
    let hints = [
        "⏵⏵ accept edits on (shift+tab to cycle) · ← for agents",
        "⏵⏵ bypass permissions on (shift+tab to cycle)",
        "⏸ plan mode on (shift+tab to cycle)",
        "? for shortcuts",
    ];
    for (index, hint) in hints.iter().enumerate() {
        let screen = pane("● Ready.\n\n✻ Crunched for 2s", "❯\u{a0}", hint, &[]);
        let root = scratch(&format!("send-{index}"));
        let cwd = root.join("work");
        std::fs::create_dir_all(&cwd).expect("cwd");
        let mux = Arc::new(FakeMux::new());
        mux.set_capture(support::capture(&screen)).await;
        let adapter =
            ClaudeAdapter::new(support::shared(&mux), root.join("seats"), root.join("home"))
                .with_source(Arc::new(MemorySource {
                    text: "ping".to_owned(),
                }))
                .with_submit_wait(Duration::from_millis(200));
        let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
        std::fs::create_dir_all(adapter.seat_dir(&binding.seat)).expect("dir");
        std::fs::write(
            adapter.seat_dir(&binding.seat).join("claude-session-id"),
            "11111111-1111-4111-8111-111111111111",
        )
        .expect("id");
        let io = adapter.io_for(&binding).await.expect("io");
        let receipt = io.submit("ping", SubmitMode::Enter).await.expect("submit");
        assert_eq!(receipt.bytes, 4);
        let enters = mux
            .recorded()
            .await
            .iter()
            .flatten()
            .filter(|arg| *arg == "Enter")
            .count();
        assert_eq!(enters, 1, "{hint}");
        drop(std::fs::remove_dir_all(&root));
    }
}

#[test]
fn draft_continuations_are_not_ready() {
    let update = "✘ Auto-update failed: no write permission to npm prefix · Run claude doctor";
    let focus = "tmux focus-events off · add 'set -g focus-events on' to ~/.tmux.conf and re…";
    let weekly = "You've used 96% of your weekly limit · resets 12pm (UTC)";
    let before = "● Ready.\n\n✻ Crunched for 2s";
    let nested = [
        format!("❯ unfinished review draft\n  {BORDER}\n  ❯\u{a0}"),
        "❯ unfinished review draft\n  ───\n  ❯\u{a0}".to_owned(),
    ];
    let trailers: [&[&str]; 2] = [&[update, focus], &[update, weekly]];
    for (composer, trailer) in nested.iter().zip(trailers) {
        let screen = pane(before, composer, BAR, trailer);
        let class = classify_text(&screen);
        assert!(matches!(class, ClaudeScreen::Draft), "{class:?}\n{screen}");
    }
    let path = support::fixtures().join("drafts/continuation-under-empty-prompt.txt");
    let screen = support::read(&path);
    let class = classify_text(&screen);
    assert!(matches!(class, ClaudeScreen::Draft), "{class:?}\n{screen}");
}

#[test]
fn completed_work_with_suffix_is_ready() {
    let update = "✘ Auto-update failed: no write permission to npm prefix · Run claude doctor";
    let focus = "tmux focus-events off · add 'set -g focus-events on' to ~/.tmux.conf and re…";
    let weekly = "You've used 96% of your weekly limit · resets 12pm (UTC)";
    let onion = "✻ Onioning… (2m 38s · ↓ 10.9k tokens · thought for 8s)";
    let thinking = "✶ Thinking… (6s · ↑ 284 tokens · thinking)";
    let befores = [
        "● Ready.".to_owned(),
        "✻ Crunched for 2s".to_owned(),
        format!("{onion}\n● Completed response.\n✻ Crunched for 2s"),
        format!("{thinking}\n● Ready."),
    ];
    let suffixes: &[&[&str]] = &[&[], &[update], &[update, focus], &[update, weekly]];
    for before in &befores {
        for suffix in suffixes {
            let screen = pane(before, "❯\u{a0}", BAR, suffix);
            let class = classify_text(&screen);
            assert!(class.is_ready(), "{before} {class:?}");
        }
    }
}

#[test]
fn walk_pane_completed_turn_is_not_busy() {
    let hashes = [
        (
            "pane-coalesced-pieces-2-and-3.txt",
            "592589409e1183c8169df140d7dc17996449fd812e563b72392e6018d3730338",
        ),
        (
            "pane-single-piece-2.txt",
            "e445eb4f521e3719f068eab4187b24ee7f8e5a05e85e293cfce28ef1d1ca43c5",
        ),
    ];
    for (name, sha) in hashes {
        let text = support::read(&support::fixtures().join("walk").join(name));
        assert_eq!(crewmux_transcript::fingerprint_raw(&text), sha, "{name}");
        assert!(text.contains("✻ Baked for 1m 24s"), "{name}");
        let class = classify_text(&text);
        assert!(matches!(class, ClaudeScreen::Draft), "{name} {class:?}");
        let Some((header, _)) = text.split_once('❯') else {
            panic!("prompt {name}");
        };
        let empty =
            format!("{header}❯\u{a0}\n{BORDER}\n  ⏵⏵ bypass permissions on (shift+tab to cycle)\n");
        assert!(
            classify_text(&empty).is_ready(),
            "{name} empty composer\n{empty}"
        );
    }
}

#[tokio::test]
async fn second_submit_after_a_completed_turn() {
    for (index, verb) in [
        "Baked", "Cooked", "Worked", "Brewed", "Sautéed", "Churned", "Crunched",
    ]
    .iter()
    .enumerate()
    {
        submit_twice(index, verb).await;
    }
}

async fn submit_twice(index: usize, verb: &str) {
    let done = pane(&format!("✻ {verb} for 1m 24s"), "❯\u{a0}", BAR, &[]);
    assert!(classify_text(&done).is_ready(), "{verb} {done}");
    let root = scratch(&format!("again-{index}"));
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let mux = Arc::new(FakeMux::new());
    let first = pane("● Ready.", "❯\u{a0}", BAR, &[]);
    mux.set_capture(support::capture(&first)).await;
    let adapter = ClaudeAdapter::new(support::shared(&mux), root.join("seats"), root.join("home"))
        .with_source(Arc::new(MemorySource {
            text: "ping".to_owned(),
        }))
        .with_submit_wait(Duration::from_millis(200));
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
    std::fs::create_dir_all(adapter.seat_dir(&binding.seat)).expect("dir");
    std::fs::write(
        adapter.seat_dir(&binding.seat).join("claude-session-id"),
        "11111111-1111-4111-8111-111111111111",
    )
    .expect("id");
    let io = adapter.io_for(&binding).await.expect("io");
    io.submit("ping", SubmitMode::Enter).await.expect("first");
    mux.set_capture(support::capture(&done)).await;
    io.submit("ping", SubmitMode::Enter)
        .await
        .unwrap_or_else(|err| panic!("second {verb}: {err}"));
    let enters = mux
        .recorded()
        .await
        .iter()
        .flatten()
        .filter(|arg| *arg == "Enter")
        .count();
    assert_eq!(enters, 2, "{verb}");
    drop(std::fs::remove_dir_all(&root));
}

struct MemorySource {
    text: String,
}

impl crewmux_transcript::TranscriptSource for MemorySource {
    fn runtime(&self) -> RuntimeKind {
        RuntimeKind::ClaudeCode
    }

    fn cursor(&self) -> Result<crewmux_transcript::TranscriptCursor, crewmux_transcript::Error> {
        Ok(crewmux_transcript::TranscriptCursor(0))
    }

    fn user_turns_after(
        &self,
        _after: crewmux_transcript::TranscriptCursor,
    ) -> Result<Vec<crewmux_transcript::UserTurn>, crewmux_transcript::Error> {
        Ok(vec![crewmux_transcript::UserTurn {
            text: self.text.clone(),
            at: crewmux_transcript::TranscriptCursor(0),
        }])
    }
}

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("rig-idle-{label}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&root));
    std::fs::create_dir_all(&root).expect("scratch");
    root
}
