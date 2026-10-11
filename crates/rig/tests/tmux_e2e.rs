//! Real tmux behind `tmux-it`. The nextest ci profile runs `::tmux_it::`.
#![expect(
    clippy::expect_used,
    reason = "setup failures should abort the integration test"
)]
#![cfg(feature = "tmux-it")]

mod live {
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    use assert_cmd::cargo::CommandCargoExt;

    pub(crate) mod tmux_it {
        #[test]
        fn up_capture_down_on_a_private_socket() {
            super::run();
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
            let root = tempfile::TempDir::with_prefix("rig-tmux-").expect("temp");
            let home = root.path().join("home");
            let rig_home = root.path().join("rig");
            std::fs::create_dir_all(&home).expect("home");
            std::fs::create_dir_all(&rig_home).expect("rig");
            let socket = format!("r{}", std::process::id());
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
            let mut kill = Command::new("tmux");
            kill.args(["-L", &self.socket, "kill-server"])
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if let Ok(mut child) = kill.spawn() {
                drop(child.wait());
            }
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
            .args(args)
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
