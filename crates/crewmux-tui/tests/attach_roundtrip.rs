//! Real tmux attach/detach. Built only with `--features tmux-it`.

#![cfg(feature = "tmux-it")]

use std::io;
use std::process::Stdio;

use crewmux_api::SessionId;
use crewmux_tui::{AttachOutcome, AttachTarget, TerminalGuard, perform};

struct FakeGuard {
    suspended: bool,
    resumed: bool,
}

impl TerminalGuard for FakeGuard {
    fn suspend(&mut self) -> io::Result<()> {
        self.suspended = true;
        Ok(())
    }

    fn resume(&mut self) -> io::Result<()> {
        self.resumed = true;
        Ok(())
    }
}

async fn tmux(socket: &str, args: &[&str]) -> io::Result<std::process::ExitStatus> {
    let mut command = vec!["-L", socket];
    command.extend_from_slice(args);
    tokio::process::Command::new("tmux")
        .args(&command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
}

#[tokio::test]
async fn fifty_attach_cycles_then_a_dead_session() {
    let socket = format!("rig{}", std::process::id());
    if let Ok(status) = tmux(&socket, &["kill-server"]).await {
        let _ = status.code();
    }
    assert!(
        tmux(&socket, &["new-session", "-d", "-s", "seat"])
            .await
            .expect("new-session")
            .success()
    );
    for _ in 0..50 {
        let socket_detach = socket.clone();
        let detacher = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if let Ok(status) = tmux(&socket_detach, &["detach-client", "-s", "seat"]).await {
                let _ = status.success();
            }
        });
        let mut guard = FakeGuard {
            suspended: false,
            resumed: false,
        };
        let outcome = perform(
            AttachTarget::Tmux {
                socket: socket.clone(),
                session: SessionId::parse("seat").expect("session"),
            },
            None,
            &mut guard,
        )
        .await;
        detacher.await.expect("detach task");
        // This process has no tty, so tmux refuses the attach and the session stays up.
        assert!(matches!(outcome, AttachOutcome::Failed(_)));
        assert!(guard.suspended && guard.resumed);
    }
    assert!(
        tmux(&socket, &["kill-session", "-t", "seat"])
            .await
            .expect("kill-session")
            .success()
    );
    let mut guard = FakeGuard {
        suspended: false,
        resumed: false,
    };
    let outcome = perform(
        AttachTarget::Tmux {
            socket: socket.clone(),
            session: SessionId::parse("seat").expect("session"),
        },
        None,
        &mut guard,
    )
    .await;
    assert_eq!(outcome, AttachOutcome::SessionGone);
    assert!(guard.suspended && guard.resumed);
    if let Ok(status) = tmux(&socket, &["kill-server"]).await {
        let _ = status.success();
    }
}
