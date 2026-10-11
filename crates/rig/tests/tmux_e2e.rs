//! Real tmux behind `tmux-it`. The nextest ci profile runs `::tmux_it::`.
#![cfg_attr(
    feature = "tmux-it",
    expect(
        clippy::expect_used,
        reason = "setup failures should abort the integration test"
    )
)]
#![cfg(feature = "tmux-it")]

mod live {
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use assert_cmd::cargo::CommandCargoExt;
    use serde_json::Value;

    pub(crate) mod tmux_it {
        #[test]
        fn up_capture_down_on_a_private_socket() {
            super::run();
        }

        #[test]
        fn clean_paste_sends_enter() {
            super::clean_paste_sends_enter();
        }

        #[test]
        fn extra_text_is_contaminated() {
            super::extra_text_is_contaminated();
        }

        #[test]
        fn wide_payload_on_80_columns_delivers() {
            super::wide_payload_on_80_columns_delivers();
        }

        #[test]
        fn failed_verify_clears_the_composer() {
            super::failed_verify_clears_the_composer();
        }

        #[test]
        fn long_clean_payloads_deliver() {
            super::long_clean_payloads_deliver();
        }

        #[test]
        fn followup_after_long_send_delivers() {
            super::followup_after_long_send_delivers();
        }

        #[test]
        fn resize_to_40_then_300_chars_delivers() {
            super::resize_to_40_then_300_chars_delivers();
        }

        #[test]
        fn buffered_contamination_submits_nothing() {
            super::buffered_contamination_submits_nothing();
        }

        #[test]
        fn send_renders_in_addressed_pane() {
            super::send_renders_in_addressed_pane();
        }

        #[test]
        fn send_verify_means_rendered() {
            super::send_verify_means_rendered();
        }
    }

    static SOCKETS: AtomicU64 = AtomicU64::new(0);

    struct Live {
        root: tempfile::TempDir,
        home: PathBuf,
        rig_home: PathBuf,
        socket: String,
    }

    impl Live {
        fn new() -> Self {
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
    }

    fn run() {
        let _stub = Command::cargo_bin("rig-stub-agent").expect("stub binary");
        let live = Live::new();
        let cwd = live.home.display();
        let spec = live.home.join("two-stub.yaml");
        std::fs::write(
            &spec,
            format!("name: pair\nseats:\n  - name: one\n    runtime: stub\n    cwd: {cwd}\n  - name: two\n    runtime: stub\n    cwd: {cwd}\n"),
        )
        .expect("spec");
        let up = wait_output(live.rig().args(["up", "two-stub.yaml", "--json"]));
        assert_eq!(up.0, 0, "{} {}", up.2, up.1);
        assert!(up.1.contains("\"ready\""), "{}", up.1);
        let size = tmux(
            &live.socket,
            &[
                "list-panes",
                "-t",
                "=one__pair",
                "-F",
                "#{pane_width}x#{pane_height}",
            ],
        );
        assert_eq!(size.0, 0, "{}", size.2);
        assert_eq!(size.1.trim(), "90x27", "{}", size.1);
        let shot = wait_output(live.rig().args(["capture", "one", "--json"]));
        assert_eq!(shot.0, 0, "{}", shot.2);
        assert!(shot.1.contains("stub> "), "{}", shot.1);
        let down = wait_output(live.rig().args(["down", "--json"]));
        assert_eq!(down.0, 0, "{}", down.2);
        let listed = tmux(&live.socket, &["ls"]);
        assert!(
            !listed.1.contains("one__pair") && !listed.1.contains("two__pair"),
            "{} {}",
            listed.1,
            listed.2
        );
    }

    fn clean_paste_sends_enter() {
        let live = boot(None);
        let (code, body, err) = send(
            &live,
            &["send", "--to", "one@pair", "marker-clean", "--json"],
        );
        assert_eq!(code, 0, "{err} {} {}", body, log(&live));
        assert_eq!(body.get("status").and_then(Value::as_str), Some("ok"));
        assert_eq!(outcome(&body), "delivered");
        wait_prompt(&live, "one");
        let (code, verify, err) = send(
            &live,
            &[
                "send",
                "--to",
                "one@pair",
                "marker-verify",
                "--verify",
                "--json",
            ],
        );
        assert_eq!(code, 0, "{err} {verify} {}", log(&live));
        assert_eq!(outcome(&verify), "delivered");
        wait_prompt(&live, "one");
        let (code, fan, err) = send(
            &live,
            &["send", "--to", "one@pair,two@pair", "marker-fan", "--json"],
        );
        assert_eq!(code, 0, "{err} {fan} {}", log(&live));
        assert_eq!(fan.get("status").and_then(Value::as_str), Some("ok"));
        assert!(
            outcomes(&fan).iter().all(|item| item == "delivered"),
            "{fan}"
        );
        let one = history(&live, "one__pair");
        let two = history(&live, "two__pair");
        assert!(one.contains("To: one@pair"), "{one}");
        assert!(one.contains("marker-clean"), "{one}");
        assert!(one.contains("marker-fan"), "{one}");
        assert!(two.contains("To: two@pair"), "{two}");
        assert!(two.contains("marker-fan"), "{two}");
        assert!(!two.contains("marker-clean"), "{two}");
    }

    fn extra_text_is_contaminated() {
        let live = boot(Some("contaminate=1"));
        let (code, body, err) = send(
            &live,
            &["send", "--to", "one@pair", "marker-dirty", "--json"],
        );
        assert_eq!(code, 1, "{err} {body} {}", log(&live));
        assert_eq!(body.get("status").and_then(Value::as_str), Some("failed"));
        assert_eq!(outcome(&body), "contaminated");
        assert!(body.get("receipts").is_some(), "{body}");
        let shot = capture(&live, "one");
        assert!(shot.contains("EXTRA"), "{shot}");
        wait_prompt(&live, "one");
        let (again, next, err) = send(
            &live,
            &["send", "--to", "one@pair", "marker-next", "--json"],
        );
        assert_eq!(again, 0, "{err} {next} {}", log(&live));
        assert_eq!(
            outcome(&next),
            "delivered",
            "{next} {}",
            capture(&live, "one")
        );
    }

    fn wide_payload_on_80_columns_delivers() {
        let live = boot(None);
        let resized = tmux(
            &live.socket,
            &["resize-window", "-t", "one__pair", "-x", "80", "-y", "27"],
        );
        assert_eq!(resized.0, 0, "{}", resized.2);
        let size = tmux(
            &live.socket,
            &["list-panes", "-t", "one__pair", "-F", "#{pane_width}"],
        );
        assert_eq!(size.1.trim(), "80", "{} {}", size.1, size.2);
        let body = "W".repeat(200);
        let (code, value, err) = send(&live, &["send", "--to", "one@pair", &body, "--json"]);
        assert_eq!(code, 0, "{err} {value} {}", log(&live));
        assert_eq!(outcome(&value), "delivered");
        wait_prompt(&live, "one");
        let flat: String = history(&live, "one__pair")
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect();
        assert!(flat.contains(&body), "wrapped payload was not delivered");
    }

    fn failed_verify_clears_the_composer() {
        let live = boot(Some("contaminate=1"));
        let (code, body, err) = send(
            &live,
            &[
                "send",
                "--to",
                "one@pair",
                "marker-forced",
                "--verify",
                "--json",
            ],
        );
        assert_eq!(code, 1, "{err} {body} {}", log(&live));
        assert_eq!(body.get("status").and_then(Value::as_str), Some("failed"));
        assert!(body.get("receipts").is_some(), "{body}");
        wait_prompt(&live, "one");
        assert!(prompt_clear(&capture(&live, "one")));
        let (again, next, err) = send(
            &live,
            &["send", "--to", "one@pair", "marker-after", "--json"],
        );
        assert_eq!(again, 0, "{err} {next} {}", log(&live));
        assert_eq!(outcome(&next), "delivered", "{next}");
    }

    fn long_clean_payloads_deliver() {
        let live = boot(None);
        let size = pane_size(&live);
        assert_eq!(size.as_deref(), Some("90x27"), "{size:?}");
        for n in [1000, 2000] {
            let body = "W".repeat(n);
            let (code, value, err) = send(&live, &["send", "--to", "one@pair", &body, "--json"]);
            assert_eq!(code, 0, "{n} {err} {value} {}", log(&live));
            assert_eq!(outcome(&value), "delivered", "{n} {value}");
            wait_prompt(&live, "one");
            let flat: String = history(&live, "one__pair")
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect();
            assert!(flat.contains(&body), "{n} chars were not delivered");
        }
    }

    fn followup_after_long_send_delivers() {
        let live = boot(None);
        assert_eq!(pane_size(&live).as_deref(), Some("90x27"));
        for (gap_ms, first_char, second_char) in [(600_u64, 'A', 'B'), (2000, 'C', 'D')] {
            let first = first_char.to_string().repeat(479);
            let second = second_char.to_string().repeat(88);
            let (code, value, err) = send(&live, &["send", "--to", "one@pair", &first, "--json"]);
            assert_eq!(code, 0, "{gap_ms} {err} {value} {}", log(&live));
            assert_eq!(outcome(&value), "delivered", "{gap_ms} {value}");
            std::thread::park_timeout(Duration::from_millis(gap_ms));
            let (code, value, err) = send(&live, &["send", "--to", "one@pair", &second, "--json"]);
            assert_eq!(code, 0, "{gap_ms} follow-up {err} {value} {}", log(&live));
            assert_eq!(outcome(&value), "delivered", "{gap_ms} {value}");
            wait_prompt(&live, "one");
            let flat: String = history(&live, "one__pair")
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect();
            assert!(flat.contains(&first), "{gap_ms} first payload missing");
            assert!(flat.contains(&second), "{gap_ms} follow-up missing");
        }
    }

    fn resize_to_40_then_300_chars_delivers() {
        let live = boot(None);
        assert_eq!(pane_size(&live).as_deref(), Some("90x27"));
        let resized = tmux(
            &live.socket,
            &["resize-window", "-t", "one__pair", "-x", "40", "-y", "27"],
        );
        assert_eq!(resized.0, 0, "{}", resized.2);
        assert_eq!(pane_size(&live).as_deref(), Some("40x27"));
        let body = "W".repeat(300);
        let (code, value, err) = send(&live, &["send", "--to", "one@pair", &body, "--json"]);
        assert_eq!(code, 0, "{err} {value} {}", log(&live));
        assert_eq!(outcome(&value), "delivered", "{value}");
        wait_prompt(&live, "one");
        let flat: String = history(&live, "one__pair")
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect();
        assert!(flat.contains(&body), "resized payload was not delivered");
    }

    fn buffered_contamination_submits_nothing() {
        let live = boot(Some("contaminate=1,buffer=1"));
        let (code, body, err) = send(
            &live,
            &[
                "send",
                "--to",
                "one@pair",
                "marker-buffer",
                "--verify",
                "--json",
            ],
        );
        assert_ne!(code, 0, "{err} {body} {}", log(&live));
        assert_ne!(outcome(&body), "delivered", "{body}");
        let shot = history(&live, "one__pair");
        assert!(
            !shot.contains("SUBMIT"),
            "buffered composer submitted after a failed verify: {shot}"
        );
        assert!(!capture(&live, "one").contains("SUBMIT"));
    }

    fn pane_size(live: &Live) -> Option<String> {
        let size = tmux(
            &live.socket,
            &[
                "list-panes",
                "-t",
                "one__pair",
                "-F",
                "#{pane_width}x#{pane_height}",
            ],
        );
        (size.0 == 0).then(|| size.1.trim().to_owned())
    }

    fn send_renders_in_addressed_pane() {
        let live = boot(None);
        let marker = "s06-render-marker-4d9b";
        let (code, body, err) = send(&live, &["send", "--to", "one@pair", marker, "--json"]);
        assert_eq!(code, 0, "{err} {body} {}", log(&live));
        assert_eq!(outcome(&body), "delivered");
        let one = capture(&live, "one");
        let two = capture(&live, "two");
        assert!(one.contains("To: one@pair"), "{one}");
        assert!(one.contains(marker), "{one}");
        assert!(!two.contains(marker), "{two}");
        assert!(!two.contains("To: one@pair"), "{two}");
    }

    fn send_verify_means_rendered() {
        let live = boot(None);
        let marker = "s06-render-marker-4d9b";
        let (code, body, err) = send(
            &live,
            &["send", "--to", "one@pair", marker, "--verify", "--json"],
        );
        assert_eq!(code, 0, "{err} {body} {}", log(&live));
        assert_eq!(body.get("status").and_then(Value::as_str), Some("ok"));
        assert_eq!(outcome(&body), "delivered");
        let shot = capture(&live, "one");
        assert!(shot.contains(marker), "{shot}");
        assert!(shot.contains("To: one@pair"), "{shot}");
    }

    fn boot(script: Option<&str>) -> Live {
        let _stub = Command::cargo_bin("rig-stub-agent").expect("stub binary");
        let live = Live::new();
        let cwd = live.home.display();
        let spec = live.home.join("two-stub.yaml");
        std::fs::write(
            &spec,
            format!("name: pair\nseats:\n  - name: one\n    runtime: stub\n    cwd: {cwd}\n  - name: two\n    runtime: stub\n    cwd: {cwd}\n"),
        )
        .expect("spec");
        let mut up = live.rig();
        if let Some(script) = script {
            up.env("RIG_STUB_SCRIPT", script);
        }
        let up = wait_output(up.args(["up", "two-stub.yaml", "--json"]));
        assert_eq!(up.0, 0, "{} {} {}", up.2, up.1, log(&live));
        assert!(up.1.contains("\"ready\""), "{}", up.1);
        live
    }

    fn send(live: &Live, args: &[&str]) -> (i32, Value, String) {
        let (code, out, err) = wait_output(live.rig().args(args));
        let value = serde_json::from_str(out.trim()).unwrap_or(Value::Null);
        (code, value, err)
    }

    fn history(live: &Live, session: &str) -> String {
        let shot = tmux(
            &live.socket,
            &["capture-pane", "-p", "-S", "-", "-t", session],
        );
        assert_eq!(shot.0, 0, "{}", shot.2);
        shot.1
    }

    fn capture(live: &Live, seat: &str) -> String {
        let (code, out, err) = wait_output(live.rig().args(["capture", seat, "--json"]));
        assert_eq!(code, 0, "{err} {out}");
        serde_json::from_str::<Value>(out.trim())
            .ok()
            .and_then(|value| value.get("text").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or(out)
    }

    fn wait_prompt(live: &Live, seat: &str) {
        let mut cleared = false;
        for _ in 0..40 {
            if prompt_clear(&capture(live, seat)) {
                cleared = true;
                break;
            }
            std::thread::park_timeout(Duration::from_millis(50));
        }
        assert!(cleared, "prompt did not clear: {}", capture(live, seat));
    }

    fn prompt_clear(text: &str) -> bool {
        let Some(idx) = text.rfind("stub> ") else {
            return false;
        };
        text.get(idx + "stub> ".len()..)
            .unwrap_or("")
            .trim()
            .is_empty()
    }

    fn outcome(body: &Value) -> String {
        body.pointer("/receipts/0/outcome")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }

    fn outcomes(body: &Value) -> Vec<String> {
        body.get("receipts")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| {
                        row.get("outcome")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn log(live: &Live) -> String {
        std::fs::read_to_string(live.rig_home.join("run/daemon.log")).unwrap_or_default()
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
