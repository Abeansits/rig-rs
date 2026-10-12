//! Real tmux restore. Private socket, and kill-server only for that socket.
#![cfg_attr(
    feature = "tmux-it",
    expect(
        clippy::expect_used,
        reason = "setup failures abort the integration test"
    )
)]
#![cfg_attr(
    feature = "tmux-it",
    expect(
        clippy::panic,
        reason = "a daemon that does not exit fails the integration test"
    )
)]
#![cfg(feature = "tmux-it")]

mod tmux_it {
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use assert_cmd::cargo::CommandCargoExt;
    use serde_json::Value;

    static SOCKETS: AtomicU64 = AtomicU64::new(0);

    struct Live {
        root: tempfile::TempDir,
        home: PathBuf,
        rig_home: PathBuf,
        socket: String,
        hooks: bool,
    }

    impl Live {
        fn new(hooks: bool) -> Self {
            let root = tempfile::TempDir::with_prefix("rig-tmux-").expect("temp");
            let home = root.path().join("home");
            let rig_home = root.path().join("rig");
            std::fs::create_dir_all(&home).expect("home");
            std::fs::create_dir_all(&rig_home).expect("rig");
            let n = SOCKETS.fetch_add(1, Ordering::Relaxed);
            let socket = format!("rig-rs-{}-{n}-e2e", std::process::id());
            kill_server(&socket);
            Self {
                root,
                home,
                rig_home,
                socket,
                hooks,
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
            if self.hooks {
                command.env("RIG_TEST_HOOKS", "1");
            }
            command
        }
    }

    impl Drop for Live {
        fn drop(&mut self) {
            let _keep = self.root.path();
            let socket = crewmux_api::daemon_socket(&self.rig_home);
            let client = crewmux_api::Client::over_uds(socket).with_timeout(Duration::from_secs(2));
            drop(client.request("POST", "/shutdown", Some(b"{}")));
            kill_server(&self.socket);
        }
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

    fn run(command: &mut Command) -> (i32, String, String) {
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

    fn write_spec(live: &Live, name: &str, body: &str) -> PathBuf {
        let path = live.home.join(name);
        std::fs::write(&path, body).expect("spec");
        path
    }

    fn spec(cwd: &std::path::Path, name: &str, seats: &[&str]) -> String {
        let cwd = cwd.display();
        let mut text = format!("name: {name}\nseats:\n");
        for seat in seats {
            text.push_str("  - name: ");
            text.push_str(seat);
            text.push_str("\n    runtime: stub\n    cwd: ");
            text.push_str(&cwd.to_string());
            text.push('\n');
        }
        text
    }

    fn post(live: &Live, path: &str, body: &Value) -> Value {
        let socket = crewmux_api::daemon_socket(&live.rig_home);
        let bytes = serde_json::to_vec(body).expect("json");
        let response = crewmux_api::Client::over_uds(socket)
            .request("POST", path, Some(&bytes))
            .expect("post");
        assert_eq!(response.status, 200, "{}", response.body);
        serde_json::from_str(&response.body).expect("json")
    }

    fn shutdown(live: &Live) {
        let pid = daemon_pid(live);
        let socket = crewmux_api::daemon_socket(&live.rig_home);
        let client = crewmux_api::Client::over_uds(socket).with_timeout(Duration::from_secs(2));
        drop(client.request("POST", "/shutdown", Some(b"{}")));
        let (_tx, rx) = std::sync::mpsc::channel::<()>();
        for _ in 0..80 {
            if daemon_gone(pid) {
                return;
            }
            if rx.recv_timeout(Duration::from_millis(50)).is_ok() {
                break;
            }
        }
        let log = std::fs::read_to_string(live.rig_home.join("run/daemon.log")).unwrap_or_default();
        panic!("daemon did not exit\n{log}");
    }

    fn daemon_pid(live: &Live) -> Option<rustix::process::Pid> {
        let text = std::fs::read_to_string(live.rig_home.join("run/daemon.pid")).ok()?;
        let raw = text.trim().parse::<i32>().ok()?;
        rustix::process::Pid::from_raw(raw)
    }

    fn daemon_gone(pid: Option<rustix::process::Pid>) -> bool {
        let Some(pid) = pid else {
            return true;
        };
        rustix::process::test_kill_process(pid).is_err()
    }

    fn seat_id(live: &Live, name: &str) -> String {
        let conn = rusqlite::Connection::open(live.rig_home.join("state/rig.db")).expect("db");
        conn.query_row("SELECT id FROM seats WHERE name = ?1", [name], |row| {
            row.get(0)
        })
        .expect("seat")
    }

    fn capture(live: &Live, seat: &str) -> String {
        let (code, out, err) = run(live.rig().args(["capture", seat, "--json"]));
        assert_eq!(code, 0, "{err} {out}");
        let value: Value = serde_json::from_str(out.trim()).expect("json");
        value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    }

    fn never_ready(body: &Value) {
        for seat in body.get("seats").and_then(Value::as_array).expect("seats") {
            assert_ne!(seat.get("outcome").and_then(Value::as_str), Some("ready"));
        }
    }

    #[test]
    fn wave_restore_after_kill_server() {
        let _stub = Command::cargo_bin("rig-stub-agent").expect("stub");
        let live = Live::new(false);
        let spec = write_spec(
            &live,
            "trio.yaml",
            &spec(&live.home, "trio", &["mid", "alpha", "zeta"]),
        );
        let (code, _, err) = run(live.rig().arg("up").arg(&spec).arg("--json"));
        assert_eq!(code, 0, "{err}");
        post(
            &live,
            "/api/edges",
            &serde_json::json!({"rig": "trio", "source": "alpha", "target": "mid", "kind": "delegates_to"}),
        );
        post(
            &live,
            "/api/edges",
            &serde_json::json!({"rig": "trio", "source": "zeta", "target": "mid", "kind": "spawned_by"}),
        );
        let (code, _, err) =
            run(live
                .rig()
                .args(["send", "--to", "alpha@trio", "replay-alpha", "--json"]));
        assert_eq!(code, 0, "{err}");
        let zeta = seat_id(&live, "zeta");
        std::fs::remove_file(live.rig_home.join("resume").join(format!("{zeta}.token")))
            .expect("token");
        shutdown(&live);
        kill_server(&live.socket);
        let (code, out, err) = run(live.rig().args(["restore", "trio", "--json"]));
        assert_eq!(code, 0, "{err} {out}");
        let body: Value = serde_json::from_str(out.trim()).expect("json");
        never_ready(&body);
        assert_eq!(
            body.get("outcome").and_then(Value::as_str),
            Some("fully_restored")
        );
        assert_eq!(
            body.pointer("/waves/0/0").and_then(Value::as_str),
            Some("alpha")
        );
        assert_eq!(
            body.pointer("/waves/1/0").and_then(Value::as_str),
            Some("mid")
        );
        assert_eq!(
            body.pointer("/waves/2/0").and_then(Value::as_str),
            Some("zeta")
        );
        let alpha = capture(&live, "alpha");
        assert!(alpha.contains("replay-alpha"), "{alpha}");
        assert!(alpha.contains("posture: "), "{alpha}");
    }

    #[test]
    fn gate_and_locked_restore() {
        let _stub = Command::cargo_bin("rig-stub-agent").expect("stub");
        let live = Live::new(false);
        let gated = write_spec(
            &live,
            "gated.yaml",
            &spec(&live.home, "gated", &["gate-trust", "plain"]),
        );
        let (code, _, err) = run(live.rig().arg("up").arg(&gated).arg("--json"));
        assert_eq!(code, 0, "{err}");
        shutdown(&live);
        kill_server(&live.socket);
        let (code, out, err) = run(live.rig().args(["restore", "gated", "--json"]));
        assert_eq!(code, 0, "{err} {out}");
        let body: Value = serde_json::from_str(out.trim()).expect("json");
        never_ready(&body);
        assert_eq!(
            body.get("outcome").and_then(Value::as_str),
            Some("partially_restored")
        );
        let gate = body
            .get("seats")
            .and_then(Value::as_array)
            .and_then(|seats| {
                seats
                    .iter()
                    .find(|seat| seat.get("seat").and_then(Value::as_str) == Some("gate-trust"))
            })
            .cloned()
            .expect("gate");
        assert_eq!(
            gate.get("outcome").and_then(Value::as_str),
            Some("attention_required")
        );
        assert!(
            gate.get("truth").and_then(Value::as_str).is_some(),
            "{gate}"
        );

        let locked = write_spec(
            &live,
            "locked.yaml",
            &spec(&live.home, "locked", &["locked-one"]),
        );
        let (code, _, err) = run(live.rig().arg("up").arg(&locked).arg("--json"));
        assert_eq!(code, 0, "{err}");
        shutdown(&live);
        kill_server(&live.socket);
        let (code, out, err) =
            run(live
                .rig()
                .args(["restore", "locked", "--posture", "full_bypass", "--json"]));
        assert_eq!(code, 0, "{err} {out}");
        let body: Value = serde_json::from_str(out.trim()).expect("json");
        never_ready(&body);
        assert_eq!(
            body.pointer("/seats/0/posture").and_then(Value::as_str),
            Some("locked")
        );
        let shot = capture(&live, "locked-one");
        assert!(shot.contains("posture: locked"), "{shot}");
    }

    #[test]
    fn direct_histogram_and_queue_are_not_duplicated() {
        let _stub = Command::cargo_bin("rig-stub-agent").expect("stub");
        let live = Live::new(true);
        let spec = write_spec(
            &live,
            "trio.yaml",
            &spec(&live.home, "trio", &["alpha", "beta", "gamma"]),
        );
        let (code, _, err) = run(live.rig().arg("up").arg(&spec).arg("--json"));
        assert_eq!(code, 0, "{err}");
        let seats = ["alpha", "beta", "gamma"];
        let phases = ["before", "", "ack"];
        let mut n = 0u32;
        for phase in phases {
            for _ in 0..2 {
                for seat in seats {
                    std::fs::write(live.rig_home.join("direct-phase"), phase).expect("phase");
                    let body = format!("m{n:02}");
                    let (code, out, err) = run(live.rig().args([
                        "send",
                        "--to",
                        &format!("{seat}@trio"),
                        &body,
                        "--json",
                    ]));
                    assert_eq!(code, 0, "{err} {out}");
                    n += 1;
                }
            }
        }
        post(
            &live,
            "/api/queue",
            &serde_json::json!({
                "id": "baton-1",
                "source": "alice@host",
                "destination": "alpha@trio",
                "body": "obligation-secret-xyz",
            }),
        );
        shutdown(&live);
        mark_unconfirmed(&live, "baton-1");
        kill_server(&live.socket);
        let (code, out, err) = run(live.rig().args(["restore", "trio", "--json"]));
        assert_eq!(code, 0, "{err} {out}");
        let mut hist = std::collections::BTreeMap::new();
        for index in 0..18 {
            let body = format!("m{index:02}");
            let count = count_turns(&live, &body);
            *hist.entry(count).or_insert(0u32) += 1;
        }
        assert_eq!(hist.get(&1).copied(), Some(18), "{hist:?}");
        let shown = post(
            &live,
            "/api/queue/show",
            &serde_json::json!({"id": "baton-1"}),
        );
        assert_eq!(
            shown.pointer("/item/body").and_then(Value::as_str),
            Some("obligation-secret-xyz")
        );
        let shot = capture(&live, "alpha");
        assert!(!shot.contains("obligation-secret-xyz"), "{shot}");
        assert!(shot.contains("baton-1"), "{shot}");
    }

    #[test]
    fn crash_cart_kernel_cancel_and_continue() {
        let _stub = Command::cargo_bin("rig-stub-agent").expect("stub");
        let live = Live::new(false);
        let fleet = write_spec(&live, "fleet.yaml", &spec(&live.home, "fleet", &["oseat"]));
        let kernel = write_spec(
            &live,
            "kernel.yaml",
            &spec(&live.home, "kernel", &["kseat"]),
        );
        let (code, _, err) = run(live.rig().arg("up").arg(&fleet).arg("--json"));
        assert_eq!(code, 0, "{err}");
        let (code, _, err) = run(live.rig().arg("up").arg(&kernel).arg("--json"));
        assert_eq!(code, 0, "{err}");
        post(&live, "/api/kernel", &serde_json::json!({"rig": "kernel"}));
        let (code, out, err) =
            run(live
                .rig()
                .args(["crash-cart", "restore", "--cancel", "--json"]));
        assert_eq!(code, 0, "{err} {out}");
        let body: Value = serde_json::from_str(out.trim()).expect("json");
        assert_eq!(
            body.pointer("/rigs/0/rig").and_then(Value::as_str),
            Some("kernel")
        );
        assert_eq!(
            body.pointer("/rigs/0/kernel").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            body.pointer("/rigs/1/outcome").and_then(Value::as_str),
            Some("not_attempted")
        );
        assert_eq!(
            body.get("cancelled_before").and_then(Value::as_str),
            Some("fleet")
        );
        shutdown(&live);
        let conn = rusqlite::Connection::open(live.rig_home.join("state/rig.db")).expect("db");
        conn.execute(
            "UPDATE seats SET cwd = '/no/such/rig-slice8-cwd' WHERE name = 'kseat'",
            [],
        )
        .expect("cwd");
        drop(conn);
        kill_server(&live.socket);
        let (code, out, err) = run(live.rig().args(["crash-cart", "restore", "--json"]));
        assert_eq!(code, 0, "{err} {out}");
        let body: Value = serde_json::from_str(out.trim()).expect("json");
        let rows = body.get("rigs").and_then(Value::as_array).expect("rigs");
        assert_eq!(rows.len(), 2);
        assert_ne!(outcome(rows, "kernel"), "not_attempted");
        assert_ne!(outcome(rows, "fleet"), "not_attempted");
        assert_eq!(outcome(rows, "fleet"), "fully_restored");
    }

    fn outcome<'a>(rows: &'a [Value], name: &str) -> &'a str {
        rows.iter()
            .find(|row| row.get("rig").and_then(Value::as_str) == Some(name))
            .and_then(|row| row.get("outcome"))
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    fn mark_unconfirmed(live: &Live, id: &str) {
        let path = live.rig_home.join("state/rig.db");
        let conn = rusqlite::Connection::open(&path).expect("db");
        let doc: String = conn
            .query_row("SELECT doc FROM queue_items WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .expect("doc");
        let mut value: Value = serde_json::from_str(&doc).expect("json");
        let object = value.as_object_mut().expect("queue doc");
        object.insert(
            "last_nudge".to_owned(),
            serde_json::json!({
                "at": 1,
                "result": "delivered-ack-pending",
                "text": "pointer",
            }),
        );
        let next = serde_json::to_string(&value).expect("json");
        conn.execute("UPDATE queue_items SET doc = ?1 WHERE id = ?2", (next, id))
            .expect("update");
    }

    fn count_turns(live: &Live, body: &str) -> usize {
        let root = live.rig_home.join("transcripts");
        let Ok(seats) = std::fs::read_dir(root) else {
            return 0;
        };
        let mut count = 0usize;
        for seat in seats.flatten() {
            let Ok(files) = std::fs::read_dir(seat.path()) else {
                continue;
            };
            for file in files.flatten() {
                let plain = file
                    .path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"));
                if !plain {
                    continue;
                }
                let contents = std::fs::read_to_string(file.path()).unwrap_or_default();
                count += contents.matches(body).count();
            }
        }
        count
    }
}
