//! Real tmux coverage. The inner module name matches the nextest `tmux_it` filter.

#[cfg(not(feature = "tmux-it"))]
#[test]
fn tmux_it_is_opt_in() {}

#[cfg(feature = "tmux-it")]
#[expect(
    clippy::expect_used,
    reason = "a failed tmux harness step fails the test"
)]
#[expect(clippy::panic, reason = "a failed tmux harness step fails the test")]
mod suite {
    mod tmux_it {
        use std::path::{Path, PathBuf};
        use std::process::Stdio;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{Duration, Instant};

        use crewmux_mux::{
            CONTROL_SESSION, CaptureRange, DeathStream, Key, Multiplexer, MuxEvent, OutputStream,
            PaneRef, PasteOpts, SessionSpec, TmuxConfig, TmuxMux,
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::process::{Child, Command};
        use tokio::sync::broadcast;

        const PTY: &str = r#"
import os, sys, pty, struct, fcntl, termios, select
socket, session = sys.argv[1], sys.argv[2]
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 120, 0, 0))
if os.fork() == 0:
    os.setsid()
    os.dup2(slave, 0); os.dup2(slave, 1); os.dup2(slave, 2)
    os.close(master); os.close(slave)
    os.environ["TERM"] = "xterm-256color"
    os.execvp("tmux", ["tmux", "-L", socket, "-f", "/dev/null", "attach-session", "-t", session])
os.close(slave)
sys.stdout.write("READY\n"); sys.stdout.flush()
while True:
    readable, _, _ = select.select([sys.stdin, master], [], [], 1.0)
    if sys.stdin in readable:
        data = os.read(sys.stdin.fileno(), 1024)
        if not data:
            break
        os.write(master, data)
    if master in readable:
        try:
            os.read(master, 4096)
        except OSError:
            break
"#;

        struct Server {
            socket: String,
            mux: TmuxMux,
        }

        impl Server {
            async fn boot() -> Self {
                let socket = next_socket();
                let mux = TmuxMux::connect(TmuxConfig {
                    binary: PathBuf::from("tmux"),
                    socket: socket.clone(),
                    pause_after: false,
                })
                .await
                .expect("connect");
                Self { socket, mux }
            }
        }

        impl Drop for Server {
            fn drop(&mut self) {
                drop(
                    std::process::Command::new("tmux")
                        .args(["-L", &self.socket, "-f", "/dev/null", "kill-server"])
                        .status(),
                );
            }
        }

        fn next_socket() -> String {
            static N: AtomicU64 = AtomicU64::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            format!("rig-rs-{}-{n}", std::process::id())
        }

        fn scratch() -> PathBuf {
            let path = std::env::temp_dir().join(next_socket());
            std::fs::create_dir_all(&path).expect("scratch");
            path
        }

        async fn tmux_text(socket: &str, args: &[&str]) -> String {
            let output = Command::new("tmux")
                .arg("-L")
                .arg(socket)
                .arg("-f")
                .arg("/dev/null")
                .args(args)
                .output()
                .await
                .expect("tmux");
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            text
        }

        async fn tmux_ok(socket: &str, args: &[&str]) -> String {
            let output = Command::new("tmux")
                .arg("-L")
                .arg(socket)
                .arg("-f")
                .arg("/dev/null")
                .args(args)
                .output()
                .await
                .expect("tmux");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).into_owned()
        }

        fn target(session: &str) -> String {
            format!("={session}:")
        }

        async fn pane_pid(server: &Server, pane: &PaneRef) -> u32 {
            let panes = server.mux.list_panes().await.expect("panes");
            panes
                .iter()
                .find(|info| Some(&info.pane_id) == pane.pane_id.as_ref())
                .expect("pane")
                .pid
        }

        fn signal(pid: u32, name: &str) {
            let status = std::process::Command::new("kill")
                .arg(format!("-{name}"))
                .arg(pid.to_string())
                .status()
                .expect("kill");
            assert!(status.success(), "kill {name} {pid}");
        }

        async fn wait_event(
            rx: &mut broadcast::Receiver<MuxEvent>,
            timeout: Duration,
            mut pred: impl FnMut(&MuxEvent) -> bool,
        ) -> MuxEvent {
            let deadline = Instant::now() + timeout;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(!remaining.is_zero(), "event timeout");
                match tokio::time::timeout(remaining, rx.recv()).await {
                    Ok(Ok(event)) if pred(&event) => return event,
                    Ok(Ok(_) | Err(broadcast::error::RecvError::Lagged(_))) => {}
                    Ok(Err(broadcast::error::RecvError::Closed)) => panic!("event bus closed"),
                    Err(err) => panic!("event timeout: {err}"),
                }
            }
        }

        async fn wait_death(deaths: &mut DeathStream, timeout: Duration) -> MuxEvent {
            tokio::time::timeout(timeout, deaths.recv())
                .await
                .expect("death timeout")
                .expect("death closed")
        }

        struct Human {
            child: Child,
        }

        impl Human {
            async fn attach(socket: &str, session: &str) -> Self {
                let mut child = Command::new("python3")
                    .arg("-c")
                    .arg(PTY)
                    .arg(socket)
                    .arg(session)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .expect("python");
                let stdout = child.stdout.as_mut().expect("stdout");
                let mut buf = [0_u8; 32];
                let n = tokio::time::timeout(Duration::from_secs(5), stdout.read(&mut buf))
                    .await
                    .expect("ready timeout")
                    .expect("read");
                let text = String::from_utf8_lossy(buf.get(..n).unwrap_or_default());
                assert!(text.contains("READY"), "{text}");
                Self { child }
            }

            async fn key(&mut self, bytes: &[u8]) {
                let stdin = self.child.stdin.as_mut().expect("stdin");
                stdin.write_all(bytes).await.expect("key");
                stdin.flush().await.expect("flush");
            }
        }

        impl Drop for Human {
            fn drop(&mut self) {
                drop(self.child.start_kill());
            }
        }

        async fn wait_humans(server: &Server, session: &str, count: usize) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let humans = server.mux.human_clients(session).await.expect("humans");
                if humans.len() == count {
                    return;
                }
                assert!(Instant::now() < deadline, "humans={}", humans.len());
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }

        fn sleep_spec(name: &str) -> SessionSpec {
            let mut spec = SessionSpec::new(name, std::env::temp_dir());
            spec.argv = vec!["sleep".into(), "120".into()];
            spec
        }

        #[derive(Default)]
        struct ComposerLog {
            buf: Vec<u8>,
            ready: bool,
            pastes: Vec<Vec<u8>>,
            submits: usize,
        }

        impl ComposerLog {
            fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
                self.buf.extend_from_slice(bytes);
                self.drain()
            }

            fn drain(&mut self) -> Result<(), String> {
                loop {
                    if !self.ready {
                        let Some(pos) = find_bytes(&self.buf, b"READY\n") else {
                            return Ok(());
                        };
                        self.buf.drain(..pos + 6);
                        self.ready = true;
                        continue;
                    }
                    if self.buf.first() == Some(&b'S') {
                        if !self.take_submit()? {
                            return Ok(());
                        }
                        continue;
                    }
                    if self.buf.first() == Some(&b'P') {
                        if !self.take_paste()? {
                            return Ok(());
                        }
                        continue;
                    }
                    if self.buf.is_empty() {
                        return Ok(());
                    }
                    return Err(format!(
                        "unexpected composer byte {}",
                        self.buf.first().copied().unwrap_or(0)
                    ));
                }
            }

            fn take_submit(&mut self) -> Result<bool, String> {
                match self.buf.get(1) {
                    None => Ok(false),
                    Some(b'\n') => {
                        self.buf.drain(..2);
                        self.submits += 1;
                        Ok(true)
                    }
                    Some(_) => Err("bad submit".into()),
                }
            }

            fn take_paste(&mut self) -> Result<bool, String> {
                let Some(nl) = self.buf.iter().position(|byte| *byte == b'\n') else {
                    return Ok(false);
                };
                let header = std::str::from_utf8(self.buf.get(..nl).unwrap_or_default())
                    .map_err(|_| "utf8")?;
                let len: usize = header
                    .trim_start_matches("P ")
                    .trim()
                    .parse()
                    .map_err(|_| "len")?;
                let start = nl + 1;
                let end = start + len;
                if self.buf.len() < end + 1 {
                    return Ok(false);
                }
                if self.buf.get(end) != Some(&b'\n') {
                    return Err("paste frame".into());
                }
                let data = self.buf.get(start..end).unwrap_or_default().to_vec();
                self.buf.drain(..=end);
                self.pastes.push(data);
                Ok(true)
            }
        }

        fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
            hay.windows(needle.len())
                .position(|window| window == needle)
        }

        async fn pull(
            stream: &mut OutputStream,
            log: &mut ComposerLog,
            timeout: Duration,
            done: impl Fn(&ComposerLog) -> bool,
        ) {
            let deadline = Instant::now() + timeout;
            while !done(log) {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "composer timeout submits={} pastes={}",
                    log.submits,
                    log.pastes.len()
                );
                match tokio::time::timeout(remaining, stream.recv())
                    .await
                    .expect("read")
                    .expect("closed")
                {
                    MuxEvent::Output { bytes, .. } => log.push(&bytes).expect("protocol"),
                    MuxEvent::Gap { dropped, .. } => panic!("gap {dropped}"),
                    event @ (MuxEvent::PaneDied { .. }
                    | MuxEvent::WindowClosed { .. }
                    | MuxEvent::SessionsChanged
                    | MuxEvent::ServerExited
                    | MuxEvent::Reconnected) => panic!("unexpected {event:?}"),
                }
            }
        }

        async fn composer_pane(
            server: &Server,
            name: &str,
        ) -> (PaneRef, OutputStream, ComposerLog) {
            let bin = env!("CARGO_BIN_EXE_rig-fake-composer");
            let dir = scratch();
            let flag = dir.join("flag");
            // READY is printed only after subscribe. Output from before the
            // window is linked into the control session never arrives.
            let command = format!(
                "stty raw -echo -opost; while [ ! -f {} ]; do sleep 0.05; done; exec {bin} composer",
                flag.display()
            );
            let mut spec = SessionSpec::new(name, dir);
            spec.argv = vec!["sh".into(), "-c".into(), command];
            let pane = server.mux.create_session(&spec).await.expect("composer");
            let mut stream = server.mux.subscribe_output(&pane).await.expect("sub");
            std::fs::write(&flag, b"go").expect("flag");
            let mut log = ComposerLog::default();
            pull(&mut stream, &mut log, Duration::from_secs(5), |log| {
                log.ready
            })
            .await;
            (pane, stream, log)
        }

        fn one_line() -> String {
            "café $HOME `whoami` \"quoted\" 'single'".to_owned()
        }

        fn fifty_lines() -> String {
            (0..50)
                .map(|row| format!("row-{row}\tvalue"))
                .collect::<Vec<_>>()
                .join("\n")
        }

        fn big_payload() -> String {
            let mut out = String::from("α");
            while out.len() < 64 * 1024 {
                out.push('x');
            }
            out
        }

        async fn paste_exact(
            server: &Server,
            pane: &PaneRef,
            stream: &mut OutputStream,
            log: &mut ComposerLog,
            payload: &str,
        ) {
            let pastes = log.pastes.len();
            let submits = log.submits;
            server
                .mux
                .paste(pane, payload, PasteOpts::default())
                .await
                .expect("paste");
            pull(stream, log, Duration::from_secs(5), |log| {
                log.pastes.len() > pastes || log.submits > submits
            })
            .await;
            assert_eq!(log.submits, submits, "premature submit");
            assert_eq!(
                log.pastes.last().map(Vec::as_slice),
                Some(payload.as_bytes())
            );
        }

        #[tokio::test]
        async fn session_names_round_trip() {
            let server = Server::boot().await;
            for (requested, stored) in [
                ("foo.bar", "foo_bar"),
                ("a:b", "a_b"),
                ("=worker", "=worker"),
            ] {
                let pane = server
                    .mux
                    .create_session(&sleep_spec(requested))
                    .await
                    .expect(requested);
                assert_eq!(pane.session, stored);
                assert!(server.mux.has_session(requested).await.expect("has"));
            }
            server
                .mux
                .create_session(&sleep_spec("worker-long"))
                .await
                .expect("long");
            assert!(!server.mux.has_session("worker").await.expect("prefix"));
        }

        #[tokio::test]
        async fn create_session_applies_spec() {
            let server = Server::boot().await;
            let dir = scratch();
            let mut spec = SessionSpec::new("spec", dir.clone());
            spec.env.insert("RIG_MARKER".into(), "zeta".into());
            spec.argv = vec![
                "sh".into(),
                "-c".into(),
                "printf '%s\\n' \"$RIG_MARKER\"; pwd; sleep 30".into(),
            ];
            spec.remain_on_exit = true;
            let pane = server.mux.create_session(&spec).await.expect("create");
            assert!(pane.pane_id.is_some());
            tokio::time::sleep(Duration::from_millis(300)).await;
            let capture = server
                .mux
                .capture(&pane, CaptureRange::Visible)
                .await
                .expect("capture");
            assert!(capture.text.contains("zeta"), "{capture:?}");
            assert!(
                capture.text.contains(&dir.display().to_string()),
                "{capture:?}"
            );
            let panes = server.mux.list_panes().await.expect("panes");
            let info = panes
                .iter()
                .find(|info| info.session == pane.session)
                .expect("info");
            assert_eq!((info.size.cols, info.size.rows), (90, 27));
            let again = server
                .mux
                .capture(&pane, CaptureRange::Visible)
                .await
                .expect("again");
            assert_eq!(capture.hash, again.hash);
            let guard = server.mux.guard_inputs(&pane, 5).await.expect("guard");
            let tail = server
                .mux
                .capture(&pane, CaptureRange::LastLines(5))
                .await
                .expect("tail");
            assert_eq!(guard.tail.hash, tail.hash);
            assert_eq!(guard.tail.text, tail.text);
            assert!(guard.last_own_write.is_none());
        }

        #[tokio::test]
        async fn paste_failure_cleans_buffer() {
            let server = Server::boot().await;
            let bogus = PaneRef {
                session: "missing".into(),
                window: None,
                pane: None,
                pane_id: Some("%99999".into()),
            };
            let err = server
                .mux
                .paste(&bogus, "hi", PasteOpts::default())
                .await
                .expect_err("paste");
            assert!(
                err.to_string().contains("pane") || err.to_string().contains("find"),
                "{err}"
            );
            let buffers = tmux_text(&server.socket, &["list-buffers"]).await;
            assert!(!buffers.contains("rig-"), "{buffers}");
        }

        #[tokio::test]
        async fn paste_reliability() {
            let server = Server::boot().await;
            let (pane, mut stream, mut log) = composer_pane(&server, "composer").await;
            for payload in [one_line(), fifty_lines(), big_payload()] {
                for _ in 0..200 {
                    paste_exact(&server, &pane, &mut stream, &mut log, &payload).await;
                }
            }
            let before = log.submits;
            let bare = PasteOpts {
                bracketed: false,
                preserve_lf: false,
            };
            for _ in 0..3 {
                server
                    .mux
                    .paste(&pane, "one\ntwo\n", bare)
                    .await
                    .expect("negative");
                let seen = log.submits;
                pull(&mut stream, &mut log, Duration::from_secs(5), |log| {
                    log.submits > seen
                })
                .await;
            }
            assert!(log.submits > before, "negative control produced no submits");
        }

        #[tokio::test]
        async fn control_fidelity_10_panes() {
            let server = Server::boot().await;
            let bin = env!("CARGO_BIN_EXE_rig-fake-composer");
            let dir = scratch();
            let mut panes = Vec::new();
            let mut flags = Vec::new();
            let mut outs = Vec::new();
            for index in 0..10 {
                let flag = dir.join(format!("flag-{index}"));
                let out = dir.join(format!("out-{index}"));
                // Sleep after produce: tmux 3.5a drops queued %output when the process exits.
                let command = format!(
                    "stty raw -echo -opost; while [ ! -f {} ]; do sleep 0.05; done; {bin} produce {}; sleep 30",
                    flag.display(),
                    out.display()
                );
                let mut spec = SessionSpec::new(format!("prod-{index}"), dir.clone());
                spec.argv = vec!["sh".into(), "-c".into(), command];
                panes.push(server.mux.create_session(&spec).await.expect("producer"));
                flags.push(flag);
                outs.push(out);
            }
            let mut streams = Vec::new();
            for pane in &panes {
                streams.push(server.mux.subscribe_output(pane).await.expect("sub"));
            }
            let mut readers = tokio::task::JoinSet::new();
            for (mut stream, out) in streams.into_iter().zip(outs) {
                readers.spawn(async move {
                    let got = read_produced(&mut stream, &out).await;
                    let expected = std::fs::read(&out).expect("file");
                    assert_eq!(got, expected, "{}", out.display());
                    got.len()
                });
            }
            let started = Instant::now();
            for flag in &flags {
                tokio::fs::write(flag, b"go").await.expect("flag");
            }
            let mut total = 0_u32;
            while let Some(joined) = readers.join_next().await {
                let len = joined.expect("reader");
                total += u32::try_from(len).expect("len");
            }
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            let per_min = f64::from(total) / elapsed * 60.0;
            assert!(per_min >= 10_000_000.0, "{per_min} bytes/min");
        }

        async fn read_produced(stream: &mut OutputStream, path: &Path) -> Vec<u8> {
            let mut got = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(done) = produced_prefix(&got, path) {
                    return done;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "fidelity timeout at {} bytes",
                    got.len()
                );
                match tokio::time::timeout(remaining, stream.recv())
                    .await
                    .expect("read")
                    .expect("closed")
                {
                    MuxEvent::Output { bytes, .. } => got.extend_from_slice(&bytes),
                    MuxEvent::Gap { dropped, .. } => panic!("gap {dropped}"),
                    event @ (MuxEvent::PaneDied { .. }
                    | MuxEvent::WindowClosed { .. }
                    | MuxEvent::SessionsChanged
                    | MuxEvent::ServerExited
                    | MuxEvent::Reconnected) => panic!("{event:?}"),
                }
            }
        }

        fn produced_prefix(got: &[u8], path: &Path) -> Option<Vec<u8>> {
            if !path.exists() {
                return None;
            }
            let expected = std::fs::read(path).expect("out");
            if got.len() < expected.len() {
                return None;
            }
            Some(got.get(..expected.len()).unwrap_or_default().to_vec())
        }

        #[tokio::test]
        async fn reader_never_blocks() {
            let server = Server::boot().await;
            let bin = env!("CARGO_BIN_EXE_rig-fake-composer");
            let dir = scratch();
            let flag = dir.join("flag");
            let out = dir.join("out");
            // tmux 3.5a drops %output blocks still queued when the pane process
            // exits, so the producer sleeps instead of being exec'd.
            let command = format!(
                "stty raw -echo -opost; while [ ! -f {} ]; do sleep 0.05; done; {bin} produce {}; sleep 30",
                flag.display(),
                out.display()
            );
            let mut spec = SessionSpec::new("burst", dir);
            spec.argv = vec!["sh".into(), "-c".into(), command];
            let pane = server.mux.create_session(&spec).await.expect("pane");
            let mut slow = server.mux.subscribe_output(&pane).await.expect("slow");
            let mut fast = server.mux.subscribe_output(&pane).await.expect("fast");
            let reader = tokio::spawn(async move { read_produced(&mut fast, &out).await });
            std::fs::write(&flag, b"go").expect("flag");
            let got = reader.await.expect("join");
            assert!(got.len() > 64 * 1024, "{}", got.len());
            let mut saw_gap = false;
            for _ in 0..80 {
                match tokio::time::timeout(Duration::from_secs(2), slow.recv()).await {
                    Ok(Some(MuxEvent::Gap { .. })) => {
                        saw_gap = true;
                        break;
                    }
                    Ok(Some(MuxEvent::Output { .. })) => {}
                    Ok(Some(
                        event @ (MuxEvent::PaneDied { .. }
                        | MuxEvent::WindowClosed { .. }
                        | MuxEvent::SessionsChanged
                        | MuxEvent::ServerExited
                        | MuxEvent::Reconnected),
                    )) => panic!("slow subscriber: {event:?}"),
                    Ok(None) => panic!("slow subscriber closed"),
                    Err(err) => panic!("slow subscriber: {err}"),
                }
            }
            assert!(saw_gap);
        }

        #[tokio::test]
        async fn reconnect_after_kill_server() {
            let server = Server::boot().await;
            let mut events = server.mux.events();
            server
                .mux
                .create_session(&sleep_spec("before"))
                .await
                .expect("before");
            let status = std::process::Command::new("tmux")
                .args(["-L", &server.socket, "-f", "/dev/null", "kill-server"])
                .status()
                .expect("kill-server");
            assert!(status.success());
            wait_event(&mut events, Duration::from_secs(5), |event| {
                matches!(event, MuxEvent::ServerExited)
            })
            .await;
            wait_event(&mut events, Duration::from_secs(8), |event| {
                matches!(event, MuxEvent::Reconnected)
            })
            .await;
            let pane = server
                .mux
                .create_session(&sleep_spec("after"))
                .await
                .expect("after");
            let panes = server.mux.list_panes().await.expect("panes");
            assert!(panes.iter().any(|info| info.session == pane.session));
        }

        #[tokio::test]
        async fn death_window_close() {
            let server = Server::boot().await;
            let pane = server
                .mux
                .create_session(&sleep_spec("close"))
                .await
                .expect("create");
            let mut deaths = server.mux.death_events();
            let pid = pane_pid(&server, &pane).await;
            let start = Instant::now();
            signal(pid, "KILL");
            let deadline = start + Duration::from_millis(100);
            let mut saw_died = false;
            let mut saw_closed = false;
            while !(saw_died && saw_closed) {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(!remaining.is_zero(), "death took {:?}", start.elapsed());
                match wait_death(&mut deaths, remaining).await {
                    MuxEvent::PaneDied { .. } => saw_died = true,
                    MuxEvent::WindowClosed { .. } => saw_closed = true,
                    event @ (MuxEvent::Output { .. }
                    | MuxEvent::SessionsChanged
                    | MuxEvent::ServerExited
                    | MuxEvent::Reconnected
                    | MuxEvent::Gap { .. }) => panic!("{event:?}"),
                }
            }
        }

        #[tokio::test]
        async fn death_hook_doorbell_latency() {
            let server = Server::boot().await;
            for trial in 0..15 {
                let elapsed = hook_trial(&server, trial).await;
                assert!(
                    elapsed < Duration::from_millis(100),
                    "trial {trial} took {elapsed:?}"
                );
            }
        }

        async fn hook_trial(server: &Server, trial: u32) -> Duration {
            let mut spec = sleep_spec(&format!("hook-{trial}"));
            // A foreground `sleep` swallows TERM in dash/bash, so the trap
            // never runs. Background sleep plus `wait` lets the shell exit 7.
            spec.argv = vec![
                "bash".into(),
                "-c".into(),
                "trap 'exit 7' TERM; sleep 120 & wait".into(),
            ];
            spec.remain_on_exit = true;
            let mut deaths = server.mux.death_events();
            let pane = server.mux.create_session(&spec).await.expect("create");
            let pid = pane_pid(server, &pane).await;
            let start = Instant::now();
            signal(pid, "TERM");
            let event = wait_death(&mut deaths, Duration::from_millis(500)).await;
            let elapsed = start.elapsed();
            match event {
                MuxEvent::PaneDied { status, pane_id } => {
                    assert_eq!(Some(pane_id).as_ref(), pane.pane_id.as_ref());
                    assert_eq!(status, Some(7), "{elapsed:?}");
                }
                event @ (MuxEvent::Output { .. }
                | MuxEvent::WindowClosed { .. }
                | MuxEvent::SessionsChanged
                | MuxEvent::ServerExited
                | MuxEvent::Reconnected
                | MuxEvent::Gap { .. }) => panic!("{event:?}"),
            }
            elapsed
        }

        #[tokio::test]
        async fn death_backstop() {
            let server = Server::boot().await;
            let mut spec = sleep_spec("back");
            spec.remain_on_exit = true;
            let pane = server.mux.create_session(&spec).await.expect("create");
            let hook_target = target(&pane.session);
            tmux_ok(
                &server.socket,
                &["set-hook", "-u", "-t", &hook_target, "pane-died"],
            )
            .await;
            let hooks = tmux_text(
                &server.socket,
                &["show-hooks", "-t", &hook_target, "pane-died"],
            )
            .await;
            assert!(!hooks.contains("rig-dead"), "{hooks}");
            let mut deaths = server.mux.death_events();
            signal(pane_pid(&server, &pane).await, "KILL");
            let event = wait_death(&mut deaths, Duration::from_secs(3)).await;
            assert!(matches!(event, MuxEvent::PaneDied { .. }), "{event:?}");
            let second = tokio::time::timeout(Duration::from_millis(1500), deaths.recv()).await;
            assert!(second.is_err(), "duplicate death: {second:?}");
        }

        #[tokio::test]
        async fn control_client_not_human() {
            let server = Server::boot().await;
            let control = server
                .mux
                .human_clients(CONTROL_SESSION)
                .await
                .expect("control");
            assert_eq!(control.len(), 0);
            let format = concat!("#{", "session_", "attached", "}");
            let attached = tmux_ok(
                &server.socket,
                &["display", "-p", "-t", "crewmux-control:", format],
            )
            .await;
            assert_eq!(attached.trim(), "1", "{attached}");
            let pane = server
                .mux
                .create_session(&sleep_spec("watched"))
                .await
                .expect("user");
            let _human = Human::attach(&server.socket, &pane.session).await;
            wait_humans(&server, &pane.session, 1).await;
            let humans = server
                .mux
                .human_clients(&pane.session)
                .await
                .expect("humans");
            assert_eq!(humans.len(), 1);
            assert!(!humans.first().expect("human").control_mode);
        }

        #[tokio::test]
        async fn geometry_stable() {
            let server = Server::boot().await;
            let pane = server
                .mux
                .create_session(&sleep_spec("sized"))
                .await
                .expect("create");
            let window = target(&pane.session);
            let before = tmux_ok(
                &server.socket,
                &[
                    "display",
                    "-p",
                    "-t",
                    &window,
                    "#{window_width}x#{window_height}",
                ],
            )
            .await;
            assert_eq!(before.trim(), "90x27", "{before}");
            let _human = Human::attach(&server.socket, &pane.session).await;
            tokio::time::sleep(Duration::from_millis(400)).await;
            let after = tmux_ok(
                &server.socket,
                &[
                    "display",
                    "-p",
                    "-t",
                    &window,
                    "#{window_width}x#{window_height}",
                ],
            )
            .await;
            assert_eq!(after.trim(), "90x27", "{after}");
        }

        #[tokio::test]
        async fn activity_ignores_own_writes() {
            let server = Server::boot().await;
            let pane = server
                .mux
                .create_session(&sleep_spec("keys"))
                .await
                .expect("create");
            let mut human = Human::attach(&server.socket, &pane.session).await;
            wait_humans(&server, &pane.session, 1).await;
            tokio::time::sleep(Duration::from_millis(1200)).await;
            let before = server.mux.guard_inputs(&pane, 2).await.expect("before");
            assert!(before.last_human_keystroke_tmux.is_some());
            assert!(before.last_own_write.is_none());
            server
                .mux
                .paste(&pane, "hello", PasteOpts::default())
                .await
                .expect("paste");
            server
                .mux
                .send_keys(&pane, &[Key::Literal("hi".into()), Key::Enter])
                .await
                .expect("keys");
            let mid = server.mux.guard_inputs(&pane, 2).await.expect("mid");
            assert_eq!(
                mid.last_human_keystroke_tmux,
                before.last_human_keystroke_tmux
            );
            assert!(mid.last_own_write.is_some());
            human.key(b"x").await;
            tokio::time::sleep(Duration::from_millis(1200)).await;
            let after = server.mux.guard_inputs(&pane, 2).await.expect("after");
            assert!(after.last_human_keystroke_tmux > before.last_human_keystroke_tmux);
        }

        #[tokio::test]
        async fn copy_mode_detected() {
            let server = Server::boot().await;
            let pane = server
                .mux
                .create_session(&sleep_spec("copy"))
                .await
                .expect("create");
            assert!(!server.mux.pane_in_mode(&pane).await.expect("mode"));
            let id = pane.pane_id.clone().expect("id");
            tmux_ok(&server.socket, &["copy-mode", "-t", &id]).await;
            assert!(server.mux.pane_in_mode(&pane).await.expect("in copy"));
            tmux_ok(&server.socket, &["send-keys", "-t", &id, "-X", "cancel"]).await;
            assert!(!server.mux.pane_in_mode(&pane).await.expect("left copy"));
        }
    }
}
