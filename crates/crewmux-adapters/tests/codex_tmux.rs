//! Real tmux coverage for the Codex launch path. No Codex login is required.

#[cfg(not(feature = "tmux-it"))]
#[test]
fn tmux_it_is_opt_in() {}

#[cfg(feature = "tmux-it")]
mod tmux_it {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use crewmux_adapters::{AgentAdapter, CodexAdapter, LaunchOutcome};
    use crewmux_mux::{Multiplexer, TmuxConfig, TmuxMux};

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

    fn adapter_tmux(mux: &Arc<TmuxMux>, bin: &std::path::Path) -> CodexAdapter {
        let owned = Arc::clone(mux);
        let dyn_mux: Arc<dyn Multiplexer> = owned;
        CodexAdapter::with_bin(dyn_mux, bin.to_path_buf())
            .with_budget(super::support::fast_budget())
    }

    #[expect(clippy::expect_used, reason = "a missing tmux server fails the test")]
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
