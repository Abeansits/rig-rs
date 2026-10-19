//! Real tmux coverage for one deadline fire. The inner module matches the nextest filter.

#[expect(
    clippy::expect_used,
    reason = "a failed tmux harness step fails the test"
)]
mod tmux_it {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use crewmux_adapters::{AdapterRegistry, StubAdapter};
    use crewmux_core::Timestamp;
    use crewmux_messaging::{NewItem, QueueBook, SessionRef};
    use crewmux_mux::{Multiplexer, TmuxConfig, TmuxMux};
    use crewmux_orchestration::{Engine, Instance, Keepalive, Status};
    use crewmux_store::Store;
    use crewmux_types::{QItemId, WorkflowInstanceId};
    use tokio_util::sync::CancellationToken;

    use crate::launch::bring_up;
    use crate::outbox::tick;
    use crate::state::App;

    const STUB: &str = "#!/usr/bin/env python3\nimport sys, time\nsys.stdout.write('stub> \\n')\nsys.stdout.flush()\nwhile True:\n    time.sleep(3600)\n";

    fn unique() -> u64 {
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }

    struct Live {
        app: App,
        mux: Arc<TmuxMux>,
    }

    async fn boot(path: PathBuf, home: PathBuf) -> Live {
        let script = home.join("stub.py");
        tokio::fs::write(&script, STUB).await.expect("script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = tokio::fs::metadata(&script).await.expect("meta");
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            tokio::fs::set_permissions(&script, perms)
                .await
                .expect("chmod");
        }
        let socket = format!("rig-rs-{}-w{}", std::process::id(), unique());
        let mux = Arc::new(
            TmuxMux::connect(TmuxConfig {
                binary: PathBuf::from("tmux"),
                socket,
                pause_after: false,
            })
            .await
            .expect("tmux"),
        );
        let cloned = Arc::clone(&mux);
        let shared: Arc<dyn Multiplexer> = cloned;
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(StubAdapter::new(
            Arc::clone(&shared),
            script.display().to_string(),
        )));
        let store = Arc::new(Store::open(&path).expect("db"));
        let app = App::assemble(
            store,
            shared,
            Arc::new(registry),
            None,
            CancellationToken::new(),
            home.clone(),
        );
        app.outbox.hold.store(true, Ordering::SeqCst);
        app.outbox
            .now_ms
            .store(5 * 60 * 60 * 1000, Ordering::SeqCst);
        app.launch_workers().await;
        let spec = format!(
            "name: pair\nseats:\n  - name: one\n    runtime: stub\n    cwd: {}\n",
            home.join("work").display()
        );
        bring_up(&app, &spec).await.expect("up");
        Live { app, mux }
    }

    fn overdue() -> (Engine, QueueBook) {
        let mut book = QueueBook::new();
        book.create(
            NewItem {
                id: Some(QItemId::new("q-tmux").expect("id")),
                source: SessionRef::new("src@pair").expect("src"),
                destination: SessionRef::new("one@pair").expect("dest"),
                body: "tmux-secret-body".to_owned(),
                priority: 0,
                tier: None,
                tags: Vec::new(),
                handed_off_from: None,
                last_nudge: None,
                explicit_human: false,
                fingerprint: "fp".to_owned(),
            },
            Timestamp::from_millis(0),
        )
        .expect("packet");
        let engine = Engine {
            instances: vec![Instance {
                id: WorkflowInstanceId::new("inst-tmux").expect("id"),
                workflow_name: "wf".to_owned(),
                workflow_version: "1".to_owned(),
                created_by: SessionRef::new("creator@pair").expect("by"),
                created_at: Timestamp::from_millis(0),
                status: Status::Active,
                frontier: vec![QItemId::new("q-tmux").expect("q")],
                current_step_id: None,
                hop_count: 0,
                fallback_synthesis: None,
                last_decision: None,
                completed_at: None,
                version: 1,
                resume_count: 0,
                hops_baseline: 0,
                bound_rig: None,
            }],
            keepalives: vec![Keepalive {
                instance_id: "inst-tmux".to_owned(),
                target_session: "one@pair".to_owned(),
                interval_seconds: 60,
                deadline_gated: true,
                armed: true,
                reason: None,
                yaml: "policy: workflow-keepalive\ntarget:\n  session: one@pair\ncontext:\n  workflow_instance_id: inst-tmux\ndeadline_gated: true\n".to_owned(),
            }],
            ..Engine::default()
        };
        (engine, book)
    }

    async fn text(app: &App) -> String {
        let shot = crate::query::capture(app, "one", None)
            .await
            .expect("capture");
        shot.get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }

    #[tokio::test]
    async fn deadline_fires_once_across_restart_on_a_private_socket() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let home = std::env::temp_dir().join(format!(
            "rig-outbox-tmux-{}-{}-{nanos}",
            std::process::id(),
            unique()
        ));
        tokio::fs::create_dir_all(home.join("work"))
            .await
            .expect("work");
        let path = home.join("rig.sqlite");
        let live = boot(path.clone(), home.clone()).await;
        let (engine, book) = overdue();
        *live.app.workflow.lock().await = engine;
        *live.app.queue.lock().await = book;
        tick(&live.app).await.expect("fire");
        assert_eq!(
            text(&live.app).await.matches("Workflow deadline:").count(),
            1
        );
        assert!(!text(&live.app).await.contains("tmux-secret-body"));
        let mux = Arc::clone(&live.mux);
        let app = live.app;
        app.cancel.cancel();
        app.tasks.close();
        tokio::time::timeout(std::time::Duration::from_secs(2), app.tasks.wait())
            .await
            .expect("stop");
        drop(app);
        let again = reopen(path, home, mux).await;
        tick(&again).await.expect("restart");
        assert_eq!(text(&again).await.matches("Workflow deadline:").count(), 1);
    }

    async fn reopen(path: PathBuf, home: PathBuf, mux: Arc<TmuxMux>) -> App {
        let script = home.join("stub.py");
        let cloned = Arc::clone(&mux);
        let shared: Arc<dyn Multiplexer> = cloned;
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(StubAdapter::new(
            Arc::clone(&shared),
            script.display().to_string(),
        )));
        let store = Arc::new(Store::open(&path).expect("db"));
        let app = App::assemble(
            store,
            shared,
            Arc::new(registry),
            None,
            CancellationToken::new(),
            home,
        );
        app.outbox.hold.store(true, Ordering::SeqCst);
        app.outbox
            .now_ms
            .store(5 * 60 * 60 * 1000, Ordering::SeqCst);
        app.launch_workers().await;
        app
    }
}
