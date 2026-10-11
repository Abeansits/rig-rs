// Derived from openrig packages/daemon/src/adapters/stub-script.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `rig-stub-agent` process: banner, optional gate, composer, and echo.

use std::io::{BufRead, Write};
use std::os::fd::AsFd;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

use crate::error::Error;
use crate::stub::script::{StubScript, parse_script};

/// Run the fixture until stdin closes or `exit_after_ms` elapses.
///
/// # Errors
///
/// Returns [`Error`] when the script is invalid or stdio fails.
pub fn run_stub_agent() -> Result<(), Error> {
    let raw = std::env::var("RIG_STUB_SCRIPT").unwrap_or_default();
    let script = parse_script(&raw)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .enable_io()
        .build()?;
    runtime.block_on(drive(script))
}

async fn drive(script: StubScript) -> Result<(), Error> {
    let started = tokio::time::Instant::now();
    let mut stdout = tokio::io::stdout();
    stdout.write_all(StubScript::banner().as_bytes()).await?;
    stdout.flush().await?;
    if script.ready_after_ms > 0 {
        tokio::time::sleep(Duration::from_millis(script.ready_after_ms)).await;
    }
    let rest = script.screen().replacen(StubScript::banner(), "", 1);
    stdout.write_all(rest.as_bytes()).await?;
    stdout.flush().await?;
    echo_until(
        script
            .exit_after_ms
            .map(|ms| started + Duration::from_millis(ms)),
        script.contaminate,
    )
    .await
}

async fn echo_until(exit_at: Option<tokio::time::Instant>, contaminate: bool) -> Result<(), Error> {
    // Tokio's stdin read cannot be cancelled, so a timer cannot win while tmux
    // holds the pty. Holding `Stdin::lock` also deadlocks process exit, which
    // flushes stdio. A detached thread reads a dup'd fd instead.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    drop(std::thread::spawn(move || {
        // Dup the fd so this read does not hold the stdio lock. `process::exit`
        // flushes stdio and would deadlock on that lock until the pty closed.
        let Ok(fd) = std::io::stdin().as_fd().try_clone_to_owned() else {
            return;
        };
        let mut reader = std::io::BufReader::new(std::fs::File::from(fd));
        let mut line = String::new();
        let mut first = true;
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if contaminate && first {
                        first = false;
                        let mut out = std::io::stdout();
                        drop(writeln!(out, "EXTRA"));
                        drop(out.flush());
                    }
                    if tx.send(std::mem::take(&mut line)).is_err() {
                        break;
                    }
                }
            }
        }
    }));
    let mut stdout = tokio::io::stdout();
    // Lines in one paste arrive together. Enter comes later, after the
    // post-paste check, and that gap is what reprints the prompt. Reprinting
    // on every echoed line would clear the draft and the next send would not
    // see the leftover text.
    let mut seen = false;
    let mut last = tokio::time::Instant::now();
    loop {
        if exit_due(exit_at) {
            return Ok(());
        }
        let line = match exit_at {
            Some(deadline) => {
                let wait = deadline.saturating_duration_since(tokio::time::Instant::now());
                tokio::select! {
                    () = tokio::time::sleep(wait) => return Ok(()),
                    msg = rx.recv() => msg,
                }
            }
            None => rx.recv().await,
        };
        let Some(line) = line else {
            return Ok(());
        };
        let now = tokio::time::Instant::now();
        let gap = now.saturating_duration_since(last);
        last = now;
        stdout.write_all(line.as_bytes()).await?;
        if seen && gap >= Duration::from_millis(40) {
            stdout.write_all(b"stub> \n").await?;
        }
        seen = true;
        stdout.flush().await?;
    }
}

fn exit_due(exit_at: Option<tokio::time::Instant>) -> bool {
    exit_at.is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
}
