//! Herdr control-socket ping. Doctor reads one newline-delimited JSON reply.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::doctor_facts::HerdrView;
use crate::paths::Paths;
use crate::spawn::which;

/// Absent when the binary is missing. Otherwise the socket `ping` protocol, or
/// unreachable when the socket does not answer.
#[must_use]
pub(crate) fn probe(paths: &Paths) -> HerdrView {
    if which(&paths.path_env, "herdr").is_none() {
        return HerdrView::Absent;
    }
    match ping_socket(&socket_path(paths)) {
        Some(protocol) => HerdrView::Protocol(protocol),
        None => HerdrView::Unreachable,
    }
}

/// `HERDR_SOCKET_PATH`, else the session socket, else `~/.config/herdr/herdr.sock`.
#[must_use]
pub(crate) fn socket_path(paths: &Paths) -> PathBuf {
    if let Some(path) = paths.vars.get("HERDR_SOCKET_PATH")
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }
    let root = paths
        .xdg_config_home
        .clone()
        .unwrap_or_else(|| paths.home.join(".config"))
        .join("herdr");
    if let Some(session) = paths.vars.get("HERDR_SESSION")
        && !session.is_empty()
    {
        return root.join("sessions").join(session).join("herdr.sock");
    }
    root.join("herdr.sock")
}

/// Send `ping` and read `protocol` from a bare pong or a `{result: ...}` body.
#[must_use]
pub(crate) fn ping_socket(path: &Path) -> Option<u32> {
    let mut stream = UnixStream::connect(path).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(300)))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_millis(300)))
        .ok()?;
    stream
        .write_all(b"{\"id\":\"rig-doctor\",\"method\":\"ping\",\"params\":{}}\n")
        .ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    protocol_from_pong(&line)
}

fn protocol_from_pong(line: &str) -> Option<u32> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let body = value.get("result").unwrap_or(&value);
    let protocol = body.get("protocol")?.as_u64()?;
    u32::try_from(protocol).ok()
}

/// Last word of `tmux show -g window-size`, such as `latest` or `manual`.
#[must_use]
pub(crate) fn parse_window_size(text: &str) -> Option<String> {
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    line.split_whitespace().last().map(str::to_owned)
}

/// `launchctl managername` prints `Aqua` for a macOS Aqua session.
#[must_use]
pub(crate) fn launchctl_is_aqua(text: &str) -> bool {
    text.trim() == "Aqua"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pong_protocol_and_window_size_and_aqua() {
        assert_eq!(
            protocol_from_pong("{\"type\":\"pong\",\"protocol\":22}\n"),
            Some(22)
        );
        assert_eq!(
            protocol_from_pong("{\"id\":\"1\",\"result\":{\"type\":\"pong\",\"protocol\":21}}\n"),
            Some(21)
        );
        assert_eq!(
            parse_window_size("window-size manual\n").as_deref(),
            Some("manual")
        );
        assert!(launchctl_is_aqua("Aqua\n"));
        assert!(!launchctl_is_aqua("Background\n"));
    }

    #[test]
    fn live_socket_returns_the_protocol() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("herdr.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            std::io::BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            stream
                .write_all(b"{\"type\":\"pong\",\"version\":\"0.9.3\",\"protocol\":22}\n")
                .unwrap();
        });
        assert_eq!(ping_socket(&sock), Some(22));
        server.join().unwrap();
    }
}
