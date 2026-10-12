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
        (process_start_ticks(server), process_start_ticks(client))
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

/// Clock ticks since boot from `/proc/<pid>/stat`, or the Unix start second from `ps`.
///
/// Linux prefers `/proc` so two processes started in the same second stay distinct.
/// Other platforms use `ps -o lstart=` under `LC_ALL=C`. `None` when neither is readable.
#[must_use]
pub fn process_start_ticks(pid: u32) -> Option<u64> {
    if let Some(ticks) = proc_start_ticks(pid) {
        return Some(ticks);
    }
    parse_lstart(&ps_column_c(pid, "lstart=")?)
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

fn ps_column_c(pid: u32, column: &str) -> Option<String> {
    ps_column_with(pid, column, true)
}

fn ps_column(pid: u32, column: &str) -> Option<String> {
    ps_column_with(pid, column, false)
}

fn ps_column_with(pid: u32, column: &str, locale_c: bool) -> Option<String> {
    let pid_text = pid.to_string();
    let mut command = Command::new("ps");
    command
        .args(["-ww", "-p", &pid_text, "-o", column])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if locale_c {
        command.env("LC_ALL", "C").env("LANG", "C");
    }
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

/// `LC_ALL=C ps -o lstart=` is `Dow Mon DD HH:MM:SS YYYY`.
fn parse_lstart(text: &str) -> Option<u64> {
    let mut parts = text.split_whitespace();
    let _dow = parts.next()?;
    let month = month_index(parts.next()?)?;
    let day: u64 = parts.next()?.parse().ok()?;
    let clock = parts.next()?;
    let year: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let mut clock_parts = clock.split(':');
    let hour: u64 = clock_parts.next()?.parse().ok()?;
    let minute: u64 = clock_parts.next()?.parse().ok()?;
    let second: u64 = clock_parts.next()?.parse().ok()?;
    if clock_parts.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let tod = hour
        .checked_mul(3_600)?
        .checked_add(minute.checked_mul(60)?)?
        .checked_add(second)?;
    u64::try_from(days)
        .ok()?
        .checked_mul(86_400)?
        .checked_add(tod)
}

fn month_index(name: &str) -> Option<u64> {
    Some(match name {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    })
}

/// Howard Hinnant's `days_from_civil`, public domain. `1970-01-01` is zero.
fn days_from_civil(mut year: i64, month: u64, day: u64) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > 31 {
        return None;
    }
    if month <= 2 {
        year = year.checked_sub(1)?;
    }
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let month_index = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * month_index + 2) / 5 + day - 1;
    let yoe = u64::try_from(yoe).ok()?;
    let doe = yoe.checked_mul(365)? + yoe / 4 - yoe / 100 + doy;
    let era_days = era.checked_mul(146_097)?;
    let doe = i64::try_from(doe).ok()?;
    era_days.checked_add(doe)?.checked_sub(719_468)
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
    use super::{
        born_with_client, days_from_civil, elapsed_means_created, parse_etime, parse_lstart,
        proc_start_ticks,
    };

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
    fn unix_epoch_is_zero_days() {
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(parse_lstart("Thu Jan  1 00:00:00 1970"), Some(0));
        assert_eq!(parse_lstart("Thu Jan 1 00:00:01 1970"), Some(1));
        assert_eq!(parse_lstart("not a time"), None);
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
