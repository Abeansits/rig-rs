//! Real-tmux acceptance checks that `FakeMux` cannot prove.
#![cfg_attr(
    feature = "tmux-it",
    expect(
        clippy::expect_used,
        reason = "setup failures should abort the integration test"
    )
)]
#![cfg(feature = "tmux-it")]

mod live {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant, SystemTime};

    use assert_cmd::cargo::CommandCargoExt;
    use serde_json::Value;

    pub(crate) mod tmux_it {
        #[test]
        fn ps_after_down_shows_stopped() {
            super::stopped_after_down();
        }

        #[test]
        fn second_up_is_already_running_on_tmux() {
            super::second_up();
        }

        #[test]
        fn ps_is_unattached() {
            super::unattached();
        }

        #[test]
        fn attach_uses_the_server_socket_under_tmux_tmpdir() {
            super::attach_socket();
        }

        #[test]
        fn exit_after_marks_dead_on_tmux() {
            super::exit_after();
        }

        #[test]
        fn invalid_cwd_is_partial_on_tmux() {
            super::partial_cwd();
        }

        #[test]
        fn hermetic_home_on_real_tmux() {
            super::hermetic();
        }

        #[test]
        fn linked_window_dies_with_the_session() {
            super::linked_window_dies();
        }
    }

    struct Live {
        root: tempfile::TempDir,
        home: PathBuf,
        rig_home: PathBuf,
        socket: String,
    }

    impl Live {
        fn new() -> Self {
            let root = tempfile::TempDir::with_prefix("rig-accept-").expect("temp");
            let home = root.path().join("home");
            let rig_home = root.path().join("rig");
            std::fs::create_dir_all(&home).expect("home");
            std::fs::create_dir_all(&rig_home).expect("rig");
            Self {
                // The Linux reap nudge only signals sockets named `rig-rs` or `rig-rs-*`.
                socket: format!("rig-rs-{}", std::process::id()),
                root,
                home,
                rig_home,
            }
        }

        fn rig(&self) -> Command {
            let mut command = Command::cargo_bin("rig").expect("rig");
            command
                .env("HOME", &self.home)
                .env("RIG_HOME", &self.rig_home)
                .env("RIG_TMUX_SOCKET", &self.socket)
                .env_remove("RIG_FAKE_MUX")
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
            if !crewmux_mux::socket_owned_by(&self.socket, std::process::id()) {
                return;
            }
            let mut kill = Command::new("tmux");
            kill.args(["-L", &self.socket, "kill-server"])
                .env_remove("TMUX")
                .env_remove("TMUX_TMPDIR")
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if let Ok(mut child) = kill.spawn() {
                drop(child.wait());
            }
            // A server that has already exited leaves the socket file.
            drop(std::fs::remove_file(crewmux_mux::socket_path(&self.socket)));
        }
    }

    fn stopped_after_down() {
        let live = Live::new();
        up_two(&live);
        let (code, down, err) = json(live.rig().args(["down", "--json"]));
        assert_eq!(code, 0, "{err}");
        assert_eq!(down.get("stopped").and_then(Value::as_bool), Some(true));
        let (code, listed, err) = json(live.rig().args(["ps", "--json"]));
        assert_eq!(code, 0, "{err}");
        assert_eq!(seat(&listed, 0, "state"), "stopped");
        assert_eq!(seat(&listed, 1, "state"), "stopped");
    }

    fn second_up() {
        let live = Live::new();
        up_two(&live);
        let spec = live.home.join("two-stub.yaml");
        let (code, again, err) =
            json(
                live.rig()
                    .args(["up", spec.to_str().expect("utf8"), "--json"]),
            );
        assert_eq!(code, 0, "{err} {again}");
        assert_eq!(
            again.get("status").and_then(Value::as_str),
            Some("already_running")
        );
    }

    fn unattached() {
        let live = Live::new();
        up_two(&live);
        let (code, listed, err) = json(live.rig().args(["ps", "--json"]));
        assert_eq!(code, 0, "{err}");
        assert_eq!(seat(&listed, 0, "attached"), "false");
    }

    fn attach_socket() {
        let live = Live::new();
        up_two(&live);
        let client = crewmux_api::Client::over_uds(crewmux_api::daemon_socket(&live.rig_home));
        let response = client
            .request("POST", "/api/seats/attach", Some(br#"{"seat":"one"}"#))
            .expect("attach");
        assert_eq!(response.status, 200, "{}", response.body);
        let value: Value = serde_json::from_str(&response.body).expect("json");
        let argv: Vec<String> = value
            .get("argv")
            .and_then(Value::as_array)
            .expect("argv")
            .iter()
            .map(|item| item.as_str().expect("str").to_owned())
            .collect();
        let binary = argv.first().expect("binary");
        assert!(Path::new(binary).is_absolute(), "{argv:?}");
        assert_eq!(argv.get(1).map(String::as_str), Some("-S"));
        let path = argv.get(2).expect("socket").as_str();
        let foreign = live.root.path().join("foreign-tmux");
        std::fs::create_dir_all(&foreign).expect("foreign");
        let ok = Command::new(binary)
            .args(["-S", path, "has-session", "-t", "=one__pair"])
            .env("TMUX_TMPDIR", &foreign)
            .env_remove("TMUX")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("has-session")
            .wait_with_output()
            .expect("has-session");
        assert!(
            ok.status.success(),
            "{}",
            String::from_utf8_lossy(&ok.stderr)
        );
        let missed = Command::new("tmux")
            .args(["-L", &live.socket, "has-session", "-t", "=one__pair"])
            .env("TMUX_TMPDIR", &foreign)
            .env_remove("TMUX")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("wrong socket");
        assert!(!missed.success(), " -L followed TMUX_TMPDIR");
    }

    fn exit_after() {
        let live = Live::new();
        let spec = live.home.join("one.yaml");
        std::fs::write(
            &spec,
            format!(
                "name: pair\nseats:\n  - name: one\n    runtime: stub\n    cwd: {}\n",
                live.home.display()
            ),
        )
        .expect("spec");
        let (code, _, err) = json(
            live.rig()
                .env("RIG_STUB_SCRIPT", "exit_after_ms=400")
                .args(["up", spec.to_str().expect("utf8"), "--json"]),
        );
        assert_eq!(code, 0, "{err}");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = String::new();
        while Instant::now() < deadline {
            let (code, listed, err) = json(live.rig().args(["ps", "--json"]));
            assert_eq!(code, 0, "{err}");
            state = seat(&listed, 0, "state");
            if state == "dead" {
                break;
            }
            std::thread::park_timeout(Duration::from_millis(30));
        }
        assert_eq!(state, "dead");
    }

    fn partial_cwd() {
        let live = Live::new();
        let missing = live.root.path().join("missing-cwd");
        let body = format!(
            "name: pair\nseats:\n  - name: good\n    runtime: stub\n    cwd: {}\n  - name: bad\n    runtime: stub\n    cwd: {}\n",
            live.home.display(),
            missing.display()
        );
        let spec = live.home.join("partial.yaml");
        std::fs::write(&spec, body).expect("spec");
        let (code, up, err) = json(
            live.rig()
                .args(["up", spec.to_str().expect("utf8"), "--json"]),
        );
        assert_eq!(code, 3, "{err} {up}");
        assert_eq!(up.get("status").and_then(Value::as_str), Some("partial"));
    }

    fn linked_window_dies() {
        let live = Live::new();
        up_two(&live);
        let listed = tmux_text(
            &live,
            &[
                "list-panes",
                "-a",
                "-F",
                "#{session_name} #{pane_pid} #{window_id}",
            ],
        );
        let mut seat_pid = None;
        let mut seat_window = None;
        for line in listed.lines() {
            let mut parts = line.split_whitespace();
            let Some(session) = parts.next() else {
                continue;
            };
            let Some(pid) = parts.next() else { continue };
            let Some(window) = parts.next() else { continue };
            if session == "one__pair" {
                seat_pid = pid.parse::<u32>().ok();
                seat_window = Some(window.to_owned());
            }
        }
        let pid = seat_pid.expect("pane pid");
        let window = seat_window.expect("window");
        let during = tmux_text(
            &live,
            &[
                "list-windows",
                "-t",
                "crewmux-control",
                "-F",
                "#{window_id}",
            ],
        );
        assert!(
            during.lines().any(|line| line == window),
            "control session never linked {window}: {during}"
        );
        let (code, down, err) = json(live.rig().args(["down", "--json"]));
        assert_eq!(code, 0, "{err} {down}");
        let after = tmux_text(
            &live,
            &["list-windows", "-a", "-F", "#{window_id}|#{session_name}"],
        );
        assert!(
            !after.lines().any(|line| line.starts_with(&window)),
            "linked window survived down: {after}"
        );
        assert!(!process_alive(pid), "pane process {pid} survived down");
        let (code, listed, err) = json(live.rig().args(["ps", "--json"]));
        assert_eq!(code, 0, "{err}");
        assert_eq!(seat(&listed, 0, "state"), "stopped");
        assert_eq!(seat(&listed, 1, "state"), "stopped");
    }

    fn hermetic() {
        let real_home = PathBuf::from(std::env::var("HOME").expect("HOME"));
        let before = mtime(&real_home);
        let live = Live::new();
        up_two(&live);
        assert_eq!(mtime(&real_home), before);
        assert!(live.rig_home.starts_with(live.root.path()));
    }

    fn up_two(live: &Live) {
        let cwd = live.home.display();
        let spec = live.home.join("two-stub.yaml");
        std::fs::write(
            &spec,
            format!("name: pair\nseats:\n  - name: one\n    runtime: stub\n    cwd: {cwd}\n  - name: two\n    runtime: stub\n    cwd: {cwd}\n"),
        )
        .expect("spec");
        let (code, up, err) = json(
            live.rig()
                .args(["up", spec.to_str().expect("utf8"), "--json"]),
        );
        assert_eq!(code, 0, "{err} {up}");
        assert!(
            up.get("status").and_then(Value::as_str) == Some("ready"),
            "{up}"
        );
    }

    fn json(command: &mut Command) -> (i32, Value, String) {
        let output = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn")
            .wait_with_output()
            .expect("wait");
        let out = String::from_utf8_lossy(&output.stdout).into_owned();
        let err = String::from_utf8_lossy(&output.stderr).into_owned();
        let value = serde_json::from_str(out.trim()).unwrap_or(Value::Null);
        (output.status.code().unwrap_or(1), value, err)
    }

    fn seat(body: &Value, index: usize, field: &str) -> String {
        match body.pointer(&format!("/rigs/0/seats/{index}/{field}")) {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Bool(value)) => value.to_string(),
            _ => String::new(),
        }
    }

    fn mtime(path: &Path) -> Option<SystemTime> {
        std::fs::metadata(path)
            .ok()
            .and_then(|meta| meta.modified().ok())
    }

    fn tmux_text(live: &Live, args: &[&str]) -> String {
        let mut command = Command::new("tmux");
        command
            .args(["-L", &live.socket])
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
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn process_alive(pid: u32) -> bool {
        let status = Command::new("ps")
            .args(["-p", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        matches!(status, Ok(code) if code.success())
    }
}
