//! Real tmux coverage for the Codex launch path. No Codex login is required.

#[cfg(not(feature = "tmux-it"))]
#[test]
fn tmux_it_is_opt_in() {}

#[cfg(feature = "tmux-it")]
mod tmux_it {
    #![expect(clippy::expect_used, reason = "a failed tmux step fails the test")]
    #![expect(
        clippy::panic,
        reason = "a pane that never shows the composer fails the test"
    )]

    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use std::time::{Instant, SystemTime};

    use crewmux_adapters::{AgentAdapter, CodexAdapter, LaunchOutcome};
    use crewmux_mux::{
        CaptureRange, ComposerState, GuardConfig, GuardVerdict, Multiplexer, SessionSpec,
        TmuxConfig, TmuxMux, evaluate,
    };
    use crewmux_types::RuntimeKind;

    use super::support::{NOT_LOGGED_IN, binding, home_with, opts};

    #[tokio::test]
    async fn not_logged_in_creates_no_session() {
        let home = home_with(NOT_LOGGED_IN);
        let mux = boot().await;
        let seat = adapter_tmux(&mux, &home.bin);
        let binding = binding(&home.cwd, "codex-seat");
        let outcome = seat
            .launch(&binding, opts(&home, "codex-seat"))
            .await
            .expect("launch");
        assert!(matches!(outcome, LaunchOutcome::LoginRequired { .. }));
        assert!(!mux.has_session("codex-seat").await.expect("has"));
    }

    #[tokio::test]
    async fn logged_in_stub_creates_a_session() {
        let home = home_with(super::support::LOGGED_IN);
        let mux = boot().await;
        let seat = adapter_tmux(&mux, &home.bin);
        let binding = binding(&home.cwd, "codex-live");
        let outcome = seat
            .launch(&binding, opts(&home, "codex-live"))
            .await
            .expect("launch");
        assert!(matches!(outcome, LaunchOutcome::Ok { .. }));
        assert!(mux.has_session("codex-live").await.expect("has"));
        mux.kill_session("codex-live").await.expect("kill");
    }

    #[tokio::test]
    async fn coloured_idle_delivers_and_a_typed_draft_defers() {
        let mux = boot().await;
        let idle = pane_with(&mux, "codex-colour-idle", &coloured_idle()).await;
        assert_guard(&mux, &idle, true).await;
        let draft = pane_with(&mux, "codex-colour-draft", &coloured_draft()).await;
        assert_guard(&mux, &draft, false).await;
        mux.kill_session("codex-colour-idle").await.expect("idle");
        mux.kill_session("codex-colour-draft").await.expect("draft");
    }

    async fn pane_with(mux: &TmuxMux, name: &str, screen: &str) -> crewmux_mux::PaneRef {
        let path =
            std::env::temp_dir().join(format!("rig-codex-screen-{}-{name}", std::process::id()));
        std::fs::write(&path, screen).expect("screen");
        let mut spec = SessionSpec::new(name, std::env::temp_dir());
        spec.remain_on_exit = true;
        spec.argv = vec![
            "sh".into(),
            "-c".into(),
            format!("cat {}; sleep 30", path.display()),
        ];
        let pane = mux.create_session(&spec).await.expect("create");
        let mut last = String::new();
        for _ in 0..40 {
            last = mux
                .capture(&pane, CaptureRange::VisibleEscapes)
                .await
                .expect("capture")
                .text;
            if last.contains("shortcuts") {
                return pane;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("pane never showed the composer: {last}");
    }

    async fn assert_guard(mux: &TmuxMux, pane: &crewmux_mux::PaneRef, deliver: bool) {
        let inputs = mux.guard_inputs(pane, 40).await.expect("guard");
        assert!(
            inputs.tail.text.contains('\u{1b}'),
            "guard capture dropped SGR: {}",
            inputs.tail.text
        );
        let composer =
            crewmux_adapters::runtime_composer(RuntimeKind::Codex, &inputs.tail.text, None)
                .expect("codex composer");
        let cfg = GuardConfig {
            wall: SystemTime::now(),
            ..GuardConfig::default()
        };
        let evaluated = evaluate(&inputs, composer, &cfg, Instant::now());
        if deliver {
            assert!(
                matches!(composer, ComposerState::AtPrompt),
                "{composer:?} {}",
                inputs.tail.text
            );
            assert!(
                matches!(evaluated.verdict, GuardVerdict::Deliver),
                "{:?}",
                evaluated.verdict
            );
        } else {
            assert!(
                matches!(composer, ComposerState::Draft { .. }),
                "{composer:?} {}",
                inputs.tail.text
            );
            assert!(
                matches!(evaluated.verdict, GuardVerdict::Defer { .. }),
                "{:?}",
                evaluated.verdict
            );
        }
    }

    fn coloured_idle() -> String {
        "\u{1b}[1m›\u{1b}[22m \u{1b}[2mAsk Codex to do anything\u{1b}[22m\n  ? for shortcuts\n"
            .to_owned()
    }

    fn coloured_draft() -> String {
        "\u{1b}[1m›\u{1b}[22m hello\n  ? for shortcuts\n".to_owned()
    }

    fn adapter_tmux(mux: &Arc<TmuxMux>, bin: &std::path::Path) -> CodexAdapter {
        let owned = Arc::clone(mux);
        let dyn_mux: Arc<dyn Multiplexer> = owned;
        CodexAdapter::with_bin(dyn_mux, bin.to_path_buf())
            .with_budget(super::support::fast_budget())
    }

    async fn boot() -> Arc<TmuxMux> {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let socket = format!("rig-rs-{}-cx{n}", std::process::id());
        let mux = TmuxMux::connect(TmuxConfig {
            binary: std::path::PathBuf::from("tmux"),
            socket,
            pause_after: false,
        })
        .await
        .expect("tmux");
        Arc::new(mux)
    }
}

#[cfg(feature = "tmux-it")]
#[path = "codex_support.rs"]
mod support;
