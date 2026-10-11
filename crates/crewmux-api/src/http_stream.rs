//! Incremental HTTP/1.1 body reader for the chat watch stream.

use std::io::{self, Read, Write};
use std::time::Duration;

use super::{Client, Error, IoStream, build_head, validate_method, validate_path};

const MAX_CHUNK: usize = 1_048_576;

/// A response whose body can be read before the peer closes.
pub struct HttpStream {
    /// Status code.
    pub status: u16,
    /// `Content-Type`, or empty when the response omitted it.
    pub content_type: String,
    io: IoStream,
    raw: Vec<u8>,
    raw_pos: usize,
    decoded: Vec<u8>,
    decoded_pos: usize,
    mode: BodyMode,
    eof: bool,
}

enum BodyMode {
    Chunked,
    Length(u64),
    UntilClose,
}

impl Client {
    /// Open a request and return once the response headers have arrived.
    ///
    /// # Errors
    ///
    /// Same as [`Client::request`](crate::Client::request).
    pub fn open(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
    ) -> Result<HttpStream, Error> {
        validate_method(method)?;
        validate_path(path)?;
        let head = build_head(method, path, b"", headers)?;
        let (mut stream, target) = self.dial()?;
        stream
            .write_all(head.as_bytes())
            .map_err(|source| Error::Exchange {
                target: target.clone(),
                source,
            })?;
        read_headers(stream, &target)
    }
}

impl HttpStream {
    /// Replace the socket read timeout. `rig chatroom watch` uses a short one so SIGINT is noticed.
    ///
    /// # Errors
    ///
    /// Returns the OS error from the socket.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.io.set_read_timeout(timeout)
    }
}

impl Read for HttpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.pump()?;
        let available = self.decoded.get(self.decoded_pos..).unwrap_or(&[]);
        if available.is_empty() {
            return Ok(0);
        }
        let n = buf.len().min(available.len());
        let Some(src) = available.get(..n) else {
            return Ok(0);
        };
        let Some(dst) = buf.get_mut(..n) else {
            return Ok(0);
        };
        dst.copy_from_slice(src);
        self.decoded_pos = self.decoded_pos.saturating_add(n);
        Ok(n)
    }
}

impl HttpStream {
    fn pump(&mut self) -> io::Result<()> {
        if self.has_decoded() || self.eof {
            return Ok(());
        }
        loop {
            self.compact();
            if self.pull_decoded()? {
                return Ok(());
            }
            let mut tmp = [0_u8; 1024];
            match self.io.read(&mut tmp) {
                Ok(0) => return self.finish_socket_eof(),
                Ok(n) => {
                    if let Some(chunk) = tmp.get(..n) {
                        self.raw.extend_from_slice(chunk);
                    }
                }
                Err(err) => return Err(err),
            }
        }
    }

    fn has_decoded(&self) -> bool {
        self.decoded.len() > self.decoded_pos
    }

    fn compact(&mut self) {
        if self.raw_pos == 0 || self.raw_pos < 4096 {
            return;
        }
        let rest = self.raw.get(self.raw_pos..).unwrap_or(&[]).to_vec();
        self.raw = rest;
        self.raw_pos = 0;
        if self.decoded_pos > 0 {
            let rest = self.decoded.get(self.decoded_pos..).unwrap_or(&[]).to_vec();
            self.decoded = rest;
            self.decoded_pos = 0;
        }
    }

    fn pull_decoded(&mut self) -> io::Result<bool> {
        match self.mode {
            BodyMode::Chunked => self.pull_chunk(),
            BodyMode::Length(left) => Ok(self.pull_length(left)),
            BodyMode::UntilClose => Ok(false),
        }
    }

    fn pull_length(&mut self, left: u64) -> bool {
        if left == 0 {
            self.eof = true;
            return true;
        }
        let unread = self.raw.get(self.raw_pos..).unwrap_or(&[]);
        if unread.is_empty() {
            return false;
        }
        let take = usize::try_from(left)
            .unwrap_or(usize::MAX)
            .min(unread.len());
        let Some(chunk) = unread.get(..take) else {
            return false;
        };
        self.decoded.extend_from_slice(chunk);
        self.raw_pos = self.raw_pos.saturating_add(take);
        let next = left.saturating_sub(u64::try_from(take).unwrap_or(u64::MAX));
        self.mode = BodyMode::Length(next);
        if next == 0 {
            self.eof = true;
        }
        true
    }

    fn pull_chunk(&mut self) -> io::Result<bool> {
        let unread = self.raw.get(self.raw_pos..).unwrap_or(&[]);
        let Some(line_at) = find_crlf(unread) else {
            if unread.len() > MAX_CHUNK {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "chunk size"));
            }
            return Ok(false);
        };
        let line = unread.get(..line_at).unwrap_or(&[]);
        let text = std::str::from_utf8(line)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "chunk size"))?;
        let size = chunk_size(text)?;
        let data_at = line_at.saturating_add(2);
        if size == 0 {
            self.raw_pos = self.raw_pos.saturating_add(data_at);
            self.eof = true;
            return Ok(true);
        }
        let need = data_at.saturating_add(size).saturating_add(2);
        if unread.len() < need {
            return Ok(false);
        }
        let Some(data) = unread.get(data_at..data_at.saturating_add(size)) else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "chunk"));
        };
        self.decoded.extend_from_slice(data);
        self.raw_pos = self.raw_pos.saturating_add(need);
        Ok(true)
    }

    fn finish_socket_eof(&mut self) -> io::Result<()> {
        let unread = self.raw.get(self.raw_pos..).unwrap_or(&[]);
        match self.mode {
            BodyMode::UntilClose => {
                self.decoded.extend_from_slice(unread);
                self.raw_pos = self.raw.len();
                self.eof = true;
                Ok(())
            }
            BodyMode::Length(0) | BodyMode::Chunked if unread.is_empty() => {
                self.eof = true;
                Ok(())
            }
            BodyMode::Length(_) | BodyMode::Chunked => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated HTTP body",
            )),
        }
    }
}

fn read_headers(mut stream: IoStream, target: &str) -> Result<HttpStream, Error> {
    let mut raw = Vec::new();
    let mut tmp = [0_u8; 1024];
    let split_at = loop {
        if let Some(at) = find_header_end(&raw) {
            break at;
        }
        if raw.len() > MAX_CHUNK {
            return Err(Error::Protocol("response headers are too large".to_owned()));
        }
        let n = stream.read(&mut tmp).map_err(|source| Error::Exchange {
            target: target.to_owned(),
            source,
        })?;
        if n == 0 {
            return Err(Error::Protocol("missing header terminator".to_owned()));
        }
        if let Some(chunk) = tmp.get(..n) {
            raw.extend_from_slice(chunk);
        }
    };
    let head = raw
        .get(..split_at)
        .ok_or_else(|| Error::Protocol("missing header terminator".to_owned()))?;
    let head = std::str::from_utf8(head).map_err(|_| Error::BodyEncoding)?;
    let status = status_code(head)?;
    let content_type = header_value(head, "content-type")
        .unwrap_or_default()
        .to_owned();
    let chunked = header_value(head, "transfer-encoding")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .split(',')
        .any(|part| part.trim() == "chunked");
    let length = header_value(head, "content-length").and_then(|text| text.trim().parse().ok());
    let mode = if chunked {
        BodyMode::Chunked
    } else if let Some(length) = length {
        BodyMode::Length(length)
    } else {
        BodyMode::UntilClose
    };
    let raw_pos = split_at.saturating_add(4);
    Ok(HttpStream {
        status,
        content_type,
        io: stream,
        raw,
        raw_pos,
        decoded: Vec::new(),
        decoded_pos: 0,
        mode,
        eof: false,
    })
}

fn status_code(head: &str) -> Result<u16, Error> {
    let line = head.lines().next().unwrap_or(head).trim_end_matches('\r');
    let code = line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| Error::Protocol("missing status code".to_owned()))?;
    code.parse::<u16>()
        .map_err(|_| Error::Protocol(format!("bad status code {code}")))
}

fn header_value<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    for line in head.lines() {
        let line = line.trim_end_matches('\r');
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.eq_ignore_ascii_case(name) {
            return Some(value.trim());
        }
    }
    None
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn find_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|window| window == b"\r\n")
}

fn chunk_size(text: &str) -> io::Result<usize> {
    let hex = text.split(';').next().unwrap_or(text).trim();
    let size = usize::from_str_radix(hex, 16)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "chunk size"))?;
    if size > MAX_CHUNK {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "chunk too large",
        ));
    }
    Ok(size)
}
