//! Claude launch argv uses per-seat flags, never the user config file.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "temp-dir setup failures should fail the test"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crewmux_adapters::{
    AgentAdapter, BudgetOverrides, ClaudeAdapter, DeliveryCheck, Error, LaunchBudget, LaunchOpts,
    ProbeQuery, ResumeToken, SeatIo, SubmitMode, TurnVerdict,
};
use crewmux_mux::FakeMux;
use crewmux_types::SeatId;

#[path = "common/mod.rs"]
mod support;

#[tokio::test]
async fn uses_flags_not_global_config() {
    let root = scratch("argv");
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let mux = Arc::new(FakeMux::new());
    let ready = support::read(&support::fixtures().join("2.1.286/claude-020-ready.txt"));
    mux.set_capture(support::capture(&ready)).await;
    let adapter = ClaudeAdapter::new(
        support::shared(&mux),
        support::null_check(),
        root.join("seats"),
        root.join("home"),
    );
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
    let mut binding = binding;
    binding.model = Some("haiku".to_owned());
    let outcome = adapter.launch(&binding, opts(None)).await.expect("launch");
    let crewmux_adapters::LaunchOutcome::Ok { resume, .. } = outcome else {
        panic!("failed");
    };
    let id = resume.expect("token").value;
    let stored = std::fs::read_to_string(adapter.seat_dir(&binding.seat).join("claude-session-id"))
        .expect("id file");
    assert_eq!(stored.trim(), id);
    let calls = mux.recorded().await;
    let flat: Vec<&str> = calls.iter().flatten().map(String::as_str).collect();
    assert!(flat.contains(&"--session-id"), "{flat:?}");
    assert!(flat.contains(&"--mcp-config"), "{flat:?}");
    assert!(flat.contains(&"--strict-mcp-config"), "{flat:?}");
    assert!(flat.contains(&"--settings"), "{flat:?}");
    assert!(flat.contains(&"--model"), "{flat:?}");
    assert!(flat.contains(&id.as_str()), "{flat:?}");
    assert!(
        !flat.iter().any(|arg| arg.contains("claude.json")),
        "{flat:?}"
    );
    let settings = std::fs::read_to_string(adapter.seat_dir(&binding.seat).join("settings.json"))
        .expect("settings");
    assert!(!settings.to_ascii_lowercase().contains("fullscreen"));
    drop(std::fs::remove_dir_all(&root));
}

#[tokio::test]
async fn resume_uses_stored_id_flag() {
    let root = scratch("resume");
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let mux = Arc::new(FakeMux::new());
    let ready = support::read(&support::fixtures().join("2.1.286/claude-020-ready.txt"));
    mux.set_capture(support::capture(&ready)).await;
    let adapter = ClaudeAdapter::new(
        support::shared(&mux),
        support::null_check(),
        root.join("seats"),
        root.join("home"),
    );
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
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
    assert!(
        flat.iter()
            .any(|arg| arg == "11111111-1111-4111-8111-111111111111")
    );
    assert!(!flat.iter().any(|arg| arg == "--session-id"), "{flat:?}");
    assert!(!flat.iter().any(|arg| arg == "--model"), "{flat:?}");
    drop(std::fs::remove_dir_all(&root));
}

#[tokio::test]
async fn submit_matches_one_turn_and_rejects_a_composite() {
    let root = scratch("submit");
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let ready = support::read(&support::fixtures().join("2.1.286/claude-020-ready.txt"));
    let mux = Arc::new(FakeMux::new());
    mux.set_capture(support::capture(&ready)).await;
    let adapter = ClaudeAdapter::new(
        support::shared(&mux),
        Arc::new(ScriptCheck {
            turns: vec!["hello\nworld".to_owned()],
            verdict: TurnVerdict::Equal,
        }),
        root.join("seats"),
        root.join("home"),
    )
    .with_submit_wait(Duration::from_millis(200));
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
    std::fs::create_dir_all(adapter.seat_dir(&binding.seat)).expect("dir");
    std::fs::write(
        adapter.seat_dir(&binding.seat).join("claude-session-id"),
        "sid",
    )
    .expect("id");
    let io = adapter.io_for(&binding).await.expect("io");
    let receipt = io
        .submit("hello\nworld", SubmitMode::Enter)
        .await
        .expect("submit");
    assert_eq!(receipt.bytes, "hello\nworld".len());
    assert!(support::sent_paste(&mux.recorded().await));

    let busy = Arc::new(FakeMux::new());
    busy.set_capture(support::capture("esc to interrupt\n"))
        .await;
    let adapter = ClaudeAdapter::new(
        support::shared(&busy),
        support::null_check(),
        root.join("seats"),
        root.join("home"),
    );
    let io = adapter.io_for(&binding).await.expect("io");
    let err = io.submit("x", SubmitMode::Enter).await.expect_err("busy");
    assert!(matches!(err, Error::NotReady));
    assert!(!support::sent_paste(&busy.recorded().await));

    let mux = Arc::new(FakeMux::new());
    mux.set_capture(support::capture(&ready)).await;
    let adapter = ClaudeAdapter::new(
        support::shared(&mux),
        Arc::new(ScriptCheck {
            turns: vec!["wrapped twice".to_owned()],
            verdict: TurnVerdict::Composite,
        }),
        root.join("seats"),
        root.join("home"),
    )
    .with_submit_wait(Duration::from_secs(30));
    let io = adapter.io_for(&binding).await.expect("io");
    let started = std::time::Instant::now();
    let err = io
        .submit("wrapped twice", SubmitMode::Enter)
        .await
        .expect_err("mix");
    assert!(matches!(err, Error::Contaminated));
    assert!(started.elapsed() < Duration::from_secs(2));
    drop(std::fs::remove_dir_all(&root));
}

#[tokio::test]
async fn missing_cwd_does_not_launch() {
    let root = scratch("missing");
    let mux = Arc::new(FakeMux::new());
    let adapter = ClaudeAdapter::new(
        support::shared(&mux),
        support::null_check(),
        root.join("seats"),
        root.join("home"),
    );
    let missing = root.join("nope");
    let binding = crewmux_adapters::binding_for(SeatId::new(), missing, "seat__rig");
    let outcome = adapter.launch(&binding, opts(None)).await.expect("outcome");
    assert!(matches!(
        outcome,
        crewmux_adapters::LaunchOutcome::Failed { .. }
    ));
    assert_eq!(mux.recorded().await, Vec::<Vec<String>>::new());
    drop(std::fs::remove_dir_all(&root));
}

#[tokio::test]
async fn interrupt_sends_no_key() {
    let root = scratch("interrupt");
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let mux = Arc::new(FakeMux::new());
    let adapter = ClaudeAdapter::new(
        support::shared(&mux),
        support::null_check(),
        root.join("seats"),
        root.join("home"),
    );
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
    std::fs::create_dir_all(adapter.seat_dir(&binding.seat)).expect("dir");
    std::fs::write(
        adapter.seat_dir(&binding.seat).join("claude-session-id"),
        "sid",
    )
    .expect("id");
    let io = adapter.io_for(&binding).await.expect("io");
    let err = io.interrupt().await.expect_err("no key");
    assert!(matches!(err, Error::Message(_)));
    assert_eq!(mux.recorded().await, Vec::<Vec<String>>::new());
    drop(std::fs::remove_dir_all(&root));
}

struct ScriptCheck {
    turns: Vec<String>,
    verdict: TurnVerdict,
}

impl DeliveryCheck for ScriptCheck {
    fn cursor(&self, _query: &ProbeQuery) -> Result<u64, Error> {
        Ok(0)
    }

    fn turns(&self, _query: &ProbeQuery) -> Result<Vec<String>, Error> {
        Ok(self.turns.clone())
    }

    fn verdict(&self, _sent: &str, _stored: &str) -> TurnVerdict {
        self.verdict
    }
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
            poll: Some(Duration::from_millis(10)),
        })),
        claude_bin: None,
    }
}

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rig-claude-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    drop(std::fs::remove_dir_all(&root));
    std::fs::create_dir_all(&root).expect("scratch");
    root
}

#[cfg(feature = "tmux-it")]
mod tmux_it {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use crewmux_adapters::{AgentAdapter, ClaudeAdapter, LaunchOpts, Readiness};
    use crewmux_mux::{Multiplexer, TmuxConfig, TmuxMux};
    use crewmux_types::SeatId;

    use super::support;

    static SOCKETS: AtomicU64 = AtomicU64::new(0);

    #[tokio::test]
    async fn fake_claude_binary_argv_has_seat_flags() {
        let n = SOCKETS.fetch_add(1, Ordering::Relaxed);
        let socket = format!("rig-rs-{}-claude{n}", std::process::id());
        let mux = TmuxMux::connect(TmuxConfig {
            binary: PathBuf::from("tmux"),
            socket,
            pause_after: false,
        })
        .await
        .expect("tmux");
        let root = super::scratch("tmux");
        let cwd = root.join("work");
        std::fs::create_dir_all(&cwd).expect("cwd");
        let out = root.join("argv.txt");
        let script = root.join("claude");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nexec sleep 30\n",
                out.display()
            ),
        )
        .expect("script");
        let mut perms = std::fs::metadata(&script).expect("meta").permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
        }
        std::fs::set_permissions(&script, perms).expect("chmod");
        let mux: std::sync::Arc<dyn Multiplexer> = std::sync::Arc::new(mux);
        let adapter = ClaudeAdapter::new(
            mux,
            support::null_check(),
            root.join("seats"),
            root.join("home"),
        );
        let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
        let mut opts = super::opts(None);
        opts.budget = Some(crewmux_adapters::LaunchBudget::with_overrides(
            crewmux_adapters::BudgetOverrides {
                first_screen: Some(Duration::from_millis(200)),
                ready_after_last_gate: Some(Duration::from_millis(200)),
                inconclusive_stable: Some(Duration::from_millis(200)),
                overall: Some(Duration::from_secs(2)),
                poll: Some(Duration::from_millis(50)),
            },
        ));
        opts.claude_bin = Some(script);
        drop(adapter.launch(&binding, opts).await.expect("launch"));
        let text = std::fs::read_to_string(&out).expect("argv file");
        for flag in [
            "--session-id",
            "--mcp-config",
            "--strict-mcp-config",
            "--settings",
        ] {
            assert!(text.contains(flag), "{text}");
        }
        assert!(!text.contains("claude.json"), "{text}");
        drop(std::fs::remove_dir_all(&root));
    }

    #[tokio::test]
    async fn live_claude_is_opt_in() {
        if std::env::var("RIG_LIVE_CLAUDE").ok().as_deref() != Some("1") {
            return;
        }
        let n = SOCKETS.fetch_add(1, Ordering::Relaxed);
        let socket = format!("rig-rs-{}-live{n}", std::process::id());
        let mux = TmuxMux::connect(TmuxConfig {
            binary: std::path::PathBuf::from("tmux"),
            socket,
            pause_after: false,
        })
        .await
        .expect("tmux");
        let root = super::scratch("live");
        let cwd = root.join("work");
        std::fs::create_dir_all(&cwd).expect("cwd");
        let home = root.join("home");
        std::fs::create_dir_all(&home).expect("home");
        let mux: std::sync::Arc<dyn Multiplexer> = std::sync::Arc::new(mux);
        let adapter = ClaudeAdapter::new(mux, support::null_check(), root.join("seats"), home);
        let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "live__rig");
        let mut opts = LaunchOpts {
            session_name: "live__rig".to_owned(),
            env: std::collections::BTreeMap::new(),
            resume: None,
            fork: false,
            auto_trust: false,
            budget: None,
            claude_bin: None,
        };
        opts.budget = Some(crewmux_adapters::LaunchBudget::claude());
        let outcome = adapter.launch(&binding, opts).await.expect("launch");
        assert!(matches!(
            outcome,
            crewmux_adapters::LaunchOutcome::Ok { .. }
        ));
        let ready = adapter.check_ready(&binding).await.expect("ready");
        assert!(
            matches!(
                ready,
                Readiness::Ready
                    | Readiness::Gate(_)
                    | Readiness::Booting
                    | Readiness::Inconclusive
            ),
            "{ready:?}"
        );
        drop(std::fs::remove_dir_all(&root));
    }
}
