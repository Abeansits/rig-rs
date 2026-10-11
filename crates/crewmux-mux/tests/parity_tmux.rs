//! Ported `OpenRig` tmux cases that do not need a live server.
//!
//! Each test names the TypeScript case it stands in for. Live tmux behaviour
//! stays in `tmux_it` behind the `tmux-it` feature.

use std::path::PathBuf;

use crewmux_mux::{FakeMux, Key, Multiplexer, PasteOpts, SessionSpec};

fn spec(name: &str) -> SessionSpec {
    let mut spec = SessionSpec::new(name, PathBuf::from("/home/user/my project/code"));
    spec.env.insert("PATH".to_owned(), "/a b:/c".to_owned());
    spec.env.insert("EMPTY".to_owned(), String::new());
    spec
}

fn find_argv<'a>(calls: &'a [Vec<String>], command: &str) -> &'a [String] {
    match calls
        .iter()
        .find(|argv| argv.iter().any(|arg| arg == command))
    {
        Some(argv) => argv.as_slice(),
        None => &[],
    }
}

/// ts: packages/daemon/test/tmux-argv-exec.test.ts > "createSession passes cwd/env as verbatim argv units"
#[tokio::test]
async fn create_session_passes_cwd_and_env_as_argv_units() {
    let mux = FakeMux::new();
    mux.create_session(&spec("r01-dev1-impl"))
        .await
        .expect("create");
    let calls = mux.recorded().await;
    let argv = find_argv(&calls, "new-session");
    assert!(
        argv.windows(2)
            .any(|pair| pair == ["-c", "/home/user/my project/code"])
    );
    assert!(argv.windows(2).any(|pair| pair == ["-e", "EMPTY="]));
    assert!(argv.windows(2).any(|pair| pair == ["-e", "PATH=/a b:/c"]));
    assert!(!argv.iter().any(|arg| arg == "sh" || arg == "bash"));
}

/// ts: packages/daemon/test/tmux-argv-exec.test.ts > "sendKeys passes key names as separate verbatim units"
#[tokio::test]
async fn send_keys_are_separate_argv_units() {
    let mux = FakeMux::new();
    let pane = mux.create_session(&spec("dev")).await.expect("create");
    mux.send_keys(
        &pane,
        &[Key::Literal("Enter; rm -rf /".to_owned()), Key::CtrlC],
    )
    .await
    .expect("keys");
    let calls = mux.recorded().await;
    let literal = calls
        .iter()
        .find(|argv| argv.iter().any(|arg| arg == "Enter; rm -rf /"))
        .expect("literal");
    assert!(literal.windows(2).any(|pair| pair == ["send-keys", "-l"]));
    let named = calls
        .iter()
        .find(|argv| argv.iter().any(|arg| arg == "C-c"))
        .expect("named");
    assert!(named.iter().any(|arg| arg == "send-keys"));
    assert_ne!(named.iter().position(|arg| arg == "C-c"), Some(0));
}

/// ts: packages/daemon/test/tmux-argv-exec.test.ts > "sendText still stages through buffer files on the argv path"
/// ts: packages/daemon/test/tmux-adapter.test.ts > "writes a temp file via fs and pastes it; payload never in any exec command"
#[tokio::test]
async fn send_text_stages_through_a_buffer_file() {
    let mux = FakeMux::new();
    let pane = mux.create_session(&spec("dev")).await.expect("create");
    let receipt = mux
        .paste(&pane, "hello\nworld", PasteOpts::default())
        .await
        .expect("paste");
    assert!(receipt.buffer.starts_with("rig-"));
    let calls = mux.recorded().await;
    let load = find_argv(&calls, "load-buffer");
    let paste = find_argv(&calls, "paste-buffer");
    let buffer = receipt.buffer.as_str();
    assert!(load.windows(2).any(|pair| pair == ["-b", buffer]));
    assert!(paste.windows(2).any(|pair| pair == ["-d", "-p"]));
    assert!(paste.iter().any(|arg| arg == "-r"));
    assert!(
        !calls
            .iter()
            .flatten()
            .any(|arg| arg == "hello\nworld" || arg == "Enter")
    );
}

/// ts: packages/daemon/test/tmux-exact-target.test.ts > "a capture of a session that has ended is missing, not its prefix sibling's screen"
#[tokio::test]
async fn exact_target_does_not_select_a_prefix_sibling() {
    let mux = FakeMux::new();
    mux.create_session(&spec("worker-long"))
        .await
        .expect("sibling");
    assert!(!mux.has_session("worker").await.expect("prefix"));
    assert!(mux.has_session("worker-long").await.expect("exact"));
    let calls = mux.recorded().await;
    let created = find_argv(&calls, "new-session");
    assert!(created.windows(2).any(|pair| pair == ["-s", "worker-long"]));
}

/// ts: packages/daemon/test/tmux-session-names-native.test.ts > "rewrites dot and colon in the stored name and ignores a prefix"
#[tokio::test]
async fn session_names_rewrite_dot_and_colon() {
    let mux = FakeMux::new();
    let pane = mux
        .create_session(&spec("dev.qa:rig"))
        .await
        .expect("create");
    assert_eq!(pane.session, "dev_qa_rig");
    assert!(mux.has_session("dev.qa:rig").await.expect("alias"));
    assert!(!mux.has_session("dev").await.expect("prefix"));
}

/// ts: packages/daemon/test/tmux-session-env-native.test.ts > "preserves exact literal names and session-id reads"
#[tokio::test]
async fn session_env_is_a_verbatim_e_flag() {
    let mux = FakeMux::new();
    mux.create_session(&spec("dev")).await.expect("create");
    let calls = mux.recorded().await;
    let argv = find_argv(&calls, "new-session");
    assert!(argv.windows(2).any(|pair| pair == ["-e", "EMPTY="]));
    assert!(argv.windows(2).any(|pair| pair == ["-e", "PATH=/a b:/c"]));
}
