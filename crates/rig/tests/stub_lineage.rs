//! Stub seats refuse a respawned or exec'd pane. Real tmux, private socket.
#![cfg_attr(
    feature = "tmux-it",
    expect(
        clippy::expect_used,
        clippy::panic,
        reason = "setup failures should abort the integration test"
    )
)]
#![cfg(feature = "tmux-it")]

mod live {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use assert_cmd::cargo::CommandCargoExt;

    pub(crate) mod tmux_it {
        #[test]
        fn normal_stub_seat_still_delivers() {
            super::normal_stub_seat_still_delivers();
        }

        #[test]
        fn respawn_pane_delivers_nothing() {
            super::respawn_pane_delivers_nothing();
        }

        #[test]
        fn exec_into_bash_delivers_nothing() {
            super::exec_into_bash_delivers_nothing();
        }
    }

    const MARKER: &str = "zz-stub-lineage-9f3a";

    static SOCKETS: AtomicU64 = AtomicU64::new(0);

    struct Live {
        root: tempfile::TempDir,
        home: PathBuf,
        rig_home: PathBuf,
        socket: String,
    }

    impl Live {
        fn new() -> Self {
            let root = tempfile::TempDir::with_prefix("rig-stub-lin-").expect("temp");
            let home = root.path().join("home");
            let rig_home = root.path().join("rig");
            std::fs::create_dir_all(&home).expect("home");
            std::fs::create_dir_all(&rig_home).expect("rig");
            let n = SOCKETS.fetch_add(1, Ordering::Relaxed);
            let socket = format!("rig-rs-{}-{n}-lin", std::process::id());
            kill_server(&socket);
            Self {
                root,
                home,
                rig_home,
                socket,
            }
        }

        fn rig(&self) -> Command {
            let mut command = Command::cargo_bin("rig").expect("rig");
            command
                .env("HOME", &self.home)
                .env("RIG_HOME", &self.rig_home)
                .env("RIG_TMUX_SOCKET", &self.socket)
                .env_remove("RIG_FAKE_MUX")
                .env_remove("RIG_STUB_BIN")
                .env_remove("RIG_STUB_SCRIPT")
                .env_remove("TMUX")
                .env_remove("TMUX_TMPDIR")
                .current_dir(&self.home);
            command
        }
    }

    impl Drop for Live {
        fn drop(&mut self) {
            let socket = crewmux_api::daemon_socket(&self.rig_home);
            let client = crewmux_api::Client::over_uds(socket).with_timeout(Duration::from_secs(2));
            drop(client.request("POST", "/shutdown", Some(b"{}")));
            kill_server(&self.socket);
        }
    }

    fn normal_stub_seat_still_delivers() {
        let _stub = Command::cargo_bin("rig-stub-agent").expect("stub binary");
        let live = Live::new();
        boot(&live, None);
        wait_prompt(&live);
        let before = history(&live);
        let (code, out, err) = send(&live);
        assert_eq!(code, 0, "{err} {out}");
        let after = history(&live);
        assert!(after.contains(MARKER), "{after}");
        assert!(delivered(&before, &after) > 0, "{after}");
    }

    fn respawn_pane_delivers_nothing() {
        let _stub = Command::cargo_bin("rig-stub-agent").expect("stub binary");
        let live = Live::new();
        boot(&live, None);
        wait_prompt(&live);
        let (pane, pid) = pane_of(&live);
        let script = live.root.path().join("lookalike.py");
        std::fs::write(
            &script,
            "import sys, time\nsys.stdout.write('stub-agent\\nstub> \\n')\nsys.stdout.flush()\nwhile True:\n    time.sleep(30)\n",
        )
        .expect("script");
        let respawn = tmux(
            &live.socket,
            &[
                "respawn-pane",
                "-k",
                "-t",
                &pane,
                "--",
                "python3",
                script.to_str().expect("script"),
            ],
        );
        assert_eq!(respawn.0, 0, "{}", respawn.2);
        wait_until(&live, |shot, now| now != pid && shot.contains("stub> "));
        let before = history(&live);
        let (code, out, err) = send(&live);
        assert_ne!(code, 0, "{out}");
        assert!(err.contains("lineage:"), "{err} {out}");
        assert_eq!(delivered(&before, &history(&live)), 0, "{}", history(&live));
    }

    fn exec_into_bash_delivers_nothing() {
        let live = Live::new();
        let script = live.root.path().join("exec-bash.py");
        std::fs::write(
            &script,
            "#!/usr/bin/env python3\nimport os, sys, time\nsys.stdout.write('stub-agent\\nstub> \\n')\nsys.stdout.flush()\ntrigger = os.environ.get('RIG_STUB_SCRIPT', '')\ndeadline = time.time() + 25\nwhile trigger and not os.path.exists(trigger):\n    if time.time() > deadline:\n        raise SystemExit('trigger')\n    time.sleep(0.05)\nos.execv('/bin/bash', ['bash'])\n",
        )
        .expect("script");
        let mut perms = std::fs::metadata(&script).expect("meta").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&script, perms).expect("chmod");
        let trigger = live.root.path().join("go");
        boot(&live, Some((script.as_path(), trigger.as_path())));
        wait_prompt(&live);
        let (_pane, pid) = pane_of(&live);
        std::fs::write(&trigger, "1").expect("trigger");
        wait_image(pid, "bash");
        let before = history(&live);
        let (code, out, err) = send(&live);
        assert_ne!(code, 0, "{out}");
        assert!(err.contains("lineage:"), "{err} {out}");
        assert_eq!(delivered(&before, &history(&live)), 0, "{}", history(&live));
    }

    fn boot(live: &Live, stub: Option<(&Path, &Path)>) {
        let cwd = live.home.display();
        let spec = live.home.join("one-stub.yaml");
        std::fs::write(
            &spec,
            format!("name: pair\nseats:\n  - name: one\n    runtime: stub\n    cwd: {cwd}\n"),
        )
        .expect("spec");
        let mut command = live.rig();
        if let Some((bin, trigger)) = stub {
            command
                .env("RIG_STUB_BIN", bin)
                .env("RIG_STUB_SCRIPT", trigger);
        }
        let up = wait_output(command.args(["up", "one-stub.yaml", "--json"]));
        assert_eq!(up.0, 0, "{} {}", up.2, up.1);
        assert!(up.1.contains("\"ready\""), "{}", up.1);
    }

    fn send(live: &Live) -> (i32, String, String) {
        wait_output(live.rig().args(["send", "--to", "one@pair", MARKER]))
    }

    fn delivered(before: &str, after: &str) -> usize {
        after
            .matches(MARKER)
            .count()
            .saturating_sub(before.matches(MARKER).count())
            .saturating_mul(MARKER.len())
    }

    fn wait_prompt(live: &Live) {
        wait_until(live, |shot, _pid| shot.contains("stub> "));
    }

    fn wait_until(live: &Live, ready: impl Fn(&str, u32) -> bool) {
        let start = Instant::now();
        let mut last = String::new();
        while start.elapsed() < Duration::from_secs(20) {
            let shot = history(live);
            let pid = pane_of(live).1;
            if ready(&shot, pid) {
                return;
            }
            last = shot;
            std::thread::park_timeout(Duration::from_millis(50));
        }
        panic!("pane did not become ready: {last}");
    }

    fn wait_image(pid: u32, want: &str) {
        let start = Instant::now();
        let mut last = String::new();
        while start.elapsed() < Duration::from_secs(20) {
            last = process_image(pid);
            if image_base(&last) == want {
                return;
            }
            std::thread::park_timeout(Duration::from_millis(50));
        }
        panic!("pid {pid} image is {last:?}, wanted {want}");
    }

    /// Linux names the image in `/proc`. macOS does not, so `ps` is the fallback.
    fn process_image(pid: u32) -> String {
        let proc = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        let proc = proc.trim();
        if !proc.is_empty() {
            return proc.to_owned();
        }
        ps_comm(pid)
    }

    fn ps_comm(pid: u32) -> String {
        let mut command = Command::new("ps");
        command
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .env("LC_ALL", "C")
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let Ok(child) = command.spawn() else {
            return String::new();
        };
        let Ok(output) = child.wait_with_output() else {
            return String::new();
        };
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn image_base(command: &str) -> &str {
        let name = command.rsplit(['/', '\\']).next().unwrap_or(command);
        name.strip_prefix('-').unwrap_or(name)
    }

    fn pane_of(live: &Live) -> (String, u32) {
        let shot = tmux(
            &live.socket,
            &[
                "list-panes",
                "-t",
                "one__pair",
                "-F",
                "#{pane_id}\t#{pane_pid}",
            ],
        );
        assert_eq!(shot.0, 0, "{}", shot.2);
        let row = shot.1.lines().next().unwrap_or_default();
        let (id, pid) = row.split_once('\t').unwrap_or((row, ""));
        let pid = pid.trim().parse().unwrap_or(0);
        assert_ne!(pid, 0, "{}", shot.1);
        (id.to_owned(), pid)
    }

    fn history(live: &Live) -> String {
        let shot = tmux(
            &live.socket,
            &["capture-pane", "-p", "-N", "-S", "-", "-t", "one__pair"],
        );
        assert_eq!(shot.0, 0, "{}", shot.2);
        shot.1
    }

    fn kill_server(socket: &str) {
        if !crewmux_mux::socket_owned_by(socket, std::process::id()) {
            return;
        }
        let mut kill = Command::new("tmux");
        kill.args(["-L", socket, "-f", "/dev/null", "kill-server"])
            .env_remove("TMUX")
            .env_remove("TMUX_TMPDIR")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Ok(mut child) = kill.spawn() {
            drop(child.wait());
        }
        drop(std::fs::remove_file(crewmux_mux::socket_path(socket)));
    }

    fn wait_output(command: &mut Command) -> (i32, String, String) {
        let output = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn")
            .wait_with_output()
            .expect("wait");
        (
            output.status.code().unwrap_or(1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    fn tmux(socket: &str, args: &[&str]) -> (i32, String, String) {
        let mut command = Command::new("tmux");
        command
            .arg("-L")
            .arg(socket)
            .arg("-f")
            .arg("/dev/null")
            .args(args)
            .env_remove("TMUX")
            .env_remove("TMUX_TMPDIR")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = command
            .spawn()
            .expect("tmux")
            .wait_with_output()
            .expect("tmux");
        (
            output.status.code().unwrap_or(1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}
