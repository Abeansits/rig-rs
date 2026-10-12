//! One-shot HTTP server on a private Unix socket. Records each request.
#![expect(clippy::expect_used, reason = "a missing temp socket fails the test")]

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use parking_lot::Mutex;

#[derive(Clone)]
pub(crate) struct Hit {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) body: String,
    pub(crate) headers: String,
}

pub(crate) struct Scripted {
    pub(crate) status: u16,
    pub(crate) body: String,
}

pub(crate) struct FakeDaemon {
    _dir: tempfile::TempDir,
    path: PathBuf,
    hits: Arc<Mutex<Vec<Hit>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeDaemon {
    pub(crate) fn start(script: Vec<Scripted>) -> Self {
        let dir = tempfile::tempdir().expect("socket dir");
        let path = dir.path().join("daemon.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let hits = Arc::new(Mutex::new(Vec::new()));
        let script = Arc::new(Mutex::new(script));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread(listener, Arc::clone(&hits), script, Arc::clone(&stop));
        Self {
            _dir: dir,
            path,
            hits,
            stop,
            thread: Some(thread),
        }
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub(crate) fn hits(&self) -> Vec<Hit> {
        self.hits.lock().clone()
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(stream) = UnixStream::connect(&self.path) {
            drop(stream);
        }
        if let Some(thread) = self.thread.take() {
            drop(thread.join());
        }
        drop(std::fs::remove_file(&self.path));
    }
}

fn thread(
    listener: UnixListener,
    hits: Arc<Mutex<Vec<Hit>>>,
    script: Arc<Mutex<Vec<Scripted>>>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let Ok(mut stream) = listener.accept().map(|(stream, _)| stream) else {
                break;
            };
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let Ok(request) = read_request(&mut stream) else {
                continue;
            };
            hits.lock().push(request_hit(&request));
            let scripted = script.lock().pop_front_script();
            drop(write_response(&mut stream, &scripted));
        }
    })
}

trait PopFront {
    fn pop_front_script(&mut self) -> Scripted;
}

impl PopFront for Vec<Scripted> {
    fn pop_front_script(&mut self) -> Scripted {
        if self.is_empty() {
            Scripted {
                status: 500,
                body: "{\"error\":\"no script\"}".to_owned(),
            }
        } else {
            self.remove(0)
        }
    }
}

struct RawRequest {
    head: String,
    body: String,
}

fn request_hit(request: &RawRequest) -> Hit {
    let mut lines = request.head.lines();
    let start = lines.next().unwrap_or("");
    let mut parts = start.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let path = parts.next().unwrap_or("").to_owned();
    Hit {
        method,
        path,
        body: request.body.clone(),
        headers: request.head.clone(),
    }
}

fn read_request(stream: &mut UnixStream) -> std::io::Result<RawRequest> {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break None;
        }
        let Some(piece) = chunk.get(..n) else {
            break None;
        };
        buf.extend_from_slice(piece);
        if let Some(index) = find_header_end(&buf) {
            break Some(index);
        }
        if buf.len() > 1_000_000 {
            break None;
        }
    };
    let Some(index) = header_end else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "no header",
        ));
    };
    let Some(head_bytes) = buf.get(..index) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "header index",
        ));
    };
    let head = String::from_utf8_lossy(head_bytes).into_owned();
    let length = content_length(&head);
    let Some(rest) = buf.get(index.saturating_add(4)..) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "header end",
        ));
    };
    let mut body = rest.to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        let Some(piece) = chunk.get(..n) else {
            break;
        };
        body.extend_from_slice(piece);
    }
    body.truncate(length);
    Ok(RawRequest {
        head,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(head: &str) -> usize {
    for line in head.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse().unwrap_or(0);
        }
    }
    0
}

fn write_response(stream: &mut UnixStream, scripted: &Scripted) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        scripted.status,
        scripted.body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(scripted.body.as_bytes())?;
    stream.flush()
}
