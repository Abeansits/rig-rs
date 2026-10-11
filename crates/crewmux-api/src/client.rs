//! Blocking HTTP/1.1 client for the daemon socket.
//!
//! rig-rs uses `$RIG_HOME/run/daemon.sock`. The TypeScript daemon uses TCP.
//! This client implements both transports and adds no new routes.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::Error;

/// How long a single daemon request may block, unless [`Client::with_timeout`] says otherwise.
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// One HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// Status code from the status line.
    pub status: u16,
    /// Body after the header block. May be empty.
    pub body: String,
}

/// Typed client. Construct it with [`Client::over_uds`] or [`Client::over_tcp`].
#[derive(Debug, Clone)]
pub struct Client {
    target: Target,
    timeout: Duration,
}

#[derive(Debug, Clone)]
enum Target {
    Uds(PathBuf),
    Tcp(SocketAddr),
}

impl Client {
    /// Talk HTTP over the rig-rs Unix socket.
    #[must_use]
    pub fn over_uds(path: impl Into<PathBuf>) -> Self {
        Self {
            target: Target::Uds(path.into()),
            timeout: IO_TIMEOUT,
        }
    }

    /// Talk HTTP over TCP. The diff harness rejects this for the rig-rs side.
    #[must_use]
    pub fn over_tcp(addr: SocketAddr) -> Self {
        Self {
            target: Target::Tcp(addr),
            timeout: IO_TIMEOUT,
        }
    }

    /// Override the read and write timeout. `rig up` uses this; probes stay short.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// `GET /healthz`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the socket cannot be reached or the response is not HTTP.
    pub fn get_healthz(&self) -> Result<Response, Error> {
        self.request("GET", "/healthz", None)
    }

    /// Send one request and read the response until the peer closes.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on an illegal path or method, a dial failure, or a malformed response.
    pub fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<Response, Error> {
        validate_method(method)?;
        validate_path(path)?;
        let head = build_head(method, path, body.unwrap_or(b""));
        let payload = body.unwrap_or(b"");
        match &self.target {
            Target::Uds(path) => {
                let target = path.display().to_string();
                let mut stream = UnixStream::connect(path).map_err(|source| Error::Connect {
                    target: target.clone(),
                    source,
                })?;
                prepare_stream(&mut stream, &target, self.timeout)?;
                exchange(&mut stream, &target, &head, payload)
            }
            Target::Tcp(addr) => {
                let target = addr.to_string();
                let mut stream =
                    TcpStream::connect_timeout(addr, self.timeout).map_err(|source| {
                        Error::Connect {
                            target: target.clone(),
                            source,
                        }
                    })?;
                prepare_stream(&mut stream, &target, self.timeout)?;
                exchange(&mut stream, &target, &head, payload)
            }
        }
    }
}

/// `RIG_HOME`, or `~/.rig-rs` when the variable is unset.
#[must_use]
pub fn rig_home() -> PathBuf {
    if let Some(value) = std::env::var_os("RIG_HOME") {
        return PathBuf::from(value);
    }
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from(".rig-rs"),
        |home| PathBuf::from(home).join(".rig-rs"),
    )
}

/// Path of the rig-rs daemon socket under a `RIG_HOME`.
///
/// Paths that would not fit in `sockaddr_un` are replaced with a short socket
/// under `$TMPDIR/rig-<uid>/` (or `/tmp/rig-<uid>/`). The client and the daemon
/// share this function. The directory is private; callers must not chmod `TMPDIR`.
#[must_use]
pub fn daemon_socket(rig_home: &Path) -> PathBuf {
    let preferred = rig_home.join("run").join("daemon.sock");
    if preferred.as_os_str().len() <= 100 {
        return preferred;
    }
    short_socket(&preferred)
}

fn short_socket(preferred: &Path) -> PathBuf {
    let file = format!(
        "rig-{:016x}.sock",
        fnv64(preferred.as_os_str().as_encoded_bytes())
    );
    let dir = format!("rig-{}", current_uid());
    let under = |root: &Path| root.join(&dir).join(&file);
    if let Some(tmp) = std::env::var_os("TMPDIR") {
        let candidate = under(Path::new(&tmp));
        if candidate.as_os_str().len() <= 100 {
            return candidate;
        }
    }
    under(Path::new("/tmp"))
}

fn current_uid() -> u32 {
    if let Ok(text) = std::fs::read_to_string("/proc/self/status") {
        for line in text.lines() {
            let Some(rest) = line.strip_prefix("Uid:") else {
                continue;
            };
            if let Some(token) = rest.split_whitespace().next()
                && let Ok(uid) = token.parse()
            {
                return uid;
            }
        }
    }
    let Ok(child) = std::process::Command::new("id")
        .arg("-u")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return 0;
    };
    let Ok(output) = child.wait_with_output() else {
        return 0;
    };
    String::from_utf8(output.stdout)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

fn fnv64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn validate_method(method: &str) -> Result<(), Error> {
    if method.is_empty() || !method.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err(Error::BadMethod);
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), Error> {
    if !path.starts_with('/') || path.contains(['\r', '\n', ' ']) {
        return Err(Error::BadPath);
    }
    Ok(())
}

fn build_head(method: &str, path: &str, payload: &[u8]) -> String {
    format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nAccept: application/json\r\nContent-Length: {}\r\n\r\n",
        payload.len()
    )
}

fn prepare_stream(
    stream: &mut impl TimeoutStream,
    target: &str,
    timeout: Duration,
) -> Result<(), Error> {
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|source| Error::Exchange {
            target: target.to_owned(),
            source,
        })?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|source| Error::Exchange {
            target: target.to_owned(),
            source,
        })?;
    Ok(())
}

fn exchange(
    stream: &mut (impl Read + Write),
    target: &str,
    head: &str,
    payload: &[u8],
) -> Result<Response, Error> {
    stream
        .write_all(head.as_bytes())
        .map_err(|source| Error::Exchange {
            target: target.to_owned(),
            source,
        })?;
    stream
        .write_all(payload)
        .map_err(|source| Error::Exchange {
            target: target.to_owned(),
            source,
        })?;
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .map_err(|source| Error::Exchange {
            target: target.to_owned(),
            source,
        })?;
    parse_response(&buf)
}

fn parse_response(buf: &[u8]) -> Result<Response, Error> {
    let text = String::from_utf8(buf.to_vec()).map_err(|_| Error::BodyEncoding)?;
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| Error::Protocol("missing header terminator".to_owned()))?;
    let status_line = match head.split('\n').next() {
        Some(line) => line.trim_end_matches('\r'),
        None => head,
    };
    let mut parts = status_line.split_whitespace();
    let _version = parts.next();
    let code = parts
        .next()
        .ok_or_else(|| Error::Protocol("missing status code".to_owned()))?;
    let status = code
        .parse::<u16>()
        .map_err(|_| Error::Protocol(format!("bad status code {code}")))?;
    Ok(Response {
        status,
        body: body.to_owned(),
    })
}

/// Streams that accept the same timeouts as `UnixStream` and `TcpStream`.
trait TimeoutStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()>;
    fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()>;
}

impl TimeoutStream for UnixStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        UnixStream::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        UnixStream::set_write_timeout(self, timeout)
    }
}

impl TimeoutStream for TcpStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        TcpStream::set_write_timeout(self, timeout)
    }
}
