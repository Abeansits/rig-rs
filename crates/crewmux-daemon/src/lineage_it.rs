//! Real tmux coverage for submit. The inner module name matches the nextest filter.

#[expect(
    clippy::expect_used,
    reason = "a failed tmux harness step fails the test"
)]
mod tmux_it {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use crewmux_adapters::{AdapterRegistry, ClaudeAdapter};
    use crewmux_core::{Rig, RigRepo, Seat, SeatRepo, SeatState, Timestamp};
    use crewmux_mux::{CaptureRange, Multiplexer, PaneRef, SessionSpec, TmuxConfig, TmuxMux};
    use crewmux_types::{RigId, RigName, RuntimeKind, SeatId, SeatName};
    use tokio_util::sync::CancellationToken;

    use crate::db::write;
    use crate::state::App;
    use crate::submit::submit_text;

    const MARKER: &str = "zz-lineage-probe";
    const ORIG: &str = "11111111-1111-4111-8111-111111111111";
    const OTHER: &str = "22222222-2222-4222-8222-222222222222";

    const AGENT: &str = r#"
import os, sys, time
args = sys.argv[1:]
def flag(name):
    return args[args.index(name) + 1]
mode, trigger, script = flag("--mode"), flag("--trigger"), flag("--script")
rule = "─" * 40
sys.stdout.write(rule + "\n❯ \n" + rule + "\n  ? for shortcuts\n")
sys.stdout.flush()
if mode == "stay":
    while True:
        time.sleep(1)
deadline = time.time() + 20
while not os.path.exists(trigger):
    if time.time() > deadline:
        raise SystemExit("trigger")
    time.sleep(0.05)
if mode == "bash":
    os.execv("/bin/bash", ["bash"])
os.execv(sys.executable, [
    sys.executable, script, "--mode", "stay", "--trigger", trigger,
    "--script", script, "--session-id", flag("--other"),
])
"#;

    const NODE: &str = r"
const fs = require('fs');
const args = process.argv.slice(2);
function flag(name) { return args[args.indexOf(name) + 1]; }
const when = flag('--title');
const ready = flag('--ready');
const go = flag('--go');
if (when === 'before') process.title = '2.1.289';
fs.writeFileSync(ready, '1');
if (when === 'after') {
  const deadline = Date.now() + 20000;
  while (!fs.existsSync(go) && Date.now() < deadline) {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 50);
  }
  process.title = '2.1.289';
  fs.writeFileSync(ready + '.titled', '1');
}
setInterval(() => {}, 1000);
";

    struct Live {
        app: App,
        mux: Arc<TmuxMux>,
        root: PathBuf,
        socket: String,
    }

    impl Drop for Live {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.root));
        }
    }

    fn unique() -> u64 {
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }

    async fn boot() -> Live {
        let root =
            std::env::temp_dir().join(format!("rig-lin-{}-{}", std::process::id(), unique()));
        tokio::fs::create_dir_all(root.join("work"))
            .await
            .expect("work");
        tokio::fs::create_dir_all(root.join("seats"))
            .await
            .expect("seats");
        let socket = format!("rig-rs-{}-l{}", std::process::id(), unique());
        let mux = Arc::new(
            TmuxMux::connect(TmuxConfig {
                binary: PathBuf::from("tmux"),
                socket: socket.clone(),
                pause_after: false,
            })
            .await
            .expect("tmux"),
        );
        let cloned: Arc<TmuxMux> = Arc::clone(&mux);
        let shared: Arc<dyn Multiplexer> = cloned;
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(
            ClaudeAdapter::new(Arc::clone(&shared), root.join("seats"), root.clone())
                .with_submit_wait(Duration::from_millis(200)),
        ));
        let store =
            Arc::new(crewmux_store::Store::open(std::path::Path::new(":memory:")).expect("db"));
        let app = App::assemble(
            store,
            shared,
            Arc::new(registry),
            None,
            CancellationToken::new(),
            root.clone(),
        );
        Live {
            app,
            mux,
            root,
            socket,
        }
    }

    async fn seed(app: &App, tmux: &str, pane_id: &str) -> Seat {
        let seat = SeatId::new();
        let rig = RigId::new();
        let cwd = app.home.join("work");
        let dir = app.home.join("seats").join(seat.to_string());
        tokio::fs::create_dir_all(&dir).await.expect("seat dir");
        tokio::fs::write(dir.join("claude-session-id"), ORIG)
            .await
            .expect("session");
        let tmux = tmux.to_owned();
        let pane_id = pane_id.to_owned();
        write(&app.store, move |db| {
            RigRepo::upsert(
                db,
                &Rig {
                    id: rig,
                    name: RigName::new("demo").expect("rig"),
                    created_at: Timestamp::from_millis(0),
                },
            )?;
            SeatRepo::upsert(
                db,
                &Seat {
                    id: seat,
                    rig_id: rig,
                    name: SeatName::new("claude").expect("seat"),
                    runtime: RuntimeKind::ClaudeCode,
                    cwd: Some(cwd),
                    model: None,
                    state: SeatState::Ready,
                    reason: None,
                    tmux_session: Some(tmux),
                    pane_id: Some(pane_id),
                },
            )?;
            Ok(())
        })
        .await
        .expect("seed");
        crate::query::find_seat(app, "claude").await.expect("seat")
    }

    async fn pane_info(mux: &TmuxMux, session: &str) -> crewmux_mux::PaneInfo {
        mux.list_panes()
            .await
            .expect("panes")
            .into_iter()
            .find(|info| info.session == session)
            .expect("pane")
    }

    async fn argv_of(pid: u32) -> String {
        let path = format!("/proc/{pid}/cmdline");
        if let Ok(bytes) = tokio::fs::read(&path).await {
            let text = String::from_utf8_lossy(&bytes).replace('\0', " ");
            if !text.trim().is_empty() {
                return text;
            }
        }
        let output = tokio::process::Command::new("ps")
            .args(["-ww", "-p", &pid.to_string(), "-o", "args="])
            .env("LC_ALL", "C")
            .output()
            .await
            .expect("ps");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    async fn until<F, Fut>(mut probe: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let start = Instant::now();
        loop {
            if probe().await {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(8), "timed out");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn capture(mux: &TmuxMux, pane: &PaneRef) -> String {
        mux.capture(pane, CaptureRange::Visible)
            .await
            .expect("capture")
            .text
    }

    async fn refuse(app: &App, pane: &PaneRef, mux: &TmuxMux, code: &str) {
        let err = submit_text(app, "claude", MARKER)
            .await
            .expect_err("refused");
        assert!(err.to_string().contains(code), "{err}");
        let text = capture(mux, pane).await;
        assert!(!text.contains(MARKER), "{text}");
    }

    async fn agent_pane(live: &Live, mode: &str) -> (PaneRef, PathBuf) {
        let script = live.root.join("agent.py");
        tokio::fs::write(&script, AGENT).await.expect("script");
        let trigger = live.root.join(format!("go-{mode}"));
        let name = format!("lin{}", unique());
        let mut spec = SessionSpec::new(&name, live.root.join("work"));
        spec.argv = vec![
            "python3".into(),
            script.display().to_string(),
            "--mode".into(),
            mode.into(),
            "--trigger".into(),
            trigger.display().to_string(),
            "--script".into(),
            script.display().to_string(),
            "--other".into(),
            OTHER.into(),
            "--session-id".into(),
            ORIG.into(),
        ];
        let pane = live.mux.create_session(&spec).await.expect("session");
        let session = pane.session.clone();
        until(|| {
            let mux = Arc::clone(&live.mux);
            let session = session.clone();
            async move {
                let info = pane_info(&mux, &session).await;
                let Some(pid) = info.pid else { return false };
                argv_of(pid).await.contains(ORIG)
            }
        })
        .await;
        (pane, trigger)
    }

    #[tokio::test]
    async fn exec_into_another_session_pastes_nothing() {
        let live = boot().await;
        let (pane, trigger) = agent_pane(&live, "switch").await;
        let info = pane_info(&live.mux, &pane.session).await;
        let seat = seed(&live.app, &pane.session, &info.pane_id).await;
        crate::lineage::record(&live.app, &seat, &pane)
            .await
            .expect("record");
        tokio::fs::write(&trigger, "1").await.expect("trigger");
        let pid = info.pid.expect("pid");
        let switched = format!("--session-id {OTHER}");
        until(move || {
            let switched = switched.clone();
            async move {
                let argv = argv_of(pid).await;
                argv.contains(&switched) && !argv.contains(ORIG)
            }
        })
        .await;
        refuse(&live.app, &pane, &live.mux, "lineage:session_id_mismatch").await;
    }

    #[tokio::test]
    async fn respawn_under_another_session_pastes_nothing() {
        let live = boot().await;
        let (pane, _trigger) = agent_pane(&live, "stay").await;
        let info = pane_info(&live.mux, &pane.session).await;
        let seat = seed(&live.app, &pane.session, &info.pane_id).await;
        crate::lineage::record(&live.app, &seat, &pane)
            .await
            .expect("record");
        let old = info.pid.expect("pid");
        let script = live.root.join("agent.py");
        let trigger = live.root.join("stay-trigger");
        let status = tokio::process::Command::new("tmux")
            .args([
                "-L",
                &live.socket,
                "-f",
                "/dev/null",
                "respawn-pane",
                "-k",
                "-t",
                info.pane_id.as_str(),
                "--",
                "python3",
                script.to_str().expect("script"),
                "--mode",
                "stay",
                "--trigger",
                trigger.to_str().expect("trigger"),
                "--script",
                script.to_str().expect("script"),
                "--session-id",
                OTHER,
            ])
            .env_remove("TMUX")
            .env_remove("TMUX_TMPDIR")
            .status()
            .await
            .expect("respawn");
        assert!(status.success());
        until(|| {
            let mux = Arc::clone(&live.mux);
            let session = pane.session.clone();
            async move {
                let info = pane_info(&mux, &session).await;
                info.pid.is_some_and(|pid| pid != old)
            }
        })
        .await;
        refuse(&live.app, &pane, &live.mux, "lineage:pane_pid_mismatch").await;
    }

    #[tokio::test]
    async fn exec_into_bash_pastes_nothing() {
        let live = boot().await;
        let (pane, trigger) = agent_pane(&live, "bash").await;
        let info = pane_info(&live.mux, &pane.session).await;
        let seat = seed(&live.app, &pane.session, &info.pane_id).await;
        crate::lineage::record(&live.app, &seat, &pane)
            .await
            .expect("record");
        tokio::fs::write(&trigger, "1").await.expect("trigger");
        until(|| {
            let mux = Arc::clone(&live.mux);
            let session = pane.session.clone();
            async move {
                let cmd = pane_info(&mux, &session).await.current_cmd;
                cmd == "bash" || cmd.ends_with("/bash") || cmd == "-bash"
            }
        })
        .await;
        refuse(&live.app, &pane, &live.mux, "lineage:bare_shell").await;
    }

    #[expect(clippy::panic, reason = "the process.title test needs a node binary")]
    async fn node_bin() -> String {
        for name in ["node", "nodejs"] {
            let status = tokio::process::Command::new(name)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            if matches!(status, Ok(code) if code.success()) {
                return name.to_owned();
            }
        }
        panic!("node is required to reproduce process.title");
    }

    async fn node_pane(live: &Live, when: &str) -> (PaneRef, PathBuf, PathBuf) {
        let bin = node_bin().await;
        let script = live.root.join("launch.js");
        tokio::fs::write(&script, NODE).await.expect("script");
        let ready = live.root.join(format!("ready-{when}"));
        let go = live.root.join(format!("title-{when}"));
        let name = format!("node{}", unique());
        let mut spec = SessionSpec::new(&name, live.root.join("work"));
        spec.argv = vec![
            bin,
            script.display().to_string(),
            "--title".into(),
            when.into(),
            "--ready".into(),
            ready.display().to_string(),
            "--go".into(),
            go.display().to_string(),
            "--session-id".into(),
            ORIG.into(),
        ];
        let pane = live.mux.create_session(&spec).await.expect("session");
        (pane, ready, go)
    }

    async fn title_still_submits(when: &str) {
        let live = boot().await;
        let (pane, ready, go) = node_pane(&live, when).await;
        until(|| {
            let ready = ready.clone();
            async move { tokio::fs::try_exists(&ready).await.unwrap_or(false) }
        })
        .await;
        if when == "before" {
            let session = pane.session.clone();
            until(|| {
                let mux = Arc::clone(&live.mux);
                let session = session.clone();
                async move { pane_info(&mux, &session).await.current_cmd == "2.1.289" }
            })
            .await;
        }
        if when == "after" {
            let session = pane.session.clone();
            until(|| {
                let mux = Arc::clone(&live.mux);
                let session = session.clone();
                async move {
                    let info = pane_info(&mux, &session).await;
                    let Some(pid) = info.pid else { return false };
                    info.current_cmd == "node" && argv_of(pid).await.contains("--session-id")
                }
            })
            .await;
        }
        let info = pane_info(&live.mux, &pane.session).await;
        let seat = seed(&live.app, &pane.session, &info.pane_id).await;
        crate::lineage::record(&live.app, &seat, &pane)
            .await
            .expect("record");
        if when == "after" {
            tokio::fs::write(&go, "1").await.expect("go");
            let titled = format!("{}.titled", ready.display());
            until(|| {
                let titled = titled.clone();
                let mux = Arc::clone(&live.mux);
                let session = pane.session.clone();
                async move {
                    if !tokio::fs::try_exists(&titled).await.unwrap_or(false) {
                        return false;
                    }
                    pane_info(&mux, &session).await.current_cmd == "2.1.289"
                }
            })
            .await;
        }
        crate::lineage::ensure(&live.app, &seat, &pane)
            .await
            .expect("version title");
        match submit_text(&live.app, "claude", MARKER).await {
            Ok(_) => {}
            Err(err) => assert!(!err.to_string().contains("lineage:"), "{err}"),
        }
    }

    #[tokio::test]
    async fn node_title_before_launch_still_submits() {
        title_still_submits("before").await;
    }

    #[tokio::test]
    async fn node_title_after_launch_still_submits() {
        title_still_submits("after").await;
    }
}
