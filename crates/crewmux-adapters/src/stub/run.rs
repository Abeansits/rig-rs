// Derived from openrig packages/daemon/src/testing/stub-script.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `rig-stub-agent` process: banner, optional gate, composer, and echo.

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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
    )
    .await
}

async fn echo_until(exit_at: Option<tokio::time::Instant>) -> Result<(), Error> {
    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();
    loop {
        if exit_due(exit_at) {
            return Ok(());
        }
        let read = stdin.read_line(&mut line);
        let result = match exit_at {
            Some(deadline) => {
                let wait = deadline.saturating_duration_since(tokio::time::Instant::now());
                tokio::select! {
                    () = tokio::time::sleep(wait) => return Ok(()),
                    result = read => result,
                }
            }
            None => read.await,
        };
        let n = result?;
        if n == 0 {
            return Ok(());
        }
        stdout.write_all(line.as_bytes()).await?;
        stdout.flush().await?;
        line.clear();
    }
}

fn exit_due(exit_at: Option<tokio::time::Instant>) -> bool {
    exit_at.is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
}
