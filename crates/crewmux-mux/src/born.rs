//! Whether this control client started the tmux server.
//!
//! `new-session -A` attaches when a server is already listening. Start times
//! measured after that command tell a server this client forked from a server
//! that appeared earlier.

use std::process::{Command, Stdio};

/// `true` when `server` started at the same moment as `client` or later.
///
/// `None` when neither `/proc` nor `ps` can see both processes. Linux uses
/// clock ticks. Other platforms use elapsed seconds from `ps`, so a server
/// that is older by less than a second can look the same age.
pub(crate) fn server_not_older(server: u32, client: u32) -> Option<bool> {
    if let (Some(server_ticks), Some(client_ticks)) =
        (proc_start_ticks(server), proc_start_ticks(client))
    {
        return Some(born_with_client(server_ticks, client_ticks));
    }
    let server_elapsed = elapsed_secs(server)?;
    let client_elapsed = elapsed_secs(client)?;
    Some(elapsed_means_created(server_elapsed, client_elapsed))
}

/// Server start tick is not before the client that forked it.
fn born_with_client(server_start: u64, client_start: u64) -> bool {
    server_start >= client_start
}

/// A larger elapsed time is an older process. Equal seconds are not older.
fn elapsed_means_created(server_elapsed: u64, client_elapsed: u64) -> bool {
    server_elapsed <= client_elapsed
}

fn proc_start_ticks(pid: u32) -> Option<u64> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `comm` is wrapped in parentheses and may itself contain ')'.
    let rest = text.rsplit_once(')')?.1;
    // Field 22 of `stat` is starttime, the 20th token after `comm`.
    rest.split_whitespace().nth(19)?.parse().ok()
}

fn elapsed_secs(pid: u32) -> Option<u64> {
    if let Some(text) = ps_column(pid, "etimes=")
        && let Ok(secs) = text.parse::<u64>()
    {
        return Some(secs);
    }
    ps_column(pid, "etime=").and_then(|text| parse_etime(&text))
}

fn ps_column(pid: u32, column: &str) -> Option<String> {
    let pid_text = pid.to_string();
    let mut command = Command::new("ps");
    command
        .args(["-ww", "-p", &pid_text, "-o", column])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = command.spawn().ok()?;
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

/// `ps -o etime=` is `[[dd-]hh:]mm:ss`.
fn parse_etime(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (days, clock) = match text.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, text),
    };
    let mut parts = clock.split(':');
    let first = parts.next()?.parse::<u64>().ok()?;
    let second = parts.next()?.parse::<u64>().ok()?;
    match parts.next() {
        None => Some(days * 86_400 + first * 60 + second),
        Some(third) => {
            if parts.next().is_some() {
                return None;
            }
            let third = third.parse::<u64>().ok()?;
            Some(days * 86_400 + first * 3_600 + second * 60 + third)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{born_with_client, elapsed_means_created, parse_etime, proc_start_ticks};

    #[test]
    fn an_older_server_was_not_forked_by_this_client() {
        assert!(born_with_client(20, 10));
        assert!(born_with_client(10, 10));
        assert!(!born_with_client(9, 10));
        assert!(elapsed_means_created(1, 4));
        assert!(elapsed_means_created(4, 4));
        assert!(!elapsed_means_created(5, 4));
    }

    #[test]
    fn etime_parses_minutes_hours_and_days() {
        assert_eq!(parse_etime("00:05"), Some(5));
        assert_eq!(parse_etime(" 01:02 "), Some(62));
        assert_eq!(parse_etime("01:02:03"), Some(3_723));
        assert_eq!(parse_etime("1-00:00:01"), Some(86_401));
        assert_eq!(parse_etime(""), None);
        assert_eq!(parse_etime("nope"), None);
        assert_eq!(parse_etime("1:2:3:4"), None);
    }

    #[test]
    fn this_process_start_tick_is_stable_when_proc_exists() {
        let Some(ticks) = proc_start_ticks(std::process::id()) else {
            return;
        };
        assert_eq!(proc_start_ticks(std::process::id()), Some(ticks));
        assert!(ticks > 0);
    }
}
