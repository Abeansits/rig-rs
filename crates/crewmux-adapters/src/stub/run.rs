// Derived from openrig packages/daemon/src/adapters/stub-script.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `rig-stub-agent` process: banner, optional gate, composer, and echo.

use std::io::{BufRead, Read, Write};
use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::AsyncWriteExt;

use crate::error::Error;
use crate::stub::buffer::{Scanner, StdinEvent};
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
    if script.buffer {
        // tmux wraps `paste-buffer -p` only after the pane asks for bracketed paste.
        stdout.write_all(b"\x1b[?2004h").await?;
    }
    stdout.flush().await?;
    echo_until(
        script
            .exit_after_ms
            .map(|ms| started + Duration::from_millis(ms)),
        script.contaminate,
        script.buffer,
        Arc::new(AtomicBool::new(true)),
    )
    .await
}

/// `C-u`. The kernel eats this in canonical mode; tmux sends it as the byte.
const CLEAR_LINE: u8 = 0x15;

async fn echo_until(
    exit_at: Option<tokio::time::Instant>,
    mut contaminate: bool,
    buffer: bool,
    echo_lines: Arc<AtomicBool>,
) -> Result<(), Error> {
    // Tokio's stdin read cannot be cancelled, so a timer cannot win while tmux
    // holds the pty. Holding `Stdin::lock` also deadlocks process exit, which
    // flushes stdio. A detached thread reads a dup'd fd instead.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let echoed = Arc::clone(&echo_lines);
    drop(std::thread::spawn(move || {
        read_stdin(&tx, contaminate, buffer, &echoed);
    }));
    let mut stdout = tokio::io::stdout();
    loop {
        if exit_due(exit_at) {
            return Ok(());
        }
        let event = match exit_at {
            Some(deadline) => {
                let wait = deadline.saturating_duration_since(tokio::time::Instant::now());
                tokio::select! {
                    () = tokio::time::sleep(wait) => return Ok(()),
                    msg = rx.recv() => msg,
                }
            }
            None => rx.recv().await,
        };
        let Some(event) = event else {
            return Ok(());
        };
        write_event(&mut stdout, event, &mut contaminate, &echo_lines).await?;
    }
}

async fn write_event(
    stdout: &mut tokio::io::Stdout,
    event: StdinEvent,
    contaminate: &mut bool,
    echo_lines: &AtomicBool,
) -> Result<(), Error> {
    match event {
        StdinEvent::Redraw => {
            stdout.write_all(b"stub> \n").await?;
        }
        StdinEvent::Bracket => {
            if *contaminate {
                *contaminate = false;
                stdout.write_all(b"EXTRA\n").await?;
            }
        }
        StdinEvent::Partial(text) => {
            stdout.write_all(b"stub> \n").await?;
            stdout.write_all(text.as_bytes()).await?;
            if !text.ends_with('\n') {
                stdout.write_all(b"\n").await?;
            }
        }
        StdinEvent::Submit(text) => {
            stdout.write_all(b"SUBMIT\n").await?;
            stdout.write_all(text.as_bytes()).await?;
            stdout.write_all(b"\nstub> \n").await?;
        }
        StdinEvent::Finish(line) => {
            stdout.write_all(line.as_bytes()).await?;
            stdout.write_all(b"stub> \n").await?;
        }
        // A tty with ECHO already showed this line. Writing it again races the
        // rest of the paste and tears the composer (`WWWWFrom: …`). A pipe has
        // no kernel echo, so the line is written there.
        StdinEvent::Line(line) if echo_lines.load(Ordering::Acquire) => {
            stdout.write_all(line.as_bytes()).await?;
        }
        StdinEvent::Line(_) => {}
    }
    stdout.flush().await?;
    Ok(())
}

fn read_stdin(
    tx: &tokio::sync::mpsc::UnboundedSender<StdinEvent>,
    contaminate: bool,
    buffer: bool,
    echo_lines: &AtomicBool,
) {
    // Dup the fd so this read does not hold the stdio lock. `process::exit`
    // flushes stdio and would deadlock on that lock until the pty closed.
    let Ok(fd) = std::io::stdin().as_fd().try_clone_to_owned() else {
        return;
    };
    let mut file = std::fs::File::from(fd);
    let raw = disable_canonical(&file);
    // Set before any read so a `Line` event is not echoed twice.
    echo_lines.store(!raw, Ordering::Release);
    if raw && buffer {
        read_buffered(&mut file, tx);
    } else if raw {
        read_raw(&mut file, tx, contaminate);
    } else {
        read_lines(file, tx, contaminate);
    }
}

fn read_buffered(file: &mut std::fs::File, tx: &tokio::sync::mpsc::UnboundedSender<StdinEvent>) {
    let mut scan = Scanner::new();
    let mut buf = [0_u8; 256];
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let Some(bytes) = buf.get(..n) else {
            return;
        };
        for &byte in bytes {
            if let Some(event) = scan.push(byte)
                && tx.send(event).is_err()
            {
                return;
            }
        }
    }
}

/// Turn off `ICANON` so `C-u` is delivered. `ECHOCTL` would print that newline
/// as `^J` once it is no longer a canonical special character. A pipe is not a
/// tty; leave it alone.
fn disable_canonical(file: &std::fs::File) -> bool {
    let Ok(mut termios) = rustix::termios::tcgetattr(file) else {
        return false;
    };
    termios
        .local_modes
        .remove(rustix::termios::LocalModes::ICANON | rustix::termios::LocalModes::ECHOCTL);
    set_special(&mut termios, rustix::termios::SpecialCodeIndex::VMIN, 1);
    set_special(&mut termios, rustix::termios::SpecialCodeIndex::VTIME, 0);
    rustix::termios::tcsetattr(file, rustix::termios::OptionalActions::Now, &termios).is_ok()
}

fn set_special(
    termios: &mut rustix::termios::Termios,
    index: rustix::termios::SpecialCodeIndex,
    value: u8,
) {
    termios.special_codes[index] = value;
}

fn read_raw(
    file: &mut std::fs::File,
    tx: &tokio::sync::mpsc::UnboundedSender<StdinEvent>,
    mut contaminate: bool,
) {
    let mut pending = Vec::new();
    let mut last_byte = std::time::Instant::now();
    let mut buf = [0_u8; 256];
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let Some(bytes) = buf.get(..n) else {
            return;
        };
        for &byte in bytes {
            if byte == CLEAR_LINE {
                pending.clear();
                last_byte = std::time::Instant::now();
                if tx.send(StdinEvent::Redraw).is_err() {
                    return;
                }
                continue;
            }
            let now = std::time::Instant::now();
            let gap = now.saturating_duration_since(last_byte);
            last_byte = now;
            let held = !pending.is_empty();
            pending.push(byte);
            if byte != b'\n' {
                continue;
            }
            if contaminate {
                contaminate = false;
                write_extra();
            }
            let line = String::from_utf8_lossy(&pending).into_owned();
            pending.clear();
            // A paste is one burst: the newline follows its line immediately.
            // Enter arrives later, completing the reply line that had no newline.
            // Reprinting on the first line of the next paste would split that
            // paste across a new prompt and the composer check would miss it.
            let event = if paused_enter(held, gap) {
                StdinEvent::Finish(line)
            } else {
                StdinEvent::Line(line)
            };
            if tx.send(event).is_err() {
                return;
            }
        }
    }
}

/// Enter is a newline that arrives after a pause while a partial line is held.
fn paused_enter(held_partial: bool, gap: Duration) -> bool {
    held_partial && gap >= Duration::from_millis(40)
}

fn read_lines(
    file: std::fs::File,
    tx: &tokio::sync::mpsc::UnboundedSender<StdinEvent>,
    mut contaminate: bool,
) {
    let mut reader = std::io::BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                if contaminate {
                    contaminate = false;
                    write_extra();
                }
                if tx
                    .send(StdinEvent::Line(std::mem::take(&mut line)))
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}

fn write_extra() {
    let mut out = std::io::stdout();
    drop(writeln!(out, "EXTRA"));
    drop(out.flush());
}

fn exit_due(exit_at: Option<tokio::time::Instant>) -> bool {
    exit_at.is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::paused_enter;

    #[test]
    fn only_a_paused_newline_on_a_held_line_is_enter() {
        assert!(paused_enter(true, Duration::from_millis(40)));
        assert!(!paused_enter(true, Duration::from_millis(39)));
        assert!(!paused_enter(false, Duration::from_secs(2)));
    }
}
