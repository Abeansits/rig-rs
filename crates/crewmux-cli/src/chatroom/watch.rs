//! `rig chatroom watch`. Ctrl-C ends the process with status 0.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::autostart::ensure_daemon;
use crate::error::Error;
use crate::output;

use super::http::{format_message, open_watch};

pub(super) fn run(rig: &str, json: bool) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let (_path, mut stream) = open_watch(&client, rig)?;
    if stream.status != 200 {
        let mut body = String::new();
        let _read = stream.read_to_string(&mut body);
        return Err(Error::User(format!(
            "Watch failed (HTTP {})",
            stream.status
        )));
    }
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(Error::Io)?;
    let stop = Arc::new(AtomicBool::new(false));
    let _hook = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&stop))
        .map_err(Error::Io)?;
    let mut pending = Vec::new();
    let mut discarding = false;
    let mut buf = [0_u8; 1024];
    let mut skips = 0_u32;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(0);
        }
        match stream.read(&mut buf) {
            Ok(0) => {
                emit_lines(&mut pending, json, &mut discarding);
                return Ok(0);
            }
            Ok(n) => {
                skips = 0;
                if let Some(chunk) = buf.get(..n) {
                    pending.extend_from_slice(chunk);
                }
                emit_lines(&mut pending, json, &mut discarding);
            }
            Err(err) if timed_out(&err) => {}
            Err(err) if oversized(&err) => {
                skips = skips.saturating_add(1);
                output::stderr_line("warning: skipped an oversized chat frame");
                if skips > SKIP_LIMIT {
                    return Err(Error::Io(err));
                }
            }
            Err(err) => {
                if stop.load(Ordering::Relaxed) {
                    return Ok(0);
                }
                return Err(Error::Io(err));
            }
        }
    }
}

/// A frame this long is past the chat body cap plus the SSE envelope.
const MAX_LINE: usize = 128 * 1024;
/// Repeated oversized-frame errors mean the stream is not making progress.
const SKIP_LIMIT: u32 = 4;

fn emit_lines(pending: &mut Vec<u8>, json: bool, discarding: &mut bool) {
    if *discarding {
        let Some(at) = pending.iter().position(|byte| *byte == b'\n') else {
            pending.clear();
            return;
        };
        let rest = pending.get(at.saturating_add(1)..).unwrap_or(&[]).to_vec();
        *pending = rest;
        *discarding = false;
    }
    if pending.len() > MAX_LINE && !pending.contains(&b'\n') {
        output::stderr_line("warning: skipped an oversized chat frame");
        pending.clear();
        *discarding = true;
        return;
    }
    for line in take_lines(pending) {
        if line.len() > MAX_LINE {
            output::stderr_line("warning: skipped an oversized chat frame");
            continue;
        }
        print_data(&line, json);
    }
}

fn print_data(line: &str, json: bool) {
    let Some(data) = line.strip_prefix("data:") else {
        return;
    };
    let data = data.trim();
    if data.is_empty() {
        return;
    }
    if json {
        output::stdout_line(data);
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return;
    };
    let payload = value.get("payload").unwrap_or(&value);
    output::stdout_line(&format_message(payload));
}

fn take_lines(pending: &mut Vec<u8>) -> Vec<String> {
    let taken = std::mem::take(pending);
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in taken.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        if let Some(slice) = taken.get(start..index) {
            let text = String::from_utf8_lossy(slice);
            lines.push(text.trim_end_matches('\r').to_owned());
        }
        start = index.saturating_add(1);
    }
    if let Some(tail) = taken.get(start..) {
        pending.extend_from_slice(tail);
    }
    lines
}

fn timed_out(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

fn oversized(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::InvalidData
}
