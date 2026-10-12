//! QA fixes for the Claude seat: draft, spinner, paste check, UUID, and $HOME.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a wrong classification or a temp-dir failure should fail the test"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crewmux_adapters::claude::{ClaudeScreen, LoginKind, classify_text};
use crewmux_adapters::{
    AgentAdapter, BudgetOverrides, ClaudeAdapter, Error, LaunchBudget, LaunchOpts, LaunchOutcome,
    Readiness, ResumeToken, SeatIo, SubmitMode,
};
use crewmux_mux::FakeMux;
use crewmux_types::{DialogId, ReadinessCode, SeatId};

#[path = "common/mod.rs"]
mod support;

const READY: &str = "\
╭────────────────────────────────────────╮
│ >                                      │
╰────────────────────────────────────────╯
  ? for shortcuts
";

const DRAFT: &str = "\
╭────────────────────────────────────────╮
│ > half typed draft                     │
╰────────────────────────────────────────╯
  ? for shortcuts
";

const MULTILINE_DRAFT: &str = "\
╭────────────────────────────────────────╮
│ > unfinished                           │
│   review draft                         │
╰────────────────────────────────────────╯
  ? for shortcuts
";

#[test]
fn draft_composer_is_attention() {
    let screen = classify_text(DRAFT);
    assert!(matches!(screen, ClaudeScreen::Draft), "{screen:?}");
    assert!(screen.requires_attention());
    assert!(!screen.is_ready());
    assert_eq!(screen.code(), Some(ReadinessCode::PromptDraft));
    assert_eq!(
        readiness_of(DRAFT),
        Readiness::Gate(ReadinessCode::PromptDraft)
    );
}

#[test]
fn multiline_draft_is_not_ready() {
    let screen = classify_text(MULTILINE_DRAFT);
    assert!(matches!(screen, ClaudeScreen::Draft), "{screen:?}");
    assert!(!screen.is_ready());
    let nested = "\
❯ unfinished review draft
  ────────────────────────────────────────────────────────────────────────────────
  ❯\u{a0}
  ? for shortcuts
";
    assert!(
        !classify_text(nested).is_ready(),
        "a continuation must not look like an empty prompt"
    );
}

#[test]
fn history_prompt_is_not_the_current_draft() {
    let border = "─".repeat(80);
    let bar = "⏵⏵ accept edits on (shift+tab to cycle) · ← for agents";
    let update = "✘ Auto-update failed: no write permission to npm prefix · Run claude doctor";
    let focus = "tmux focus-events off · add 'set -g focus-events on' to ~/.tmux.conf and re…";
    let screen = format!(
        "❯ previous user request\n● Completed response.\n\n{border}\n❯\u{a0}\n{border}\n  {bar}\n  {update}\n  {focus}\n"
    );
    assert!(classify_text(&screen).is_ready(), "{screen}");
}

#[test]
fn spinner_without_interrupt_is_busy() {
    for body in [
        "✶ Thinking… (6s · ↑ 284 tokens · thinking)",
        "✻ Onioning… (2m 38s · ↓ 10.9k tokens · thought for 8s)",
        "⠋ Processing",
        "Working on it…",
    ] {
        let screen = format!("{body}\n{READY}");
        let class = classify_text(&screen);
        assert!(matches!(class, ClaudeScreen::Busy), "{body} {class:?}");
        assert!(!class.is_ready());
    }
}

#[test]
fn hook_spinner_is_busy() {
    let screen = format!("✽ Sketching… (running PostToolUse hook · 3m 12s)\n{READY}");
    assert!(matches!(classify_text(&screen), ClaudeScreen::Busy));
}

#[test]
fn unframed_spinner_is_busy() {
    let screen = "✶ Thinking… (6s · ↑ 284 tokens · thinking)\n❯ \n? for shortcuts\n";
    assert!(matches!(classify_text(screen), ClaudeScreen::Busy));
}

#[test]
fn crunched_with_empty_shortcuts_composer_is_ready() {
    let screen = format!("✻ Crunched for 2s\n{READY}");
    assert!(classify_text(&screen).is_ready(), "{screen}");
}

#[test]
fn working_on_it_is_not_ready() {
    let screen = classify_text("Working on it…");
    assert!(matches!(screen, ClaudeScreen::Busy), "{screen:?}");
    assert!(!screen.is_ready());
}

#[test]
fn live_tui_with_draft_is_not_ready() {
    let screen = "\
Claude Code v2.1.89
❯ Working on a task.
────────────────────────────────────────────────────────────────────────────────
  ? for shortcuts                                             ● high · /effort
";
    let class = classify_text(screen);
    assert!(matches!(class, ClaudeScreen::Draft), "{class:?}");
    assert!(!class.is_ready());
}

#[test]
fn edit_approval_footer_is_ready() {
    let screen = "\
Loading startup skills and recovering identity.

────────────────────────────────────────────────────────────────────────────────
❯ 
────────────────────────────────────────────────────────────────────────────────
  ⏵⏵ accept edits on (shift+tab to cycle)                     ● high · /effort
";
    let class = classify_text(screen);
    assert!(class.is_ready(), "{class:?}");
}

#[test]
fn prose_composer_with_text_is_not_ready() {
    let screen = "\
Claude Code v2.1.89
❯ Compare Resume from summary with Resume full session as-is in the recovery notes.
────────────────────────────────────────────────────────────────────────────────
  ⏵⏵ accept edits on (shift+tab to cycle)                     ● high · /effort
";
    let class = classify_text(screen);
    assert!(class.requires_attention(), "{class:?}");
    assert!(!class.is_ready());
}

#[test]
fn active_tui_without_footer_is_not_ready() {
    let screen = "\
Claude Code v2.1.89

 ❯ accept edits on

[work in progress]
";
    assert!(!classify_text(screen).is_ready());
}

#[test]
fn shell_prompt_is_not_ready() {
    let screen = classify_text("user@example.test rigged %");
    assert!(matches!(screen, ClaudeScreen::Inconclusive), "{screen:?}");
    assert!(!screen.is_ready());
}

#[test]
fn trust_prompt_is_not_ready() {
    let screen = "\
Accessing workspace:
/some/workspace

Quick safety check: Is this a project you created or one you trust?
  ❯ No, exit
    Yes, I trust this folder
";
    let class = classify_text(screen);
    assert_eq!(class.code(), Some(ReadinessCode::TrustGate));
    assert!(!class.is_ready());
    let ClaudeScreen::Dialog(dialog) = classify_text(screen) else {
        panic!("trust");
    };
    assert!(matches!(dialog.id, DialogId::ClaudeTrust));
}

#[test]
fn mcp_checkbox_is_a_gate() {
    let screen = "\
────────────────────────────────────────────────────────────────────────────────
  2 new MCP servers found in .mcp.json
  Select any you wish to enable.

  ❯ [✔] exa
    [✔] context7
 Space to select · Enter to confirm · Esc to reject all
";
    let class = classify_text(screen);
    assert_eq!(class.code(), Some(ReadinessCode::UnknownMenu), "{class:?}");
    assert!(class.requires_attention());
    assert!(!class.is_ready());
}

#[test]
fn login_footer_is_not_ready() {
    let screen = "\
Claude Code v2.1.101
────────────────────────────────────────────────────────────────────────────────
❯ 
────────────────────────────────────────────────────────────────────────────────
                                                    Not logged in · Run /login
";
    let class = classify_text(screen);
    assert_eq!(class, ClaudeScreen::Login(LoginKind::LoginMethod));
    assert_eq!(class.code(), Some(ReadinessCode::LoginRequired));
    assert!(!class.is_ready());
}

#[test]
fn no_conversation_is_attention() {
    let screen =
        classify_text("No conversation found with session ID: abc123\nuser@example.test %");
    assert!(
        matches!(screen, ClaudeScreen::UnknownMenu { .. }),
        "{screen:?}"
    );
    assert!(screen.requires_attention());
    assert!(!screen.is_ready());
}

#[test]
fn resume_selection_is_attention() {
    let numbered = "\
Choose a conversation to resume:

  1. project-foo (modified 2h ago)
  2. project-bar (modified yesterday)
  3. project-baz (modified last week)

Enter a number, or press q to cancel.
";
    let arrow = "\
Choose the conversation to resume:

› 1. recent project
  2. older project
";
    for screen in [numbered, arrow] {
        let class = classify_text(screen);
        assert!(
            matches!(class, ClaudeScreen::UnknownMenu { .. }),
            "{class:?}"
        );
        assert!(class.requires_attention());
        assert!(!class.is_ready());
    }
}

#[test]
fn resume_mode_prompt_is_attention() {
    let screen = "\
How would you like to resume?

❯ Resume from summary
  Resume full session as-is
";
    let class = classify_text(screen);
    assert!(
        matches!(class, ClaudeScreen::UnknownMenu { .. }),
        "{class:?}"
    );
    assert!(class.requires_attention());
}

#[test]
fn prose_is_not_a_resume_menu() {
    for screen in [
        "The recovery notes recommend Resume from summary when context is stale.",
        "The docs say: 'Choose a conversation to focus on.'",
    ] {
        let class = classify_text(screen);
        assert!(!class.requires_attention(), "{screen} {class:?}");
        assert!(!class.is_ready());
    }
}

#[test]
fn prose_does_not_open_a_dialog() {
    let screen = "\
Remember: never pick Bypass Permissions by habit.

❯ 1. No, exit
  2. Yes, I accept
";
    let class = classify_text(screen);
    assert!(
        matches!(class, ClaudeScreen::UnknownMenu { .. }),
        "{class:?}"
    );
    assert!(!matches!(
        class,
        ClaudeScreen::Dialog(ref dialog) if dialog.id == DialogId::ClaudeBypassConsent
    ));
}

#[test]
fn borderless_prompt_is_not_ready() {
    let screen = "● Ready.\n❯ \n  ? for shortcuts\n";
    assert!(!classify_text(screen).is_ready());
}

#[test]
fn indented_empty_composer_stays_ready() {
    let indented = READY
        .lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(classify_text(&indented).is_ready(), "{indented}");
}

#[test]
fn draft_and_warning_are_not_ready() {
    let screens = [
        DRAFT,
        "tmux focus-events off · add 'set -g focus-events on'\n",
        "❯ unfinished message\n────────────────────────────────────────────────────────────────────────────────\n  ⏵⏵ accept edits on (shift+tab to cycle)\n",
        "Do you want to proceed?\n❯ 1. Yes\n  2. No\n",
        "✶ Thinking… (6s · ↑ 284 tokens · thinking)\n❯ \n",
        "esc to interrupt\n",
        "❯ \nunfinished second line\n",
        "⏵⏵ accept edits on (shift+tab to cycle)\n✘ Auto-update failed\n",
        "Not logged in · Run /login\n",
    ];
    for screen in screens {
        assert!(!classify_text(screen).is_ready(), "{screen}");
    }
}

#[test]
fn question_menu_is_attention() {
    let class = classify_text("Do you want to proceed?\n❯ 1. Yes\n  2. No\n");
    assert!(class.requires_attention(), "{class:?}");
    assert!(!class.is_ready());
}

fn readiness_of(text: &str) -> Readiness {
    crewmux_adapters::claude::readiness(&classify_text(text))
}

#[tokio::test]
async fn submit_refuses_a_draft() {
    let (adapter, mux, binding, root) = ready_adapter("draft", DRAFT).await;
    let io = adapter.io_for(&binding).await.expect("io");
    let err = io
        .submit("hello", SubmitMode::Enter)
        .await
        .expect_err("draft");
    assert!(matches!(err, Error::NotReady));
    assert!(!support::sent_paste(&mux.recorded().await));
    assert!(!support::sent_enter(&mux.recorded().await));
    drop(std::fs::remove_dir_all(&root));
}

#[tokio::test]
async fn submit_refuses_while_spinning() {
    let screen = format!("✶ Thinking… (6s)\n{READY}");
    let (adapter, mux, binding, root) = ready_adapter("spin", &screen).await;
    let io = adapter.io_for(&binding).await.expect("io");
    let err = io
        .submit("hello", SubmitMode::Enter)
        .await
        .expect_err("busy");
    assert!(matches!(err, Error::NotReady));
    assert!(!support::sent_enter(&mux.recorded().await));
    drop(std::fs::remove_dir_all(&root));
}

#[tokio::test]
async fn paste_mismatch_sends_no_enter() {
    let mux = Arc::new(FakeMux::new());
    mux.replay_screens(vec![support::capture(READY)]).await;
    let root = scratch("mismatch");
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let adapter = ClaudeAdapter::new(support::shared(&mux), root.join("seats"), root.join("home"));
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
    std::fs::create_dir_all(adapter.seat_dir(&binding.seat)).expect("seat");
    std::fs::write(
        adapter.seat_dir(&binding.seat).join("claude-session-id"),
        "11111111-1111-4111-8111-111111111111",
    )
    .expect("id");
    let io = adapter.io_for(&binding).await.expect("io");
    let err = io
        .submit("hello", SubmitMode::Enter)
        .await
        .expect_err("mismatch");
    assert!(matches!(err, Error::PasteMismatch), "{err}");
    let calls = mux.recorded().await;
    assert!(support::sent_paste(&calls), "{calls:?}");
    assert!(calls.iter().flatten().any(|arg| arg == "C-u"), "{calls:?}");
    assert!(!support::sent_enter(&calls), "{calls:?}");
    drop(std::fs::remove_dir_all(&root));
}

#[tokio::test]
async fn resume_rejects_a_non_uuid_token() {
    let root = scratch("uuid");
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let mux = Arc::new(FakeMux::new());
    let adapter = ClaudeAdapter::new(support::shared(&mux), root.join("seats"), root.join("home"));
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
    let err = adapter
        .launch(
            &binding,
            opts(Some(ResumeToken {
                kind: "claude".to_owned(),
                value: "tok; echo injected".to_owned(),
            })),
        )
        .await
        .expect_err("uuid");
    assert!(err.to_string().contains("UUID"), "{err}");
    assert_eq!(mux.recorded().await, Vec::<Vec<String>>::new());
    drop(std::fs::remove_dir_all(&root));
}

#[tokio::test]
async fn resume_stores_the_canonical_uuid() {
    let canonical = "11111111-1111-4111-8111-111111111111";
    let forms = [
        "{11111111-1111-4111-8111-111111111111}",
        "urn:uuid:11111111-1111-4111-8111-111111111111",
        "11111111111141118111111111111111",
    ];
    for (index, form) in forms.iter().enumerate() {
        let root = scratch(&format!("uuid-form-{index}"));
        let cwd = root.join("work");
        std::fs::create_dir_all(&cwd).expect("cwd");
        let mux = Arc::new(FakeMux::new());
        mux.set_capture(support::capture(READY)).await;
        let adapter =
            ClaudeAdapter::new(support::shared(&mux), root.join("seats"), root.join("home"));
        let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
        drop(
            adapter
                .launch(
                    &binding,
                    opts(Some(ResumeToken {
                        kind: "claude".to_owned(),
                        value: (*form).to_owned(),
                    })),
                )
                .await
                .expect("launch"),
        );
        let stored =
            std::fs::read_to_string(adapter.seat_dir(&binding.seat).join("claude-session-id"))
                .expect("id");
        assert_eq!(stored, canonical, "{form}");
        drop(std::fs::remove_dir_all(&root));
    }
}

#[tokio::test]
async fn resume_emits_model() {
    let root = scratch("model");
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let mux = Arc::new(FakeMux::new());
    mux.set_capture(support::capture(READY)).await;
    let adapter = ClaudeAdapter::new(support::shared(&mux), root.join("seats"), root.join("home"));
    let mut binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
    binding.model = Some("haiku".to_owned());
    drop(
        adapter
            .launch(
                &binding,
                opts(Some(ResumeToken {
                    kind: "claude".to_owned(),
                    value: "11111111-1111-4111-8111-111111111111".to_owned(),
                })),
            )
            .await
            .expect("launch"),
    );
    let flat: Vec<String> = mux.recorded().await.into_iter().flatten().collect();
    assert!(flat.iter().any(|arg| arg == "--resume"), "{flat:?}");
    assert!(flat.iter().any(|arg| arg == "--model"), "{flat:?}");
    assert!(flat.iter().any(|arg| arg == "haiku"), "{flat:?}");
    drop(std::fs::remove_dir_all(&root));
}

#[tokio::test]
async fn home_cwd_does_not_launch() {
    let root = scratch("home");
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("home");
    let mux = Arc::new(FakeMux::new());
    let adapter = ClaudeAdapter::new(support::shared(&mux), root.join("seats"), home.clone());
    let binding = crewmux_adapters::binding_for(SeatId::new(), home, "seat__rig");
    let outcome = adapter.launch(&binding, opts(None)).await.expect("outcome");
    assert!(
        matches!(outcome, LaunchOutcome::Failed { .. }),
        "{outcome:?}"
    );
    assert_eq!(mux.recorded().await, Vec::<Vec<String>>::new());
    drop(std::fs::remove_dir_all(&root));
}

async fn ready_adapter(
    label: &str,
    screen: &str,
) -> (
    ClaudeAdapter,
    Arc<FakeMux>,
    crewmux_adapters::NodeBinding,
    PathBuf,
) {
    let mux = Arc::new(FakeMux::new());
    mux.set_capture(support::capture(screen)).await;
    let root = scratch(label);
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let adapter = ClaudeAdapter::new(support::shared(&mux), root.join("seats"), root.join("home"));
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
    std::fs::create_dir_all(adapter.seat_dir(&binding.seat)).expect("seat");
    std::fs::write(
        adapter.seat_dir(&binding.seat).join("claude-session-id"),
        "11111111-1111-4111-8111-111111111111",
    )
    .expect("id");
    (adapter, mux, binding, root)
}

fn opts(resume: Option<ResumeToken>) -> LaunchOpts {
    LaunchOpts {
        session_name: "seat__rig".to_owned(),
        env: BTreeMap::new(),
        resume,
        fork: false,
        auto_trust: false,
        budget: Some(LaunchBudget::with_overrides(BudgetOverrides {
            first_screen: Some(Duration::from_millis(20)),
            ready_after_last_gate: Some(Duration::from_millis(20)),
            inconclusive_stable: Some(Duration::from_millis(20)),
            overall: Some(Duration::from_millis(80)),
            poll: Some(Duration::from_millis(5)),
        })),
        claude_bin: None,
    }
}

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("rig-qa-{label}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&root));
    std::fs::create_dir_all(&root).expect("scratch");
    root
}
