//! [`FakeMux`](crewmux_mux::FakeMux) contract: argv recording and scripted guard inputs.

use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use crewmux_mux::{
    Capture, CaptureRange, ClientInfo, FakeMux, GuardInputs, Key, Multiplexer, MuxEvent, PasteOpts,
    SessionSpec,
};

fn spec(name: &str) -> SessionSpec {
    SessionSpec::new(name, PathBuf::from("/tmp"))
}

#[tokio::test]
async fn paste_argv_is_load_then_paste_without_enter() {
    let mux = FakeMux::new();
    let pane = mux.create_session(&spec("demo")).await.expect("create");
    let receipt = mux
        .paste(&pane, "hello\n", PasteOpts::default())
        .await
        .expect("paste");
    assert!(receipt.buffer.starts_with("rig-"));
    assert!(receipt.bracketed);
    let calls = mux.recorded().await;
    let paste = calls
        .iter()
        .find(|argv| argv.iter().any(|arg| arg == "paste-buffer"))
        .expect("paste");
    assert!(paste.windows(2).any(|pair| pair == ["-d", "-p"]));
    assert!(paste.iter().any(|arg| arg == "-r"));
    assert!(paste.iter().any(|arg| arg == "-L"));
    assert_eq!(paste.get(2).map(String::as_str), Some("rig-rs"));
    assert!(
        !calls
            .iter()
            .flatten()
            .any(|arg| arg == "Enter" || arg == "sh")
    );
    let load = calls
        .iter()
        .find(|argv| argv.iter().any(|arg| arg == "load-buffer"))
        .expect("load");
    assert_eq!(
        load.get(load.len().saturating_sub(3)).map(String::as_str),
        Some("-b")
    );
}

#[tokio::test]
async fn failed_paste_records_delete_buffer() {
    let mux = FakeMux::new();
    let pane = mux.create_session(&spec("demo")).await.expect("create");
    mux.fail_next_paste().await;
    let err = mux
        .paste(&pane, "x", PasteOpts::default())
        .await
        .expect_err("paste");
    assert!(err.to_string().contains("can't find pane"), "{err}");
    let calls = mux.recorded().await;
    let delete = calls
        .iter()
        .find(|argv| argv.iter().any(|arg| arg == "delete-buffer"))
        .expect("delete");
    let load = calls
        .iter()
        .find(|argv| argv.iter().any(|arg| arg == "load-buffer"))
        .expect("load");
    let load_buf = load
        .windows(2)
        .find(|pair| pair[0] == "-b")
        .map(|pair| pair[1].clone());
    let delete_buf = delete
        .windows(2)
        .find(|pair| pair[0] == "-b")
        .map(|pair| pair[1].clone());
    assert_eq!(load_buf, delete_buf);
}

#[tokio::test]
async fn remain_on_exit_records_the_doorbell_hook() {
    let mux = FakeMux::with_socket("sock");
    let mut spec = spec("demo");
    spec.remain_on_exit = true;
    mux.create_session(&spec).await.expect("create");
    let calls = mux.recorded().await;
    let hook = calls
        .iter()
        .find(|argv| argv.iter().any(|arg| arg == "pane-died"))
        .expect("hook");
    let joined = hook.join(" ");
    assert!(joined.contains("run-shell -b"), "{joined}");
    assert!(joined.contains("rig-dead-"), "{joined}");
    assert!(joined.contains("-L sock"), "{joined}");
    assert!(joined.contains("=demo:"), "{joined}");
}

#[tokio::test]
async fn scripted_guard_and_injected_events_round_trip() {
    let mux = FakeMux::new();
    let pane = mux.create_session(&spec("demo")).await.expect("create");
    let scripted = GuardInputs {
        human_clients: Vec::new(),
        last_human_keystroke_tmux: Some(UNIX_EPOCH + Duration::from_secs(11)),
        last_human_keystroke_tap: None,
        pane_in_mode: true,
        tail: Capture::new("tail".to_owned(), (1, 2)),
        last_own_write: None,
    };
    mux.script_guard(scripted.clone()).await;
    let got = mux.guard_inputs(&pane, 4).await.expect("guard");
    assert_eq!(got, scripted);
    let mut events = mux.events();
    mux.inject(MuxEvent::PaneDied {
        pane_id: "%1".to_owned(),
        status: Some(7),
    })
    .await;
    let event = events.recv().await.expect("event");
    assert_eq!(
        event,
        MuxEvent::PaneDied {
            pane_id: "%1".to_owned(),
            status: Some(7)
        }
    );
}

#[tokio::test]
async fn human_clients_drop_the_control_mode_client() {
    let mux = FakeMux::new();
    let activity = UNIX_EPOCH + Duration::from_secs(20);
    mux.set_clients(vec![
        ClientInfo {
            tty: String::new(),
            session: "demo".to_owned(),
            activity,
            control_mode: true,
        },
        ClientInfo {
            tty: "/dev/pts/2".to_owned(),
            session: "demo".to_owned(),
            activity,
            control_mode: false,
        },
    ])
    .await;
    let humans = mux.human_clients("demo").await.expect("humans");
    assert_eq!(humans.len(), 1);
    assert!(!humans.first().expect("human").control_mode);
    mux.set_capture(Capture::new("screen".to_owned(), (0, 1)))
        .await;
    mux.set_in_mode(true).await;
    let pane = mux.create_session(&spec("demo")).await.expect("create");
    let guard = mux.guard_inputs(&pane, 3).await.expect("guard");
    assert!(guard.pane_in_mode);
    assert_eq!(guard.tail.text, "screen");
    let again = mux
        .capture(&pane, CaptureRange::LastLines(3))
        .await
        .expect("capture");
    assert_eq!(guard.tail.hash, again.hash);
}

#[tokio::test]
async fn session_prefixes_do_not_collide() {
    let mux = FakeMux::new();
    mux.create_session(&spec("worker-long"))
        .await
        .expect("long");
    assert!(!mux.has_session("worker").await.expect("has"));
    assert!(mux.has_session("worker-long").await.expect("has long"));
    assert!(!mux.has_session("foo.bar").await.expect("missing"));
    let dotted = mux.create_session(&spec("foo.bar")).await.expect("dot");
    assert_eq!(dotted.session, "foo_bar");
    assert!(mux.has_session("foo.bar").await.expect("dot has"));
}

#[tokio::test]
async fn own_writes_are_visible_to_an_unscripted_guard() {
    let mux = FakeMux::new();
    let pane = mux.create_session(&spec("demo")).await.expect("create");
    let before = mux.guard_inputs(&pane, 1).await.expect("before");
    assert!(before.last_own_write.is_none());
    mux.send_keys(&pane, &[Key::Enter]).await.expect("keys");
    let after = mux.guard_inputs(&pane, 1).await.expect("after");
    assert!(after.last_own_write.is_some());
    assert!(after.last_human_keystroke_tmux.is_none());
}
