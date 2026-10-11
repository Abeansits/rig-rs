// Shared real-tmux helpers, included inside `suite::tmux_it`.

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
                reap_stale_sockets().await;
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
                        .env_remove("TMUX")
                        .env_remove("TMUX_TMPDIR")
                        .status(),
                );
                drop(std::fs::remove_file(socket_file(&self.socket)));
            }
        }

        fn socket_dir() -> PathBuf {
            PathBuf::from(format!("/tmp/tmux-{}", rustix::process::getuid().as_raw()))
        }

        fn socket_file(name: &str) -> PathBuf {
            socket_dir().join(name)
        }

        async fn reap_stale_sockets() {
            let dir = socket_dir();
            let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
                return;
            };
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if !name.starts_with("rig-rs-") {
                    continue;
                }
                let Some(pid) = owner_pid(name) else {
                    continue;
                };
                if !process_dead(pid) {
                    continue;
                }
                reap_one(name, &entry.path()).await;
            }
        }

        fn owner_pid(name: &str) -> Option<u32> {
            let rest = name.strip_prefix("rig-rs-")?;
            let token = rest.split('-').next()?;
            let pid = token.parse::<u32>().ok()?;
            if pid <= 1 { None } else { Some(pid) }
        }

        fn process_dead(pid: u32) -> bool {
            let Ok(raw) = i32::try_from(pid) else {
                return false;
            };
            let Some(pid) = rustix::process::Pid::from_raw(raw) else {
                return false;
            };
            matches!(
                rustix::process::test_kill_process(pid),
                Err(err) if err == rustix::io::Errno::SRCH
            )
        }

        #[test]
        fn stale_socket_owner_is_the_pid_in_the_name() {
            assert_eq!(owner_pid("rig-rs-42-7"), Some(42));
            assert_eq!(owner_pid("rig-rs-42"), Some(42));
            assert_eq!(owner_pid("rig-rs"), None);
            assert_eq!(owner_pid("rig-rs-0"), None);
            assert_eq!(owner_pid("rig-rs-1"), None);
            assert!(!process_dead(std::process::id()));
        }

        async fn reap_one(name: &str, path: &Path) {
            let output = Command::new("tmux")
                .args(["-L", name, "-f", "/dev/null", "list-sessions"])
                .env_remove("TMUX")
                .env_remove("TMUX_TMPDIR")
                .output()
                .await;
            let Ok(output) = output else {
                return;
            };
            let stderr = String::from_utf8_lossy(&output.stderr);
            let dead = !output.status.success() && stderr.contains("no server running");
            if !dead {
                drop(
                    Command::new("tmux")
                        .args(["-L", name, "-f", "/dev/null", "kill-server"])
                        .env_remove("TMUX")
                        .env_remove("TMUX_TMPDIR")
                        .output()
                        .await,
                );
            }
            drop(tokio::fs::remove_file(path).await);
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
                .expect("live pane pid")
        }

        fn signal(pid: u32, name: &str) {
            assert!(pid > 0, "refusing to signal pid 0");
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
            // Drain while tmux echoes the paste. Waiting until paste() returns
            // lets the subscriber queue fill under load.
            let paste = server.mux.paste(pane, payload, PasteOpts::default());
            let read = pull(stream, log, Duration::from_secs(5), |log| {
                log.pastes.len() > pastes || log.submits > submits
            });
            let (receipt, ()) = tokio::join!(paste, read);
            receipt.expect("paste");
            assert_eq!(log.submits, submits, "premature submit");
            assert_eq!(
                log.pastes.last().map(Vec::as_slice),
                Some(payload.as_bytes())
            );
        }

