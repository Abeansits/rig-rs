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

/// How long a single daemon request may block.
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
        }
    }

    /// Talk HTTP over TCP. The diff harness rejects this for the rig-rs side.
    #[must_use]
    pub fn over_tcp(addr: SocketAddr) -> Self {
        Self {
            target: Target::Tcp(addr),
        }
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
                prepare_stream(&mut stream, &target)?;
                exchange(&mut stream, &target, &head, payload)
            }
            Target::Tcp(addr) => {
                let target = addr.to_string();
                let mut stream = TcpStream::connect(addr).map_err(|source| Error::Connect {
                    target: target.clone(),
                    source,
                })?;
                prepare_stream(&mut stream, &target)?;
                exchange(&mut stream, &target, &head, payload)
            }
        }
    }
}

/// Path of the rig-rs daemon socket under a `RIG_HOME`.
#[must_use]
pub fn daemon_socket(rig_home: &Path) -> PathBuf {
    rig_home.join("run").join("daemon.sock")
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

fn prepare_stream(stream: &mut impl TimeoutStream, target: &str) -> Result<(), Error> {
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|source| Error::Exchange {
            target: target.to_owned(),
            source,
        })?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
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
