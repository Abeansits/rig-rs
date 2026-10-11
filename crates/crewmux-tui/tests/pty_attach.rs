//! Real pty attach/detach. Built only with `--features tmux-it`.
//!
//! The nextest `::tmux_it::` filter keeps this off the default local run.
//! `tmux-matrix` and the ci profile run it.

#![cfg(feature = "tmux-it")]

mod tmux_it {
    use std::fs::{self, File};
    use std::io::{self, Read, Write};
    use std::os::fd::{AsFd, OwnedFd};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use crewmux_api::{
        AttachInfo, PodName, RigId, RigName, RuntimeKind, SeatId, SeatName, SeatState, SeatView,
        SessionId, Snapshot, daemon_bind_path, tui_socket_path,
    };
    use nix::poll::{PollFd, PollFlags, poll};
    use nix::pty::openpty;
    use nix::sys::signal::{Signal, kill};
    use nix::sys::termios::{LocalFlags, Termios, tcgetattr};
    use nix::unistd::Pid;
    use parking_lot::Mutex;

    #[test]
    fn fifty_attach_detach_cycles_restore_the_terminal() {
        if let Err(err) = drive_fifty() {
            panic!("{err}");
        }
    }

    #[test]
    fn daemon_drop_reconnects() {
        if let Err(err) = drive_reconnect() {
            panic!("{err}");
        }
    }

    struct Screen {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Screen {
        fn contains(&self, needle: &str) -> bool {
            String::from_utf8_lossy(&self.bytes.lock()).contains(needle)
        }

        fn len(&self) -> usize {
            self.bytes.lock().len()
        }

        fn since(&self, mark: usize) -> Vec<u8> {
            self.bytes.lock().get(mark..).unwrap_or(&[]).to_vec()
        }
    }

    struct Daemon {
        runtime: Option<tokio::runtime::Runtime>,
        cancel: tokio_util::sync::CancellationToken,
        task: tokio::task::JoinHandle<()>,
        path: PathBuf,
        sock_name: String,
    }

    impl Daemon {
        fn start(path: &Path, sock_name: &str) -> Result<Self, String> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            }
            drop(fs::remove_file(path));
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .map_err(|err| err.to_string())?;
            let bound = path.to_path_buf();
            let listener = runtime
                .block_on(async move { tokio::net::UnixListener::bind(bound) })
                .map_err(|err| err.to_string())?;
            let cancel = tokio_util::sync::CancellationToken::new();
            let shutdown = cancel.clone();
            let name = sock_name.to_owned();
            let app = router(&name)?;
            let task = runtime.spawn(async move {
                drop(
                    axum::serve(listener, app)
                        .with_graceful_shutdown(async move { shutdown.cancelled().await })
                        .await,
                );
            });
            Ok(Self {
                runtime: Some(runtime),
                cancel,
                task,
                path: path.to_path_buf(),
                sock_name: sock_name.to_owned(),
            })
        }
    }

    impl Drop for Daemon {
        fn drop(&mut self) {
            self.cancel.cancel();
            self.task.abort();
            if let Some(runtime) = self.runtime.take() {
                runtime.shutdown_background();
            }
            drop(fs::remove_file(&self.path));
        }
    }

    fn router(sock_name: &str) -> Result<axum::Router, String> {
        let seat = SeatId::parse("s1").map_err(|err| err.to_string())?;
        let session = SessionId::parse("seat").map_err(|err| err.to_string())?;
        let view = SeatView {
            id: seat.clone(),
            name: SeatName::parse("alpha").map_err(|err| err.to_string())?,
            rig_id: RigId::parse("rig-1").map_err(|err| err.to_string())?,
            rig_name: RigName::parse("demo").map_err(|err| err.to_string())?,
            pod: PodName::parse("pod").map_err(|err| err.to_string())?,
            state: SeatState::Ready,
            runtime: RuntimeKind::ClaudeCode,
            session: Some(session.clone()),
            spec: None,
        };
        let fixture = Fixture {
            snapshot: Snapshot {
                seats: vec![view],
                health: None,
                activity: vec![],
            },
            attach: AttachInfo {
                seat,
                socket: sock_name.to_owned(),
                session,
            },
        };
        Ok(axum::Router::new()
            .route("/api/snapshot", axum::routing::get(snapshot))
            .route("/api/seats", axum::routing::get(seats))
            .route("/api/seats/{seat}/attach", axum::routing::get(attach))
            .route("/api/events", axum::routing::get(events))
            .with_state(fixture))
    }

    #[derive(Clone)]
    struct Fixture {
        snapshot: Snapshot,
        attach: AttachInfo,
    }

    async fn snapshot(
        axum::extract::State(fixture): axum::extract::State<Fixture>,
    ) -> axum::Json<Snapshot> {
        axum::Json(fixture.snapshot)
    }

    async fn seats(
        axum::extract::State(fixture): axum::extract::State<Fixture>,
    ) -> axum::Json<crewmux_api::SeatList> {
        axum::Json(crewmux_api::SeatList {
            seats: fixture.snapshot.seats,
        })
    }

    async fn attach(
        axum::extract::State(fixture): axum::extract::State<Fixture>,
        axum::extract::Path(seat): axum::extract::Path<String>,
    ) -> Result<axum::Json<AttachInfo>, axum::http::StatusCode> {
        if seat == fixture.attach.seat.as_str() {
            Ok(axum::Json(fixture.attach))
        } else {
            Err(axum::http::StatusCode::NOT_FOUND)
        }
    }

    async fn events() -> axum::response::sse::Sse<
        impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>
        + Send,
    > {
        use axum::response::sse::Event;
        use futures_util::stream::{self, StreamExt};
        let event = Event::default()
            .id("1")
            .json_data(serde_json::json!({
                "id": 1,
                "type": "activity",
                "payload": {"seat": "s1", "summary": "ok"}
            }))
            .unwrap_or_else(|_| Event::default().data("{}"));
        let once = stream::once(async move { Ok(event) });
        let rest = stream::pending::<Result<Event, std::convert::Infallible>>();
        axum::response::sse::Sse::new(once.chain(rest))
    }

    struct Pty {
        child: Child,
        master: File,
        slave: OwnedFd,
        screen: Screen,
        stop: Arc<AtomicBool>,
        reader: Option<JoinHandle<()>>,
    }

    impl Drop for Pty {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            drop(self.child.kill());
            drop(self.child.wait());
            if let Some(reader) = self.reader.take() {
                drop(reader.join());
            }
        }
    }

    fn open_rig(home: &Path, sock: &str) -> Result<Pty, String> {
        let winsize = nix::pty::Winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let pair = openpty(Some(&winsize), None::<&Termios>).map_err(|err| err.to_string())?;
        let slave_in = pair.slave.try_clone().map_err(|err| err.to_string())?;
        let slave_out = pair.slave.try_clone().map_err(|err| err.to_string())?;
        let slave_err = pair.slave.try_clone().map_err(|err| err.to_string())?;
        let read_fd = pair.master.try_clone().map_err(|err| err.to_string())?;
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let reader = thread::spawn({
            let bytes = Arc::clone(&bytes);
            let stop = Arc::clone(&stop);
            move || pump(read_fd, bytes, stop)
        });
        let mut command = Command::new(rig_bin());
        command
            .arg("tui")
            .env("RIG_HOME", home)
            .env("RIG_TMUX_SOCKET", sock)
            .env("TERM", "xterm-256color")
            .env_remove("TMUX")
            .env_remove("RIG_LAUNCHER")
            .stdin(Stdio::from(slave_in))
            .stdout(Stdio::from(slave_out))
            .stderr(Stdio::from(slave_err));
        let child = command.spawn().map_err(|err| err.to_string())?;
        Ok(Pty {
            child,
            master: File::from(pair.master),
            slave: pair.slave,
            screen: Screen { bytes },
            stop,
            reader: Some(reader),
        })
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the reader thread owns the buffer and the stop flag"
    )]
    fn pump(fd: OwnedFd, bytes: Arc<Mutex<Vec<u8>>>, stop: Arc<AtomicBool>) {
        let mut file = File::from(fd);
        let mut tmp = [0_u8; 4096];
        while !stop.load(Ordering::Relaxed) {
            let mut fds = [PollFd::new(file.as_fd(), PollFlags::POLLIN)];
            let ready = poll(&mut fds, 50_u16).unwrap_or(0);
            if ready <= 0 {
                continue;
            }
            match file.read(&mut tmp) {
                Ok(0) | Err(_) => break,
                Ok(n) => bytes.lock().extend(tmp.get(..n).unwrap_or(&[])),
            }
        }
    }

    fn rig_bin() -> PathBuf {
        if let Some(path) = std::env::var_os("RIG_BIN") {
            return PathBuf::from(path);
        }
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR") {
            return PathBuf::from(dir).join(profile).join("rig");
        }
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("target")
            .join(profile)
            .join("rig")
    }

    fn current_uid() -> u32 {
        let Ok(text) = fs::read_to_string("/proc/self/status") else {
            return 0;
        };
        text.lines()
            .find_map(|line| line.strip_prefix("Uid:"))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(0)
    }

    struct TmuxOut {
        ok: bool,
        stdout: String,
    }

    fn tmux(sock: &str, args: &[&str]) -> io::Result<TmuxOut> {
        let mut command = Command::new("tmux");
        command.arg("-L").arg(sock).arg("-f").arg("/dev/null");
        command.args(args);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let mut stdout = String::new();
        if let Some(mut pipe) = child.stdout.take() {
            pipe.read_to_string(&mut stdout)?;
        }
        let status = child.wait()?;
        Ok(TmuxOut {
            ok: status.success(),
            stdout,
        })
    }

    struct Server {
        sock: String,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            drop(tmux(&self.sock, &["kill-server"]));
        }
    }

    fn wait_for(screen: &Screen, needle: &str, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if screen.contains(needle) {
                return true;
            }
            thread::park_timeout(Duration::from_millis(20));
        }
        false
    }

    fn wait_pred(pred: impl Fn() -> bool, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if pred() {
                return true;
            }
            thread::park_timeout(Duration::from_millis(20));
        }
        false
    }

    fn redraws(bytes: &[u8]) -> bool {
        let text = String::from_utf8_lossy(bytes);
        [
            "\u{1b}[?1049h",
            "\u{1b}[?1000h",
            "\u{1b}[?1002h",
            "\u{1b}[?1003h",
            "\u{1b}[?2004h",
        ]
        .iter()
        .all(|seq| text.contains(seq))
    }

    /// Force a real size change. A `SIGWINCH` that leaves the column count
    /// unchanged is a no-op diff, so shrink to 80x24 before growing to 100x30.
    fn wait_for_wide(pty: &Pty) -> Result<(), String> {
        let mut last = String::new();
        for _ in 0..4 {
            set_winsize(pty, 80, 24)?;
            thread::park_timeout(Duration::from_millis(200));
            set_winsize(pty, 100, 30)?;
            let wide = pty.screen.len();
            if wait_pred(
                || wider_than_80(&pty.screen.since(wide)),
                Duration::from_secs(2),
            ) {
                return Ok(());
            }
            last = String::from_utf8_lossy(&pty.screen.since(wide)).into_owned();
        }
        Err(last)
    }

    fn session_gone(screen: &Screen) -> bool {
        let bytes = screen.bytes.lock();
        let text = String::from_utf8_lossy(&bytes);
        text.contains("session is gone") || (text.contains("sessi") && text.contains("n is gone"))
    }

    fn wider_than_80(bytes: &[u8]) -> bool {
        let text = String::from_utf8_lossy(bytes);
        let owned = text.into_owned();
        let mut rest = owned.as_str();
        while let Some(pos) = rest.find(';') {
            let Some(after) = rest.get(pos.saturating_add(1)..) else {
                break;
            };
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty()
                && digits.parse::<u16>().ok() > Some(80)
                && after
                    .get(digits.len()..)
                    .is_some_and(|tail| tail.starts_with('H') || tail.starts_with('G'))
            {
                return true;
            }
            rest = after;
        }
        false
    }

    fn is_raw(slave: &OwnedFd) -> bool {
        tcgetattr(slave).is_ok_and(|termios| !termios.local_flags.contains(LocalFlags::ICANON))
    }

    fn mode_of(slave: &OwnedFd) -> Result<Termios, String> {
        tcgetattr(slave).map_err(|err| err.to_string())
    }

    fn children_of(pid: u32) -> Vec<u32> {
        if let Ok(text) = fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")) {
            return text
                .split_whitespace()
                .filter_map(|raw| raw.parse().ok())
                .collect();
        }
        let Ok(mut child) = Command::new("ps")
            .args(["-o", "pid=", "-ppid", &pid.to_string()])
            .stdout(Stdio::piped())
            .spawn()
        else {
            return Vec::new();
        };
        let mut stdout = String::new();
        if let Some(mut pipe) = child.stdout.take() {
            drop(pipe.read_to_string(&mut stdout));
        }
        drop(child.wait());
        stdout
            .split_whitespace()
            .filter_map(|raw| raw.parse().ok())
            .collect()
    }

    fn select_seat(path: &Path) -> Result<(), String> {
        use std::io::{BufRead, BufReader};
        let mut stream =
            std::os::unix::net::UnixStream::connect(path).map_err(|err| err.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|err| err.to_string())?;
        stream
            .write_all(b"{\"cmd\":\"select\",\"seat\":\"s1\"}\n")
            .map_err(|err| err.to_string())?;
        let mut line = String::new();
        BufReader::new(&stream)
            .read_line(&mut line)
            .map_err(|err| err.to_string())?;
        if !line.contains("\"ok\":true") {
            return Err(format!("select reply {line}"));
        }
        Ok(())
    }

    fn has_client(sock: &str) -> bool {
        tmux(sock, &["list-clients", "-t", "seat"]).is_ok_and(|out| !out.stdout.trim().is_empty())
    }

    fn press(pty: &mut Pty, bytes: &[u8]) -> Result<(), String> {
        pty.master.write_all(bytes).map_err(|err| err.to_string())
    }

    fn set_winsize(pty: &Pty, cols: u16, rows: u16) -> Result<(), String> {
        rustix::termios::tcsetwinsize(
            &pty.master,
            rustix::termios::Winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .map_err(|err| err.to_string())?;
        let pid = i32::try_from(pty.child.id()).map_err(|err| err.to_string())?;
        kill(Pid::from_raw(pid), Signal::SIGWINCH).map_err(|err| err.to_string())
    }

    fn drive_fifty() -> Result<(), String> {
        let home = tempfile::tempdir().map_err(|err| err.to_string())?;
        let sock = format!("r{}", std::process::id());
        let _server = Server { sock: sock.clone() };
        let started = tmux(
            &sock,
            &["new-session", "-d", "-s", "seat", "-x", "80", "-y", "24"],
        )
        .map_err(|err| err.to_string())?;
        if !started.ok {
            return Err("tmux new-session failed".to_owned());
        }
        let uid = current_uid();
        let daemon_path = daemon_bind_path(home.path(), &std::env::temp_dir(), uid);
        let control = tui_socket_path(home.path(), &std::env::temp_dir(), uid);
        let daemon = Daemon::start(&daemon_path, &sock)?;
        let mut pty = open_rig(home.path(), &sock)?;
        if !pty.screen.contains("alpha") && !wait_for(&pty.screen, "alpha", Duration::from_secs(8))
        {
            return Err(format!("no seat row: {}", dump(&pty.screen)));
        }
        select_seat(&control)?;
        let baseline = mode_of(&pty.slave)?;
        if baseline.local_flags.contains(LocalFlags::ICANON) {
            return Err("pre-attach termios is not raw".to_owned());
        }
        for cycle in 1..=50 {
            one_cycle(&mut pty, &sock, cycle)?;
        }
        let after = mode_of(&pty.slave)?;
        if after.local_flags != baseline.local_flags
            || after.input_flags != baseline.input_flags
            || after.output_flags != baseline.output_flags
            || after.control_flags != baseline.control_flags
            || after.control_chars != baseline.control_chars
        {
            return Err("termios drifted from the pre-attach baseline".to_owned());
        }
        let pid = pty.child.id();
        if !wait_pred(|| children_of(pid).is_empty(), Duration::from_secs(2)) {
            return Err(format!("tui still has children {:?}", children_of(pid)));
        }
        press(&mut pty, b"a")?;
        if !wait_pred(|| has_client(&sock), Duration::from_secs(3)) {
            press(&mut pty, b"a")?;
        }
        if !wait_pred(|| has_client(&sock), Duration::from_secs(3)) {
            return Err("session-gone attach found no client".to_owned());
        }
        let killed = tmux(&sock, &["kill-session", "-t", "seat"]).map_err(|err| err.to_string())?;
        if !killed.ok {
            return Err("kill-session failed".to_owned());
        }
        if !wait_pred(|| session_gone(&pty.screen), Duration::from_secs(5)) {
            return Err(format!("missing notice: {}", dump(&pty.screen)));
        }
        press(&mut pty, b"q")?;
        let status = wait_exit(&mut pty.child, Duration::from_secs(5))?;
        if !status.success() {
            return Err(format!("q exited {status}"));
        }
        drop(daemon);
        Ok(())
    }

    fn one_cycle(pty: &mut Pty, sock: &str, cycle: u16) -> Result<(), String> {
        let mark = pty.screen.len();
        press(pty, b"a")?;
        if !wait_pred(|| has_client(sock), Duration::from_secs(3)) {
            press(pty, b"a")?;
            if !wait_pred(|| has_client(sock), Duration::from_secs(3)) {
                return Err(format!(
                    "cycle {cycle}: no tmux client\n{}",
                    dump(&pty.screen)
                ));
            }
        }
        let detached =
            tmux(sock, &["detach-client", "-s", "seat"]).map_err(|err| err.to_string())?;
        if !detached.ok && has_client(sock) {
            return Err(format!("cycle {cycle}: detach failed"));
        }
        let redrawn = wait_pred(
            || redraws(&pty.screen.since(mark)) && is_raw(&pty.slave),
            Duration::from_secs(5),
        );
        if !redrawn {
            return Err(format!(
                "cycle {cycle}: redraw missing raw={} dump={}",
                is_raw(&pty.slave),
                dump(&pty.screen)
            ));
        }
        if cycle == 10
            && let Err(tail) = wait_for_wide(pty)
        {
            let size = rustix::termios::tcgetwinsize(&pty.slave)
                .map_or((0, 0), |size| (size.ws_col, size.ws_row));
            return Err(format!(
                "cycle 10 stayed within 80 columns size={size:?}\n{tail}"
            ));
        }
        Ok(())
    }

    fn drive_reconnect() -> Result<(), String> {
        let home = tempfile::tempdir().map_err(|err| err.to_string())?;
        let sock = format!("q{}", std::process::id());
        let _server = Server { sock: sock.clone() };
        let started = tmux(
            &sock,
            &["new-session", "-d", "-s", "seat", "-x", "80", "-y", "24"],
        )
        .map_err(|err| err.to_string())?;
        if !started.ok {
            return Err("tmux new-session failed".to_owned());
        }
        let uid = current_uid();
        let daemon_path = daemon_bind_path(home.path(), &std::env::temp_dir(), uid);
        let daemon = Daemon::start(&daemon_path, &sock)?;
        let mut pty = open_rig(home.path(), &sock)?;
        if !wait_for(&pty.screen, "connected", Duration::from_secs(8)) {
            return Err(format!("never connected: {}", dump(&pty.screen)));
        }
        let path = daemon.path.clone();
        let name = daemon.sock_name.clone();
        drop(daemon);
        thread::park_timeout(Duration::from_millis(50));
        let dropped = wait_pred(
            || {
                pty.screen.contains("dropped")
                    || pty.screen.contains("reconnecting")
                    || pty.screen.contains(" stale")
            },
            Duration::from_secs(6),
        );
        if !dropped {
            return Err(format!("no dropped status: {}", dump(&pty.screen)));
        }
        let mark = pty.screen.len();
        let _daemon = Daemon::start(&path, &name)?;
        let back = wait_pred(
            || String::from_utf8_lossy(&pty.screen.since(mark)).contains("connected"),
            Duration::from_secs(8),
        );
        if !back {
            return Err(format!("did not reconnect: {}", dump(&pty.screen)));
        }
        press(&mut pty, b"q")?;
        let status = wait_exit(&mut pty.child, Duration::from_secs(5))?;
        if !status.success() {
            return Err(format!("q exited {status}"));
        }
        Ok(())
    }

    fn wait_exit(child: &mut Child, timeout: Duration) -> Result<ExitStatus, String> {
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
                return Ok(status);
            }
            if start.elapsed() > timeout {
                return Err("tui did not exit".to_owned());
            }
            thread::park_timeout(Duration::from_millis(30));
        }
    }

    fn dump(screen: &Screen) -> String {
        let bytes = screen.bytes.lock();
        let start = bytes.len().saturating_sub(400);
        String::from_utf8_lossy(bytes.get(start..).unwrap_or(&[])).to_string()
    }
}
