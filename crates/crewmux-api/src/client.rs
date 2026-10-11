//! Blocking HTTP/1.1 client for the daemon socket.
//!
//! rig-rs uses `$RIG_HOME/run/daemon.sock`. The TypeScript daemon uses TCP.
//! This client implements both transports and adds no new routes.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::Error;

/// macOS `sun_path` is about 104 bytes. The TUI guards at 100.
pub const MAX_SUN_PATH_BYTES: usize = 100;

/// Byte length of a path, which is what `sun_path` counts.
#[must_use]
pub fn path_bytes(path: &Path) -> usize {
    path.as_os_str().len()
}

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
        self.round_trip(method, path, body, &[])
    }

    /// Send one request with extra headers and read until the peer closes.
    ///
    /// # Errors
    ///
    /// Same as [`request`](Self::request), plus [`Error::BadHeader`] when a header contains a line break.
    pub fn request_with_headers(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
    ) -> Result<Response, Error> {
        self.round_trip(method, path, body, headers)
    }

    fn round_trip(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
    ) -> Result<Response, Error> {
        validate_method(method)?;
        validate_path(path)?;
        let payload = body.unwrap_or(b"");
        let head = build_head(method, path, payload, headers)?;
        let (mut stream, target) = self.dial()?;
        exchange(&mut stream, &target, &head, payload)
    }

    fn dial(&self) -> Result<(IoStream, String), Error> {
        match &self.target {
            Target::Uds(path) => {
                let target = path.display().to_string();
                let stream = UnixStream::connect(path).map_err(|source| Error::Connect {
                    target: target.clone(),
                    source,
                })?;
                prepare_stream(&stream, &target, self.timeout)?;
                Ok((IoStream::Uds(stream), target))
            }
            Target::Tcp(addr) => {
                let target = addr.to_string();
                let stream = TcpStream::connect_timeout(addr, self.timeout).map_err(|source| {
                    Error::Connect {
                        target: target.clone(),
                        source,
                    }
                })?;
                prepare_stream(&stream, &target, self.timeout)?;
                Ok((IoStream::Tcp(stream), target))
            }
        }
    }
}

enum IoStream {
    Uds(UnixStream),
    Tcp(TcpStream),
}

impl Read for IoStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Uds(stream) => stream.read(buf),
            Self::Tcp(stream) => stream.read(buf),
        }
    }
}

impl Write for IoStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Uds(stream) => stream.write(buf),
            Self::Tcp(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Uds(stream) => stream.flush(),
            Self::Tcp(stream) => stream.flush(),
        }
    }
}

impl IoStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Uds(stream) => stream.set_read_timeout(timeout),
            Self::Tcp(stream) => stream.set_read_timeout(timeout),
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

/// `$RIG_HOME/run/tui.sock`, with the same `sun_path` fallback as [`daemon_socket`].
#[must_use]
pub fn tui_socket(rig_home: &Path) -> PathBuf {
    let preferred = rig_home.join("run").join("tui.sock");
    if preferred.as_os_str().len() <= 100 {
        return preferred;
    }
    short_socket(&preferred)
}

/// Daemon socket path. `tmp` and `uid` are ignored; slice 1's [`daemon_socket`] picks the fallback.
#[must_use]
pub fn daemon_bind_path(rig_home: &Path, _tmp: &Path, _uid: u32) -> PathBuf {
    daemon_socket(rig_home)
}

/// TUI control socket. `tmp` and `uid` are ignored; see [`tui_socket`].
#[must_use]
pub fn tui_socket_path(rig_home: &Path, _tmp: &Path, _uid: u32) -> PathBuf {
    tui_socket(rig_home)
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

fn build_head(
    method: &str,
    path: &str,
    payload: &[u8],
    headers: &[(&str, &str)],
) -> Result<String, Error> {
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    let mut accept = false;
    for (name, value) in headers {
        if !header_token(name) || value_has_break(value) {
            return Err(Error::BadHeader);
        }
        if name.eq_ignore_ascii_case("accept") {
            accept = true;
        }
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    if !accept {
        head.push_str("Accept: application/json\r\n");
    }
    let _ = write!(head, "Content-Length: {}\r\n\r\n", payload.len());
    Ok(head)
}

fn header_token(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn value_has_break(value: &str) -> bool {
    value.bytes().any(|byte| matches!(byte, b'\r' | b'\n' | 0))
}

#[path = "http_stream.rs"]
mod http_stream;

pub use http_stream::HttpStream;

fn prepare_stream(
    stream: &impl TimeoutStream,
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
    parse_response(&read_message(stream, target)?)
}

/// Read one response. A reset after a complete `Content-Length` body is the
/// peer closing; the bytes already read are the response.
fn read_message(stream: &mut impl Read, target: &str) -> Result<Vec<u8>, Error> {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        if message_complete(&buf) {
            return Ok(buf);
        }
        match stream.read(&mut chunk) {
            Ok(0) => {
                if header_end(&buf).is_some() {
                    return Ok(buf);
                }
                return Err(Error::Exchange {
                    target: target.to_owned(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "closed before an HTTP response",
                    ),
                });
            }
            Ok(n) => {
                if let Some(bytes) = chunk.get(..n) {
                    buf.extend_from_slice(bytes);
                }
            }
            Err(_) if message_complete(&buf) => return Ok(buf),
            Err(source) if source.kind() == std::io::ErrorKind::Interrupted => {}
            Err(source) => {
                return Err(Error::Exchange {
                    target: target.to_owned(),
                    source,
                });
            }
        }
    }
}

fn message_complete(buf: &[u8]) -> bool {
    let Some(split) = header_end(buf) else {
        return false;
    };
    let Some(head) = buf.get(..split) else {
        return false;
    };
    let Some(len) = content_length(head) else {
        return false;
    };
    buf.len().saturating_sub(split.saturating_add(4)) >= len
}

fn header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(head: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(head).ok()?;
    for line in text.split("\r\n").skip(1) {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
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
