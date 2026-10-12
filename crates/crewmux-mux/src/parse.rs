//! Parsers for tmux `-F` rows.
//!
//! The separator is `|`. tmux 3.6 rewrites literal control characters in format
//! output to `_`, so a command that contained a tab still occupies one field.
//! A session name may contain `|`; it is the final field and is rejoined.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::MuxError;
use crate::types::{CONTROL_SESSION, ClientInfo, Geometry, PaneInfo};

/// Parse `list-panes -F` output, dropping the hidden control session.
pub(crate) fn parse_panes(stdout: &str) -> Result<Vec<PaneInfo>, MuxError> {
    let mut panes = Vec::new();
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let pane = parse_pane_line(line)?;
        if pane.session != CONTROL_SESSION {
            panes.push(pane);
        }
    }
    Ok(panes)
}

/// Parse one pane row, including a 3.6-sanitised command and a `|` inside the session name.
pub(crate) fn parse_pane_line(line: &str) -> Result<PaneInfo, MuxError> {
    let parts: Vec<&str> = line.split('|').collect();
    if parts.len() < 10 {
        return Err(MuxError::Control(format!("short pane row: {line}")));
    }
    let pane_id = field(&parts, 0, line)?.to_owned();
    let window_id = field(&parts, 1, line)?.to_owned();
    let dead = field(&parts, 3, line)? == "1";
    let pid = parse_pane_pid(field(&parts, 2, line)?, dead, line)?;
    let status_field = field(&parts, 4, line)?;
    let dead_status = if status_field.is_empty() {
        None
    } else {
        Some(parse_i32(status_field, "pane_dead_status")?)
    };
    let cols = parse_u16(field(&parts, 5, line)?, "pane_width")?;
    let rows = parse_u16(field(&parts, 6, line)?, "pane_height")?;
    let in_mode = field(&parts, 7, line)? == "1";
    let current_cmd = field(&parts, 8, line)?.to_owned();
    let session = parts.get(9..).unwrap_or(&[]).join("|");
    if session.is_empty() {
        return Err(MuxError::Control(format!(
            "pane row missing session: {line}"
        )));
    }
    Ok(PaneInfo {
        pane_id,
        window_id,
        session,
        pid,
        started_ticks: None,
        current_cmd,
        argv: None,
        dead,
        dead_status,
        size: Geometry { cols, rows },
        in_mode,
    })
}

/// Parse `list-clients` rows. Callers drop `control_mode` clients.
pub(crate) fn parse_clients(stdout: &str) -> Result<Vec<ClientInfo>, MuxError> {
    let mut clients = Vec::new();
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        clients.push(parse_client_line(line)?);
    }
    Ok(clients)
}

fn parse_client_line(line: &str) -> Result<ClientInfo, MuxError> {
    let parts: Vec<&str> = line.split('|').collect();
    if parts.len() < 4 {
        return Err(MuxError::Control(format!("short client row: {line}")));
    }
    let control_mode = field(&parts, 0, line)? == "1";
    let activity = parse_activity(field(&parts, 1, line)?)?;
    let tty = field(&parts, 2, line)?.to_owned();
    let session = parts.get(3..).unwrap_or(&[]).join("|");
    Ok(ClientInfo {
        tty,
        session,
        activity,
        control_mode,
    })
}

fn parse_activity(raw: &str) -> Result<SystemTime, MuxError> {
    let secs = raw
        .parse::<u64>()
        .map_err(|_| MuxError::Control(format!("bad client_activity: {raw}")))?;
    Ok(UNIX_EPOCH + Duration::from_secs(secs))
}

/// Human clients on `session`, newest activity first not required.
#[must_use]
pub(crate) fn human_clients_in<'a>(
    clients: &'a [ClientInfo],
    session: &str,
) -> Vec<&'a ClientInfo> {
    clients
        .iter()
        .filter(|client| !client.control_mode && client.session == session)
        .collect()
}

/// Newest human `client_activity` on `session`.
#[must_use]
pub(crate) fn latest_human_activity(clients: &[ClientInfo], session: &str) -> Option<SystemTime> {
    human_clients_in(clients, session)
        .into_iter()
        .map(|client| client.activity)
        .max()
}

fn field<'a>(parts: &'a [&str], index: usize, line: &str) -> Result<&'a str, MuxError> {
    parts
        .get(index)
        .copied()
        .ok_or_else(|| MuxError::Control(format!("short row: {line}")))
}

/// A dead remain-on-exit pane has an empty `#{pane_pid}` on tmux 3.8. That is not an error.
fn parse_pane_pid(raw: &str, dead: bool, line: &str) -> Result<Option<u32>, MuxError> {
    if raw.is_empty() {
        if dead {
            return Ok(None);
        }
        return Err(MuxError::Control(format!("bad pane_pid: {line}")));
    }
    Ok(Some(parse_u32(raw, "pane_pid")?))
}

fn parse_u32(raw: &str, what: &str) -> Result<u32, MuxError> {
    raw.parse::<u32>()
        .map_err(|_| MuxError::Control(format!("bad {what}: {raw}")))
}

fn parse_u16(raw: &str, what: &str) -> Result<u16, MuxError> {
    raw.parse::<u16>()
        .map_err(|_| MuxError::Control(format!("bad {what}: {raw}")))
}

fn parse_i32(raw: &str, what: &str) -> Result<i32, MuxError> {
    raw.parse::<i32>()
        .map_err(|_| MuxError::Control(format!("bad {what}: {raw}")))
}

/// `new-session -P -F '#{pane_id}|#{session_name}'`.
pub(crate) fn parse_created(stdout: &str) -> Result<(String, String), MuxError> {
    let line = stdout
        .lines()
        .rev()
        .find(|line| line.contains('|'))
        .ok_or_else(|| MuxError::Control(format!("new-session returned no pane id: {stdout}")))?;
    let (pane, session) = line
        .split_once('|')
        .ok_or_else(|| MuxError::Control(format!("new-session returned no pane id: {stdout}")))?;
    let session = session.trim();
    if !crate::argv::is_pane_id(pane) || session.is_empty() {
        return Err(MuxError::Control(format!(
            "new-session returned no pane id: {stdout}"
        )));
    }
    Ok((pane.to_owned(), session.to_owned()))
}

/// `#{cursor_x} #{cursor_y}`.
pub(crate) fn parse_cursor(stdout: &str) -> Result<(u16, u16), MuxError> {
    let mut parts = stdout.split_whitespace();
    let x = parts.next().unwrap_or("0");
    let y = parts.next().unwrap_or("0");
    let x = x
        .parse::<u16>()
        .map_err(|_| MuxError::Control(format!("bad cursor: {stdout}")))?;
    let y = y
        .parse::<u16>()
        .map_err(|_| MuxError::Control(format!("bad cursor: {stdout}")))?;
    Ok((x, y))
}

/// `#{pane_dead} #{pane_dead_status}`.
pub(crate) fn parse_dead_status(stdout: &str) -> Result<(bool, Option<i32>), MuxError> {
    let mut parts = stdout.split_whitespace();
    let dead = parts.next().unwrap_or("0") == "1";
    let status = match parts.next() {
        Some(raw) => Some(parse_i32(raw, "pane_dead_status")?),
        None => None,
    };
    Ok((dead, status))
}

#[cfg(test)]
mod tests {
    use super::parse_pane_line;

    #[test]
    fn tmux_3_6_sanitised_format_parses() {
        // A tab inside the command was rewritten to `_` by tmux 3.6. The session
        // name itself contains the field separator.
        let line = "%12|@3|4242|1||90|27|0|ba_sh|agent|notes";
        let pane = parse_pane_line(line).expect("parse");
        assert_eq!(pane.pane_id, "%12");
        assert_eq!(pane.window_id, "@3");
        assert_eq!(pane.pid, Some(4242));
        assert!(pane.dead);
        assert_eq!(pane.dead_status, None);
        assert_eq!(pane.size.cols, 90);
        assert_eq!(pane.size.rows, 27);
        assert!(!pane.in_mode);
        assert_eq!(pane.current_cmd, "ba_sh");
        assert_eq!(pane.session, "agent|notes");
    }

    #[test]
    fn empty_pid_on_a_dead_pane_is_valid() {
        let line = "%1|@1||1|7|90|27|0|bash|demo";
        let pane = parse_pane_line(line).expect("dead pane");
        assert!(pane.dead);
        assert_eq!(pane.pid, None);
        assert_eq!(pane.dead_status, Some(7));
        let alive = "%1|@1||0||90|27|0|bash|demo";
        let err = parse_pane_line(alive).expect_err("live pane needs a pid");
        assert!(err.to_string().contains("pane_pid"), "{err}");
    }
}
